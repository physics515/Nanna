//! Safety gate for untrusted tool schemas received from MCP servers.
//!
//! A tool's `input_schema` is arbitrary JSON authored by a **remote, untrusted**
//! server. Two failure classes matter for a client that ingests these into its own
//! tool registry (and later hands them to an LLM):
//!
//! 1. **External `$ref` dereferencing (SSRF).** JSON Schema `$ref` can point at an
//!    external URI (`https://…`, `file://…`, `./other.json`). A client that resolves
//!    such a ref would make an attacker-chosen network/filesystem fetch. We never
//!    dereference, but we go further and **reject** any schema carrying an external
//!    `$ref` at ingest — an unresolvable ref is either malicious or unusable, so the
//!    tool should not enter the registry. Only pure-fragment refs (`#/…`, or bare
//!    `#`) are allowed through (they reference within the same document and need no
//!    fetch). This is the client-side half of the 2026-07-28 MCP spec hardening.
//!
//! 2. **Pathological size/nesting (DoS).** `serde_json` already caps *parse* recursion
//!    at 128, but a schema that is merely deep-but-legal, or enormously wide, still
//!    costs unbounded time in every later traversal (`schema_to_parameters`, token
//!    estimation, serialization to the LLM). We bound both **depth** and **total node
//!    count** up front so one hostile tool cannot stall the whole toolset.
//!
//! The traversal is iterative with an explicit, bounded work stack — never recursive —
//! so validating an adversarial schema cannot itself overflow the native stack.

use serde_json::Value;

/// Maximum nesting depth allowed in a tool `input_schema`.
///
/// A function-call tool schema describes call arguments; even a richly-structured one
/// (object → array-of → object → …) realistically nests a handful of levels. `32` is a
/// deliberately generous ceiling — roughly 5× any legitimate tool schema — while still
/// sitting far below `serde_json`'s 128 parse-recursion limit, so it bounds traversal
/// work without rejecting anything a real server would send. The root object is depth 1.
pub const MCP_SCHEMA_DEPTH_MAX: usize = 32;

/// Maximum total JSON node count (every scalar, array, object, and member) allowed in a
/// tool `input_schema`.
///
/// Bounds the *breadth* that a depth cap alone cannot: a flat schema with a million
/// properties is shallow yet ruinous to convert and to price in tokens. A genuine
/// function signature has at most tens of properties; `10_000` nodes is orders of
/// magnitude above that, so it only ever trips on an abusive schema. It also caps the
/// work the validator itself performs (the stack visits at most this many nodes).
pub const MCP_SCHEMA_NODES_MAX: usize = 10_000;

/// Why a tool schema was rejected. Carries the offending value so callers can log it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaViolation {
    /// Nesting exceeded [`MCP_SCHEMA_DEPTH_MAX`].
    TooDeep { depth_max: usize },
    /// Node count exceeded [`MCP_SCHEMA_NODES_MAX`].
    TooManyNodes { nodes_max: usize },
    /// A `$ref` pointed outside the document (needs a fetch we refuse to perform).
    ExternalRef { reference: String },
}

impl std::fmt::Display for SchemaViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooDeep { depth_max } => {
                write!(f, "schema nesting exceeds the maximum depth of {depth_max}")
            }
            Self::TooManyNodes { nodes_max } => {
                write!(f, "schema exceeds the maximum of {nodes_max} nodes")
            }
            Self::ExternalRef { reference } => {
                write!(
                    f,
                    "schema contains a non-fragment $ref ({reference:?}) that would require an external fetch"
                )
            }
        }
    }
}

/// True when a `$ref` value points *outside* the current document.
///
/// A `$ref` needs no fetch exactly when it is a pure fragment — it starts with `#`.
/// That covers all three intra-document forms: the whole-document root (`#`), a JSON
/// Pointer (`#/properties/x`), and a plain-name anchor (`#node`, resolving a
/// 2020-12 `$anchor`). Anything else — an absolute URI (`https://…`, `file://…`,
/// `urn:…`), a relative document path (`other.json`, `./defs`, `defs.json#/x`), or an
/// empty reference — targets a separate document and would require a fetch this client
/// refuses to make, so it is external.
fn is_external_ref(reference: &str) -> bool {
    !reference.starts_with('#')
}

/// Validate an untrusted MCP tool schema against the depth, node-count, and external-
/// `$ref` bounds. Returns the first violation found, or `Ok(())` if the schema is safe
/// to ingest.
///
/// Iterative (explicit stack, no recursion) and bounded: it visits at most
/// [`MCP_SCHEMA_NODES_MAX`] nodes before failing closed, so an adversarial schema can
/// neither overflow the stack nor run unbounded.
///
/// # Errors
///
/// Returns [`SchemaViolation`] describing the bound that was breached.
pub fn validate_tool_schema(schema: &Value) -> Result<(), SchemaViolation> {
    // (node, depth) work items. Root is depth 1 so a bare scalar schema is depth 1.
    let mut stack: Vec<(&Value, usize)> = Vec::new();
    stack.push((schema, 1));
    let mut nodes_seen: usize = 0;

    while let Some((node, depth)) = stack.pop() {
        // Positive-space bounds, checked before any further descent.
        if depth > MCP_SCHEMA_DEPTH_MAX {
            return Err(SchemaViolation::TooDeep {
                depth_max: MCP_SCHEMA_DEPTH_MAX,
            });
        }
        nodes_seen += 1;
        if nodes_seen > MCP_SCHEMA_NODES_MAX {
            return Err(SchemaViolation::TooManyNodes {
                nodes_max: MCP_SCHEMA_NODES_MAX,
            });
        }

        match node {
            Value::Object(map) => {
                // Reject an external `$ref` regardless of where it sits in the tree.
                if let Some(Value::String(reference)) = map.get("$ref")
                    && is_external_ref(reference)
                {
                    return Err(SchemaViolation::ExternalRef {
                        reference: reference.clone(),
                    });
                }
                for child in map.values() {
                    stack.push((child, depth + 1));
                }
            }
            Value::Array(items) => {
                for child in items {
                    stack.push((child, depth + 1));
                }
            }
            // Scalars are leaves — counted above, nothing to descend into.
            _ => {}
        }
    }

    debug_assert!(
        nodes_seen <= MCP_SCHEMA_NODES_MAX,
        "validator must fail closed at the node bound, saw {nodes_seen}"
    );
    debug_assert!(nodes_seen >= 1, "every schema has at least the root node");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_a_normal_tool_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "file path" },
                "count": { "type": "integer" },
                "opts": {
                    "type": "object",
                    "properties": { "recursive": { "type": "boolean" } }
                }
            },
            "required": ["path"]
        });
        assert_eq!(validate_tool_schema(&schema), Ok(()));
    }

    #[test]
    fn accepts_internal_fragment_refs() {
        let schema = json!({
            "type": "object",
            "properties": { "node": { "$ref": "#/$defs/node" } },
            "$defs": { "node": { "type": "string" } }
        });
        assert_eq!(validate_tool_schema(&schema), Ok(()));

        let bare = json!({ "$ref": "#" });
        assert_eq!(validate_tool_schema(&bare), Ok(()));
    }

    #[test]
    fn rejects_https_ref() {
        let schema = json!({
            "type": "object",
            "properties": { "x": { "$ref": "https://evil.example/schema.json" } }
        });
        assert_eq!(
            validate_tool_schema(&schema),
            Err(SchemaViolation::ExternalRef {
                reference: "https://evil.example/schema.json".to_string()
            })
        );
    }

    #[test]
    fn rejects_file_and_relative_and_empty_refs() {
        for reference in ["file:///etc/passwd", "./other.json", "defs.json#/x", ""] {
            let schema = json!({ "$ref": reference });
            assert_eq!(
                validate_tool_schema(&schema),
                Err(SchemaViolation::ExternalRef {
                    reference: reference.to_string()
                }),
                "external ref {reference:?} must be rejected"
            );
        }
    }

    #[test]
    fn rejects_a_ref_nested_deep_in_the_tree() {
        let schema = json!({
            "type": "object",
            "properties": {
                "a": { "type": "array", "items": { "$ref": "http://x/y" } }
            }
        });
        assert!(matches!(
            validate_tool_schema(&schema),
            Err(SchemaViolation::ExternalRef { .. })
        ));
    }

    #[test]
    fn rejects_over_deep_nesting() {
        // Build an object nested one level past the cap.
        let mut node = json!({ "type": "string" });
        for _ in 0..MCP_SCHEMA_DEPTH_MAX {
            node = json!({ "type": "object", "properties": { "n": node } });
        }
        assert_eq!(
            validate_tool_schema(&node),
            Err(SchemaViolation::TooDeep {
                depth_max: MCP_SCHEMA_DEPTH_MAX
            })
        );
    }

    #[test]
    fn accepts_nesting_at_the_depth_limit() {
        // A chain whose deepest scalar sits exactly at MCP_SCHEMA_DEPTH_MAX.
        // Root object = depth 1; each wrap adds two levels (object, then its member
        // value), so stay comfortably within the cap.
        let mut node = json!("leaf");
        for _ in 0..(MCP_SCHEMA_DEPTH_MAX / 2 - 1) {
            node = json!({ "k": node });
        }
        assert_eq!(validate_tool_schema(&node), Ok(()));
    }

    #[test]
    fn rejects_too_many_nodes() {
        // A wide-but-shallow object with more members than the node cap.
        let mut map = serde_json::Map::new();
        for i in 0..=MCP_SCHEMA_NODES_MAX {
            map.insert(format!("k{i}"), json!(1));
        }
        assert_eq!(
            validate_tool_schema(&Value::Object(map)),
            Err(SchemaViolation::TooManyNodes {
                nodes_max: MCP_SCHEMA_NODES_MAX
            })
        );
    }

    #[test]
    fn is_external_ref_classifies_correctly() {
        assert!(!is_external_ref("#"));
        assert!(!is_external_ref("#/properties/x"));
        assert!(!is_external_ref("#node")); // plain-name anchor — intra-document
        assert!(is_external_ref("https://x/y"));
        assert!(is_external_ref("file:///x"));
        assert!(is_external_ref("relative.json"));
        assert!(is_external_ref(""));
    }
}
