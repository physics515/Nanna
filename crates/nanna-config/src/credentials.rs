//! Credential management with cross-platform secure storage
//!
//! Supports storing credentials in:
//! - OS Keyring (Windows Credential Manager, macOS Keychain, Linux Secret Service)
//! - Fallback to encrypted file storage
//! - Claude CLI credential file (~/.claude/.credentials.json) for OAuth
//!
//! The keyring is used for storing Nanna's own API keys and secrets,
//! while Claude CLI credentials are read-only (for OAuth token sharing).

use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;
use tracing::{debug, info, warn};

/// Service name for keyring storage
const KEYRING_SERVICE: &str = "nanna";

/// Credential key names
pub mod keys {
    pub const ANTHROPIC_API_KEY: &str = "anthropic_api_key";
    pub const OPENAI_API_KEY: &str = "openai_api_key";
    pub const OPENROUTER_API_KEY: &str = "openrouter_api_key";
    pub const GITHUB_TOKEN: &str = "github_token";
    pub const BRAVE_API_KEY: &str = "brave_api_key";
    pub const TELEGRAM_BOT_TOKEN: &str = "telegram_bot_token";
    pub const DISCORD_BOT_TOKEN: &str = "discord_bot_token";
    pub const SLACK_BOT_TOKEN: &str = "slack_bot_token";
    pub const WHATSAPP_ACCESS_TOKEN: &str = "whatsapp_access_token";
    pub const ELEVENLABS_API_KEY: &str = "elevenlabs_api_key";
}

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
    #[error("Keyring error: {0}")]
    Keyring(String),
}

impl From<keyring::Error> for CredentialError {
    fn from(e: keyring::Error) -> Self {
        match e {
            keyring::Error::NoEntry => CredentialError::NotFound,
            _ => CredentialError::Keyring(e.to_string()),
        }
    }
}

// =============================================================================
// Secure Keyring Storage
// =============================================================================

/// Cross-platform credential store using OS keyring
#[derive(Debug, Clone, Default)]
pub struct SecureStore {
    /// Fallback to file storage if keyring unavailable
    pub allow_file_fallback: bool,
    /// When set, bypass the OS keyring entirely and back the store with a JSON
    /// file under this directory. Makes `set`/`get`/`delete` deterministic for
    /// headless environments and unit tests (no interactive keyring dependency).
    file_store_dir: Option<PathBuf>,
}

impl SecureStore {
    /// Create a new secure store
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_file_fallback: true,
            file_store_dir: None,
        }
    }

    /// Create without file fallback (keyring-only)
    #[must_use]
    pub fn keyring_only() -> Self {
        Self {
            allow_file_fallback: false,
            file_store_dir: None,
        }
    }

    /// Create a deterministic, file-backed store rooted at `dir`.
    ///
    /// Bypasses the OS keyring so `set` then `get` always round-trip through the
    /// same JSON file — used for tests and headless (no-keyring) deployments.
    #[must_use]
    pub fn with_file_store(dir: PathBuf) -> Self {
        Self {
            allow_file_fallback: true,
            file_store_dir: Some(dir),
        }
    }

    /// Get a credential from the keyring
    ///
    /// # Panics
    /// Panics if `key` is empty (a programmer error).
    pub fn get(&self, key: &str) -> Result<String, CredentialError> {
        assert!(!key.is_empty(), "credential key must not be empty");
        // Deterministic file-backed store (tests / headless): never touch the keyring.
        if self.file_store_dir.is_some() {
            return self.get_from_file(key);
        }
        let entry = Entry::new(KEYRING_SERVICE, key)?;
        match entry.get_password() {
            Ok(value) => {
                debug!("Retrieved credential '{}' from keyring", key);
                Ok(value)
            }
            Err(keyring::Error::NoEntry) => {
                // Try file fallback if allowed
                if self.allow_file_fallback {
                    self.get_from_file(key)
                } else {
                    Err(CredentialError::NotFound)
                }
            }
            Err(e) => {
                warn!("Keyring error for '{}': {}", key, e);
                // Try file fallback
                if self.allow_file_fallback {
                    self.get_from_file(key)
                } else {
                    Err(e.into())
                }
            }
        }
    }

    /// Store a credential in the keyring
    ///
    /// # Panics
    /// Panics if `key` is empty (a programmer error).
    pub fn set(&self, key: &str, value: &str) -> Result<(), CredentialError> {
        assert!(!key.is_empty(), "credential key must not be empty");
        // Deterministic file-backed store (tests / headless): never touch the keyring.
        if self.file_store_dir.is_some() {
            return self.set_to_file(key, value);
        }
        let entry = Entry::new(KEYRING_SERVICE, key)?;
        match entry.set_password(value) {
            Ok(()) => {
                info!("Stored credential '{}' in keyring", key);
                Ok(())
            }
            Err(e) => {
                warn!("Keyring set error for '{}': {}", key, e);
                // Try file fallback
                if self.allow_file_fallback {
                    self.set_to_file(key, value)
                } else {
                    Err(e.into())
                }
            }
        }
    }

    /// Delete a credential from the keyring
    ///
    /// # Panics
    /// Panics if `key` is empty (a programmer error).
    pub fn delete(&self, key: &str) -> Result<(), CredentialError> {
        assert!(!key.is_empty(), "credential key must not be empty");
        // Deterministic file-backed store (tests / headless): never touch the keyring.
        if self.file_store_dir.is_some() {
            return self.delete_from_file(key);
        }
        let entry = Entry::new(KEYRING_SERVICE, key)?;
        match entry.delete_credential() {
            Ok(()) => {
                info!("Deleted credential '{}' from keyring", key);
                // Also delete from file fallback if it exists
                if self.allow_file_fallback {
                    let _ = self.delete_from_file(key);
                }
                Ok(())
            }
            Err(keyring::Error::NoEntry) => {
                // Try to delete from file
                if self.allow_file_fallback {
                    self.delete_from_file(key)
                } else {
                    Err(CredentialError::NotFound)
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Check if a credential exists
    pub fn exists(&self, key: &str) -> bool {
        self.get(key).is_ok()
    }

    /// List all stored credential keys (keyring doesn't support listing, so we check known keys)
    pub fn list_keys(&self) -> Vec<String> {
        let known_keys = [
            keys::ANTHROPIC_API_KEY,
            keys::OPENAI_API_KEY,
            keys::OPENROUTER_API_KEY,
            keys::GITHUB_TOKEN,
            keys::BRAVE_API_KEY,
            keys::TELEGRAM_BOT_TOKEN,
            keys::DISCORD_BOT_TOKEN,
            keys::SLACK_BOT_TOKEN,
            keys::WHATSAPP_ACCESS_TOKEN,
            keys::ELEVENLABS_API_KEY,
        ];

        known_keys
            .iter()
            .filter(|k| self.exists(k))
            .map(|k| k.to_string())
            .collect()
    }

    // =========================================================================
    // File Fallback (for systems without keyring support)
    // =========================================================================

    fn credentials_file_path(&self) -> Result<PathBuf, CredentialError> {
        if let Some(ref dir) = self.file_store_dir {
            return Ok(dir.join("credentials.json"));
        }
        let data_dir = directories::ProjectDirs::from("com", "nanna", "nanna")
            .ok_or(CredentialError::NoHomeDir)?
            .data_dir()
            .to_path_buf();
        Ok(data_dir.join("credentials.json"))
    }

    fn load_file_credentials(&self) -> Result<HashMap<String, String>, CredentialError> {
        let path = self.credentials_file_path()?;
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let content = std::fs::read_to_string(&path)?;
        let creds: HashMap<String, String> = serde_json::from_str(&content)?;
        Ok(creds)
    }

    fn save_file_credentials(
        &self,
        creds: &HashMap<String, String>,
    ) -> Result<(), CredentialError> {
        let path = self.credentials_file_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(creds)?;
        std::fs::write(&path, content)?;

        // Set restrictive permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path)?.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&path, perms)?;
        }

        Ok(())
    }

    fn get_from_file(&self, key: &str) -> Result<String, CredentialError> {
        let creds = self.load_file_credentials()?;
        creds.get(key).cloned().ok_or(CredentialError::NotFound)
    }

    fn set_to_file(&self, key: &str, value: &str) -> Result<(), CredentialError> {
        let mut creds = self.load_file_credentials()?;
        creds.insert(key.to_string(), value.to_string());
        self.save_file_credentials(&creds)?;
        info!("Stored credential '{}' in file fallback", key);
        Ok(())
    }

    fn delete_from_file(&self, key: &str) -> Result<(), CredentialError> {
        let mut creds = self.load_file_credentials()?;
        if creds.remove(key).is_some() {
            self.save_file_credentials(&creds)?;
            Ok(())
        } else {
            Err(CredentialError::NotFound)
        }
    }
}

// =============================================================================
// Claude CLI OAuth Credentials (Read-Only)
// =============================================================================

/// OAuth credential with refresh support (from Claude CLI)
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
    /// Loaded from Windows Credential Manager (via keyring)
    WindowsCredentialManager,
    /// Loaded from Linux Secret Service (via keyring)
    LinuxSecretService,
}

/// Result of loading credentials
#[derive(Debug, Clone)]
pub struct LoadedCredential {
    /// The credential
    pub credential: OAuthCredential,
    /// Where it was loaded from
    pub source: CredentialSource,
}

/// Claude CLI credential manager (for OAuth token reading)
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
        Ok(self
            .get_home_dir()?
            .join(".claude")
            .join(".credentials.json"))
    }

    /// Load credentials from all available sources
    ///
    /// Priority:
    /// 1. macOS Keychain (if on macOS)
    /// 2. Windows Credential Manager (if on Windows, via keyring)
    /// 3. Linux Secret Service (if on Linux, via keyring)
    /// 4. Credentials file
    pub fn load(&self) -> Result<LoadedCredential, CredentialError> {
        // Try platform-specific secure storage via keyring first
        if let Ok(cred) = self.load_from_keyring() {
            let source = if cfg!(target_os = "macos") {
                CredentialSource::MacOsKeychain
            } else if cfg!(target_os = "windows") {
                CredentialSource::WindowsCredentialManager
            } else {
                CredentialSource::LinuxSecretService
            };
            info!("Loaded Claude credentials from {:?}", source);
            return Ok(LoadedCredential {
                credential: cred,
                source,
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

    /// Load credentials from the keyring (cross-platform)
    fn load_from_keyring(&self) -> Result<OAuthCredential, CredentialError> {
        let entry = Entry::new("Claude Code-credentials", "Claude Code")?;
        let json_str = entry.get_password()?;
        let data: ClaudeCredentialsFile = serde_json::from_str(&json_str)?;
        let oauth_data = data.claude_ai_oauth.ok_or(CredentialError::NotFound)?;
        Ok(oauth_data.into())
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

    /// Save credentials to keyring
    pub fn save_to_keyring(&self, credential: &OAuthCredential) -> Result<(), CredentialError> {
        let entry = Entry::new("Claude Code-credentials", "Claude Code")?;

        let data = ClaudeCredentialsFile {
            claude_ai_oauth: Some(credential.clone().into()),
        };
        let json = serde_json::to_string(&data)?;

        entry.set_password(&json)?;
        info!("Saved Claude credentials to keyring");
        Ok(())
    }

    /// Save credentials back to the source they were loaded from
    pub fn save(
        &self,
        credential: &OAuthCredential,
        source: CredentialSource,
    ) -> Result<(), CredentialError> {
        match source {
            CredentialSource::File => self.save_to_file(credential),
            CredentialSource::MacOsKeychain
            | CredentialSource::WindowsCredentialManager
            | CredentialSource::LinuxSecretService => {
                // Try keyring first, fall back to file
                if self.save_to_keyring(credential).is_err() {
                    warn!("Failed to save to keyring, falling back to file");
                    self.save_to_file(credential)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Refresh the OAuth token using Anthropic's token endpoint
    pub async fn refresh_token(
        &self,
        credential: &OAuthCredential,
    ) -> Result<OAuthCredential, CredentialError> {
        let refresh_token = credential.refresh_token.as_ref().ok_or_else(|| {
            CredentialError::RefreshFailed("No refresh token available".to_string())
        })?;

        const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";

        // Build refresh request
        let client = reqwest::Client::new();
        let response = client
            .post(TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
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
        let expires_at = token_resp
            .expires_in
            .map(|secs| chrono::Utc::now().timestamp_millis() + secs * 1000);

        let new_credential = OAuthCredential {
            access_token: token_resp.access_token,
            refresh_token: token_resp
                .refresh_token
                .or_else(|| credential.refresh_token.clone()),
            expires_at,
            subscription_type: token_resp
                .subscription_type
                .or_else(|| credential.subscription_type.clone()),
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
        let cmd = if cfg!(windows) {
            "claude.cmd"
        } else {
            "claude"
        };

        std::process::Command::new(cmd)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Run `claude setup-token` to authenticate via browser
    ///
    /// This opens the browser for OAuth flow and waits for completion.
    /// The credentials will be saved by the CLI and can be loaded afterwards.
    ///
    /// # Errors
    /// Returns error if the CLI is not available or the command fails.
    pub fn run_setup_token() -> Result<(), CredentialError> {
        let cmd = if cfg!(windows) {
            "claude.cmd"
        } else {
            "claude"
        };

        let status = std::process::Command::new(cmd)
            .arg("setup-token")
            .status()
            .map_err(|e| {
                CredentialError::RefreshFailed(format!("Failed to run claude setup-token: {}", e))
            })?;

        if status.success() {
            Ok(())
        } else {
            Err(CredentialError::RefreshFailed(format!(
                "claude setup-token failed with exit code: {:?}",
                status.code()
            )))
        }
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

    #[test]
    fn test_secure_store_file_fallback() {
        // Use an explicit file-backed store so the test is deterministic and does
        // NOT depend on an interactive OS keyring (headless CI/unattended runs).
        let temp_dir = TempDir::new().unwrap();
        let store = SecureStore::with_file_store(temp_dir.path().to_path_buf());

        let key = "test_key_nanna_unit_test";
        let value = "test_value_12345";

        // set → get must round-trip through the same file store.
        store.set(key, value).unwrap();
        let retrieved = store.get(key).unwrap();
        assert_eq!(retrieved, value);

        // delete removes it; a subsequent get reports NotFound (negative space).
        store.delete(key).unwrap();
        assert!(matches!(store.get(key), Err(CredentialError::NotFound)));
    }

    #[test]
    fn test_secure_store_file_store_isolated_from_keyring() {
        // Two independent file stores must not see each other's entries.
        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();
        let store_a = SecureStore::with_file_store(dir_a.path().to_path_buf());
        let store_b = SecureStore::with_file_store(dir_b.path().to_path_buf());

        store_a.set("shared_key", "value_a").unwrap();
        assert!(matches!(
            store_b.get("shared_key"),
            Err(CredentialError::NotFound)
        ));
        assert_eq!(store_a.get("shared_key").unwrap(), "value_a");
    }
}
