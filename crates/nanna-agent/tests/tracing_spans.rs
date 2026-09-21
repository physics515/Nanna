//! The agent loop's span tree (P6), asserted through the real loop.
//!
//! One run against a scripted Ollama stub: the model calls a tool, then
//! answers. The test installs a subscriber that captures every span — its
//! name, parent, and every field value it was opened or recorded with — and
//! asserts the hierarchy `agent_run → agent_iteration → {llm_call,
//! tool_call}` plus the outcome fields each span must carry when it closes.
//!
//! Driving the real loop rather than the span helpers is the point: the
//! helpers are trivial, and the failure this guards is a span opened in the
//! wrong place — an iteration span with no run parent, a tool span that is
//! never entered, an outcome recorded on a span that already closed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nanna_agent::spans::{ITERATION_SPAN, LLM_CALL_SPAN, RUN_SPAN, TOOL_CALL_SPAN};
use nanna_agent::{Agent, AgentConfig, RunOptions};
use nanna_llm::LlmClient;
use nanna_tools::{
    ParameterType, Tool, ToolDefinition, ToolError, ToolParameter, ToolRegistry, ToolResult,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

/// What the tool returns — its byte count must reach the span.
const TOOL_OUTPUT: &str = "stub tool output";

struct StubEcho;

#[async_trait]
impl Tool for StubEcho {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("echo_stub", "Echo a word back.").param(ToolParameter {
            name: "word".to_string(),
            description: "The word".to_string(),
            param_type: ParameterType::String,
            required: true,
            default: None,
            enum_values: None,
        })
    }

    async fn execute(&self, _params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success(TOOL_OUTPUT))
    }
}

/// One captured span: name, parent's name, fields, and whether it closed.
#[derive(Clone, Debug, Default)]
struct CapturedSpan {
    name: &'static str,
    parent: Option<&'static str>,
    fields: HashMap<String, String>,
    closed: bool,
}

#[derive(Default)]
struct FieldVisitor(HashMap<String, String>);

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

/// An event's message and the name of the span it fired in, if any.
type CapturedEvent = (String, Option<&'static str>);

/// Spans by id, plus every event's message with the span it fired in.
#[derive(Clone, Default)]
struct Capture {
    spans: Arc<Mutex<HashMap<u64, CapturedSpan>>>,
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl<S> Layer<S> for Capture
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        attrs.record(&mut visitor);
        let parent = ctx
            .span(id)
            .and_then(|span| span.parent())
            .map(|parent| parent.name());
        let span = CapturedSpan {
            name: attrs.metadata().name(),
            parent,
            fields: visitor.0,
            closed: false,
        };
        self.spans
            .lock()
            .expect("capture lock")
            .insert(id.into_u64(), span);
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        values.record(&mut visitor);
        if let Some(span) = self
            .spans
            .lock()
            .expect("capture lock")
            .get_mut(&id.into_u64())
        {
            span.fields.extend(visitor.0);
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let message = visitor.0.remove("message").unwrap_or_default();
        let inside = ctx.event_span(event).map(|span| span.name());
        self.events
            .lock()
            .expect("capture lock")
            .push((message, inside));
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        if let Some(span) = self
            .spans
            .lock()
            .expect("capture lock")
            .get_mut(&id.into_u64())
        {
            span.closed = true;
        }
    }
}

async fn read_http_request(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let request_line = headers.lines().next().unwrap_or("").to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let body_start = header_end + 4;
    while buf.len() < body_start + content_length {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body_end = (body_start + content_length).min(buf.len());
    Some((
        request_line,
        String::from_utf8_lossy(&buf[body_start..body_end]).to_string(),
    ))
}

async fn respond(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Serve: first chat request → one `echo_stub` call; every later one → a
/// plain answer that ends the run.
async fn serve_tool_then_answer(listener: TcpListener) {
    let mut chat_requests = 0usize;
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            break;
        };
        let Some((request_line, _body)) = read_http_request(&mut socket).await else {
            continue;
        };
        if !request_line.contains("/api/chat") {
            respond(&mut socket, "{}").await;
            continue;
        }
        chat_requests += 1;
        let reply = if chat_requests == 1 {
            r#"{"model":"stub","message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"echo_stub","arguments":{"word":"hi"}}}]},"done":true,"done_reason":"stop","prompt_eval_count":20,"eval_count":8}"#
        } else {
            r#"{"model":"stub","message":{"role":"assistant","content":"The tool said: stub tool output."},"done":true,"done_reason":"stop","prompt_eval_count":30,"eval_count":9}"#
        };
        respond(&mut socket, reply).await;
    }
}

fn spans_named(spans: &[CapturedSpan], name: &str) -> Vec<CapturedSpan> {
    spans.iter().filter(|s| s.name == name).cloned().collect()
}

#[tokio::test(flavor = "current_thread")]
async fn a_run_opens_the_documented_span_tree_and_records_its_outcomes() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    // Thread-local default: the current-thread runtime polls the loop, the
    // stub server and the tool on this one thread, so all of it is seen.
    let _guard = tracing::subscriber::set_default(subscriber);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    tokio::spawn(serve_tool_then_answer(listener));

    let registry = Arc::new(ToolRegistry::new());
    registry.register(StubEcho).await;
    let config = AgentConfig {
        model: "span-tree-stub-model:1b".to_string(),
        ..Default::default()
    };
    let agent = Agent::new(
        config,
        Arc::new(LlmClient::ollama(format!("http://{addr}"))),
        registry,
    );
    agent
        .run("Echo hi with the tool.", RunOptions::default())
        .await
        .expect("the stubbed run must complete");

    let spans: Vec<CapturedSpan> = capture
        .spans
        .lock()
        .expect("capture lock")
        .values()
        .cloned()
        .collect();

    // agent_run: exactly one, closed, carrying how the run ended.
    let runs = spans_named(&spans, RUN_SPAN);
    assert_eq!(runs.len(), 1, "one run, one agent_run span: {runs:?}");
    let run = &runs[0];
    assert!(run.closed, "the run span must close when run() returns");
    assert_eq!(
        run.fields.get("outcome").map(String::as_str),
        Some("responded")
    );
    assert_eq!(run.fields.get("iterations").map(String::as_str), Some("2"));
    assert_eq!(
        run.fields.get("input_tokens").map(String::as_str),
        Some("50")
    );
    assert_eq!(
        run.fields.get("output_tokens").map(String::as_str),
        Some("17")
    );

    // agent_iteration: one per LLM round, each a child of the run.
    let iterations = spans_named(&spans, ITERATION_SPAN);
    assert_eq!(
        iterations.len(),
        2,
        "tool round + answer round: {iterations:?}"
    );
    for iteration in &iterations {
        assert_eq!(
            iteration.parent,
            Some(RUN_SPAN),
            "iteration outside its run: {iteration:?}"
        );
        assert!(iteration.closed);
    }
    let mut numbers: Vec<&str> = iterations
        .iter()
        .filter_map(|s| s.fields.get("iteration").map(String::as_str))
        .collect();
    numbers.sort_unstable();
    assert_eq!(
        numbers,
        ["1", "2"],
        "iterations are numbered from 1, matching the loop's count"
    );

    // llm_call: one per iteration, under it, with the settled outcome.
    let calls = spans_named(&spans, LLM_CALL_SPAN);
    assert_eq!(calls.len(), 2, "{calls:?}");
    for call in &calls {
        assert_eq!(
            call.parent,
            Some(ITERATION_SPAN),
            "llm_call outside an iteration: {call:?}"
        );
        assert!(call.closed);
        assert_eq!(call.fields.get("outcome").map(String::as_str), Some("ok"));
        assert_eq!(
            call.fields.get("model").map(String::as_str),
            Some("span-tree-stub-model:1b")
        );
        assert!(
            !call.fields.contains_key("served_model"),
            "served_model is recorded only when escalation moved the model: {call:?}"
        );
        assert!(
            call.fields.contains_key("latency_ms"),
            "latency must be recorded: {call:?}"
        );
    }
    let mut tool_counts: Vec<&str> = calls
        .iter()
        .filter_map(|s| s.fields.get("tool_calls").map(String::as_str))
        .collect();
    tool_counts.sort_unstable();
    assert_eq!(
        tool_counts,
        ["0", "1"],
        "the first round called one tool, the second none"
    );

    // tool_call: the one dispatched call, under its iteration, with its IO size.
    let tools = spans_named(&spans, TOOL_CALL_SPAN);
    assert_eq!(tools.len(), 1, "{tools:?}");
    let tool = &tools[0];
    assert_eq!(
        tool.parent,
        Some(ITERATION_SPAN),
        "tool_call outside an iteration: {tool:?}"
    );
    assert!(tool.closed);
    assert_eq!(
        tool.fields.get("tool").map(String::as_str),
        Some("echo_stub")
    );
    assert_eq!(tool.fields.get("success").map(String::as_str), Some("true"));
    assert_eq!(
        tool.fields.get("short_circuited").map(String::as_str),
        Some("false")
    );
    assert_eq!(
        tool.fields.get("output_bytes").map(String::as_str),
        Some(TOOL_OUTPUT.len().to_string().as_str())
    );
    assert!(tool.fields.contains_key("duration_ms"));

    // Opened is not entered: a span created but never entered would still
    // close with every field above, while the tool's own log lines ran
    // outside it. The loop logs "Executing tool" from inside the call.
    let events = capture.events.lock().expect("capture lock");
    let executing: Vec<_> = events
        .iter()
        .filter(|(m, _)| m == "Executing tool")
        .collect();
    assert_eq!(executing.len(), 1, "{executing:?}");
    assert_eq!(
        executing[0].1,
        Some(TOOL_CALL_SPAN),
        "the tool ran outside its span"
    );
}
