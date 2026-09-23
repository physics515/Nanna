#![warn(clippy::pedantic, clippy::nursery, clippy::all)]
//! A 2026-07-28 MCP server built on the official Rust SDK (`rmcp`), over
//! stdio: a second, independent implementation for nanna-mcp's live interop
//! suite, next to the TypeScript SDK fixtures.
//!
//! Tools: `add` (a plain call) and `greet` (asks for a name through
//! multi-round-trip elicitation, then answers with it).

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{
        router::tool::ToolRouter,
        tool::{InputResponses, RequestState},
        wrapper::Parameters,
    },
    model::{
        CallToolResponse, CallToolResult, ContentBlock, ElicitRequest, ElicitRequestParams,
        ErrorData, InputRequest, InputRequests, InputRequiredResult, ServerCapabilities,
        ServerConfig,
    },
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;

#[derive(Deserialize, schemars::JsonSchema)]
struct AddArgs {
    a: i64,
    b: i64,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct GreetArgs {
    greeting: String,
}

#[derive(Clone)]
struct Fixture {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl Fixture {
    #[tool(description = "Add two integers")]
    fn add(&self, Parameters(AddArgs { a, b }): Parameters<AddArgs>) -> String {
        (a + b).to_string()
    }

    #[tool(description = "Greet the user by name, asking for it first")]
    async fn greet(
        &self,
        Parameters(args): Parameters<GreetArgs>,
        RequestState(state): RequestState,
        InputResponses(responses): InputResponses,
    ) -> Result<CallToolResponse, ErrorData> {
        if state.as_deref() != Some("asked-name") {
            let mut requests = InputRequests::new();
            let schema = serde_json::from_value(serde_json::json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }))
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            requests.insert(
                "name".to_string(),
                InputRequest::Elicitation(ElicitRequest::new(
                    ElicitRequestParams::FormElicitationParams {
                        meta: None,
                        message: "What is your name?".into(),
                        requested_schema: schema,
                    },
                )),
            );
            return Ok(InputRequiredResult::new(Some(requests), Some("asked-name".into())).into());
        }
        let name = responses
            .as_ref()
            .and_then(|r| r.get("name"))
            .and_then(|r| r["content"]["name"].as_str())
            .ok_or_else(|| ErrorData::invalid_params("no name in the input responses", None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "{}, {name}!",
            args.greeting
        ))])
        .into())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Fixture {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture {
        tool_router: Fixture::tool_router(),
    };
    let service = fixture.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
