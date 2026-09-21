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

/// The dotted path of the Ollama token, which is brought in by a change of
/// server as well as of value.
const OLLAMA_TOKEN: &str = "llm.ollama_api_key";

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
        self.brought_in_secrets(previous)
            .is_none_or(|brought_in| !brought_in.is_empty())
    }

    /// Each secret this config holds that `previous` does not, by its dotted
    /// path ([`Self::brings_in_secrets`]); `None` when either config cannot
    /// be serialized.
    fn brought_in_secrets(&self, previous: &Self) -> Option<BTreeMap<String, Value>> {
        let token_moved =
            ollama_server_changed(&previous.memory.ollama_host, &self.memory.ollama_host);
        let before = previous.held_secrets()?;
        let mut held = self.held_secrets()?;
        held.retain(|path, secret| {
            (token_moved && path == OLLAMA_TOKEN) || before.get(path) != Some(secret)
        });
        Some(held)
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

    /// File in `store` the secrets this config brings in over `previous`
    /// ([`Self::brings_in_secrets`]), as [`Self::file_secrets_in`] files
    /// them, and no other: this config is left as it is.
    ///
    /// For a change of a running config that is then saved (the daemon's
    /// `config.set`, reset and import). Besides the secrets entered with the
    /// change, a running config holds ones the environment supplied at load,
    /// which are never filed: filed, one outlives its variable — unset or
    /// rotated, and the next start runs on the stale copy — and a secret only
    /// ever exported is on disk. The rest it holds are filed already.
    ///
    /// A config that cannot be serialized, and so cannot say which it brings
    /// in, files every secret it holds: an unneeded filing costs a store
    /// write, a missed one costs the secret.
    ///
    /// # Errors
    ///
    /// As [`Self::file_secrets_in`].
    pub fn file_secrets_brought_in(
        &self,
        previous: &Self,
        store: &SecureStore,
    ) -> Result<(), CredentialError> {
        self.brought_in_secrets(previous)
            .and_then(|brought_in| self.holding_only(&brought_in))
            .as_ref()
            .unwrap_or(self)
            .file_secrets_in(store)
    }

    /// This config holding, of its secrets, only `kept` (dotted path to
    /// value, as [`Self::held_secrets`] reads them off): the copy a save
    /// writes, with each of those put back. `None` when the copy cannot be
    /// serialized, or a path is not in it.
    fn holding_only(&self, kept: &BTreeMap<String, Value>) -> Option<Self> {
        let mut stripped = self.clone();
        stripped.strip_secrets_for_disk();
        let mut holding = serde_json::to_value(&stripped).ok()?;
        for (path, secret) in kept {
            // A secret the save blanks to `None` is left out of the copy, so
            // it goes back in by its parent.
            let (parent, key) = path.rsplit_once('.').unwrap_or(("", path));
            parent
                .split('.')
                .filter(|part| !part.is_empty())
                .try_fold(&mut holding, |node, part| node.get_mut(part))?
                .as_object_mut()?
                .insert(key.to_string(), secret.clone());
        }
        serde_json::from_value(holding).ok()
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

    /// A change files what it brings in and nothing else the config holds —
    /// in a running config, those include what the environment supplied,
    /// which is never filed.
    #[test]
    fn a_change_files_only_the_secrets_it_brings_in() {
        let (_dir, store) = store();
        let previous = holding_every_secret();
        let mut changed = previous.clone();
        changed.llm.github_token = Some("ghp-rotated".to_string());
        if let Some(telegram) = &mut changed.channels.telegram {
            telegram.bot_token = "telegram-bot-rotated".to_string();
        }
        let held = changed.held_secrets().expect("serializes");

        changed
            .file_secrets_brought_in(&previous, &store)
            .expect("filed");

        assert_eq!(
            store.list_keys(),
            [keys::GITHUB_TOKEN, keys::TELEGRAM_BOT_TOKEN],
            "only what the change brings in"
        );
        assert_eq!(
            store.get(keys::GITHUB_TOKEN).ok().as_deref(),
            Some("ghp-rotated")
        );
        assert_eq!(
            store.get(keys::TELEGRAM_BOT_TOKEN).ok().as_deref(),
            Some("telegram-bot-rotated")
        );
        assert!(!store.exists(keys::OLLAMA_API_KEY_HOST));
        assert_eq!(
            changed.held_secrets().expect("serializes"),
            held,
            "the config keeps what it holds"
        );
    }

    /// The Ollama token is brought in by a move to another server, and is
    /// filed for that one.
    #[test]
    fn the_ollama_token_a_move_brings_in_is_filed_for_the_new_server() {
        let (_dir, store) = store();
        let mut previous = Config::default();
        previous.memory.ollama_host = "https://a.example/ollama".to_string();
        previous.llm.ollama_api_key = Some("token".to_string());
        previous.tools.brave_api_key = Some("brave-held".to_string());
        let mut moved = previous.clone();
        moved.memory.ollama_host = "https://b.example/ollama".to_string();

        moved
            .file_secrets_brought_in(&previous, &store)
            .expect("filed");

        assert_eq!(store.list_keys(), [keys::OLLAMA_API_KEY]);
        assert_eq!(store.ollama_token().as_deref(), Some("token"));
        assert_eq!(
            store
                .ollama_token_host()
                .expect("a file store answers")
                .as_deref(),
            Some("https://b.example/ollama")
        );
    }

    /// Each secret can be filed alone. One that could not would make the
    /// filing of a change fall back to every secret the config holds.
    #[test]
    fn each_secret_can_be_held_alone() {
        use std::collections::BTreeMap;

        let every = holding_every_secret();
        let held = every.held_secrets().expect("serializes");
        for (path, secret) in &held {
            let alone = BTreeMap::from([(path.clone(), secret.clone())]);
            let holding = every
                .holding_only(&alone)
                .unwrap_or_else(|| panic!("{path} is put back"));
            assert_eq!(holding.held_secrets(), Some(alone), "{path}");
            assert_eq!(
                holding.memory.ollama_host, every.memory.ollama_host,
                "{path}: the rest of the config is kept"
            );
        }
        let none = every.holding_only(&BTreeMap::new()).expect("serializes");
        assert_eq!(none.held_secrets(), Some(BTreeMap::new()));
    }
}
