//! Onboarding and setup wizard for Nanna.

use console::{style, Emoji};
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Password, Select};
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
    Config::default_config_path()
        .map(|p| !p.exists())
        .unwrap_or(true)
}

/// Check if API key is configured.
pub fn has_api_key(config: &Config) -> bool {
    config.llm.api_key.is_some() || std::env::var("ANTHROPIC_API_KEY").is_ok()
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

    let env_var = match config.llm.provider.as_str() {
        "openai" => "OPENAI_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        _ => "ANTHROPIC_API_KEY",
    };

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
    config.save()?;

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

    let api_key: String = Password::with_theme(&theme)
        .with_prompt("Enter your Anthropic API key")
        .interact()?;

    if api_key.is_empty() {
        anyhow::bail!("API key is required. Set ANTHROPIC_API_KEY or run 'nanna init'");
    }

    config.llm.api_key = Some(api_key);
    config.save()?;

    println!("{CHECK}API key saved.");
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
