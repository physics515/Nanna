#![warn(clippy::all, clippy::restriction)]
#![deny(clippy::pedantic, clippy::nursery)]

//! LLM API client for Nanna
//!
//! Supports Anthropic Claude with proper tool calling and streaming.

use async_stream::stream;
use futures::Stream;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LlmError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error: {status} - {message}")]
    Api { status: u16, message: String },
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Stream error: {0}")]
    Stream(String),
    #[error("Missing API key for provider: {0}")]
    MissingApiKey(String),
}

/// LLM Provider
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenAI,
    OpenRouter,
}

/// Message role
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Chat message (simple text format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

/// Anthropic message content block
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, #[serde(skip_serializing_if = "Option::is_none")] is_error: Option<bool> },
}

/// Anthropic message format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

impl AnthropicMessage {
    pub fn user(content: Vec<ContentBlock>) -> Self {
        Self { role: "user".to_string(), content }
    }

    pub fn assistant(content: Vec<ContentBlock>) -> Self {
        Self { role: "assistant".to_string(), content }
    }

    pub fn user_text(text: impl Into<String>) -> Self {
        Self::user(vec![ContentBlock::Text { text: text.into() }])
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self::assistant(vec![ContentBlock::Text { text: text.into() }])
    }
}

/// Tool definition for Anthropic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Completion request with tools
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
}

impl Default for CompletionRequest {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".to_string(),
            messages: Vec::new(),
            max_tokens: Some(4096),
            temperature: Some(0.7),
            stream: false,
        }
    }
}

/// Full Anthropic request with tools
#[derive(Debug, Clone, Serialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

/// Anthropic API response
#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: String,
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// LLM client
#[derive(Clone)]
pub struct LlmClient {
    http: Client,
    provider: Provider,
    api_key: String,
    base_url: String,
}

impl LlmClient {
    /// Build HTTP client with sensible timeouts
    fn build_http_client() -> Client {
        Client::builder()
            .timeout(std::time::Duration::from_secs(120))  // 2 min total timeout
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new())
    }

    /// Create a new Anthropic client
    pub fn anthropic(api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::Anthropic,
            api_key: api_key.into(),
            base_url: "https://api.anthropic.com".to_string(),
        }
    }

    /// Create a new OpenAI client
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::OpenAI,
            api_key: api_key.into(),
            base_url: "https://api.openai.com".to_string(),
        }
    }

    /// Create a new OpenRouter client
    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self {
            http: Self::build_http_client(),
            provider: Provider::OpenRouter,
            api_key: api_key.into(),
            base_url: "https://openrouter.ai/api".to_string(),
        }
    }

    /// Validate the API key by making a lightweight request
    pub async fn validate(&self) -> Result<(), LlmError> {
        match self.provider {
            Provider::Anthropic => {
                // Use count_tokens endpoint for validation - lightweight
                let response = self
                    .http
                    .post(format!("{}/v1/messages/count_tokens", self.base_url))
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&serde_json::json!({
                        "model": "claude-sonnet-4-20250514",
                        "messages": [{"role": "user", "content": "hi"}]
                    }))
                    .send()
                    .await?;

                if response.status().as_u16() == 401 {
                    return Err(LlmError::Api {
                        status: 401,
                        message: "Invalid API key".to_string(),
                    });
                }
                Ok(())
            }
            _ => Ok(()), // Skip validation for other providers for now
        }
    }

    /// Send a completion request (simple, no tools)
    pub async fn complete(&self, request: &CompletionRequest) -> Result<String, LlmError> {
        match self.provider {
            Provider::Anthropic => self.complete_anthropic_simple(request).await,
            Provider::OpenAI | Provider::OpenRouter => self.complete_openai(request).await,
        }
    }

    /// Send a full Anthropic request with tools
    pub async fn complete_anthropic(&self, request: &AnthropicRequest) -> Result<AnthropicResponse, LlmError> {
        let response = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, message });
        }

        let result: AnthropicResponse = response.json().await?;
        Ok(result)
    }

    async fn complete_anthropic_simple(&self, request: &CompletionRequest) -> Result<String, LlmError> {
        // Extract system message
        let system_msg = request
            .messages
            .iter()
            .find(|m| m.role == Role::System)
            .map(|m| m.content.clone());

        // Convert to Anthropic format
        let messages: Vec<AnthropicMessage> = request
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .map(|m| AnthropicMessage {
                role: match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System => "user", // shouldn't happen
                }.to_string(),
                content: vec![ContentBlock::Text { text: m.content.clone() }],
            })
            .collect();

        let anthropic_request = AnthropicRequest {
            model: request.model.clone(),
            messages,
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: request.temperature,
            system: system_msg,
            tools: None,
            stream: None,
        };

        let result = self.complete_anthropic(&anthropic_request).await?;
        
        // Extract text from response
        let text = result
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(text)
    }

    async fn complete_openai(&self, request: &CompletionRequest) -> Result<String, LlmError> {
        #[derive(Serialize)]
        struct OpenAIRequest<'a> {
            model: &'a str,
            messages: &'a [Message],
            #[serde(skip_serializing_if = "Option::is_none")]
            max_tokens: Option<u32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            temperature: Option<f32>,
        }

        #[derive(Deserialize)]
        struct OpenAIResponse {
            choices: Vec<Choice>,
        }

        #[derive(Deserialize)]
        struct Choice {
            message: Message,
        }

        let body = OpenAIRequest {
            model: &request.model,
            messages: &request.messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
        };

        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, message });
        }

        let result: OpenAIResponse = response.json().await?;
        Ok(result
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default())
    }
}

/// Helper trait for building requests fluently
pub trait RequestBuilder {
    fn with_model(self, model: impl Into<String>) -> Self;
    fn with_message(self, message: Message) -> Self;
    fn with_max_tokens(self, max_tokens: u32) -> Self;
    fn with_temperature(self, temperature: f32) -> Self;
}

impl RequestBuilder for CompletionRequest {
    fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    fn with_message(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }
}

// ============================================================================
// Embeddings Support
// ============================================================================

/// Embedding client for generating vector embeddings
#[derive(Clone)]
pub struct EmbeddingClient {
    http: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl EmbeddingClient {
    /// Create OpenAI embedding client
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| Client::new()),
            api_key: api_key.into(),
            base_url: "https://api.openai.com".to_string(),
            model: "text-embedding-3-small".to_string(),
        }
    }

    /// Create client with custom model
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Get embeddings for a batch of texts
    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, LlmError> {
        #[derive(Serialize)]
        struct EmbedRequest<'a> {
            model: &'a str,
            input: &'a [&'a str],
        }

        #[derive(Deserialize)]
        struct EmbedResponse {
            data: Vec<EmbedData>,
        }

        #[derive(Deserialize)]
        struct EmbedData {
            embedding: Vec<f32>,
        }

        let response = self
            .http
            .post(format!("{}/v1/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&EmbedRequest {
                model: &self.model,
                input: texts,
            })
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, message });
        }

        let result: EmbedResponse = response.json().await?;
        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }

    /// Get embedding for a single text
    pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>, LlmError> {
        let mut results = self.embed(&[text]).await?;
        results.pop().ok_or_else(|| LlmError::Api {
            status: 500,
            message: "No embedding returned".to_string(),
        })
    }
}

// ============================================================================
// Streaming Support
// ============================================================================

/// A streaming event from the Anthropic API
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Start of a new message
    MessageStart { id: String, model: String },
    /// Start of a content block
    ContentBlockStart { index: usize, content_type: String },
    /// Text delta within a content block
    TextDelta { index: usize, text: String },
    /// Tool use delta (JSON fragment)
    ToolUseDelta { index: usize, partial_json: String },
    /// Content block finished
    ContentBlockStop { index: usize },
    /// Message finished
    MessageStop { stop_reason: String },
    /// Usage statistics
    MessageDelta { 
        stop_reason: Option<String>,
        output_tokens: u32,
    },
    /// Ping (keepalive)
    Ping,
    /// Error event
    Error { message: String },
}

/// Anthropic SSE event types
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicSSE {
    MessageStart { message: MessageStartData },
    ContentBlockStart { index: usize, content_block: ContentBlockData },
    ContentBlockDelta { index: usize, delta: DeltaData },
    ContentBlockStop { index: usize },
    MessageDelta { delta: MessageDeltaData, usage: Option<UsageDelta> },
    MessageStop,
    Ping,
    Error { error: ErrorData },
}

#[derive(Debug, Deserialize)]
struct MessageStartData {
    id: String,
    model: String,
}

#[derive(Debug, Deserialize)]
struct ContentBlockData {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DeltaData {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Deserialize)]
struct MessageDeltaData {
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageDelta {
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ErrorData {
    message: String,
}

impl LlmClient {
    /// Stream a completion request, yielding events as they arrive
    pub fn stream_anthropic(
        &self,
        request: &AnthropicRequest,
    ) -> impl Stream<Item = Result<StreamEvent, LlmError>> + '_ {
        let mut request = request.clone();
        request.stream = Some(true);

        let http = self.http.clone();
        let api_key = self.api_key.clone();
        let base_url = self.base_url.clone();

        stream! {
            let response = match http
                .post(format!("{}/v1/messages", base_url))
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield Err(LlmError::Http(e));
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let message = response.text().await.unwrap_or_default();
                yield Err(LlmError::Api { status, message });
                return;
            }

            // Read full response as text (simpler approach for now)
            // TODO: True SSE streaming with bytes_stream + tokio-stream
            let text = match response.text().await {
                Ok(t) => t,
                Err(e) => {
                    yield Err(LlmError::Http(e));
                    return;
                }
            };

            // Parse SSE events from the text
            let mut buffer = text;
            while let Some(event) = extract_sse_event(&mut buffer) {
                if let Some(stream_event) = parse_sse_event(&event) {
                    yield Ok(stream_event);
                }
            }
        }
    }
}

/// Extract a complete SSE event from the buffer
fn extract_sse_event(buffer: &mut String) -> Option<String> {
    // SSE events are separated by double newlines
    if let Some(pos) = buffer.find("\n\n") {
        let event = buffer[..pos].to_string();
        *buffer = buffer[pos + 2..].to_string();
        Some(event)
    } else {
        None
    }
}

/// Parse an SSE event string into a StreamEvent
fn parse_sse_event(event: &str) -> Option<StreamEvent> {
    let mut event_type = None;
    let mut data = None;

    for line in event.lines() {
        if let Some(rest) = line.strip_prefix("event: ") {
            event_type = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data: ") {
            data = Some(rest.trim().to_string());
        }
    }

    let data = data?;
    
    // Parse the JSON data based on event type
    match serde_json::from_str::<AnthropicSSE>(&data) {
        Ok(sse) => match sse {
            AnthropicSSE::MessageStart { message } => Some(StreamEvent::MessageStart {
                id: message.id,
                model: message.model,
            }),
            AnthropicSSE::ContentBlockStart { index, content_block } => {
                Some(StreamEvent::ContentBlockStart {
                    index,
                    content_type: content_block.block_type,
                })
            }
            AnthropicSSE::ContentBlockDelta { index, delta } => match delta {
                DeltaData::TextDelta { text } => Some(StreamEvent::TextDelta { index, text }),
                DeltaData::InputJsonDelta { partial_json } => {
                    Some(StreamEvent::ToolUseDelta { index, partial_json })
                }
            },
            AnthropicSSE::ContentBlockStop { index } => {
                Some(StreamEvent::ContentBlockStop { index })
            }
            AnthropicSSE::MessageDelta { delta, usage } => Some(StreamEvent::MessageDelta {
                stop_reason: delta.stop_reason,
                output_tokens: usage.map(|u| u.output_tokens).unwrap_or(0),
            }),
            AnthropicSSE::MessageStop => Some(StreamEvent::MessageStop {
                stop_reason: "end_turn".to_string(),
            }),
            AnthropicSSE::Ping => Some(StreamEvent::Ping),
            AnthropicSSE::Error { error } => Some(StreamEvent::Error {
                message: error.message,
            }),
        },
        Err(_) => None, // Skip malformed events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_construction() {
        let msg = Message::user("Hello");
        assert!(matches!(msg.role, Role::User));
        assert_eq!(msg.content, "Hello");
    }

    #[test]
    fn test_request_builder() {
        let request = CompletionRequest::default()
            .with_model("gpt-4")
            .with_message(Message::user("Hi"))
            .with_max_tokens(1000)
            .with_temperature(0.5);

        assert_eq!(request.model, "gpt-4");
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.max_tokens, Some(1000));
        assert_eq!(request.temperature, Some(0.5));
    }
}
