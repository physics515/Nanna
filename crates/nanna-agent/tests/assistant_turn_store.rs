//! Regression tests for how the loop stores the assistant turn in context.
//!
//! Two bugs, one seam. The run loop stores every LLM response into context
//! unconditionally right after processing it ("// Store assistant response").
//!
//! 1. The narration-loop, repetition, and mission-continuation branches each
//!    stored the SAME response a second time before injecting their nudge —
//!    `store_assistant_response` just pushes, it does not dedupe — so every
//!    nudged round replayed the model's broken turn to it twice, forever
//!    (context is append-only for the run). The first test drives the
//!    narration branch through the real loop against a scripted Ollama stub
//!    and asserts the follow-up request carries the narrated turn exactly
//!    once.
//!
//! 2. When EVERY structured tool call in a round had malformed JSON, the
//!    stream assembler synthesizes placeholder tool_use blocks plus paired
//!    error tool_results — but `tool_uses` is empty, so the round took the
//!    tool-free exit and the error results were silently dropped: the stored
//!    assistant turn kept tool_use blocks with no tool_result, and the model
//!    never learned its call was unparseable. The second test streams exactly
//!    that shape and asserts the retry request carries the paired error
//!    result.

use std::sync::Arc;

use nanna_agent::{Agent, AgentConfig, RunOptions};
use nanna_llm::LlmClient;
use nanna_tools::ToolRegistry;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Read one HTTP/1.1 request off the socket; returns (request_line, body).
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
    let body = String::from_utf8_lossy(&buf[body_start..body_end]).to_string();
    Some((request_line, body))
}

async fn respond(stream: &mut TcpStream, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Messages of a captured request body, whatever the wire dialect named the
/// array field.
fn request_messages(body: &str) -> Vec<Value> {
    let parsed: Value = serde_json::from_str(body).expect("captured body must be JSON");
    parsed["messages"]
        .as_array()
        .cloned()
        .expect("captured body must carry a messages array")
}

/// A nudged round must store the model's broken turn exactly ONCE. The
/// narration branch (like the repetition and mission-continuation branches)
/// used to call `store_assistant_response` again after the unconditional
/// store, so the request after the nudge carried the identical assistant
/// message twice.
#[tokio::test(flavor = "multi_thread")]
async fn a_narration_nudged_round_stores_the_assistant_turn_exactly_once() {
    // ≥4 narration phrases in <500 chars with no tool history trips
    // `detect_narration_loop` on the first tool-free round.
    const NARRATION: &str = "Let me read the config file first. Let me check the loader next. \
                             Let me verify the parsing logic. Now let me write the fix.";

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let chat_bodies: Arc<tokio::sync::Mutex<Vec<String>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let bodies_for_server = Arc::clone(&chat_bodies);
    tokio::spawn(async move {
        let mut chat_requests = 0usize;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let Some((request_line, body)) = read_http_request(&mut socket).await else {
                continue;
            };
            if !request_line.contains("/api/chat") {
                respond(&mut socket, "application/json", "{}").await;
                continue;
            }
            chat_requests += 1;
            bodies_for_server.lock().await.push(body);
            let content = if chat_requests == 1 {
                NARRATION
            } else {
                // A plain answer that trips no detector ends the run.
                "The config loader is fine as written."
            };
            let reply = format!(
                r#"{{"model":"stub","message":{{"role":"assistant","content":"{content}"}},"done":true,"done_reason":"stop","prompt_eval_count":20,"eval_count":8}}"#
            );
            respond(&mut socket, "application/json", &reply).await;
        }
    });

    let config = AgentConfig {
        model: "store-dedup-stub-model:1b".to_string(),
        ..Default::default()
    };
    let llm = Arc::new(LlmClient::ollama(format!("http://{addr}")));
    let agent = Agent::new(config, llm, Arc::new(ToolRegistry::new()));

    agent
        .run("Fix the config loader.", RunOptions::default())
        .await
        .expect("the nudged run must complete");

    let bodies = chat_bodies.lock().await;
    assert_eq!(
        bodies.len(),
        2,
        "expected the narrated round plus one post-nudge retry"
    );

    let messages = request_messages(&bodies[1]);
    // The branch actually fired: the nudge follows the stored turn.
    assert!(
        messages.iter().any(|m| m["content"]
            .as_str()
            .is_some_and(|c| c.contains("narrated tool calls instead of actually executing"))),
        "precondition broken: the narration nudge never fired, so the \
         double-store branch was not exercised"
    );
    let narrated_turns = messages
        .iter()
        .filter(|m| {
            m["role"] == "assistant"
                && m["content"]
                    .as_str()
                    .is_some_and(|c| c.contains("Now let me write the fix"))
        })
        .count();
    assert_eq!(
        narrated_turns, 1,
        "the nudged round must store the assistant turn exactly once — \
         a second copy means the branch-local store is back"
    );
}

/// A round whose structured tool calls ALL had malformed JSON must return the
/// synthesized error results to the model, paired with the stored turn's
/// placeholder tool_use blocks — not exit as if the round were tool-free and
/// drop them.
#[tokio::test(flavor = "multi_thread")]
async fn an_all_malformed_round_returns_the_error_results_to_the_model() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let chat_bodies: Arc<tokio::sync::Mutex<Vec<String>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let bodies_for_server = Arc::clone(&chat_bodies);
    tokio::spawn(async move {
        let mut chat_requests = 0usize;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let Some((request_line, body)) = read_http_request(&mut socket).await else {
                continue;
            };
            if !request_line.contains("/v1/chat/completions") {
                respond(&mut socket, "application/json", "{}").await;
                continue;
            }
            chat_requests += 1;
            bodies_for_server.lock().await.push(body);
            let sse = if chat_requests == 1 {
                // One tool call whose argument fragments concatenate to
                // brace-free garbage `heal_json` cannot salvage: the
                // assembler emits a placeholder tool_use plus a paired
                // error tool_result, and `tool_uses` stays EMPTY.
                concat!(
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_bad\",\"function\":{\"name\":\"exec\",\"arguments\":\"not json at all\"}}]},\"finish_reason\":null}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
                    "data: [DONE]\n\n"
                )
            } else {
                concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"Retrying with valid arguments.\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n"
                )
            };
            respond(&mut socket, "text/event-stream", sse).await;
        }
    });

    let config = AgentConfig {
        model: "malformed-args-stub-model:1b".to_string(),
        ..Default::default()
    };
    // ClaudeProxy speaks OpenAI-compat SSE at a caller-chosen base URL — the
    // only client shape that lets a local stub deliver raw argument fragments
    // (native Ollama re-serializes parsed JSON, so its args always heal).
    let llm = Arc::new(LlmClient::claude_proxy(format!("http://{addr}")));
    let agent = Agent::new(config, llm, Arc::new(ToolRegistry::new()));

    // `error_tool_results` only exist on the streaming path.
    let options = RunOptions {
        on_text: Some(Box::new(|_| {})),
        ..Default::default()
    };
    agent
        .run("Run the build.", options)
        .await
        .expect("the malformed round must continue the run, not wedge it");

    let bodies = chat_bodies.lock().await;
    assert_eq!(
        bodies.len(),
        2,
        "the all-malformed round must retry — a single request means the \
         round exited as tool-free and dropped the error results"
    );

    let messages = request_messages(&bodies[1]);
    // The stored assistant turn carries the placeholder call exactly once...
    let placeholder_turns = messages
        .iter()
        .filter(|m| {
            m["role"] == "assistant"
                && m["tool_calls"]
                    .as_array()
                    .is_some_and(|calls| calls.iter().any(|c| c["id"] == "call_bad"))
        })
        .count();
    assert_eq!(
        placeholder_turns, 1,
        "the assistant turn with the placeholder tool_use must be stored \
         exactly once"
    );
    // ...and the synthesized error result rides back paired with it, telling
    // the model WHAT failed so it can retry.
    assert!(
        messages.iter().any(|m| {
            m["role"] == "tool"
                && m["tool_call_id"] == "call_bad"
                && m["content"]
                    .as_str()
                    .is_some_and(|c| c.contains("malformed JSON arguments"))
        }),
        "the paired error tool_result must reach the model on the retry"
    );
}
