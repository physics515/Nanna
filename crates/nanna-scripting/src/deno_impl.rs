//! Deno (V8) JavaScript engine implementation
//!
//! Full-featured JavaScript/TypeScript engine with JIT compilation.
//! Used as fallback when Boa cannot execute a script.

use crate::{NannaBridge, Result, ScriptError, ScriptedTool};
use deno_core::{
    anyhow::Error as AnyError, extension, op2, JsRuntime, ModuleSpecifier, OpState, RuntimeOptions,
};
use serde_json::Value;
use std::rc::Rc;
use std::sync::Arc;
use std::cell::RefCell;

/// Execute a tool with Deno
pub async fn execute(
    tool: &ScriptedTool,
    input: &Value,
    bridge: &Arc<NannaBridge>,
) -> Result<Value> {
    let source = tool.source.clone();
    let input_clone = input.clone();
    let bridge_clone = bridge.clone();
    let timeout_ms = tool.timeout_ms;
    let is_typescript = tool.is_typescript;

    // Run in a blocking task because JsRuntime isn't Send
    let result = tokio::task::spawn_blocking(move || {
        // Create a new runtime for this execution
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ScriptError::Execution(format!("Failed to create runtime: {e}")))?;

        rt.block_on(execute_inner(&source, &input_clone, &bridge_clone, is_typescript))
    });

    // Apply timeout
    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), result).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => Err(ScriptError::Execution(format!("Task panicked: {e}"))),
        Err(_) => Err(ScriptError::Timeout(timeout_ms)),
    }
}

async fn execute_inner(
    source: &str,
    input: &Value,
    bridge: &Arc<NannaBridge>,
    is_typescript: bool,
) -> Result<Value> {
    // Transpile TypeScript if needed
    let js_source = if is_typescript {
        transpile_typescript(source)?
    } else {
        source.to_string()
    };

    // Create runtime with our extensions
    let mut runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![nanna_extension::init_ops()],
        ..Default::default()
    });

    // Inject INPUT
    let input_json = serde_json::to_string(input)?;
    runtime
        .execute_script(
            "<input>",
            format!("globalThis.INPUT = {input_json};").into(),
        )
        .map_err(|e| ScriptError::Execution(format!("Failed to inject INPUT: {e}")))?;

    // Inject wrapper and execute
    let wrapped = format!(
        r#"
        (async function() {{
            {js_source}
            
            // Find the execute function
            if (typeof execute === 'function') {{
                return await execute(INPUT);
            }}
            
            // Check for default export pattern
            if (typeof module !== 'undefined' && module.exports) {{
                const exp = module.exports.default || module.exports;
                if (exp && typeof exp.execute === 'function') {{
                    return await exp.execute(INPUT);
                }}
            }}
            
            throw new Error('No execute function found');
        }})()
        "#
    );

    let result = runtime
        .execute_script("<tool>", wrapped.into())
        .map_err(|e| ScriptError::Execution(format!("Script error: {e}")))?;

    // Run the event loop to completion (for async operations)
    runtime
        .run_event_loop(Default::default())
        .await
        .map_err(|e| ScriptError::Execution(format!("Event loop error: {e}")))?;

    // Get the result
    let scope = &mut runtime.handle_scope();
    let local = deno_core::v8::Local::new(scope, result);
    
    // Convert V8 value to serde_json::Value
    let result_value = deno_core::serde_v8::from_v8::<Value>(scope, local)
        .map_err(|e| ScriptError::Execution(format!("Failed to convert result: {e}")))?;

    Ok(result_value)
}

/// Transpile TypeScript to JavaScript using deno_ast
fn transpile_typescript(source: &str) -> Result<String> {
    use deno_ast::{MediaType, ParseParams, SourceTextInfo, TranspileOptions};

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
            &Default::default(),
        )
        .map_err(|e| ScriptError::Transpile(format!("Transpile error: {e}")))?;

    Ok(transpiled.into_source().text.to_string())
}

// Define Nanna extension with ops
extension!(
    nanna_extension,
    ops = [
        op_nanna_log,
        op_nanna_fetch,
        op_nanna_read_file,
        op_nanna_write_file,
    ],
    esm_entry_point = "ext:nanna_extension/runtime.js",
    esm = [
        dir "src/js",
        "runtime.js" = r#"
            globalThis.Nanna = {
                log: (level, msg) => Deno.core.ops.op_nanna_log(level, msg),
                fetch: async (url, options) => {
                    const result = await Deno.core.ops.op_nanna_fetch(url, options || {});
                    return JSON.parse(result);
                },
                readFile: async (path) => await Deno.core.ops.op_nanna_read_file(path),
                writeFile: async (path, content) => await Deno.core.ops.op_nanna_write_file(path, content),
            };
            
            globalThis.console = {
                log: (...args) => Nanna.log('info', args.map(a => String(a)).join(' ')),
                warn: (...args) => Nanna.log('warn', args.map(a => String(a)).join(' ')),
                error: (...args) => Nanna.log('error', args.map(a => String(a)).join(' ')),
                debug: (...args) => Nanna.log('debug', args.map(a => String(a)).join(' ')),
            };
        "#,
    ],
);

#[op2]
fn op_nanna_log(#[string] level: String, #[string] message: String) {
    match level.as_str() {
        "debug" => tracing::debug!(target: "script", "{}", message),
        "info" => tracing::info!(target: "script", "{}", message),
        "warn" => tracing::warn!(target: "script", "{}", message),
        "error" => tracing::error!(target: "script", "{}", message),
        _ => tracing::info!(target: "script", "{}", message),
    }
}

#[op2(async)]
#[string]
async fn op_nanna_fetch(
    #[string] url: String,
    #[serde] options: serde_json::Value,
) -> std::result::Result<String, AnyError> {
    // TODO: Implement with permission checking via bridge
    // For now, just do a basic fetch
    let client = reqwest::Client::new();
    
    let method = options
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET");
    
    let mut request = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        _ => client.get(&url),
    };
    
    if let Some(body) = options.get("body").and_then(Value::as_str) {
        request = request.body(body.to_string());
    }
    
    let response = request.send().await?;
    let status = response.status().as_u16();
    let body = response.text().await?;
    
    Ok(serde_json::json!({
        "status": status,
        "body": body
    }).to_string())
}

#[op2(async)]
#[string]
async fn op_nanna_read_file(#[string] path: String) -> std::result::Result<String, AnyError> {
    // TODO: Permission checking
    Ok(tokio::fs::read_to_string(&path).await?)
}

#[op2(async)]
async fn op_nanna_write_file(
    #[string] path: String,
    #[string] content: String,
) -> std::result::Result<(), AnyError> {
    // TODO: Permission checking
    Ok(tokio::fs::write(&path, &content).await?)
}

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
