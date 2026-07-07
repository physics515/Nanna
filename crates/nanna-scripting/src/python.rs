//! Embedded Python interpreter powered by RustPython.
//!
//! Provides a zero-dependency Python execution environment for AI tool use.
//! No system Python installation required — the interpreter is compiled into the binary.
//!
//! Supports standard library modules (os, json, re, pathlib, collections, etc.)
//! but not C-extension packages (numpy, requests, etc.).

use crate::{Result, ScriptError};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Result of executing Python code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Manages embedded RustPython execution.
///
/// Each execution creates a fresh interpreter to ensure isolation.
/// This is lightweight enough for tool-call frequency.
pub struct PythonEngine;

impl PythonEngine {
    /// Create a new Python engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Execute Python code and capture stdout/stderr.
    ///
    /// Each call gets a fresh interpreter — variables don't leak between calls.
    /// Working directory is set via `os.chdir()` if provided.
    pub async fn execute(
        &self,
        code: &str,
        workdir: Option<&str>,
        timeout_secs: u64,
    ) -> Result<PythonResult> {
        let code = code.to_string();
        let workdir = workdir.map(String::from);

        let start = std::time::Instant::now();

        // Run in a blocking task since RustPython is synchronous and !Send
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            tokio::task::spawn_blocking(move || {
                execute_isolated(&code, workdir.as_deref())
            }),
        )
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(mut py_result)) => {
                py_result.duration_ms = duration_ms;
                debug!(
                    success = py_result.success,
                    duration_ms,
                    stdout_len = py_result.stdout.len(),
                    "Python execution complete"
                );
                Ok(py_result)
            }
            Ok(Err(e)) => Err(ScriptError::Execution(format!(
                "Python task panicked: {e}"
            ))),
            Err(_) => Err(ScriptError::Timeout(timeout_secs * 1000)),
        }
    }
}

/// Execute code in a fresh, isolated interpreter.
///
/// Uses a two-phase approach:
/// 1. Wrap user code in a try/except that captures stdout, stderr, and errors
/// 2. Write results to temp files that we read back from Rust
///
/// This avoids needing to extract Python objects from the VM directly.
fn execute_isolated(code: &str, workdir: Option<&str>) -> PythonResult {
    use rustpython_vm as vm;
    use rustpython_vm::compiler::Mode;

    // Create a fresh interpreter with stdlib (native C-equivalent modules + frozen Python bytecode)
    let interp = vm::Interpreter::with_init(Default::default(), |vm| {
        vm.add_native_modules(rustpython_stdlib::get_module_inits());
        vm.add_frozen(rustpython_pylib::FROZEN_STDLIB);
    });

    // Build wrapper code that captures everything
    let wrapper = build_wrapper(code, workdir);

    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();

        match vm
            .compile(&wrapper, Mode::Exec, "<nanna>".to_owned())
            .map_err(|e| format!("SyntaxError: {e}"))
            .and_then(|code_obj| {
                vm.run_code_obj(code_obj, scope.clone())
                    .map_err(|exc| format_exception(vm, exc))
            }) {
            Ok(_) => {
                // Extract results from the _nanna_result dict in scope
                let extract = r#"
import json as _nj
_nanna_json = _nj.dumps(_nanna_result)
"#;
                if let Ok(code_obj) = vm.compile(extract, Mode::Exec, "<nanna-extract>".to_owned()) {
                    let _ = vm.run_code_obj(code_obj, scope.clone());
                }

                // Get the JSON result by running a minimal expression
                let json_code = "_nanna_json";
                if let Ok(code_obj) = vm.compile(json_code, Mode::Eval, "<nanna-eval>".to_owned()) {
                    if let Ok(val) = vm.run_code_obj(code_obj, scope) {
                        if let Ok(s) = val.str(vm) {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s.as_str()) {
                                return PythonResult {
                                    stdout: parsed.get("stdout").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                    stderr: parsed.get("stderr").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                    success: parsed.get("success").and_then(|v| v.as_bool()).unwrap_or(false),
                                    error: parsed.get("error").and_then(|v| v.as_str()).map(String::from),
                                    duration_ms: 0,
                                };
                            }
                        }
                    }
                }

                // Fallback if extraction failed
                PythonResult {
                    stdout: String::new(),
                    stderr: String::new(),
                    success: true,
                    error: None,
                    duration_ms: 0,
                }
            }
            Err(error_msg) => {
                // The wrapper itself failed (shouldn't happen normally)
                PythonResult {
                    stdout: String::new(),
                    stderr: String::new(),
                    success: false,
                    error: Some(error_msg),
                    duration_ms: 0,
                }
            }
        }
    })
}

/// Build wrapper Python code that captures stdout, stderr, and exceptions.
fn build_wrapper(user_code: &str, workdir: Option<&str>) -> String {
    let chdir = if let Some(wd) = workdir {
        format!("import os; os.chdir({})\n", python_string_literal(wd))
    } else {
        String::new()
    };

    // Escape the user code for embedding in a triple-quoted string
    // We use exec() with the code as a variable to avoid any escaping issues
    let escaped_code = user_code
        .replace('\\', "\\\\")
        .replace("\"\"\"", "\\\"\\\"\\\"");

    format!(
        r#"
import sys, io, traceback
{chdir}
_nanna_stdout_buf = io.StringIO()
_nanna_stderr_buf = io.StringIO()
_nanna_orig_stdout = sys.stdout
_nanna_orig_stderr = sys.stderr
sys.stdout = _nanna_stdout_buf
sys.stderr = _nanna_stderr_buf

_nanna_result = {{"stdout": "", "stderr": "", "success": True, "error": None}}

try:
    _nanna_user_code = """{escaped_code}"""
    exec(compile(_nanna_user_code, '<python>', 'exec'))
except SystemExit:
    pass
except:
    _nanna_result["success"] = False
    _nanna_result["error"] = traceback.format_exc()
finally:
    sys.stdout = _nanna_orig_stdout
    sys.stderr = _nanna_orig_stderr
    _nanna_result["stdout"] = _nanna_stdout_buf.getvalue()
    _nanna_result["stderr"] = _nanna_stderr_buf.getvalue()
"#
    )
}

/// Format a Python exception into a human-readable string.
fn format_exception(
    vm: &rustpython_vm::VirtualMachine,
    exc: rustpython_vm::PyRef<rustpython_vm::builtins::PyBaseException>,
) -> String {
    // Convert to PyObject and call str()
    use rustpython_vm::AsObject;
    let obj = exc.as_object();
    let class_name = obj.class().name().to_string();
    let msg = obj
        .str(vm)
        .ok()
        .map(|s| s.as_str().to_owned())
        .unwrap_or_default();

    if msg.is_empty() {
        class_name
    } else {
        format!("{class_name}: {msg}")
    }
}

/// Escape a string for use as a Python string literal.
fn python_string_literal(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_execution() {
        let engine = PythonEngine::new();
        let result = engine.execute("print('hello world')", None, 10).await.unwrap();
        assert!(result.success, "error: {:?}", result.error);
        assert_eq!(result.stdout.trim(), "hello world");
    }

    #[tokio::test]
    async fn test_error_handling() {
        let engine = PythonEngine::new();
        let result = engine.execute("raise ValueError('test error')", None, 10).await.unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("ValueError"));
    }

    #[tokio::test]
    async fn test_multiline() {
        let engine = PythonEngine::new();
        let code = r#"
import json
data = {"name": "nanna", "version": "0.1.0"}
print(json.dumps(data, indent=2))
"#;
        let result = engine.execute(code, None, 10).await.unwrap();
        assert!(result.success, "error: {:?}", result.error);
        assert!(result.stdout.contains("nanna"));
    }

    #[tokio::test]
    async fn test_stdlib_modules() {
        let engine = PythonEngine::new();
        let code = r#"
import os, re, collections, pathlib
print("os:", hasattr(os, 'path'))
print("re:", bool(re.match(r'\d+', '42')))
print("Counter:", collections.Counter('aabbc'))
print("Path:", type(pathlib.Path('.')).__name__)
"#;
        let result = engine.execute(code, None, 10).await.unwrap();
        assert!(result.success, "stderr: {}, error: {:?}", result.stderr, result.error);
        assert!(result.stdout.contains("os: True"));
    }

    #[tokio::test]
    async fn test_file_operations() {
        let engine = PythonEngine::new();
        let code = r#"
import tempfile, os
with tempfile.NamedTemporaryFile(mode='w', suffix='.txt', delete=False) as f:
    f.write('hello from python')
    name = f.name
with open(name) as f:
    print(f.read())
os.unlink(name)
"#;
        let result = engine.execute(code, None, 10).await.unwrap();
        assert!(result.success, "error: {:?}", result.error);
        assert!(result.stdout.contains("hello from python"));
    }

    #[tokio::test]
    async fn test_isolation() {
        let engine = PythonEngine::new();
        // Set a variable in first execution
        let _ = engine.execute("shared_var = 42", None, 10).await.unwrap();
        // Second execution should not see it
        let result = engine.execute("print(shared_var)", None, 10).await.unwrap();
        assert!(!result.success, "Variable leaked between executions!");
    }

    #[tokio::test]
    async fn test_syntax_error() {
        let engine = PythonEngine::new();
        let result = engine.execute("def foo(", None, 10).await.unwrap();
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn test_code_with_triple_quotes() {
        let engine = PythonEngine::new();
        let code = r#"
msg = '''hello
world'''
print(msg)
"#;
        let result = engine.execute(code, None, 10).await.unwrap();
        assert!(result.success, "error: {:?}", result.error);
        assert!(result.stdout.contains("hello\nworld"));
    }
}
