//! Channel handlers for the [`ControlPlane`].

use super::*;

impl ControlPlane {
    // =========================================================================
    // Channel Handlers
    // =========================================================================
    
    pub(super) async fn handle_channel(&self, _client_id: &str, action: ChannelAction) -> Value {
        // Note: Full channel management requires ChannelManager which needs to be
        // added to daemon. For now, we read config to report available channels.
        let config = self.config.read().await;
        
        match action {
            ChannelAction::List => {
                let mut channels = vec![];
                
                // Check which channels are configured (have credentials)
                let telegram_configured = config.channels.telegram
                    .as_ref()
                    .map(|t| !t.bot_token.is_empty())
                    .unwrap_or(false);
                channels.push(json!({
                    "id": "telegram",
                    "type": "telegram",
                    "configured": telegram_configured,
                }));
                
                let discord_configured = config.channels.discord
                    .as_ref()
                    .map(|d| !d.bot_token.is_empty())
                    .unwrap_or(false);
                channels.push(json!({
                    "id": "discord",
                    "type": "discord",
                    "configured": discord_configured,
                }));
                
                let slack_configured = config.channels.slack
                    .as_ref()
                    .map(|s| !s.bot_token.is_empty())
                    .unwrap_or(false);
                channels.push(json!({
                    "id": "slack",
                    "type": "slack",
                    "configured": slack_configured,
                }));
                
                let signal_configured = config.channels.signal
                    .as_ref()
                    .map(|s| !s.phone_number.is_empty())
                    .unwrap_or(false);
                channels.push(json!({
                    "id": "signal",
                    "type": "signal",
                    "configured": signal_configured,
                }));
                
                let whatsapp_configured = config.channels.whatsapp
                    .as_ref()
                    .map(|w| w.access_token.is_some())
                    .unwrap_or(false);
                channels.push(json!({
                    "id": "whatsapp",
                    "type": "whatsapp",
                    "configured": whatsapp_configured,
                }));
                
                json!({ "channels": channels })
            }
            ChannelAction::Status { id } => {
                let Some(status_manager) = self.status_manager.as_ref() else {
                    return json!({
                        "status": "unavailable",
                        "message": "Channel status manager not attached (no channel manager running)",
                    });
                };

                match id {
                    Some(channel_id) => match status_manager.get(&channel_id).await {
                        Some(status) => json!({ "channel": status }),
                        None => json!({
                            "error": "not_found",
                            "channel_id": channel_id,
                            "message": "No status registered for this channel",
                        }),
                    },
                    None => {
                        let channels: Vec<_> = status_manager
                            .all()
                            .await
                            .into_values()
                            .collect();
                        let summary = status_manager.summary().await;
                        json!({ "channels": channels, "summary": summary })
                    }
                }
            }
            ChannelAction::Enable { id } => {
                // Would need to modify config and potentially start listener
                json!({ 
                    "status": "not_implemented",
                    "message": "Use Config::Set to enable/disable channels",
                    "id": id 
                })
            }
            ChannelAction::Disable { id } => {
                json!({ 
                    "status": "not_implemented",
                    "message": "Use Config::Set to enable/disable channels",
                    "id": id 
                })
            }
            ChannelAction::Test { id } => {
                // Would attempt to connect and send test message
                json!({ 
                    "status": "not_implemented",
                    "message": "Channel connection testing not yet implemented",
                    "id": id 
                })
            }
            ChannelAction::Send { channel_id, target, content: _ } => {
                // Would send through MessageRouter
                json!({ 
                    "status": "not_implemented",
                    "message": "Direct channel send requires MessageRouter integration",
                    "channel_id": channel_id,
                    "target": target,
                })
            }
        }
    }
}
