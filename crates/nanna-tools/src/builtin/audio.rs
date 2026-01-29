//! Audio tools - TTS and transcription

use crate::{Tool, ToolDefinition, ToolError, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

/// Callback for text-to-speech
pub type TtsFn = Arc<
    dyn Fn(String, Option<String>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + Send>>
        + Send
        + Sync,
>;

/// Callback for speech-to-text transcription
pub type TranscribeFn = Arc<
    dyn Fn(Vec<u8>, Option<String>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Text-to-speech tool
pub struct TextToSpeechTool {
    tts_fn: Option<TtsFn>,
    output_dir: Option<String>,
}

impl TextToSpeechTool {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tts_fn: None,
            output_dir: None,
        }
    }

    /// Set the TTS callback function.
    #[must_use]
    pub fn with_tts_fn(mut self, f: TtsFn) -> Self {
        self.tts_fn = Some(f);
        self
    }

    /// Set the output directory for audio files.
    #[must_use]
    pub fn with_output_dir(mut self, dir: impl Into<String>) -> Self {
        self.output_dir = Some(dir.into());
        self
    }
}

impl Default for TextToSpeechTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TextToSpeechTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("text_to_speech", "Convert text to speech audio")
            .string_param("text", "Text to convert to speech", true)
            .string_param("voice", "Voice ID or name (optional)", false)
            .string_param("output_file", "Output file path (optional)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let text = params
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'text' parameter".to_string()))?;

        let voice = params.get("voice").and_then(|v| v.as_str()).map(String::from);

        let tts_fn = self.tts_fn.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("TTS not configured".to_string())
        })?;

        debug!("Generating speech for {} chars", text.len());

        let audio_data = tts_fn(text.to_string(), voice)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("TTS failed: {}", e)))?;

        // Save to file if output specified or default dir
        let output_path = params
            .get("output_file")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                self.output_dir.as_ref().map(|dir| {
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    format!("{}/tts_{}.mp3", dir, timestamp)
                })
            });

        if let Some(path) = &output_path {
            tokio::fs::write(path, &audio_data)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write audio: {}", e)))?;

            Ok(ToolResult::success(format!("Generated speech saved to: {}", path))
                .with_data(serde_json::json!({
                    "path": path,
                    "size_bytes": audio_data.len(),
                })))
        } else {
            // Return base64-encoded audio
            let base64 = base64_encode(&audio_data);
            Ok(ToolResult::success(format!("Generated {} bytes of audio (base64)", audio_data.len()))
                .with_data(serde_json::json!({
                    "audio_base64": base64,
                    "size_bytes": audio_data.len(),
                    "format": "mp3",
                })))
        }
    }
}

/// Speech-to-text transcription tool
pub struct TranscribeTool {
    transcribe_fn: Option<TranscribeFn>,
}

impl TranscribeTool {
    #[must_use]
    pub fn new() -> Self {
        Self { transcribe_fn: None }
    }

    /// Set the transcription callback function.
    #[must_use]
    pub fn with_transcribe_fn(mut self, f: TranscribeFn) -> Self {
        self.transcribe_fn = Some(f);
        self
    }
}

impl Default for TranscribeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TranscribeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new("transcribe", "Transcribe audio to text using Whisper")
            .string_param("audio", "Path to audio file or base64-encoded audio", true)
            .string_param("language", "Language code (optional, auto-detect if not specified)", false)
    }

    async fn execute(&self, params: HashMap<String, Value>) -> Result<ToolResult, ToolError> {
        let audio = params
            .get("audio")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'audio' parameter".to_string()))?;

        let language = params.get("language").and_then(|v| v.as_str()).map(String::from);

        let transcribe_fn = self.transcribe_fn.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed("Transcription not configured".to_string())
        })?;

        // Load audio data
        let audio_data = if audio.starts_with('/') || audio.contains(':') {
            // Treat as file path
            tokio::fs::read(audio)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read audio file: {}", e)))?
        } else {
            // Treat as base64
            base64_decode(audio)
                .map_err(|e| ToolError::ExecutionFailed(format!("Invalid base64 audio: {}", e)))?
        };

        debug!("Transcribing {} bytes of audio", audio_data.len());

        let transcript = transcribe_fn(audio_data, language)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Transcription failed: {}", e)))?;

        Ok(ToolResult::success(transcript))
    }
}

// Simple base64 encoding/decoding
fn base64_encode(data: &[u8]) -> String {
    base64_simd::STANDARD.encode_to_string(data)
}

fn base64_decode(data: &str) -> Result<Vec<u8>, String> {
    base64_simd::STANDARD
        .decode_to_vec(data.as_bytes())
        .map_err(|e| e.to_string())
}

/// OpenAI TTS client helper
pub struct OpenAiTts {
    api_key: String,
    model: String,
    voice: String,
}

impl OpenAiTts {
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "tts-1".to_string(),
            voice: "alloy".to_string(),
        }
    }

    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    #[must_use]
    pub fn with_voice(mut self, voice: impl Into<String>) -> Self {
        self.voice = voice.into();
        self
    }

    /// Generate speech from text.
    ///
    /// # Errors
    ///
    /// Returns an error if the API request fails.
    pub async fn speak(&self, text: &str, voice_override: Option<&str>) -> Result<Vec<u8>, String> {
        let voice = voice_override.unwrap_or(&self.voice);

        let client = reqwest::Client::new();
        let response = client
            .post("https://api.openai.com/v1/audio/speech")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "model": self.model,
                "input": text,
                "voice": voice,
            }))
            .send()
            .await
            .map_err(|e| format!("TTS request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("TTS API error {}: {}", status, body));
        }

        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("Failed to read audio: {}", e))
    }
}

/// OpenAI Whisper transcription client helper
pub struct OpenAiWhisper {
    api_key: String,
    model: String,
}

#[derive(Deserialize)]
struct WhisperResponse {
    text: String,
}

impl OpenAiWhisper {
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "whisper-1".to_string(),
        }
    }

    /// Transcribe audio to text.
    ///
    /// # Errors
    ///
    /// Returns an error if the API request fails.
    pub async fn transcribe(&self, audio: &[u8], language: Option<&str>) -> Result<String, String> {
        let client = reqwest::Client::new();

        // Build multipart form
        let file_part = reqwest::multipart::Part::bytes(audio.to_vec())
            .file_name("audio.mp3")
            .mime_str("audio/mpeg")
            .map_err(|e| format!("Failed to create form part: {}", e))?;

        let mut form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .part("file", file_part);

        if let Some(lang) = language {
            form = form.text("language", lang.to_string());
        }

        let response = client
            .post("https://api.openai.com/v1/audio/transcriptions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("Transcription request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Whisper API error {}: {}", status, body));
        }

        let result: WhisperResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        Ok(result.text)
    }
}
