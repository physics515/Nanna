//! Applying hand edits to `config.toml` without a restart.
//!
//! The daemon already re-applies config live when a client asks
//! (`config.reload` — the GUI's Settings path). An operator who edits the file
//! in a text editor had no such hop: nothing noticed, and the change waited for
//! a restart. This polls the one file the control plane loads and saves.
//!
//! **Polling, not a watcher crate.** One `stat` every two seconds costs nothing
//! measurable and needs no new dependency; inotify/FSEvents semantics around
//! editors that write-by-rename are exactly the edge a watcher crate exists to
//! paper over, and a stat of the path sidesteps them.
//!
//! **Validate before apply, and only apply a real change.** A file caught
//! mid-edit that does not parse is logged and the running config is kept. A
//! file whose parsed content equals the running config — the daemon's own
//! `config.set` save, or an editor touching without changing — is not
//! re-applied, so clients do not see a spurious `config_changed`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use nanna_config::Config;
use tracing::{debug, info, warn};

use super::ControlPlane;

/// How often the file is checked. A hand edit shows up within this.
pub const CONFIG_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How long a change must hold still before it is read — an editor's save is
/// several writes, and reading between them sees a truncated file.
pub const CONFIG_SETTLE: Duration = Duration::from_millis(500);

/// Identity of the file's current version, as cheaply as the OS gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    modified: SystemTime,
    len: u64,
}

fn stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(FileStamp {
        modified: meta.modified().ok()?,
        len: meta.len(),
    })
}

/// Whether a loaded config differs from the running one in any field. Pure.
///
/// Compared through serde rather than `PartialEq`, which `Config` does not
/// derive; both sides went through the same `with_env_overrides`.
fn differs(running: &Config, loaded: &Config) -> bool {
    match (serde_json::to_value(running), serde_json::to_value(loaded)) {
        (Ok(running), Ok(loaded)) => running != loaded,
        // Unserializable is not expected; applying is the conservative answer.
        _ => true,
    }
}

impl ControlPlane {
    /// Watch the config file this control plane loads, until `shutdown`.
    /// Does nothing when the control plane has no config path.
    pub fn spawn_config_watcher(
        self: &Arc<Self>,
        mut shutdown: tokio::sync::broadcast::Receiver<()>,
    ) {
        let Some(path) = self.config_path.clone() else {
            return;
        };
        let control = Arc::clone(self);
        tokio::spawn(async move {
            let mut last = stamp(&path);
            let mut ticker = tokio::time::interval(CONFIG_POLL_INTERVAL);
            ticker.tick().await;
            loop {
                tokio::select! {
                    _ = shutdown.recv() => break,
                    _ = ticker.tick() => {}
                }
                let seen = stamp(&path);
                if seen == last {
                    continue;
                }
                tokio::time::sleep(CONFIG_SETTLE).await;
                if stamp(&path) != seen {
                    // Still being written; the next tick looks again.
                    continue;
                }
                last = seen;
                if seen.is_some() {
                    control.reload_changed_config(&path).await;
                }
            }
        });
    }

    /// Load `path`, and apply it when it parses and differs from what is running.
    async fn reload_changed_config(&self, path: &Path) {
        let owned: PathBuf = path.to_path_buf();
        // Loaded as the replacement for what is running: a token saved with
        // no server recorded stays the running server's instead of following
        // the address the edit names.
        let running_host = self.config.read().await.memory.ollama_host.clone();
        let store = self.credential_store.clone();
        let loaded = tokio::task::spawn_blocking(move || {
            Config::load_from_replacing(&owned, &running_host, &store)
        })
        .await;
        let loaded = match loaded {
            Ok(Ok(config)) => config.with_env_overrides(),
            Ok(Err(e)) => {
                warn!(
                    "{} changed but does not load ({e}); keeping the running configuration",
                    path.display()
                );
                return;
            }
            Err(e) => {
                warn!("config reload task failed: {e}");
                return;
            }
        };
        if !differs(&*self.config.read().await, &loaded) {
            debug!(
                "{} changed on disk with no effective difference",
                path.display()
            );
            return;
        }
        info!("{} changed on disk; applying it", path.display());
        self.apply_loaded_config(loaded).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_real_difference_counts() {
        let running = Config::default();
        assert!(!differs(&running, &Config::default()));
        let mut edited = Config::default();
        edited.scheduler.heartbeat_enabled = !running.scheduler.heartbeat_enabled;
        assert!(differs(&running, &edited));
    }

    /// A hand edit of the address (which the agent can make) with a token
    /// saved by an older build, which records no server: the reload used to
    /// load the file as if it were the first, read the token as the new
    /// address's, and apply it — chat, embeddings and the probe all sent the
    /// old server's token to the edited one.
    #[tokio::test]
    async fn a_hand_edit_does_not_hand_a_legacy_ollama_token_to_the_new_address() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = nanna_config::SecureStore::file_only_at(dir.path().to_path_buf());
        store
            .set(
                nanna_config::credentials::keys::OLLAMA_API_KEY,
                "legacy-token",
            )
            .expect("set");
        let mut control = ControlPlane::new(Arc::new(crate::session::SessionManager::new()));
        control.credential_store = store;
        {
            // As the boot load left it: the legacy token, for the configured server.
            let mut config = control.config.write().await;
            config.memory.ollama_host = "https://gpu.example/ollama".to_string();
            config.llm.ollama_api_key = Some("legacy-token".to_string());
        }

        let file = dir.path().join("config.toml");
        let mut edited = Config::default();
        edited.memory.ollama_host = "https://elsewhere.example".to_string();
        edited.save_to(&file).expect("save");
        control.reload_changed_config(&file).await;

        let config = control.config.read().await;
        assert_eq!(
            config.memory.ollama_host, "https://elsewhere.example",
            "the edit applied"
        );
        assert_ne!(
            config.llm.ollama_api_key.as_deref(),
            Some("legacy-token"),
            "the edited address must not get the token that was going to the old one"
        );
    }

    /// The watcher re-reads the daemon's own `config.set` save. That save
    /// strips every secret, so a key set through `config.set` and never filed
    /// in the store read back as missing: a difference, applied — the key was
    /// gone from the running daemon within one poll of being set.
    #[tokio::test]
    async fn the_watcher_keeps_a_secret_set_through_config_set() {
        use crate::protocol::{Action, ConfigAction};
        // The environment's key wins every load, by design; with one set there
        // is no store read to show.
        if std::env::var("BRAVE_API_KEY").is_ok_and(|key| !key.trim().is_empty()) {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("config.toml");
        let mut control = ControlPlane::new(Arc::new(crate::session::SessionManager::new()));
        control.config_path = Some(file.clone());
        control.credential_store =
            nanna_config::SecureStore::file_only_at(dir.path().join("store"));
        let control = Arc::new(control);

        let resp = control
            .handle(
                "test",
                Action::Config(ConfigAction::Set {
                    path: "tools.brave_api_key".into(),
                    value: serde_json::json!("brave-set-by-config-set"),
                }),
            )
            .await;
        assert_eq!(resp["status"], "updated", "{resp}");
        control.reload_changed_config(&file).await;

        assert_eq!(
            control.config.read().await.tools.brave_api_key.as_deref(),
            Some("brave-set-by-config-set"),
            "the save the watcher read back still has the key"
        );
    }

    #[test]
    fn a_stamp_tracks_length_and_absence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("config.toml");
        assert_eq!(stamp(&file), None);
        std::fs::write(&file, "a").expect("write");
        let first = stamp(&file).expect("stamp");
        std::fs::write(&file, "ab").expect("write");
        assert_ne!(
            stamp(&file),
            Some(first),
            "a length change is a new version"
        );
    }
}
