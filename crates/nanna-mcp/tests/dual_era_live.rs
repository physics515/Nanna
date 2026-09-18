//! The dual-era client against the REAL reference SDK servers, not a fixture
//! written from our own reading of the spec.
//!
//! Ignored by default (needs `node` and the pinned packages). To run:
//!
//! ```sh
//! (cd crates/nanna-mcp/tests/fixtures/sdk-servers && npm install)
//! cargo test -p nanna-mcp --test dual_era_live -- --ignored
//! ```
#![cfg(all(feature = "stdio", feature = "http"))]

use nanna_mcp::{McpClient, ProtocolEra};
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sdk-servers")
}

fn script(relative: &str) -> String {
    let path = fixture_dir().join(relative);
    assert!(
        path.exists(),
        "{} missing — run `npm install` in the fixture dir",
        path.display()
    );
    path.to_string_lossy().into_owned()
}

async fn shout(client: &McpClient<nanna_mcp::StdioTransport>) -> String {
    let result = client
        .call_tool(
            "shout",
            Some(serde_json::json!({ "text": "hello from nanna" })),
        )
        .await
        .expect("tools/call must succeed");
    serde_json::to_value(&result).expect("serializable")["content"][0]["text"]
        .as_str()
        .expect("text content")
        .to_string()
}

#[tokio::test]
#[ignore = "needs node + `npm install` in tests/fixtures/sdk-servers"]
async fn a_modern_only_sdk_server_connects_and_runs_a_tool() {
    let modern = script("modern.mjs");
    let client = McpClient::spawn("node", &[modern.as_str()])
        .await
        .expect("modern-only server must connect");
    assert_eq!(
        client.era().await,
        ProtocolEra::Modern {
            version: "2026-07-28".into()
        }
    );
    assert_eq!(
        client.server_info().await.expect("identity").name,
        "modern-fixture"
    );
    let tools = client.list_tools().await.expect("tools/list");
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["shout"]
    );
    assert_eq!(shout(&client).await, "HELLO FROM NANNA");
    client.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs node + `npm install` in tests/fixtures/sdk-servers"]
async fn a_dual_era_sdk_server_is_spoken_to_as_modern() {
    let modern = script("modern.mjs");
    let client = McpClient::spawn("node", &[modern.as_str(), "dual"])
        .await
        .expect("dual-era server must connect");
    assert_eq!(
        client.era().await,
        ProtocolEra::Modern {
            version: "2026-07-28".into()
        }
    );
    assert_eq!(shout(&client).await, "HELLO FROM NANNA");
    client.close().await.expect("close");
}

#[tokio::test]
#[ignore = "needs node + `npm install` in tests/fixtures/sdk-servers"]
async fn the_legacy_everything_server_falls_back_to_initialize() {
    let everything = script("node_modules/@modelcontextprotocol/server-everything/dist/index.js");
    let client = McpClient::spawn("node", &[everything.as_str(), "stdio"])
        .await
        .expect("legacy server must connect");
    assert_eq!(client.era().await, ProtocolEra::Legacy);
    let tools = client.list_tools().await.expect("tools/list");
    assert!(
        tools.iter().any(|t| t.name == "echo"),
        "everything server lists echo: {tools:?}"
    );
    let echoed = client
        .call_tool("echo", Some(serde_json::json!({ "message": "legacy ok" })))
        .await
        .expect("echo");
    let text = serde_json::to_value(&echoed).expect("serializable")["content"][0]["text"].clone();
    assert!(
        text.as_str().unwrap_or_default().contains("legacy ok"),
        "{text}"
    );
    client.close().await.expect("close");
}

// ---------------------------------------------------------------------------
// Streamable HTTP
// ---------------------------------------------------------------------------

/// A fixture HTTP server on a free port, killed on drop.
struct HttpServer {
    child: std::process::Child,
    url: String,
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

async fn start_http(args: &[&str], env: &[(&str, String)]) -> HttpServer {
    let port = free_port();
    let mut command = std::process::Command::new("node");
    for arg in args {
        command.arg(arg.replace("{port}", &port.to_string()));
    }
    for (key, value) in env {
        command.env(key, value.replace("{port}", &port.to_string()));
    }
    // Owned by the guard from the start, so every path (including the
    // panic below) kills and reaps it.
    let server = HttpServer {
        child: command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("node"),
        url: format!("http://127.0.0.1:{port}/mcp"),
    };
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return server;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("fixture HTTP server did not listen on {port}");
}

async fn call_text(
    client: &McpClient<nanna_mcp::StreamableHttpTransport>,
    tool: &str,
    arguments: serde_json::Value,
) -> String {
    let result = client
        .call_tool(tool, Some(arguments))
        .await
        .expect("tools/call must succeed");
    serde_json::to_value(&result).expect("serializable")["content"][0]["text"]
        .as_str()
        .expect("text content")
        .to_string()
}

#[tokio::test]
#[ignore = "needs node + `npm install` in tests/fixtures/sdk-servers"]
async fn a_modern_http_server_answering_json_and_sse_is_spoken_to_statelessly() {
    let modern = script("modern-http.mjs");
    for mode in ["json", "sse"] {
        let server = start_http(&[modern.as_str(), "{port}", mode], &[]).await;
        let client = McpClient::connect_streamable(&server.url, None)
            .await
            .expect("connect");
        assert_eq!(
            client.era().await,
            ProtocolEra::Modern {
                version: "2026-07-28".into()
            },
            "{mode}"
        );
        let tools = client.list_tools().await.expect("tools/list");
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["shout", "regional"], "{mode}");
        // The server rejects a tools/call whose Mcp-Name header is missing
        // (-32020), so success proves the routing headers are right.
        let text = call_text(&client, "shout", serde_json::json!({ "text": "over http" })).await;
        assert_eq!(text, "OVER HTTP", "{mode}");
        // `region` is annotated x-mcp-header: the call only passes the
        // server's header/body validation if Mcp-Param-Region is mirrored.
        let text = call_text(
            &client,
            "regional",
            serde_json::json!({ "region": "us-west1" }),
        )
        .await;
        assert_eq!(text, "region=us-west1", "{mode}");
        let text = call_text(
            &client,
            "regional",
            serde_json::json!({ "region": "Zürich " }),
        )
        .await;
        assert_eq!(
            text, "region=Zürich ",
            "{mode}: a non-ASCII value travels base64-wrapped"
        );
        client.close().await.expect("close");
    }
}

#[tokio::test]
#[ignore = "needs node + `npm install` in tests/fixtures/sdk-servers"]
async fn a_bearer_token_is_sent_and_a_wrong_one_is_named_not_retried_as_legacy() {
    let modern = script("modern-http.mjs");
    let server = start_http(&[modern.as_str(), "{port}", "json", "s3cret"], &[]).await;
    let client = McpClient::connect_streamable(&server.url, Some("s3cret".into()))
        .await
        .expect("connect");
    assert_eq!(
        call_text(&client, "shout", serde_json::json!({ "text": "authed" })).await,
        "AUTHED"
    );

    match McpClient::connect_streamable(&server.url, Some("wrong".into())).await {
        Err(nanna_mcp::McpError::HttpStatus { status, .. }) => assert_eq!(status, 401),
        Err(other) => panic!("expected HTTP 401, got {other:?}"),
        Ok(_) => panic!("a wrong token must not connect"),
    }
}

#[tokio::test]
#[ignore = "needs node + `npm install` in tests/fixtures/sdk-servers"]
async fn the_legacy_everything_http_server_falls_back_to_a_session() {
    let everything = script("node_modules/@modelcontextprotocol/server-everything/dist/index.js");
    let server = start_http(
        &[everything.as_str(), "streamableHttp"],
        &[("PORT", "{port}".into())],
    )
    .await;
    let client = McpClient::connect_streamable(&server.url, None)
        .await
        .expect("legacy connect");
    assert_eq!(client.era().await, ProtocolEra::Legacy);
    let echoed = call_text(
        &client,
        "echo",
        serde_json::json!({ "message": "legacy http" }),
    )
    .await;
    assert!(echoed.contains("legacy http"), "{echoed}");
    client.close().await.expect("close ends the session");
}

// ---------------------------------------------------------------------------
// Tool-list changes reach the registry
// ---------------------------------------------------------------------------

#[cfg(feature = "tools-integration")]
mod registry {
    use super::{McpClient, script, start_http};

    /// Run the manager's watch loop until `wanted` shows up in `registry` (or
    /// the deadline passes); returns whether it did.
    async fn watch_until_registered<T: nanna_mcp::Transport + 'static>(
        manager: &nanna_mcp::McpToolsManager<T>,
        registry: &nanna_tools::ToolRegistry,
        wanted: &str,
    ) -> bool {
        let (found_tx, found_rx) = tokio::sync::oneshot::channel::<()>();
        let poll = async {
            for _ in 0..200 {
                if registry.get(wanted).await.is_some() {
                    let _ = found_tx.send(());
                    return true;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            false
        };
        let stop = async {
            let _ = found_rx.await;
        };
        let (found, ()) = tokio::join!(poll, async {
            tokio::select! {
                () = manager.watch_list_changes(registry, stop) => {}
                () = tokio::time::sleep(std::time::Duration::from_secs(12)) => {}
            }
        });
        found
    }

    #[tokio::test]
    #[ignore = "needs node + `npm install` in tests/fixtures/sdk-servers"]
    async fn a_tool_added_by_a_modern_stdio_server_reaches_the_registry() {
        let modern = script("modern.mjs");
        let client = McpClient::spawn("node", &[modern.as_str(), "grow"])
            .await
            .expect("connect");
        let manager = nanna_mcp::McpToolsManager::new();
        manager.register("grow", client).await.expect("register");
        let registry = nanna_tools::ToolRegistry::new();
        manager
            .register_with_registry(&registry)
            .await
            .expect("registry");
        assert!(
            registry.get("mcp__grow__late").await.is_none(),
            "not there before the change"
        );
        assert!(
            watch_until_registered(&manager, &registry, "mcp__grow__late").await,
            "the late tool must arrive through subscriptions/listen"
        );
        assert!(
            registry.get("mcp__grow__shout").await.is_some(),
            "the old tool stays"
        );
        manager.close_all().await.expect("close");
    }

    #[tokio::test]
    #[ignore = "needs node + `npm install` in tests/fixtures/sdk-servers"]
    async fn a_tool_added_by_a_modern_http_server_reaches_the_registry() {
        let modern = script("modern-http.mjs");
        let server = start_http(&[modern.as_str(), "{port}", "json"], &[]).await;
        let client = McpClient::connect_streamable(&server.url, None)
            .await
            .expect("connect");
        let manager = nanna_mcp::McpToolsManager::new();
        manager.register("remote", client).await.expect("register");
        let registry = nanna_tools::ToolRegistry::new();
        manager
            .register_with_registry(&registry)
            .await
            .expect("registry");
        assert!(registry.get("mcp__remote__late").await.is_none());

        // Let the listen stream open, then change the server's tools.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let grow_url = server.url.replace("/mcp", "/grow");
        reqwest::Client::new()
            .post(&grow_url)
            .send()
            .await
            .expect("grow");
        assert!(
            watch_until_registered(&manager, &registry, "mcp__remote__late").await,
            "the late tool must arrive through the HTTP listen stream"
        );
        manager.close_all().await.expect("close");
    }
}
