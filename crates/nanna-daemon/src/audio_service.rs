//! The `audio.tts` and `audio.transcribe` services.
//!
//! The bundled `text_to_speech` and `transcribe` skills declare these and
//! nothing registered them, so both were withheld at every boot (found by
//! `tests/skill_services_are_registered.rs`). The `OpenAI` clients behind them —
//! `OpenAiTts`, `OpenAiWhisper` — have been complete and unreachable.
//!
//! **Both are registered only when an `OpenAI` key is configured.** Whisper and
//! the TTS endpoint are `OpenAI`'s, not the chat router's, so there is no model
//! list to fall through; without a key the services do not exist and the two
//! skills stay withheld, with the boot warning naming what is missing.
//!
//! **`audio.tts` writes a file and returns its path.** The skill's own text
//! promised only a byte count, which meant the daemon spent an API call to
//! produce audio and then dropped it — audio nobody can play is not a
//! capability. The file lands under `{data_dir}/audio/`, and the skill now
//! reports where.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use nanna_scripting::ServiceFn;
use nanna_tools::{TranscribeFn, TtsFn, create_transcribe_tool_fn, create_tts_fn};
use serde_json::{Value, json};
use tracing::{info, warn};

/// Ceiling on one `audio.tts` request, in characters.
///
/// `OpenAI`'s speech endpoint rejects input over 4096 characters, so this is the
/// provider's limit restated where the caller can be told about it, rather than
/// a number chosen here. Refusing locally costs nothing; sending spends a
/// request to be told the same thing less clearly.
pub const TTS_INPUT_CHARS_MAX: usize = 4096;

/// Ceiling on an audio file sent to `audio.transcribe`, in bytes.
///
/// `OpenAI`'s transcription endpoint caps uploads at 25 MB. Same reasoning as
/// above: the bound is the provider's, and it is checked from `metadata()`
/// before the bytes are buffered so an oversized file is refused rather than
/// read and then refused.
pub const TRANSCRIBE_BYTES_MAX: u64 = 25 * 1024 * 1024;

/// Subdirectory of the data dir that generated speech is written to.
const AUDIO_DIR_NAME: &str = "audio";

/// Extension for generated speech. `OpenAI`'s speech endpoint returns MP3 unless
/// another format is requested, and nothing here requests one.
const TTS_FILE_EXTENSION: &str = "mp3";

/// Why this text cannot be spoken, or `None` if it can.
///
/// Pure so the ceiling is testable without a key or a socket — and the ceiling
/// counts **characters, not bytes**, because the provider's limit is in
/// characters and a byte count would refuse a multi-byte script at a quarter of
/// the real limit.
fn tts_input_refusal(text: &str) -> Option<String> {
    if text.is_empty() {
        return Some("audio.tts requires a non-empty `text`".to_string());
    }
    let char_count = text.chars().count();
    if char_count > TTS_INPUT_CHARS_MAX {
        return Some(format!(
            "text is {char_count} characters; the speech endpoint rejects \
             anything over {TTS_INPUT_CHARS_MAX}. Split it and call again"
        ));
    }
    None
}

/// Where one generated clip should be written.
///
/// Pure, so the naming rule is testable without an API key: the timestamp makes
/// concurrent calls land on different files, which matters because two skills
/// can be speaking at once and a fixed name would have them overwrite each
/// other silently.
fn tts_output_path(audio_dir: &std::path::Path, generated_at_nanos: u128) -> PathBuf {
    audio_dir.join(format!("tts-{generated_at_nanos}.{TTS_FILE_EXTENSION}"))
}

/// Build `audio.tts` and `audio.transcribe`, or an empty map without a key.
#[allow(
    clippy::implicit_hasher,
    reason = "must match the concrete map the daemon builds, not a generic one"
)]
pub fn build_audio_services(
    openai_api_key: Option<&str>,
    data_dir: &std::path::Path,
) -> HashMap<String, ServiceFn> {
    let Some(key) = openai_api_key.map(str::trim).filter(|k| !k.is_empty()) else {
        info!(
            "No OpenAI key; audio.tts and audio.transcribe stay unregistered and the \
             text_to_speech / transcribe skills stay withheld. Set [llm] openai_api_key \
             (or OPENAI_API_KEY) to enable them."
        );
        return HashMap::new();
    };

    let audio_dir = data_dir.join(AUDIO_DIR_NAME);
    info!(audio_dir = ?audio_dir, "Registering audio.tts and audio.transcribe");

    let speak: TtsFn = create_tts_fn(key, Some("nova"));
    let transcribe: TranscribeFn = create_transcribe_tool_fn(key);

    let mut services: HashMap<String, ServiceFn> = HashMap::new();
    services.insert("audio.tts".to_string(), tts_service(speak, audio_dir));
    services.insert(
        "audio.transcribe".to_string(),
        transcribe_service(transcribe),
    );

    debug_assert_eq!(
        services.len(),
        2,
        "both audio services must register together"
    );
    services
}

/// `audio.tts`: speak the text, write the clip, report where it went.
fn tts_service(speak: TtsFn, audio_dir: PathBuf) -> ServiceFn {
    Arc::new(move |params: Value| {
        let speak = speak.clone();
        let audio_dir = audio_dir.clone();
        Box::pin(async move {
            let text = params
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default()
                .to_string();
            if let Some(refusal) = tts_input_refusal(&text) {
                return Err(refusal);
            }
            let voice = params
                .get("voice")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string);

            let audio = speak(text, voice.clone()).await?;
            debug_assert!(!audio.is_empty(), "the speech endpoint returned no bytes");

            tokio::fs::create_dir_all(&audio_dir)
                .await
                .map_err(|e| format!("cannot create {}: {e}", audio_dir.display()))?;
            let generated_at_nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            let path = tts_output_path(&audio_dir, generated_at_nanos);
            let size = audio.len();
            tokio::fs::write(&path, audio)
                .await
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;

            info!(path = ?path, size, "Wrote generated speech");
            Ok(json!({
                "path": path.to_string_lossy(),
                "size": size,
                "voice": voice.unwrap_or_else(|| "nova".to_string()),
            }))
        })
    })
}

/// `audio.transcribe`: read the file, send it, return the text.
fn transcribe_service(transcribe: TranscribeFn) -> ServiceFn {
    Arc::new(move |params: Value| {
        let transcribe = transcribe.clone();
        Box::pin(async move {
            let path = params
                .get("path")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default()
                .to_string();
            if path.is_empty() {
                return Err("audio.transcribe requires a `path`".to_string());
            }
            let language = params
                .get("language")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string);

            let size_bytes = tokio::fs::metadata(&path)
                .await
                .map_err(|e| format!("Cannot stat {path}: {e}"))?
                .len();
            if size_bytes > TRANSCRIBE_BYTES_MAX {
                return Err(format!(
                    "Audio too large: {size_bytes} bytes (max {TRANSCRIBE_BYTES_MAX}); \
                         the transcription endpoint rejects anything larger"
                ));
            }
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|e| format!("Cannot read {path}: {e}"))?;
            if bytes.is_empty() {
                return Err(format!("{path} is empty; there is nothing to transcribe"));
            }

            match transcribe(bytes, language).await {
                Ok(text) => Ok(json!({ "text": text })),
                Err(e) => {
                    warn!(%path, error = %e, "Transcription failed");
                    Err(format!("transcription failed: {e}"))
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_means_no_services() {
        let dir = tempfile::tempdir().unwrap();
        assert!(build_audio_services(None, dir.path()).is_empty());
    }

    #[test]
    fn a_blank_key_is_not_a_key() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            build_audio_services(Some("   "), dir.path()).is_empty(),
            "whitespace was taken for a configured key, so both skills would \
             load and fail at call time",
        );
    }

    #[test]
    fn a_key_registers_both_services_together() {
        let dir = tempfile::tempdir().unwrap();
        let services = build_audio_services(Some("sk-test"), dir.path());
        assert!(services.contains_key("audio.tts"));
        assert!(services.contains_key("audio.transcribe"));
    }

    #[test]
    fn generated_clips_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let first = tts_output_path(dir.path(), 1);
        let second = tts_output_path(dir.path(), 2);
        assert_ne!(
            first, second,
            "two clips generated in one run would overwrite each other",
        );
        assert!(first.starts_with(dir.path()));
        assert_eq!(
            first.extension().and_then(|e| e.to_str()),
            Some(TTS_FILE_EXTENSION)
        );
    }

    #[tokio::test]
    async fn tts_refuses_empty_text_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let services = build_audio_services(Some("sk-test"), dir.path());
        let tts = services.get("audio.tts").expect("registered");

        let err = tts(json!({ "text": "   " })).await.unwrap_err();
        assert!(err.contains("text"), "the refusal must name it: {err}");
    }

    #[tokio::test]
    async fn tts_refuses_oversized_text_before_spending_a_request() {
        let dir = tempfile::tempdir().unwrap();
        let services = build_audio_services(Some("sk-test"), dir.path());
        let tts = services.get("audio.tts").expect("registered");

        let too_long = "a".repeat(TTS_INPUT_CHARS_MAX + 1);
        let err = tts(json!({ "text": too_long })).await.unwrap_err();
        assert!(
            err.contains(&TTS_INPUT_CHARS_MAX.to_string()),
            "the refusal must name the ceiling: {err}"
        );
        // Nothing reached the network, so nothing reached disk either.
        assert!(!dir.path().join(AUDIO_DIR_NAME).exists());
    }

    /// The ceiling counts characters, not bytes — otherwise a multi-byte script
    /// would be refused at a quarter of the real limit. Asserted on the pure
    /// check so it needs no key and makes no request.
    #[test]
    fn the_tts_ceiling_counts_characters_not_bytes() {
        // 4-byte characters: under the character ceiling, far over it in bytes.
        let text = "𝄞".repeat(TTS_INPUT_CHARS_MAX - 1);
        assert!(
            text.len() > TTS_INPUT_CHARS_MAX,
            "fixture is not multi-byte"
        );
        assert_eq!(
            tts_input_refusal(&text),
            None,
            "a multi-byte text was refused on its byte length",
        );

        let one_over = "a".repeat(TTS_INPUT_CHARS_MAX + 1);
        assert!(tts_input_refusal(&one_over).is_some());
        assert_eq!(tts_input_refusal(&"a".repeat(TTS_INPUT_CHARS_MAX)), None);
        assert!(tts_input_refusal("").is_some());
    }

    #[tokio::test]
    async fn transcribe_refuses_a_missing_path_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let services = build_audio_services(Some("sk-test"), dir.path());
        let transcribe = services.get("audio.transcribe").expect("registered");

        let err = transcribe(json!({})).await.unwrap_err();
        assert!(err.contains("path"), "the refusal must name it: {err}");
    }

    #[tokio::test]
    async fn transcribe_refuses_an_empty_file_rather_than_sending_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("silence.mp3");
        std::fs::write(&path, b"").unwrap();
        let services = build_audio_services(Some("sk-test"), dir.path());
        let transcribe = services.get("audio.transcribe").expect("registered");

        let err = transcribe(json!({ "path": path.to_string_lossy() }))
            .await
            .unwrap_err();
        assert!(err.contains("empty"), "unhelpful refusal: {err}");
    }

    #[tokio::test]
    async fn transcribe_refuses_an_oversized_file_before_buffering_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.mp3");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(TRANSCRIBE_BYTES_MAX + 1).unwrap();
        drop(file);

        let services = build_audio_services(Some("sk-test"), dir.path());
        let transcribe = services.get("audio.transcribe").expect("registered");
        let err = transcribe(json!({ "path": path.to_string_lossy() }))
            .await
            .unwrap_err();
        assert!(err.contains("too large"), "unhelpful refusal: {err}");
    }
}
