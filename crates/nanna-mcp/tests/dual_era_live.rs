//! The dual-era client against the REAL reference SDK servers, not a fixture
//! written from our own reading of the spec.
//!
//! Ignored by default (needs `node` and the pinned packages). To run:
//!
//! ```sh
//! (cd crates/nanna-mcp/tests/fixtures/sdk-servers && npm install)
//! cargo test -p nanna-mcp --test dual_era_live -- --ignored
//! ```
#![cfg(feature = "stdio")]

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
