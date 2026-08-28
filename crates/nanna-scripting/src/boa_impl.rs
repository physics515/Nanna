//! Boa JavaScript engine implementation
//!
//! Pure Rust JS engine - lightweight but limited ECMAScript support.

use crate::{NannaBridge, Result, ScriptError, ScriptedTool};
use boa_engine::{
    js_string, property::Attribute, Context, JsArgs, JsResult,
    JsValue, NativeFunction, Source,
};
use serde_json::Value;
use std::cell::RefCell;
use std::sync::Arc;

// Thread-local bridge for native function access
thread_local! {
    static BRIDGE: RefCell<Option<Arc<NannaBridge>>> = const { RefCell::new(None) };
}

/// Execute a tool with Boa
pub async fn execute(
    tool: &ScriptedTool,
    input: &Value,
    bridge: &Arc<NannaBridge>,
) -> Result<Value> {
    // Transpile TypeScript if needed
    let source = if tool.is_typescript {
        transpile_typescript(&tool.source)?
    } else {
        tool.source.clone()
    };

    // Wrap execution in a blocking task (Boa is sync)
    let source_clone = source.clone();
    let input_clone = input.clone();
    let bridge_clone = bridge.clone();
    let timeout_ms = tool.timeout_ms;

    let result = tokio::task::spawn_blocking(move || {
        execute_sync(&source_clone, &input_clone, &bridge_clone)
    });

    // Apply timeout
    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), result).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => Err(ScriptError::Execution(format!("Task panicked: {e}"))),
        Err(_) => Err(ScriptError::Timeout(timeout_ms)),
    }
}

/// Synchronous execution (runs in blocking task)
fn execute_sync(
    source: &str,
    input: &Value,
    bridge: &Arc<NannaBridge>,
) -> Result<Value> {
    // Store bridge in thread-local for native function access
    BRIDGE.with(|b| *b.borrow_mut() = Some(bridge.clone()));
    
    let mut context = Context::default();

    // Register console.log
    register_console(&mut context)?;

    // Register Nanna bridge functions
    register_nanna_bridge(&mut context)?;

    // Inject INPUT as a global
    let input_js = json_to_js(input, &mut context)?;
    context
        .register_global_property(js_string!("INPUT"), input_js, Attribute::READONLY)
        .map_err(|e| ScriptError::Execution(format!("Failed to set INPUT: {e}")))?;

    // Transform ES6 "export default" to a variable assignment that Boa can handle
    // Boa doesn't support ES modules, so we convert to CommonJS-style
    let transformed_source = source.replace("export default", "var __exported__ =");
    
    tracing::debug!(target: "script", "Transformed source for Boa execution");

    // Wrap source in an IIFE that calls execute()
    let wrapped = format!(
        r#"
        (function() {{
            {transformed_source}
            
            // Find the exported tool object
            if (typeof __exported__ !== 'undefined' && __exported__ && typeof __exported__.execute === 'function') {{
                return __exported__.execute(INPUT);
            }}
            
            // Get the default export (CommonJS style)
            if (typeof module !== 'undefined' && module.exports && module.exports.default) {{
                var tool = module.exports.default;
                if (typeof tool.execute === 'function') {{
                    return tool.execute(INPUT);
                }}
            }}
            
            // Try to find execute function in global scope
            if (typeof execute === 'function') {{
                return execute(INPUT);
            }}
            
            throw new Error('No execute function found in tool. Make sure your tool exports an object with an execute function.');
        }})()
        "#
    );

    // Parse and execute
    let result = context
        .eval(Source::from_bytes(&wrapped))
        .map_err(|e| {
            tracing::error!(target: "script", "JS execution failed: {}", e);
            ScriptError::Execution(format!("Execution failed: {e}"))
        })?;
    
    tracing::debug!(target: "script", "JS execution completed successfully");

    // Convert result back to JSON
    js_to_json(&result, &mut context)
}

/// Register console.log/warn/error
fn register_console(context: &mut Context) -> Result<()> {
    // Create console object
    let console = boa_engine::object::ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(console_log),
            js_string!("log"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(console_warn),
            js_string!("warn"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(console_error),
            js_string!("error"),
            0,
        )
        .build();

    context
        .register_global_property(js_string!("console"), console, Attribute::all())
        .map_err(|e| ScriptError::Execution(format!("Failed to register console: {e}")))?;

    Ok(())
}

fn console_log(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let msg = format_console_args(args, context);
    tracing::info!(target: "script", "{}", msg);
    Ok(JsValue::undefined())
}

fn console_warn(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let msg = format_console_args(args, context);
    tracing::warn!(target: "script", "{}", msg);
    Ok(JsValue::undefined())
}

fn console_error(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let msg = format_console_args(args, context);
    tracing::error!(target: "script", "{}", msg);
    Ok(JsValue::undefined())
}

fn format_console_args(args: &[JsValue], context: &mut Context) -> String {
    args.iter()
        .map(|v| {
            v.to_string(context)
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|_| "[object]".to_string())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Register Nanna bridge functions
fn register_nanna_bridge(context: &mut Context) -> Result<()> {
    // Create Nanna object with bridge functions
    // Note: exec() is blocking here - real async would need Boa's promise/job queue
    
    let nanna = boa_engine::object::ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(nanna_log),
            js_string!("log"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_exec),
            js_string!("exec"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_read_file),
            js_string!("readFile"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_write_file),
            js_string!("writeFile"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_list_dir),
            js_string!("listDir"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_stat),
            js_string!("stat"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_fetch),
            js_string!("fetch"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_get_env),
            js_string!("getEnv"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_list_tools),
            js_string!("listTools"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_search_tools),
            js_string!("searchTools"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_service),
            js_string!("service"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_session_id),
            js_string!("sessionId"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(nanna_workdir),
            js_string!("workdir"),
            0,
        )
        .property(
            js_string!("platform"),
            js_string!(NannaBridge::platform()),
            Attribute::READONLY,
        )
        .build();

    context
        .register_global_property(js_string!("Nanna"), nanna, Attribute::all())
        .map_err(|e| ScriptError::Execution(format!("Failed to register Nanna: {e}")))?;

    Ok(())
}

fn nanna_log(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let level = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();
    let msg = args.get_or_undefined(1).to_string(context)?.to_std_string_escaped();
    
    match level.as_str() {
        "debug" => tracing::debug!(target: "script", "{}", msg),
        "info" => tracing::info!(target: "script", "{}", msg),
        "warn" => tracing::warn!(target: "script", "{}", msg),
        "error" => tracing::error!(target: "script", "{}", msg),
        _ => tracing::info!(target: "script", "{}", msg),
    }
    
    Ok(JsValue::undefined())
}

fn nanna_list_tools(_: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let bridge = BRIDGE.with(|b| b.borrow().clone())
        .ok_or_else(|| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!("Bridge not initialized")))
        })?;

    match bridge.list_tools() {
        Some(defs) => json_to_js(defs, context).map_err(|e| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.to_string().as_str())))
        }),
        None => Ok(JsValue::null()),
    }
}

/// `Nanna.searchTools(query[, limit])` — ranked BM25 tool search.
///
/// Contract: NEVER throws. A missing bridge, an unattached search function
/// (older embedding contexts / bare test harnesses), or an unserializable
/// result all degrade to an empty array so the calling skill can fall back
/// to its own matching.
fn nanna_search_tools(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    fn empty_array(context: &mut Context) -> JsResult<JsValue> {
        Ok(boa_engine::object::builtins::JsArray::new(context)?.into())
    }

    let Ok(query) = args.get_or_undefined(0).to_string(context) else {
        return empty_array(context);
    };
    let query = query.to_std_string_escaped();

    let limit = args
        .get(1)
        .filter(|v| !v.is_undefined() && !v.is_null())
        .and_then(|v| v.to_number(context).ok())
        .filter(|n| n.is_finite() && *n >= 1.0)
        .map_or(crate::bridge::DEFAULT_TOOL_SEARCH_LIMIT, |n| n as usize);

    let Some(bridge) = BRIDGE.with(|b| b.borrow().clone()) else {
        return empty_array(context);
    };

    match bridge.search_tools(&query, limit) {
        Some(hits) => json_to_js(&hits, context).or_else(|_| empty_array(context)),
        None => empty_array(context),
    }
}

/// Byte bound on one captured-output excerpt in the exec log line.
///
/// Reuses the width the exec skill itself already uses when it puts a one-line
/// excerpt of a bridge error or a checker's stderr in front of the model
/// (`default-skills/exec/tool.ts`: `if (err.length > 200) err =
/// err.substring(0, 200)`). Same job — identify an output, do not reproduce it
/// — so it gets the same width rather than a second number.
const EXEC_LOG_EXCERPT_BYTES: usize = 200;

/// One log-safe line identifying a captured stream: its first non-empty line,
/// bounded.
///
/// The first non-empty line is the identity the rest of this codebase already
/// uses for a result (`loop_runner`'s failure signature normalizes to exactly
/// that), and it keeps a log LINE a line — output arrives with its own
/// newlines and an embedded one would split the record in two.
///
/// Slices on a char boundary, never a byte index: what comes back here is
/// arbitrary command output, and a byte index that splits a multi-byte
/// character panics (an em dash in an error message has already killed this
/// daemon once).
fn log_excerpt(text: &str) -> String {
    let Some(line) = text.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return String::new();
    };
    if line.len() <= EXEC_LOG_EXCERPT_BYTES {
        return line.to_string();
    }
    let mut end = EXEC_LOG_EXCERPT_BYTES;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

fn nanna_exec(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let command = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();
    let workdir = args.get(1)
        .filter(|v| !v.is_undefined() && !v.is_null())
        .map(|v| v.to_string(context).map(|s| s.to_std_string_escaped()))
        .transpose()?;
    let timeout_secs = args.get(2)
        .filter(|v| !v.is_undefined() && !v.is_null())
        .and_then(|v| v.to_number(context).ok())
        .map(|n| n as u64);
    
    tracing::info!(target: "script", "Nanna.exec called with command: {}", command);
    
    // Get bridge from thread-local storage
    let bridge = BRIDGE.with(|b| b.borrow().clone())
        .ok_or_else(|| {
            tracing::error!(target: "script", "Bridge not initialized in thread-local storage");
            boa_engine::JsError::from_opaque(JsValue::from(js_string!("Bridge not initialized")))
        })?;
    
    tracing::info!(target: "script", "Bridge found, run permission: {}", bridge.permissions.run);
    
    // Execute synchronously using a new thread with its own runtime
    // (we're in a blocking task but need async for the bridge)
    let result = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");
        rt.block_on(bridge.exec_with_timeout(&command, workdir.as_deref(), timeout_secs))
    })
    .join()
    .map_err(|e| {
        tracing::error!(target: "script", "Thread panicked: {:?}", e);
        boa_engine::JsError::from_opaque(JsValue::from(js_string!("Thread panicked")))
    })?;
    
    match result {
        Ok(response) => {
            // Excerpts, plus the real sizes. This used to print stdout and
            // stderr in full: a command whose output was one line repeated put
            // ~300 MB of it into a single day's log, and the log write itself
            // is on the turn's critical path. The lengths are the diagnostic
            // that was actually missing anyway — a reader could not tell a
            // huge result from a small one without measuring the log line.
            tracing::info!(
                target: "script",
                "Exec success: {}, code: {:?}, stdout: {} bytes [{}], stderr: {} bytes [{}]",
                response.success,
                response.code,
                response.stdout.len(),
                log_excerpt(&response.stdout),
                response.stderr.len(),
                log_excerpt(&response.stderr)
            );
            // Build result object
            let obj = boa_engine::object::JsObject::with_object_proto(context.intrinsics());
            obj.set(js_string!("success"), JsValue::from(response.success), false, context)?;
            obj.set(js_string!("code"), response.code.map_or(JsValue::null(), |c| JsValue::from(c)), false, context)?;
            obj.set(js_string!("stdout"), JsValue::from(js_string!(response.stdout.as_str())), false, context)?;
            obj.set(js_string!("stderr"), JsValue::from(js_string!(response.stderr.as_str())), false, context)?;
            // A deadline kill still ran the command: the tool layer needs both
            // facts to say so instead of "nothing ran".
            obj.set(js_string!("timed_out"), JsValue::from(response.timed_out), false, context)?;
            // JS numbers are f64: every integer below 2^53 is exact, and an exec
            // elapsed time cannot reach that (the deadline bounds it to minutes).
            obj.set(
                js_string!("elapsed_ms"),
                JsValue::from(response.elapsed_ms as f64),
                false,
                context,
            )?;
            Ok(obj.into())
        }
        Err(e) => {
            tracing::error!(target: "script", "Exec failed: {}", e);
            Err(boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.to_string().as_str()))))
        }
    }
}

fn nanna_read_file(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let path = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();

    let bridge = BRIDGE.with(|b| b.borrow().clone())
        .ok_or_else(|| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!("Bridge not initialized")))
        })?;

    let result = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");
        rt.block_on(bridge.read_file(&path))
    })
    .join()
    .map_err(|_| {
        boa_engine::JsError::from_opaque(JsValue::from(js_string!("Thread panicked")))
    })?;

    match result {
        Ok(content) => Ok(JsValue::from(js_string!(content.as_str()))),
        Err(e) => Err(boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.to_string().as_str())))),
    }
}

fn nanna_write_file(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let path = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();
    let content = args.get_or_undefined(1).to_string(context)?.to_std_string_escaped();

    let bridge = BRIDGE.with(|b| b.borrow().clone())
        .ok_or_else(|| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!("Bridge not initialized")))
        })?;

    let result = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");
        rt.block_on(bridge.write_file(&path, &content))
    })
    .join()
    .map_err(|_| {
        boa_engine::JsError::from_opaque(JsValue::from(js_string!("Thread panicked")))
    })?;

    match result {
        Ok(()) => Ok(JsValue::undefined()),
        Err(e) => Err(boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.to_string().as_str())))),
    }
}

fn nanna_list_dir(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let path = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();
    let recursive = args.get(1)
        .filter(|v| !v.is_undefined() && !v.is_null())
        .map(|v| v.to_boolean())
        .unwrap_or(false);
    // Optional third arg: bounded query — collect at most this many entries
    // (see NannaBridge::list_dir). Non-numeric / non-positive values mean
    // "no bound", matching a caller that simply omitted the argument.
    let max_entries = args.get(2)
        .filter(|v| !v.is_undefined() && !v.is_null())
        .map(|v| v.to_number(context))
        .transpose()?
        .filter(|n| n.is_finite() && *n >= 1.0)
        .map(|n| n as usize);

    let bridge = BRIDGE.with(|b| b.borrow().clone())
        .ok_or_else(|| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!("Bridge not initialized")))
        })?;

    let result = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");
        rt.block_on(bridge.list_dir(&path, recursive, max_entries))
    })
    .join()
    .map_err(|_| {
        boa_engine::JsError::from_opaque(JsValue::from(js_string!("Thread panicked")))
    })?;

    match result {
        Ok(entries) => {
            // boa 1.0-dev: JsArray::new is fallible.
            let js_array = boa_engine::object::builtins::JsArray::new(context)?;
            for entry in entries {
                let obj = boa_engine::object::JsObject::with_object_proto(context.intrinsics());
                obj.set(js_string!("name"), JsValue::from(js_string!(entry.name.as_str())), false, context)?;
                obj.set(js_string!("entry_type"), JsValue::from(js_string!(entry.entry_type.as_str())), false, context)?;
                obj.set(js_string!("size"), JsValue::from(entry.size as f64), false, context)?;
                obj.set(js_string!("modified"), entry.modified.map_or(JsValue::null(), |m| JsValue::from(m as f64)), false, context)?;
                js_array.push(JsValue::from(obj), context)?;
            }
            Ok(js_array.into())
        }
        Err(e) => Err(boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.to_string().as_str())))),
    }
}

fn nanna_stat(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let path = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();

    let bridge = BRIDGE.with(|b| b.borrow().clone())
        .ok_or_else(|| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!("Bridge not initialized")))
        })?;

    let result = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");
        rt.block_on(bridge.stat(&path))
    })
    .join()
    .map_err(|_| {
        boa_engine::JsError::from_opaque(JsValue::from(js_string!("Thread panicked")))
    })?;

    match result {
        Ok(stat) => {
            let obj = boa_engine::object::JsObject::with_object_proto(context.intrinsics());
            obj.set(js_string!("size"), JsValue::from(stat.size as f64), false, context)?;
            obj.set(js_string!("is_file"), JsValue::from(stat.is_file), false, context)?;
            obj.set(js_string!("is_dir"), JsValue::from(stat.is_dir), false, context)?;
            obj.set(js_string!("modified"), stat.modified.map_or(JsValue::null(), |m| JsValue::from(m as f64)), false, context)?;
            Ok(obj.into())
        }
        Err(e) => Err(boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.to_string().as_str())))),
    }
}

fn nanna_fetch(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let url = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();

    // Parse optional options object
    let options = if let Some(opts_val) = args.get(1).filter(|v| v.is_object()) {
        let opts_json = js_to_json(opts_val, context).map_err(|e| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.to_string().as_str())))
        })?;
        serde_json::from_value::<crate::bridge::FetchOptions>(opts_json).ok()
    } else {
        None
    };

    let bridge = BRIDGE.with(|b| b.borrow().clone())
        .ok_or_else(|| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!("Bridge not initialized")))
        })?;

    let result = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");
        rt.block_on(bridge.fetch(&url, options))
    })
    .join()
    .map_err(|_| {
        boa_engine::JsError::from_opaque(JsValue::from(js_string!("Thread panicked")))
    })?;

    match result {
        Ok(response) => {
            let obj = boa_engine::object::JsObject::with_object_proto(context.intrinsics());
            obj.set(js_string!("status"), JsValue::from(response.status as f64), false, context)?;
            obj.set(js_string!("body"), JsValue::from(js_string!(response.body.as_str())), false, context)?;
            // Convert headers to JS object
            let headers_obj = boa_engine::object::JsObject::with_object_proto(context.intrinsics());
            for (key, value) in &response.headers {
                headers_obj.set(js_string!(key.as_str()), JsValue::from(js_string!(value.as_str())), false, context)?;
            }
            obj.set(js_string!("headers"), JsValue::from(headers_obj), false, context)?;
            Ok(obj.into())
        }
        Err(e) => Err(boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.to_string().as_str())))),
    }
}

fn nanna_get_env(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();

    let bridge = BRIDGE.with(|b| b.borrow().clone())
        .ok_or_else(|| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!("Bridge not initialized")))
        })?;

    // get_env is sync — no thread spawn needed
    match bridge.get_env(&key) {
        Ok(Some(value)) => Ok(JsValue::from(js_string!(value.as_str()))),
        Ok(None) => Ok(JsValue::null()),
        Err(e) => Err(boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.to_string().as_str())))),
    }
}

fn nanna_session_id(_: &JsValue, _args: &[JsValue], _context: &mut Context) -> JsResult<JsValue> {
    BRIDGE.with(|cell| {
        let borrow = cell.borrow();
        if let Some(ref bridge) = *borrow {
            if let Some(sid) = bridge.session_id() {
                Ok(JsValue::from(js_string!(sid)))
            } else {
                Ok(JsValue::null())
            }
        } else {
            Ok(JsValue::null())
        }
    })
}

fn nanna_workdir(_: &JsValue, _args: &[JsValue], _context: &mut Context) -> JsResult<JsValue> {
    BRIDGE.with(|cell| {
        let borrow = cell.borrow();
        if let Some(ref bridge) = *borrow {
            if let Some(wd) = bridge.workdir() {
                Ok(JsValue::from(js_string!(wd)))
            } else {
                Ok(JsValue::null())
            }
        } else {
            Ok(JsValue::null())
        }
    })
}

fn nanna_service(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();
    let params = if let Some(p) = args.get(1).filter(|v| v.is_object()) {
        js_to_json(p, context).map_err(|e| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.to_string().as_str())))
        })?
    } else {
        Value::Object(serde_json::Map::new())
    };

    let bridge = BRIDGE.with(|b| b.borrow().clone())
        .ok_or_else(|| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!("Bridge not initialized")))
        })?;

    let result = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");
        rt.block_on(bridge.call_service(&name, params))
    })
    .join()
    .map_err(|_| {
        boa_engine::JsError::from_opaque(JsValue::from(js_string!("Thread panicked")))
    })?;

    match result {
        Ok(value) => json_to_js(&value, context).map_err(|e| {
            boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.to_string().as_str())))
        }),
        Err(e) => Err(boa_engine::JsError::from_opaque(JsValue::from(js_string!(e.as_str())))),
    }
}

/// Convert JSON Value to Boa JsValue
fn json_to_js(value: &Value, context: &mut Context) -> Result<JsValue> {
    match value {
        Value::Null => Ok(JsValue::null()),
        Value::Bool(b) => Ok(JsValue::from(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(JsValue::from(i as f64))
            } else if let Some(f) = n.as_f64() {
                Ok(JsValue::from(f))
            } else {
                Ok(JsValue::from(0.0))
            }
        }
        Value::String(s) => Ok(JsValue::from(js_string!(s.as_str()))),
        Value::Array(arr) => {
            // boa 1.0-dev: JsArray::new is fallible.
            let js_array = boa_engine::object::builtins::JsArray::new(context)
                .map_err(|e| ScriptError::Execution(format!("Array creation failed: {e}")))?;
            for item in arr {
                let js_item = json_to_js(item, context)?;
                js_array.push(js_item, context)
                    .map_err(|e| ScriptError::Execution(format!("Array push failed: {e}")))?;
            }
            Ok(js_array.into())
        }
        Value::Object(obj) => {
            let js_obj = boa_engine::object::JsObject::with_object_proto(context.intrinsics());
            for (key, val) in obj {
                let js_val = json_to_js(val, context)?;
                js_obj.set(js_string!(key.as_str()), js_val, false, context)
                    .map_err(|e| ScriptError::Execution(format!("Object set failed: {e}")))?;
            }
            Ok(js_obj.into())
        }
    }
}

/// Convert Boa JsValue to JSON Value
fn js_to_json(value: &JsValue, context: &mut Context) -> Result<Value> {
    // Use Boa's variant enum for pattern matching
    use boa_engine::value::JsVariant;
    
    match value.variant() {
        JsVariant::Undefined | JsVariant::Null => Ok(Value::Null),
        JsVariant::Boolean(b) => Ok(Value::Bool(b)),
        JsVariant::Integer32(i) => Ok(Value::Number(i.into())),
        JsVariant::Float64(f) => {
            serde_json::Number::from_f64(f)
                .map(Value::Number)
                .ok_or_else(|| ScriptError::Execution("Invalid float".to_string()))
        }
        JsVariant::String(s) => Ok(Value::String(s.to_std_string_escaped())),
        JsVariant::Object(obj) => {
            // Check if it's an array
            if obj.is_array() {
                let length = obj
                    .get(js_string!("length"), context)
                    .map_err(|e| ScriptError::Execution(format!("Get length failed: {e}")))?
                    .to_u32(context)
                    .map_err(|e| ScriptError::Execution(format!("Length to u32 failed: {e}")))?;

                let mut arr = Vec::with_capacity(length as usize);
                for i in 0..length {
                    let item = obj
                        .get(i, context)
                        .map_err(|e| ScriptError::Execution(format!("Array get failed: {e}")))?;
                    arr.push(js_to_json(&item, context)?);
                }
                Ok(Value::Array(arr))
            } else {
                // Regular object
                let keys = obj
                    .own_property_keys(context)
                    .map_err(|e| ScriptError::Execution(format!("Get keys failed: {e}")))?;

                let mut map = serde_json::Map::new();
                for key in keys {
                    // Convert PropertyKey to string
                    let key_str = match &key {
                        boa_engine::property::PropertyKey::String(s) => s.to_std_string_escaped(),
                        boa_engine::property::PropertyKey::Symbol(_) => continue, // Skip symbols
                        boa_engine::property::PropertyKey::Index(i) => i.get().to_string(),
                    };
                    
                    let val = obj
                        .get(key, context)
                        .map_err(|e| ScriptError::Execution(format!("Object get failed: {e}")))?;
                    
                    // Skip functions
                    if !val.is_callable() {
                        map.insert(key_str, js_to_json(&val, context)?);
                    }
                }
                Ok(Value::Object(map))
            }
        }
        JsVariant::BigInt(_) => Ok(Value::Null), // BigInt not directly representable in JSON
        JsVariant::Symbol(_) => Ok(Value::Null), // Symbols not representable in JSON
    }
}

/// Transpile TypeScript to JavaScript
/// Note: Boa doesn't support TypeScript natively. If actual TS syntax is present,
/// execution will fail and trigger Deno fallback (which has real TS support).
fn transpile_typescript(source: &str) -> Result<String> {
    // Just pass through - Boa handles plain JS, Deno handles TS
    Ok(source.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_roundtrip() {
        let mut context = Context::default();
        
        let input = serde_json::json!({
            "name": "test",
            "count": 42.0,  // Use float to match Boa's internal representation
            "enabled": true,
            "tags": ["a", "b", "c"]
        });
        
        let js_val = json_to_js(&input, &mut context).unwrap();
        let output = js_to_json(&js_val, &mut context).unwrap();
        
        // Compare structurally (Boa uses f64 internally, so integers become floats)
        assert_eq!(output["name"], "test");
        assert_eq!(output["count"].as_f64().unwrap() as i64, 42);
        assert_eq!(output["enabled"], true);
        assert_eq!(output["tags"].as_array().unwrap().len(), 3);
    }

    /// The exec log line used to print stdout in full: one repeated line
    /// produced ~300 MB of log in a single day. The excerpt is bounded and
    /// stays ONE line whatever the command emitted.
    #[test]
    fn exec_log_excerpt_is_bounded_and_single_line() {
        let repetitive = "same line\n".repeat(100_000);
        let excerpt = log_excerpt(&repetitive);
        assert_eq!(excerpt, "same line");

        let one_huge_line = "x".repeat(10_000_000);
        let excerpt = log_excerpt(&one_huge_line);
        assert!(
            excerpt.len() <= EXEC_LOG_EXCERPT_BYTES + '…'.len_utf8(),
            "excerpt of a 10 MB single line was {} bytes",
            excerpt.len()
        );
        assert!(!excerpt.contains('\n'));
    }

    /// Command output is arbitrary bytes, and a byte index that splits a
    /// multi-byte character panics — the defect that killed the daemon once.
    #[test]
    fn exec_log_excerpt_cuts_on_a_char_boundary() {
        // Every char is 3 bytes, so the bound lands mid-character.
        let em_dashes = "—".repeat(500);
        let excerpt = log_excerpt(&em_dashes);
        assert!(excerpt.len() <= EXEC_LOG_EXCERPT_BYTES + '…'.len_utf8());
        assert!(excerpt.ends_with('…'));
    }

    /// Leading blank lines are skipped: the identifying line is the first one
    /// with content, not whatever whitespace the shell emitted first.
    #[test]
    fn exec_log_excerpt_skips_leading_blank_lines() {
        assert_eq!(log_excerpt("\n\n  \nreal output\nmore\n"), "real output");
        assert_eq!(log_excerpt(""), "");
        assert_eq!(log_excerpt("\n \n"), "");
    }
}
