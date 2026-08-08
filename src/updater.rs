//! Nanna auto-update mechanism
//!
//! Checks for updates on launch and provides silent install capability.

use std::sync::Arc;
use tauri_plugin_updater::{UpdaterExt, UpdaterBuilder};

/// Result of an update check
pub struct UpdateResult {
    /// Whether a newer version is available
    pub available: bool,
    /// Version string if update is available
    pub current_version: String,
    /// Version string of the latest release
    pub latest_version: Option<String>,
}

/// Check for updates and return the result
pub async fn check_for_updates() -> UpdateResult {
    let updater = UpdaterBuilder::new()
        .build()
        .expect("Failed to create updater");

    match updater.check().await {
        Ok(update) => UpdateResult {
            available: update.is_available(),
            current_version: String::from(env!("CARGO_PKG_VERSION")),
            latest_version: update.latest_version(),
        },
        Err(e) => {
            eprintln!("Update check failed: {}", e);
            UpdateResult {
                available: false,
                current_version: String::from(env!("CARGO_PKG_VERSION")),
                latest_version: None,
            }
        }
    }
}

/// Install the update silently (for non-headless mode)
pub async fn install_update() -> Result<(), Box<dyn std::error::Error>> {
    let updater = UpdaterBuilder::new()
        .build()
        .expect("Failed to create updater");

    updater
        .install()
        .await?;

    Ok(())
}

/// Get the current version string
pub fn current_version() -> String {
    String::from(env!("CARGO_PKG_VERSION"))
}
