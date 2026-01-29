#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! HTTP server for Nanna
//!
//! Provides REST API and webhook endpoints for channel integrations.

mod routes;
mod state;
mod webhooks;

pub use routes::create_router;
pub use state::{AppState, AppStateBuilder};

use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub webhook_secret: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 3000,
            webhook_secret: None,
        }
    }
}

/// Start the HTTP server.
///
/// # Errors
///
/// Returns an error if the server fails to bind or start.
pub async fn start_server(config: ServerConfig, state: AppState) -> anyhow::Result<()> {
    let app = create_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    
    info!("🚀 Server listening on {}", addr);
    
    axum::serve(listener, app).await?;
    
    Ok(())
}
