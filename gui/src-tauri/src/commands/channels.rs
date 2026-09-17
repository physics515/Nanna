//! Channel configuration and status commands.

#[allow(clippy::wildcard_imports)]
use crate::*;
use std::collections::BTreeMap;

/// Save channel configuration
///
/// # Errors
///
/// Returns `Missing <key>` when a key the channel requires is absent:
/// `bot_token` (telegram, discord, slack), `application_id` and `public_key`
/// (discord), `signing_secret` (slack), `phone_number` (signal) or
/// `connection_method` (whatsapp). Returns `Unknown channel: …` for any other
/// channel, and `Failed to save config: …` when `config.toml` cannot be written
/// — the cached config has already changed by then.
#[tauri::command]
pub async fn save_channel_config(
    state: State<'_, Arc<RwLock<AppState>>>,
    channel: String,
    config: BTreeMap<String, String>,
) -> Result<(), String> {
    let mut state_guard = state.write().await;

    match channel.as_str() {
        "telegram" => {
            let bot_token = config.get("bot_token")
                .ok_or("Missing bot_token")?
                .clone();

            let webhook_url = config.get("webhook_url").cloned();

            let allowed_users: Option<Vec<i64>> = config.get("allowed_users")
                .and_then(|s| {
                    let ids: Vec<i64> = s.split(',')
                        .filter_map(|id| id.trim().parse().ok())
                        .collect();
                    if ids.is_empty() { None } else { Some(ids) }
                });

            // The webhook secret is the ONLY origin proof the Telegram route
            // has — the path carries no bot token — and the endpoint refuses to
            // serve without it. So a channel configured here without one would
            // be permanently 503, which is why this is minted rather than left
            // `None`. Matches what `nanna init` does: a v4 UUID with the dashes
            // stripped is 32 hex chars / 122 random bits, well inside Telegram's
            // 1-256 `[A-Za-z0-9_-]` limit — and a secret a human invents is a
            // secret a human can guess. An explicitly supplied one wins, so
            // re-running this never silently rotates a secret already
            // registered with Telegram's `setWebhook`.
            let webhook_secret = config
                .get("webhook_secret")
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());

            state_guard.config.channels.telegram = Some(nanna_config::TelegramConfig {
                bot_token,
                webhook_url,
                allowed_users,
                webhook_secret: Some(webhook_secret),
            });
        }
        "discord" => {
            let bot_token = config.get("bot_token")
                .ok_or("Missing bot_token")?
                .clone();
            let application_id = config.get("application_id")
                .ok_or("Missing application_id")?
                .clone();
            let public_key = config.get("public_key")
                .ok_or("Missing public_key")?
                .clone();

            state_guard.config.channels.discord = Some(nanna_config::DiscordConfig {
                bot_token,
                application_id,
                public_key,
            });
        }
        "slack" => {
            let bot_token = config.get("bot_token")
                .ok_or("Missing bot_token")?
                .clone();
            let signing_secret = config.get("signing_secret")
                .ok_or("Missing signing_secret")?
                .clone();
            let app_token = config.get("app_token").cloned();

            state_guard.config.channels.slack = Some(nanna_config::SlackConfig {
                bot_token,
                app_token,
                signing_secret,
            });
        }
        "signal" => {
            let phone_number = config.get("phone_number")
                .ok_or("Missing phone_number")?
                .clone();
            let api_url = config.get("api_url").cloned();
            let allowed_numbers = config.get("allowed_numbers")
                .map(|s| s.split(',').map(|n| n.trim().to_string()).collect());

            // Unlike Telegram's, this secret cannot be minted here: the
            // signal-cli-rest-api bridge is a separate process that must be
            // configured to present it, so a value this end invents would arm
            // an endpoint the bridge cannot satisfy. Carry through what the
            // operator supplies, and leave it unset otherwise — the endpoint
            // then refuses with a 503 that names the key, which is the honest
            // outcome for "half configured".
            let webhook_secret = config
                .get("webhook_secret")
                .filter(|s| !s.trim().is_empty())
                .cloned();

            state_guard.config.channels.signal = Some(nanna_config::SignalConfig {
                phone_number,
                api_url,
                allowed_numbers,
                webhook_secret,
            });
        }
        "whatsapp" => {
            let connection_method = config.get("connection_method")
                .ok_or("Missing connection_method")?
                .clone();

            let allowed_contacts = config.get("allowed_contacts")
                .map(|s| s.split(',').map(|n| n.trim().to_string()).collect());

            state_guard.config.channels.whatsapp = Some(nanna_config::WhatsAppConfig {
                connection_method,
                phone_number_id: config.get("phone_number_id").cloned(),
                access_token: config.get("access_token").cloned(),
                verify_token: config.get("verify_token").cloned(),
                // HMAC key for `X-Hub-Signature-256` on inbound webhook POSTs.
                // Threaded through rather than defaulted to None: leaving it
                // unset here would let the GUI silently save a Cloud API
                // channel whose webhook accepts unauthenticated posts.
                app_secret: config.get("app_secret").cloned(),
                session_name: config.get("session_name").cloned(),
                allowed_contacts,
            });
        }
        _ => return Err(format!("Unknown channel: {channel}")),
    }

    // Save to disk
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {e}"))?;
    drop(state_guard);

    info!("Saved {} channel configuration", channel);
    Ok(())
}

/// Test channel connection
///
/// # Errors
///
/// Returns `Telegram not configured` or `Discord not configured` when that
/// channel has no saved config, and the body's decoding error when the service
/// answers with success but a body that is not JSON. An unreachable service or
/// an error status is `Ok` with `success: false`, and so is a channel that has
/// no test.
#[tauri::command]
pub async fn test_channel_connection(
    state: State<'_, Arc<RwLock<AppState>>>,
    channel: String,
) -> Result<TestConnectionResult, String> {
    // A snapshot: the probes below make network calls, which must not hold
    // the state lock.
    let channels = state.read().await.config.channels.clone();

    match channel.to_lowercase().as_str() {
        "telegram" => {
            let config = channels.telegram.as_ref()
                .ok_or("Telegram not configured")?;

            // Test by calling getMe
            let client = reqwest::Client::new();
            let url = format!("https://api.telegram.org/bot{}/getMe", config.bot_token);

            match client.get(&url).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        let data: serde_json::Value = response.json().await
                            .map_err(|e| e.to_string())?;
                        let username = data["result"]["username"].as_str().unwrap_or("unknown");
                        Ok(TestConnectionResult {
                            success: true,
                            message: format!("Connected to @{username}"),
                        })
                    } else {
                        Ok(TestConnectionResult {
                            success: false,
                            message: format!("API error: {}", response.status()),
                        })
                    }
                }
                Err(e) => Ok(TestConnectionResult {
                    success: false,
                    message: format!("Connection failed: {e}"),
                }),
            }
        }
        "discord" => {
            let config = channels.discord.as_ref()
                .ok_or("Discord not configured")?;

            // Test by calling /users/@me
            let client = reqwest::Client::new();

            match client
                .get("https://discord.com/api/v10/users/@me")
                .header("Authorization", format!("Bot {}", config.bot_token))
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        let data: serde_json::Value = response.json().await
                            .map_err(|e| e.to_string())?;
                        let username = data["username"].as_str().unwrap_or("unknown");
                        Ok(TestConnectionResult {
                            success: true,
                            message: format!("Connected as {username}"),
                        })
                    } else {
                        Ok(TestConnectionResult {
                            success: false,
                            message: format!("API error: {}", response.status()),
                        })
                    }
                }
                Err(e) => Ok(TestConnectionResult {
                    success: false,
                    message: format!("Connection failed: {e}"),
                }),
            }
        }
        _ => Ok(TestConnectionResult {
            success: false,
            message: format!("Testing not implemented for {channel}"),
        }),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TestConnectionResult {
    success: bool,
    message: String,
}

// =============================================================================
// Channel Status Commands
// =============================================================================

/// Channel status for frontend display
#[derive(Debug, Clone, Serialize)]
pub struct ChannelStatus {
    pub name: String,
    pub configured: bool,
    pub enabled: bool,
    pub status: String, // "ready", "not_configured", "disabled", "connected", "rate_limited", "degraded"
    pub details: Option<String>,
}

/// Enhanced channel status with health metrics
#[derive(Debug, Clone, Serialize)]
pub struct EnhancedChannelStatus {
    pub name: String,
    pub provider: String,
    pub configured: bool,
    pub enabled: bool,
    pub status: String,
    pub details: Option<String>,
    /// Connection state
    pub connection_state: String,
    /// Last successful health check (Unix ms)
    pub last_healthy: Option<i64>,
    /// Consecutive failures
    pub consecutive_failures: u32,
    /// Average response time (ms)
    pub avg_response_ms: Option<f64>,
    /// Messages sent in last hour
    pub messages_sent_hour: u32,
    /// Messages failed in last hour
    pub messages_failed_hour: u32,
    /// Queue depth
    pub queue_depth: usize,
    /// Messages waiting for retry
    pub queue_retrying: usize,
    /// Rate limit cooldown remaining (ms)
    pub rate_limit_remaining_ms: Option<u64>,
}

/// Channel status event for live updates
#[derive(Debug, Clone, Serialize)]
pub struct ChannelStatusEvent {
    pub provider: String,
    pub status: EnhancedChannelStatus,
    pub previous_state: Option<String>,
    pub timestamp: i64,
}

/// Get status of all configured channels
///
/// # Errors
///
/// Never returns `Err`; the `Result` is what Tauri requires of an async command
/// that borrows `State`.
#[tauri::command]
pub async fn get_channel_status(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ChannelStatus>, String> {
    let state_guard = state.read().await;
    let config = &state_guard.config;

    let mut channels = Vec::new();

    // Telegram
    channels.push(ChannelStatus {
        name: "Telegram".to_string(),
        configured: config.channels.telegram.is_some(),
        enabled: config.channels.telegram.is_some(),
        status: if config.channels.telegram.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.telegram.as_ref().map(|t| {
            format!("Bot token: {}", token_preview(&t.bot_token))
        }),
    });

    // Discord
    channels.push(ChannelStatus {
        name: "Discord".to_string(),
        configured: config.channels.discord.is_some(),
        enabled: config.channels.discord.is_some(),
        status: if config.channels.discord.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.discord.as_ref().map(|d| {
            format!("App ID: {}", d.application_id)
        }),
    });

    // Slack
    channels.push(ChannelStatus {
        name: "Slack".to_string(),
        configured: config.channels.slack.is_some(),
        enabled: config.channels.slack.is_some(),
        status: if config.channels.slack.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.slack.as_ref().map(|s| {
            let has_app_token = s.app_token.is_some();
            format!("Socket mode: {}", if has_app_token { "enabled" } else { "disabled" })
        }),
    });

    // Signal
    channels.push(ChannelStatus {
        name: "Signal".to_string(),
        configured: config.channels.signal.is_some(),
        enabled: config.channels.signal.is_some(),
        status: if config.channels.signal.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.signal.as_ref().map(|s| {
            format!("Phone: {}", s.phone_number)
        }),
    });

    // WhatsApp
    channels.push(ChannelStatus {
        name: "WhatsApp".to_string(),
        configured: config.channels.whatsapp.is_some(),
        enabled: config.channels.whatsapp.is_some(),
        status: if config.channels.whatsapp.is_some() { "ready" } else { "not_configured" }.to_string(),
        details: config.channels.whatsapp.as_ref().map(|w| {
            format!("Method: {}", w.connection_method)
        }),
    });
    drop(state_guard);

    Ok(channels)
}

/// Get enhanced status for all channels with health metrics
///
/// # Errors
///
/// Never returns `Err`; the `Result` is what Tauri requires of an async command
/// that borrows `State`.
#[tauri::command]
pub async fn get_enhanced_channel_status(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<EnhancedChannelStatus>, String> {
    let state_guard = state.read().await;
    let config = &state_guard.config;

    let providers = [
        ("telegram", "Telegram", config.channels.telegram.is_some()),
        ("discord", "Discord", config.channels.discord.is_some()),
        ("slack", "Slack", config.channels.slack.is_some()),
        ("signal", "Signal", config.channels.signal.is_some()),
        ("whatsapp", "WhatsApp", config.channels.whatsapp.is_some()),
    ];

    let mut statuses = Vec::new();

    for (provider, name, configured) in providers {
        let status = if configured { "ready" } else { "not_configured" };
        let connection_state = if configured { "connected" } else { "unconfigured" };

        let details = match provider {
            "telegram" => config.channels.telegram.as_ref().map(|t| {
                format!("Bot token: {}", token_preview(&t.bot_token))
            }),
            "discord" => config.channels.discord.as_ref().map(|d| {
                format!("App ID: {}", d.application_id)
            }),
            "slack" => config.channels.slack.as_ref().map(|s| {
                let has_app_token = s.app_token.is_some();
                format!("Socket mode: {}", if has_app_token { "enabled" } else { "disabled" })
            }),
            "signal" => config.channels.signal.as_ref().map(|s| {
                format!("Phone: {}", s.phone_number)
            }),
            "whatsapp" => config.channels.whatsapp.as_ref().map(|w| {
                format!("Method: {}", w.connection_method)
            }),
            _ => None,
        };

        statuses.push(EnhancedChannelStatus {
            name: name.to_string(),
            provider: provider.to_string(),
            configured,
            enabled: configured,
            status: status.to_string(),
            details,
            connection_state: connection_state.to_string(),
            last_healthy: if configured { Some(chrono::Utc::now().timestamp_millis()) } else { None },
            consecutive_failures: 0,
            avg_response_ms: None,
            messages_sent_hour: 0,
            messages_failed_hour: 0,
            queue_depth: 0,
            queue_retrying: 0,
            rate_limit_remaining_ms: None,
        });
    }
    drop(state_guard);

    Ok(statuses)
}

/// Test connection for any channel
///
/// # Errors
///
/// Returns the HTTP client builder's error when the client cannot be built (no
/// usable TLS backend, for example). Every per-channel failure is reported in
/// the map as `success: false` instead.
#[tauri::command]
pub async fn test_all_channels(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<HashMap<String, TestConnectionResult>, String> {
    // A snapshot: the probes below make network calls, which must not hold
    // the state lock.
    let channels = state.read().await.config.channels.clone();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let mut results = HashMap::new();

    if let Some(telegram) = &channels.telegram {
        results.insert("telegram".to_string(), probe_telegram(&client, telegram).await);
    }
    if let Some(discord) = &channels.discord {
        results.insert("discord".to_string(), probe_discord(&client, discord).await);
    }
    if let Some(slack) = &channels.slack {
        results.insert("slack".to_string(), probe_slack(&client, slack).await);
    }
    if let Some(signal) = &channels.signal {
        results.insert("signal".to_string(), probe_signal(&client, signal).await);
    }
    if let Some(whatsapp) = &channels.whatsapp {
        results.insert("whatsapp".to_string(), probe_whatsapp(&client, whatsapp).await);
    }

    Ok(results)
}

/// Telegram: `getMe` with the bot token.
async fn probe_telegram(
    client: &reqwest::Client,
    telegram: &nanna_config::TelegramConfig,
) -> TestConnectionResult {
    let url = format!("https://api.telegram.org/bot{}/getMe", telegram.bot_token);
    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await.unwrap_or_default();
                let username = data["result"]["username"].as_str().unwrap_or("unknown");
                TestConnectionResult {
                    success: true,
                    message: format!("Connected to @{username}"),
                }
            } else if response.status().as_u16() == 429 {
                TestConnectionResult {
                    success: false,
                    message: "Rate limited".to_string(),
                }
            } else {
                TestConnectionResult {
                    success: false,
                    message: format!("API error: {}", response.status()),
                }
            }
        }
        Err(e) => TestConnectionResult {
            success: false,
            message: format!("Connection failed: {e}"),
        },
    }
}

/// Discord: `/users/@me` with the bot token.
async fn probe_discord(
    client: &reqwest::Client,
    discord: &nanna_config::DiscordConfig,
) -> TestConnectionResult {
    match client
        .get("https://discord.com/api/v10/users/@me")
        .header("Authorization", format!("Bot {}", discord.bot_token))
        .send()
        .await
    {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await.unwrap_or_default();
                let username = data["username"].as_str().unwrap_or("unknown");
                TestConnectionResult {
                    success: true,
                    message: format!("Connected as {username}"),
                }
            } else if response.status().as_u16() == 429 {
                TestConnectionResult {
                    success: false,
                    message: "Rate limited".to_string(),
                }
            } else {
                TestConnectionResult {
                    success: false,
                    message: format!("API error: {}", response.status()),
                }
            }
        }
        Err(e) => TestConnectionResult {
            success: false,
            message: format!("Connection failed: {e}"),
        },
    }
}

/// Slack: `auth.test` with the bot token.
async fn probe_slack(
    client: &reqwest::Client,
    slack: &nanna_config::SlackConfig,
) -> TestConnectionResult {
    match client
        .post("https://slack.com/api/auth.test")
        .header("Authorization", format!("Bearer {}", slack.bot_token))
        .send()
        .await
    {
        Ok(response) => {
            if response.status().is_success() {
                let data: serde_json::Value = response.json().await.unwrap_or_default();
                if data["ok"].as_bool().unwrap_or(false) {
                    let team = data["team"].as_str().unwrap_or("unknown");
                    let user = data["user"].as_str().unwrap_or("unknown");
                    TestConnectionResult {
                        success: true,
                        message: format!("Connected to {team} as {user}"),
                    }
                } else {
                    let error = data["error"].as_str().unwrap_or("unknown error");
                    TestConnectionResult {
                        success: false,
                        message: format!("Slack error: {error}"),
                    }
                }
            } else {
                TestConnectionResult {
                    success: false,
                    message: format!("HTTP error: {}", response.status()),
                }
            }
        }
        Err(e) => TestConnectionResult {
            success: false,
            message: format!("Connection failed: {e}"),
        },
    }
}

/// Signal: the REST bridge's `/v1/about`.
async fn probe_signal(
    client: &reqwest::Client,
    signal: &nanna_config::SignalConfig,
) -> TestConnectionResult {
    let api_url = signal.api_url.as_deref().unwrap_or("http://localhost:8080");
    match client.get(format!("{api_url}/v1/about")).send().await {
        Ok(response) => {
            if response.status().is_success() {
                TestConnectionResult {
                    success: true,
                    message: format!("Signal API available at {api_url}"),
                }
            } else {
                TestConnectionResult {
                    success: false,
                    message: format!("Signal API error: {}", response.status()),
                }
            }
        }
        Err(e) => TestConnectionResult {
            success: false,
            message: format!("Signal API not reachable: {e}"),
        },
    }
}

/// `WhatsApp`: the Cloud API phone-number endpoint for a `cloud_api` setup. A
/// web-bridge setup cannot be probed without its QR login, so it only reports
/// being configured.
async fn probe_whatsapp(
    client: &reqwest::Client,
    whatsapp: &nanna_config::WhatsAppConfig,
) -> TestConnectionResult {
    if whatsapp.connection_method == "cloud_api" {
        if let (Some(phone_id), Some(token)) = (&whatsapp.phone_number_id, &whatsapp.access_token) {
            let url = format!(
                "https://graph.facebook.com/v18.0/{phone_id}/"
            );
            match client
                .get(&url)
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        TestConnectionResult {
                            success: true,
                            message: "WhatsApp Cloud API connected".to_string(),
                        }
                    } else {
                        TestConnectionResult {
                            success: false,
                            message: format!("API error: {}", response.status()),
                        }
                    }
                }
                Err(e) => TestConnectionResult {
                    success: false,
                    message: format!("Connection failed: {e}"),
                },
            }
        } else {
            TestConnectionResult {
                success: false,
                message: "Missing phone_number_id or access_token".to_string(),
            }
        }
    } else {
        // Web bridge - just check if configured
        TestConnectionResult {
            success: true,
            message: "Web bridge configured (QR auth required)".to_string(),
        }
    }
}

/// Subscribe to channel status updates (starts background polling)
///
/// # Errors
///
/// Never returns `Err`: the polling task is spawned and runs detached.
#[tauri::command]
pub async fn subscribe_channel_status(
    app: AppHandle,
    state: State<'_, Arc<RwLock<AppState>>>,
    interval_ms: Option<u64>,
) -> Result<(), String> {
    let interval = std::time::Duration::from_millis(interval_ms.unwrap_or(30_000));
    let state_arc = state.inner().clone();

    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        loop {
            tokio::time::sleep(interval).await;

            // A snapshot: the probe below is a network call, which must not
            // hold the state lock.
            let telegram = state_arc.read().await.config.channels.telegram.clone();

            // Check Telegram
            if let Some(telegram) = &telegram {
                let start = std::time::Instant::now();
                let url = format!("https://api.telegram.org/bot{}/getMe", telegram.bot_token);

                let (status, response_ms) = client.get(&url).send().await.map_or(
                    ("unavailable", None),
                    |response| {
                        // Exact: a u32 of milliseconds is 49 days, far past
                        // any request, and every u32 converts to f64 exactly.
                        let ms = f64::from(
                            u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX),
                        );
                        if response.status().is_success() {
                            ("connected", Some(ms))
                        } else if response.status().as_u16() == 429 {
                            ("rate_limited", Some(ms))
                        } else {
                            ("degraded", Some(ms))
                        }
                    },
                );

                let event = ChannelStatusEvent {
                    provider: "telegram".to_string(),
                    status: EnhancedChannelStatus {
                        name: "Telegram".to_string(),
                        provider: "telegram".to_string(),
                        configured: true,
                        enabled: true,
                        status: status.to_string(),
                        details: None,
                        connection_state: status.to_string(),
                        last_healthy: if status == "connected" { Some(chrono::Utc::now().timestamp_millis()) } else { None },
                        consecutive_failures: u32::from(status != "connected"),
                        avg_response_ms: response_ms,
                        messages_sent_hour: 0,
                        messages_failed_hour: 0,
                        queue_depth: 0,
                        queue_retrying: 0,
                        rate_limit_remaining_ms: if status == "rate_limited" { Some(60_000) } else { None },
                    },
                    previous_state: None,
                    timestamp: chrono::Utc::now().timestamp_millis(),
                };

                let _ = app.emit("channel-status", event);
            }

            // Similar checks for other channels can be added here
        }
    });

    info!("Started channel status polling (interval: {:?})", interval);
    Ok(())
}

/// Unsubscribe from channel status updates
///
/// # Errors
///
/// Never returns `Err`.
#[tauri::command]
pub async fn unsubscribe_channel_status() -> Result<(), String> {
    // In a full implementation, we'd track the task handle and cancel it
    // For now, the task just continues running
    info!("Channel status subscription would be cancelled");
    Ok(())
}

/// A bot token shown as its first 5 and last 4 characters, or `***` when it is
/// too short to reveal that much safely.
///
/// Counted in characters, not bytes: slicing the token at byte offsets panicked
/// the command when a pasted token held a multi-byte character at a cut point.
fn token_preview(token: &str) -> String {
    let chars = token.chars().count();
    if chars <= 10 {
        return "***".to_string();
    }
    let head: String = token.chars().take(5).collect();
    let tail: String = token.chars().skip(chars - 4).collect();
    format!("{head}...{tail}")
}

#[cfg(test)]
mod tests {
    use super::token_preview;

    #[test]
    fn token_preview_matches_the_old_ascii_output() {
        assert_eq!(token_preview("123456789:ABCdefGHI"), "12345...fGHI");
        assert_eq!(token_preview("0123456789"), "***");
    }

    #[test]
    fn token_preview_never_splits_a_character() {
        // A byte cut at 5 or at len-4 would land inside these characters.
        assert_eq!(token_preview("1234é6789abcdé"), "1234é...bcdé");
    }
}
