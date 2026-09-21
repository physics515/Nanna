//! Forgetting a secret someone clears, and keeping the ones a change of the
//! running config merely lacks.
//!
//! The other half of `secret_filing`. [`Config::save_to`] writes no secret,
//! and every load fills each unset one from the environment, else the secure
//! store. So what a running config holds is only half of what the next load
//! gives it, and a change made in memory alone is undone by that load — a
//! restart, or the daemon's config watcher reading its own save back seconds
//! later:
//!
//! - **A secret someone clears** is still in the store, and the next load
//!   has it again. [`Config::forget_secret`] deletes it there too.
//! - **A secret a change merely lacks** — a reset to defaults, an import of
//!   an export that carries none, a `config.set` of a whole section — is no
//!   one clearing it. The store keeps it, and
//!   [`Config::refill_secrets_replacing`] gives it back to the running config
//!   as the next load will, instead of the running config going without it
//!   until then.

use serde_json::Value;

use crate::channel_secrets;
use crate::credentials::{CredentialError, SecureStore, keys};
use crate::{Config, UnboundOllamaToken, process_env, same_ollama_server};

/// A secret outside the channels, other than the Ollama token.
struct Secret {
    /// Its dotted path in the config.
    path: &'static str,
    /// Its field.
    slot: fn(&mut Config) -> &mut Option<String>,
    /// Deletes it from the secure store.
    delete: fn(&SecureStore) -> Result<(), CredentialError>,
}

/// Every secret [`Config::strip_secrets_for_disk`] blanks, but the channels'
/// ([`channel_secrets`] has those) and the Ollama token ([`OLLAMA_TOKEN`]).
const SECRETS: [Secret; 7] = [
    Secret {
        path: "llm.api_key",
        slot: |config| &mut config.llm.api_key,
        delete: |store| forget(store, keys::ANTHROPIC_API_KEY),
    },
    Secret {
        path: "llm.openai_api_key",
        slot: |config| &mut config.llm.openai_api_key,
        delete: |store| forget(store, keys::OPENAI_API_KEY),
    },
    Secret {
        path: "llm.openrouter_api_key",
        slot: |config| &mut config.llm.openrouter_api_key,
        delete: |store| forget(store, keys::OPENROUTER_API_KEY),
    },
    Secret {
        path: "llm.github_token",
        slot: |config| &mut config.llm.github_token,
        delete: |store| forget(store, keys::GITHUB_TOKEN),
    },
    Secret {
        path: "llm.anthropic_oauth_token",
        slot: |config| &mut config.llm.anthropic_oauth_token,
        // The bare token a load fills this from, and the login it is the
        // access token of: kept in lockstep (`save_anthropic_oauth`), and the
        // router signs in with the login. Forgetting one alone forgets
        // neither.
        delete: SecureStore::delete_anthropic_oauth,
    },
    Secret {
        path: "tools.brave_api_key",
        slot: |config| &mut config.tools.brave_api_key,
        delete: |store| forget(store, keys::BRAVE_API_KEY),
    },
    Secret {
        path: "server.webhook_secret",
        slot: |config| &mut config.server.webhook_secret,
        delete: |store| forget(store, keys::SERVER_WEBHOOK_SECRET),
    },
];

/// The Ollama token's path. It is filed with the server it was saved for
/// (`SecureStore::save_ollama_token`), so it is forgotten by server, not by
/// key alone.
const OLLAMA_TOKEN: &str = "llm.ollama_api_key";

impl Config {
    /// Whether dotted `path` names a secret: a field
    /// [`Self::strip_secrets_for_disk`] blanks.
    #[must_use]
    pub fn names_a_secret(path: &str) -> bool {
        secret_paths().any(|secret| secret == path)
    }

    /// Whether this config holds the secret at dotted `path`: `path` names a
    /// secret, and it is set, not blank.
    #[must_use]
    pub fn holds_secret(&self, path: &str) -> bool {
        Self::names_a_secret(path)
            && serde_json::to_value(self).is_ok_and(|config| holds(&config, path))
    }

    /// Forget the secret at dotted `path`, which someone cleared: unset it
    /// here, as a save leaves it, and delete it from `store`, so that the
    /// next load does not bring it back. A `path` that names no secret
    /// changes nothing.
    ///
    /// The Ollama token is deleted only when it is this config's server's:
    /// filed for that server, or for none (an older build's, which a load
    /// gives the server the config names — a move of the address since would
    /// have recorded the server it was going to). Another server's token is
    /// not this one's to clear, and stays filed for it.
    ///
    /// Only this store is cleared: a secret the environment supplies is
    /// supplied again by the next load, and is the operator's to unset.
    ///
    /// # Errors
    ///
    /// The store's, when it cannot delete the secret, or cannot say whether
    /// the saved Ollama token is this server's. The secret may then still be
    /// filed; this config is left unset either way.
    pub fn forget_secret(
        &mut self,
        path: &str,
        store: &SecureStore,
    ) -> Result<(), CredentialError> {
        if path == OLLAMA_TOKEN {
            self.llm.ollama_api_key = None;
            return forget_ollama_token(store, &self.memory.ollama_host);
        }
        if let Some(secret) = SECRETS.iter().find(|secret| secret.path == path) {
            *(secret.slot)(self) = None;
            return (secret.delete)(store);
        }
        channel_secrets::unset(&mut self.channels, path).map_or(Ok(()), |key| forget(store, key))
    }

    /// Whether `previous` holds a secret this config holds none of: what a
    /// change from `previous` drops. A blank one is none; a new value for one
    /// is brought in, not dropped (`Self::brings_in_secrets`).
    ///
    /// A config that cannot be serialized counts as dropping one: an
    /// unneeded refill costs a store read, a missed one a credential.
    #[must_use]
    pub fn drops_secrets(&self, previous: &Self) -> bool {
        let (Ok(now), Ok(before)) = (serde_json::to_value(self), serde_json::to_value(previous))
        else {
            return true;
        };
        secret_paths().any(|path| holds(&before, path) && !holds(&now, path))
    }

    /// Fill every unset secret from the environment, else `store`, as the
    /// load that replaces a configuration running with Ollama server
    /// `running_ollama_host` fills them ([`Self::load_replacing`]). A secret
    /// this config holds is kept.
    ///
    /// For a config changed in memory that drops secrets
    /// ([`Self::drops_secrets`]): it then runs with what the next load of its
    /// save gives, rather than without them until that load. A stored Ollama
    /// token with no recorded server was the running server's, and is
    /// recorded as such, as the load records it.
    pub fn refill_secrets_replacing(&mut self, running_ollama_host: &str, store: &SecureStore) {
        self.refill_secrets_replacing_with(running_ollama_host, store, process_env);
    }

    /// [`Self::refill_secrets_replacing`] against a given environment: the
    /// one a process loads with, when it is not the process environment (a
    /// test's daemon control plane).
    pub fn refill_secrets_replacing_with(
        &mut self,
        running_ollama_host: &str,
        store: &SecureStore,
        env: impl Fn(&str) -> Option<String>,
    ) {
        self.load_secrets_with(
            store,
            &env,
            UnboundOllamaToken::RunningServer(running_ollama_host),
        );
    }
}

/// The dotted path of every secret a save strips.
fn secret_paths() -> impl Iterator<Item = &'static str> {
    SECRETS
        .iter()
        .map(|secret| secret.path)
        .chain([OLLAMA_TOKEN])
        .chain(channel_secrets::fields())
}

/// Whether `config` holds a secret, not a blank one, at dotted `path`.
fn holds(config: &Value, path: &str) -> bool {
    path.split('.')
        .try_fold(config, |node, part| node.get(part))
        .and_then(Value::as_str)
        .is_some_and(|secret| !secret.trim().is_empty())
}

/// Delete `key` from `store`; one it does not hold is already forgotten.
fn forget(store: &SecureStore, key: &str) -> Result<(), CredentialError> {
    match store.delete(key) {
        Ok(()) | Err(CredentialError::NotFound) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Delete the saved Ollama token when it is the server at `host`'s: filed
/// for it, or for no server.
fn forget_ollama_token(store: &SecureStore, host: &str) -> Result<(), CredentialError> {
    if store.lookup(keys::OLLAMA_API_KEY)?.is_none() {
        return Ok(());
    }
    let this_servers = store
        .ollama_token_host()?
        .is_none_or(|bound| same_ollama_server(&bound, host));
    if this_servers {
        store.delete_ollama_token()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::secret_paths;
    use crate::credentials::{OAuthCredential, SecureStore, keys};
    use crate::{
        ChannelsConfig, Config, DiscordConfig, SignalConfig, SlackConfig, TelegramConfig,
        WhatsAppConfig,
    };

    const HOST: &str = "https://gpu.example/ollama";

    /// A hermetic store: its own directory, never the OS keyring.
    fn fresh_store() -> (tempfile::TempDir, SecureStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SecureStore::file_only_at(dir.path().join("store"));
        (dir, store)
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    /// Every secret [`Config::strip_secrets_for_disk`] blanks, each set.
    fn holding_every_secret() -> Config {
        let mut config = Config::default();
        config.memory.ollama_host = HOST.to_string();
        config.llm.api_key = Some("sk-ant-held".to_string());
        config.llm.openai_api_key = Some("sk-openai-held".to_string());
        config.llm.openrouter_api_key = Some("sk-or-held".to_string());
        config.llm.github_token = Some("ghp-held".to_string());
        config.llm.anthropic_oauth_token = Some("oauth-held".to_string());
        config.llm.ollama_api_key = Some("ollama-held".to_string());
        config.tools.brave_api_key = Some("brave-held".to_string());
        config.server.webhook_secret = Some("webhook-held".to_string());
        config.channels = ChannelsConfig {
            telegram: Some(TelegramConfig {
                bot_token: "tg-bot-held".into(),
                webhook_url: None,
                allowed_users: Some(vec![42]),
                webhook_secret: Some("tg-webhook-held".into()),
            }),
            discord: Some(DiscordConfig {
                bot_token: "dc-bot-held".into(),
                application_id: "dc-application-id".into(),
                public_key: "dc-public-key".into(),
            }),
            slack: Some(SlackConfig {
                bot_token: "xoxb-held".into(),
                app_token: Some("xapp-held".into()),
                signing_secret: "slack-signing-held".into(),
            }),
            signal: Some(SignalConfig {
                webhook_secret: Some("signal-webhook-held".into()),
                phone_number: "+15550100".into(),
                api_url: None,
                allowed_numbers: None,
            }),
            whatsapp: Some(WhatsAppConfig {
                connection_method: "cloud-api".into(),
                phone_number_id: None,
                access_token: Some("wa-access-held".into()),
                verify_token: Some("wa-verify-held".into()),
                app_secret: Some("wa-app-held".into()),
                session_name: None,
                allowed_contacts: None,
            }),
        };
        config
    }

    fn json(config: &Config) -> serde_json::Value {
        serde_json::to_value(config).expect("json")
    }

    /// Forgetting every secret path leaves the config as a save leaves it,
    /// and the store holding none of them: no secret the save strips is
    /// missing from the paths, and none of the paths is anything else.
    #[test]
    fn every_secret_a_save_strips_can_be_forgotten() {
        let (_dir, store) = fresh_store();
        let every = holding_every_secret();
        every.file_secrets_in(&store).expect("filed");
        store
            .save_anthropic_oauth(&OAuthCredential {
                access_token: "oauth-held".to_string(),
                refresh_token: Some("oauth-refresh-held".to_string()),
                expires_at: None,
                subscription_type: None,
                account_id: None,
                organization_id: None,
            })
            .expect("login saved");
        assert!(!store.list_keys().is_empty(), "the store holds them first");

        let mut forgotten = every.clone();
        for path in secret_paths() {
            assert!(Config::names_a_secret(path), "{path}");
            forgotten.forget_secret(path, &store).expect("forgotten");
        }

        let mut stripped = every;
        stripped.strip_secrets_for_disk();
        assert_eq!(json(&forgotten), json(&stripped));
        assert!(store.list_keys().is_empty(), "{:?}", store.list_keys());
        assert!(!store.exists(keys::OLLAMA_API_KEY_HOST));
    }

    #[test]
    fn a_path_that_names_no_secret_is_not_forgotten() {
        let (_dir, store) = fresh_store();
        store.set(keys::BRAVE_API_KEY, "brave-stored").expect("set");
        let mut config = holding_every_secret();
        let before = json(&config);
        for path in [
            "llm.model",
            "tools",
            "channels.discord.public_key",
            "llm.api_key.x",
            "",
        ] {
            assert!(!Config::names_a_secret(path), "{path}");
            config
                .forget_secret(path, &store)
                .expect("nothing to forget");
        }
        assert_eq!(json(&config), before);
        assert_eq!(
            store.get(keys::BRAVE_API_KEY).ok().as_deref(),
            Some("brave-stored")
        );
    }

    /// The store holds one Ollama token, filed with the server it was saved
    /// for. Clearing the token clears this server's.
    #[test]
    fn the_ollama_token_forgotten_is_this_servers() {
        let clear = |store: &SecureStore| {
            let mut config = Config::default();
            config.memory.ollama_host = HOST.to_string();
            config.llm.ollama_api_key = Some("held".to_string());
            config
                .forget_secret("llm.ollama_api_key", store)
                .expect("forgotten");
            assert_eq!(config.llm.ollama_api_key, None, "unset here either way");
        };

        let (_dir, store) = fresh_store();
        store
            .save_ollama_token("token", "https://gpu.example/ollama/")
            .expect("save");
        clear(&store);
        assert_eq!(store.ollama_token(), None, "filed for this server");
        assert!(!store.exists(keys::OLLAMA_API_KEY_HOST));

        let (_dir, store) = fresh_store();
        store.set(keys::OLLAMA_API_KEY, "legacy").expect("set");
        clear(&store);
        assert_eq!(store.ollama_token(), None, "filed for no server");

        let (_dir, store) = fresh_store();
        store
            .save_ollama_token("token", "https://other.example/ollama")
            .expect("save");
        clear(&store);
        assert_eq!(
            store.ollama_token().as_deref(),
            Some("token"),
            "another server's"
        );
        assert_eq!(
            store.ollama_token_host().expect("answers").as_deref(),
            Some("https://other.example/ollama")
        );
    }

    /// A store that cannot delete says so: the secret may still be filed.
    #[test]
    fn a_secret_the_store_cannot_delete_is_an_error() {
        let (dir, store) = fresh_store();
        store.set(keys::BRAVE_API_KEY, "brave-stored").expect("set");
        std::fs::write(
            dir.path().join("store").join("credentials.enc"),
            b"not a store",
        )
        .expect("write");
        let mut config = holding_every_secret();
        assert!(config.forget_secret("tools.brave_api_key", &store).is_err());
        assert!(config.forget_secret("llm.ollama_api_key", &store).is_err());
    }

    #[test]
    fn a_change_drops_the_secrets_it_holds_none_of() {
        let before = holding_every_secret();
        let mut after = before.clone();
        assert!(!after.drops_secrets(&before), "no change");
        after.llm.model = "nanna-test-other-model".to_string();
        assert!(!after.drops_secrets(&before), "a change of no secret");
        after.tools.brave_api_key = Some("brave-rotated".to_string());
        assert!(!after.drops_secrets(&before), "a new value is brought in");
        after.tools.brave_api_key = Some("  ".to_string());
        assert!(after.drops_secrets(&before), "blanked");
        after.tools.brave_api_key = None;
        assert!(after.drops_secrets(&before), "unset");

        let mut after = before.clone();
        after.channels.slack = None;
        assert!(after.drops_secrets(&before), "with its channel");
        assert!(
            !before.drops_secrets(&Config::default()),
            "none held before, none dropped"
        );
        assert!(
            Config::default().drops_secrets(&before),
            "a reset drops them all"
        );
    }

    /// Refilled, a config holds what the next load of its save would: the
    /// environment's secret, else the store's, and a stored Ollama token
    /// only for the server it is filed for.
    #[test]
    fn refilled_a_config_holds_what_the_next_load_gives() {
        let (dir, store) = fresh_store();
        store
            .set(keys::OPENROUTER_API_KEY, "sk-or-stored")
            .expect("set");
        store.set(keys::BRAVE_API_KEY, "brave-stored").expect("set");
        store
            .save_ollama_token("ollama-stored", HOST)
            .expect("save");
        let env = |name: &str| (name == "BRAVE_API_KEY").then(|| "brave-from-env".to_string());

        let mut config = Config::default();
        config.memory.ollama_host = HOST.to_string();
        config.llm.github_token = Some("ghp-held".to_string());
        config.refill_secrets_replacing_with(HOST, &store, env);

        assert_eq!(
            config.llm.openrouter_api_key.as_deref(),
            Some("sk-or-stored")
        );
        assert_eq!(
            config.tools.brave_api_key.as_deref(),
            Some("brave-from-env")
        );
        assert_eq!(config.llm.ollama_api_key.as_deref(), Some("ollama-stored"));
        assert_eq!(config.llm.github_token.as_deref(), Some("ghp-held"), "kept");

        let path = dir.path().join("config.toml");
        config.save_to(&path).expect("save");
        let loaded = Config::load_from_replacing_with(&path, HOST, &store, env).expect("loads");
        assert_eq!(
            loaded.llm.openrouter_api_key, config.llm.openrouter_api_key,
            "as the load gives it"
        );
        assert_eq!(loaded.tools.brave_api_key, config.tools.brave_api_key);
        assert_eq!(loaded.llm.ollama_api_key, config.llm.ollama_api_key);

        // Another server's token is withheld from the default address.
        let mut reset = Config::default();
        reset.refill_secrets_replacing_with(HOST, &store, no_env);
        assert_eq!(reset.llm.ollama_api_key, None);
        assert_eq!(
            reset.llm.openrouter_api_key.as_deref(),
            Some("sk-or-stored")
        );
    }

    /// A token with no server recorded was the running server's: refilled
    /// for another address it is withheld, and recorded as the running one's.
    #[test]
    fn a_refill_records_an_unbound_token_as_the_running_servers() {
        let (_dir, store) = fresh_store();
        store.set(keys::OLLAMA_API_KEY, "legacy").expect("set");
        let mut reset = Config::default();
        reset.refill_secrets_replacing_with(HOST, &store, no_env);
        assert_eq!(reset.llm.ollama_api_key, None);
        assert_eq!(
            store.ollama_token_host().expect("answers").as_deref(),
            Some(HOST)
        );
    }
}
