//! End-to-end regression test for the unserved-tool heal.
//!
//! Some Ollama templates/parsers (observed live 2026-08-08: ministral-3:8b,
//! Mistral family) hard-reject a GENERATED tool call whose name is absent
//! from the request's `tools` array: the API returns HTTP 500 with
//! `{"error":"tool 'exec' not found"}` and the call never reaches the
//! registry, so the normal unknown-tool guidance can never fire. Under the
//! two-tier tool system the model routinely knows canonical names (they are
//! in the system prompt) before it has discovered them, so this killed a
//! whole task: every retry of the identical request died identically until
//! the harness abandoned it (smoke run, task #5).
//!
//! The heal in `loop_runner` treats the provider's rejection as the
//! discovery signal it is: activate the named tool, rebuild the request's
//! tool definitions, and re-send. This test drives the REAL agent loop and
//! LLM client against a scripted local HTTP stub that plays the failing
//! provider, and asserts the exact contract:
//!
//! 1. the first request does NOT serve `exec` (the failure precondition),
//! 2. the healed retry DOES serve `exec`,
//! 3. the previously-rejected call then executes for real,
//! 4. the activation is reported on the response, so a harness run
//!    persists it for every later step.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use nanna_agent::{Agent, AgentConfig, RunOptions};
use nanna_llm::LlmClient;
use nanna_tools::{
    ParameterType, Tool, ToolDefinition, ToolError, ToolParameter, ToolRegistry, ToolResult,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A stand-in `exec` with the real tool's name and calling shape; the heal
/// only needs the registry to resolve the name and serve a definition.
struct StubExec;

#[async_trait]
impl Tool for StubExec {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("exec", "Run a shell command.").param(ToolParameter {
            name: "command".to_string(),
            description: "The command to run".to_string(),
            param_type: ParameterType::String,
            required: true,
            default: None,
            enum_values: None,
        })
    }

    async fn execute(&self, _params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success("hi"))
    }
}

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

async fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = if status == 200 {
        "OK"
    } else {
        "Internal Server Error"
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_unserved_tool_rejection_heals_by_activation() {
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
                respond(&mut socket, 200, "{}").await;
                continue;
            }
            chat_requests += 1;
            bodies_for_server.lock().await.push(body);
            match chat_requests {
                // The live failure, verbatim: the model generated a call to
                // `exec`, the request did not serve it, the provider refused
                // the whole exchange.
                1 => {
                    respond(&mut socket, 500, r#"{"error":"tool 'exec' not found"}"#).await;
                }
                // With the definition served, the same generation parses.
                2 => {
                    respond(
                        &mut socket,
                        200,
                        r#"{"model":"stub","message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"exec","arguments":{"command":"echo hi"}}}]},"done":true,"done_reason":"stop","prompt_eval_count":20,"eval_count":8}"#,
                    )
                    .await;
                }
                // Wrap up: a plain text turn ends the run.
                _ => {
                    respond(
                        &mut socket,
                        200,
                        r#"{"model":"stub","message":{"role":"assistant","content":"ran echo hi: hi"},"done":true,"done_reason":"stop","prompt_eval_count":20,"eval_count":8}"#,
                    )
                    .await;
                }
            }
        }
    });

    let registry = Arc::new(ToolRegistry::new());
    registry.register(StubExec).await;

    let config = AgentConfig {
        model: "stub-model:1b".to_string(),
        ..Default::default()
    };
    let llm = Arc::new(LlmClient::ollama(format!("http://{addr}")));
    let agent = Agent::new(config, llm, registry);

    // Restricted mode with an empty scope: the core set applies and none of
    // it is registered here, so the first request serves NO tools — the
    // exact precondition of the live failure.
    let options = RunOptions {
        restrict_to_active_tools: true,
        ..Default::default()
    };
    let response = agent
        .run("Run `echo hi` and tell me its output.", options)
        .await
        .expect("the heal must absorb the provider's unserved-tool rejection");

    let bodies = chat_bodies.lock().await;
    assert!(
        bodies.len() >= 2,
        "expected the rejected request plus at least one healed retry, saw {}",
        bodies.len()
    );
    assert!(
        !bodies[0].contains(r#""name":"exec""#),
        "precondition broken: the first request already served exec, so the \
         heal was never exercised"
    );
    assert!(
        bodies[1].contains(r#""name":"exec""#),
        "the healed retry must carry the exec definition"
    );
    assert!(
        response.tool_calls.iter().any(|call| call.name == "exec"),
        "the previously-rejected call must actually execute after the heal"
    );
    assert!(
        response.active_tools.iter().any(|tool| tool == "exec"),
        "the activation must be reported so a harness run persists it \
         across steps"
    );
}
