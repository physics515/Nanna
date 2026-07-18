//! Response healing for malformed LLM JSON.
//!
//! Free-form model output often wraps JSON in markdown fences, drops commas,
//! or concatenates objects. Call sites (chat tool args, embeddings HTTP bodies,
//! summarization / extraction) share this heuristic salvage path.

use serde_json::Value;

/// Strip common wrappers (markdown fences, leading prose) and salvage a JSON value.
///
/// Order of attempts:
/// 1. Direct parse of the trimmed input
/// 2. Strip ``` / ```json fences, retry
/// 3. Extract the first balanced `{...}` or `[...]` span, retry
/// 4. Apply cheap repairs (trailing commas, single quotes, bare keys), retry
#[must_use]
pub fn heal_json(input: &str) -> Option<Value> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Some(v);
    }

    let unfenced = strip_code_fence(trimmed);
    if unfenced != trimmed {
        if let Ok(v) = serde_json::from_str::<Value>(unfenced) {
            return Some(v);
        }
    }

    if let Some(span) = extract_json_span(unfenced) {
        if let Ok(v) = serde_json::from_str::<Value>(span) {
            return Some(v);
        }
        let repaired = repair_common(span);
        if let Ok(v) = serde_json::from_str::<Value>(&repaired) {
            return Some(v);
        }
    }

    let repaired = repair_common(unfenced);
    serde_json::from_str::<Value>(&repaired).ok()
}

/// Like [`heal_json`], but typed.
pub fn heal_json_as<T: serde::de::DeserializeOwned>(input: &str) -> Option<T> {
    heal_json(input).and_then(|v| serde_json::from_value(v).ok())
}

/// Heal a free-form model response expected to be a JSON object of tool args.
#[must_use]
pub fn heal_tool_args(input: &str) -> Value {
    match heal_json(input) {
        Some(Value::Object(map)) => Value::Object(map),
        Some(other) => serde_json::json!({ "value": other }),
        None => Value::Object(serde_json::Map::new()),
    }
}

fn strip_code_fence(s: &str) -> &str {
    let t = s.trim();
    if !t.starts_with("```") {
        return t;
    }
    let rest = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```JSON"))
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    rest.strip_suffix("```").unwrap_or(rest).trim()
}

/// First top-level balanced JSON object/array span.
fn extract_json_span(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{' || b == b'[')?;
    let open = bytes[start];
    let close = if open == b'{' { b'}' } else { b']' };
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b if b == open => depth += 1,
            b if b == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=i]);
                }
            }
            _ => {}
        }
    }
    // Unterminated: take from start and try repair later
    if depth > 0 {
        return Some(s[start..].trim_end());
    }
    None
}

fn repair_common(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut in_string = false;
    let mut escape = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '\'' => {
                // Treat single-quoted strings as double-quoted
                out.push('"');
                i += 1;
                while i < chars.len() {
                    let sc = chars[i];
                    if sc == '\'' && (i == 0 || chars[i - 1] != '\\') {
                        out.push('"');
                        break;
                    }
                    if sc == '"' {
                        out.push_str("\\\"");
                    } else {
                        out.push(sc);
                    }
                    i += 1;
                }
            }
            ',' => {
                // Drop trailing commas before } or ]
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                    // skip the comma
                } else {
                    out.push(c);
                }
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                // Bare key: word followed by colon → "word":
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '-')
                {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let mut j = i;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == ':' {
                    out.push('"');
                    out.push_str(&word);
                    out.push('"');
                    continue; // i already advanced
                }
                out.push_str(&word);
                continue;
            }
            _ => out.push(c),
        }
        i += 1;
    }

    // Balance braces/brackets if truncated
    let mut depth_obj = 0i32;
    let mut depth_arr = 0i32;
    in_string = false;
    escape = false;
    for c in out.chars() {
        if in_string {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth_obj += 1,
            '}' => depth_obj -= 1,
            '[' => depth_arr += 1,
            ']' => depth_arr -= 1,
            _ => {}
        }
    }
    if in_string {
        out.push('"');
    }
    for _ in 0..depth_arr.max(0) {
        out.push(']');
    }
    for _ in 0..depth_obj.max(0) {
        out.push('}');
    }
    out
}

/// Count balanced top-level JSON objects/arrays in `s`.
///
/// A count `> 1` is the signal that a single streamed `tool_use` argument buffer
/// holds *multiple* concatenated objects — i.e. several distinct tool calls
/// collapsed into one block (see the agent stream accumulator). String-aware, so
/// braces inside string literals do not inflate the count. Single pass, O(n).
#[must_use]
pub fn count_balanced_top_level_objects(s: &str) -> usize {
    let (mut count, mut depth, mut in_string, mut escape) = (0usize, 0i32, false, false);
    for &b in s.as_bytes() {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    count += 1;
                }
            }
            _ => {}
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_json() {
        let v = heal_json(r#"{"a":1}"#).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn strips_markdown_fence() {
        let raw = "```json\n[{\"content\":\"x\",\"category\":\"fact\"}]\n```";
        let v = heal_json(raw).unwrap();
        assert!(v.is_array());
        assert_eq!(v[0]["content"], "x");
    }

    #[test]
    fn extracts_object_from_prose() {
        let raw = "Sure! Here you go:\n{\"name\":\"Bob\",\"ok\":true}\nHope that helps.";
        let v = heal_json(raw).unwrap();
        assert_eq!(v["name"], "Bob");
    }

    #[test]
    fn repairs_trailing_comma() {
        let v = heal_json(r#"{"a":1,"b":2,}"#).unwrap();
        assert_eq!(v["b"], 2);
    }

    #[test]
    fn repairs_truncated_object() {
        let v = heal_json(r#"{"a": "hello", "b": 3"#).unwrap();
        assert_eq!(v["a"], "hello");
        assert_eq!(v["b"], 3);
    }

    #[test]
    fn tool_args_fallback_empty_object() {
        let v = heal_tool_args("not json at all");
        assert!(v.as_object().unwrap().is_empty());
    }

    #[test]
    fn typed_array_heal() {
        #[derive(serde::Deserialize)]
        struct Item {
            content: String,
        }
        let items: Vec<Item> = heal_json_as(r#"[{"content":"hi"}]"#).unwrap();
        assert_eq!(items[0].content, "hi");
    }

    #[test]
    fn last_object_concatenated_still_finds_first() {
        // Balanced extraction returns the first complete object
        let raw = r#"{"a":1} garbage {"b":2}"#;
        let v = heal_json(raw).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn single_object_counts_one() {
        assert_eq!(count_balanced_top_level_objects(r#"{"a":1}"#), 1);
        assert_eq!(count_balanced_top_level_objects(r#"  {"a":{"b":2}}  "#), 1);
    }

    #[test]
    fn two_concatenated_objects_counts_two() {
        // The streaming-collapse shape: two tool calls' args in one buffer.
        let raw = r#"{"a":1}{"b":2}"#;
        assert_eq!(count_balanced_top_level_objects(raw), 2);
        // heal_json still salvages the FIRST object — count is the collapse
        // signal, heal is the (lossy) fallback.
        assert_eq!(heal_json(raw).unwrap()["a"], 1);
    }

    #[test]
    fn braces_inside_strings_ignored() {
        assert_eq!(count_balanced_top_level_objects(r#"{"a":"}{"}"#), 1);
        assert_eq!(count_balanced_top_level_objects(r#"{"a":"\""}"#), 1);
    }
}
