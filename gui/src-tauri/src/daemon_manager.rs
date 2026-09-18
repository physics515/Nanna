//! Daemon Manager - Manages the nanna-daemon sidecar lifecycle
//!
//! Responsibilities:
//! - Start daemon on app boot
//! - Monitor daemon health
//! - Restart on crash
//! - Stop on app exit

use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_shell::{ShellExt, process::CommandChild};
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Daemon process state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

/// Daemon manager configuration
#[derive(Debug, Clone)]
pub struct DaemonManagerConfig {
    /// Port for the daemon to listen on
    pub port: u16,
    /// Host to bind to
    pub host: String,
    /// Maximum restart attempts before giving up
    pub max_restarts: u32,
    /// Delay between restart attempts
    pub restart_delay: Duration,
    /// Health check interval
    pub health_check_interval: Duration,
    /// Startup timeout
    pub startup_timeout: Duration,
}

/// The daemon port `NANNA_DAEMON_PORT` asks for, if it names a usable one.
///
/// For isolated runs (automated GUI verification above all): a GUI on the
/// default port attaches to whatever daemon already listens there — on a
/// developer's machine, their own, with their real data. Pure.
#[must_use]
pub fn port_override(value: Option<&str>) -> Option<u16> {
    value?
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port >= 1024)
}

#[cfg(test)]
mod port_override_tests {
    use super::port_override;

    #[test]
    fn only_a_usable_port_overrides_the_default() {
        assert_eq!(port_override(Some("51997")), Some(51997));
        assert_eq!(port_override(Some(" 6000 ")), Some(6000));
        assert_eq!(port_override(None), None);
        assert_eq!(port_override(Some("")), None);
        assert_eq!(
            port_override(Some("80")),
            None,
            "privileged ports are not a daemon's"
        );
        assert_eq!(port_override(Some("70000")), None);
        assert_eq!(port_override(Some("fifty")), None);
    }
}

impl Default for DaemonManagerConfig {
    fn default() -> Self {
        Self {
            port: 5149,
            host: "127.0.0.1".to_string(),
            max_restarts: 3,
            restart_delay: Duration::from_secs(2),
            health_check_interval: Duration::from_secs(30),
            // Generous: first boot can be slow (DB open + tool discovery). The
            // slow embedding-model probe was moved off the daemon's startup
            // critical path, so readiness is normally a few seconds — this is
            // purely a worst-case ceiling.
            startup_timeout: Duration::from_secs(90),
        }
    }
}

/// Kill the daemon sidecar's process tree by PID, best-effort. A current
/// daemon's own kill-on-close Job Object (adopted at startup) already reaps
/// its children when it dies; this is belt-and-braces for daemons where
/// adoption failed or that predate it.
///
/// Windows delegates to the shared `taskkill /T` walk in `nanna_proc` (which
/// suppresses the console window a windows-subsystem process like this one
/// would otherwise flash). Unix is a deliberate no-op — tauri-plugin-shell
/// owns the sidecar spawn, so the process_group(0)-at-spawn contract behind
/// `nanna_proc`'s group kill does not hold here; the caller's
/// `CommandChild::kill()` covers the direct child.
#[cfg(windows)]
async fn kill_sidecar_tree(pid: u32) {
    nanna_proc::kill_process_tree(pid).await;
}

/// The Unix no-op. It returns a ready future rather than being an `async fn`
/// with nothing to await, so callers `.await` both platforms the same way.
#[cfg(not(windows))]
fn kill_sidecar_tree(_pid: u32) -> std::future::Ready<()> {
    std::future::ready(())
}

/// Request id of the version probe; the reply is matched on it.
const VERSION_PROBE_ID: &str = "daemon-manager-version";

/// End-to-end ceiling on one version probe: connect, ask, read the reply.
/// The same 2 s the graceful-shutdown connect gets, plus 1 s for the reply —
/// `system version` is a constant built into the daemon binary, with no I/O.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// How long an evicted daemon gets to release the port. A graceful stop
/// cancels any in-flight turn and saves stats before exiting; `stop()` allows
/// 5 s for the same thing on a daemon it owns and then tree-kills it. We did
/// not spawn this one and cannot kill it, so it gets twice that.
const EVICTION_TIMEOUT: Duration = Duration::from_secs(10);

/// What answered on the daemon port.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PortOccupant {
    /// Nothing accepted a connection.
    Nobody,
    /// A daemon accepted. `version` is `None` when it did not report one in
    /// time — still mid-init, or too old to know the request.
    Daemon { version: Option<String> },
}

/// The occupant's version when it is a daemon from a *different* release.
/// An occupant that did not report a version is never called stale.
fn stale_version<'a>(occupant: &'a PortOccupant, ours: &str) -> Option<&'a str> {
    match occupant {
        PortOccupant::Daemon { version: Some(theirs) } if theirs != ours => Some(theirs),
        _ => None,
    }
}

/// The daemon's answer to the version probe.
#[derive(Debug, PartialEq, Eq)]
struct VersionReply {
    /// `None` when the reply carried no version (an error response).
    version: Option<String>,
}

/// Parse one IPC frame; `None` unless it is the reply to the probe.
fn version_from_reply(text: &str) -> Option<VersionReply> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value.get("id").and_then(serde_json::Value::as_str) != Some(VERSION_PROBE_ID) {
        return None;
    }
    let version = value
        .pointer("/result/data/version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Some(VersionReply { version })
}

/// Connect to `url` and ask whatever is there for its version.
async fn probe_occupant(url: &str) -> PortOccupant {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let deadline = tokio::time::Instant::now() + PROBE_TIMEOUT;
    let Ok(Ok((mut ws, _))) =
        tokio::time::timeout_at(deadline, tokio_tungstenite::connect_async(url)).await
    else {
        return PortOccupant::Nobody;
    };
    let request = serde_json::json!({
        "id": VERSION_PROBE_ID,
        "action": { "type": "system", "action": "version" }
    });
    let mut version = None;
    if ws.send(Message::Text(request.to_string().into())).await.is_ok() {
        // Unsolicited event frames may arrive first; skip until the reply.
        while let Ok(Some(Ok(frame))) = tokio::time::timeout_at(deadline, ws.next()).await {
            if let Message::Text(text) = frame
                && let Some(reply) = version_from_reply(&text)
            {
                version = reply.version;
                break;
            }
        }
    }
    let _ = ws.close(None).await;
    PortOccupant::Daemon { version }
}

/// Manages the daemon sidecar process
pub struct DaemonManager {
    config: DaemonManagerConfig,
    state: Arc<RwLock<DaemonState>>,
    restart_count: Arc<RwLock<u32>>,
    child: Arc<RwLock<Option<CommandChild>>>,
    /// Whether the spawned sidecar process has terminated. Distinguishes a
    /// live sidecar we own (stop = graceful shutdown, then tree-kill) from a
    /// dead one whose PID may have been recycled — e.g. the `AlreadyRunning`
    /// exit when we merely attached to a standalone daemon, which is not
    /// ours to stop.
    sidecar_exited: Arc<std::sync::atomic::AtomicBool>,
}

impl DaemonManager {
    /// Create a new daemon manager
    #[must_use]
    pub fn new(config: DaemonManagerConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(DaemonState::Stopped)),
            restart_count: Arc::new(RwLock::new(0)),
            child: Arc::new(RwLock::new(None)),
            sidecar_exited: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
    
    /// Get the daemon WebSocket URL
    #[must_use]
    pub fn ws_url(&self) -> String {
        format!("ws://{}:{}", self.config.host, self.config.port)
    }
    
    /// Get current daemon state
    pub async fn state(&self) -> DaemonState {
        *self.state.read().await
    }
    
    /// Start the daemon sidecar
    ///
    /// Returns `Ok` straight away when the daemon is already `Running` or
    /// `Starting`.
    ///
    /// # Errors
    ///
    /// - `"Failed to create sidecar command: …"` when the bundled
    ///   `nanna-daemon` sidecar cannot be resolved;
    /// - `"Failed to spawn daemon: …"` when its process fails to start;
    /// - `"Daemon startup timeout"` when nothing answers on the daemon port
    ///   within `startup_timeout` (90 s by default). The spawned child is
    ///   killed first so it cannot hold the port and the database lock.
    pub async fn start(&self, app: &AppHandle) -> Result<(), String> {
        let current_state = *self.state.read().await;
        if current_state == DaemonState::Running || current_state == DaemonState::Starting {
            return Ok(());
        }
        
        *self.state.write().await = DaemonState::Starting;
        self.evict_stale_daemon(env!("CARGO_PKG_VERSION")).await;
        info!("Starting nanna-daemon sidecar...");

        // Spawn the sidecar
        let shell = app.shell();
        info!("Creating sidecar command for nanna-daemon...");
        let sidecar = shell.sidecar("nanna-daemon")
            .map_err(|e| {
                error!("Failed to create sidecar command: {}", e);
                format!("Failed to create sidecar command: {e}")
            })?;
        
        let args = self.sidecar_args();
        info!("Spawning daemon with args: {:?}", args);
        let (mut rx, child) = sidecar
            .args(args)
            .spawn()
            .map_err(|e| {
                error!("Failed to spawn daemon: {}", e);
                format!("Failed to spawn daemon: {e}")
            })?;
        
        // Store the child handle
        *self.child.write().await = Some(child);
        self.sidecar_exited
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // Spawn a task to log daemon output (use info level so it's visible in production)
        // It also flips the shared state to Crashed on termination so wait_for_ready
        // can abort immediately instead of polling out the full startup timeout.
        let state_for_events = self.state.clone();
        let sidecar_exited = self.sidecar_exited.clone();
        tokio::spawn(async move {
            use tauri_plugin_shell::process::CommandEvent;
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        info!("daemon stdout: {}", msg);
                    }
                    CommandEvent::Stderr(line) => {
                        let msg = String::from_utf8_lossy(&line);
                        error!("daemon stderr: {}", msg);
                    }
                    CommandEvent::Terminated(payload) => {
                        // Record the death so any in-flight ready-wait bails
                        // out now, and stop() can tell a graceful exit from a
                        // hang (and never tree-kills a recycled PID).
                        sidecar_exited.store(true, std::sync::atomic::Ordering::SeqCst);
                        {
                            let mut state = state_for_events.write().await;
                            if *state == DaemonState::Starting || *state == DaemonState::Running {
                                *state = DaemonState::Crashed;
                            }
                        }
                        if let Some(code) = payload.code {
                            if code != 0 {
                                error!("daemon terminated with exit code: {}", code);
                            } else {
                                info!("daemon terminated normally (code 0)");
                            }
                        } else if let Some(signal) = payload.signal {
                            warn!("daemon terminated by signal: {}", signal);
                        } else {
                            warn!("daemon terminated (unknown reason)");
                        }
                        break;
                    }
                    CommandEvent::Error(err) => {
                        error!("daemon error event: {}", err);
                    }
                    _ => {}
                }
            }
        });
        
        // Wait for daemon to be ready
        let ready = self.wait_for_ready().await;

        if ready {
            *self.state.write().await = DaemonState::Running;
            *self.restart_count.write().await = 0;
            info!("Daemon started successfully on {}", self.ws_url());
            Ok(())
        } else {
            *self.state.write().await = DaemonState::Crashed;
            error!("Daemon failed to start within timeout");
            // Kill the spawned child: an orphaned daemon would keep running,
            // holding the nanna.db file lock (and port 5149) — which would make
            // the embedded fallback unable to open storage at all.
            // The child slot stays locked until the kill finishes, exactly as
            // long as the guard used to live in the `if let`: a concurrent
            // `stop()` waits for this kill instead of reporting `Stopped`
            // while it is still in flight.
            let mut child_slot = self.child.write().await;
            if let Some(child) = child_slot.take() {
                warn!("Killing unresponsive daemon child so it cannot orphan the DB lock");
                // Tree-kill only a sidecar that is still alive: a dead one's
                // PID may already belong to an unrelated process.
                if !self.sidecar_exited.load(std::sync::atomic::Ordering::SeqCst) {
                    kill_sidecar_tree(child.pid()).await;
                }
                if let Err(e) = child.kill() {
                    warn!("Failed to kill unresponsive daemon: {}", e);
                }
            }
            drop(child_slot);
            Err("Daemon startup timeout".to_string())
        }
    }
    
    /// The sidecar's command line: the bind address, the optional dev-store
    /// isolation, then `run`.
    fn sidecar_args(&self) -> Vec<String> {
        // A dev build must never share the installed app's store: the daemon
        // takes an exclusive lock on nanna.db and runs migrations at startup,
        // so two builds pointed at one file is unsupported and unsafe.
        // NANNA_DEV_DATA_DIR isolates a dev run; unset in production, where
        // the daemon resolves its own default data dir as before.
        let port = self.config.port.to_string();
        let mut args: Vec<String> = vec![
            "--port".into(),
            port,
            "--host".into(),
            self.config.host.clone(),
        ];
        if let Ok(dev_data_dir) = std::env::var("NANNA_DEV_DATA_DIR")
            && !dev_data_dir.trim().is_empty() {
                info!("NANNA_DEV_DATA_DIR set — isolating daemon store at {dev_data_dir}");
                args.push("--data-dir".into());
                args.push(dev_data_dir);
            }
        args.push("run".into());
        args
    }

    /// Shut down a daemon from a different release that is holding our port.
    ///
    /// An app update replaces the GUI and its sidecar binary, but a daemon
    /// that outlived the old GUI keeps port 5149 and the `nanna.db` lock. The
    /// new sidecar then cannot bind, exits, and the GUI would attach to the old
    /// daemon — a v0.3.20 UI driving a v0.3.19 server, still running every bug
    /// the update shipped to fix (2026-09-17). Evicting it first lets the new
    /// sidecar start.
    ///
    /// Only a daemon that *reports* a different version is touched. One that
    /// cannot be identified is left alone: attaching to it is the old
    /// behaviour, and shutting down something we cannot name is worse.
    async fn evict_stale_daemon(&self, ours: &str) {
        let url = self.ws_url();
        let occupant = probe_occupant(&url).await;
        let Some(theirs) = stale_version(&occupant, ours) else {
            return;
        };
        warn!(
            "A v{theirs} daemon is running on {url}, but this app is v{ours} — asking it to shut down so the matching daemon can start"
        );
        if !self.request_graceful_shutdown().await {
            error!("Could not deliver a shutdown request to the v{theirs} daemon on {url}");
            return;
        }
        let deadline = tokio::time::Instant::now() + EVICTION_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if probe_occupant(&url).await == PortOccupant::Nobody {
                info!("The v{theirs} daemon released {url}");
                return;
            }
            sleep(Duration::from_millis(200)).await;
        }
        error!(
            "The v{theirs} daemon on {url} is still running {}s after a shutdown request — the app will be talking to a server from a different release",
            EVICTION_TIMEOUT.as_secs()
        );
    }

    /// Wait for daemon to be ready (accepting connections)
    async fn wait_for_ready(&self) -> bool {
        let url = self.ws_url();
        let deadline = tokio::time::Instant::now() + self.config.startup_timeout;

        let mut child_exited = false;
        let mut evicted = false;
        while tokio::time::Instant::now() < deadline {
            // A dead child can still mean a healthy daemon: the sidecar
            // exits AlreadyRunning when a standalone daemon holds the
            // PID-file lock (single-instance guard), and that daemon may
            // still be mid-init (IPC binds late). Keep polling the port
            // until the deadline; only fail when nobody ever answers.
            if !child_exited && *self.state.read().await == DaemonState::Crashed {
                child_exited = true;
                warn!(
                    "Sidecar exited during startup — polling {} for an existing daemon instance",
                    url
                );
            }
            match probe_occupant(&url).await {
                PortOccupant::Nobody => {
                    // Not ready yet, wait and retry
                    sleep(Duration::from_millis(200)).await;
                }
                occupant @ PortOccupant::Daemon { .. } => {
                    // A daemon from another release can win the port between
                    // the pre-spawn check and here. Evict it once; answering
                    // is not the same as being ours.
                    if stale_version(&occupant, env!("CARGO_PKG_VERSION")).is_some() && !evicted {
                        evicted = true;
                        self.evict_stale_daemon(env!("CARGO_PKG_VERSION")).await;
                        continue;
                    }
                    if child_exited {
                        info!("Attached to an existing daemon instance on {}", url);
                    }
                    return true;
                }
            }
        }
        
        false
    }
    
    /// Stop the daemon
    ///
    /// # Errors
    ///
    /// Never returns `Err` today: an undeliverable or ignored shutdown request
    /// falls back to a tree-kill, a failed kill is logged, and the manager
    /// always ends `Stopped`.
    pub async fn stop(&self) -> Result<(), String> {
        let current_state = *self.state.read().await;
        if current_state == DaemonState::Stopped || current_state == DaemonState::Stopping {
            return Ok(());
        }

        *self.state.write().await = DaemonState::Stopping;
        info!("Stopping nanna-daemon...");

        // Held until the shutdown below completes, exactly as long as the guard
        // used to live in the `if let`: a racing `start()` can neither store a
        // new child nor kill one while this shutdown is in progress.
        let mut child_slot = self.child.write().await;
        if let Some(child) = child_slot.take() {
            if self.sidecar_exited.load(std::sync::atomic::Ordering::SeqCst) {
                // The sidecar died earlier — usually its AlreadyRunning exit
                // after attaching to a standalone daemon. That daemon isn't
                // ours to stop, and the sidecar's PID may have been recycled,
                // so neither a shutdown request nor a tree-kill is safe here.
                info!("Sidecar already exited — leaving any attached daemon running");
            } else {
                let pid = child.pid();
                // Prefer a graceful IPC shutdown: the daemon flushes state and
                // its kill-on-close Job Object reaps in-flight exec children on
                // exit. A pre-Job-Object daemon acks the request without
                // stopping — the bounded wait catches that and falls through.
                let exited = self.request_graceful_shutdown().await
                    && self.wait_for_terminated(Duration::from_secs(5)).await;
                if exited {
                    info!("Daemon exited gracefully");
                } else {
                    // Hard stop: kill the TREE, not just the daemon — a bare
                    // kill() orphans in-flight exec children on daemons whose
                    // Job Object never adopted (or that predate it).
                    warn!("Graceful daemon shutdown failed — tree-killing PID {pid}");
                    kill_sidecar_tree(pid).await;
                    if let Err(e) = child.kill() {
                        warn!("Failed to kill daemon process: {}", e);
                    }
                }
            }
        }
        drop(child_slot);

        *self.state.write().await = DaemonState::Stopped;
        info!("Daemon stopped");
        Ok(())
    }

    /// Ask the daemon to stop via its control plane. Returns whether the
    /// request was delivered — not whether the daemon actually exited.
    async fn request_graceful_shutdown(&self) -> bool {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::Message;

        let url = self.ws_url();
        let Ok(Ok((mut ws, _))) =
            tokio::time::timeout(Duration::from_secs(2), tokio_tungstenite::connect_async(&url))
                .await
        else {
            return false;
        };
        let request = serde_json::json!({
            "id": "daemon-manager-shutdown",
            "action": { "type": "system", "action": "shutdown" }
        });
        // send() flushes before returning; the daemon acks, then stops.
        let sent = ws.send(Message::Text(request.to_string().into())).await.is_ok();
        let _ = ws.close(None).await;
        sent
    }

    /// Wait (bounded) for the sidecar's Terminated event, recorded by the
    /// event task in `sidecar_exited`.
    async fn wait_for_terminated(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if self.sidecar_exited.load(std::sync::atomic::Ordering::SeqCst) {
                return true;
            }
            sleep(Duration::from_millis(100)).await;
        }
        false
    }
    
    /// Restart the daemon
    ///
    /// # Errors
    ///
    /// Returns [`Self::start`]'s error; the stop half does not fail.
    pub async fn restart(&self, app: &AppHandle) -> Result<(), String> {
        self.stop().await?;
        sleep(Duration::from_millis(500)).await;
        self.start(app).await
    }
    
    /// Start health monitoring (call once after start)
    pub fn start_health_monitor(self: Arc<Self>, app: AppHandle) {
        let manager = self.clone();
        let config = self.config.clone();
        
        tokio::spawn(async move {
            loop {
                sleep(config.health_check_interval).await;
                
                // `Crashed` is watched too. When the app attaches to a daemon
                // it did not spawn, its own sidecar exits *after* readiness and
                // the Terminated handler flips Running → Crashed. Skipping that
                // state meant the attached daemon's death was never noticed and
                // nothing was ever restarted.
                let state = *manager.state.read().await;
                if !matches!(state, DaemonState::Running | DaemonState::Crashed) {
                    continue;
                }

                // Health check: try to connect
                let url = manager.ws_url();
                match tokio_tungstenite::connect_async(&url).await {
                    Ok((mut ws, _)) => {
                        let _ = futures_util::SinkExt::close(&mut ws).await;
                        if state == DaemonState::Crashed {
                            // Our sidecar is gone, but the daemon we attached
                            // to is alive and answering.
                            *manager.state.write().await = DaemonState::Running;
                        }
                        debug!("Daemon health check: OK");
                    }
                    Err(e) => {
                        warn!("Daemon health check failed: {}", e);
                        *manager.state.write().await = DaemonState::Crashed;
                        
                        // Try to restart
                        let restart_count = *manager.restart_count.read().await;
                        if restart_count < config.max_restarts {
                            *manager.restart_count.write().await = restart_count + 1;
                            warn!("Attempting daemon restart ({}/{})", restart_count + 1, config.max_restarts);
                            
                            sleep(config.restart_delay).await;
                            if let Err(e) = manager.start(&app).await {
                                error!("Daemon restart failed: {}", e);
                            }
                        } else {
                            error!("Max daemon restarts exceeded, giving up");
                        }
                    }
                }
            }
        });
    }
}

impl Drop for DaemonManager {
    fn drop(&mut self) {
        // Note: async drop not possible, but child will be killed when dropped
        info!("DaemonManager dropped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    fn daemon(version: Option<&str>) -> PortOccupant {
        PortOccupant::Daemon { version: version.map(str::to_string) }
    }

    #[test]
    fn only_a_daemon_reporting_another_version_is_stale() {
        assert_eq!(stale_version(&daemon(Some("0.3.19")), "0.3.20"), Some("0.3.19"));
        assert_eq!(stale_version(&daemon(Some("0.3.20")), "0.3.20"), None);
        // Unidentified: attach as before rather than shut down a stranger.
        assert_eq!(stale_version(&daemon(None), "0.3.20"), None);
        assert_eq!(stale_version(&PortOccupant::Nobody, "0.3.20"), None);
    }

    #[test]
    fn version_is_read_only_from_the_probe_reply() {
        let reply = r#"{"id":"daemon-manager-version","result":{"status":"success","data":{"version":"0.3.19","name":"nanna-daemon"}}}"#;
        assert_eq!(version_from_reply(reply), Some(VersionReply { version: Some("0.3.19".to_string()) }));

        let error = r#"{"id":"daemon-manager-version","result":{"status":"error","code":"x","message":"y"}}"#;
        assert_eq!(version_from_reply(error), Some(VersionReply { version: None }));

        let other = r#"{"id":"something-else","result":{"status":"success","data":{"version":"9.9.9"}}}"#;
        assert_eq!(version_from_reply(other), None);
        assert_eq!(version_from_reply(r#"{"event":"heartbeat"}"#), None);
        assert_eq!(version_from_reply("not json"), None);
    }

    /// Serve one connection the way a daemon would: an unsolicited event
    /// first, then the reply to the version request.
    async fn fake_daemon(version: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let Some(Ok(Message::Text(request))) = ws.next().await else { return };
            let request: serde_json::Value = serde_json::from_str(&request).unwrap();
            assert_eq!(request.pointer("/action/action"), Some(&serde_json::json!("version")));
            let event = serde_json::json!({ "event": "heartbeat" });
            let reply = serde_json::json!({
                "id": request["id"],
                "result": { "status": "success", "data": { "version": version } }
            });
            let _ = ws.send(Message::Text(event.to_string().into())).await;
            let _ = ws.send(Message::Text(reply.to_string().into())).await;
        });
        url
    }

    #[tokio::test]
    async fn probe_reports_the_version_past_unsolicited_events() {
        let url = fake_daemon("0.3.19").await;
        assert_eq!(probe_occupant(&url).await, daemon(Some("0.3.19")));
    }

    #[tokio::test]
    async fn probe_of_a_closed_port_finds_nobody() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        drop(listener);
        assert_eq!(probe_occupant(&url).await, PortOccupant::Nobody);
    }
}
