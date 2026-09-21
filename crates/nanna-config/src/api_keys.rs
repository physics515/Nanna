//! The `[llm]` keys and `[tools].brave_api_key`: each provider's API key, the
//! GitHub token, the Anthropic OAuth token, the Ollama server's bearer token
//! and the Brave Search key.
//!
//! Like every other secret they are held in the secure store and never written
//! to `config.toml`: `Config::strip_secrets_for_disk` blanks them in the copy a
//! save writes, `Config::migrate_secrets_to_keyring` files the ones held in
//! memory, and each load fills them from the environment, else the store.
//!
//! **A `config.toml` that holds one** — written by hand, or left by a build
//! from before saves stripped them — is migrated by [`adopt`] when the file is
//! loaded, as the channel and webhook secrets are: each key it holds is filed
//! in the store before anything could save the config, so the save that next
//! strips it from the file loses nothing. The file itself is not rewritten by
//! a load (every CLI command, the GUI and the daemon load it, some at once,
//! and a rewrite from the parsed config would drop its comments); the key
//! leaves it at the next save of any kind, and until then each load says it
//! can be deleted.

use crate::credentials::{SecureStore, keys};
use crate::{Config, StoredCopy, adopt_file_secret, adopt_file_secret_with, ollama_server_changed};

/// One key filed under a store entry of its own.
struct Secret {
    /// Its key in `config.toml`, for messages (never the value).
    field: &'static str,
    /// Its key in the secure store.
    key: &'static str,
    /// The environment variable that supplies it, for messages.
    env: &'static str,
    /// Where the config holds it.
    slot: fn(&mut Config) -> &mut Option<String>,
}

/// Every key but the Ollama token, which is filed for its server
/// ([`adopt_ollama_token`]).
const KEYS: [Secret; 6] = [
    Secret {
        field: "llm.api_key",
        key: keys::ANTHROPIC_API_KEY,
        env: "ANTHROPIC_API_KEY",
        slot: |c| &mut c.llm.api_key,
    },
    Secret {
        field: "llm.openai_api_key",
        key: keys::OPENAI_API_KEY,
        env: "OPENAI_API_KEY",
        slot: |c| &mut c.llm.openai_api_key,
    },
    Secret {
        field: "llm.openrouter_api_key",
        key: keys::OPENROUTER_API_KEY,
        env: "OPENROUTER_API_KEY",
        slot: |c| &mut c.llm.openrouter_api_key,
    },
    Secret {
        field: "llm.github_token",
        key: keys::GITHUB_TOKEN,
        env: "GITHUB_TOKEN",
        slot: |c| &mut c.llm.github_token,
    },
    // As a token entered in Settings is filed: the bare token the config is
    // filled from. A stored login's credential, which carries the refresh
    // token and which request auth prefers, is left alone — it came first
    // while the file held this token too.
    Secret {
        field: "llm.anthropic_oauth_token",
        key: keys::ANTHROPIC_OAUTH_TOKEN,
        env: "ANTHROPIC_OAUTH_TOKEN",
        slot: |c| &mut c.llm.anthropic_oauth_token,
    },
    Secret {
        field: "tools.brave_api_key",
        key: keys::BRAVE_API_KEY,
        env: "BRAVE_API_KEY",
        slot: |c| &mut c.tools.brave_api_key,
    },
];

/// File in `store` each key `config` holds as parsed from `config.toml`
/// ([`adopt_file_secret`]: the file's replaces a different stored one, or the
/// save that strips it would switch keys). `config` keeps each one, trimmed as
/// it is filed: the loads after that save read it from the store, and must run
/// with what this one did. A blank one is none.
///
/// Call on the parsed file only, before anything is filled in — a key from
/// the environment is never filed — and after
/// `provider_key::refile_provider_key`, so that an `[llm].api_key` the old
/// layout wrote for another provider is filed as that provider's, not as
/// Anthropic's.
pub fn adopt(config: &mut Config, store: &SecureStore) {
    for secret in &KEYS {
        let slot = (secret.slot)(config);
        let Some(held) = trimmed(slot.as_deref()) else {
            continue;
        };
        adopt_file_secret(secret.field, secret.env, secret.key, &held, store);
        *slot = Some(held);
    }
    adopt_ollama_token(config, store);
}

/// File the Ollama token `config` holds for the server it names,
/// `[memory].ollama_host`: every load of the file sends it there, and a stored
/// token is loaded only for the server it was filed for. Filed for another
/// server, or for none (the same token as an older build filed it), it is
/// filed again, for this one.
fn adopt_ollama_token(config: &mut Config, store: &SecureStore) {
    let Some(held) = trimmed(config.llm.ollama_api_key.as_deref()) else {
        return;
    };
    let host = &config.memory.ollama_host;
    let filed_for_host = || {
        store
            .ollama_token_host()
            .is_ok_and(|bound| bound.is_some_and(|bound| !ollama_server_changed(&bound, host)))
    };
    let stored = match store.ollama_token() {
        None => StoredCopy::None,
        Some(token) if token == held && filed_for_host() => StoredCopy::Same,
        Some(_) => StoredCopy::Other,
    };
    adopt_file_secret_with("llm.ollama_api_key", "OLLAMA_API_KEY", stored, || {
        store.save_ollama_token(&held, host)
    });
    config.llm.ollama_api_key = Some(held);
}

/// `held` trimmed, when it is not blank.
fn trimmed(held: Option<&str>) -> Option<String> {
    held.map(str::trim)
        .filter(|held| !held.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use crate::Config;
    use crate::credentials::{OAuthCredential, SecureStore, keys};
    use std::fmt::Write as _;
    use std::path::Path;

    /// Every key filed under a store entry of its own: its section and field
    /// in `config.toml`, its store key, its environment variable, and the
    /// value the tests give it. The Ollama token, bound to a server, is
    /// tested on its own.
    const EVERY_KEY: [(&str, &str, &str, &str, &str); 6] = [
        (
            "llm",
            "api_key",
            keys::ANTHROPIC_API_KEY,
            "ANTHROPIC_API_KEY",
            "sk-ant-api03-file-key",
        ),
        (
            "llm",
            "openai_api_key",
            keys::OPENAI_API_KEY,
            "OPENAI_API_KEY",
            "sk-proj-file-key",
        ),
        (
            "llm",
            "openrouter_api_key",
            keys::OPENROUTER_API_KEY,
            "OPENROUTER_API_KEY",
            "sk-or-v1-file-key",
        ),
        (
            "llm",
            "github_token",
            keys::GITHUB_TOKEN,
            "GITHUB_TOKEN",
            "ghp_file_token",
        ),
        (
            "llm",
            "anthropic_oauth_token",
            keys::ANTHROPIC_OAUTH_TOKEN,
            "ANTHROPIC_OAUTH_TOKEN",
            "sk-ant-oat01-file-token",
        ),
        (
            "tools",
            "brave_api_key",
            keys::BRAVE_API_KEY,
            "BRAVE_API_KEY",
            "brave-file-key",
        ),
    ];

    const GPU: &str = "https://gpu.example/ollama";
    const OLLAMA_TOKEN: &str = "ollama-file-token";

    /// A hermetic store: its own directory, never the OS keyring.
    fn store() -> (tempfile::TempDir, SecureStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SecureStore::file_only_at(dir.path().to_path_buf());
        (dir, store)
    }

    fn stored(store: &SecureStore, key: &str) -> Option<String> {
        store.get(key).ok()
    }

    /// The server the store records for its Ollama token; the test store
    /// always answers.
    fn recorded_server(store: &SecureStore) -> Option<String> {
        store.ollama_token_host().expect("a file store answers")
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    /// The key `config` holds for the `EVERY_KEY` entry filed under `key`.
    fn held<'a>(config: &'a Config, key: &str) -> Option<&'a str> {
        match key {
            keys::ANTHROPIC_API_KEY => config.llm.api_key.as_deref(),
            keys::OPENAI_API_KEY => config.llm.openai_api_key.as_deref(),
            keys::OPENROUTER_API_KEY => config.llm.openrouter_api_key.as_deref(),
            keys::GITHUB_TOKEN => config.llm.github_token.as_deref(),
            keys::ANTHROPIC_OAUTH_TOKEN => config.llm.anthropic_oauth_token.as_deref(),
            keys::BRAVE_API_KEY => config.tools.brave_api_key.as_deref(),
            other => panic!("no such key in EVERY_KEY: {other}"),
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("config.toml")
    }

    /// A hand-written `config.toml` holding every `EVERY_KEY` entry, each
    /// given by `value(its default)`, and the Ollama token for [`GPU`].
    fn write_config_holding(path: &Path, value: impl Fn(&str) -> String) {
        let lines = |section: &str| {
            let mut lines = String::new();
            for (_, field, _, _, default) in EVERY_KEY.iter().filter(|(s, ..)| *s == section) {
                writeln!(lines, "{field} = \"{}\"", value(default)).expect("a String takes it");
            }
            lines
        };
        let toml = format!(
            "[llm]\n{}ollama_api_key = \"{}\"\n\n[tools]\n{}\n[memory]\nollama_host = \"{GPU}\"\n",
            lines("llm"),
            value(OLLAMA_TOKEN),
            lines("tools"),
        );
        std::fs::write(path, toml).expect("write");
    }

    fn write_config_with_every_key(path: &Path) {
        write_config_holding(path, str::to_owned);
    }

    // -----------------------------------------------------------------
    // A config.toml that holds them
    // -----------------------------------------------------------------

    #[test]
    fn every_key_in_config_toml_is_filed_when_it_is_loaded() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        write_config_with_every_key(&path);
        let before = read(&path);

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        for (_, field, key, _, value) in EVERY_KEY {
            assert_eq!(held(&loaded, key), Some(value), "{field} takes effect");
            assert_eq!(
                stored(&store, key).as_deref(),
                Some(value),
                "{field} is filed"
            );
        }
        assert_eq!(loaded.llm.ollama_api_key.as_deref(), Some(OLLAMA_TOKEN));
        assert_eq!(store.ollama_token().as_deref(), Some(OLLAMA_TOKEN));
        assert_eq!(
            recorded_server(&store).as_deref(),
            Some(GPU),
            "the Ollama token is filed for the server the file names"
        );
        assert_eq!(read(&path), before, "loading does not rewrite config.toml");
    }

    /// The failure this exists for: a key hand-written into `config.toml`
    /// worked until the first save of anything (a tool toggle in the daemon,
    /// any `config.set`), which stripped it from the file, and the next load
    /// — the daemon's config watcher reloads within seconds of its own save —
    /// ran without it.
    #[test]
    fn a_hand_written_key_survives_the_next_save() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        write_config_with_every_key(&path);

        let mut loaded = Config::load_from_with(&path, &store, no_env).expect("loads");
        loaded.tools.disabled.push("web_search".to_string());
        loaded.save_to(&path).expect("save");
        let on_disk = read(&path);
        let reloaded = Config::load_from_with(&path, &store, no_env).expect("reloads");

        for (_, field, key, _, value) in EVERY_KEY {
            assert!(
                !on_disk.contains(value),
                "{field} leaked into config.toml: {on_disk}"
            );
            assert_eq!(held(&reloaded, key), Some(value), "{field} is kept");
        }
        assert!(
            !on_disk.contains(OLLAMA_TOKEN),
            "leaked into config.toml: {on_disk}"
        );
        assert_eq!(reloaded.llm.ollama_api_key.as_deref(), Some(OLLAMA_TOKEN));
    }

    #[test]
    fn a_reload_files_them_too() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        write_config_with_every_key(&path);

        let loaded = Config::load_from_replacing_with(&path, GPU, &store, no_env).expect("loads");

        for (_, field, key, _, value) in EVERY_KEY {
            assert_eq!(held(&loaded, key), Some(value), "{field}");
            assert_eq!(stored(&store, key).as_deref(), Some(value), "{field}");
        }
        assert_eq!(store.ollama_token().as_deref(), Some(OLLAMA_TOKEN));
        assert_eq!(recorded_server(&store).as_deref(), Some(GPU));
    }

    #[test]
    fn a_key_written_into_config_toml_replaces_the_stored_one() {
        // The file's key is the one every load of it runs with; were the
        // stored one kept, the save that strips the file would switch keys.
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        for (_, _, key, _, _) in EVERY_KEY {
            store.set(key, "stored-key").expect("set");
        }
        store.save_ollama_token("stored-token", GPU).expect("save");
        write_config_with_every_key(&path);

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        for (_, field, key, _, value) in EVERY_KEY {
            assert_eq!(held(&loaded, key), Some(value), "{field}");
            assert_eq!(stored(&store, key).as_deref(), Some(value), "{field}");
        }
        assert_eq!(loaded.llm.ollama_api_key.as_deref(), Some(OLLAMA_TOKEN));
        assert_eq!(store.ollama_token().as_deref(), Some(OLLAMA_TOKEN));
    }

    #[test]
    fn a_padded_key_runs_as_it_is_filed() {
        // The store holds it trimmed, so this process runs with it trimmed
        // too: the loads after the next save read it from there.
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        write_config_holding(&path, |value| format!("  {value} "));

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        for (_, field, key, _, value) in EVERY_KEY {
            assert_eq!(held(&loaded, key), Some(value), "{field}");
            assert_eq!(stored(&store, key).as_deref(), Some(value), "{field}");
        }
        assert_eq!(loaded.llm.ollama_api_key.as_deref(), Some(OLLAMA_TOKEN));
        assert_eq!(store.ollama_token().as_deref(), Some(OLLAMA_TOKEN));
    }

    #[test]
    fn a_blank_key_in_config_toml_is_not_filed() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        for (_, _, key, _, _) in EVERY_KEY {
            store.set(key, "stored-key").expect("set");
        }
        store.save_ollama_token("stored-token", GPU).expect("save");
        write_config_holding(&path, |_| "  ".to_string());

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");

        for (_, field, key, _, _) in EVERY_KEY {
            assert_eq!(
                stored(&store, key).as_deref(),
                Some("stored-key"),
                "{field}"
            );
            assert_eq!(
                held(&loaded, key),
                Some("stored-key"),
                "{field}: a blank one is none"
            );
        }
        assert_eq!(store.ollama_token().as_deref(), Some("stored-token"));
        assert_eq!(loaded.llm.ollama_api_key.as_deref(), Some("stored-token"));
    }

    #[test]
    fn loading_never_files_a_key_from_the_environment() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, format!("[memory]\nollama_host = \"{GPU}\"\n")).expect("write");
        let env = |name: &str| {
            EVERY_KEY
                .iter()
                .find(|(.., var, _)| *var == name)
                .map(|(.., value)| format!("env-{value}"))
                .or_else(|| (name == "OLLAMA_API_KEY").then(|| "env-token".to_string()))
        };

        let loaded = Config::load_from_with(&path, &store, env).expect("loads");

        for (_, field, key, _, value) in EVERY_KEY {
            assert_eq!(
                held(&loaded, key),
                Some(format!("env-{value}").as_str()),
                "{field}"
            );
            assert_eq!(stored(&store, key), None, "{field}");
        }
        assert_eq!(loaded.llm.ollama_api_key.as_deref(), Some("env-token"));
        assert_eq!(store.ollama_token(), None);
    }

    // -----------------------------------------------------------------
    // `[llm].api_key` under another provider
    // -----------------------------------------------------------------

    /// The old layout wrote a non-Anthropic provider's key as `api_key`; it
    /// is read as that provider's (`provider_key::refile_provider_key`), so
    /// it is filed as that provider's — never as Anthropic's, which would
    /// hand it to api.anthropic.com after the save.
    #[test]
    fn an_old_layout_key_in_config_toml_is_filed_as_its_providers() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[llm]\nprovider = \"openrouter\"\napi_key = \"sk-or-v1-old-layout\"\n",
        )
        .expect("write");

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");
        loaded.save_to(&path).expect("save");
        let reloaded = Config::load_from_with(&path, &store, no_env).expect("reloads");

        assert_eq!(
            stored(&store, keys::OPENROUTER_API_KEY).as_deref(),
            Some("sk-or-v1-old-layout")
        );
        assert_eq!(stored(&store, keys::ANTHROPIC_API_KEY), None);
        assert_eq!(
            reloaded.llm.openrouter_api_key.as_deref(),
            Some("sk-or-v1-old-layout")
        );
        assert_eq!(reloaded.llm.api_key, None);
    }

    #[test]
    fn an_anthropic_key_in_config_toml_is_anthropics_under_any_provider() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[llm]\nprovider = \"openrouter\"\napi_key = \"sk-ant-api03-anthropic\"\n",
        )
        .expect("write");

        Config::load_from_with(&path, &store, no_env).expect("loads");

        assert_eq!(
            stored(&store, keys::ANTHROPIC_API_KEY).as_deref(),
            Some("sk-ant-api03-anthropic")
        );
        assert_eq!(stored(&store, keys::OPENROUTER_API_KEY), None);
    }

    // -----------------------------------------------------------------
    // The Ollama token, bound to its server
    // -----------------------------------------------------------------

    fn write_ollama_config(path: &Path, host: &str, token: &str) {
        std::fs::write(
            path,
            format!("[llm]\nollama_api_key = \"{token}\"\n\n[memory]\nollama_host = \"{host}\"\n"),
        )
        .expect("write");
    }

    /// Filed for the file's server, it goes on reaching that server after the
    /// save, and no other.
    #[test]
    fn the_ollama_token_in_config_toml_stays_with_the_server_the_file_names() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        write_ollama_config(&path, GPU, OLLAMA_TOKEN);

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");
        loaded.save_to(&path).expect("save");
        let reloaded = Config::load_from_with(&path, &store, no_env).expect("reloads");
        assert_eq!(reloaded.llm.ollama_api_key.as_deref(), Some(OLLAMA_TOKEN));

        let mut elsewhere = Config::default();
        elsewhere.memory.ollama_host = "https://elsewhere.example".to_string();
        elsewhere.load_secrets_from(&store, no_env);
        assert_eq!(
            elsewhere.llm.ollama_api_key, None,
            "another server never gets it"
        );
    }

    /// The store holds the same token for another server, or for none (an
    /// older build's): the file says it is this server's, and every load of
    /// the file sends it here, so it is filed for this server.
    #[test]
    fn an_ollama_token_filed_for_another_server_is_filed_for_this_one() {
        for bound_to in [Some("https://elsewhere.example"), None] {
            let (dir, store) = store();
            let path = dir.path().join("config.toml");
            match bound_to {
                Some(host) => store.save_ollama_token(OLLAMA_TOKEN, host).expect("save"),
                None => store.set(keys::OLLAMA_API_KEY, OLLAMA_TOKEN).expect("set"),
            }
            write_ollama_config(&path, GPU, OLLAMA_TOKEN);

            Config::load_from_with(&path, &store, no_env).expect("loads");

            assert_eq!(
                recorded_server(&store).as_deref(),
                Some(GPU),
                "{bound_to:?}"
            );
            assert_eq!(
                store.ollama_token().as_deref(),
                Some(OLLAMA_TOKEN),
                "{bound_to:?}"
            );
        }
    }

    /// Already filed for this server, however its address is spelled: left
    /// as it is, not written again at every load.
    #[test]
    fn an_ollama_token_already_filed_for_this_server_is_left_as_it_is() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        store
            .save_ollama_token(OLLAMA_TOKEN, "https://GPU.example:443/ollama/")
            .expect("save");
        write_ollama_config(&path, GPU, OLLAMA_TOKEN);

        Config::load_from_with(&path, &store, no_env).expect("loads");

        assert_eq!(
            recorded_server(&store).as_deref(),
            Some("https://GPU.example:443/ollama"),
            "not re-recorded"
        );
    }

    // -----------------------------------------------------------------
    // The Anthropic OAuth token next to a stored login
    // -----------------------------------------------------------------

    /// Filed as a token entered in Settings is (`migrate_secrets_to`): the
    /// bare token the config is filled from. A stored login's credential —
    /// which carries the refresh token, and which request auth prefers, as it
    /// did while the file held the token — is left alone.
    #[test]
    fn an_oauth_token_in_config_toml_leaves_a_stored_login_alone() {
        let (dir, store) = store();
        let path = dir.path().join("config.toml");
        let login = OAuthCredential {
            access_token: "sk-ant-oat01-logged-in".to_string(),
            refresh_token: Some("sk-ant-ort01-refresh".to_string()),
            expires_at: None,
            subscription_type: None,
            account_id: None,
            organization_id: None,
        };
        store.save_anthropic_oauth(&login).expect("save login");
        std::fs::write(
            &path,
            "[llm]\nanthropic_oauth_token = \"sk-ant-oat01-from-the-file\"\n",
        )
        .expect("write");

        let loaded = Config::load_from_with(&path, &store, no_env).expect("loads");
        loaded.save_to(&path).expect("save");
        let reloaded = Config::load_from_with(&path, &store, no_env).expect("reloads");

        assert_eq!(
            reloaded.llm.anthropic_oauth_token.as_deref(),
            Some("sk-ant-oat01-from-the-file")
        );
        let still = store
            .load_anthropic_oauth()
            .expect("the login is still there");
        assert_eq!(still.access_token, "sk-ant-oat01-logged-in");
        assert_eq!(still.refresh_token.as_deref(), Some("sk-ant-ort01-refresh"));
    }
}
