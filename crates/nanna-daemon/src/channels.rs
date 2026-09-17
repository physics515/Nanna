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
            spawn_reply_forwarder(
                Arc::clone(&control.sessions),
                events,
                Arc::clone(&router),
                Arc::clone(&control.channel_counters),
            );
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
        control.channel_counters.received(&msg.channel.provider);

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
            send_reply(router, &control.channel_counters, &msg, reply).await;
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
        send_reply(router, &control.channel_counters, &msg, refusal).await;
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
async fn send_reply(
    router: &MessageRouter,
    counters: &crate::channel_counters::ChannelCounters,
    msg: &IncomingMessage,
    text: String,
) {
    debug_assert!(!text.trim().is_empty(), "a reply says something");
    let outgoing = OutgoingMessage {
        channel: msg.channel.clone(),
        content: MessageContent::Text { text },
        reply_to: Some(msg.id.clone()),
    };
    if let Err(e) = router.send(outgoing).await {
        counters.send_failed(&msg.channel.provider);
        error!(
            "Failed to send response to {}:{}: {}",
            msg.channel.provider, msg.channel.id, e
        );
    } else {
        counters.sent(&msg.channel.provider);
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
    /// `/status` — is Nanna up, busy here, and able to answer at all.
    Status,
    /// `/stop` — cancel the turn running in this chat.
    Stop,
    /// `/new` — start this chat over: forget the conversation so far.
    New,
    /// `/help` or `/start` (what Telegram sends when a chat opens).
    Help,
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
    match command {
        "/status" => return Some(ChannelCommand::Status),
        "/help" | "/start" => return Some(ChannelCommand::Help),
        "/stop" => return Some(ChannelCommand::Stop),
        "/new" => return Some(ChannelCommand::New),
        "/model" => {}
        _ => return None,
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

/// What a chat-app user can type instead of talking to the model.
const CHANNEL_HELP: &str = "Commands:\n\
/status — whether Nanna is up, busy in this chat, and able to answer\n\
/model — the model this chat uses; /model <name> pins one; /model default undoes it\n\
/stop — stop what Nanna is working on in this chat\n\
/new — start this chat over (Nanna forgets the conversation so far; reminders and the model pin stay)\n\
/help — this list\n\
Anything else is a message to Nanna.";

/// Everything `/status` reports, gathered before it is worded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusFacts {
    uptime_secs: u64,
    turn_running: bool,
    pinned_model: Option<String>,
    providers: Vec<&'static str>,
    pending_reminders: Option<usize>,
}

/// Word `/status`. Pure.
///
/// Leads with the one fact that explains silence: no provider means no turn
/// can ever answer, which a chat user cannot otherwise tell from "busy".
fn status_text(facts: &StatusFacts) -> String {
    let hours = facts.uptime_secs / 3600;
    let minutes = (facts.uptime_secs % 3600) / 60;
    let mut lines = vec![format!("Nanna is up ({hours}h {minutes}m).")];
    if facts.providers.is_empty() {
        lines.push(
            "No model provider is configured, so no message can be answered until one is.".into(),
        );
    } else {
        lines.push(format!("Providers: {}.", facts.providers.join(", ")));
    }
    lines.push(if facts.turn_running {
        "This chat: working on your last message.".into()
    } else {
        "This chat: idle.".into()
    });
    lines.push(facts.pinned_model.as_ref().map_or_else(
        || "Model: Nanna's default.".to_string(),
        |spec| format!("Model: pinned to `{spec}`."),
    ));
    if let Some(count) = facts.pending_reminders {
        lines.push(format!("Reminders pending here: {count}."));
    }
    debug_assert!(lines.len() >= 4);
    lines.join("\n")
}

/// Gather `/status` for one session.
async fn status_facts(control: &ControlPlane, session_id: &str) -> StatusFacts {
    let pending_reminders = match control.scheduler() {
        Some(scheduler) => {
            let tasks = scheduler.read().await.list_tasks().await;
            let here = tasks.iter().filter(|task| {
                task.name == crate::reminder_service::REMINDER_TASK_NAME
                    && task.enabled
                    && task.target_session.as_deref() == Some(session_id)
            });
            Some(here.count())
        }
        None => None,
    };
    StatusFacts {
        uptime_secs: control.uptime_secs(),
        turn_running: control.chat_runs.is_active(session_id).await,
        pinned_model: control
            .sessions
            .get(session_id)
            .await
            .and_then(|session| session.chat_model().map(str::to_string)),
        providers: control.router().map_or_else(Vec::new, |router| {
            router
                .available_providers_sorted()
                .into_iter()
                .map(crate::llm_router::ProviderId::name)
                .collect()
        }),
        pending_reminders,
    }
}

/// Carry out a channel command against the session; returns the reply.
async fn run_channel_command(
    command: ChannelCommand<'_>,
    control: &Arc<ControlPlane>,
    session_id: &str,
) -> String {
    let sessions = &control.sessions;
    match command {
        ChannelCommand::Status => status_text(&status_facts(control, session_id).await),
        ChannelCommand::Help => CHANNEL_HELP.to_string(),
        ChannelCommand::Stop => {
            let response = control.handle_chat_cancel_for_channel(session_id).await;
            stop_reply(&response)
        }
        ChannelCommand::New => start_over(control, session_id).await,
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

/// The reply to `/stop`, from the daemon's cancel response. Pure.
fn stop_reply(response: &serde_json::Value) -> String {
    match response.get("status").and_then(serde_json::Value::as_str) {
        Some("cancelled") => "Stopped. Anything already written stays; send a new message to continue.".to_string(),
        Some("not_active") => "Nothing is running in this chat.".to_string(),
        _ => refusal_text(response)
            .unwrap_or_else(|| "The stop request got an answer Nanna did not recognise; nothing is known to have stopped.".to_string()),
    }
}

/// `/new`: empty the chat's conversation so the next message starts fresh.
///
/// Refused while a turn runs — clearing under a live turn would leave it
/// appending to a history that no longer holds its own question. A parked turn
/// (waiting on a provider outage) is dropped too, or it would resume into the
/// emptied conversation. The model pin and reminders belong to the chat, not to
/// the conversation, so they stay.
async fn start_over(control: &ControlPlane, session_id: &str) -> String {
    if control.chat_runs.is_active(session_id).await {
        return "Nanna is still working on your last message. Send /stop first, then /new."
            .to_string();
    }
    let dropped_park = control.chat_runs.clear_park(session_id).await.is_some();
    if !control.clear_session(session_id).await {
        return "This conversation could not be found, so nothing was cleared.".to_string();
    }
    if dropped_park {
        "Started over. The reply that was waiting for a model provider was dropped too.".to_string()
    } else {
        "Started over — your next message begins a new conversation.".to_string()
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

/// How often a running turn re-sends the chat app's "typing…" indicator.
///
/// Derived from the providers: Telegram shows a chat action for 5 s (or until
/// the bot's next message), Discord's typing lasts 10 s. Re-sending every 4 s
/// keeps the shorter one continuous; it is throttled per session, so a burst
/// of deltas costs one request per interval, not one per token.
const TYPING_REFRESH: std::time::Duration = std::time::Duration::from_secs(4);

/// What an event says about a turn's liveness in a session. Pure.
#[derive(Debug, PartialEq, Eq)]
enum TurnSignal<'a> {
    /// The turn is producing something — keep "typing…" up.
    Working(&'a str),
    /// The turn finished; its reply is being sent.
    Finished(&'a str),
}

fn turn_signal(event: &Event) -> Option<TurnSignal<'_>> {
    match event {
        Event::MessageStart { session_id, .. }
        | Event::MessageDelta { session_id, .. }
        | Event::ThinkingDelta { session_id, .. }
        | Event::StepStarted { session_id, .. }
        | Event::ToolStart { session_id, .. }
        | Event::ToolEnd { session_id, .. }
        | Event::LivenessBeat { session_id, .. } => Some(TurnSignal::Working(session_id)),
        Event::MessageEnd { session_id, .. } => Some(TurnSignal::Finished(session_id)),
        _ => None,
    }
}

/// Whether to send "typing…" now, given when it was last sent. Pure.
fn typing_due(last_sent: Option<tokio::time::Instant>, now: tokio::time::Instant) -> bool {
    last_sent.is_none_or(|last| now.saturating_duration_since(last) >= TYPING_REFRESH)
}

/// A routed session whose turn is running, as the typing keepalive sees it.
struct Typing {
    route: ReplyChannel,
    last_sent: tokio::time::Instant,
    last_event: tokio::time::Instant,
}

/// How long a turn may go without any event before the keepalive stops
/// showing "typing…" for it. Pure.
///
/// A live turn emits a liveness beat every [`crate::liveness::beat_interval_secs`]
/// even when it is otherwise silent (a long tool call), so two missed beats
/// mean the turn is gone without a `message_end` — a crash — and the chat must
/// not show "typing…" forever.
fn typing_abandon_after() -> std::time::Duration {
    std::time::Duration::from_secs(crate::liveness::beat_interval_secs().saturating_mul(2))
}

/// Send one "typing…" for `route`; failures are logged, never retried.
async fn send_typing_to(router: &RwLock<MessageRouter>, session_id: &str, route: &ReplyChannel) {
    let router = router.read().await;
    if let Some(channel) = router.get(&route.provider) {
        let target = ChannelId::new(&route.provider, &route.id);
        if let Err(e) = channel.send_typing(&target).await {
            debug!("typing indicator for {session_id} not sent: {e}");
        }
    }
}

/// Forward replies for channel-routed sessions back through `router`.
///
/// One forwarder per router: the webhook processor shares the channel
/// manager's router when there is one, and gets its own forwarder only when it
/// builds a standalone router — so no reply is sent twice. Sessions without a
/// reply route (GUI, CLI) are skipped; their clients read the same events.
///
/// While a routed session's turn runs, the chat also shows "typing…": a
/// Telegram user used to see nothing at all between sending a message and a
/// reply minutes later. A turn's first event starts it; a tick every
/// [`TYPING_REFRESH`] keeps it up through silent stretches (a long tool call);
/// `message_end`, or no event for [`typing_abandon_after`], ends it. The map
/// holds only sessions mid-turn and is bounded by the channel session count.
pub fn spawn_reply_forwarder(
    sessions: Arc<SessionManager>,
    mut events: broadcast::Receiver<Event>,
    router: Arc<RwLock<MessageRouter>>,
    counters: Arc<crate::channel_counters::ChannelCounters>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut typing: std::collections::HashMap<String, Typing> =
            std::collections::HashMap::new();
        let mut tick = tokio::time::interval(TYPING_REFRESH);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let received = tokio::select! {
                received = events.recv() => received,
                _ = tick.tick() => {
                    let now = tokio::time::Instant::now();
                    typing.retain(|_, state| {
                        now.saturating_duration_since(state.last_event) < typing_abandon_after()
                    });
                    for (session_id, state) in &mut typing {
                        if typing_due(Some(state.last_sent), now) {
                            state.last_sent = now;
                            send_typing_to(&router, session_id, &state.route).await;
                        }
                    }
                    continue;
                }
            };
            let event = match received {
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
            match turn_signal(&event) {
                Some(TurnSignal::Working(session_id)) => {
                    let now = tokio::time::Instant::now();
                    if let Some(state) = typing.get_mut(session_id) {
                        state.last_event = now;
                    } else if let Some(route) = sessions.reply_channel(session_id).await {
                        send_typing_to(&router, session_id, &route).await;
                        typing.insert(
                            session_id.to_string(),
                            Typing {
                                route,
                                last_sent: now,
                                last_event: now,
                            },
                        );
                    }
                }
                Some(TurnSignal::Finished(session_id)) => {
                    typing.remove(session_id);
                }
                None => {}
            }
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
                counters.send_failed(&route.provider);
                error!(
                    "Failed to send reply for {session_id} to {}:{}: {e}",
                    route.provider, route.id
                );
            } else {
                counters.sent(&route.provider);
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
        async fn send_typing(&self, channel_id: &ChannelId) -> Result<(), ChannelError> {
            self.sent
                .lock()
                .expect("sent")
                .push((channel_id.id.clone(), "<typing>".to_string()));
            Ok(())
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
        let counted = control.channel_counters.snapshot();
        assert_eq!(counted.len(), 1, "{counted:?}");
        assert_eq!(
            (counted[0].1.received, counted[0].1.sent),
            (1, 1),
            "{counted:?}"
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
        assert_eq!(parse_channel_command("/status"), Some(Status));
        assert_eq!(parse_channel_command("/status@NannaBot"), Some(Status));
        assert_eq!(parse_channel_command("/help"), Some(Help));
        assert_eq!(parse_channel_command("/start"), Some(Help));
        assert_eq!(parse_channel_command("/stop"), Some(Stop));
        assert_eq!(parse_channel_command("/new@NannaBot"), Some(New));
        assert_eq!(parse_channel_command("/news today"), None);
        for message in [
            "/statuses",
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
    fn status_leads_with_why_nothing_can_answer() {
        let mut facts = StatusFacts {
            uptime_secs: 3 * 3600 + 7 * 60 + 5,
            turn_running: false,
            pinned_model: None,
            providers: Vec::new(),
            pending_reminders: Some(2),
        };
        let bare = status_text(&facts);
        assert_eq!(
            bare,
            "Nanna is up (3h 7m).\nNo model provider is configured, so no message can be answered until one is.\nThis chat: idle.\nModel: Nanna's default.\nReminders pending here: 2."
        );
        facts.providers = vec!["ollama", "anthropic"];
        facts.turn_running = true;
        facts.pinned_model = Some("ollama/qwen3.5:9b".into());
        facts.pending_reminders = None;
        let busy = status_text(&facts);
        assert!(busy.contains("Providers: ollama, anthropic."), "{busy}");
        assert!(busy.contains("working on your last message"), "{busy}");
        assert!(busy.contains("pinned to `ollama/qwen3.5:9b`"), "{busy}");
        assert!(
            !busy.contains("Reminders"),
            "no scheduler, no claim: {busy}"
        );
    }

    #[tokio::test]
    async fn status_and_help_answer_through_the_channel() {
        let sessions = Arc::new(SessionManager::new());
        let control = Arc::new(ControlPlane::new(sessions.clone()));
        let (router, sent) = recording_router();
        let router = router.read().await;
        ChannelManager::process_message(incoming("/status"), &control, &router).await;
        ChannelManager::process_message(incoming("/help"), &control, &router).await;
        ChannelManager::process_message(incoming("/stop"), &control, &router).await;
        let replies: Vec<String> = sent
            .lock()
            .expect("sent")
            .iter()
            .map(|(_, text)| text.clone())
            .collect();
        assert_eq!(replies.len(), 3, "{replies:?}");
        assert!(replies[0].starts_with("Nanna is up ("), "{}", replies[0]);
        assert!(replies[2].contains("agent_unavailable"), "{}", replies[2]);
        assert!(
            replies[0].contains("No model provider is configured"),
            "{}",
            replies[0]
        );
        assert!(replies[1].starts_with("Commands:"), "{}", replies[1]);
        assert!(
            sessions
                .get(SESSION)
                .await
                .expect("session")
                .messages
                .is_empty()
        );
    }

    #[tokio::test]
    async fn new_forgets_the_conversation_but_keeps_the_pin() {
        let sessions = Arc::new(SessionManager::new());
        let (event_tx, mut events) = broadcast::channel(16);
        let control = Arc::new(ControlPlane::new(sessions.clone()).with_event_tx(event_tx));
        let (router, sent) = recording_router();
        let router = router.read().await;
        ChannelManager::process_message(incoming("/model ollama/qwen3.5:9b"), &control, &router)
            .await;
        sessions
            .add_message(
                SESSION,
                crate::session::MessageRole::User,
                "an earlier question",
            )
            .await
            .expect("message");

        assert!(
            control.chat_runs.try_claim(SESSION).await,
            "the test turn claims"
        );
        ChannelManager::process_message(incoming("/new"), &control, &router).await;
        assert_eq!(
            sessions.get(SESSION).await.expect("session").messages.len(),
            1,
            "a running turn keeps its history"
        );
        control.chat_runs.release(SESSION).await;

        ChannelManager::process_message(incoming("/new"), &control, &router).await;
        let session = sessions.get(SESSION).await.expect("session");
        assert!(session.messages.is_empty());
        assert_eq!(session.chat_model(), Some("ollama/qwen3.5:9b"));
        let replies: Vec<String> = sent
            .lock()
            .expect("sent")
            .iter()
            .map(|(_, text)| text.clone())
            .collect();
        assert_eq!(replies.len(), 3, "{replies:?}");
        assert!(replies[1].contains("Send /stop first"), "{}", replies[1]);
        assert!(replies[2].starts_with("Started over"), "{}", replies[2]);
        let cleared: Vec<Event> = std::iter::from_fn(|| events.try_recv().ok())
            .filter(|event| matches!(event, Event::SessionCleared { .. }))
            .collect();
        assert_eq!(
            cleared.len(),
            1,
            "only the clear that happened is announced: {cleared:?}"
        );
        assert_eq!(cleared[0].session_id(), Some(SESSION));
    }

    #[test]
    fn stop_says_what_happened() {
        let cancelled = serde_json::json!({ "status": "cancelled", "session_id": "s" });
        assert!(stop_reply(&cancelled).starts_with("Stopped."));
        let idle = serde_json::json!({ "status": "not_active", "session_id": "s" });
        assert_eq!(stop_reply(&idle), "Nothing is running in this chat.");
        let no_agent = serde_json::json!({ "error": "agent_unavailable" });
        assert_eq!(
            stop_reply(&no_agent),
            "Nanna could not answer this message: agent_unavailable"
        );
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

    #[test]
    fn typing_is_throttled_and_follows_the_turn() {
        let start = tokio::time::Instant::now();
        assert!(typing_due(None, start));
        assert!(!typing_due(Some(start), start + Duration::from_secs(3)));
        assert!(typing_due(Some(start), start + TYPING_REFRESH));
        assert!(
            !typing_due(Some(start + Duration::from_secs(1)), start),
            "clock skew never panics"
        );

        let beat = Event::ToolStart {
            session_id: "s".into(),
            call_id: "c".into(),
            name: "read_file".into(),
            input: serde_json::Value::Null,
            model: None,
            tokens: None,
            total_tokens: None,
        };
        assert_eq!(turn_signal(&beat), Some(TurnSignal::Working("s")));
        let end = Event::MessageEnd {
            session_id: "s".into(),
            message_id: "m".into(),
            content: String::new(),
        };
        assert_eq!(turn_signal(&end), Some(TurnSignal::Finished("s")));
        assert_eq!(turn_signal(&Event::WorkspacesChanged), None);
    }

    #[tokio::test]
    async fn a_routed_turn_shows_typing_once_per_interval_and_gui_turns_do_not() {
        let sessions = Arc::new(SessionManager::new());
        let route = ReplyChannel {
            provider: "test".into(),
            id: "chat-7".into(),
        };
        sessions
            .ensure_channel_session(SESSION, "test · ada", &route)
            .await;
        let gui = sessions.create(None).await;
        let (events_tx, events_rx) = broadcast::channel(64);
        let (router, sent) = recording_router();
        let counters = Arc::new(crate::channel_counters::ChannelCounters::default());
        let forwarder = spawn_reply_forwarder(sessions.clone(), events_rx, router, counters);

        for session_id in [gui.id.as_str(), SESSION] {
            events_tx
                .send(Event::MessageStart {
                    session_id: session_id.into(),
                    message_id: "m".into(),
                })
                .expect("a receiver");
            for delta in ["a", "b", "c"] {
                events_tx
                    .send(Event::MessageDelta {
                        session_id: session_id.into(),
                        message_id: "m".into(),
                        delta: delta.into(),
                    })
                    .expect("a receiver");
            }
        }
        events_tx
            .send(Event::MessageEnd {
                session_id: SESSION.into(),
                message_id: "m".into(),
                content: "abc".into(),
            })
            .expect("a receiver");

        let delivered = wait_for(&sent, 2).await;
        drop(events_tx);
        forwarder.await.expect("forwarder exits");
        assert_eq!(
            delivered,
            vec![
                ("chat-7".to_string(), "<typing>".to_string()),
                ("chat-7".to_string(), "abc".to_string()),
            ],
            "one typing for a burst of deltas, none for the GUI session"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn typing_stays_up_through_a_silent_tool_call_and_stops_for_a_dead_turn() {
        let sessions = Arc::new(SessionManager::new());
        let route = ReplyChannel {
            provider: "test".into(),
            id: "chat-7".into(),
        };
        sessions
            .ensure_channel_session(SESSION, "test · ada", &route)
            .await;
        let (events_tx, events_rx) = broadcast::channel(16);
        let (router, sent) = recording_router();
        let counters = Arc::new(crate::channel_counters::ChannelCounters::default());
        let forwarder = spawn_reply_forwarder(sessions.clone(), events_rx, router, counters);
        let typings = |sent: &Sent| {
            sent.lock()
                .expect("sent")
                .iter()
                .filter(|(_, text)| text == "<typing>")
                .count()
        };

        events_tx
            .send(Event::MessageStart {
                session_id: SESSION.into(),
                message_id: "m".into(),
            })
            .expect("a receiver");
        // A tool call with no events for 10 s: the tick keeps "typing…" up.
        tokio::time::sleep(Duration::from_secs(10)).await;
        let during = typings(&sent);
        assert!(
            (3..=4).contains(&during),
            "start + a refresh every 4 s over 10 s of silence, got {during}"
        );

        // No event at all for well past two beats: the turn is gone.
        tokio::time::sleep(typing_abandon_after() + Duration::from_secs(10)).await;
        let settled = typings(&sent);
        tokio::time::sleep(Duration::from_secs(20)).await;
        assert_eq!(
            typings(&sent),
            settled,
            "a turn with no events stops showing typing"
        );

        drop(events_tx);
        forwarder.await.expect("forwarder exits");
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
        let counters = Arc::new(crate::channel_counters::ChannelCounters::default());
        let forwarder =
            spawn_reply_forwarder(sessions.clone(), events_rx, router, counters.clone());

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
        let counted = counters.snapshot();
        assert_eq!(counted.len(), 1);
        assert_eq!(counted[0].1.sent, 2, "both deliveries counted: {counted:?}");
    }
}
