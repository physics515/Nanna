use tauri::State;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct UninstallService {
    pub app_dir: PathBuf,
}

impl UninstallService {
    pub fn new(app_dir: PathBuf) -> Self {
        Self { app_dir }
    }

    /// Get the list of all files and directories to uninstall
    pub fn get_uninstall_list(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();

        // Main application directory
        if self.app_dir.exists() {
            files.push(self.app_dir.clone());
        }

        // User data directory
        let user_data = self.app_dir.join("user-data");
        if user_data.exists() {
            files.push(user_data);
        }

        // Credentials file (if it exists and is not in user-data)
        let creds_file = self.app_dir.join("credentials.json");
        if creds_file.exists() {
            files.push(creds_file);
        }

        // MCP configuration
        let mcp_dir = self.app_dir.join("mcp");
        if mcp_dir.exists() {
            files.push(mcp_dir);
        }

        // Databases
        for db_path in &[
            "sqlite.db",
            "postgres.db",
            "redis.db",
            "mongo.db",
        ] {
            let db_file = self.app_dir.join(db_path);
            if db_file.exists() {
                files.push(db_file);
            }
        }

        // Cache directory
        let cache_dir = self.app_dir.join("cache");
        if cache_dir.exists() {
            files.push(cache_dir);
        }

        // Logs directory
        let logs_dir = self.app_dir.join("logs");
        if logs_dir.exists() {
            files.push(logs_dir);
        }

        // Temporary files
        let temp_dir = self.app_dir.join("temp");
        if temp_dir.exists() {
            files.push(temp_dir);
        }

        files
    }

    /// Uninstall the application and all its data
    pub async fn uninstall(&self) -> Result<(), String> {
        let files = self.get_uninstall_list();

        for path in &files {
            if path.exists() {
                match std::fs::remove_dir_all(path) {
                    Ok(_) => {}
                    Err(e) => {
                        return Err(format!("Failed to remove {}: {}", path.display(), e));
                    }
                }
            }
        }

        // Remove user data directory if it exists
        let user_data = self.app_dir.join("user-data");
        if user_data.exists() {
            match std::fs::remove_dir_all(&user_data) {
                Ok(_) => {}
                Err(e) => {
                    return Err(format!("Failed to remove user data: {}", e));
                }
            }
        }

        // Remove credentials file if it exists
        let creds_file = self.app_dir.join("credentials.json");
        if creds_file.exists() {
            match std::fs::remove_file(&creds_file) {
                Ok(_) => {}
                Err(e) => {
                    return Err(format!("Failed to remove credentials: {}", e));
                }
            }
        }

        // Remove MCP directory if it exists
        let mcp_dir = self.app_dir.join("mcp");
        if mcp_dir.exists() {
            match std::fs::remove_dir_all(&mcp_dir) {
                Ok(_) => {}
                Err(e) => {
                    return Err(format!("Failed to remove MCP directory: {}", e));
                }
            }
        }

        // Remove database files if they exist
        for db_path in &[
            "sqlite.db",
            "postgres.db",
            "redis.db",
            "mongo.db",
        ] {
            let db_file = self.app_dir.join(db_path);
            if db_file.exists() {
                match std::fs::remove_file(&db_file) {
                    Ok(_) => {}
                    Err(e) => {
                        return Err(format!("Failed to remove database {}: {}", db_path, e));
                    }
                }
            }
        }

        // Remove cache directory if it exists
        let cache_dir = self.app_dir.join("cache");
        if cache_dir.exists() {
            match std::fs::remove_dir_all(&cache_dir) {
                Ok(_) => {}
                Err(e) => {
                    return Err(format!("Failed to remove cache: {}", e));
                }
            }
        }

        // Remove logs directory if it exists
        let logs_dir = self.app_dir.join("logs");
        if logs_dir.exists() {
            match std::fs::remove_dir_all(&logs_dir) {
                Ok(_) => {}
                Err(e) => {
                    return Err(format!("Failed to remove logs: {}", e));
                }
            }
        }

        // Remove temporary files if they exist
        let temp_dir = self.app_dir.join("temp");
        if temp_dir.exists() {
            match std::fs::remove_dir_all(&temp_dir) {
                Ok(_) => {}
                Err(e) => {
                    return Err(format!("Failed to remove temp files: {}", e));
                }
            }
        }

        Ok(())
    }

    /// Check if the application is installed
    pub fn is_installed(&self) -> bool {
        self.app_dir.exists()
    }

    /// Get the list of items that will be removed
    pub async fn preview_uninstall(&self) -> Result<Vec<PathBuf>, String> {
        let files = self.get_uninstall_list();
        Ok(files)
    }
}
