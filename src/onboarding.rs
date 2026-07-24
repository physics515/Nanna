//! Onboarding and setup wizard for Nanna.

use console::{Emoji, style};
use dialoguer::{Confirm, Input, Password, Select, theme::ColorfulTheme};
use nanna_config::{Config, DiscordConfig, SlackConfig, TelegramConfig};
use std::path::PathBuf;

static MOON: Emoji<'_, '_> = Emoji("🌙 ", "");
static CHECK: Emoji<'_, '_> = Emoji("✓ ", "[ok] ");
static ROCKET: Emoji<'_, '_> = Emoji("🚀 ", "");
static KEY: Emoji<'_, '_> = Emoji("🔑 ", "");
static GEAR: Emoji<'_, '_> = Emoji("⚙️  ", "");

const BANNER: &str = r"
         🌙
        /|\
       / | \
      /  |  \
     /   |   \
    /____|____\
       NANNA
";

/// Check if this is a first run (no config exists).
pub fn is_first_run() -> bool {
    Config::default_config_path().map_or(true, |p| !p.exists())
}

/// Environment variable that holds the API key for `provider`.
///
/// `None` means the provider needs no key at all. A local Ollama server is the case that matters:
/// a fully-local install is the intended default experience, so it must never be nagged for a
/// credential it does not use. An unrecognised provider falls back to Anthropic, preserving the
/// historical behaviour rather than silently declaring an unknown setup "configured".
fn provider_api_key_env(provider: &str) -> Option<&'static str> {
    match provider {
        "ollama" => None,
        "openai" => Some("OPENAI_API_KEY"),
        "openrouter" => Some("OPENROUTER_API_KEY"),
        _ => Some("ANTHROPIC_API_KEY"),
    }
}

/// Whether a credential is present, given the configured key and a way to read the environment.
///
/// Pure so it can be tested without mutating process-global environment variables, which is
/// unsound under a parallel test runner. A value that is present but blank does not count — an
/// exported-but-empty `OPENAI_API_KEY` is a misconfiguration, not a credential.
fn has_api_key_with(
    provider: &str,
    configured_key: Option<&str>,
    read_env: impl Fn(&str) -> Option<String>,
) -> bool {
    if configured_key.is_some_and(|key| !key.trim().is_empty()) {
        return true;
    }
    // No variable means the provider needs no key, which counts as configured.
    provider_api_key_env(provider)
        .is_none_or(|variable| read_env(variable).is_some_and(|value| !value.trim().is_empty()))
}

/// Check if an API key is configured for the *selected* provider.
///
/// Previously this only ever looked at `ANTHROPIC_API_KEY`, so an `OpenAI` or `OpenRouter` user with
/// their key exported was told it was missing and re-prompted on every launch, and an Ollama user
/// was asked for a key that provider has no concept of.
pub fn has_api_key(config: &Config) -> bool {
    // Prefer an already-hydrated in-memory key or the process environment;
    // fall back to the OS keyring / encrypted store so a key saved at
    // onboarding (and never written to config.toml) still counts.
    has_api_key_with(
        &config.llm.provider,
        config.llm.api_key.as_deref(),
        |variable| {
            if let Ok(v) = std::env::var(variable) {
                if !v.trim().is_empty() {
                    return Some(v);
                }
            }
            // Map the env-var name to the SecureStore key.
            let store_key = match variable {
                "ANTHROPIC_API_KEY" => nanna_config::credentials::keys::ANTHROPIC_API_KEY,
                "OPENAI_API_KEY" => nanna_config::credentials::keys::OPENAI_API_KEY,
                "OPENROUTER_API_KEY" => nanna_config::credentials::keys::OPENROUTER_API_KEY,
                "GITHUB_TOKEN" => nanna_config::credentials::keys::GITHUB_TOKEN,
                other => other,
            };
            nanna_config::credentials::SecureStore::new()
                .get(store_key)
                .ok()
                .filter(|v| !v.trim().is_empty())
        },
    )
}

/// Persist config: secrets → keyring, non-secrets → config.toml.
fn persist_config(config: &mut Config) -> anyhow::Result<()> {
    if let Err(e) = config.migrate_secrets_to_keyring() {
        // Don't write secrets to disk as a consolation prize.
        config.strip_secrets_for_disk();
        return Err(anyhow::anyhow!("failed to store secrets securely: {e}"));
    }
    config.save()?;
    Ok(())
}


/// Configure LLM settings (provider, API key, model).
fn configure_llm(config: &mut Config, theme: &ColorfulTheme) -> anyhow::Result<()> {
    println!("{GEAR}{}", style("LLM Configuration").bold());

    // Provider selection
    let providers = vec!["Anthropic (Claude)", "OpenAI", "OpenRouter"];
    let provider_idx = Select::with_theme(theme)
        .with_prompt("Which LLM provider do you want to use?")
        .items(&providers)
        .default(0)
        .interact()?;

    config.llm.provider = match provider_idx {
        1 => "openai".to_string(),
        2 => "openrouter".to_string(),
        _ => "anthropic".to_string(),
    };

    // API Key
    println!("\n{KEY}{}", style("API Key").bold());

    // Same mapping `has_api_key` checks against — one definition, so the prompt can never name a
    // different variable than the one that actually satisfies the check.
    let env_var = provider_api_key_env(&config.llm.provider).unwrap_or("ANTHROPIC_API_KEY");

    let api_key_hint = format!(
        "Enter your {} API key (or set {} env var)",
        config.llm.provider, env_var
    );

    let api_key: String = Password::with_theme(theme)
        .with_prompt(&api_key_hint)
        .allow_empty_password(true)
        .interact()?;

    if !api_key.is_empty() {
        config.llm.api_key = Some(api_key);
    }

    // Model selection
    let models = match config.llm.provider.as_str() {
        "anthropic" => vec![
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
            "claude-3-5-haiku-20241022",
        ],
        "openai" => vec!["gpt-4o", "gpt-4-turbo", "gpt-3.5-turbo"],
        "openrouter" => vec![
            "anthropic/claude-sonnet-4",
            "openai/gpt-4o",
            "google/gemini-pro",
        ],
        _ => vec!["claude-sonnet-4-20250514"],
    };

    let model_idx = Select::with_theme(theme)
        .with_prompt("Which model do you want to use?")
        .items(&models)
        .default(0)
        .interact()?;

    config.llm.model = models[model_idx].to_string();
    Ok(())
}

/// Configure messaging channels (Telegram, Discord, Slack).
fn configure_channels(config: &mut Config, theme: &ColorfulTheme) -> anyhow::Result<()> {
    println!("\n{ROCKET}{}", style("Channel Configuration").bold());

    // Telegram
    if Confirm::with_theme(theme)
        .with_prompt("Do you want to set up Telegram?")
        .default(false)
        .interact()?
    {
        let token: String = Password::with_theme(theme)
            .with_prompt("Enter your Telegram bot token (from @BotFather)")
            .interact()?;

        if !token.is_empty() {
            config.channels.telegram = Some(TelegramConfig {
                bot_token: token,
                webhook_url: None,
                allowed_users: None,
            });
            println!("  {CHECK}Telegram configured");
        }
    }

    // Discord
    if Confirm::with_theme(theme)
        .with_prompt("Do you want to set up Discord?")
        .default(false)
        .interact()?
    {
        let token: String = Password::with_theme(theme)
            .with_prompt("Enter your Discord bot token")
            .interact()?;

        let app_id: String = Input::with_theme(theme)
            .with_prompt("Enter your Discord application ID")
            .interact_text()?;

        let public_key: String = Input::with_theme(theme)
            .with_prompt("Enter your Discord public key")
            .interact_text()?;

        if !token.is_empty() && !app_id.is_empty() && !public_key.is_empty() {
            config.channels.discord = Some(DiscordConfig {
                bot_token: token,
                application_id: app_id,
                public_key,
            });
            println!("  {CHECK}Discord configured");
        }
    }

    // Slack
    if Confirm::with_theme(theme)
        .with_prompt("Do you want to set up Slack?")
        .default(false)
        .interact()?
    {
        let token: String = Password::with_theme(theme)
            .with_prompt("Enter your Slack bot token (xoxb-...)")
            .interact()?;

        let signing_secret: String = Password::with_theme(theme)
            .with_prompt("Enter your Slack signing secret")
            .interact()?;

        if !token.is_empty() && !signing_secret.is_empty() {
            config.channels.slack = Some(SlackConfig {
                bot_token: token,
                app_token: None,
                signing_secret,
            });
            println!("  {CHECK}Slack configured");
        }
    }

    Ok(())
}

/// Configure server settings.
fn configure_server(config: &mut Config, theme: &ColorfulTheme) -> anyhow::Result<()> {
    println!("\n{GEAR}{}", style("Server Configuration").bold());

    config.server.enabled = Confirm::with_theme(theme)
        .with_prompt("Enable HTTP server for webhooks?")
        .default(true)
        .interact()?;

    if config.server.enabled {
        config.server.port = Input::with_theme(theme)
            .with_prompt("Server port")
            .default(3000)
            .interact_text()?;
    }

    Ok(())
}

/// Configure workspace and name.
fn configure_workspace(config: &mut Config, theme: &ColorfulTheme) -> anyhow::Result<()> {
    println!("\n{GEAR}{}", style("Workspace").bold());

    let default_workspace = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .display()
        .to_string();

    let workspace: String = Input::with_theme(theme)
        .with_prompt("Workspace directory (for file operations)")
        .default(default_workspace)
        .interact_text()?;

    config.general.workspace = Some(PathBuf::from(workspace));

    config.general.name = Input::with_theme(theme)
        .with_prompt("What should I call myself?")
        .default("Nanna".to_string())
        .interact_text()?;

    Ok(())
}

/// Run the onboarding wizard.
pub fn run_onboarding() -> anyhow::Result<Config> {
    println!("{}", style(BANNER).cyan());
    println!(
        "{MOON}Welcome to {} - the moon rises.",
        style("Nanna").cyan().bold()
    );
    println!("Let's get you set up.\n");

    let theme = ColorfulTheme::default();
    let mut config = Config::default();

    configure_llm(&mut config, &theme)?;
    configure_channels(&mut config, &theme)?;
    configure_server(&mut config, &theme)?;
    configure_workspace(&mut config, &theme)?;

    // Save config
    println!("\n{CHECK}{}", style("Saving configuration...").bold());
    persist_config(&mut config)?;

    let config_path = Config::default_config_path()?;
    println!(
        "  {CHECK}Configuration saved to {}",
        style(config_path.display()).green()
    );

    println!(
        "\n{MOON}{}",
        style("Setup complete! You're ready to go.").green().bold()
    );
    println!("\nTry these commands:");
    println!("  {} - Interactive chat", style("nanna chat").cyan());
    println!("  {} - Start the server", style("nanna server").cyan());
    println!(
        "  {} - One-shot query",
        style("nanna run \"Hello!\"").cyan()
    );

    Ok(config)
}

/// Quick setup - just get API key.
pub fn quick_setup(config: &mut Config) -> anyhow::Result<()> {
    println!(
        "\n{}No API key found. Let's fix that.",
        style("⚠️  ").yellow()
    );

    let theme = ColorfulTheme::default();

    // Ask for the key the *configured* provider actually uses. Hardcoding Anthropic here asked
    // OpenAI/OpenRouter users for a credential that would never be read back.
    let provider = config.llm.provider.clone();
    let env_var = provider_api_key_env(&provider).unwrap_or("ANTHROPIC_API_KEY");

    let api_key: String = Password::with_theme(&theme)
        .with_prompt(format!("Enter your {provider} API key"))
        .interact()?;

    if api_key.trim().is_empty() {
        anyhow::bail!("API key is required. Set {env_var} or run 'nanna init'");
    }

    config.llm.api_key = Some(api_key);
    persist_config(config)?;

    println!("{CHECK}API key saved to the OS keychain.");
    Ok(())
}

/// Show setup status.
pub fn show_status(config: &Config) -> anyhow::Result<()> {
    println!("{}", style("Nanna Configuration Status").bold());
    println!("{}", "─".repeat(40));

    // Config path
    let config_path = Config::default_config_path()?;
    println!(
        "Config: {}",
        if config_path.exists() {
            style(config_path.display().to_string()).green()
        } else {
            style("not created".to_string()).red()
        }
    );

    // LLM
    println!("\n{}", style("LLM").bold());
    println!("  Provider: {}", config.llm.provider);
    println!("  Model: {}", config.llm.model);
    println!(
        "  API Key: {}",
        if has_api_key(config) {
            style("configured").green()
        } else {
            style("missing").red()
        }
    );

    // Channels
    println!("\n{}", style("Channels").bold());
    println!(
        "  Telegram: {}",
        if config.channels.telegram.is_some() {
            style("configured").green()
        } else {
            style("not configured").dim()
        }
    );
    println!(
        "  Discord: {}",
        if config.channels.discord.is_some() {
            style("configured").green()
        } else {
            style("not configured").dim()
        }
    );
    println!(
        "  Slack: {}",
        if config.channels.slack.is_some() {
            style("configured").green()
        } else {
            style("not configured").dim()
        }
    );

    // Server
    println!("\n{}", style("Server").bold());
    println!(
        "  Enabled: {}",
        if config.server.enabled { "yes" } else { "no" }
    );
    if config.server.enabled {
        println!("  Port: {}", config.server.port);
    }

    // Workspace
    println!("\n{}", style("Workspace").bold());
    if let Some(ref ws) = config.general.workspace {
        println!("  Path: {}", ws.display());
    } else {
        println!("  Path: (current directory)");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{has_api_key_with, provider_api_key_env};
    use std::collections::HashMap;

    /// Build an environment reader over a fixed map — never touches the real process env, which
    /// is global state and unsound to mutate under a parallel test runner.
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |variable: &str| map.get(variable).cloned()
    }

    #[test]
    fn each_cloud_provider_maps_to_its_own_variable() {
        assert_eq!(provider_api_key_env("anthropic"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(provider_api_key_env("openai"), Some("OPENAI_API_KEY"));
        assert_eq!(
            provider_api_key_env("openrouter"),
            Some("OPENROUTER_API_KEY")
        );
    }

    #[test]
    fn ollama_needs_no_key() {
        assert_eq!(provider_api_key_env("ollama"), None);
        // The local-first default must never be blocked on a credential it does not use.
        assert!(has_api_key_with("ollama", None, env_of(&[])));
    }

    #[test]
    fn an_unknown_provider_falls_back_to_anthropic() {
        assert_eq!(
            provider_api_key_env("something-else"),
            Some("ANTHROPIC_API_KEY")
        );
        assert!(!has_api_key_with("something-else", None, env_of(&[])));
    }

    #[test]
    fn the_selected_providers_variable_satisfies_the_check() {
        // The bug: only ANTHROPIC_API_KEY was ever consulted, so these read as "missing".
        assert!(has_api_key_with(
            "openai",
            None,
            env_of(&[("OPENAI_API_KEY", "sk-x")])
        ));
        assert!(has_api_key_with(
            "openrouter",
            None,
            env_of(&[("OPENROUTER_API_KEY", "or-x")])
        ));
    }

    #[test]
    fn another_providers_variable_does_not_satisfy_the_check() {
        // Having an Anthropic key does not let an OpenAI-configured install run.
        assert!(!has_api_key_with(
            "openai",
            None,
            env_of(&[("ANTHROPIC_API_KEY", "sk-ant")])
        ));
    }

    #[test]
    fn an_explicitly_configured_key_wins_over_the_environment() {
        assert!(has_api_key_with(
            "openai",
            Some("sk-configured"),
            env_of(&[])
        ));
    }

    #[test]
    fn blank_values_are_not_credentials() {
        assert!(!has_api_key_with("openai", Some("   "), env_of(&[])));
        assert!(!has_api_key_with(
            "openai",
            None,
            env_of(&[("OPENAI_API_KEY", "")])
        ));
        assert!(!has_api_key_with(
            "openai",
            None,
            env_of(&[("OPENAI_API_KEY", "  ")])
        ));
    }

    #[test]
    fn a_missing_variable_reads_as_unconfigured() {
        assert!(!has_api_key_with("anthropic", None, env_of(&[])));
    }
}
