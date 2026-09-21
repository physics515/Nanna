//! Each provider's API key in its own place.
//!
//! Until 2026-09-18 `[llm].api_key` meant two things. `nanna init` and the
//! CLI's missing-key prompt stored the key of whichever `[llm].provider` was
//! picked there, and filed it in the keyring as the Anthropic key; the daemon,
//! the GUI and every other reader took that field and entry for Anthropic's
//! whatever the provider. A config written by `nanna init` for `OpenRouter` or
//! `OpenAI` therefore handed that provider's secret to Anthropic, where a
//! `claude-*` chat or an `anthropic/` summary posted it to api.anthropic.com.
//!
//! `api_key` is now Anthropic's only: the CLI stores every other provider's
//! key in that provider's own field and keyring entry. This module moves the
//! keys the old layout left behind, keyed on `[llm].provider`.

use crate::LlmConfig;
use crate::credentials::{CredentialError, SecureStore, keys};

/// A provider `nanna init` offers besides Anthropic — one whose key the old
/// layout filed as Anthropic's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonAnthropicProvider {
    OpenAI,
    OpenRouter,
}

impl NonAnthropicProvider {
    /// The provider `[llm].provider` names, when it is one of these. Anything
    /// else — `anthropic`, and a name the CLI does not know, whose chat falls
    /// back to Anthropic — keeps its key in `api_key`.
    pub fn of(provider: &str) -> Option<Self> {
        match provider {
            "openai" => Some(Self::OpenAI),
            "openrouter" => Some(Self::OpenRouter),
            _ => None,
        }
    }

    /// The keyring entry this provider's key is filed under.
    const fn store_key(self) -> &'static str {
        match self {
            Self::OpenAI => keys::OPENAI_API_KEY,
            Self::OpenRouter => keys::OPENROUTER_API_KEY,
        }
    }

    /// The key in this provider's own `[llm]` field.
    pub fn key(self, llm: &LlmConfig) -> Option<&str> {
        match self {
            Self::OpenAI => llm.openai_api_key.as_deref(),
            Self::OpenRouter => llm.openrouter_api_key.as_deref(),
        }
    }

    /// This provider's own `[llm]` field, to store a key in.
    pub const fn slot_mut(self, llm: &mut LlmConfig) -> &mut Option<String> {
        match self {
            Self::OpenAI => &mut llm.openai_api_key,
            Self::OpenRouter => &mut llm.openrouter_api_key,
        }
    }

    /// The name logs use.
    const fn name(self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::OpenRouter => "OpenRouter",
        }
    }

    /// The name of [`Self::slot_mut`]'s field in config.toml.
    const fn field_name(self) -> &'static str {
        match self {
            Self::OpenAI => "openai_api_key",
            Self::OpenRouter => "openrouter_api_key",
        }
    }
}

/// What becomes of a key filed as Anthropic's while `[llm].provider` names
/// another provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refile {
    /// Nothing is filed as Anthropic's.
    Nothing,
    /// It is Anthropic's own (see [`ANTHROPIC_KEY_PREFIX`]), whatever the
    /// provider: it stays where it is.
    Anthropic,
    /// The provider's own place is empty: the key moves there.
    Move,
    /// The provider's own place already holds this key: only the copy filed
    /// as Anthropic's goes.
    Duplicate,
    /// The provider's own place holds a different key. Nothing tells which
    /// of the two is current, so both stay and the operator is told.
    Conflict,
}

/// The prefix of every key Anthropic issues (`sk-ant-api03-…`,
/// `sk-ant-admin01-…`, OAuth's `sk-ant-oat01-…`). A key carrying it is
/// Anthropic's whatever `[llm].provider` says, so it is never moved out.
const ANTHROPIC_KEY_PREFIX: &str = "sk-ant-";

/// Decide [`Refile`] for the key filed as Anthropic's and the one in the
/// provider's own place. Blank values are absent.
fn decide(filed_as_anthropic: Option<&str>, own: Option<&str>) -> Refile {
    fn present(value: Option<&str>) -> Option<&str> {
        value.map(str::trim).filter(|value| !value.is_empty())
    }
    let Some(filed) = present(filed_as_anthropic) else {
        return Refile::Nothing;
    };
    if filed.starts_with(ANTHROPIC_KEY_PREFIX) {
        return Refile::Anthropic;
    }
    match present(own) {
        None => Refile::Move,
        Some(own) if own == filed => Refile::Duplicate,
        Some(_) => Refile::Conflict,
    }
}

/// File a non-Anthropic `[llm].provider`'s key under its own name, as a
/// config is loaded and before its secrets are hydrated: the key
/// `config.toml` itself carries ([`refile_file_key`]) and the one the
/// keyring holds ([`refile_stored_key`]).
pub fn refile_provider_key(llm: &mut LlmConfig, store: &SecureStore) {
    let Some(provider) = NonAnthropicProvider::of(&llm.provider) else {
        return;
    };
    refile_file_key(provider, llm);
    refile_stored_key(provider, store);
}

/// Read the `[llm].api_key` a `config.toml` carries as it was written: the
/// key of `[llm].provider`.
///
/// Nothing writes a key into `config.toml` any more (`Config::save_to`
/// strips them), so one there was written by the old layout or by hand
/// following it. Applied at every load — the file is only read here — until
/// the next save drops it from the file.
fn refile_file_key(provider: NonAnthropicProvider, llm: &mut LlmConfig) {
    match decide(llm.api_key.as_deref(), provider.key(llm)) {
        Refile::Nothing | Refile::Anthropic => {}
        Refile::Move => *provider.slot_mut(llm) = llm.api_key.take(),
        Refile::Duplicate => llm.api_key = None,
        Refile::Conflict => tracing::warn!(
            "config.toml holds two {name} keys: `[llm].api_key`, which `provider = \"{provider}\"` \
             made the {name} key when it was written, and a different `{field}`. `{field}` is \
             used; `api_key` is now read as the Anthropic key. Remove whichever is wrong.",
            name = provider.name(),
            provider = llm.provider,
            field = provider.field_name(),
        ),
    }
}

/// Move a key the old layout filed in the keyring as Anthropic's into
/// `provider`'s own entry — once per store.
///
/// The keyring is the machine's, shared by every config (a scratch
/// `NANNA_CONFIG_PATH` one too), so it is sorted out only under a provider
/// whose key the old layout misfiled, and then marked
/// ([`keys::LLM_KEYS_FILED_BY_PROVIDER`]): from then on nothing but
/// Anthropic's key is saved under the Anthropic entry, and a key saved there
/// is never moved again. A conflict is not marked — nothing was decided, and
/// the warning repeats until the operator removes one of the keys.
///
/// A store that cannot be read or written leaves everything as it is, with
/// a warning, and is tried again at the next load. The steps are ordered so
/// that a failure part-way loses nothing: the provider's entry is written
/// before the Anthropic one is removed, and the mark is set last.
fn refile_stored_key(provider: NonAnthropicProvider, store: &SecureStore) {
    let name = provider.name();
    match sort_out_store(provider, store) {
        Ok(Refile::Conflict) => tracing::warn!(
            "The keyring holds two {name} keys: one filed as the Anthropic key by an older \
             `nanna init`, and a different one under {name}'s own entry. The {name} entry is \
             used; the other is still read as the Anthropic key. Remove whichever is wrong \
             (`nanna init`, or Settings)."
        ),
        Ok(Refile::Move) => tracing::info!(
            "Moved the {name} key an older `nanna init` filed as the Anthropic key to {name}'s \
             own keyring entry"
        ),
        Ok(Refile::Duplicate) => tracing::info!(
            "Removed the copy of the {name} key an older `nanna init` filed as the Anthropic key"
        ),
        Ok(Refile::Nothing | Refile::Anthropic) => {}
        Err(e) => tracing::warn!(
            "Could not check the keyring for a {name} key filed as the Anthropic key ({e}); \
             trying again at the next load"
        ),
    }
}

/// [`refile_stored_key`]'s reads and writes; the decision it acted on.
fn sort_out_store(
    provider: NonAnthropicProvider,
    store: &SecureStore,
) -> Result<Refile, CredentialError> {
    if store.lookup(keys::LLM_KEYS_FILED_BY_PROVIDER)?.is_some() {
        return Ok(Refile::Nothing);
    }
    let filed = store.lookup(keys::ANTHROPIC_API_KEY)?;
    let own = store.lookup(provider.store_key())?;
    let decision = decide(filed.as_deref(), own.as_deref());
    match decision {
        Refile::Conflict => return Ok(decision),
        Refile::Nothing | Refile::Anthropic => {}
        Refile::Move => {
            let key = filed.as_deref().map_or("", str::trim);
            store.set(provider.store_key(), key)?;
            remove_anthropic_entry(store)?;
        }
        Refile::Duplicate => remove_anthropic_entry(store)?,
    }
    store.set(keys::LLM_KEYS_FILED_BY_PROVIDER, "1")?;
    Ok(decision)
}

/// Remove the Anthropic entry; already gone is done. The GUI and its daemon
/// load the config at nearly the same moment, so another process may have
/// just moved it.
fn remove_anthropic_entry(store: &SecureStore) -> Result<(), CredentialError> {
    match store.delete(keys::ANTHROPIC_API_KEY) {
        Ok(()) | Err(CredentialError::NotFound) => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::{NonAnthropicProvider, Refile, decide, refile_file_key, refile_stored_key};
    use crate::credentials::{SecureStore, keys};
    use crate::{Config, LlmConfig};

    const OPENROUTER_KEY: &str = "sk-or-v1-openrouter-key";
    const ANTHROPIC_KEY: &str = "sk-ant-api03-anthropic-key";

    /// A hermetic store: its own directory, never the OS keyring.
    fn store() -> (tempfile::TempDir, SecureStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = SecureStore::file_only_at(dir.path().to_path_buf());
        (dir, store)
    }

    fn stored(store: &SecureStore, key: &str) -> Option<String> {
        store.get(key).ok()
    }

    #[test]
    fn a_key_moves_to_an_empty_own_place() {
        assert_eq!(decide(Some(OPENROUTER_KEY), None), Refile::Move);
        assert_eq!(decide(Some(OPENROUTER_KEY), Some("  ")), Refile::Move);
        // A key with no recognisable prefix is the provider's too: the
        // provider is what says whose it is (a local OpenAI-compatible
        // server's key is often a placeholder).
        assert_eq!(decide(Some("EMPTY"), None), Refile::Move);
    }

    #[test]
    fn nothing_filed_is_nothing_to_move() {
        assert_eq!(decide(None, None), Refile::Nothing);
        assert_eq!(decide(Some(" \t"), Some(OPENROUTER_KEY)), Refile::Nothing);
    }

    #[test]
    fn an_anthropic_key_stays_anthropics_whatever_the_provider() {
        assert_eq!(decide(Some(ANTHROPIC_KEY), None), Refile::Anthropic);
        assert_eq!(decide(Some(ANTHROPIC_KEY), Some(OPENROUTER_KEY)), Refile::Anthropic);
    }

    #[test]
    fn a_copy_of_the_own_key_is_a_duplicate_and_a_different_one_a_conflict() {
        assert_eq!(
            decide(Some(OPENROUTER_KEY), Some(&format!(" {OPENROUTER_KEY}\n"))),
            Refile::Duplicate
        );
        assert_eq!(
            decide(Some(OPENROUTER_KEY), Some("sk-or-v1-another-key")),
            Refile::Conflict
        );
    }

    #[test]
    fn the_provider_names_the_place() {
        assert_eq!(NonAnthropicProvider::of("openai"), Some(NonAnthropicProvider::OpenAI));
        assert_eq!(
            NonAnthropicProvider::of("openrouter"),
            Some(NonAnthropicProvider::OpenRouter)
        );
        for provider in ["anthropic", "ollama", "something-else", ""] {
            assert_eq!(NonAnthropicProvider::of(provider), None, "{provider}");
        }
    }

    /// config.toml's `api_key` under `provider = "openrouter"`: `OpenRouter`'s.
    #[test]
    fn a_file_key_is_read_as_the_providers() {
        let mut llm = LlmConfig {
            provider: "openrouter".to_string(),
            api_key: Some(OPENROUTER_KEY.to_string()),
            ..LlmConfig::default()
        };
        refile_file_key(NonAnthropicProvider::OpenRouter, &mut llm);
        assert_eq!(llm.api_key, None);
        assert_eq!(llm.openrouter_api_key.as_deref(), Some(OPENROUTER_KEY));
        assert_eq!(llm.openai_api_key, None);

        let mut llm = LlmConfig {
            provider: "openai".to_string(),
            api_key: Some("sk-proj-openai".to_string()),
            openai_api_key: Some("sk-proj-openai".to_string()),
            ..LlmConfig::default()
        };
        refile_file_key(NonAnthropicProvider::OpenAI, &mut llm);
        assert_eq!(llm.api_key, None, "a duplicate goes");
        assert_eq!(llm.openai_api_key.as_deref(), Some("sk-proj-openai"));
    }

    #[test]
    fn a_file_key_stays_where_it_cannot_be_moved() {
        // Anthropic's key, and a conflict: untouched.
        for (api_key, own) in [
            (ANTHROPIC_KEY, None),
            (OPENROUTER_KEY, Some("sk-or-v1-another-key")),
        ] {
            let mut llm = LlmConfig {
                provider: "openrouter".to_string(),
                api_key: Some(api_key.to_string()),
                openrouter_api_key: own.map(str::to_string),
                ..LlmConfig::default()
            };
            refile_file_key(NonAnthropicProvider::OpenRouter, &mut llm);
            assert_eq!(llm.api_key.as_deref(), Some(api_key), "{api_key}");
            assert_eq!(llm.openrouter_api_key.as_deref(), own, "{api_key}");
        }
    }

    #[test]
    fn a_stored_key_moves_to_the_providers_own_entry() {
        for provider in [NonAnthropicProvider::OpenAI, NonAnthropicProvider::OpenRouter] {
            let (_dir, store) = store();
            store.set(keys::ANTHROPIC_API_KEY, "the-chat-key").expect("set");

            refile_stored_key(provider, &store);

            assert_eq!(stored(&store, keys::ANTHROPIC_API_KEY), None, "{provider:?}");
            assert_eq!(
                stored(&store, provider.store_key()).as_deref(),
                Some("the-chat-key"),
                "{provider:?}"
            );
        }
    }

    #[test]
    fn a_stored_duplicate_is_removed_from_the_anthropic_entry() {
        let (_dir, store) = store();
        store.set(keys::ANTHROPIC_API_KEY, OPENROUTER_KEY).expect("set");
        store.set(keys::OPENROUTER_API_KEY, OPENROUTER_KEY).expect("set");

        refile_stored_key(NonAnthropicProvider::OpenRouter, &store);

        assert_eq!(stored(&store, keys::ANTHROPIC_API_KEY), None);
        assert_eq!(stored(&store, keys::OPENROUTER_API_KEY).as_deref(), Some(OPENROUTER_KEY));
    }

    #[test]
    fn a_stored_anthropic_key_and_a_conflict_are_left_as_they_are() {
        for (filed, own) in [
            (ANTHROPIC_KEY, None),
            (ANTHROPIC_KEY, Some(OPENROUTER_KEY)),
            (OPENROUTER_KEY, Some("sk-or-v1-another-key")),
        ] {
            let (_dir, store) = store();
            store.set(keys::ANTHROPIC_API_KEY, filed).expect("set");
            if let Some(own) = own {
                store.set(keys::OPENROUTER_API_KEY, own).expect("set");
            }

            refile_stored_key(NonAnthropicProvider::OpenRouter, &store);

            assert_eq!(stored(&store, keys::ANTHROPIC_API_KEY).as_deref(), Some(filed));
            assert_eq!(stored(&store, keys::OPENROUTER_API_KEY).as_deref(), own);
        }
    }

    /// One-time: once a store has been sorted out, a key saved under the
    /// Anthropic entry afterwards was saved as Anthropic's and stays there,
    /// even one a later build could not recognise by its prefix.
    #[test]
    fn the_store_is_sorted_out_once() {
        let (_dir, store) = store();
        store.set(keys::ANTHROPIC_API_KEY, "the-chat-key").expect("set");
        refile_stored_key(NonAnthropicProvider::OpenRouter, &store);
        assert_eq!(stored(&store, keys::OPENROUTER_API_KEY).as_deref(), Some("the-chat-key"));

        store.set(keys::ANTHROPIC_API_KEY, "saved-as-anthropics-later").expect("set");
        refile_stored_key(NonAnthropicProvider::OpenRouter, &store);
        refile_stored_key(NonAnthropicProvider::OpenAI, &store);

        assert_eq!(
            stored(&store, keys::ANTHROPIC_API_KEY).as_deref(),
            Some("saved-as-anthropics-later")
        );
        assert_eq!(stored(&store, keys::OPENROUTER_API_KEY).as_deref(), Some("the-chat-key"));
        assert_eq!(stored(&store, keys::OPENAI_API_KEY), None);
    }

    /// An empty Anthropic entry is sorted out too: whatever is saved there
    /// later is Anthropic's.
    #[test]
    fn an_empty_store_is_sorted_out_as_well() {
        let (_dir, store) = store();
        refile_stored_key(NonAnthropicProvider::OpenRouter, &store);

        store.set(keys::ANTHROPIC_API_KEY, "saved-as-anthropics-later").expect("set");
        refile_stored_key(NonAnthropicProvider::OpenRouter, &store);

        assert_eq!(
            stored(&store, keys::ANTHROPIC_API_KEY).as_deref(),
            Some("saved-as-anthropics-later")
        );
        assert_eq!(stored(&store, keys::OPENROUTER_API_KEY), None);
    }

    /// A conflict is not a decision: it is looked at again on the next load,
    /// and moves once the provider's own entry is gone.
    #[test]
    fn a_conflict_is_revisited() {
        let (_dir, store) = store();
        store.set(keys::ANTHROPIC_API_KEY, "the-chat-key").expect("set");
        store.set(keys::OPENROUTER_API_KEY, "sk-or-v1-another-key").expect("set");
        refile_stored_key(NonAnthropicProvider::OpenRouter, &store);
        assert_eq!(stored(&store, keys::ANTHROPIC_API_KEY).as_deref(), Some("the-chat-key"));

        store.delete(keys::OPENROUTER_API_KEY).expect("delete");
        refile_stored_key(NonAnthropicProvider::OpenRouter, &store);

        assert_eq!(stored(&store, keys::ANTHROPIC_API_KEY), None);
        assert_eq!(stored(&store, keys::OPENROUTER_API_KEY).as_deref(), Some("the-chat-key"));
    }

    // -----------------------------------------------------------------
    // Through the load path every reader funnels through
    // -----------------------------------------------------------------

    fn no_env(_: &str) -> Option<String> {
        None
    }

    /// Write `toml` as a config file in its own directory.
    fn config_file(toml: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, toml).expect("write config");
        (dir, path)
    }

    /// What `nanna init` left behind for `OpenRouter`: the provider in
    /// config.toml, its key in the keyring's Anthropic entry. Loaded, the key
    /// is `OpenRouter`'s and the daemon's Anthropic slot is empty.
    #[test]
    fn an_old_init_config_loads_with_the_key_as_the_providers() {
        for (provider, own) in [
            ("openrouter", keys::OPENROUTER_API_KEY),
            ("openai", keys::OPENAI_API_KEY),
        ] {
            let (_store_dir, store) = store();
            store.set(keys::ANTHROPIC_API_KEY, "the-chat-key").expect("set");
            let (_dir, path) = config_file(&format!("[llm]\nprovider = \"{provider}\"\n"));

            let config = Config::load_from_with(&path, &store, no_env).expect("load");

            assert_eq!(config.llm.api_key, None, "{provider}: nothing is Anthropic's");
            assert_eq!(config.llm.provider_api_key(), Some("the-chat-key"), "{provider}");
            assert_eq!(stored(&store, keys::ANTHROPIC_API_KEY), None, "{provider}");
            assert_eq!(stored(&store, own).as_deref(), Some("the-chat-key"), "{provider}");
        }
    }

    /// Saved the way `nanna init` saves it, an `OpenRouter` key reaches
    /// `OpenRouter`'s own keyring entry and leaves the Anthropic one empty.
    #[test]
    fn an_entered_key_is_filed_under_its_providers_entry() {
        let (_store_dir, store) = store();
        let mut config = Config::default();
        config.llm.provider = "openrouter".to_string();
        *config.llm.provider_api_key_mut() = Some(OPENROUTER_KEY.to_string());

        config.migrate_secrets_to(&store).expect("store");

        assert_eq!(stored(&store, keys::OPENROUTER_API_KEY).as_deref(), Some(OPENROUTER_KEY));
        assert_eq!(stored(&store, keys::ANTHROPIC_API_KEY), None);
    }

    /// A key config.toml carries under a non-Anthropic provider loads as that
    /// provider's.
    #[test]
    fn a_key_in_the_file_loads_as_the_providers() {
        let (_store_dir, store) = store();
        let (_dir, path) = config_file(&format!(
            "[llm]\nprovider = \"openrouter\"\napi_key = \"{OPENROUTER_KEY}\"\n"
        ));

        let config = Config::load_from_with(&path, &store, no_env).expect("load");

        assert_eq!(config.llm.api_key, None);
        assert_eq!(config.llm.openrouter_api_key.as_deref(), Some(OPENROUTER_KEY));
    }

    /// Loading an Anthropic config neither moves nor settles anything: the
    /// keyring is shared by every config on the machine (a scratch
    /// `NANNA_CONFIG_PATH` one included), so it is sorted out only under a
    /// provider whose key the old layout misfiled.
    #[test]
    fn an_anthropic_config_leaves_the_store_to_the_one_that_needs_it() {
        let (_store_dir, store) = store();
        store.set(keys::ANTHROPIC_API_KEY, "the-chat-key").expect("set");
        let (_dir, anthropic) = config_file("[llm]\nprovider = \"anthropic\"\n");

        let config = Config::load_from_with(&anthropic, &store, no_env).expect("load");
        assert_eq!(config.llm.api_key.as_deref(), Some("the-chat-key"));
        assert_eq!(stored(&store, keys::ANTHROPIC_API_KEY).as_deref(), Some("the-chat-key"));

        let (_dir, openrouter) = config_file("[llm]\nprovider = \"openrouter\"\n");
        let config = Config::load_from_with(&openrouter, &store, no_env).expect("load");
        assert_eq!(config.llm.api_key, None);
        assert_eq!(config.llm.openrouter_api_key.as_deref(), Some("the-chat-key"));
    }

    /// `ANTHROPIC_API_KEY` names its provider: it is Anthropic's under any
    /// `[llm].provider`, and is never moved.
    #[test]
    fn the_anthropic_environment_key_stays_anthropics() {
        let (_store_dir, store) = store();
        let (_dir, path) = config_file("[llm]\nprovider = \"openrouter\"\n");

        let config = Config::load_from_with(&path, &store, |name| {
            (name == "ANTHROPIC_API_KEY").then(|| "from-the-environment".to_string())
        })
        .expect("load");

        assert_eq!(config.llm.api_key.as_deref(), Some("from-the-environment"));
        assert_eq!(config.llm.openrouter_api_key, None);
    }
}
