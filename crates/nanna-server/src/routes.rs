//! HTTP route definitions

use crate::state::AppState;
use crate::webhooks;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Create the main router
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Health check
        .route("/health", get(health_check))
        // API routes
        .route("/api/v1/chat", post(chat))
        .route("/api/v1/sessions", post(create_session))
        .route("/api/v1/sessions/{session_id}", get(get_session))
        .route("/api/v1/sessions/{session_id}/messages", post(send_message))
        .route("/api/v1/sessions/{session_id}/messages", get(get_messages))
        // Webhook routes
        .route("/webhooks/telegram", post(webhooks::telegram::handle))
        .route("/webhooks/discord", post(webhooks::discord::handle))
        .route("/webhooks/slack", post(webhooks::slack::handle))
        .route("/webhooks/generic", post(webhooks::generic::handle))
        .with_state(state)
}

/// Health check response
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    gpu: bool,
}

async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        gpu: state.bot.has_gpu(),
    })
}

/// Chat request
#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub session_id: Option<String>,
    pub system_prompt: Option<String>,
}

/// Chat response
#[derive(Serialize)]
pub struct ChatResponse {
    pub session_id: String,
    pub message: String,
}

/// Simple chat endpoint - creates session if needed
async fn chat(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, String)> {
    let session_id = req.session_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    
    let response = state
        .process_message(&session_id, &req.message, req.system_prompt.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    
    Ok(Json(ChatResponse {
        session_id,
        message: response,
    }))
}

/// Create session request
#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub channel: Option<String>,
    pub user_id: Option<String>,
    pub system_prompt: Option<String>,
}

/// Session response
#[derive(Serialize)]
pub struct SessionResponse {
    pub session_id: String,
    pub channel: String,
    pub user_id: Option<String>,
    pub created_at: String,
}

async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<Json<SessionResponse>, (StatusCode, String)> {
    let session_id = Uuid::new_v4().to_string();
    let channel = req.channel.unwrap_or_else(|| "api".to_string());
    
    // Create in storage
    let session = state
        .storage
        .sessions()
        .create(&session_id, &channel, req.user_id.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Pre-create agent with system prompt if provided
    if req.system_prompt.is_some() {
        let _ = state.get_or_create_agent(&session_id, req.system_prompt.as_deref()).await;
    }
    
    Ok(Json(SessionResponse {
        session_id: session.session_id,
        channel: session.channel,
        user_id: session.user_id,
        created_at: session.created_at,
    }))
}

async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionResponse>, (StatusCode, String)> {
    let session = state
        .storage
        .sessions()
        .get(&session_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    
    Ok(Json(SessionResponse {
        session_id: session.session_id,
        channel: session.channel,
        user_id: session.user_id,
        created_at: session.created_at,
    }))
}

/// Send message request
#[derive(Deserialize)]
pub struct SendMessageRequest {
    pub content: String,
    pub system_prompt: Option<String>,
}

/// Message response
#[derive(Serialize)]
pub struct MessageResponse {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub created_at: String,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
}

async fn send_message(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<MessageResponse>, (StatusCode, String)> {
    let response = state
        .process_message(&session_id, &req.content, req.system_prompt.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    
    // Get the last message from storage for accurate info
    let messages = state
        .storage
        .messages()
        .get_by_session(&session_id, 1)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    
    if let Some(msg) = messages.last() {
        Ok(Json(MessageResponse {
            id: msg.id,
            role: msg.role.clone(),
            content: msg.content.clone(),
            created_at: msg.created_at.clone(),
            tokens_in: msg.tokens_in,
            tokens_out: msg.tokens_out,
        }))
    } else {
        Ok(Json(MessageResponse {
            id: 0,
            role: "assistant".to_string(),
            content: response,
            created_at: chrono::Utc::now().to_rfc3339(),
            tokens_in: None,
            tokens_out: None,
        }))
    }
}

async fn get_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<MessageResponse>>, (StatusCode, String)> {
    let messages = state
        .storage
        .messages()
        .get_by_session(&session_id, 100)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    
    let responses: Vec<MessageResponse> = messages
        .into_iter()
        .map(|msg| MessageResponse {
            id: msg.id,
            role: msg.role,
            content: msg.content,
            created_at: msg.created_at,
            tokens_in: msg.tokens_in,
            tokens_out: msg.tokens_out,
        })
        .collect();
    
    Ok(Json(responses))
}
