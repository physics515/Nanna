//! `credentials` subcommand handlers (Claude CLI OAuth).

use crate::CredentialsAction;
use nanna_config::Config;
use tracing::warn;

/// Print credentials status
fn print_credentials_status(
    manager: &nanna_config::ClaudeCredentialManager,
) {
    use nanna_config::{ClaudeCredentialManager, CredentialSource};

    println!("🔐 Claude CLI Credentials Status\n");

    if ClaudeCredentialManager::is_claude_cli_available() {
        println!("   ✓ Claude CLI installed");
    } else {
        println!("   ✗ Claude CLI not found");
        println!("     Install with: npm install -g @anthropic-ai/claude-code");
    }

    match manager.load() {
        Ok(loaded) => {
            let source = match loaded.source {
                CredentialSource::File => "file (~/.claude/.credentials.json)",
                CredentialSource::MacOsKeychain => "macOS Keychain",
                CredentialSource::WindowsCredentialManager => "Windows Credential Manager",
                CredentialSource::LinuxSecretService => "Linux Secret Service",
            };
            println!("   ✓ Credentials found ({source})");

            if let Some(secs) = loaded.credential.seconds_until_expiry() {
                if secs > 0 {
                    let hours = secs / 3600;
                    let mins = (secs % 3600) / 60;
                    if hours > 0 {
                        println!("   ⏱ Expires in {hours}h {mins}m");
                    } else {
                        println!("   ⏱ Expires in {mins}m");
                    }
                } else {
                    println!("   ⚠ Token expired ({} seconds ago)", -secs);
                    if loaded.credential.can_refresh() {
                        println!("     Run 'nanna credentials refresh' to renew");
                    }
                }
            }

            if let Some(ref sub) = loaded.credential.subscription_type {
                println!("   📋 Subscription: {sub}");
            }

            if loaded.credential.can_refresh() {
                println!("   🔄 Can auto-refresh: yes");
            } else {
                println!("   🔄 Can auto-refresh: no (no refresh token)");
            }
        }
        Err(e) => {
            println!("   ✗ No credentials found: {e}");
            println!("\n   To authenticate:");
            println!("   • Run 'nanna credentials setup' (requires Claude CLI)");
            println!("   • Or run 'claude login' directly, then 'nanna credentials import'");
        }
    }
}

/// Import Claude CLI credentials into Nanna config
async fn import_credentials(
    manager: &nanna_config::ClaudeCredentialManager,
) -> anyhow::Result<()> {
    use nanna_config::CredentialSource;

    println!("🔐 Importing Claude CLI Credentials...\n");

    match manager.load() {
        Ok(loaded) => {
            let source = match loaded.source {
                CredentialSource::File => "file",
                CredentialSource::MacOsKeychain => "macOS Keychain",
                CredentialSource::WindowsCredentialManager => "Windows Credential Manager",
                CredentialSource::LinuxSecretService => "Linux Secret Service",
            };

            if loaded.credential.is_expired() {
                println!("⚠ Warning: Token is expired");
                if loaded.credential.can_refresh() {
                    println!("   Attempting refresh...");
                    match manager.refresh_token(&loaded.credential).await {
                        Ok(new_cred) => {
                            if let Err(e) = manager.save(&new_cred, loaded.source) {
                                warn!("Failed to save refreshed token: {}", e);
                            }
                            println!("✅ Token refreshed and imported from {source}");
                            if let Some(ref sub) = new_cred.subscription_type {
                                println!("   Subscription: {sub}");
                            }
                        }
                        Err(e) => {
                            println!("❌ Refresh failed: {e}");
                            println!("   Run 'claude login' to re-authenticate");
                            return Ok(());
                        }
                    }
                } else {
                    println!("   Cannot auto-refresh (no refresh token)");
                    println!("   Run 'claude login' to re-authenticate");
                    return Ok(());
                }
            } else {
                println!("✅ Credentials imported from {source}");
                if let Some(ref sub) = loaded.credential.subscription_type {
                    println!("   Subscription: {sub}");
                }
                if let Some(secs) = loaded.credential.seconds_until_expiry() {
                    let hours = secs / 3600;
                    println!("   Expires in: {hours}h");
                }
            }

            let mut config = Config::load().unwrap_or_default();
            config.llm.anthropic_oauth_token = Some(loaded.credential.access_token.clone());
            config.llm.anthropic_use_oauth = true;
            if let Err(e) = config.save() {
                warn!("Failed to save config: {}", e);
            } else {
                println!("   Config updated to use OAuth");
            }
        }
        Err(e) => {
            println!("❌ No credentials found: {e}");
            println!("\n   Run 'claude login' first, or use 'nanna credentials setup'");
        }
    }

    Ok(())
}

/// Run interactive Claude CLI setup and import credentials
fn setup_credentials(
    manager: &nanna_config::ClaudeCredentialManager,
) {
    use nanna_config::ClaudeCredentialManager;

    println!("🔐 Setting up Claude CLI Authentication...\n");

    if !ClaudeCredentialManager::is_claude_cli_available() {
        println!("❌ Claude CLI not found");
        println!("   Install with: npm install -g @anthropic-ai/claude-code");
        return;
    }

    println!("Running 'claude setup-token'...");
    println!("This will open your browser for authentication.\n");

    if let Err(e) = ClaudeCredentialManager::run_setup_token() {
        println!("❌ Setup failed: {e}");
        return;
    }

    match manager.load() {
        Ok(loaded) => {
            let mut config = Config::load().unwrap_or_default();
            config.llm.anthropic_oauth_token = Some(loaded.credential.access_token.clone());
            config.llm.anthropic_use_oauth = true;
            if let Err(e) = config.save() {
                warn!("Failed to save config: {}", e);
            }

            println!("\n✅ Authentication complete!");
            if let Some(ref sub) = loaded.credential.subscription_type {
                println!("   Subscription: {sub}");
            }
            println!("   Nanna is now configured to use OAuth");
        }
        Err(e) => {
            println!("\n⚠ Setup completed but couldn't import credentials: {e}");
        }
    }
}

/// Refresh an existing OAuth token
async fn refresh_credentials(
    manager: &nanna_config::ClaudeCredentialManager,
) -> anyhow::Result<()> {
    println!("🔄 Refreshing OAuth Token...\n");

    match manager.load() {
        Ok(loaded) => {
            if !loaded.credential.can_refresh() {
                println!("❌ Cannot refresh: no refresh token available");
                println!("   Run 'nanna credentials setup' to re-authenticate");
                return Ok(());
            }

            match manager.refresh_token(&loaded.credential).await {
                Ok(new_cred) => {
                    if let Err(e) = manager.save(&new_cred, loaded.source) {
                        warn!("Failed to save to original source: {}", e);
                    }

                    let mut config = Config::load().unwrap_or_default();
                    config.llm.anthropic_oauth_token = Some(new_cred.access_token.clone());
                    if let Err(e) = config.save() {
                        warn!("Failed to save config: {}", e);
                    }

                    println!("✅ Token refreshed!");
                    if let Some(secs) = new_cred.seconds_until_expiry() {
                        let hours = secs / 3600;
                        println!("   New expiry: {hours}h from now");
                    }
                }
                Err(e) => {
                    println!("❌ Refresh failed: {e}");
                    println!("   You may need to re-authenticate with 'nanna credentials setup'");
                }
            }
        }
        Err(e) => {
            println!("❌ No credentials found: {e}");
        }
    }

    Ok(())
}

/// Clear stored OAuth credentials
fn clear_credentials() {
    println!("🗑 Clearing OAuth Credentials...\n");

    let mut config = Config::load().unwrap_or_default();
    config.llm.anthropic_oauth_token = None;
    config.llm.anthropic_use_oauth = false;
    if let Err(e) = config.save() {
        warn!("Failed to save config: {}", e);
    } else {
        println!("✅ Cleared OAuth token from Nanna config");
    }

    println!("\n   Note: Claude CLI credentials in ~/.claude/.credentials.json are not modified.");
    println!("   To fully log out, run 'claude logout' as well.");
}

/// Handle credentials subcommands
pub(crate) async fn handle_credentials_command(action: CredentialsAction) -> anyhow::Result<()> {
    use nanna_config::ClaudeCredentialManager;

    let manager = ClaudeCredentialManager::new();

    match action {
        CredentialsAction::Status => print_credentials_status(&manager),
        CredentialsAction::Import => import_credentials(&manager).await?,
        CredentialsAction::Setup => setup_credentials(&manager),
        CredentialsAction::Refresh => refresh_credentials(&manager).await?,
        CredentialsAction::Clear => clear_credentials(),
    }

    Ok(())
}
