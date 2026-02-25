//! Vision backend wiring
//!
//! Connects vision tools to the Anthropic Claude vision API.

use super::vision::{AnalyzeImageTool, VisionFn};
use nanna_llm::{AnthropicMessage, AnthropicRequest, ContentBlock, LlmClient};
use std::sync::Arc;

/// Create an analyze_image tool wired to an LLM client with vision support.
///
/// The LLM client should support Anthropic's vision API (claude-3-* models).
pub fn create_vision_tool(llm: Arc<LlmClient>, model: String) -> AnalyzeImageTool {
    let vision_fn: VisionFn = Arc::new(move |image: String, prompt: String, media_type: String| {
        let llm = llm.clone();
        let model = model.clone();

        Box::pin(async move {
            // Determine if image is URL or base64
            let content = if image.starts_with("http://") || image.starts_with("https://") {
                // URL-based image
                vec![
                    ContentBlock::Image {
                        source: nanna_llm::ImageSource::Url { url: image },
                    },
                    ContentBlock::Text { text: prompt },
                ]
            } else {
                // Base64-encoded image
                vec![
                    ContentBlock::Image {
                        source: nanna_llm::ImageSource::Base64 {
                            media_type,
                            data: image,
                        },
                    },
                    ContentBlock::Text { text: prompt },
                ]
            };

            let request = AnthropicRequest {
                model,
                messages: vec![AnthropicMessage::user(content)],
                max_tokens: 4096,
                temperature: Some(0.3),
                system: Some("You are a helpful vision assistant. Analyze the image and respond to the user's prompt accurately and concisely.".to_string()),
                tools: None,
                stream: None,
                thinking: None,
                cache_control: None,
            };

            let response = llm
                .complete_anthropic(&request)
                .await
                .map_err(|e| e.to_string())?;

            // Extract text from response
            let text: String = response
                .content
                .iter()
                .filter_map(|block| {
                    if let ContentBlock::Text { text } = block {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");

            Ok(text)
        })
    });

    AnalyzeImageTool::new().with_vision_fn(vision_fn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vision_tool_creation() {
        // Just verify the types work
        let llm = Arc::new(LlmClient::anthropic("test-key"));
        let tool = create_vision_tool(llm, "claude-sonnet-4-20250514".to_string());
        let def = tool.definition();
        assert_eq!(def.name, "analyze_image");
    }
}
