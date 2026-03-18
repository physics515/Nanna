//! Platform service integration
//!
//! Handles running the daemon as a system service on different platforms:
//! - Windows: Windows Service
//! - macOS: launchd
//! - Linux: systemd

use std::path::PathBuf;

/// Service status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Starting,
    Stopping,
    Unknown,
}

/// Service configuration
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            name: "nanna-daemon".to_string(),
            display_name: "Nanna Daemon".to_string(),
            description: "Nanna AI assistant background service".to_string(),
            executable: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nanna-daemon")),
            arguments: vec!["run".to_string()],
            working_directory: None,
        }
    }
}

/// Platform-specific service operations
pub struct ServiceManager {
    config: ServiceConfig,
}

impl ServiceManager {
    pub fn new(config: ServiceConfig) -> Self {
        Self { config }
    }
    
    /// Install the service
    pub fn install(&self) -> Result<(), String> {
        #[cfg(windows)]
        return self.install_windows();
        
        #[cfg(target_os = "macos")]
        return self.install_macos();
        
        #[cfg(target_os = "linux")]
        return self.install_linux();
        
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        Err("Service installation not supported on this platform".to_string())
    }
    
    /// Uninstall the service
    pub fn uninstall(&self) -> Result<(), String> {
        #[cfg(windows)]
        return self.uninstall_windows();
        
        #[cfg(target_os = "macos")]
        return self.uninstall_macos();
        
        #[cfg(target_os = "linux")]
        return self.uninstall_linux();
        
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        Err("Service uninstallation not supported on this platform".to_string())
    }
    
    /// Start the service
    pub fn start(&self) -> Result<(), String> {
        #[cfg(windows)]
        return self.start_windows();
        
        #[cfg(target_os = "macos")]
        return self.start_macos();
        
        #[cfg(target_os = "linux")]
        return self.start_linux();
        
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        Err("Service start not supported on this platform".to_string())
    }
    
    /// Stop the service
    pub fn stop(&self) -> Result<(), String> {
        #[cfg(windows)]
        return self.stop_windows();
        
        #[cfg(target_os = "macos")]
        return self.stop_macos();
        
        #[cfg(target_os = "linux")]
        return self.stop_linux();
        
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        Err("Service stop not supported on this platform".to_string())
    }
    
    /// Get service status
    pub fn status(&self) -> ServiceStatus {
        #[cfg(windows)]
        return self.status_windows();
        
        #[cfg(target_os = "macos")]
        return self.status_macos();
        
        #[cfg(target_os = "linux")]
        return self.status_linux();
        
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        ServiceStatus::Unknown
    }
    
    // =========================================================================
    // Windows implementation
    // =========================================================================
    
    #[cfg(windows)]
    fn install_windows(&self) -> Result<(), String> {
        // TODO: Use windows-service crate
        Err(format!(
            "Windows service installation not yet implemented for '{}'",
            self.config.name
        ))
    }
    
    #[cfg(windows)]
    fn uninstall_windows(&self) -> Result<(), String> {
        Err(format!(
            "Windows service uninstallation not yet implemented for '{}'",
            self.config.name
        ))
    }
    
    #[cfg(windows)]
    fn start_windows(&self) -> Result<(), String> {
        Err(format!(
            "Windows service start not yet implemented for '{}'",
            self.config.name
        ))
    }
    
    #[cfg(windows)]
    fn stop_windows(&self) -> Result<(), String> {
        Err(format!(
            "Windows service stop not yet implemented for '{}'",
            self.config.name
        ))
    }
    
    #[cfg(windows)]
    fn status_windows(&self) -> ServiceStatus {
        // TODO: Query Windows SCM for service '{}' status
        eprintln!("Windows service status not yet implemented for '{}'", self.config.name);
        ServiceStatus::Unknown
    }
    
    // =========================================================================
    // macOS implementation (launchd)
    // =========================================================================
    
    #[cfg(target_os = "macos")]
    fn install_macos(&self) -> Result<(), String> {
        let plist_path = self.launchd_plist_path();
        let plist_content = self.generate_launchd_plist();
        
        std::fs::create_dir_all(plist_path.parent().unwrap())
            .map_err(|e| e.to_string())?;
        std::fs::write(&plist_path, plist_content)
            .map_err(|e| e.to_string())?;
        
        // Load the service
        std::process::Command::new("launchctl")
            .args(["load", plist_path.to_str().unwrap()])
            .status()
            .map_err(|e| e.to_string())?;
        
        Ok(())
    }
    
    #[cfg(target_os = "macos")]
    fn uninstall_macos(&self) -> Result<(), String> {
        let plist_path = self.launchd_plist_path();
        
        // Unload first
        let _ = std::process::Command::new("launchctl")
            .args(["unload", plist_path.to_str().unwrap()])
            .status();
        
        // Remove plist
        if plist_path.exists() {
            std::fs::remove_file(&plist_path).map_err(|e| e.to_string())?;
        }
        
        Ok(())
    }
    
    #[cfg(target_os = "macos")]
    fn start_macos(&self) -> Result<(), String> {
        std::process::Command::new("launchctl")
            .args(["start", &format!("com.nanna.{}", self.config.name)])
            .status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    
    #[cfg(target_os = "macos")]
    fn stop_macos(&self) -> Result<(), String> {
        std::process::Command::new("launchctl")
            .args(["stop", &format!("com.nanna.{}", self.config.name)])
            .status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    
    #[cfg(target_os = "macos")]
    fn status_macos(&self) -> ServiceStatus {
        let output = std::process::Command::new("launchctl")
            .args(["list", &format!("com.nanna.{}", self.config.name)])
            .output();
        
        match output {
            Ok(o) if o.status.success() => ServiceStatus::Running,
            _ => ServiceStatus::Stopped,
        }
    }
    
    #[cfg(target_os = "macos")]
    fn launchd_plist_path(&self) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("com.nanna.{}.plist", self.config.name))
    }
    
    #[cfg(target_os = "macos")]
    fn generate_launchd_plist(&self) -> String {
        let exe = self.config.executable.display();
        let args: String = self.config.arguments.iter()
            .map(|a| format!("        <string>{}</string>", a))
            .collect::<Vec<_>>()
            .join("\n");
        
        format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.nanna.{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
{}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/nanna-daemon.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/nanna-daemon.err</string>
</dict>
</plist>"#, self.config.name, exe, args)
    }
    
    // =========================================================================
    // Linux implementation (systemd)
    // =========================================================================
    
    #[cfg(target_os = "linux")]
    fn install_linux(&self) -> Result<(), String> {
        let unit_path = self.systemd_unit_path();
        let unit_content = self.generate_systemd_unit();
        
        std::fs::create_dir_all(unit_path.parent().unwrap())
            .map_err(|e| e.to_string())?;
        std::fs::write(&unit_path, unit_content)
            .map_err(|e| e.to_string())?;
        
        // Reload systemd
        std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status()
            .map_err(|e| e.to_string())?;
        
        // Enable the service
        std::process::Command::new("systemctl")
            .args(["--user", "enable", &self.config.name])
            .status()
            .map_err(|e| e.to_string())?;
        
        Ok(())
    }
    
    #[cfg(target_os = "linux")]
    fn uninstall_linux(&self) -> Result<(), String> {
        // Disable and stop
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "--now", &self.config.name])
            .status();
        
        // Remove unit file
        let unit_path = self.systemd_unit_path();
        if unit_path.exists() {
            std::fs::remove_file(&unit_path).map_err(|e| e.to_string())?;
        }
        
        // Reload systemd
        std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status()
            .map_err(|e| e.to_string())?;
        
        Ok(())
    }
    
    #[cfg(target_os = "linux")]
    fn start_linux(&self) -> Result<(), String> {
        std::process::Command::new("systemctl")
            .args(["--user", "start", &self.config.name])
            .status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    
    #[cfg(target_os = "linux")]
    fn stop_linux(&self) -> Result<(), String> {
        std::process::Command::new("systemctl")
            .args(["--user", "stop", &self.config.name])
            .status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    
    #[cfg(target_os = "linux")]
    fn status_linux(&self) -> ServiceStatus {
        let output = std::process::Command::new("systemctl")
            .args(["--user", "is-active", &self.config.name])
            .output();
        
        match output {
            Ok(o) => {
                let status = String::from_utf8_lossy(&o.stdout).trim().to_string();
                match status.as_str() {
                    "active" => ServiceStatus::Running,
                    "inactive" => ServiceStatus::Stopped,
                    "activating" => ServiceStatus::Starting,
                    "deactivating" => ServiceStatus::Stopping,
                    _ => ServiceStatus::Unknown,
                }
            }
            _ => ServiceStatus::Unknown,
        }
    }
    
    #[cfg(target_os = "linux")]
    fn systemd_unit_path(&self) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".config/systemd/user")
            .join(format!("{}.service", self.config.name))
    }
    
    #[cfg(target_os = "linux")]
    fn generate_systemd_unit(&self) -> String {
        let exe = self.config.executable.display();
        let args = self.config.arguments.join(" ");
        
        format!(r#"[Unit]
Description={}
After=network.target

[Service]
Type=simple
ExecStart={} {}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#, self.config.description, exe, args)
    }
}
