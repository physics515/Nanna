//! Protocol-era detection for a dual-era MCP client.
//!
//! MCP revision `2026-07-28` removed the `initialize` handshake: every request
//! carries its protocol version and client capabilities in
//! `_meta["io.modelcontextprotocol/*"]`, and servers MUST implement
//! `server/discover`. A client that only speaks the legacy handshake cannot
//! reach a modern-only server at all ("legacy clients have no fall-forward
//! mechanism"), so the stdio client probes first:
//!
//! * a `DiscoverResult` → **modern**, pick a mutually supported version;
//! * a recognized modern error (`-32022` with a `supported` list, `-32021`,
//!   `-32020`) → **modern**, and never fall back to `initialize`;
//! * any other error, or no answer within the request timeout → **legacy**.
//!
//! Everything here is pure so the whole decision table is pinned by unit tests
//! without a server process. Sources:
//! <https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning>,
//! <https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio>.

use crate::protocol::{ServerCapabilities, ServerInfo, error_codes};
use crate::{McpError, Result};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// Modern revisions this client speaks, most preferred first.
pub const MODERN_PROTOCOL_VERSIONS: &[&str] = &["2026-07-28"];

/// The one legacy (handshake) revision this client speaks.
pub const LEGACY_PROTOCOL_VERSION: &str = "2024-11-05";

/// `_meta` key carrying a modern request's protocol version.
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// `_meta` key carrying a modern request's client capabilities (required).
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
/// `_meta` key carrying the client's name and version (recommended).
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
/// `_meta` key a modern server identifies itself under, in any result.
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// Upper bound on the version lists read from a server. A real server lists a
/// handful; the cap only stops a hostile list from costing unbounded work.
const SUPPORTED_VERSIONS_MAX: usize = 64;

/// Which protocol era a connected server speaks. A property of the server
/// process, decided once at connect and cached for its lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolEra {
    /// Per-request `_meta`, no handshake; `version` is the negotiated revision.
    Modern { version: String },
    /// The `initialize` handshake at [`LEGACY_PROTOCOL_VERSION`].
    Legacy,
}

/// The body of a `server/discover` answer.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverResult {
    pub supported_versions: Vec<String>,
    #[serde(default)]
    pub capabilities: ServerCapabilities,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Map<String, Value>>,
}

impl DiscoverResult {
    /// The server's self-reported identity, if it sent one. Display only —
    /// the spec forbids keying behaviour on it.
    #[must_use]
    pub fn server_info(&self) -> Option<ServerInfo> {
        let info = self.meta.as_ref()?.get(META_SERVER_INFO)?;
        serde_json::from_value(info.clone()).ok()
    }
}

/// What the probe decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeVerdict {
    /// Modern server, and this version is one both ends speak.
    Modern { version: String },
    /// Fall back to the `initialize` handshake.
    Legacy,
    /// A modern server this client cannot talk to; `reason` is user-facing.
    Incompatible { reason: String },
}

/// Pick the revision to speak from a server's `supported` list.
///
/// A mutual modern revision wins. Otherwise a list naming our legacy revision
/// means a dual-era server that will still answer `initialize`; anything else
/// is a modern server with nothing in common.
#[must_use]
pub fn select_version(supported: &[String]) -> ProbeVerdict {
    let supported = &supported[..supported.len().min(SUPPORTED_VERSIONS_MAX)];
    let mutual = MODERN_PROTOCOL_VERSIONS
        .iter()
        .find(|ours| supported.iter().any(|theirs| theirs == *ours));
    if let Some(version) = mutual {
        debug_assert!(supported.iter().any(|theirs| theirs == *version));
        return ProbeVerdict::Modern {
            version: (*version).to_string(),
        };
    }
    if supported.iter().any(|v| v == LEGACY_PROTOCOL_VERSION) {
        return ProbeVerdict::Legacy;
    }
    ProbeVerdict::Incompatible {
        reason: format!(
            "the server speaks MCP {} and this client speaks {} or {LEGACY_PROTOCOL_VERSION}",
            if supported.is_empty() {
                "no advertised revision".to_string()
            } else {
                supported.join(", ")
            },
            MODERN_PROTOCOL_VERSIONS.join(", "),
        ),
    }
}

/// Classify the outcome of the `server/discover` probe.
///
/// `Err` is returned only for faults that say nothing about the era — the
/// process died or the pipe broke — so the caller surfaces them instead of
/// sending a handshake into a dead server.
///
/// # Errors
///
/// Returns the transport error unchanged for [`McpError::ConnectionClosed`],
/// [`McpError::Io`] and [`McpError::Transport`].
pub fn classify_probe(outcome: Result<Value>) -> Result<ProbeVerdict> {
    match outcome {
        Ok(result) => Ok(serde_json::from_value::<DiscoverResult>(result).map_or(
            // An answer that is not a DiscoverResult is not a modern server's.
            ProbeVerdict::Legacy,
            |discover| select_version(&discover.supported_versions),
        )),
        Err(McpError::JsonRpc {
            code,
            data,
            message,
        }) => Ok(classify_error(code, data.as_ref(), &message)),
        Err(error @ (McpError::ConnectionClosed | McpError::Io(_) | McpError::Transport(_))) => {
            Err(error)
        }
        // A refused credential says nothing about the era, and falling back
        // would only repeat the refusal under a less useful message.
        Err(
            error @ McpError::HttpStatus {
                status: 401 | 403, ..
            },
        ) => Err(error),
        // No answer within the request timeout (the spec's "does not respond"
        // case), an HTTP error with no modern JSON-RPC body (the Streamable
        // HTTP binding's legacy signal), or an answer we could not read: legacy.
        Err(_) => Ok(ProbeVerdict::Legacy),
    }
}

/// Classify a JSON-RPC error answer to the probe. Only the three codes the
/// 2026-07-28 revision defines identify a modern server; the fallback must not
/// key on one specific legacy code (legacy servers answer `-32601`, `-32602`,
/// or something else entirely).
fn classify_error(code: i32, data: Option<&Value>, message: &str) -> ProbeVerdict {
    match code {
        error_codes::UNSUPPORTED_PROTOCOL_VERSION => {
            let supported = data
                .and_then(|d| d.get("supported"))
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .take(SUPPORTED_VERSIONS_MAX)
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            select_version(&supported)
        }
        error_codes::MISSING_REQUIRED_CLIENT_CAPABILITY | error_codes::HEADER_MISMATCH => {
            ProbeVerdict::Incompatible {
                reason: format!("the modern MCP server refused the probe (code {code}): {message}"),
            }
        }
        _ => ProbeVerdict::Legacy,
    }
}

/// The `_meta` object every modern request carries.
///
/// # Panics
///
/// Panics if `version` is empty — a programmer error, never wire input.
#[must_use]
pub fn modern_meta(version: &str) -> Value {
    assert!(
        !version.is_empty(),
        "a modern request must name its revision"
    );
    json!({
        META_PROTOCOL_VERSION: version,
        // Nothing is declared: this client answers no server-to-client
        // requests (sampling, elicitation, roots), so it must not invite them.
        META_CLIENT_CAPABILITIES: {},
        META_CLIENT_INFO: { "name": "nanna", "version": env!("CARGO_PKG_VERSION") },
    })
}

/// Attach the modern `_meta` to a request's params, creating the params
/// object when the request had none. A caller-supplied `_meta` keeps its own
/// keys; the protocol keys are always ours.
///
/// # Errors
///
/// Returns [`McpError::Protocol`] if `params` is present but not an object —
/// JSON-RPC by-position params cannot carry `_meta`.
pub fn with_modern_meta(params: Option<Value>, version: &str) -> Result<Value> {
    let mut object = match params {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(object)) => object,
        Some(other) => {
            return Err(McpError::Protocol(format!(
                "modern MCP params must be an object, got {other}"
            )));
        }
    };
    let meta = object
        .entry("_meta")
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(meta) = meta else {
        return Err(McpError::Protocol(
            "request `_meta` must be an object".into(),
        ));
    };
    if let Value::Object(ours) = modern_meta(version) {
        meta.extend(ours);
    }
    debug_assert!(meta.contains_key(META_PROTOCOL_VERSION));
    debug_assert!(meta.contains_key(META_CLIENT_CAPABILITIES));
    Ok(Value::Object(object))
}

/// Check a modern result's `resultType`.
///
/// Absent means `complete` (older servers never send it); `input_required` is a multi-round-trip request
/// this client does not serve; any other value MUST be treated as invalid.
///
/// # Errors
///
/// Returns [`McpError::Protocol`] naming the result type for anything but
/// `complete`.
pub fn ensure_complete(result: &Value) -> Result<()> {
    match result.get("resultType").and_then(Value::as_str) {
        None | Some("complete") => Ok(()),
        Some("input_required") => Err(McpError::Protocol(
            "the MCP server asked for more input (sampling, elicitation or roots) \
             before it can answer; this client does not provide any"
                .into(),
        )),
        Some(other) => Err(McpError::Protocol(format!(
            "the MCP server answered with an unknown resultType `{other}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions(list: &[&str]) -> Vec<String> {
        list.iter().map(|v| (*v).to_string()).collect()
    }

    fn rpc_error(code: i32, data: Option<Value>) -> Result<Value> {
        Err(McpError::JsonRpc {
            code,
            message: "nope".into(),
            data,
        })
    }

    #[test]
    fn a_discover_result_with_our_revision_is_modern() {
        // Byte-for-byte the answer @modelcontextprotocol/server 2.0.0 gives.
        let answer = json!({
            "supportedVersions": ["2026-07-28"],
            "capabilities": { "tools": { "listChanged": true } },
            "resultType": "complete", "ttlMs": 0, "cacheScope": "private",
            "_meta": { META_SERVER_INFO: { "name": "modern-fixture", "version": "1.0.0" } }
        });
        let discover: DiscoverResult = serde_json::from_value(answer.clone()).unwrap();
        assert_eq!(discover.server_info().unwrap().name, "modern-fixture");
        assert!(discover.capabilities.tools.is_some());
        assert_eq!(
            classify_probe(Ok(answer)).unwrap(),
            ProbeVerdict::Modern {
                version: "2026-07-28".into()
            }
        );
    }

    #[test]
    fn unsupported_version_with_a_mutual_revision_stays_modern() {
        let data = json!({ "supported": ["2026-07-28"], "requested": "2099-01-01" });
        assert_eq!(
            classify_probe(rpc_error(-32022, Some(data))).unwrap(),
            ProbeVerdict::Modern {
                version: "2026-07-28".into()
            }
        );
    }

    #[test]
    fn unsupported_version_naming_our_legacy_revision_falls_back() {
        let data = json!({ "supported": ["2099-01-01", "2024-11-05"] });
        assert_eq!(
            classify_probe(rpc_error(-32022, Some(data))).unwrap(),
            ProbeVerdict::Legacy
        );
    }

    #[test]
    fn a_modern_server_with_nothing_in_common_is_incompatible_not_legacy() {
        // The spec's "do NOT fall back to initialize" case.
        let data = json!({ "supported": ["2099-01-01"] });
        match classify_probe(rpc_error(-32022, Some(data))).unwrap() {
            ProbeVerdict::Incompatible { reason } => {
                assert!(reason.contains("2099-01-01"), "{reason}");
            }
            other => panic!("expected incompatible, got {other:?}"),
        }
        match classify_probe(rpc_error(-32022, None)).unwrap() {
            ProbeVerdict::Incompatible { reason } => {
                assert!(reason.contains("no advertised revision"), "{reason}");
            }
            other => panic!("expected incompatible, got {other:?}"),
        }
    }

    #[test]
    fn the_other_modern_codes_never_fall_back() {
        for code in [-32021, -32020] {
            assert!(
                matches!(
                    classify_probe(rpc_error(code, None)).unwrap(),
                    ProbeVerdict::Incompatible { .. }
                ),
                "code {code}"
            );
        }
    }

    #[test]
    fn any_other_error_or_silence_is_legacy() {
        // -32601 is what @modelcontextprotocol/sdk 1.30 (server-everything) answers.
        for code in [-32601, -32602, -32600, -32000, -32002, 1] {
            assert_eq!(
                classify_probe(rpc_error(code, None)).unwrap(),
                ProbeVerdict::Legacy,
                "code {code}"
            );
        }
        assert_eq!(
            classify_probe(Err(McpError::Timeout)).unwrap(),
            ProbeVerdict::Legacy
        );
        assert_eq!(
            classify_probe(Ok(json!({ "tools": [] }))).unwrap(),
            ProbeVerdict::Legacy
        );
    }

    #[test]
    fn a_dead_server_is_an_error_not_an_era() {
        assert!(matches!(
            classify_probe(Err(McpError::ConnectionClosed)),
            Err(McpError::ConnectionClosed)
        ));
    }

    #[test]
    fn preference_order_is_ours_not_the_servers() {
        assert_eq!(
            select_version(&versions(&["2024-11-05", "2026-07-28"])),
            ProbeVerdict::Modern {
                version: "2026-07-28".into()
            }
        );
    }

    #[test]
    fn modern_meta_is_merged_into_params() {
        let bare = with_modern_meta(None, "2026-07-28").unwrap();
        assert_eq!(bare["_meta"][META_PROTOCOL_VERSION], "2026-07-28");
        assert_eq!(bare["_meta"][META_CLIENT_CAPABILITIES], json!({}));

        let call = with_modern_meta(
            Some(json!({ "name": "shout", "arguments": { "text": "hi" }, "_meta": { "progressToken": 7 } })),
            "2026-07-28",
        )
        .unwrap();
        assert_eq!(call["name"], "shout");
        assert_eq!(
            call["_meta"]["progressToken"], 7,
            "caller meta keys survive"
        );
        assert_eq!(call["_meta"][META_PROTOCOL_VERSION], "2026-07-28");

        assert!(with_modern_meta(Some(json!([1, 2])), "2026-07-28").is_err());
        assert!(with_modern_meta(Some(json!({ "_meta": 3 })), "2026-07-28").is_err());
    }

    #[test]
    fn only_complete_results_pass() {
        assert!(ensure_complete(&json!({ "tools": [] })).is_ok());
        assert!(ensure_complete(&json!({ "resultType": "complete" })).is_ok());
        assert!(ensure_complete(&json!({ "resultType": "input_required" })).is_err());
        assert!(ensure_complete(&json!({ "resultType": "teleport" })).is_err());
    }
}
