//! `nanna export` — write a session out as Markdown or JSON.
//!
//! Goes through the daemon: it owns the session database (turso holds an
//! exclusive lock on it) and renders the document itself (`session.export`),
//! so the file is exactly what any other client would get.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, bail};
use nanna_client::{Client, ClientConfig, ExportFormat};
use nanna_config::bind::LOOPBACK_HOST;
use nanna_daemon::DEFAULT_IPC_PORT;
use serde_json::Value;

/// Wait this long for the daemon to accept the connection — the budget
/// `nanna daemon status` already uses for the same probe.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// The document format, as the CLI spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ExportFormatArg {
    /// A readable transcript
    #[value(alias = "md")]
    Markdown,
    /// The whole session as stored — lossless
    Json,
}

impl From<ExportFormatArg> for ExportFormat {
    fn from(arg: ExportFormatArg) -> Self {
        match arg {
            ExportFormatArg::Markdown => Self::Markdown,
            ExportFormatArg::Json => Self::Json,
        }
    }
}

/// Export `session_id` to `output` — a file, or a directory that receives the
/// daemon's suggested file name — or to stdout when `output` is `None`.
pub async fn export_session(
    session_id: &str,
    format: ExportFormatArg,
    output: Option<PathBuf>,
) -> anyhow::Result<()> {
    let address = format!("ws://{LOOPBACK_HOST}:{DEFAULT_IPC_PORT}");
    let connect = Client::connect(ClientConfig::new(&address));
    let client = match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
        Ok(Ok(client)) => client,
        Ok(Err(e)) => bail!(
            "could not reach the Nanna daemon at {address}: {e}. Start it with \
             `nanna daemon start` — it owns the session database, so export goes through it"
        ),
        Err(_) => bail!(
            "the Nanna daemon at {address} did not answer within {}s. Start it with \
             `nanna daemon start`",
            CONNECT_TIMEOUT.as_secs()
        ),
    };
    let reply = client
        .sessions()
        .export(session_id, format.into())
        .await
        .context("the daemon did not answer the export request")?;
    client.disconnect().await;

    let (filename, content) = document_from_reply(&reply)?;
    match output {
        None => print!("{content}"),
        Some(path) => {
            let target = resolve_target(&path, filename);
            std::fs::write(&target, content)
                .with_context(|| format!("could not write {}", target.display()))?;
            eprintln!("Exported session {session_id} to {}", target.display());
        }
    }
    Ok(())
}

/// Pull the document out of the daemon's reply. An `{error, message}` reply is
/// the daemon saying no — an unknown id — and becomes the command's error.
fn document_from_reply(reply: &Value) -> anyhow::Result<(&str, &str)> {
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
        let message = reply
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or(error);
        bail!("{message}");
    }
    let filename = reply
        .get("filename")
        .and_then(Value::as_str)
        .context("the daemon's export reply carries no filename")?;
    let content = reply
        .get("content")
        .and_then(Value::as_str)
        .context("the daemon's export reply carries no content")?;
    Ok((filename, content))
}

/// A directory receives the suggested file name; any other path is the file.
/// Only the name's final component is used, so a reply can never steer the
/// write outside the directory the user chose.
fn resolve_target(path: &Path, filename: &str) -> PathBuf {
    if !path.is_dir() {
        return path.to_path_buf();
    }
    let name = Path::new(filename)
        .file_name()
        .map_or_else(|| "session-export".into(), std::ffi::OsStr::to_os_string);
    path.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_document_reply_yields_its_name_and_content() {
        let reply = json!({ "format": "markdown", "filename": "a.md", "content": "# a\n" });
        let (filename, content) = document_from_reply(&reply).expect("a document reply");
        assert_eq!(filename, "a.md");
        assert_eq!(content, "# a\n");
    }

    #[test]
    fn an_error_reply_becomes_the_commands_error() {
        let reply = json!({ "error": "not_found", "message": "Session nope not found" });
        let err = document_from_reply(&reply).expect_err("an error reply is an error");
        assert_eq!(err.to_string(), "Session nope not found");
    }

    #[test]
    fn a_directory_target_uses_only_the_suggested_names_last_component() {
        let dir = std::env::temp_dir().join(format!("nanna-export-target-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let target = resolve_target(&dir, "../../escape.md");
        assert_eq!(target, dir.join("escape.md"));
        let file = dir.join("chosen.md");
        assert_eq!(
            resolve_target(&file, "ignored.md"),
            file,
            "a file path is used as given"
        );
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn the_cli_spells_formats_as_the_protocol_does() {
        assert_eq!(
            ExportFormat::from(ExportFormatArg::Markdown),
            ExportFormat::Markdown
        );
        assert_eq!(
            ExportFormat::from(ExportFormatArg::Json),
            ExportFormat::Json
        );
    }
}
