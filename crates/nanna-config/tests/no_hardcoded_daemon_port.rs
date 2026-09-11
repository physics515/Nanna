//! Guard: the daemon IPC port has exactly one definition.
//!
//! History, and why a test rather than a comment. The port was once a literal
//! repeated across the daemon, the CLI and the docs. The copies drifted —
//! `nanna daemon start` bound `9999` while `nanna daemon status` probed `5149`
//! — and a CLI-started daemon reported itself as not running.
//!
//! `DEFAULT_IPC_PORT` was introduced to end that, and it did not: on
//! 2026-09-09 both `nanna-client/src/connection.rs` and
//! `gui/src-tauri/src/daemon_client.rs` still built `ws://127.0.0.1:5149` from
//! a string literal. The GUI could not even see the constant — it deliberately
//! does not link `nanna-daemon` (P16: the GUI is a thin daemon client) — so the
//! constant now lives in `nanna-config`, and this test is what keeps the
//! literal from coming back.
//!
//! Scope is deliberately narrow: **Rust sources, excluding tests, benches and
//! fixtures.** A test asserting a URL is documenting an expectation, not
//! creating a second source of truth.

use std::path::{Path, PathBuf};

/// Locate the workspace root from this crate's manifest dir.
fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/crates/nanna-config → up two levels.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate should live two levels below the workspace root")
        .to_path_buf()
}

/// Collect every production `.rs` file under `dir`.
fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // Build outputs, vendored code, and the test/fixture trees whose
            // literals are assertions rather than configuration.
            if matches!(
                name.as_ref(),
                "target" | "node_modules" | ".git" | "tests" | "benches" | "e2e" | "dist" | ".nuxt"
            ) {
                continue;
            }
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_rust_source_hardcodes_the_daemon_ws_url() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_sources(&root, &mut files);
    assert!(
        files.len() > 50,
        "the walk found only {} files — it is not reaching the sources it is meant to guard",
        files.len()
    );

    let port = nanna_config::DEFAULT_IPC_PORT.to_string();
    let mut offenders = Vec::new();

    for file in &files {
        // The one file allowed to spell the port out is the one that defines it.
        if file.ends_with("bind.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            // Comments are excluded: this history is deliberately written down
            // in several places.
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('*') {
                continue;
            }
            // A ws:// or http:// URL carrying the port literal — the exact
            // shape that drifted.
            if (line.contains("ws://") || line.contains("http://")) && line.contains(&port) {
                offenders.push(format!(
                    "{}:{}: {}",
                    file.strip_prefix(&root).unwrap_or(file).display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "the daemon URL is hardcoded instead of built from \
         `nanna_config::default_daemon_ws_url()`:\n  {}\n\nThis is how `nanna daemon start` once \
         bound 9999 while `nanna daemon status` probed 5149.",
        offenders.join("\n  ")
    );
}

/// The IPC read limit has one definition too, and every client applies it.
///
/// A WebSocket read limit protects only the side that sets it. The daemon
/// raised its own to 128 MB after long sessions overflowed tungstenite's
/// 16 MiB frame default, the GUI copied the number beside a comment saying it
/// "must match", and `nanna-client` never set one — so every large reply the
/// daemon sent to the CLI could still drop the connection. Found 2026-09-11.
#[test]
fn the_ipc_message_limit_has_one_definition_and_the_client_applies_it() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_sources(&root, &mut files);
    assert!(
        files.len() > 50,
        "the walk is not reaching the sources it guards"
    );

    // The drift's shape, not its number: a WebSocket read limit set from
    // anything but the shared constant, or a local copy of it. (Matching on
    // `128 * 1024 * 1024` itself also caught the embedded Python interpreter's
    // unrelated 128 MiB stack floor.)
    let mut offenders = Vec::new();
    let mut limit_sites = 0_usize;
    for file in &files {
        if file.ends_with("bind.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('*') {
                continue;
            }
            let sets_limit = line.contains("max_message_size") || line.contains("max_frame_size");
            let uses_shared = line.contains("IPC_MAX_MESSAGE_BYTES");
            let local_copy = trimmed.contains("const ")
                && ["MESSAGE_SIZE", "FRAME_SIZE", "MESSAGE_BYTES"]
                    .iter()
                    .any(|name| line.contains(name));
            limit_sites += usize::from(sets_limit && uses_shared);
            if (sets_limit && !uses_shared) || local_copy {
                offenders.push(format!(
                    "{}:{}: {}",
                    file.strip_prefix(&root).unwrap_or(file).display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the IPC message limit is set or copied instead of using \
         `nanna_config::bind::IPC_MAX_MESSAGE_BYTES`:\n  {}",
        offenders.join("\n  ")
    );
    // Positive space: the daemon's accept, nanna-client's connect and the
    // GUI's daemon_client each set both limits — or the guard reaches nothing.
    assert!(
        limit_sites >= 6,
        "expected the three IPC endpoints to set both limits from the shared \
         constant; found {limit_sites} such lines"
    );

    // The client must pass an explicit config: a bare `connect_async` reads
    // with tungstenite's 16 MiB frame default, which is the bug itself.
    let client = std::fs::read_to_string(root.join("crates/nanna-client/src/connection.rs"))
        .expect("nanna-client's connection module exists");
    assert!(
        client.contains("connect_async_with_config"),
        "nanna-client must connect with an explicit WebSocketConfig"
    );
    assert!(
        !client.contains("connect_async(&"),
        "nanna-client still has a bare connect_async call, which reads with the 16 MiB default"
    );
}

#[test]
fn the_url_helper_agrees_with_its_parts() {
    let url = nanna_config::default_daemon_ws_url();
    assert!(url.starts_with("ws://"), "got {url}");
    assert!(
        url.contains(nanna_config::LOOPBACK_HOST),
        "must bind loopback, got {url}"
    );
    assert!(
        url.ends_with(&format!(":{}", nanna_config::DEFAULT_IPC_PORT)),
        "must carry the one port constant, got {url}"
    );
}
