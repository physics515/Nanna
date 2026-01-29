//! WhatsApp channel implementation via Cloud API
//!
//! Uses Meta's official WhatsApp Business Cloud API.
//! See: https://developers.facebook.com/docs/whatsapp/cloud-api

use crate::{
    Channel, ChannelCapabilities, ChannelError, ChannelFeatures, MessageContent, OutgoingMessage,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;
use tracing::debug;

const WHATSAPP_API_BASE: &str = "https://graph.facebook.com/v21.0";

/// WhatsApp Cloud API client
#[derive(Clone)]
pub struct WhatsAppChannel {
    client: Client,
    access_token: String,
    phone_number_id: String,
    business_account_id: Option<String>,
}

impl WhatsAppChannel {
    /// Create a new WhatsApp channel.
    ///
    /// - `access_token`: Meta access token with whatsapp_business_messaging permission
    /// - `phone_number_id`: WhatsApp Business phone number ID
    pub fn new(access_token: impl Into<String>, phone_number_id: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            access_token: access_token.into(),
            phone_number_id: phone_number_id.into(),
            business_account_id: None,
        }
    }

    /// Set business account ID (for some operations).
    #[must_use]
    pub fn with_business_account(mut self, account_id: impl Into<String>) -> Self {
        self.business_account_id = Some(account_id.into());
        self
    }

    /// Make an API request.
    async fn api<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<impl Serialize>,
    ) -> Result<T, ChannelError> {
        let url = format!("{WHATSAPP_API_BASE}/{path}");

        let mut request = self
            .client
            .request(method, &url)
            .header("Authorization", format!("Bearer {}", self.access_token));

        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .json(&body);
        }

        let response = request
            .send()
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;

        let status = response.status();
        
        if !status.is_success() {
            let error: WhatsAppErrorResponse = response
                .json()
                .await
                .unwrap_or_else(|_| WhatsAppErrorResponse {
                    error: WhatsAppError {
                        message: format!("HTTP {status}"),
                        code: status.as_u16() as i32,
                        error_subcode: None,
                        fbtrace_id: None,
                    },
                });

            // Handle rate limiting
            if status.as_u16() == 429 || error.error.code == 80007 {
                return Err(ChannelError::RateLimited);
            }

            return Err(ChannelError::Send(format!(
                "WhatsApp API error {}: {}",
                error.error.code, error.error.message
            )));
        }

        response
            .json()
            .await
            .map_err(|e| ChannelError::Send(format!("Failed to parse response: {e}")))
    }

    // ========================================================================
    // Message Operations
    // ========================================================================

    /// Send a text message.
    pub async fn send_text(
        &self,
        to: &str,
        text: &str,
        preview_url: bool,
    ) -> Result<WhatsAppMessageResponse, ChannelError> {
        debug!(to, "Sending WhatsApp text message");

        let body = SendMessageRequest {
            messaging_product: "whatsapp".to_string(),
            recipient_type: "individual".to_string(),
            to: to.to_string(),
            message_type: "text".to_string(),
            text: Some(TextMessage {
                body: text.to_string(),
                preview_url,
            }),
            image: None,
            document: None,
            audio: None,
            video: None,
            sticker: None,
            location: None,
            template: None,
            interactive: None,
            reaction: None,
        };

        self.api(
            reqwest::Method::POST,
            &format!("{}/messages", self.phone_number_id),
            Some(&body),
        )
        .await
    }

    /// Send an image message.
    pub async fn send_image(
        &self,
        to: &str,
        image_url: &str,
        caption: Option<&str>,
    ) -> Result<WhatsAppMessageResponse, ChannelError> {
        debug!(to, "Sending WhatsApp image");

        let body = SendMessageRequest {
            messaging_product: "whatsapp".to_string(),
            recipient_type: "individual".to_string(),
            to: to.to_string(),
            message_type: "image".to_string(),
            text: None,
            image: Some(MediaMessage {
                link: Some(image_url.to_string()),
                id: None,
                caption: caption.map(String::from),
                filename: None,
            }),
            document: None,
            audio: None,
            video: None,
            sticker: None,
            location: None,
            template: None,
            interactive: None,
            reaction: None,
        };

        self.api(
            reqwest::Method::POST,
            &format!("{}/messages", self.phone_number_id),
            Some(&body),
        )
        .await
    }

    /// Send a document.
    pub async fn send_document(
        &self,
        to: &str,
        document_url: &str,
        filename: &str,
        caption: Option<&str>,
    ) -> Result<WhatsAppMessageResponse, ChannelError> {
        debug!(to, filename, "Sending WhatsApp document");

        let body = SendMessageRequest {
            messaging_product: "whatsapp".to_string(),
            recipient_type: "individual".to_string(),
            to: to.to_string(),
            message_type: "document".to_string(),
            text: None,
            image: None,
            document: Some(MediaMessage {
                link: Some(document_url.to_string()),
                id: None,
                caption: caption.map(String::from),
                filename: Some(filename.to_string()),
            }),
            audio: None,
            video: None,
            sticker: None,
            location: None,
            template: None,
            interactive: None,
            reaction: None,
        };

        self.api(
            reqwest::Method::POST,
            &format!("{}/messages", self.phone_number_id),
            Some(&body),
        )
        .await
    }

    /// Send a reaction.
    pub async fn send_reaction(
        &self,
        to: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<WhatsAppMessageResponse, ChannelError> {
        debug!(to, message_id, emoji, "Sending WhatsApp reaction");

        let body = SendMessageRequest {
            messaging_product: "whatsapp".to_string(),
            recipient_type: "individual".to_string(),
            to: to.to_string(),
            message_type: "reaction".to_string(),
            text: None,
            image: None,
            document: None,
            audio: None,
            video: None,
            sticker: None,
            location: None,
            template: None,
            interactive: None,
            reaction: Some(ReactionMessage {
                message_id: message_id.to_string(),
                emoji: emoji.to_string(),
            }),
        };

        self.api(
            reqwest::Method::POST,
            &format!("{}/messages", self.phone_number_id),
            Some(&body),
        )
        .await
    }

    /// Remove a reaction (send empty emoji).
    pub async fn remove_reaction(
        &self,
        to: &str,
        message_id: &str,
    ) -> Result<WhatsAppMessageResponse, ChannelError> {
        self.send_reaction(to, message_id, "").await
    }

    /// Send a location.
    pub async fn send_location(
        &self,
        to: &str,
        latitude: f64,
        longitude: f64,
        name: Option<&str>,
        address: Option<&str>,
    ) -> Result<WhatsAppMessageResponse, ChannelError> {
        debug!(to, latitude, longitude, "Sending WhatsApp location");

        let body = SendMessageRequest {
            messaging_product: "whatsapp".to_string(),
            recipient_type: "individual".to_string(),
            to: to.to_string(),
            message_type: "location".to_string(),
            text: None,
            image: None,
            document: None,
            audio: None,
            video: None,
            sticker: None,
            location: Some(LocationMessage {
                latitude,
                longitude,
                name: name.map(String::from),
                address: address.map(String::from),
            }),
            template: None,
            interactive: None,
            reaction: None,
        };

        self.api(
            reqwest::Method::POST,
            &format!("{}/messages", self.phone_number_id),
            Some(&body),
        )
        .await
    }

    /// Send a template message.
    pub async fn send_template(
        &self,
        to: &str,
        template_name: &str,
        language_code: &str,
        components: Option<Vec<TemplateComponent>>,
    ) -> Result<WhatsAppMessageResponse, ChannelError> {
        debug!(to, template_name, "Sending WhatsApp template");

        let body = SendMessageRequest {
            messaging_product: "whatsapp".to_string(),
            recipient_type: "individual".to_string(),
            to: to.to_string(),
            message_type: "template".to_string(),
            text: None,
            image: None,
            document: None,
            audio: None,
            video: None,
            sticker: None,
            location: None,
            template: Some(TemplateMessage {
                name: template_name.to_string(),
                language: TemplateLanguage {
                    code: language_code.to_string(),
                },
                components,
            }),
            interactive: None,
            reaction: None,
        };

        self.api(
            reqwest::Method::POST,
            &format!("{}/messages", self.phone_number_id),
            Some(&body),
        )
        .await
    }

    /// Mark a message as read.
    pub async fn mark_read(&self, message_id: &str) -> Result<(), ChannelError> {
        debug!(message_id, "Marking WhatsApp message as read");

        #[derive(Serialize)]
        struct MarkReadRequest {
            messaging_product: &'static str,
            status: &'static str,
            message_id: String,
        }

        let body = MarkReadRequest {
            messaging_product: "whatsapp",
            status: "read",
            message_id: message_id.to_string(),
        };

        let _: serde_json::Value = self
            .api(
                reqwest::Method::POST,
                &format!("{}/messages", self.phone_number_id),
                Some(&body),
            )
            .await?;

        Ok(())
    }

    // ========================================================================
    // Media Operations
    // ========================================================================

    /// Upload media and get media ID.
    pub async fn upload_media(
        &self,
        data: &[u8],
        mime_type: &str,
    ) -> Result<String, ChannelError> {
        let url = format!("{WHATSAPP_API_BASE}/{}/media", self.phone_number_id);

        let form = reqwest::multipart::Form::new()
            .text("messaging_product", "whatsapp")
            .part(
                "file",
                reqwest::multipart::Part::bytes(data.to_vec()).mime_str(mime_type)
                    .map_err(|e| ChannelError::Send(e.to_string()))?,
            );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .multipart(form)
            .send()
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;

        #[derive(Deserialize)]
        struct MediaResponse {
            id: String,
        }

        let result: MediaResponse = response
            .json()
            .await
            .map_err(|e| ChannelError::Send(e.to_string()))?;

        Ok(result.id)
    }

    /// Get media URL from media ID.
    pub async fn get_media_url(&self, media_id: &str) -> Result<String, ChannelError> {
        #[derive(Deserialize)]
        struct MediaUrlResponse {
            url: String,
        }

        let result: MediaUrlResponse = self
            .api(reqwest::Method::GET, media_id, None::<()>)
            .await?;

        Ok(result.url)
    }
}

// ============================================================================
// Channel Trait Implementation
// ============================================================================

#[async_trait]
impl Channel for WhatsAppChannel {
    fn provider(&self) -> &str {
        "whatsapp"
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            features: ChannelFeatures::REACTIONS
                | ChannelFeatures::IMAGES
                | ChannelFeatures::AUDIO
                | ChannelFeatures::DOCUMENTS,
            max_message_length: Some(4096),
        }
    }

    async fn send(&self, message: OutgoingMessage) -> Result<String, ChannelError> {
        let to = &message.channel.id;

        let result = match message.content {
            MessageContent::Text { text } => {
                self.send_text(to, &text, true).await?
            }
            MessageContent::Image { url, caption } => {
                self.send_image(to, &url, caption.as_deref()).await?
            }
            MessageContent::Document { url, filename } => {
                self.send_document(to, &url, &filename, None).await?
            }
            MessageContent::Location { latitude, longitude } => {
                self.send_location(to, latitude, longitude, None, None).await?
            }
            _ => {
                return Err(ChannelError::Send(
                    "Unsupported message type for WhatsApp".to_string(),
                ))
            }
        };

        result
            .messages
            .first()
            .map(|m| m.id.clone())
            .ok_or_else(|| ChannelError::Send("No message ID in response".to_string()))
    }

    async fn react(&self, message_id: &str, emoji: &str) -> Result<(), ChannelError> {
        // message_id format: "recipient:wamid"
        let (to, wamid) = parse_message_id(message_id)?;
        self.send_reaction(&to, &wamid, emoji).await?;
        Ok(())
    }

    async fn unreact(&self, message_id: &str, _emoji: &str) -> Result<(), ChannelError> {
        let (to, wamid) = parse_message_id(message_id)?;
        self.remove_reaction(&to, &wamid).await?;
        Ok(())
    }
}

/// Parse message ID in format "recipient:wamid"
fn parse_message_id(id: &str) -> Result<(String, String), ChannelError> {
    let parts: Vec<&str> = id.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(ChannelError::Send(format!(
            "Invalid message ID format: {id} (expected recipient:wamid)"
        )));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Serialize)]
struct SendMessageRequest {
    messaging_product: String,
    recipient_type: String,
    to: String,
    #[serde(rename = "type")]
    message_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<TextMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<MediaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    document: Option<MediaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<MediaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    video: Option<MediaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sticker: Option<MediaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<LocationMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    template: Option<TemplateMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interactive: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reaction: Option<ReactionMessage>,
}

#[derive(Serialize)]
struct TextMessage {
    body: String,
    preview_url: bool,
}

#[derive(Serialize)]
struct MediaMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filename: Option<String>,
}

#[derive(Serialize)]
struct LocationMessage {
    latitude: f64,
    longitude: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<String>,
}

#[derive(Serialize)]
struct ReactionMessage {
    message_id: String,
    emoji: String,
}

#[derive(Serialize)]
struct TemplateMessage {
    name: String,
    language: TemplateLanguage,
    #[serde(skip_serializing_if = "Option::is_none")]
    components: Option<Vec<TemplateComponent>>,
}

#[derive(Serialize)]
struct TemplateLanguage {
    code: String,
}

/// Template component for template messages.
#[derive(Clone, Serialize)]
pub struct TemplateComponent {
    #[serde(rename = "type")]
    pub component_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Vec<TemplateParameter>>,
}

/// Template parameter.
#[derive(Clone, Serialize)]
pub struct TemplateParameter {
    #[serde(rename = "type")]
    pub param_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Message send response.
#[derive(Deserialize)]
pub struct WhatsAppMessageResponse {
    pub messaging_product: String,
    pub contacts: Vec<WhatsAppContact>,
    pub messages: Vec<WhatsAppMessageId>,
}

#[derive(Deserialize)]
pub struct WhatsAppContact {
    pub input: String,
    pub wa_id: String,
}

#[derive(Deserialize)]
pub struct WhatsAppMessageId {
    pub id: String,
}

/// Error response.
#[derive(Deserialize)]
struct WhatsAppErrorResponse {
    error: WhatsAppError,
}

#[derive(Deserialize)]
struct WhatsAppError {
    message: String,
    code: i32,
    error_subcode: Option<i32>,
    fbtrace_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message_id() {
        let (to, wamid) = parse_message_id("+1234567890:wamid.abc123").unwrap();
        assert_eq!(to, "+1234567890");
        assert_eq!(wamid, "wamid.abc123");
    }

    #[test]
    fn test_parse_invalid_message_id() {
        assert!(parse_message_id("invalid").is_err());
    }
}
