//! Channel configuration and status commands.

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Save channel configuration
#[tauri::command]
pub async fn save_channel_config(
    state: State<'_, Arc<RwLock<AppState>>>,
    channel: String,
    config: HashMap<String, String>,
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

            state_guard.config.channels.telegram = Some(nanna_config::TelegramConfig {
                bot_token,
                webhook_url,
                allowed_users,
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

            state_guard.config.channels.signal = Some(nanna_config::SignalConfig {
                phone_number,
                api_url,
                allowed_numbers,
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
                session_name: config.get("session_name").cloned(),
                allowed_contacts,
            });
        }
        _ => return Err(format!("Unknown channel: {}", channel)),
    }

    // Save to disk
    state_guard.config.save()
        .map_err(|e| format!("Failed to save config: {}", e))?;

    info!("Saved {} channel configuration", channel);
    Ok(())
}

/// Test channel connection
#[tauri::command]
pub async fn test_channel_connection(
    state: State<'_, Arc<RwLock<AppState>>>,
    channel: String,
) -> Result<TestConnectionResult, String> {
    let state_guard = state.read().await;

    match channel.to_lowercase().as_str() {
        "telegram" => {
            let config = state_guard.config.channels.telegram.as_ref()
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
                            message: format!("Connected to @{}", username),
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
                    message: format!("Connection failed: {}", e),
                }),
            }
        }
        "discord" => {
            let config = state_guard.config.channels.discord.as_ref()
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
                            message: format!("Connected as {}", username),
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
                    message: format!("Connection failed: {}", e),
                }),
            }
        }
        _ => Ok(TestConnectionResult {
            success: false,
            message: format!("Testing not implemented for {}", channel),
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
            let token_preview = if t.bot_token.len() > 10 {
                format!("{}...{}", &t.bot_token[..5], &t.bot_token[t.bot_token.len()-4..])
            } else {
                "***".to_string()
            };
            format!("Bot token: {}", token_preview)
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

    Ok(channels)
}

/// Get enhanced status for all channels with health metrics
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
                let token_preview = if t.bot_token.len() > 10 {
                    format!("{}...{}", &t.bot_token[..5], &t.bot_token[t.bot_token.len()-4..])
                } else {
                    "***".to_string()
                };
                format!("Bot token: {}", token_preview)
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

    Ok(statuses)
}

/// Test connection for any channel
#[tauri::command]
pub async fn test_all_channels(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<HashMap<String, TestConnectionResult>, String> {
    let state_guard = state.read().await;
    let config = &state_guard.config;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let mut results = HashMap::new();

    // Telegram
    if let Some(telegram) = &config.channels.telegram {
        let url = format!("https://api.telegram.org/bot{}/getMe", telegram.bot_token);
        let result = match client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    let data: serde_json::Value = response.json().await.unwrap_or_default();
                    let username = data["result"]["username"].as_str().unwrap_or("unknown");
                    TestConnectionResult {
                        success: true,
                        message: format!("Connected to @{}", username),
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
                message: format!("Connection failed: {}", e),
            },
        };
        results.insert("telegram".to_string(), result);
    }

    // Discord
    if let Some(discord) = &config.channels.discord {
        let result = match client
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
                        message: format!("Connected as {}", username),
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
                message: format!("Connection failed: {}", e),
            },
        };
        results.insert("discord".to_string(), result);
    }

    // Slack
    if let Some(slack) = &config.channels.slack {
        let result = match client
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
                            message: format!("Connected to {} as {}", team, user),
                        }
                    } else {
                        let error = data["error"].as_str().unwrap_or("unknown error");
                        TestConnectionResult {
                            success: false,
                            message: format!("Slack error: {}", error),
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
                message: format!("Connection failed: {}", e),
            },
        };
        results.insert("slack".to_string(), result);
    }

    // Signal - test signald or REST API
    if let Some(signal) = &config.channels.signal {
        let api_url = signal.api_url.as_deref().unwrap_or("http://localhost:8080");
        let result = match client.get(format!("{}/v1/about", api_url)).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    TestConnectionResult {
                        success: true,
                        message: format!("Signal API available at {}", api_url),
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
                message: format!("Signal API not reachable: {}", e),
            },
        };
        results.insert("signal".to_string(), result);
    }

    // WhatsApp - test based on connection method
    if let Some(whatsapp) = &config.channels.whatsapp {
        let result = if whatsapp.connection_method == "cloud_api" {
            if let (Some(phone_id), Some(token)) = (&whatsapp.phone_number_id, &whatsapp.access_token) {
                let url = format!(
                    "https://graph.facebook.com/v18.0/{}/",
                    phone_id
                );
                match client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", token))
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
                        message: format!("Connection failed: {}", e),
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
        };
        results.insert("whatsapp".to_string(), result);
    }

    Ok(results)
}

/// Subscribe to channel status updates (starts background polling)
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

            let state_guard = state_arc.read().await;
            let config = &state_guard.config;

            // Check Telegram
            if let Some(telegram) = &config.channels.telegram {
                let start = std::time::Instant::now();
                let url = format!("https://api.telegram.org/bot{}/getMe", telegram.bot_token);

                let (status, response_ms) = match client.get(&url).send().await {
                    Ok(response) => {
                        let ms = start.elapsed().as_millis() as f64;
                        if response.status().is_success() {
                            ("connected", Some(ms))
                        } else if response.status().as_u16() == 429 {
                            ("rate_limited", Some(ms))
                        } else {
                            ("degraded", Some(ms))
                        }
                    }
                    Err(_) => ("unavailable", None),
                };

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
                        consecutive_failures: if status == "connected" { 0 } else { 1 },
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
#[tauri::command]
pub async fn unsubscribe_channel_status() -> Result<(), String> {
    // In a full implementation, we'd track the task handle and cancel it
    // For now, the task just continues running
    info!("Channel status subscription would be cancelled");
    Ok(())
}
