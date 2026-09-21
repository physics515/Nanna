//! Channel secrets: bot and app tokens, signing and webhook secrets.
//!
//! Like the LLM keys they are held in the secure store and never written to
//! `config.toml`. A channel's section stays in the file, with its other
//! settings: it is what turns the channel on. So:
//!
//! - [`strip`] blanks them in the copy a save writes;
//! - [`file`] moves the ones held in memory (just entered) into the store;
//! - [`fill`] puts them back on load, into each channel the file names, from
//!   the environment or else the store. A stored secret never turns a channel on.
//!
//! **A `config.toml` that already holds one** — written by a build before this
//! one, or by hand — is migrated by [`adopt`] when the file is loaded: each
//! secret it holds is filed in the store, before anything could save the
//! config, so the save that next strips it from the file loses nothing. The
//! file itself is not rewritten by a load (every CLI command, the GUI and the
//! daemon load it, some at once, and a rewrite from the parsed config would
//! drop its comments); the secret leaves it at the next save of any kind, and
//! until then each load says it can be deleted.

use crate::ChannelsConfig;
use crate::credentials::{CredentialError, SecureStore, keys};
use tracing::{error, warn};

/// Where a channel's config holds one secret.
enum Slot<'a> {
    /// A field the channel's config always has; empty when unset.
    Text(&'a mut String),
    /// An optional field.
    Optional(&'a mut Option<String>),
}

impl Slot<'_> {
    /// The secret held; a blank one is none.
    fn held(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text.as_str()),
            Self::Optional(optional) => optional.as_deref(),
        }
        .filter(|secret| !secret.trim().is_empty())
    }

    /// Take the secret out, leaving the slot unset.
    fn take(&mut self) -> Option<String> {
        match self {
            Self::Text(text) => Some(std::mem::take(&mut **text)),
            Self::Optional(optional) => optional.take(),
        }
        .filter(|secret| !secret.trim().is_empty())
    }

    fn put(&mut self, secret: String) {
        match self {
            Self::Text(text) => **text = secret,
            Self::Optional(optional) => **optional = Some(secret),
        }
    }
}

/// One channel secret.
struct Secret {
    /// Its key in `config.toml`, for messages (never the value).
    field: &'static str,
    /// Its key in the secure store.
    key: &'static str,
    /// The environment variable that supplies it.
    env: &'static str,
    /// Its slot, when `config.toml` names the channel.
    slot: fn(&mut ChannelsConfig) -> Option<Slot<'_>>,
}

/// Every channel secret. Anything else in a channel's section is a setting,
/// and stays in `config.toml`: Discord's `public_key` is public, and the
/// application id, phone numbers and URLs are not secrets.
const SECRETS: [Secret; 10] = [
    Secret {
        field: "channels.telegram.bot_token",
        key: keys::TELEGRAM_BOT_TOKEN,
        env: "TELEGRAM_BOT_TOKEN",
        slot: |c| c.telegram.as_mut().map(|t| Slot::Text(&mut t.bot_token)),
    },
    Secret {
        field: "channels.telegram.webhook_secret",
        key: keys::TELEGRAM_WEBHOOK_SECRET,
        env: "TELEGRAM_WEBHOOK_SECRET",
        slot: |c| {
            c.telegram
                .as_mut()
                .map(|t| Slot::Optional(&mut t.webhook_secret))
        },
    },
    Secret {
        field: "channels.discord.bot_token",
        key: keys::DISCORD_BOT_TOKEN,
        env: "DISCORD_BOT_TOKEN",
        slot: |c| c.discord.as_mut().map(|d| Slot::Text(&mut d.bot_token)),
    },
    Secret {
        field: "channels.slack.bot_token",
        key: keys::SLACK_BOT_TOKEN,
        env: "SLACK_BOT_TOKEN",
        slot: |c| c.slack.as_mut().map(|s| Slot::Text(&mut s.bot_token)),
    },
    Secret {
        field: "channels.slack.app_token",
        key: keys::SLACK_APP_TOKEN,
        env: "SLACK_APP_TOKEN",
        slot: |c| c.slack.as_mut().map(|s| Slot::Optional(&mut s.app_token)),
    },
    Secret {
        field: "channels.slack.signing_secret",
        key: keys::SLACK_SIGNING_SECRET,
        env: "SLACK_SIGNING_SECRET",
        slot: |c| c.slack.as_mut().map(|s| Slot::Text(&mut s.signing_secret)),
    },
    Secret {
        field: "channels.signal.webhook_secret",
        key: keys::SIGNAL_WEBHOOK_SECRET,
        env: "SIGNAL_WEBHOOK_SECRET",
        slot: |c| {
            c.signal
                .as_mut()
                .map(|s| Slot::Optional(&mut s.webhook_secret))
        },
    },
    Secret {
        field: "channels.whatsapp.access_token",
        key: keys::WHATSAPP_ACCESS_TOKEN,
        env: "WHATSAPP_ACCESS_TOKEN",
        slot: |c| {
            c.whatsapp
                .as_mut()
                .map(|w| Slot::Optional(&mut w.access_token))
        },
    },
    Secret {
        field: "channels.whatsapp.verify_token",
        key: keys::WHATSAPP_VERIFY_TOKEN,
        env: "WHATSAPP_VERIFY_TOKEN",
        slot: |c| {
            c.whatsapp
                .as_mut()
                .map(|w| Slot::Optional(&mut w.verify_token))
        },
    },
    Secret {
        field: "channels.whatsapp.app_secret",
        key: keys::WHATSAPP_APP_SECRET,
        env: "WHATSAPP_APP_SECRET",
        slot: |c| {
            c.whatsapp
                .as_mut()
                .map(|w| Slot::Optional(&mut w.app_secret))
        },
    },
];

/// Blank every channel secret, in a config about to be written to disk.
pub fn strip(channels: &mut ChannelsConfig) {
    for secret in &SECRETS {
        if let Some(mut slot) = (secret.slot)(channels) {
            slot.take();
        }
    }
}

/// File every channel secret held in `channels` in `store`, trimmed, and take
/// it out of `channels`.
///
/// # Errors
///
/// The first [`SecureStore::set`] failure. As in
/// `Config::migrate_secrets_to`, each secret is taken out before its write, so
/// the failing one and those already filed are gone from `channels` and the
/// later ones are left in place.
pub fn file(channels: &mut ChannelsConfig, store: &SecureStore) -> Result<(), CredentialError> {
    for secret in &SECRETS {
        if let Some(held) = (secret.slot)(channels).and_then(|mut slot| slot.take()) {
            store.set(secret.key, held.trim())?;
        }
    }
    Ok(())
}

/// Fill each unset secret of every channel `channels` names from `env`, else
/// from `store`. A secret already held is kept, and a channel `channels` does
/// not name is neither created nor read for.
pub fn fill(
    channels: &mut ChannelsConfig,
    store: &SecureStore,
    env: &impl Fn(&str) -> Option<String>,
) {
    let set = |secret: &String| !secret.trim().is_empty();
    for secret in &SECRETS {
        let Some(mut slot) = (secret.slot)(channels) else {
            continue;
        };
        if slot.held().is_some() {
            continue;
        }
        if let Some(from_env) = env(secret.env).filter(set) {
            slot.put(from_env);
            continue;
        }
        match store.get(secret.key) {
            Ok(stored) if set(&stored) => slot.put(stored),
            Ok(_) | Err(CredentialError::NotFound) => {}
            Err(e) => warn!(
                "Cannot read {} from the secure store ({e}); the channel runs without it",
                secret.field
            ),
        }
    }
}

/// File in `store` each channel secret `channels` holds as parsed from
/// `config.toml` — so that the save which next strips it from the file loses
/// nothing. `channels` keeps it, and a stored secret it differs from is
/// replaced: every load of this file runs with the file's, so the one kept
/// must be the file's, or that save would switch the channel to another.
///
/// Call on the parsed file only, before anything is filled in: a secret from
/// the environment is never filed.
pub fn adopt(channels: &mut ChannelsConfig, store: &SecureStore) {
    for secret in &SECRETS {
        let Some(held) =
            (secret.slot)(channels).and_then(|slot| slot.held().map(|held| held.trim().to_owned()))
        else {
            continue;
        };
        let stored = store.get(secret.key);
        if stored.as_deref().is_ok_and(|stored| *stored == held) {
            warn!(
                "config.toml holds {} in plain text. It is in the secure store, so the \
                 line can be deleted; the next save of the settings removes it.",
                secret.field
            );
            continue;
        }
        match store.set(secret.key, &held) {
            Ok(()) => warn!(
                "config.toml holds {} in plain text. It is now filed in the secure store{}, \
                 so the line can be deleted; the next save of the settings removes it.",
                secret.field,
                if stored.is_ok() {
                    " in place of the one there"
                } else {
                    ""
                }
            ),
            Err(e) => error!(
                "config.toml holds {} in plain text and it cannot be filed in the secure \
                 store ({e}). This process runs with it; once the settings are next saved \
                 it is no longer in config.toml, and must be set again.",
                secret.field
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::credentials::{SecureStore, keys};
    use crate::{
        ChannelsConfig, Config, DiscordConfig, SignalConfig, SlackConfig, TelegramConfig,
        WhatsAppConfig,
    };
    use std::path::Path;

    /// Every channel secret: its store key, its environment variable, and the
    /// value [`every_channel`] gives it.
    const EVERY_SECRET: [(&str, &str, &str); 10] = [
        (
            keys::TELEGRAM_BOT_TOKEN,
            "TELEGRAM_BOT_TOKEN",
            "tg-bot-token",
        ),
        (
            keys::TELEGRAM_WEBHOOK_SECRET,
            "TELEGRAM_WEBHOOK_SECRET",
            "tg-webhook-secret",
        ),
        (keys::DISCORD_BOT_TOKEN, "DISCORD_BOT_TOKEN", "dc-bot-token"),
        (keys::SLACK_BOT_TOKEN, "SLACK_BOT_TOKEN", "xoxb-slack-bot"),
        (keys::SLACK_APP_TOKEN, "SLACK_APP_TOKEN", "xapp-slack-app"),
        (
            keys::SLACK_SIGNING_SECRET,
            "SLACK_SIGNING_SECRET",
            "slack-signing-secret",
        ),
        (
            keys::SIGNAL_WEBHOOK_SECRET,
            "SIGNAL_WEBHOOK_SECRET",
            "signal-webhook-secret",
        ),
        (
            keys::WHATSAPP_ACCESS_TOKEN,
            "WHATSAPP_ACCESS_TOKEN",
            "wa-access-token",
        ),
        (
            keys::WHATSAPP_VERIFY_TOKEN,
            "WHATSAPP_VERIFY_TOKEN",
            "wa-verify-token",
        ),
        (
            keys::WHATSAPP_APP_SECRET,
            "WHATSAPP_APP_SECRET",
            "wa-app-secret",
        ),
    ];

    /// Settings of [`every_channel`] that are not secrets.
    const EVERY_SETTING: [&str; 7] = [
        "https://example.test/webhook/telegram",
        "dc-application-id",
        "dc-public-key",
        "+15550100",
        "http://localhost:8080",
        "cloud-api",
        "wa-phone-number-id",
    ];

    /// Every channel, with every secret and setting set.
    fn every_channel() -> ChannelsConfig {
        ChannelsConfig {
            telegram: Some(TelegramConfig {
                bot_token: "tg-bot-token".into(),
                webhook_url: Some("https://example.test/webhook/telegram".into()),
                allowed_users: Some(vec![42]),
                webhook_secret: Some("tg-webhook-secret".into()),
            }),
            discord: Some(DiscordConfig {
                bot_token: "dc-bot-token".into(),
                application_id: "dc-application-id".into(),
                public_key: "dc-public-key".into(),
            }),
            slack: Some(SlackConfig {
                bot_token: "xoxb-slack-bot".into(),
                app_token: Some("xapp-slack-app".into()),
                signing_secret: "slack-signing-secret".into(),
            }),
            signal: Some(SignalConfig {
                webhook_secret: Some("signal-webhook-secret".into()),
                phone_number: "+15550100".into(),
                api_url: Some("http://localhost:8080".into()),
                allowed_numbers: None,
            }),
            whatsapp: Some(WhatsAppConfig {
                connection_method: "cloud-api".into(),
                phone_number_id: Some("wa-phone-number-id".into()),
                access_token: Some("wa-access-token".into()),
                verify_token: Some("wa-verify-token".into()),
                app_secret: Some("wa-app-secret".into()),
                session_name: None,
                allowed_contacts: None,
            }),
        }
    }

    fn with_every_channel() -> Config {
        Config {
            channels: every_channel(),
            ..Config::default()
        }
    }

    /// The secrets `channels` holds, in [`EVERY_SECRET`] order; a blank one
    /// counts as none. Read field by field, not through the module's table,
    /// so a secret the table forgets shows up here.
    fn held(channels: &ChannelsConfig) -> [Option<&str>; 10] {
        fn text(s: &str) -> Option<&str> {
            Some(s).filter(|s| !s.trim().is_empty())
        }
        fn optional(s: Option<&str>) -> Option<&str> {
            s.filter(|s| !s.trim().is_empty())
        }
        let telegram = channels.telegram.as_ref();
        let discord = channels.discord.as_ref();
        let slack = channels.slack.as_ref();
        let signal = channels.signal.as_ref();
        let whatsapp = channels.whatsapp.as_ref();
        [
            telegram.and_then(|t| text(&t.bot_token)),
            telegram.and_then(|t| optional(t.webhook_secret.as_deref())),
            discord.and_then(|d| text(&d.bot_token)),
            slack.and_then(|s| text(&s.bot_token)),
            slack.and_then(|s| optional(s.app_token.as_deref())),
            slack.and_then(|s| text(&s.signing_secret)),
            signal.and_then(|s| optional(s.webhook_secret.as_deref())),
            whatsapp.and_then(|w| optional(w.access_token.as_deref())),
            whatsapp.and_then(|w| optional(w.verify_token.as_deref())),
            whatsapp.and_then(|w| optional(w.app_secret.as_deref())),
        ]
    }

    /// What [`held`] reads from [`every_channel`].
    fn every_value() -> [Option<&'static str>; 10] {
        EVERY_SECRET.map(|(_, _, value)| Some(value))
    }

    /// Whether every channel is configured, secrets or not.
    fn every_channel_present(channels: &ChannelsConfig) -> bool {
        channels.telegram.is_some()
            && channels.discord.is_some()
            && channels.slack.is_some()
            && channels.signal.is_some()
            && channels.whatsapp.is_some()
    }

    /// A hermetic store: its own directory, never the OS keyring.
    fn store() -> (tempfile::TempDir, SecureStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SecureStore::file_only_at(dir.path().to_path_buf());
        (dir, store)
    }

    fn stored(store: &SecureStore, key: &str) -> Option<String> {
        store.get(key).ok()
    }

    fn file_every_secret(store: &SecureStore) {
        for (key, _, value) in EVERY_SECRET {
            store.set(key, value).expect("set");
        }
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn env_of(vars: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let vars = vars.to_vec();
        move |name| {
            vars.iter()
                .find(|(var, _)| *var == name)
                .map(|(_, value)| (*value).to_string())
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("config.toml")
    }

    /// `config.toml` as an older build wrote it: every secret in plain text.
    fn write_plaintext_config(path: &Path) {
        let old = toml::to_string_pretty(&with_every_channel()).expect("serializes");
        for (_, _, value) in EVERY_SECRET {
            assert!(old.contains(value), "the old layout carries {value}: {old}");
        }
        std::fs::write(path, old).expect("write");
    }

    // -----------------------------------------------------------------
    // Out of config.toml
    // -----------------------------------------------------------------

    #[test]
    fn saving_keeps_every_channel_secret_out_of_config_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let config = with_every_channel();

        config.save_to(&path).expect("save");

        let on_disk = read(&path);
        for (_, _, secret) in EVERY_SECRET {
            assert!(
                !on_disk.contains(secret),
                "{secret} leaked into config.toml: {on_disk}"
            );
        }
        for setting in EVERY_SETTING {
            assert!(
                on_disk.contains(setting),
                "{setting} is not a secret and stays: {on_disk}"
            );
        }
        assert_eq!(
            held(&config.channels),
            every_value(),
            "the running config keeps them"
        );
    }

    #[test]
    fn a_channel_saved_without_its_secrets_loads_back_configured() {
        // Once saved, no channel section holds its secrets. The section is
        // what turns the channel on, so each must still parse, and be there.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        with_every_channel().save_to(&path).expect("save");

        let reloaded: Config = toml::from_str(&read(&path)).expect("a saved config parses");

        assert!(
            every_channel_present(&reloaded.channels),
            "{:?}",
            reloaded.channels
        );
        assert_eq!(held(&reloaded.channels), [None; 10]);
        let telegram = reloaded.channels.telegram.expect("telegram");
        assert_eq!(telegram.allowed_users, Some(vec![42]));
    }

    #[test]
    fn a_channel_section_with_no_token_parses() {
        let config: Config = toml::from_str(
            r#"
[channels.telegram]
allowed_users = [7]

[channels.discord]
application_id = "app"
public_key = "key"

[channels.slack]
"#,
        )
        .expect("a section whose token lives in the secure store parses");
        assert_eq!(
            config.channels.telegram.map(|t| t.allowed_users),
            Some(Some(vec![7]))
        );
        assert!(config.channels.discord.is_some());
        assert!(config.channels.slack.is_some());
    }

    // -----------------------------------------------------------------
    // Filed and refilled
    // -----------------------------------------------------------------

    #[test]
    fn migrating_files_every_channel_secret_and_blanks_it() {
        let (_dir, store) = store();
        let mut config = with_every_channel();
        if let Some(telegram) = config.channels.telegram.as_mut() {
            telegram.bot_token = " tg-bot-token\n".into();
        }

        config.migrate_secrets_to(&store).expect("migrate");

        for (key, _, value) in EVERY_SECRET {
            assert_eq!(stored(&store, key).as_deref(), Some(value), "{key}");
        }
        assert_eq!(held(&config.channels), [None; 10], "blanked once stored");
        assert!(
            every_channel_present(&config.channels),
            "only the secrets leave"
        );
    }

    #[test]
    fn loading_refills_every_channel_config_toml_names() {
        let (_dir, store) = store();
        file_every_secret(&store);
        let mut config = with_every_channel();
        config.strip_secrets_for_disk();
        assert_eq!(held(&config.channels), [None; 10], "stripped");

        config.load_secrets_from(&store, no_env);

        assert_eq!(held(&config.channels), every_value());
    }

    #[test]
    fn a_stored_secret_does_not_turn_a_channel_on() {
        let (_dir, store) = store();
        file_every_secret(&store);
        let mut config = Config::default();

        config.load_secrets_from(&store, no_env);

        assert!(config.channels.telegram.is_none());
        assert!(config.channels.discord.is_none());
        assert!(config.channels.slack.is_none());
        assert!(config.channels.signal.is_none());
        assert!(config.channels.whatsapp.is_none());
    }

    #[test]
    fn every_channel_secret_can_come_from_the_environment() {
        let (_dir, store) = store();
        let env = env_of(&EVERY_SECRET.map(|(_, var, value)| (var, value)));
        let mut config = with_every_channel();
        config.strip_secrets_for_disk();
        assert_eq!(held(&config.channels), [None; 10], "stripped");

        config.load_secrets_from(&store, env);

        assert_eq!(held(&config.channels), every_value());
    }

    #[test]
    fn config_toml_then_the_environment_then_the_store() {
        let (_dir, store) = store();
        store
            .set(keys::TELEGRAM_BOT_TOKEN, "stored-token")
            .expect("set");
        store
            .set(keys::TELEGRAM_WEBHOOK_SECRET, "stored-secret")
            .expect("set");
        store
            .set(keys::SLACK_SIGNING_SECRET, "stored-signing")
            .expect("set");
        let env = env_of(&[
            ("TELEGRAM_BOT_TOKEN", "env-token"),
            ("TELEGRAM_WEBHOOK_SECRET", "env-secret"),
        ]);
        let mut config = with_every_channel();
        config.strip_secrets_for_disk();
        let telegram = config.channels.telegram.as_mut().expect("telegram");
        telegram.webhook_secret = Some("file-secret".into());

        config.load_secrets_from(&store, env);

        let telegram = config.channels.telegram.as_ref().expect("telegram");
        assert_eq!(telegram.webhook_secret.as_deref(), Some("file-secret"));
        assert_eq!(telegram.bot_token, "env-token");
        let slack = config.channels.slack.as_ref().expect("slack");
        assert_eq!(slack.signing_secret, "stored-signing");
    }

    #[test]
    fn a_channel_with_no_secret_anywhere_stays_configured() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        let mut config = with_every_channel();
        config.strip_secrets_for_disk();
        config.save_to(&path).expect("save");

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        assert!(
            every_channel_present(&loaded.channels),
            "{:?}",
            loaded.channels
        );
        assert_eq!(held(&loaded.channels), [None; 10]);
    }

    #[test]
    fn a_channel_set_up_once_survives_a_save_and_the_next_load() {
        // `nanna init`: the secrets entered are filed, the config is saved,
        // and the next process loads it.
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        let mut config = with_every_channel();
        config.migrate_secrets_to(&store).expect("migrate");
        config.save_to(&path).expect("save");

        let next = Config::load_from_with(&path, &store, no_env).expect("loads");

        assert_eq!(held(&next.channels), every_value());
        let on_disk = read(&path);
        for (_, _, secret) in EVERY_SECRET {
            assert!(!on_disk.contains(secret), "{secret}: {on_disk}");
        }
    }

    // -----------------------------------------------------------------
    // A config.toml that already holds them
    // -----------------------------------------------------------------

    #[test]
    fn plaintext_secrets_in_config_toml_are_filed_when_it_is_loaded() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        write_plaintext_config(&path);

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        assert_eq!(
            held(&loaded.channels),
            every_value(),
            "the process has them"
        );
        for (key, _, value) in EVERY_SECRET {
            assert_eq!(stored(&store, key).as_deref(), Some(value), "{key}");
        }
        assert_eq!(
            read(&path),
            toml::to_string_pretty(&with_every_channel()).expect("serializes"),
            "loading does not rewrite config.toml"
        );
    }

    #[test]
    fn the_save_that_drops_plaintext_secrets_from_config_toml_loses_none() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        write_plaintext_config(&path);
        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        // Any save: a Settings change, a daemon `config.set`.
        loaded.save_to(&path).expect("save");

        let on_disk = read(&path);
        for (_, _, secret) in EVERY_SECRET {
            assert!(!on_disk.contains(secret), "{secret}: {on_disk}");
        }
        let next = Config::load_from_with(&path, &store, no_env).expect("loads");
        assert_eq!(held(&next.channels), every_value());
    }

    #[test]
    fn a_reload_files_plaintext_secrets_too() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        write_plaintext_config(&path);

        let loaded =
            Config::load_from_replacing_with(&path, "http://localhost:11434", &store, no_env)
                .expect("loads");

        assert_eq!(held(&loaded.channels), every_value());
        for (key, _, value) in EVERY_SECRET {
            assert_eq!(stored(&store, key).as_deref(), Some(value), "{key}");
        }
    }

    #[test]
    fn a_token_written_into_config_toml_replaces_the_stored_one() {
        // The file's token is the one every load of it runs with; were the
        // stored one kept, the save that strips the file would switch tokens.
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        store
            .set(keys::TELEGRAM_BOT_TOKEN, "old-token")
            .expect("set");
        std::fs::write(&path, "[channels.telegram]\nbot_token = \"new-token\"\n").expect("write");

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        assert_eq!(
            stored(&store, keys::TELEGRAM_BOT_TOKEN).as_deref(),
            Some("new-token")
        );
        let telegram = loaded.channels.telegram.expect("telegram");
        assert_eq!(telegram.bot_token, "new-token");
    }

    #[test]
    fn a_blank_secret_in_config_toml_is_not_filed() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        store
            .set(keys::TELEGRAM_BOT_TOKEN, "stored-token")
            .expect("set");
        std::fs::write(&path, "[channels.telegram]\nbot_token = \"  \"\n").expect("write");

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        assert_eq!(
            stored(&store, keys::TELEGRAM_BOT_TOKEN).as_deref(),
            Some("stored-token")
        );
        let telegram = loaded.channels.telegram.expect("telegram");
        assert_eq!(telegram.bot_token, "stored-token", "a blank one is none");
    }

    #[test]
    fn loading_never_files_a_secret_from_the_environment() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[channels.telegram]\nallowed_users = [7]\n").expect("write");
        let env = env_of(&[("TELEGRAM_BOT_TOKEN", "env-token")]);

        let loaded = Config::load_from_with(&path, &store, env).expect("loads");

        assert_eq!(
            loaded.channels.telegram.expect("telegram").bot_token,
            "env-token"
        );
        assert_eq!(stored(&store, keys::TELEGRAM_BOT_TOKEN), None);
    }
}
