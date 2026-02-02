//! Claude CLI credential management
//!
//! Supports reading/writing credentials from:
//! - File: `~/.claude/.credentials.json`
//! - macOS Keychain: `security` command
//! - Windows Credential Manager: `cmdkey` / registry (future)
//!
//! Also handles OAuth token refresh using Anthropic's token endpoint.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use tracing::{debug, info, warn};

/// Credential errors
#[derive(Error, Debug)]
pub enum CredentialError {
    #[error("Credentials not found")]
    NotFound,
    #[error("Credentials expired")]
    Expired,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Token refresh failed: {0}")]
    RefreshFailed(String),
    #[error("Home directory not found")]
    NoHomeDir,
    #[error("Keychain error: {0}")]
    Keychain(String),
}

/// OAuth credential with refresh support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCredential {
    /// OAuth access token
    pub access_token: String,
    /// OAuth refresh token (for token renewal)
    pub refresh_token: Option<String>,
    /// Token expiration timestamp (milliseconds since epoch)
    pub expires_at: Option<i64>,
    /// Subscription type (e.g., "pro", "max", "free")
    pub subscription_type: Option<String>,
    /// Account ID
    pub account_id: Option<String>,
    /// Organization ID (for enterprise)
    pub organization_id: Option<String>,
}

impl OAuthCredential {
    /// Check if the token is expired (with 5-minute buffer)
    #[must_use]
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = chrono::Utc::now().timestamp_millis();
            // Add 5-minute buffer for safety
            expires_at < now + 5 * 60 * 1000
        } else {
            // No expiry info - assume valid
            false
        }
    }

    /// Check if the token can be refreshed
    #[must_use]
    pub fn can_refresh(&self) -> bool {
        self.refresh_token.is_some()
    }

    /// Get time until expiration in seconds (negative if expired)
    #[must_use]
    pub fn seconds_until_expiry(&self) -> Option<i64> {
        self.expires_at.map(|expires_at| {
            let now = chrono::Utc::now().timestamp_millis();
            (expires_at - now) / 1000
        })
    }
}

/// Claude CLI credentials file structure
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClaudeCredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<ClaudeOAuthData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClaudeOAuthData {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
    #[serde(rename = "accountId")]
    account_id: Option<String>,
    #[serde(rename = "organizationId")]
    organization_id: Option<String>,
}

impl From<ClaudeOAuthData> for OAuthCredential {
    fn from(data: ClaudeOAuthData) -> Self {
        Self {
            access_token: data.access_token,
            refresh_token: data.refresh_token,
            expires_at: data.expires_at,
            subscription_type: data.subscription_type,
            account_id: data.account_id,
            organization_id: data.organization_id,
        }
    }
}

impl From<OAuthCredential> for ClaudeOAuthData {
    fn from(cred: OAuthCredential) -> Self {
        Self {
            access_token: cred.access_token,
            refresh_token: cred.refresh_token,
            expires_at: cred.expires_at,
            subscription_type: cred.subscription_type,
            account_id: cred.account_id,
            organization_id: cred.organization_id,
        }
    }
}

/// Credential source (where credentials were loaded from)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    /// Loaded from ~/.claude/.credentials.json
    File,
    /// Loaded from macOS Keychain
    MacOsKeychain,
    /// Loaded from Windows Credential Manager
    WindowsCredentialManager,
}

/// Result of loading credentials
#[derive(Debug, Clone)]
pub struct LoadedCredential {
    /// The credential
    pub credential: OAuthCredential,
    /// Where it was loaded from
    pub source: CredentialSource,
}

/// Claude CLI credential manager
#[derive(Debug, Clone)]
pub struct ClaudeCredentialManager {
    /// Home directory override (for testing)
    home_dir: Option<PathBuf>,
}

impl Default for ClaudeCredentialManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeCredentialManager {
    /// Create a new credential manager
    #[must_use]
    pub fn new() -> Self {
        Self { home_dir: None }
    }

    /// Create with a custom home directory (for testing)
    #[must_use]
    pub fn with_home_dir(home_dir: PathBuf) -> Self {
        Self {
            home_dir: Some(home_dir),
        }
    }

    /// Get the home directory
    fn get_home_dir(&self) -> Result<PathBuf, CredentialError> {
        if let Some(ref home) = self.home_dir {
            return Ok(home.clone());
        }

        // Try USERPROFILE (Windows) first, then HOME (Unix)
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .map(PathBuf::from)
            .map_err(|_| CredentialError::NoHomeDir)
    }

    /// Get path to Claude credentials file
    fn credentials_path(&self) -> Result<PathBuf, CredentialError> {
        Ok(self.get_home_dir()?.join(".claude").join(".credentials.json"))
    }

    /// Load credentials from all available sources
    ///
    /// Priority:
    /// 1. macOS Keychain (if on macOS)
    /// 2. Windows Credential Manager (if on Windows)
    /// 3. Credentials file
    pub fn load(&self) -> Result<LoadedCredential, CredentialError> {
        // Try platform-specific secure storage first
        #[cfg(target_os = "macos")]
        if let Ok(cred) = self.load_from_macos_keychain() {
            info!("Loaded Claude credentials from macOS Keychain");
            return Ok(LoadedCredential {
                credential: cred,
                source: CredentialSource::MacOsKeychain,
            });
        }

        #[cfg(target_os = "windows")]
        if let Ok(cred) = self.load_from_windows_credential_manager() {
            info!("Loaded Claude credentials from Windows Credential Manager");
            return Ok(LoadedCredential {
                credential: cred,
                source: CredentialSource::WindowsCredentialManager,
            });
        }

        // Fall back to file
        let cred = self.load_from_file()?;
        info!("Loaded Claude credentials from file");
        Ok(LoadedCredential {
            credential: cred,
            source: CredentialSource::File,
        })
    }

    /// Load credentials from the file
    pub fn load_from_file(&self) -> Result<OAuthCredential, CredentialError> {
        let path = self.credentials_path()?;

        if !path.exists() {
            debug!("Claude credentials file not found at {:?}", path);
            return Err(CredentialError::NotFound);
        }

        let content = std::fs::read_to_string(&path)?;
        let creds: ClaudeCredentialsFile = serde_json::from_str(&content)?;

        let oauth_data = creds.claude_ai_oauth.ok_or(CredentialError::NotFound)?;

        Ok(oauth_data.into())
    }

    /// Load credentials from macOS Keychain
    #[cfg(target_os = "macos")]
    pub fn load_from_macos_keychain(&self) -> Result<OAuthCredential, CredentialError> {
        use std::process::Command;

        const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

        let output = Command::new("security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
            .output()
            .map_err(|e| CredentialError::Keychain(e.to_string()))?;

        if !output.status.success() {
            return Err(CredentialError::NotFound);
        }

        let json_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let data: ClaudeCredentialsFile =
            serde_json::from_str(&json_str).map_err(|e| CredentialError::Json(e))?;

        let oauth_data = data.claude_ai_oauth.ok_or(CredentialError::NotFound)?;

        Ok(oauth_data.into())
    }

    /// Stub for non-macOS platforms
    #[cfg(not(target_os = "macos"))]
    pub fn load_from_macos_keychain(&self) -> Result<OAuthCredential, CredentialError> {
        Err(CredentialError::NotFound)
    }

    /// Load credentials from Windows Credential Manager
    #[cfg(target_os = "windows")]
    pub fn load_from_windows_credential_manager(&self) -> Result<OAuthCredential, CredentialError> {
        // Windows stores credentials differently - Claude Code uses a custom location
        // For now, fall back to file-based credentials
        // TODO: Implement Windows credential store reading if Claude Code uses it
        Err(CredentialError::NotFound)
    }

    /// Stub for non-Windows platforms
    #[cfg(not(target_os = "windows"))]
    pub fn load_from_windows_credential_manager(&self) -> Result<OAuthCredential, CredentialError> {
        Err(CredentialError::NotFound)
    }

    /// Save credentials to file
    pub fn save_to_file(&self, credential: &OAuthCredential) -> Result<(), CredentialError> {
        let path = self.credentials_path()?;

        // Create directory if it doesn't exist
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Read existing file to preserve other fields
        let mut creds: ClaudeCredentialsFile = if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            serde_json::from_str(&content).unwrap_or(ClaudeCredentialsFile {
                claude_ai_oauth: None,
            })
        } else {
            ClaudeCredentialsFile {
                claude_ai_oauth: None,
            }
        };

        // Update OAuth data
        creds.claude_ai_oauth = Some(credential.clone().into());

        // Write back
        let content = serde_json::to_string_pretty(&creds)?;
        std::fs::write(&path, content)?;

        info!("Saved Claude credentials to {:?}", path);
        Ok(())
    }

    /// Save credentials to macOS Keychain
    #[cfg(target_os = "macos")]
    pub fn save_to_macos_keychain(&self, credential: &OAuthCredential) -> Result<(), CredentialError> {
        use std::process::Command;

        const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
        const KEYCHAIN_ACCOUNT: &str = "Claude Code";

        // Build the JSON payload
        let data = ClaudeCredentialsFile {
            claude_ai_oauth: Some(credential.clone().into()),
        };
        let json = serde_json::to_string(&data)?;

        // Escape single quotes for shell
        let escaped_json = json.replace('\'', "'\"'\"'");

        // Update existing keychain entry
        let output = Command::new("security")
            .args([
                "add-generic-password",
                "-U", // Update if exists
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                KEYCHAIN_ACCOUNT,
                "-w",
                &escaped_json,
            ])
            .output()
            .map_err(|e| CredentialError::Keychain(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CredentialError::Keychain(stderr.to_string()));
        }

        info!("Saved Claude credentials to macOS Keychain");
        Ok(())
    }

    /// Stub for non-macOS platforms
    #[cfg(not(target_os = "macos"))]
    pub fn save_to_macos_keychain(&self, _credential: &OAuthCredential) -> Result<(), CredentialError> {
        Err(CredentialError::Keychain(
            "macOS Keychain not available".to_string(),
        ))
    }

    /// Save credentials back to the source they were loaded from
    pub fn save(&self, credential: &OAuthCredential, source: CredentialSource) -> Result<(), CredentialError> {
        match source {
            CredentialSource::File => self.save_to_file(credential),
            CredentialSource::MacOsKeychain => {
                // Try keychain first, fall back to file
                if self.save_to_macos_keychain(credential).is_err() {
                    warn!("Failed to save to Keychain, falling back to file");
                    self.save_to_file(credential)
                } else {
                    Ok(())
                }
            }
            CredentialSource::WindowsCredentialManager => {
                // Fall back to file for Windows
                self.save_to_file(credential)
            }
        }
    }

    /// Refresh the OAuth token using Anthropic's token endpoint
    pub async fn refresh_token(
        &self,
        credential: &OAuthCredential,
    ) -> Result<OAuthCredential, CredentialError> {
        let refresh_token = credential
            .refresh_token
            .as_ref()
            .ok_or_else(|| CredentialError::RefreshFailed("No refresh token available".to_string()))?;

        const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";

        // Build refresh request
        let client = reqwest::Client::new();
        let response = client
            .post(TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                // Note: client_id may be needed depending on Anthropic's OAuth implementation
            ])
            .send()
            .await
            .map_err(|e| CredentialError::RefreshFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(CredentialError::RefreshFailed(format!(
                "Token refresh failed with status {}: {}",
                status, body
            )));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            refresh_token: Option<String>,
            expires_in: Option<i64>,
            #[serde(rename = "subscriptionType")]
            subscription_type: Option<String>,
        }

        let token_resp: TokenResponse = response
            .json()
            .await
            .map_err(|e| CredentialError::RefreshFailed(e.to_string()))?;

        // Calculate new expiry
        let expires_at = token_resp.expires_in.map(|secs| {
            chrono::Utc::now().timestamp_millis() + secs * 1000
        });

        let new_credential = OAuthCredential {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token.or_else(|| credential.refresh_token.clone()),
            expires_at,
            subscription_type: token_resp.subscription_type.or_else(|| credential.subscription_type.clone()),
            account_id: credential.account_id.clone(),
            organization_id: credential.organization_id.clone(),
        };

        info!(
            "Token refreshed, expires in {:?} seconds",
            new_credential.seconds_until_expiry()
        );

        Ok(new_credential)
    }

    /// Load credentials, refreshing if expired
    pub async fn load_and_refresh(&self) -> Result<LoadedCredential, CredentialError> {
        let loaded = self.load()?;

        if loaded.credential.is_expired() {
            if loaded.credential.can_refresh() {
                info!("Token expired, attempting refresh...");
                let new_credential = self.refresh_token(&loaded.credential).await?;

                // Save the refreshed token
                if let Err(e) = self.save(&new_credential, loaded.source) {
                    warn!("Failed to save refreshed token: {}", e);
                }

                return Ok(LoadedCredential {
                    credential: new_credential,
                    source: loaded.source,
                });
            } else {
                return Err(CredentialError::Expired);
            }
        }

        Ok(loaded)
    }

    /// Check if Claude CLI is installed and available
    pub fn is_claude_cli_available() -> bool {
        let cmd = if cfg!(windows) { "claude.cmd" } else { "claude" };

        std::process::Command::new(cmd)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Run `claude setup-token` to authenticate
    ///
    /// This will open a browser for OAuth authentication.
    pub fn run_setup_token() -> Result<(), CredentialError> {
        let cmd = if cfg!(windows) { "claude.cmd" } else { "claude" };

        info!("Running `claude setup-token`...");

        let output = std::process::Command::new(cmd)
            .arg("setup-token")
            .output()
            .map_err(|e| {
                CredentialError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to run claude setup-token: {}", e),
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(CredentialError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("claude setup-token failed:\n{}\n{}", stdout, stderr),
            )));
        }

        info!("`claude setup-token` completed successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_credential_expiry() {
        let cred = OAuthCredential {
            access_token: "test".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: Some(chrono::Utc::now().timestamp_millis() + 3600 * 1000),
            subscription_type: None,
            account_id: None,
            organization_id: None,
        };

        assert!(!cred.is_expired());
        assert!(cred.can_refresh());
    }

    #[test]
    fn test_expired_credential() {
        let cred = OAuthCredential {
            access_token: "test".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: Some(chrono::Utc::now().timestamp_millis() - 1000),
            subscription_type: None,
            account_id: None,
            organization_id: None,
        };

        assert!(cred.is_expired());
    }

    #[test]
    fn test_save_and_load_file() {
        let temp_dir = TempDir::new().unwrap();
        let manager = ClaudeCredentialManager::with_home_dir(temp_dir.path().to_path_buf());

        let cred = OAuthCredential {
            access_token: "test_token".to_string(),
            refresh_token: Some("refresh_token".to_string()),
            expires_at: Some(1234567890000),
            subscription_type: Some("pro".to_string()),
            account_id: None,
            organization_id: None,
        };

        // Save
        manager.save_to_file(&cred).unwrap();

        // Load
        let loaded = manager.load_from_file().unwrap();

        assert_eq!(loaded.access_token, "test_token");
        assert_eq!(loaded.refresh_token, Some("refresh_token".to_string()));
        assert_eq!(loaded.expires_at, Some(1234567890000));
        assert_eq!(loaded.subscription_type, Some("pro".to_string()));
    }
}
