//! Scripted tool definitions

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// A user-authored tool written in JavaScript/TypeScript
#[derive(Debug, Clone)]
pub struct ScriptedTool {
    /// Unique tool name
    pub name: String,
    /// Source code (JS or TS)
    pub source: String,
    /// Whether the source is TypeScript
    pub is_typescript: bool,
    /// Source file path (for error messages)
    pub source_path: Option<PathBuf>,
    /// Permissions granted to this tool
    pub permissions: ToolPermissions,
    /// Execution timeout in milliseconds
    pub timeout_ms: u64,
}

impl ScriptedTool {
    /// Create a new scripted tool from source code
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        let source = source.into();
        let name = name.into();
        let is_typescript =
            name.ends_with(".ts") || source.contains(": string") || source.contains(": number");

        Self {
            name,
            source,
            is_typescript,
            source_path: None,
            permissions: ToolPermissions::default(),
            timeout_ms: 30_000,
        }
    }

    /// Create from a file path
    pub fn from_file(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let source = std::fs::read_to_string(&path)?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string());
        let is_typescript = path.extension().map_or(false, |e| e == "ts" || e == "tsx");

        Ok(Self {
            name,
            source,
            is_typescript,
            source_path: Some(path),
            permissions: ToolPermissions::default(),
            timeout_ms: 30_000,
        })
    }

    /// Set permissions
    #[must_use]
    pub fn with_permissions(mut self, permissions: ToolPermissions) -> Self {
        self.permissions = permissions;
        self
    }

    /// Set timeout
    #[must_use]
    pub const fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Mark as TypeScript
    #[must_use]
    pub const fn typescript(mut self, is_ts: bool) -> Self {
        self.is_typescript = is_ts;
        self
    }
}

/// Permissions for scripted tools
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPermissions {
    /// Allowed network hosts (empty = none)
    pub net: Vec<String>,
    /// Allowed read paths (empty = none)
    pub read: Vec<PathBuf>,
    /// Allowed write paths (empty = none)
    pub write: Vec<PathBuf>,
    /// Allow environment variable access
    pub env: bool,
    /// Allow subprocess execution
    pub run: bool,
}

impl ToolPermissions {
    /// No permissions (fully sandboxed)
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Allow network access to specific hosts
    #[must_use]
    pub fn with_net(mut self, hosts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.net = hosts.into_iter().map(Into::into).collect();
        self
    }

    /// Allow reading from specific paths
    #[must_use]
    pub fn with_read(mut self, paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        self.read = paths.into_iter().map(Into::into).collect();
        self
    }

    /// Allow writing to specific paths
    #[must_use]
    pub fn with_write(mut self, paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        self.write = paths.into_iter().map(Into::into).collect();
        self
    }

    /// Allow environment variable access
    #[must_use]
    pub const fn with_env(mut self) -> Self {
        self.env = true;
        self
    }

    /// Check if network access to a host is allowed
    pub fn allows_net(&self, host: &str) -> bool {
        self.net
            .iter()
            .any(|h| h == "*" || h == host || host.ends_with(&format!(".{h}")))
    }

    /// Check if reading a path is allowed.
    /// Supports `*` wildcard for unrestricted access and `~` for home directory.
    pub fn allows_read(&self, path: &std::path::Path) -> bool {
        self.read
            .iter()
            .any(|p| p.to_string_lossy() == "*" || path.starts_with(p))
    }

    /// Check if writing a path is allowed.
    /// Supports `*` wildcard for unrestricted access and `~` for home directory.
    pub fn allows_write(&self, path: &std::path::Path) -> bool {
        self.write
            .iter()
            .any(|p| p.to_string_lossy() == "*" || path.starts_with(p))
    }
}

/// Where a tool's output should be routed after execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OutputTarget {
    /// Large results are chunked and stored in memory; a stub replaces them in context (default).
    #[default]
    Memory,
    /// Results always stay in context. Large results are summarized (or truncated as fallback).
    Context,
}

/// Tool manifest extracted from source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolManifest {
    /// Tool name
    pub name: String,
    /// Semantic version string (e.g. "0.1.0")
    pub version: Option<String>,
    /// Tool description
    pub description: Option<String>,
    /// Input parameter schema (JSON Schema)
    pub parameters: Option<Value>,
    /// Where tool output should be routed
    #[serde(default)]
    pub output: OutputTarget,
    /// Timeout in seconds (None = use default 30s)
    pub timeout_secs: Option<u64>,
}

/// Extract manifest from tool source (looks for default export)
pub fn extract_manifest(source: &str) -> Option<ToolManifest> {
    // Simple regex-free extraction for common patterns
    // export default { name: "...", description: "...", ... }

    let name = extract_string_field(source, "name")?;
    let version = extract_string_field(source, "version");
    let description = extract_string_field(source, "description");
    let output = match extract_string_field(source, "output").as_deref() {
        Some("context") => OutputTarget::Context,
        _ => OutputTarget::Memory,
    };

    let timeout_secs = extract_number_field(source, "timeout");

    Some(ToolManifest {
        name,
        version,
        description,
        parameters: extract_parameters_schema(source),
        output,
        timeout_secs,
    })
}

/// Extract the JSON-Schema `parameters` object from a tool manifest source.
///
/// Manifests declare `parameters` as a JS object literal (unquoted keys, maybe
/// single-quoted strings, trailing commas, or comments), which is not valid
/// JSON. We locate the balanced `{..}` block following `parameters:` and
/// normalize it to strict JSON so the LLM-facing tool definition carries the
/// real input schema instead of an empty parameter list.
///
/// Returns `None` (falling back to no parameters — the prior behavior) if the
/// block is absent or can't be normalized into valid JSON; it never guesses.
pub fn extract_parameters_schema(source: &str) -> Option<Value> {
    let block = extract_object_after_key(source, "parameters")?;
    debug_assert!(block.starts_with('{') && block.ends_with('}'));
    let json = js_object_literal_to_json(block);
    serde_json::from_str::<Value>(&json).ok()
}

#[inline]
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

#[inline]
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Find `key:` at identifier boundary followed by a `{`, and return the
/// balanced object block (including braces), respecting strings and comments.
fn extract_object_after_key<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    let bytes = source.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = source[from..].find(key) {
        let idx = from + rel;
        from = idx + key.len();
        // Must be a standalone identifier, not a substring of a longer one.
        let prev_ok = idx == 0 || !is_ident_char(bytes[idx - 1]);
        let next_ok = !bytes
            .get(idx + key.len())
            .is_some_and(|&b| is_ident_char(b));
        if !prev_ok || !next_ok {
            continue;
        }
        // Expect: optional ws, ':', optional ws, '{'.
        let mut j = idx + key.len();
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if bytes.get(j) != Some(&b':') {
            continue;
        }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if bytes.get(j) != Some(&b'{') {
            continue;
        }
        return extract_balanced_object(source, j);
    }
    None
}

/// Return the substring from the `{` at `open` to its matching `}` (inclusive),
/// skipping braces that appear inside string literals or comments.
fn extract_balanced_object(source: &str, open: usize) -> Option<&str> {
    let bytes = source.as_bytes();
    debug_assert_eq!(bytes.get(open), Some(&b'{'));
    let mut depth: usize = 0;
    let mut i = open;
    let mut in_str: Option<u8> = None;
    let mut escaped = false;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = in_str {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == q {
                in_str = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' | b'\'' => in_str = Some(c),
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[open..=i]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Normalize a JS object literal into strict JSON: convert single-quoted
/// strings to double-quoted, quote bare identifier keys, drop comments, and
/// drop trailing commas. Bare `true`/`false`/`null` values are left intact.
fn js_object_literal_to_json(block: &str) -> String {
    let bytes = block.as_bytes();
    let mut out = String::with_capacity(block.len() + 16);
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            // Line comment.
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            // Block comment.
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 2;
            }
            // String literal (either quote style) -> emit as JSON double-quoted.
            b'"' | b'\'' => {
                let (s, next) = read_js_string(block, i);
                push_json_string(&mut out, &s);
                i = next;
            }
            // Trailing comma: drop if the next significant char closes a container.
            b',' => {
                if next_significant_is_close(block, i + 1) {
                    i += 1; // skip the comma
                } else {
                    out.push(',');
                    i += 1;
                }
            }
            // Bare identifier: a key (followed by ':') gets quoted; a literal
            // value (true/false/null) is copied verbatim.
            _ if is_ident_start(c) => {
                let start = i;
                i += 1;
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                let ident = &block[start..i];
                if next_significant_is_colon(block, i) {
                    push_json_string(&mut out, ident);
                } else {
                    out.push_str(ident);
                }
            }
            _ => {
                // Copy a full UTF-8 char (structural tokens are all ASCII and
                // handled above; this arm may see multibyte text).
                let ch = block[i..].chars().next().unwrap_or(c as char);
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// Read a JS string starting at `open` (a quote char); returns the fully
/// **decoded** contents (escapes resolved to real characters, surrounding
/// quotes stripped) and the index just past the close. The caller re-encodes
/// via [`push_json_string`], so decoding here keeps escaping single-sourced.
fn read_js_string(block: &str, open: usize) -> (String, usize) {
    let bytes = block.as_bytes();
    let quote = bytes[open];
    let mut s = String::new();
    let mut i = open + 1;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' {
            match bytes.get(i + 1) {
                Some(b'"') => s.push('"'),
                Some(b'\'') => s.push('\''),
                Some(b'\\') => s.push('\\'),
                Some(b'/') => s.push('/'),
                Some(b'n') => s.push('\n'),
                Some(b't') => s.push('\t'),
                Some(b'r') => s.push('\r'),
                Some(b'b') => s.push('\u{08}'),
                Some(b'f') => s.push('\u{0C}'),
                Some(b'u') => {
                    // \uXXXX -> the code point (malformed sequences pass through).
                    if let Some(cp) = block
                        .get(i + 2..i + 6)
                        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                        .and_then(char::from_u32)
                    {
                        s.push(cp);
                        i += 6;
                        continue;
                    }
                    s.push('\\');
                    i += 1;
                    continue;
                }
                Some(_) => {
                    // Unknown escape: keep the escaped char literally.
                    i += 1;
                    if let Some(ch) = block[i..].chars().next() {
                        s.push(ch);
                        i += ch.len_utf8();
                    }
                    continue;
                }
                None => s.push('\\'),
            }
            i += 2;
            continue;
        }
        if c == quote {
            i += 1;
            break;
        }
        // Copy a full UTF-8 char so multibyte text survives.
        let ch = block[i..].chars().next().unwrap_or(c as char);
        s.push(ch);
        i += ch.len_utf8();
    }
    (s, i)
}

/// Append `raw` to `out` as a valid JSON string literal (quotes + escaping).
fn push_json_string(out: &mut String, raw: &str) {
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// True if the next non-whitespace, non-comment byte from `i` is `:`.
fn next_significant_is_colon(block: &str, i: usize) -> bool {
    matches!(peek_significant(block, i), Some(b':'))
}

/// True if the next non-whitespace, non-comment byte from `i` closes a
/// container (`}` or `]`) — i.e. the preceding comma was trailing.
fn next_significant_is_close(block: &str, i: usize) -> bool {
    matches!(peek_significant(block, i), Some(b'}') | Some(b']'))
}

/// Peek the next significant byte (skipping whitespace and comments).
fn peek_significant(block: &str, mut i: usize) -> Option<u8> {
    let bytes = block.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b if b.is_ascii_whitespace() => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 2;
            }
            b => return Some(b),
        }
    }
    None
}

fn extract_number_field(source: &str, field: &str) -> Option<u64> {
    // Match: timeout: 300 or timeout:300
    let patterns = [format!("{field}: "), format!("{field}:")];
    for pattern in &patterns {
        if let Some(start) = source.find(pattern) {
            let value_start = start + pattern.len();
            let rest = &source[value_start..];
            let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = num_str.parse::<u64>() {
                return Some(n);
            }
        }
    }
    None
}

fn extract_string_field(source: &str, field: &str) -> Option<String> {
    // Match: name: "value" or name: 'value'
    let patterns = [
        format!(r#"{field}: ""#),
        format!(r#"{field}: '"#),
        format!(r#"{field}:""#),
        format!(r#"{field}:'"#),
    ];

    for pattern in &patterns {
        if let Some(start) = source.find(pattern) {
            let quote = if pattern.ends_with('"') { '"' } else { '\'' };
            let value_start = start + pattern.len();
            if let Some(end) = source[value_start..].find(quote) {
                return Some(source[value_start..value_start + end].to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_manifest() {
        let source = r#"
            export default {
                name: "greet",
                description: "Greet someone by name",
                execute({ name }) {
                    return `Hello, ${name}!`;
                }
            }
        "#;

        let manifest = extract_manifest(source).unwrap();
        assert_eq!(manifest.name, "greet");
        assert_eq!(manifest.version, None);
        assert_eq!(
            manifest.description,
            Some("Greet someone by name".to_string())
        );
    }

    #[test]
    fn test_extract_manifest_with_version() {
        let source = r#"
            export default {
                name: "exec",
                version: "0.1.0",
                description: "Run a command",
                execute({ command }) {
                    return "done";
                }
            }
        "#;

        let manifest = extract_manifest(source).unwrap();
        assert_eq!(manifest.name, "exec");
        assert_eq!(manifest.version, Some("0.1.0".to_string()));
    }

    #[test]
    fn test_extract_manifest_with_output_context() {
        let source = r#"
            export default {
                name: "recall",
                description: "Search memory",
                output: "context",
                execute({ query }) {
                    return "results";
                }
            }
        "#;

        let manifest = extract_manifest(source).unwrap();
        assert_eq!(manifest.name, "recall");
        assert_eq!(manifest.output, OutputTarget::Context);
    }

    #[test]
    fn test_extract_manifest_output_defaults_to_memory() {
        let source = r#"
            export default {
                name: "exec",
                description: "Run a command",
                execute({ command }) {
                    return "done";
                }
            }
        "#;

        let manifest = extract_manifest(source).unwrap();
        assert_eq!(manifest.output, OutputTarget::Memory);
    }

    #[test]
    fn test_permissions() {
        let perms = ToolPermissions::none()
            .with_net(["api.example.com", "*.github.com"])
            .with_read(["/tmp"]);

        assert!(perms.allows_net("api.example.com"));
        assert!(!perms.allows_net("evil.com"));
        assert!(perms.allows_read(std::path::Path::new("/tmp/file.txt")));
        assert!(!perms.allows_read(std::path::Path::new("/etc/passwd")));
    }

    #[test]
    fn params_schema_matches_real_manifest_shape() {
        // Mirrors default-skills/read_file/tool.ts exactly.
        let source = r#"
            export default {
                name: "read_file",
                output: "context",
                description: "Read a file.",
                parameters: {
                    type: "object",
                    properties: {
                        file_path: { type: "string", description: "Path to the file." },
                        offset: { type: "integer", description: "Start line." },
                        limit: { type: "integer", description: "Max lines." }
                    },
                    required: ["file_path"]
                },
                execute: function(input) { return "x"; }
            }
        "#;
        let manifest = extract_manifest(source).unwrap();
        let params = manifest.parameters.expect("parameters must be parsed");
        assert_eq!(params["type"], "object");
        assert_eq!(params["properties"]["file_path"]["type"], "string");
        assert_eq!(
            params["properties"]["file_path"]["description"],
            "Path to the file."
        );
        assert_eq!(params["properties"]["offset"]["type"], "integer");
        assert_eq!(params["required"][0], "file_path");
    }

    #[test]
    fn params_schema_tolerates_trailing_commas_and_comments() {
        let source = r#"
            export default {
                name: "t",
                description: "d",
                parameters: {
                    // the input document
                    type: "object",
                    properties: {
                        q: { type: "string", }, /* the query */
                    },
                    required: ["q"],
                },
                execute(i) {}
            }
        "#;
        let params = extract_parameters_schema(source).expect("valid despite commas/comments");
        assert_eq!(params["properties"]["q"]["type"], "string");
        assert_eq!(params["required"][0], "q");
    }

    #[test]
    fn params_schema_handles_single_quotes_and_enum() {
        let source = r#"
            export default {
                name: 't',
                description: 'd',
                parameters: {
                    type: 'object',
                    properties: {
                        mode: { type: 'string', enum: ['fast', 'slow'], default: 'fast' }
                    }
                },
                execute(i) {}
            }
        "#;
        let params = extract_parameters_schema(source).expect("single quotes normalize");
        assert_eq!(params["properties"]["mode"]["type"], "string");
        assert_eq!(params["properties"]["mode"]["enum"][1], "slow");
        assert_eq!(params["properties"]["mode"]["default"], "fast");
    }

    #[test]
    fn params_schema_handles_escaped_quotes_in_descriptions() {
        // Mirrors default-skills/python/tool.ts: a double-quoted description
        // whose text embeds escaped double-quotes (a JSON example).
        let source = r#"
            export default {
                name: "python",
                description: "run python",
                parameters: {
                    type: "object",
                    properties: {
                        args: {
                            type: "string",
                            description: "JSON like: '{\"pattern\": \"*.rs\", \"dry_run\": true}'"
                        }
                    },
                    required: []
                },
                execute(i) {}
            }
        "#;
        let params = extract_parameters_schema(source).expect("escaped quotes normalize");
        assert_eq!(
            params["properties"]["args"]["description"],
            "JSON like: '{\"pattern\": \"*.rs\", \"dry_run\": true}'"
        );
        assert!(params["required"].as_array().unwrap().is_empty());
    }

    #[test]
    fn params_schema_absent_yields_none() {
        // A tool with no `parameters` declaration keeps the prior behavior.
        let source = r#"
            export default {
                name: "noparams",
                description: "no schema",
                execute(i) { return "ok"; }
            }
        "#;
        let manifest = extract_manifest(source).unwrap();
        assert!(manifest.parameters.is_none());
    }

    #[test]
    fn params_schema_preserves_non_ascii_descriptions() {
        // Byte-wise normalization must not corrupt multibyte UTF-8 text.
        let source = r#"
            export default {
                name: "t",
                description: "d",
                parameters: {
                    type: "object",
                    properties: {
                        note: { type: "string", description: "café — déjà vu ✓" }
                    }
                },
                execute(i) {}
            }
        "#;
        let params = extract_parameters_schema(source).expect("utf-8 survives");
        assert_eq!(
            params["properties"]["note"]["description"],
            "café — déjà vu ✓"
        );
    }

    #[test]
    fn params_schema_braces_inside_strings_dont_break_balance() {
        // A `}` inside a description string must not close the object early.
        let source = r#"
            export default {
                name: "t",
                description: "d",
                parameters: {
                    type: "object",
                    properties: {
                        tpl: { type: "string", description: "use {curly} braces }" }
                    }
                },
                execute(i) {}
            }
        "#;
        let params = extract_parameters_schema(source).expect("string braces ignored");
        assert_eq!(
            params["properties"]["tpl"]["description"],
            "use {curly} braces }"
        );
    }
}
