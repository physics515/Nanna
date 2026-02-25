//! Audio backend wiring
//!
//! Connects TTS and transcription tools to OpenAI APIs.

use super::audio::{OpenAiTts, OpenAiWhisper, TextToSpeechTool, TranscribeFn, TranscribeTool, TtsFn};
use std::sync::Arc;

/// Create a TTS tool wired to OpenAI's TTS API.
///
/// Voices: alloy, echo, fable, onyx, nova, shimmer
pub fn create_tts_tool(api_key: impl Into<String>, default_voice: Option<&str>) -> TextToSpeechTool {
    let tts_client = Arc::new(
        OpenAiTts::new(api_key)
            .with_voice(default_voice.unwrap_or("nova"))
    );

    let tts_fn: TtsFn = Arc::new(move |text: String, voice: Option<String>| {
        let client = tts_client.clone();
        Box::pin(async move {
            client.speak(&text, voice.as_deref()).await
        })
    });

    TextToSpeechTool::new().with_tts_fn(tts_fn)
}

/// Create a TTS tool with a custom output directory for saving audio files.
pub fn create_tts_tool_with_dir(
    api_key: impl Into<String>,
    default_voice: Option<&str>,
    output_dir: impl Into<String>,
) -> TextToSpeechTool {
    create_tts_tool(api_key, default_voice).with_output_dir(output_dir)
}

/// Create a transcription tool wired to OpenAI's Whisper API.
pub fn create_transcribe_tool(api_key: impl Into<String>) -> TranscribeTool {
    let whisper_client = Arc::new(OpenAiWhisper::new(api_key));

    let transcribe_fn: TranscribeFn = Arc::new(move |audio: Vec<u8>, language: Option<String>| {
        let client = whisper_client.clone();
        Box::pin(async move {
            client.transcribe(&audio, language.as_deref()).await
        })
    });

    TranscribeTool::new().with_transcribe_fn(transcribe_fn)
}

/// Create both TTS and transcription tools wired to OpenAI.
///
/// Returns (tts_tool, transcribe_tool)
pub fn create_audio_tools(
    api_key: impl Into<String>,
    default_voice: Option<&str>,
) -> (TextToSpeechTool, TranscribeTool) {
    let key = api_key.into();
    let tts = create_tts_tool(key.clone(), default_voice);
    let transcribe = create_transcribe_tool(key);
    (tts, transcribe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tool;

    #[test]
    fn test_tts_tool_creation() {
        let tool = create_tts_tool("test-key", Some("nova"));
        let def = tool.definition();
        assert_eq!(def.name, "text_to_speech");
    }

    #[test]
    fn test_transcribe_tool_creation() {
        let tool = create_transcribe_tool("test-key");
        let def = tool.definition();
        assert_eq!(def.name, "transcribe");
    }

    #[test]
    fn test_audio_tools_creation() {
        let (tts, transcribe) = create_audio_tools("test-key", None);
        assert_eq!(tts.definition().name, "text_to_speech");
        assert_eq!(transcribe.definition().name, "transcribe");
    }
}
