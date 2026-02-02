//! OAuth auto-refresh support for LLM clients
//!
//! This module provides utilities for automatically refreshing OAuth tokens
//! when using Claude CLI credentials.
//!
//! Requires the `auto-refresh` feature.

#[cfg(feature = "auto-refresh")]
use nanna_config::{ClaudeCredentialManager, CredentialError, OAuthCredential};

#[cfg(feature = "auto-refresh")]
use tracing::{info, warn};

use crate::LlmClient;

/// OAuth client wrapper that can auto-refresh tokens
#[cfg(feature = "auto-refresh")]
pub struct OAuthClient {
    /// The current OAuth credential
    pub credential: OAuthCredential,
    /// Where the credential was loaded from
    pub source: nanna_config::CredentialSource,
    /// The credential manager for refresh operations
    manager: ClaudeCredentialManager,
}

#[cfg(feature = "auto-refresh")]
impl OAuthClient {
    /// Load credentials from Claude CLI and create an OAuth client
    ///
    /// This will automatically refresh the token if it's expired.
    pub async fn load() -> Result<Self, CredentialError> {
        let manager = ClaudeCredentialManager::new();
        let loaded = manager.load_and_refresh().await?;
        
        Ok(Self {
            credential: loaded.credential,
            source: loaded.source,
            manager,
        })
    }

    /// Load credentials without auto-refresh
    pub fn load_sync() -> Result<Self, CredentialError> {
        let manager = ClaudeCredentialManager::new();
        let loaded = manager.load()?;
        
        Ok(Self {
            credential: loaded.credential,
            source: loaded.source,
            manager,
        })
    }

    /// Check if the token is expired or expiring soon (within 5 minutes)
    #[must_use]
    pub fn needs_refresh(&self) -> bool {
        self.credential.is_expired()
    }

    /// Refresh the token if needed
    ///
    /// Returns true if the token was refreshed.
    pub async fn refresh_if_needed(&mut self) -> Result<bool, CredentialError> {
        if !self.needs_refresh() {
            return Ok(false);
        }

        if !self.credential.can_refresh() {
            return Err(CredentialError::RefreshFailed(
                "No refresh token available".to_string(),
            ));
        }

        info!("OAuth token expired, refreshing...");
        let new_cred = self.manager.refresh_token(&self.credential).await?;

        // Save the refreshed token back to the source
        if let Err(e) = self.manager.save(&new_cred, self.source) {
            warn!("Failed to save refreshed token: {}", e);
        }

        self.credential = new_cred;
        info!("OAuth token refreshed successfully");
        Ok(true)
    }

    /// Get an LlmClient using the current OAuth token
    ///
    /// Call `refresh_if_needed()` first if you want to ensure the token is fresh.
    #[must_use]
    pub fn llm_client(&self) -> LlmClient {
        LlmClient::anthropic_oauth(&self.credential.access_token)
    }

    /// Get the access token
    #[must_use]
    pub fn access_token(&self) -> &str {
        &self.credential.access_token
    }

    /// Get seconds until the token expires
    #[must_use]
    pub fn seconds_until_expiry(&self) -> Option<i64> {
        self.credential.seconds_until_expiry()
    }

    /// Get the subscription type (e.g., "pro", "max")
    #[must_use]
    pub fn subscription_type(&self) -> Option<&str> {
        self.credential.subscription_type.as_deref()
    }
}

/// Create an LlmClient with auto-refreshed OAuth token
///
/// This is a convenience function that:
/// 1. Loads credentials from Claude CLI
/// 2. Refreshes the token if expired
/// 3. Returns an LlmClient configured with the fresh token
///
/// # Errors
///
/// Returns an error if credentials cannot be loaded or refreshed.
#[cfg(feature = "auto-refresh")]
pub async fn create_oauth_client() -> Result<LlmClient, CredentialError> {
    let mut oauth = OAuthClient::load().await?;
    oauth.refresh_if_needed().await?;
    Ok(oauth.llm_client())
}

/// Create an LlmClient from Claude CLI credentials without network refresh
///
/// This loads credentials but doesn't attempt to refresh them.
/// Useful when you want to check if credentials exist before doing async work.
#[cfg(feature = "auto-refresh")]
pub fn create_oauth_client_sync() -> Result<LlmClient, CredentialError> {
    let oauth = OAuthClient::load_sync()?;
    
    if oauth.needs_refresh() && !oauth.credential.can_refresh() {
        return Err(CredentialError::Expired);
    }
    
    Ok(oauth.llm_client())
}

#[cfg(test)]
#[cfg(feature = "auto-refresh")]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_oauth_client_load() {
        // This test requires actual Claude CLI credentials
        // Skip if not available
        match OAuthClient::load_sync() {
            Ok(client) => {
                println!("Loaded OAuth client, subscription: {:?}", client.subscription_type());
                println!("Expires in: {:?} seconds", client.seconds_until_expiry());
            }
            Err(e) => {
                println!("Skipping test - no credentials: {}", e);
            }
        }
    }
}
