//! `session.ask_user` — Nanna asking the user a clarifying question mid-turn.
//!
//! Owner-requested (2026-07-24: "nanna should be able to ask clarifying
//! questions"), and her choice when to — never a checkpoint she must pass.
//!
//! **The question is a message.** It is appended to the conversation and
//! announced with `session_message_added`, so it shows in an open GUI chat and
//! — through the channel reply forwarder — arrives in Telegram/Discord/Slack
//! the same way an answer does.
//!
//! **The answer is the user's next message, taken from the live turn.** While
//! a turn is running, a message the user sends does not start a new turn; the
//! chat handler queues it for the run (`PendingMessages`) to admit at the next
//! step boundary. `ask_user` waits on exactly that queue and takes what
//! arrives, so the reply comes back as this tool call's result and the turn
//! continues with it in hand — no new plumbing between the chat handler and
//! the agent loop.
//!
//! **Bounded, and honest when nobody answers.** The wait ends after
//! `wait_secs` (default 10 min, at most 30 — the scheduler's own default
//! check-in period, past which "wait for the user" has become "stall the
//! run"). The question stays in the conversation; the tool says no answer came
//! yet, and a later reply is admitted into the run like any other message.
//! With no live turn (a direct tool call) there is no queue to wait on, so it
//! returns at once and says the answer will start a new turn.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nanna_scripting::ServiceFn;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::control::chat_harness::ChatRunRegistry;
use crate::protocol::Event;
use crate::session::SessionManager;

/// Default wait for an answer.
pub const ASK_USER_WAIT_SECS_DEFAULT: u64 = 600;

/// Longest wait for an answer: the scheduler's default heartbeat period.
pub const ASK_USER_WAIT_SECS_MAX: u64 = 1800;

/// Longest question accepted, in bytes. A clarifying question is a sentence or
/// three; the bound keeps a model from pasting a document into a chat app.
pub const ASK_USER_QUESTION_BYTES_MAX: usize = 2000;

/// How often the queue is checked. An answer shows up within this; the check
/// is a read lock on a small `Vec`.
const ANSWER_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// What `ask_user` needs from the daemon.
#[derive(Clone)]
pub struct AskUserDeps {
    pub sessions: Arc<SessionManager>,
    pub events: broadcast::Sender<Event>,
    pub chat_runs: Arc<ChatRunRegistry>,
}

/// A validated request. Pure to build.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AskRequest {
    session_id: String,
    question: String,
    wait: Duration,
}

fn parse_request(params: &Value) -> Result<AskRequest, String> {
    let question = params
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if question.is_empty() {
        return Err("Nothing was asked: `question` is empty.".into());
    }
    if question.len() > ASK_USER_QUESTION_BYTES_MAX {
        return Err(format!(
            "Nothing was asked: the question is {} bytes and at most {ASK_USER_QUESTION_BYTES_MAX} fit. \
             Ask one short, specific question.",
            question.len()
        ));
    }
    let session_id = params
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if session_id.is_empty() {
        return Err("Nothing was asked: this call is not part of a conversation.".into());
    }
    let wait_secs = crate::tasks::opt_i64(params, "wait_secs")?
        .and_then(|secs| u64::try_from(secs).ok())
        .unwrap_or(ASK_USER_WAIT_SECS_DEFAULT)
        .clamp(1, ASK_USER_WAIT_SECS_MAX);
    Ok(AskRequest {
        session_id: session_id.to_string(),
        question: question.to_string(),
        wait: Duration::from_secs(wait_secs),
    })
}

/// Build `session.ask_user` and `invariants.lift`.
#[must_use]
pub fn build_ask_user_services(deps: AskUserDeps) -> HashMap<String, ServiceFn> {
    let mut services: HashMap<String, ServiceFn> = HashMap::new();
    let ask_deps = deps.clone();
    services.insert(
        "session.ask_user".to_string(),
        Arc::new(move |params: Value| {
            let deps = ask_deps.clone();
            Box::pin(async move { ask(&deps, &params).await })
        }),
    );
    services.insert(
        "invariants.lift".to_string(),
        Arc::new(move |params: Value| {
            let deps = deps.clone();
            Box::pin(async move { lift_invariant(&deps, &params).await })
        }),
    );
    services
}

/// Lift a user-declared file prohibition — only on the user's explicit yes.
///
/// A declared invariant ("don't touch tests/") stood until its registry file
/// was deleted by hand; `write_file` told the model `ask_user` was the escape
/// hatch, and a yes changed nothing. This asks, quoting the user's own
/// sentence, and hands back the registry without that rule only for an
/// unambiguous yes ([`nanna_storage::is_explicit_yes`]).
///
/// The registry travels as text, both ways: the skill reads and writes it
/// through the script bridge, which resolves `.nanna/…` exactly as
/// `write_file`'s guard does. A service resolving the path itself found a
/// different file (found by driving the real daemon), and one that wrote
/// wherever it was pointed would be a write surface of its own.
async fn lift_invariant(deps: &AskUserDeps, params: &Value) -> Result<Value, String> {
    let existing = params
        .get("registry")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let glob = params
        .get("glob")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let matching = nanna_storage::declared_invariants_for(existing, glob);
    let Some(first) = matching.first() else {
        return Err(format!(
            "Nothing was lifted: no declared invariant covers `{glob}`."
        ));
    };
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let question = format!(
        "You said: \"{}\". Nanna asks to lift that rule for `{}`{}. Reply \"yes\" to lift it; anything else keeps it.",
        first.source,
        first.glob,
        if reason.is_empty() {
            String::new()
        } else {
            format!(" because: {reason}")
        },
    );
    let mut ask_params = json!({ "question": question, "session_id": params.get("session_id") });
    if let Some(wait) = params.get("wait_secs") {
        ask_params["wait_secs"] = wait.clone();
    }
    let answer = ask(deps, &ask_params).await?;
    let reply = answer
        .get("answer")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !nanna_storage::is_explicit_yes(reply) {
        return Ok(json!({ "lifted": false, "reply": reply, "reason": answer.get("reason") }));
    }
    let Some(updated) = nanna_storage::lift_declared_invariants(existing, glob) else {
        return Ok(json!({ "lifted": false, "reason": "already_lifted" }));
    };
    debug_assert_eq!(
        nanna_storage::declared_invariants_for(&updated, glob).len(),
        0
    );
    Ok(json!({ "lifted": true, "glob": first.glob, "count": matching.len(), "registry": updated }))
}

async fn ask(deps: &AskUserDeps, params: &Value) -> Result<Value, String> {
    let request = parse_request(params)?;
    deps.sessions
        .post_assistant_message(&deps.events, &request.session_id, request.question.clone())
        .await
        .ok_or_else(|| {
            format!(
                "Nothing was asked: conversation `{}` does not exist.",
                request.session_id
            )
        })?;

    if !deps.chat_runs.is_active(&request.session_id).await {
        return Ok(json!({
            "answered": false,
            "asked": true,
            "reason": "no_live_turn",
        }));
    }
    let pending = deps.chat_runs.pending_for(&request.session_id).await;
    let started = tokio::time::Instant::now();
    loop {
        let answers = pending.drain().await;
        if !answers.is_empty() {
            debug_assert!(answers.len() <= crate::tasks::PENDING_MESSAGES_MAX);
            return Ok(json!({
                "answered": true,
                "answer": answers.join("\n\n"),
                "waited_secs": started.elapsed().as_secs(),
            }));
        }
        if started.elapsed() >= request.wait {
            return Ok(json!({
                "answered": false,
                "asked": true,
                "reason": "no_answer_yet",
                "waited_secs": started.elapsed().as_secs(),
            }));
        }
        tokio::time::sleep(ANSWER_POLL_INTERVAL).await;
    }
}

/// Put `question` to the user of `session_id` and wait for the reply.
///
/// The wait is the default [`ASK_USER_WAIT_SECS_DEFAULT`], on the same path the
/// `ask_user` tool takes, for callers inside the daemon (an MCP server's
/// elicitation). `None` when nothing came back: no such conversation, no
/// live turn to take the reply from, or no reply in time.
pub async fn ask_in_conversation(
    deps: &AskUserDeps,
    session_id: &str,
    question: &str,
) -> Option<String> {
    let params = json!({ "session_id": session_id, "question": question });
    match ask(deps, &params).await {
        Ok(outcome) if outcome["answered"] == json!(true) => {
            outcome["answer"].as_str().map(str::to_string)
        }
        Ok(_) => None,
        Err(reason) => {
            tracing::warn!("could not put a question to the user: {reason}");
            None
        }
    }
}

/// An MCP server's form elicitation, put to the user through `ask_user` in
/// the conversation whose turn made the tool call.
pub struct McpAskUser {
    pub deps: AskUserDeps,
}

#[async_trait::async_trait]
impl nanna_mcp::Elicitor for McpAskUser {
    async fn ask(&self, question: &str) -> Option<String> {
        // The tool call runs on its turn's own task, which carries the
        // conversation; a call from outside a turn has no one to ask.
        let session_id = nanna_tools::ToolRegistry::run_session_id()?;
        ask_in_conversation(&self.deps, &session_id, question).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deps() -> (AskUserDeps, broadcast::Receiver<Event>) {
        let (events, rx) = broadcast::channel(8);
        let deps = AskUserDeps {
            sessions: Arc::new(SessionManager::new()),
            events,
            chat_runs: Arc::new(ChatRunRegistry::new()),
        };
        (deps, rx)
    }

    #[test]
    fn requests_are_validated_and_the_wait_is_bounded() {
        let ok =
            parse_request(&json!({ "question": " Which folder? ", "session_id": "s" })).unwrap();
        assert_eq!(ok.question, "Which folder?");
        assert_eq!(ok.wait, Duration::from_secs(ASK_USER_WAIT_SECS_DEFAULT));
        let long =
            parse_request(&json!({ "question": "q", "session_id": "s", "wait_secs": 99_999.0 }))
                .unwrap();
        assert_eq!(long.wait, Duration::from_secs(ASK_USER_WAIT_SECS_MAX));
        let zero =
            parse_request(&json!({ "question": "q", "session_id": "s", "wait_secs": 0 })).unwrap();
        assert_eq!(zero.wait, Duration::from_secs(1));
        assert!(
            parse_request(&json!({ "question": "", "session_id": "s" }))
                .unwrap_err()
                .contains("empty")
        );
        assert!(
            parse_request(&json!({ "question": "q" }))
                .unwrap_err()
                .contains("not part of a conversation")
        );
        let huge = "x".repeat(ASK_USER_QUESTION_BYTES_MAX + 1);
        assert!(
            parse_request(&json!({ "question": huge, "session_id": "s" }))
                .unwrap_err()
                .contains("at most")
        );
    }

    #[tokio::test]
    async fn the_next_message_of_a_live_turn_is_the_answer() {
        let (deps, mut events) = deps();
        let session = deps.sessions.create(None).await;
        assert!(
            deps.chat_runs.try_claim(&session.id).await,
            "a turn is live"
        );

        let pending = deps.chat_runs.pending_for(&session.id).await;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            pending.push("the blue one".to_string()).await;
        });
        let params = json!({ "question": "Which one?", "session_id": session.id, "wait_secs": 5 });
        let result = ask(&deps, &params).await.expect("asked");

        assert_eq!(result["answered"], true, "{result}");
        assert_eq!(result["answer"], "the blue one");
        assert!(
            deps.chat_runs
                .pending_for(&session.id)
                .await
                .is_empty()
                .await,
            "consumed, not interjected twice"
        );
        let stored = deps.sessions.get(&session.id).await.expect("session");
        assert_eq!(
            stored.messages.last().expect("question").content,
            "Which one?"
        );
        assert!(matches!(
            events.try_recv(),
            Ok(Event::SessionMessageAdded { .. })
        ));
    }

    #[tokio::test]
    async fn an_mcp_elicitation_is_asked_in_the_calling_turns_conversation() {
        use nanna_mcp::Elicitor as _;
        let (deps, _events) = deps();
        let session = deps.sessions.create(None).await;
        assert!(
            deps.chat_runs.try_claim(&session.id).await,
            "a turn is live"
        );
        let pending = deps.chat_runs.pending_for(&session.id).await;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            pending.push("teal".to_string()).await;
        });
        let asker = McpAskUser { deps: deps.clone() };
        let answer = nanna_tools::ToolRegistry::with_run_session(
            session.id.clone(),
            asker.ask("The MCP server `paint` asks: Favorite color?"),
        )
        .await;
        assert_eq!(answer.as_deref(), Some("teal"));
        let stored = deps.sessions.get(&session.id).await.expect("session");
        assert!(
            stored
                .messages
                .last()
                .expect("question")
                .content
                .contains("Favorite color?"),
            "the question is in the conversation"
        );

        // Outside any turn there is no conversation to ask in: no answer, and
        // nothing is posted anywhere.
        assert_eq!(asker.ask("orphan question").await, None);
    }

    #[tokio::test]
    async fn with_no_live_turn_it_asks_and_returns_at_once() {
        let (deps, _events) = deps();
        let session = deps.sessions.create(None).await;
        let started = std::time::Instant::now();
        let result = ask(
            &deps,
            &json!({ "question": "Which one?", "session_id": session.id, "wait_secs": 30 }),
        )
        .await
        .expect("asked");
        assert!(started.elapsed() < Duration::from_secs(2), "did not wait");
        assert_eq!(result["answered"], false);
        assert_eq!(result["reason"], "no_live_turn");
        assert_eq!(
            deps.sessions
                .get(&session.id)
                .await
                .expect("s")
                .messages
                .len(),
            1,
            "still asked"
        );
    }

    #[tokio::test]
    async fn an_unanswered_question_ends_the_wait_and_says_so() {
        let (deps, _events) = deps();
        let session = deps.sessions.create(None).await;
        deps.chat_runs.try_claim(&session.id).await;
        let result = ask(
            &deps,
            &json!({ "question": "Which one?", "session_id": session.id, "wait_secs": 1 }),
        )
        .await
        .expect("asked");
        assert_eq!(result["answered"], false);
        assert_eq!(result["reason"], "no_answer_yet");
        let missing = ask(&deps, &json!({ "question": "q", "session_id": "nope" })).await;
        assert!(missing.unwrap_err().contains("does not exist"));
    }

    fn registry_with_tests_rule() -> String {
        json!({ "version": 1, "invariants": [
            { "kind": "read_only", "glob": "tests/", "source": "don't touch tests/", "scope": "session" }
        ]})
        .to_string()
    }

    async fn lift_with_reply(reply: &str) -> Value {
        let (deps, _events) = deps();
        let session = deps.sessions.create(None).await;
        deps.chat_runs.try_claim(&session.id).await;
        let pending = deps.chat_runs.pending_for(&session.id).await;
        let reply = reply.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            pending.push(reply).await;
        });
        let params = json!({
            "session_id": session.id, "registry": registry_with_tests_rule(), "glob": "tests",
            "reason": "a test is wrong", "wait_secs": 5
        });
        let result = lift_invariant(&deps, &params).await.expect("asked");
        let asked = deps
            .sessions
            .get(&session.id)
            .await
            .expect("s")
            .messages
            .last()
            .expect("q")
            .content
            .clone();
        assert!(
            asked.contains("You said: \"don't touch tests/\""),
            "{asked}"
        );
        result
    }

    #[tokio::test]
    async fn a_yes_lifts_the_rule_and_anything_else_keeps_it() {
        let lifted = lift_with_reply("yes").await;
        assert_eq!(lifted["lifted"], true, "{lifted}");
        let updated = lifted["registry"].as_str().expect("registry returned");
        assert_eq!(
            nanna_storage::declared_invariants_for(updated, "tests").len(),
            0
        );

        let kept = lift_with_reply("yes but not the fixtures").await;
        assert_eq!(kept["lifted"], false, "{kept}");
        assert!(kept.get("registry").is_none(), "nothing to write back");
    }

    #[tokio::test]
    async fn only_a_declared_rule_can_be_lifted() {
        let (deps, _events) = deps();
        let unknown = lift_invariant(
            &deps,
            &json!({ "registry": registry_with_tests_rule(), "glob": "src" }),
        )
        .await;
        assert!(
            unknown
                .unwrap_err()
                .contains("no declared invariant covers `src`")
        );
        let empty = lift_invariant(&deps, &json!({ "glob": "tests" })).await;
        assert!(empty.is_err(), "no registry, no rule");
    }
}
