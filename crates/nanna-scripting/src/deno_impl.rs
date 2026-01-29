//! Deno (V8) JavaScript engine implementation
//!
//! Full-featured JavaScript/TypeScript engine with JIT compilation.
//! Used as fallback when Boa cannot execute a script.

use crate::{NannaBridge, Result, ScriptError, ScriptedTool};
use deno_core::{extension, JsRuntime, ModuleSpecifier, RuntimeOptions, v8, scope};
use serde_json::Value;
use std::sync::Arc;

/// Execute a tool with Deno
pub async fn execute(
    tool: &ScriptedTool,
    input: &Value,
    _bridge: &Arc<NannaBridge>,
) -> Result<Value> {
    let source = tool.source.clone();
    let input_clone = input.clone();
    let timeout_ms = tool.timeout_ms;
    let is_typescript = tool.is_typescript;

    // Run in a blocking task because JsRuntime isn't Send
    let result = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ScriptError::Execution(format!("Failed to create runtime: {e}")))?;

        rt.block_on(execute_inner(&source, &input_clone, is_typescript))
    });

    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), result).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => Err(ScriptError::Execution(format!("Task panicked: {e}"))),
        Err(_) => Err(ScriptError::Timeout(timeout_ms)),
    }
}

async fn execute_inner(source: &str, input: &Value, is_typescript: bool) -> Result<Value> {
    // Transpile TypeScript if needed
    let js_source = if is_typescript {
        transpile_typescript(source)?
    } else {
        source.to_string()
    };

    // Create runtime
    let mut runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![nanna_extension::init()],
        ..Default::default()
    });

    let input_json = serde_json::to_string(input)?;

    // Execute the script - returns JSON string
    let script = format!(
        r#"
        (function() {{
            globalThis.INPUT = {input_json};
            globalThis.console = {{
                log: (...args) => {{}},
                warn: (...args) => {{}},
                error: (...args) => {{}},
                debug: (...args) => {{}},
            }};
            
            {js_source}
            
            if (typeof execute === 'function') {{
                const result = execute(INPUT);
                // Handle both sync and async results
                if (result && typeof result.then === 'function') {{
                    return result.then(r => JSON.stringify(r));
                }}
                return JSON.stringify(result);
            }}
            
            throw new Error('No execute function found');
        }})()
        "#
    );

    let result = runtime
        .execute_script("<tool>", script)
        .map_err(|e| ScriptError::Execution(format!("Script error: {e}")))?;

    // Run event loop for any pending async work
    runtime
        .run_event_loop(Default::default())
        .await
        .map_err(|e| ScriptError::Execution(format!("Event loop error: {e}")))?;

    // Check if result is a promise and resolve it
    let resolved = runtime
        .resolve(result)
        .await
        .map_err(|e| ScriptError::Execution(format!("Promise resolution error: {e}")))?;

    // Extract the JSON string using scope! macro
    let json_str: String = {
        scope!(scope, runtime);
        let local = v8::Local::new(scope, &resolved);
        deno_core::serde_v8::from_v8(scope, local)
            .map_err(|e| ScriptError::Execution(format!("Failed to deserialize: {e}")))?
    };

    // Parse JSON to Value
    let result_value: Value = serde_json::from_str(&json_str)
        .map_err(|e| ScriptError::Execution(format!("Failed to parse JSON: {e}")))?;

    Ok(result_value)
}

/// Transpile TypeScript to JavaScript using deno_ast
pub fn transpile_typescript(source: &str) -> Result<String> {
    use deno_ast::{EmitOptions, MediaType, ParseParams, TranspileModuleOptions, TranspileOptions};

    let parsed = deno_ast::parse_module(ParseParams {
        specifier: ModuleSpecifier::parse("file:///tool.ts")
            .map_err(|e| ScriptError::Transpile(format!("Invalid specifier: {e}")))?,
        text: source.into(),
        media_type: MediaType::TypeScript,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })
    .map_err(|e| ScriptError::Transpile(format!("Parse error: {e}")))?;

    let transpiled = parsed
        .transpile(
            &TranspileOptions::default(),
            &TranspileModuleOptions::default(),
            &EmitOptions::default(),
        )
        .map_err(|e| ScriptError::Transpile(format!("Transpile error: {e}")))?;

    Ok(transpiled.into_source().text.to_string())
}

// Minimal extension
extension!(nanna_extension,);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transpile() {
        let ts_source = r#"
            function greet(name: string): string {
                return `Hello, ${name}!`;
            }
            
            function execute(input: { name: string }) {
                return greet(input.name);
            }
        "#;

        let js = transpile_typescript(ts_source).unwrap();
        assert!(!js.contains(": string"));
        assert!(js.contains("function greet"));
    }
}
