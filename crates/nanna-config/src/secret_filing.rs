//! Filing the secrets a change of the running config brings in, before the
//! save that strips them from `config.toml`.
//!
//! [`Config::save_to`] never writes a secret: the secure store is their only
//! durable home. A process that changes its running config in memory and then
//! saves it (the daemon's `config.set`, reset and import) has to file what
//! the change brought in first. Filed nowhere, a secret works until the next
//! load — a restart, or the daemon's config watcher reading the save back —
//! and is gone.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::credentials::{CredentialError, SecureStore};
use crate::{Config, ollama_server_changed, same_ollama_server};

impl Config {
    /// Whether this config holds a secret `previous` does not: a new one, a
    /// new value for one, or the Ollama token for another server. That is
    /// what a change from `previous` brings in, and what the store must hold
    /// before a save strips it. A secret the change drops is not brought in.
    ///
    /// A secret is whatever [`Self::strip_secrets_for_disk`] blanks, read off
    /// by difference, so one added there is covered here with no list of its
    /// own. A config that cannot be serialized counts as bringing one in: an
    /// unneeded filing costs a store write, a missed one costs the secret.
    #[must_use]
    pub fn brings_in_secrets(&self, previous: &Self) -> bool {
        let token_moved = is_set(self.llm.ollama_api_key.as_deref())
            && ollama_server_changed(&previous.memory.ollama_host, &self.memory.ollama_host);
        if token_moved {
            return true;
        }
        let (Some(held), Some(before)) = (self.held_secrets(), previous.held_secrets()) else {
            return true;
        };
        held.iter()
            .any(|(path, secret)| before.get(path) != Some(secret))
    }

    /// File every secret this config holds in `store`, and leave this config
    /// holding them: [`Self::migrate_secrets_to_keyring`]'s filing, done on a
    /// copy — except that the Ollama token is filed for this config's server
    /// even when the store holds the same token for another.
    ///
    /// For a config that is the authority on its own address, so that the
    /// token it holds is that server's: the daemon's running configuration,
    /// where a change of address drops the token or names one of its own. A
    /// copy that may be stale about the address (the GUI's) goes through
    /// [`Self::store_secrets`], which leaves a stored token filed where it is.
    /// That one also refills from the environment first, which would swap a
    /// key just set for the environment's; this leaves the config alone.
    ///
    /// # Errors
    ///
    /// The first [`SecureStore`] write that fails; the secrets filed before
    /// it stay filed.
    pub fn file_secrets_in(&self, store: &SecureStore) -> Result<(), CredentialError> {
        let mut filing = self.clone();
        if let Some(token) = filing.llm.ollama_api_key.take() {
            let token = token.trim();
            let host = &self.memory.ollama_host;
            if !token.is_empty() && !holds_token_for(store, token, host) {
                store.save_ollama_token(token, host)?;
            }
        }
        filing.migrate_secrets_to(store)
    }

    /// Every secret this config holds, blank ones aside, by its dotted path;
    /// `None` when the config cannot be serialized.
    fn held_secrets(&self) -> Option<BTreeMap<String, Value>> {
        let held = serde_json::to_value(self).ok()?;
        let mut stripped = self.clone();
        stripped.strip_secrets_for_disk();
        let stripped = serde_json::to_value(&stripped).ok()?;
        let mut secrets = BTreeMap::new();
        collect_secrets(&held, &stripped, "", &mut secrets);
        Some(secrets)
    }
}

/// Whether `secret` is set and not blank.
fn is_set(secret: Option<&str>) -> bool {
    secret.is_some_and(|secret| !secret.trim().is_empty())
}

/// Whether `store` holds `token` for the server at `host`. A record that
/// cannot be read holds it for none.
fn holds_token_for(store: &SecureStore, token: &str, host: &str) -> bool {
    store.ollama_token().as_deref() == Some(token)
        && store
            .ollama_token_host()
            .ok()
            .flatten()
            .is_some_and(|bound| same_ollama_server(&bound, host))
}

/// Add to `secrets`, under its dotted path below `path`, each value `held`
/// has that `stripped` (the same config, its secrets blanked) lacks or has
/// otherwise. Blank values are no secret.
fn collect_secrets(
    held: &Value,
    stripped: &Value,
    path: &str,
    secrets: &mut BTreeMap<String, Value>,
) {
    if held == stripped {
        return;
    }
    let (Value::Object(held), Value::Object(stripped)) = (held, stripped) else {
        if !is_blank(held) {
            secrets.insert(path.to_string(), held.clone());
        }
        return;
    };
    for (key, value) in held {
        let path = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        match stripped.get(key) {
            Some(blanked) => collect_secrets(value, blanked, &path, secrets),
            None if !is_blank(value) => {
                secrets.insert(path, value.clone());
            }
            None => {}
        }
    }
}

fn is_blank(value: &Value) -> bool {
    value.is_null() || value.as_str().is_some_and(|text| text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use crate::Config;
    use crate::credentials::{SecureStore, keys};

    /// A hermetic store: its own directory, never the OS keyring.
    fn store() -> (tempfile::TempDir, SecureStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SecureStore::file_only_at(dir.path().join("store"));
        (dir, store)
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    /// One of each kind of secret [`Config::strip_secrets_for_disk`] blanks:
    /// every `[llm]` key, the search key, the webhook's, and a channel's.
    fn holding_every_secret() -> Config {
        let mut config = Config::default();
        config.memory.ollama_host = "https://gpu.example/ollama".to_string();
        config.llm.api_key = Some("sk-ant-held".to_string());
        config.llm.openai_api_key = Some("sk-openai-held".to_string());
        config.llm.openrouter_api_key = Some("sk-or-held".to_string());
        config.llm.github_token = Some("ghp-held".to_string());
        config.llm.anthropic_oauth_token = Some("oauth-held".to_string());
        config.llm.ollama_api_key = Some("ollama-held".to_string());
        config.tools.brave_api_key = Some("brave-held".to_string());
        config.server.webhook_secret = Some("webhook-held".to_string());
        config.channels.telegram = Some(crate::TelegramConfig {
            bot_token: "telegram-bot-held".to_string(),
            webhook_url: None,
            allowed_users: None,
            webhook_secret: Some("telegram-webhook-held".to_string()),
        });
        config
    }

    #[test]
    fn a_change_brings_in_the_secrets_it_adds_or_changes_and_no_others() {
        let before = Config::default();
        let mut after = before.clone();
        assert!(!after.brings_in_secrets(&before), "no change");
        after.llm.model = "nanna-test-other-model".to_string();
        assert!(!after.brings_in_secrets(&before), "a change of no secret");
        after.tools.brave_api_key = Some("  ".to_string());
        assert!(
            !after.brings_in_secrets(&before),
            "a blank one is no secret"
        );
        after.tools.brave_api_key = Some("brave".to_string());
        assert!(after.brings_in_secrets(&before), "a new secret");

        let before = after.clone();
        after.tools.brave_api_key = Some("brave-rotated".to_string());
        assert!(after.brings_in_secrets(&before), "a new value for one");
        after.tools.brave_api_key = None;
        assert!(!after.brings_in_secrets(&before), "a secret dropped");
    }

    #[test]
    fn every_secret_the_save_strips_is_one_a_change_can_bring_in() {
        let every = holding_every_secret();
        let mut stripped = every.clone();
        stripped.strip_secrets_for_disk();
        assert!(every.brings_in_secrets(&stripped));
        let held = every.held_secrets().expect("serializes");
        assert_eq!(
            held.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "channels.telegram.bot_token",
                "channels.telegram.webhook_secret",
                "llm.anthropic_oauth_token",
                "llm.api_key",
                "llm.github_token",
                "llm.ollama_api_key",
                "llm.openai_api_key",
                "llm.openrouter_api_key",
                "server.webhook_secret",
                "tools.brave_api_key",
            ],
            "read off what the save strips, and nothing else"
        );
    }

    #[test]
    fn the_ollama_token_for_another_server_is_brought_in() {
        let mut before = Config::default();
        before.memory.ollama_host = "https://a.example/ollama".to_string();
        before.llm.ollama_api_key = Some("token".to_string());
        let mut after = before.clone();
        after.memory.ollama_host = "https://a.example/ollama/".to_string();
        assert!(!after.brings_in_secrets(&before), "the same server");
        after.memory.ollama_host = "https://b.example/ollama".to_string();
        assert!(
            after.brings_in_secrets(&before),
            "the same token, for another server"
        );
        after.llm.ollama_api_key = None;
        assert!(
            !after.brings_in_secrets(&before),
            "the token dropped with the move"
        );
    }

    /// The round trip the daemon makes: file, save, and the next load (a
    /// restart, or the config watcher) has every secret back.
    #[test]
    fn filed_then_saved_the_next_load_has_every_secret() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        let running = holding_every_secret();
        let held = running.held_secrets().expect("serializes");

        running.file_secrets_in(&store).expect("filed");
        assert_eq!(
            running.held_secrets().expect("serializes"),
            held,
            "the running config keeps what it holds"
        );
        running.save_to(&path).expect("saved");
        let saved = std::fs::read_to_string(&path).expect("config.toml");
        for secret in held.values().filter_map(serde_json::Value::as_str) {
            assert!(!saved.contains(secret), "{secret} is not in config.toml");
        }

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");
        assert_eq!(loaded.held_secrets().expect("serializes"), held);
    }

    #[test]
    fn the_ollama_token_is_filed_for_this_configs_server() {
        // The store holds the same token for another server. Taken for
        // "already filed", it stayed that server's, and the next load
        // withheld it from this one.
        let (_dir, store) = store();
        store
            .save_ollama_token("token", "https://a.example/ollama")
            .expect("save");
        let mut config = Config::default();
        config.memory.ollama_host = "https://b.example/ollama".to_string();
        config.llm.ollama_api_key = Some(" token ".to_string());

        config.file_secrets_in(&store).expect("filed");

        assert_eq!(store.ollama_token().as_deref(), Some("token"));
        assert_eq!(
            store
                .ollama_token_host()
                .expect("a file store answers")
                .as_deref(),
            Some("https://b.example/ollama")
        );
        assert_eq!(config.llm.ollama_api_key.as_deref(), Some(" token "));
    }

    #[test]
    fn a_config_holding_no_secret_files_none() {
        let (_dir, store) = store();
        Config::default()
            .file_secrets_in(&store)
            .expect("nothing to file");
        assert!(store.list_keys().is_empty(), "{:?}", store.list_keys());
        assert!(!store.exists(keys::OLLAMA_API_KEY_HOST));
    }
}
