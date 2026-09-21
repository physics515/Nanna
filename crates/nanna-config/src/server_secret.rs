//! `[server].webhook_secret`: the shared secret `nanna server`'s generic
//! webhook requires.
//!
//! Like every other secret it is held in the secure store and never written to
//! `config.toml`: `Config::strip_secrets_for_disk` blanks it in the copy a save
//! writes, `Config::migrate_secrets_to_keyring` files one held in memory, and
//! each load fills it from [`WEBHOOK_SECRET_ENV`], else the store.
//!
//! **A `config.toml` that holds one** — hand-written, the only way it was ever
//! set, or left by a build before this one — is migrated by [`adopt`] when the
//! file is loaded, before anything could save the config, so the save that
//! next strips it from the file loses nothing. The file itself is not
//! rewritten by a load (every CLI command, the GUI and the daemon load it,
//! some at once, and a rewrite from the parsed config would drop its
//! comments); the secret leaves it at the next save of any kind, and until
//! then each load says it can be deleted.

use crate::ServerConfig;
use crate::credentials::{SecureStore, keys};

/// The environment variable that supplies the secret when `config.toml` does
/// not. Never filed in the store.
pub const WEBHOOK_SECRET_ENV: &str = "NANNA_WEBHOOK_SECRET";

/// Its key in `config.toml`, for messages (never the value).
const FIELD: &str = "server.webhook_secret";

/// File in `store` the webhook secret `server` holds as parsed from
/// `config.toml` ([`crate::adopt_file_secret`]: the file's replaces a
/// different stored one, or the save that strips it would switch the endpoint
/// to another and refuse every caller set up with this one). `server` keeps
/// it. A blank one is none.
///
/// It is filed trimmed, and `server` keeps it trimmed: the loads after that
/// save read it from the store, and must run with what this one did.
///
/// Call on the parsed file only, before anything is filled in: a secret from
/// the environment is never filed.
pub fn adopt(server: &mut ServerConfig, store: &SecureStore) {
    let Some(held) = server
        .webhook_secret
        .as_deref()
        .map(str::trim)
        .filter(|held| !held.is_empty())
        .map(str::to_owned)
    else {
        return;
    };
    crate::adopt_file_secret(
        FIELD,
        WEBHOOK_SECRET_ENV,
        keys::SERVER_WEBHOOK_SECRET,
        &held,
        store,
    );
    server.webhook_secret = Some(held);
}

#[cfg(test)]
mod tests {
    use super::WEBHOOK_SECRET_ENV;
    use crate::Config;
    use crate::credentials::{SecureStore, keys};
    use std::path::Path;

    const SECRET: &str = "s3cret-generic-webhook";

    /// A hermetic store: its own directory, never the OS keyring.
    fn store() -> (tempfile::TempDir, SecureStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SecureStore::file_only_at(dir.path().to_path_buf());
        (dir, store)
    }

    fn stored(store: &SecureStore) -> Option<String> {
        store.get(keys::SERVER_WEBHOOK_SECRET).ok()
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    /// An environment in which only [`WEBHOOK_SECRET_ENV`] is set, to `value`.
    fn env_with(value: &'static str) -> impl Fn(&str) -> Option<String> {
        move |name| (name == WEBHOOK_SECRET_ENV).then(|| value.to_owned())
    }

    fn with_secret(secret: &str) -> Config {
        let mut config = Config::default();
        config.server.webhook_secret = Some(secret.to_owned());
        config
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("config.toml")
    }

    /// A hand-written `config.toml` whose `[server]` holds `secret`.
    fn write_config_holding(path: &Path, secret: &str) {
        std::fs::write(
            path,
            format!("[server]\nport = 4100\nwebhook_secret = \"{secret}\"\n"),
        )
        .expect("write");
    }

    // -----------------------------------------------------------------
    // Out of config.toml
    // -----------------------------------------------------------------

    #[test]
    fn saving_keeps_the_webhook_secret_out_of_config_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let mut config = with_secret(SECRET);
        config.server.port = 4100;

        config.save_to(&path).expect("save");

        let on_disk = read(&path);
        assert!(
            !on_disk.contains(SECRET),
            "leaked into config.toml: {on_disk}"
        );
        assert!(
            !on_disk.contains("webhook_secret"),
            "no key is written for it: {on_disk}"
        );
        assert!(
            on_disk.contains("port = 4100"),
            "the rest of [server] stays: {on_disk}"
        );
        assert_eq!(
            config.server.webhook_secret.as_deref(),
            Some(SECRET),
            "the running config keeps it"
        );
    }

    // -----------------------------------------------------------------
    // Filed and filled
    // -----------------------------------------------------------------

    #[test]
    fn migrating_files_the_webhook_secret_and_takes_it_out() {
        let (_dir, store) = store();
        let mut config = with_secret(&format!(" {SECRET}\n"));

        config.migrate_secrets_to(&store).expect("migrate");

        assert_eq!(stored(&store).as_deref(), Some(SECRET), "filed, trimmed");
        assert_eq!(config.server.webhook_secret, None, "taken out once filed");
        assert!(
            store
                .list_keys()
                .contains(&keys::SERVER_WEBHOOK_SECRET.to_owned()),
            "the store lists it with the other secrets"
        );
    }

    #[test]
    fn loading_fills_the_webhook_secret_from_the_store() {
        let (_dir, store) = store();
        store.set(keys::SERVER_WEBHOOK_SECRET, SECRET).expect("set");
        let mut config = Config::default();

        config.load_secrets_from(&store, no_env);

        assert_eq!(config.server.webhook_secret.as_deref(), Some(SECRET));
    }

    #[test]
    fn the_webhook_secret_can_come_from_the_environment() {
        let (_dir, store) = store();
        let mut config = Config::default();

        config.load_secrets_from(&store, env_with("env-secret"));

        assert_eq!(config.server.webhook_secret.as_deref(), Some("env-secret"));
    }

    #[test]
    fn config_toml_then_the_environment_then_the_store() {
        let (_dir, store) = store();
        store
            .set(keys::SERVER_WEBHOOK_SECRET, "stored-secret")
            .expect("set");

        let mut from_file = with_secret("file-secret");
        from_file.load_secrets_from(&store, env_with("env-secret"));
        assert_eq!(
            from_file.server.webhook_secret.as_deref(),
            Some("file-secret")
        );

        let mut from_env = Config::default();
        from_env.load_secrets_from(&store, env_with("env-secret"));
        assert_eq!(
            from_env.server.webhook_secret.as_deref(),
            Some("env-secret")
        );

        let mut from_store = Config::default();
        from_store.load_secrets_from(&store, no_env);
        assert_eq!(
            from_store.server.webhook_secret.as_deref(),
            Some("stored-secret")
        );
    }

    #[test]
    fn a_webhook_secret_set_once_survives_a_save_and_the_next_load() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        let mut config = with_secret(SECRET);
        config.migrate_secrets_to(&store).expect("migrate");
        config.save_to(&path).expect("save");

        let next = Config::load_from_with(&path, &store, no_env).expect("loads");

        assert_eq!(next.server.webhook_secret.as_deref(), Some(SECRET));
        assert!(!read(&path).contains(SECRET));
    }

    // -----------------------------------------------------------------
    // A config.toml that holds it. The save that then drops it from the
    // file and loses nothing is `legacy_server_host_key_still_loads`.
    // -----------------------------------------------------------------

    #[test]
    fn a_webhook_secret_in_config_toml_is_filed_when_it_is_loaded() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        write_config_holding(&path, SECRET);
        let before = read(&path);

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        assert_eq!(
            loaded.server.webhook_secret.as_deref(),
            Some(SECRET),
            "it takes effect"
        );
        assert_eq!(stored(&store).as_deref(), Some(SECRET), "and is filed");
        assert_eq!(read(&path), before, "loading does not rewrite config.toml");
    }

    #[test]
    fn a_reload_files_it_too() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        write_config_holding(&path, SECRET);

        let loaded =
            Config::load_from_replacing_with(&path, "http://localhost:11434", &store, no_env)
                .expect("loads");

        assert_eq!(loaded.server.webhook_secret.as_deref(), Some(SECRET));
        assert_eq!(stored(&store).as_deref(), Some(SECRET));
    }

    #[test]
    fn a_webhook_secret_written_into_config_toml_replaces_the_stored_one() {
        // The file's secret is the one every load of it runs with; were the
        // stored one kept, the save that strips the file would switch
        // secrets, and every caller configured with the file's would be
        // refused.
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        store
            .set(keys::SERVER_WEBHOOK_SECRET, "old-secret")
            .expect("set");
        write_config_holding(&path, "new-secret");

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        assert_eq!(loaded.server.webhook_secret.as_deref(), Some("new-secret"));
        assert_eq!(stored(&store).as_deref(), Some("new-secret"));
    }

    #[test]
    fn a_padded_webhook_secret_runs_as_it_is_filed() {
        // The store holds it trimmed, so this process must run with it
        // trimmed too: otherwise a caller that works now is refused after the
        // next save, for no change anyone made.
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        write_config_holding(&path, &format!("  {SECRET} "));

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        assert_eq!(loaded.server.webhook_secret.as_deref(), Some(SECRET));
        assert_eq!(stored(&store).as_deref(), Some(SECRET));
    }

    #[test]
    fn a_blank_webhook_secret_in_config_toml_is_not_filed() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        store
            .set(keys::SERVER_WEBHOOK_SECRET, "stored-secret")
            .expect("set");
        write_config_holding(&path, "  ");

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        assert_eq!(stored(&store).as_deref(), Some("stored-secret"));
        assert_eq!(
            loaded.server.webhook_secret.as_deref(),
            Some("stored-secret"),
            "a blank one is none"
        );
    }

    #[test]
    fn loading_never_files_the_webhook_secret_from_the_environment() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[server]\nport = 4100\n").expect("write");

        let loaded = Config::load_from_with(&path, &store, env_with("env-secret")).expect("loads");

        assert_eq!(loaded.server.webhook_secret.as_deref(), Some("env-secret"));
        assert_eq!(stored(&store), None);
    }
}
