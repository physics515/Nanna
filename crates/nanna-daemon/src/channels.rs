//! Channel Manager - Coordinates inbound listeners and outbound channels
//!
//! Manages all channel listeners and routes incoming messages to the agent.

use crate::control::ControlPlane;
use crate::protocol::ChatAction;
use nanna_channels::{
    ChannelId, DiscordChannel, DiscordListener, IncomingMessage, ListenerManager,
    MessageContent, MessageRouter, OutgoingMessage, SlackChannel, SlackListener, StatusManager,
    TelegramChannel, TelegramListener,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, error, info, warn};

use crate::protocol::Event;
use crate::session::{ReplyChannel, SessionManager};

/// Channel configuration from config.toml
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    #[serde(default)]
    pub discord: Option<DiscordConfig>,
    #[serde(default)]
    pub slack: Option<SlackConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    #[serde(default)]
    pub allowed_chats: Vec<i64>,
    #[serde(default)]
    pub use_webhooks: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    pub bot_token: String,
    #[serde(default)]
    pub allowed_guilds: Vec<String>,
    #[serde(default)]
    pub intents: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    pub app_token: String,
    pub bot_token: String,
    #[serde(default)]
    pub allowed_channels: Vec<String>,
}

/// Manages all channels (inbound + outbound)
pub struct ChannelManager {
    /// Listener manager for inbound messages
    listener_manager: RwLock<ListenerManager>,
    /// Message router for outbound messages — Arc so it can be shared with spawned tasks
    router: Arc<RwLock<MessageRouter>>,
    /// Control plane reference for processing messages
    control: Arc<ControlPlane>,
    /// Shared status manager reported via ChannelAction::Status
    status_manager: Arc<StatusManager>,
    /// Shutdown signal
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl ChannelManager {
    /// Create a new channel manager that owns a fresh status manager.
    pub fn new(control: Arc<ControlPlane>) -> Self {
        Self::with_status_manager(control, Arc::new(StatusManager::new()))
    }

    /// Create a channel manager that reports status through the given manager
    /// (typically the one attached to the control plane).
    pub fn with_status_manager(control: Arc<ControlPlane>, status_manager: Arc<StatusManager>) -> Self {
        Self {
            listener_manager: RwLock::new(ListenerManager::new(1000)),
            router: Arc::new(RwLock::new(MessageRouter::new())),
            control,
            status_manager,
            shutdown_tx: None,
        }
    }

    /// Shared status manager used by listeners and the control plane.
    pub fn status_manager(&self) -> Arc<StatusManager> {
        Arc::clone(&self.status_manager)
    }

    /// Configure channels from config
    pub async fn configure(&self, config: &ChannelsConfig) {
        let mut lm = self.listener_manager.write().await;
        let mut router = self.router.write().await;
        let sm = Arc::clone(&self.status_manager);

        // Configure Telegram
        if let Some(tg) = &config.telegram {
            let configured = !tg.bot_token.is_empty();
            sm.register("telegram", "Telegram", configured, configured && !tg.use_webhooks).await;

            if !tg.use_webhooks {
                let listener = TelegramListener::new(&tg.bot_token)
                    .with_allowed_chats(tg.allowed_chats.clone())
                    .with_status_manager(Arc::clone(&sm));
                
                if let Err(e) = lm.add(Arc::new(listener)).await {
                    error!("Failed to start Telegram listener: {}", e);
                } else {
                    info!("Telegram listener configured");
                }
            }

            // Register outbound channel
            router.register(
                "telegram",
                Box::new(TelegramChannel::new(&tg.bot_token)),
            );
        }

        // Configure Discord
        if let Some(dc) = &config.discord {
            let configured = !dc.bot_token.is_empty();
            sm.register("discord", "Discord", configured, configured).await;

            let mut listener = DiscordListener::new(&dc.bot_token)
                .with_allowed_guilds(dc.allowed_guilds.clone())
                .with_status_manager(Arc::clone(&sm));
            
            if let Some(intents) = dc.intents {
                listener = listener.with_intents(intents);
            }

            if let Err(e) = lm.add(Arc::new(listener)).await {
                error!("Failed to start Discord listener: {}", e);
            } else {
                info!("Discord Gateway listener configured");
            }

            // Register outbound channel
            // Note: Discord channel requires application_id for some features
            // For now we use empty string; should be configurable
            router.register(
                "discord",
                Box::new(DiscordChannel::new(&dc.bot_token, "")),
            );
        }

        // Configure Slack
        if let Some(sl) = &config.slack {
            let configured = !sl.bot_token.is_empty() && !sl.app_token.is_empty();
            sm.register("slack", "Slack", configured, configured).await;

            let listener = SlackListener::new(&sl.app_token, &sl.bot_token)
                .with_allowed_channels(sl.allowed_channels.clone())
                .with_status_manager(Arc::clone(&sm));

            if let Err(e) = lm.add(Arc::new(listener)).await {
                error!("Failed to start Slack listener: {}", e);
            } else {
                info!("Slack Socket Mode listener configured");
            }

            // Register outbound channel
            router.register(
                "slack",
                Box::new(SlackChannel::new(&sl.bot_token)),
            );
        }
    }

    /// Register an outbound channel in the router.
    ///
    /// Useful for registering webhook-sourced channels that aren't covered by
    /// the standard `configure()` path (e.g. when bot tokens come from webhook
    /// config rather than channel config).
    pub async fn register_channel(&self, name: impl Into<String>, channel: Box<dyn nanna_channels::Channel>) {
        let mut router = self.router.write().await;
        router.register(name, channel);
    }

    /// Get a clone of the shared router Arc.
    ///
    /// The returned `Arc<RwLock<MessageRouter>>` can be held by external
    /// components (e.g. the webhook processor) to send outbound messages
    /// through registered channels.
    pub fn router(&self) -> Arc<RwLock<MessageRouter>> {
        Arc::clone(&self.router)
    }

    /// Start processing incoming messages
    pub async fn start(&mut self) -> Result<(), String> {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Get the message receiver from listener manager
        let mut message_rx = {
            let mut lm = self.listener_manager.write().await;
            lm.take_receiver()
                .ok_or_else(|| "Listener manager already started".to_string())?
        };

        let control = Arc::clone(&self.control);
        // Share the router Arc with the spawned task — no ownership transfer needed
        let router = Arc::clone(&self.router);

        // Subscribed BEFORE the first message is processed, so no reply can be
        // emitted into a bus nobody is listening to yet.
        if let Some(events) = control.subscribe_events() {
            spawn_reply_forwarder(Arc::clone(&control.sessions), events, Arc::clone(&router));
        } else {
            warn!(
                "No event bus attached to the control plane; channel conversations will get no replies"
            );
        }

        // Spawn the message processing loop
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Channel manager shutting down");
                        break;
                    }
                    Some(msg) = message_rx.recv() => {
                        let router_guard = router.read().await;
                        Self::process_message(msg, &control, &router_guard).await;
                    }
                }
            }
        });

        Ok(())
    }

    /// Hand an incoming channel message to the agent.
    ///
    /// The reply is NOT read from the response here. `chat.send` is a delivery
    /// ack (P22): the turn runs in a spawned task and its answer arrives as
    /// `message_end` on the event bus, which [`spawn_reply_forwarder`] sends
    /// back through the channel the session names. Reading `content` off the
    /// ack — as this function did — answered every channel message with either
    /// an empty string or "I encountered an error processing your message."
    /// Only a refusal (no agent, a turn that could not start) is answered here,
    /// with the daemon's own reason.
    ///
    /// This is `pub` so the webhook event processor in `server.rs` can call it
    /// directly after converting a `WebhookEvent` into an `IncomingMessage`.
    pub async fn process_message(msg: IncomingMessage, control: &Arc<ControlPlane>, router: &MessageRouter) {
        let text = match &msg.content {
            MessageContent::Text { text } => text.clone(),
            _ => {
                debug!("Ignoring non-text message from {}", msg.channel.provider);
                return;
            }
        };

        // Generate deterministic session ID from channel + sender
        let session_id = format!(
            "{}:{}:{}",
            msg.channel.provider, msg.channel.id, msg.sender.id
        );
        let sender_name = msg.sender.username.as_deref().unwrap_or(&msg.sender.id);

        info!(
            "Processing {} message from {} in session {}",
            msg.channel.provider, sender_name, session_id
        );

        let route = ReplyChannel {
            provider: msg.channel.provider.clone(),
            id: msg.channel.id.clone(),
        };
        let name = format!("{} · {sender_name}", msg.channel.provider);
        control
            .sessions
            .ensure_channel_session(&session_id, &name, &route)
            .await;

        // A chat app has no settings screen: the few things a GUI user
        // changes with a click, a channel user says as a command. Handled
        // here, never sent to the model.
        if let Some(command) = parse_channel_command(&text) {
            let reply = run_channel_command(command, control, &session_id).await;
            send_reply(router, &msg, reply).await;
            return;
        }

        let action = ChatAction::Send {
            session_id: session_id.clone(),
            content: text,
            attachments: vec![],
        };
        let response = control
            .handle(&format!("channel:{}", msg.channel.provider), crate::protocol::Action::Chat(action))
            .await;

        let Some(refusal) = refusal_text(&response) else {
            debug!("Turn accepted for {session_id}; the reply follows on the event bus");
            return;
        };
        warn!("Channel turn for {session_id} was refused: {refusal}");
        send_reply(router, &msg, refusal).await;
    }

    /// Stop all listeners
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        
        let mut lm = self.listener_manager.write().await;
        lm.stop_all().await;
        info!("All channel listeners stopped");
    }

    /// List running listeners
    pub async fn list_listeners(&self) -> Vec<String> {
        let lm = self.listener_manager.read().await;
        lm.list().iter().map(|s| s.to_string()).collect()
    }

    /// Send a message through a channel
    pub async fn send(&self, channel: ChannelId, content: MessageContent) -> Result<String, String> {
        let router = self.router.read().await;
        router
            .send(OutgoingMessage {
                channel,
                content,
                reply_to: None,
            })
            .await
            .map_err(|e| e.to_string())
    }
}

/// Answer `msg` in the channel it came from.
async fn send_reply(router: &MessageRouter, msg: &IncomingMessage, text: String) {
    debug_assert!(!text.trim().is_empty(), "a reply says something");
    let outgoing = OutgoingMessage {
        channel: msg.channel.clone(),
        content: MessageContent::Text { text },
        reply_to: Some(msg.id.clone()),
    };
    if let Err(e) = router.send(outgoing).await {
        error!(
            "Failed to send response to {}:{}: {}",
            msg.channel.provider, msg.channel.id, e
        );
    }
}

/// A command a channel user typed instead of a message for the model.
#[derive(Debug, PartialEq, Eq)]
enum ChannelCommand<'a> {
    /// `/model` — say which model this conversation uses.
    ShowModel,
    /// `/model <spec>` — pin this conversation to a model.
    SetModel(&'a str),
    /// `/model default` — follow the daemon's default again.
    ClearModel,
    /// `/model a b` — more than one word; nothing is changed.
    ModelUsage,
}

/// Parse a channel command. Pure. `None` means an ordinary message.
///
/// Matches the command word exactly — `/models` or `/modeling` are messages —
/// and accepts Telegram's group form `/model@SomeBot`.
fn parse_channel_command(text: &str) -> Option<ChannelCommand<'_>> {
    let text = text.trim();
    let (head, rest) = text
        .split_once(char::is_whitespace)
        .map_or((text, ""), |(head, rest)| (head, rest.trim()));
    let command = head.split_once('@').map_or(head, |(command, _bot)| command);
    if command != "/model" {
        return None;
    }
    let mut words = rest.split_whitespace();
    let command = match (words.next(), words.next()) {
        (None, _) => ChannelCommand::ShowModel,
        (Some(_), Some(_)) => ChannelCommand::ModelUsage,
        (Some(word), None) if ["default", "reset", "clear"].contains(&word) => {
            ChannelCommand::ClearModel
        }
        (Some(spec), None) => ChannelCommand::SetModel(spec),
    };
    Some(command)
}

/// Carry out a channel command against the session; returns the reply.
async fn run_channel_command(
    command: ChannelCommand<'_>,
    control: &ControlPlane,
    session_id: &str,
) -> String {
    let sessions = &control.sessions;
    match command {
        ChannelCommand::ShowModel => {
            // A rare command; cloning the session to read one key is fine.
            let Some(session) = sessions.get(session_id).await else {
                return "This conversation could not be found, so nothing was read.".to_string();
            };
            session.chat_model().map_or_else(
                || {
                    "This conversation follows Nanna's default model. Send `/model <name>` to pin one."
                        .to_string()
                },
                |spec| {
                    format!("This conversation is pinned to `{spec}`. Send `/model default` to undo.")
                },
            )
        }
        ChannelCommand::SetModel(spec) => {
            if !sessions
                .set_chat_model(session_id, Some(spec.to_string()))
                .await
            {
                return "This conversation could not be found, so no model was pinned.".to_string();
            }
            let servable = control
                .router()
                .is_some_and(|router| router.can_handle(spec));
            model_pinned_reply(spec, servable)
        }
        ChannelCommand::ClearModel => {
            if sessions.set_chat_model(session_id, None).await {
                "This conversation follows Nanna's default model again.".to_string()
            } else {
                "This conversation could not be found, so nothing was changed.".to_string()
            }
        }
        ChannelCommand::ModelUsage => {
            "Nothing was changed: a model name is one word, e.g. `/model ollama/qwen3.5:9b`. \
             `/model` alone shows the current one; `/model default` undoes a pin."
                .to_string()
        }
    }
}

/// The reply to a model pin. Pure.
///
/// The pin is kept either way — a provider may come up before the next turn,
/// and the turn is where "can this run" is decided (see `SessionAction::SetModel`).
/// What differs is whether the reply may say it will run.
fn model_pinned_reply(spec: &str, servable: bool) -> String {
    if servable {
        format!("This conversation now uses `{spec}`.")
    } else {
        format!(
            "This conversation is now pinned to `{spec}`, but no provider Nanna has configured can \
             serve it right now, so your next message will fail until one can. Send `/model default` to undo."
        )
    }
}

/// The daemon's refusal of a channel turn, if the response is one. Pure.
///
/// A started or interjected turn is not a refusal: its reply comes later, on
/// the event bus.
fn refusal_text(response: &serde_json::Value) -> Option<String> {
    let code = response.get("error")?.as_str().unwrap_or("error");
    let message = response
        .get("message")
        .and_then(serde_json::Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .unwrap_or(code);
    Some(format!("Nanna could not answer this message: {message}"))
}

/// The text an event asks to deliver to a conversation, with its session. Pure.
///
/// A finished turn (`message_end`) and a message appended outside a turn (a
/// reminder) are replies; everything else — deltas, tool events, beats — is
/// run mechanics a chat app cannot render. An empty `message_end` (a crashed
/// or content-less turn) sends nothing rather than a blank message, which
/// several providers reject outright.
fn reply_text(event: &Event) -> Option<(&str, &str)> {
    let (session_id, content) = match event {
        Event::MessageEnd {
            session_id,
            content,
            ..
        } => (session_id, content),
        Event::SessionMessageAdded {
            session_id,
            role,
            content,
            ..
        } if role == "assistant" => (session_id, content),
        _ => return None,
    };
    (!content.trim().is_empty()).then_some((session_id.as_str(), content.as_str()))
}

/// Forward replies for channel-routed sessions back through `router`.
///
/// One forwarder per router: the webhook processor shares the channel
/// manager's router when there is one, and gets its own forwarder only when it
/// builds a standalone router — so no reply is sent twice. Sessions without a
/// reply route (GUI, CLI) are skipped; their clients read the same events.
pub fn spawn_reply_forwarder(
    sessions: Arc<SessionManager>,
    mut events: broadcast::Receiver<Event>,
    router: Arc<RwLock<MessageRouter>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(
                        skipped,
                        "Channel reply forwarder fell behind the event bus; replies in the skipped events were not sent"
                    );
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            };
            let Some((session_id, text)) = reply_text(&event) else {
                continue;
            };
            let Some(route) = sessions.reply_channel(session_id).await else {
                continue;
            };
            let outgoing = OutgoingMessage {
                channel: ChannelId::new(&route.provider, &route.id),
                content: MessageContent::Text {
                    text: text.to_string(),
                },
                reply_to: None,
            };
            let router = router.read().await;
            if let Err(e) = router.send(outgoing).await {
                error!(
                    "Failed to send reply for {session_id} to {}:{}: {e}",
                    route.provider, route.id
                );
            }
        }
        debug!("Channel reply forwarder stopped: event bus closed");
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nanna_channels::{Channel, ChannelCapabilities, ChannelError, Sender};
    use std::time::Duration;

    /// Records what a channel was asked to send, and where.
    struct RecordingChannel {
        sent: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    }

    #[async_trait]
    impl Channel for RecordingChannel {
        fn provider(&self) -> String {
            "test".to_string()
        }
        fn capabilities(&self) -> ChannelCapabilities {
            ChannelCapabilities::default()
        }
        async fn send(&self, message: OutgoingMessage) -> Result<String, ChannelError> {
            if let MessageContent::Text { text } = message.content {
                self.sent
                    .lock()
                    .expect("sent")
                    .push((message.channel.id, text));
            }
            Ok("sent-1".to_string())
        }
    }

    type Sent = Arc<std::sync::Mutex<Vec<(String, String)>>>;

    fn recording_router() -> (Arc<RwLock<MessageRouter>>, Sent) {
        let sent: Sent = Arc::default();
        let mut router = MessageRouter::new();
        router.register("test", Box::new(RecordingChannel { sent: sent.clone() }));
        (Arc::new(RwLock::new(router)), sent)
    }

    fn incoming(text: &str) -> IncomingMessage {
        IncomingMessage {
            id: "in-1".to_string(),
            channel: ChannelId::new("test", "chat-7"),
            sender: Sender {
                id: "user-3".to_string(),
                name: None,
                username: Some("ada".into()),
            },
            content: MessageContent::Text {
                text: text.to_string(),
            },
            timestamp: 0,
            reply_to: None,
        }
    }

    const SESSION: &str = "test:chat-7:user-3";

    async fn wait_for(sent: &Sent, count: usize) -> Vec<(String, String)> {
        for _ in 0..200 {
            let snapshot = sent.lock().expect("sent").clone();
            if snapshot.len() >= count {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        sent.lock().expect("sent").clone()
    }

    #[tokio::test]
    async fn a_channel_message_creates_its_routed_session_and_a_refusal_says_why() {
        let sessions = Arc::new(SessionManager::new());
        let control = Arc::new(ControlPlane::new(sessions.clone()));
        let (router, sent) = recording_router();

        ChannelManager::process_message(incoming("hello there"), &control, &*router.read().await)
            .await;

        let session = sessions
            .get(SESSION)
            .await
            .expect("the channel session exists");
        assert_eq!(session.name.as_deref(), Some("test · ada"));
        assert_eq!(session.messages.len(), 1, "the user's message was recorded");
        assert_eq!(
            sessions.reply_channel(SESSION).await,
            Some(ReplyChannel {
                provider: "test".into(),
                id: "chat-7".into()
            })
        );
        // No agent on this control plane: the channel user is told the real
        // reason, not "I encountered an error processing your message."
        let sent = sent.lock().expect("sent").clone();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0].0, "chat-7");
        assert_eq!(
            sent[0].1,
            "Nanna could not answer this message: Agent service not configured"
        );
    }

    #[tokio::test]
    async fn a_second_message_reuses_the_session() {
        let sessions = Arc::new(SessionManager::new());
        let route = ReplyChannel {
            provider: "test".into(),
            id: "chat-7".into(),
        };
        assert!(
            sessions
                .ensure_channel_session(SESSION, "test · ada", &route)
                .await
        );
        assert!(
            !sessions
                .ensure_channel_session(SESSION, "test · ada", &route)
                .await
        );
        assert_eq!(sessions.list().await.len(), 1);
    }

    #[tokio::test]
    async fn the_reply_route_survives_a_restart() {
        // A reminder a channel user set yesterday is delivered after today's
        // restart only if the route was persisted, not kept in memory.
        let storage = Arc::new(nanna_storage::Storage::in_memory().await.expect("storage"));
        let before = SessionManager::with_storage(storage.clone());
        let route = ReplyChannel {
            provider: "test".into(),
            id: "chat-7".into(),
        };
        before
            .ensure_channel_session(SESSION, "test · ada", &route)
            .await;

        let after = SessionManager::with_storage(storage);
        assert!(after.load_from_db().await >= 1);
        assert_eq!(after.reply_channel(SESSION).await, Some(route));
    }

    #[test]
    fn only_finished_turns_and_appended_assistant_messages_are_replies() {
        let end = Event::MessageEnd {
            session_id: "s".into(),
            message_id: "m".into(),
            content: "done".into(),
        };
        assert_eq!(reply_text(&end), Some(("s", "done")));
        let blank = Event::MessageEnd {
            session_id: "s".into(),
            message_id: "m".into(),
            content: " \n".into(),
        };
        assert_eq!(reply_text(&blank), None);
        let reminder = Event::SessionMessageAdded {
            session_id: "s".into(),
            message_id: "m".into(),
            role: "assistant".into(),
            content: "⏰ Reminder: stretch".into(),
        };
        assert_eq!(reply_text(&reminder), Some(("s", "⏰ Reminder: stretch")));
        let user = Event::SessionMessageAdded {
            session_id: "s".into(),
            message_id: "m".into(),
            role: "user".into(),
            content: "hi".into(),
        };
        assert_eq!(reply_text(&user), None);
        let delta = Event::MessageDelta {
            session_id: "s".into(),
            message_id: "m".into(),
            delta: "d".into(),
        };
        assert_eq!(reply_text(&delta), None);
    }

    #[test]
    fn model_commands_parse_exactly() {
        use ChannelCommand::*;
        assert_eq!(parse_channel_command("/model"), Some(ShowModel));
        assert_eq!(parse_channel_command("  /model  "), Some(ShowModel));
        assert_eq!(parse_channel_command("/model@NannaBot"), Some(ShowModel));
        assert_eq!(
            parse_channel_command("/model ollama/qwen3.5:9b"),
            Some(SetModel("ollama/qwen3.5:9b"))
        );
        assert_eq!(
            parse_channel_command("/model@NannaBot claude-opus-5"),
            Some(SetModel("claude-opus-5"))
        );
        assert_eq!(parse_channel_command("/model default"), Some(ClearModel));
        assert_eq!(parse_channel_command("/model reset"), Some(ClearModel));
        assert_eq!(
            parse_channel_command("/model the big one"),
            Some(ModelUsage)
        );
        for message in [
            "/models",
            "/modeling",
            "model x",
            "what /model do you use?",
            "",
            "hi",
        ] {
            assert_eq!(parse_channel_command(message), None, "{message:?}");
        }
    }

    #[test]
    fn a_pin_only_promises_to_run_when_a_provider_can_serve_it() {
        assert_eq!(
            model_pinned_reply("claude-opus-5", true),
            "This conversation now uses `claude-opus-5`."
        );
        let unservable = model_pinned_reply("claude-opus-5", false);
        assert!(
            unservable.contains("no provider Nanna has configured can serve it"),
            "{unservable}"
        );
        assert!(!unservable.contains("now uses"), "{unservable}");
    }

    #[tokio::test]
    async fn a_model_command_pins_and_clears_without_reaching_the_model() {
        let sessions = Arc::new(SessionManager::new());
        let control = Arc::new(ControlPlane::new(sessions.clone()));
        let (router, sent) = recording_router();
        let router = router.read().await;

        ChannelManager::process_message(incoming("/model ollama/qwen3.5:9b"), &control, &router)
            .await;
        let session = sessions.get(SESSION).await.expect("session");
        assert_eq!(session.chat_model(), Some("ollama/qwen3.5:9b"));
        assert!(
            session.messages.is_empty(),
            "a command is not a conversation turn"
        );

        ChannelManager::process_message(incoming("/model"), &control, &router).await;
        ChannelManager::process_message(incoming("/model default"), &control, &router).await;
        assert_eq!(
            sessions.get(SESSION).await.expect("session").chat_model(),
            None
        );
        ChannelManager::process_message(incoming("/model a b"), &control, &router).await;

        let replies: Vec<String> = sent
            .lock()
            .expect("sent")
            .iter()
            .map(|(_, text)| text.clone())
            .collect();
        assert_eq!(replies.len(), 4, "{replies:?}");
        // No router on this control plane: the pin is kept and the reply says
        // it cannot run yet, rather than claiming it will. (The recording
        // channel is plain text, so the router has stripped the Markdown.)
        assert!(
            replies[0].contains("pinned to ollama/qwen3.5:9b, but no provider"),
            "{}",
            replies[0]
        );
        assert!(
            replies[1].contains("is pinned to ollama/qwen3.5:9b"),
            "{}",
            replies[1]
        );
        assert!(
            replies[2].contains("follows Nanna's default model again"),
            "{}",
            replies[2]
        );
        assert!(
            replies[3].starts_with("Nothing was changed"),
            "{}",
            replies[3]
        );
    }

    #[test]
    fn a_started_or_interjected_turn_is_not_a_refusal() {
        let started = crate::control::chat_harness::started_response("m-1");
        assert_eq!(refusal_text(&started), None);
        let refused = serde_json::json!({ "error": "chat_failed", "message": "no router" });
        assert_eq!(
            refusal_text(&refused).as_deref(),
            Some("Nanna could not answer this message: no router")
        );
        let bare = serde_json::json!({ "error": "agent_unavailable" });
        assert_eq!(
            refusal_text(&bare).as_deref(),
            Some("Nanna could not answer this message: agent_unavailable")
        );
    }

    #[tokio::test]
    async fn replies_and_reminders_reach_the_channel_that_owns_the_session() {
        let sessions = Arc::new(SessionManager::new());
        let route = ReplyChannel {
            provider: "test".into(),
            id: "chat-7".into(),
        };
        sessions
            .ensure_channel_session(SESSION, "test · ada", &route)
            .await;
        let gui = sessions.create(None).await;
        let (events_tx, events_rx) = broadcast::channel(16);
        let (router, sent) = recording_router();
        let forwarder = spawn_reply_forwarder(sessions.clone(), events_rx, router);

        for (session_id, content) in [(gui.id.as_str(), "gui reply"), (SESSION, "the answer")] {
            events_tx
                .send(Event::MessageEnd {
                    session_id: session_id.into(),
                    message_id: "m".into(),
                    content: content.into(),
                })
                .expect("a receiver");
        }
        events_tx
            .send(Event::SessionMessageAdded {
                session_id: SESSION.into(),
                message_id: "r".into(),
                role: "assistant".into(),
                content: "⏰ Reminder: stretch".into(),
            })
            .expect("a receiver");

        let delivered = wait_for(&sent, 2).await;
        drop(events_tx);
        forwarder
            .await
            .expect("forwarder exits when the bus closes");
        assert_eq!(
            delivered,
            vec![
                ("chat-7".to_string(), "the answer".to_string()),
                ("chat-7".to_string(), "⏰ Reminder: stretch".to_string()),
            ],
            "the GUI session's reply is not sent to any channel"
        );
    }
}
