//! Tool result formatting helpers.
//!
//! Tools return free-form text by default. When an optional `output_schema` is
//! present on the [`ToolDefinition`](crate::ToolDefinition) **and** the call
//! sets `output_mode=json` (or `json=true`), the structured `data` field is
//! preferred as compact machine-readable output. This keeps context lean for
//! verbose tools whose textual dumps would otherwise dominate token budgets.

use crate::schema::{ToolDefinition, ToolResult};
use serde_json::Value;

/// Preferred text content for a completed tool call, honouring JSON output mode.
///
/// Selection rules:
/// 1. Failures return the error string (JSON mode has nothing useful to emit).
/// 2. When JSON mode is requested and structured `data` is present, serialize
///    that data — schemas, when declared, are treated as documentation, not
///    equality confirmation.
/// 3. Otherwise keep the free-form `content` text the tool already produced.
#[must_use]
pub fn format_tool_output(
    result: &ToolResult,
    definition: Option<&ToolDefinition>,
    params: &std::collections::HashMap<String, Value>,
) -> String {
    if !result.success {
        return result
            .error
            .clone()
            .unwrap_or_else(|| "Tool execution failed".to_string());
    }

    if wants_json_output(params) {
        if let Some(data) = result.data.as_ref() {
            let _ = definition; // reserved: future schema-guided shaping
            return compact_json(data);
        }
        // JSON mode requested but no structured payload — keep free text.
        let _ = definition;
    }

    result.content.clone()
}

/// Whether the call asked for compact JSON output.
///
/// Accepted shapes: `output_mode="json"` (preferred), `json=true`, or
/// `format="json"`. Missing / other values leave text-mode content alone.
#[must_use]
pub fn wants_json_output(params: &std::collections::HashMap<String, Value>) -> bool {
    if let Some(mode) = params
        .get("output_mode")
        .or_else(|| params.get("format"))
        .and_then(Value::as_str)
    {
        return mode.eq_ignore_ascii_case("json");
    }
    params
        .get("json")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Serialize to compact (no whitespace) JSON.
fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

/// Common `output_schema` snippets for frequently used tools.
/// Kept next to the formatter so schemas and the JSON-mode path evolve
/// together; tools pick them up through [`ToolDefinition::with_output_schema`].
pub mod schemas {
    use serde_json::{json, Value};

    #[must_use]
    pub fn read_file() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "total_lines": { "type": "integer" },
                "offset": { "type": "integer" },
                "lines_returned": { "type": "integer" },
                "content": {
                    "type": "string",
                    "description": "File body (only present when json-mode also embeds content)"
                }
            },
            "required": ["path", "total_lines", "lines_returned"]
        })
    }

    #[must_use]
    pub fn list_dir() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "entries": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "kind": {
                                "type": "string",
                                "enum": ["file", "dir", "other"]
                            },
                            "size": { "type": "integer" }
                        },
                        "required": ["name", "kind"]
                    }
                }
            },
            "required": ["path", "entries"]
        })
    }

    #[must_use]
    pub fn write_file() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "bytes_written": { "type": "integer" }
            },
            "required": ["path", "bytes_written"]
        })
    }

    #[must_use]
    pub fn exec() -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "exit_code": { "type": "integer" },
                "stdout": { "type": "string" },
                "stderr": { "type": "string" },
                "workdir": { "type": "string" }
            },
            "required": ["command", "exit_code"]
        })
    }

    #[must_use]
    pub fn code_search() -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string" },
                "total_matches": { "type": "integer" },
                "files_searched": { "type": "integer" },
                "matches": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file": { "type": "string" },
                            "line": { "type": "integer" },
                            "text": { "type": "string" }
                        },
                        "required": ["file", "line", "text"]
                    }
                }
            },
            "required": ["pattern", "total_matches", "matches"]
        })
    }

    #[must_use]
    pub fn project_structure() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "total_files": { "type": "integer" },
                "total_size": { "type": "integer" },
                "total_lines": { "type": "integer" },
                "tree": { "type": "string" }
            },
            "required": ["path", "total_files"]
        })
    }

    #[must_use]
    pub fn web_search() -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "results": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string" },
                            "url": { "type": "string" },
                            "description": { "type": "string" }
                        },
                        "required": ["title", "url"]
                    }
                }
            },
            "required": ["query", "results"]
        })
    }

    #[must_use]
    pub fn web_fetch() -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "title": { "type": "string" },
                "chars": { "type": "integer" },
                "content": { "type": "string" }
            },
            "required": ["url", "chars"]
        })
    }

    #[must_use]
    pub fn memory_recall() -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "results": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "content": { "type": "string" },
                            "score": { "type": "number" }
                        },
                        "required": ["content"]
                    }
                }
            },
            "required": ["results"]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{ToolDefinition, ToolResult};
    use serde_json::json;
    use std::collections::HashMap;

    fn params(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn wants_json_detects_output_mode_json_and_format_and_bool() {
        assert!(wants_json_output(&params(&[("output_mode", json!("json"))])));
        assert!(wants_json_output(&params(&[("output_mode", json!("JSON"))])));
        assert!(wants_json_output(&params(&[("format", json!("json"))])));
        assert!(wants_json_output(&params(&[("json", json!(true))])));
        assert!(!wants_json_output(&params(&[("output_mode", json!("text"))])));
        assert!(!wants_json_output(&params(&[("json", json!(false))])));
        assert!(!wants_json_output(&HashMap::new()));
    }

    #[test]
    fn format_prefers_structured_data_when_json_mode_set() {
        let result = ToolResult::success("human text")
            .with_data(json!({"path": "a.rs", "total_lines": 10}));
        let def = ToolDefinition::new("read_file", "x").with_output_schema(schemas::read_file());
        let out = format_tool_output(
            &result,
            Some(&def),
            &params(&[("output_mode", json!("json"))]),
        );
        assert!(out.contains("\"path\":\"a.rs\""));
        assert!(!out.contains("human text"));
        // default mode still returns free text
        let text = format_tool_output(&result, Some(&def), &HashMap::new());
        assert_eq!(text, "human text");
    }

    #[test]
    fn format_falls_back_to_content_without_data() {
        let result = ToolResult::success("only text");
        let out = format_tool_output(&result, None, &params(&[("json", json!(true))]));
        assert_eq!(out, "only text");
    }

    #[test]
    fn format_errors_ignore_json_mode() {
        let result = ToolResult::error("boom");
        let out = format_tool_output(
            &result,
            None,
            &params(&[("output_mode", json!("json"))]),
        );
        assert_eq!(out, "boom");
    }
}
