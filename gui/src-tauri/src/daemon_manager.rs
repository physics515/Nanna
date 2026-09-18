//! Daemon Manager - Manages the nanna-daemon sidecar lifecycle
//!
//! Responsibilities:
//! - Start daemon on app boot
//! - Monitor daemon health
//! - Restart on crash
//! - Stop on app exit

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::AppHandle;
use tauri::async_runtime::Receiver;
use tauri_plugin_shell::{ShellExt, process::{CommandChild, CommandEvent}};
use tokio::sync::{Mutex, RwLock};
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
        }
    }
}

/// When a boot counts as slow enough to log. Readiness normally takes a few
/// seconds (claim the role, open storage, discover tools). Later notices come
/// at double the previous elapsed time, so even a boot that takes hours logs
/// only a handful of lines. The status footer switches to "Still starting"
/// at the same point (`backendLabels.ts`). Nothing here stops the wait.
const SLOW_START_NOTICE: Duration = Duration::from_secs(30);

/// Where the wait for a starting sidecar stands after one poll of its port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootWait {
    /// A daemon answered.
    Ready,
    /// Nobody answered yet and the sidecar is alive: it is still booting.
    KeepWaiting,
    /// Nobody answered and the sidecar is gone: the start failed.
    Failed,
    /// `stop()` took over while the wait was running.
    Cancelled,
}

/// The ready-wait's verdict after one poll of the daemon port. `answered`
/// says whether a daemon accepted the probe, and `sidecar_exited` whether the
/// sidecar had already exited when the probe began.
///
/// A live sidecar is still booting, however long that takes. The daemon
/// claims its PID file and reserves the port before it does anything slow,
/// and it opens the port only when it is ready. So a live process with a
/// closed port is a boot in progress. A cold model load, a big migration or a
/// slow provider can take minutes. The old 90 s timeout killed such a boot,
/// and every relaunch then did the same thing (2026-09-18).
///
/// A sidecar that has exited is a failure. That includes the `AlreadyRunning`
/// exit, where another daemon already holds the role. The poll that sees the
/// exit still probes the port, so a daemon that is already up gets attached.
/// A daemon that is still booting is not ours to wait for: the client's retry
/// loop attaches it when its port opens.
///
/// A stop wins over everything else. The wait must not leave behind a
/// `Running` or `Crashed` state after `stop()` has already reported `Stopped`.
const fn boot_wait_verdict(state: DaemonState, sidecar_exited: bool, answered: bool) -> BootWait {
    if matches!(state, DaemonState::Stopping | DaemonState::Stopped) {
        BootWait::Cancelled
    } else if answered {
        BootWait::Ready
    } else if sidecar_exited {
        BootWait::Failed
    } else {
        BootWait::KeepWaiting
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

/// One health check: complete a WebSocket handshake with the daemon, then
/// close it again.
///
/// Bounded by `timeout` end to end. An unbounded check hung for good on a
/// port that accepts the TCP connection but never completes the handshake,
/// and the monitor it runs in stopped acting with the last state frozen.
async fn check_health(url: &str, timeout: Duration) -> Result<(), String> {
    let check = async {
        let (mut ws, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| e.to_string())?;
        let _ = futures_util::SinkExt::close(&mut ws).await;
        Ok(())
    };
    tokio::time::timeout(timeout, check)
        .await
        .unwrap_or_else(|_| Err(format!("no answer within {timeout:?}")))
}

// =============================================================================
// Why a start failed
// =============================================================================

/// Why the daemon failed to start or to stay up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StartFailureKind {
    /// The bundled `nanna-daemon` program could not be located.
    SidecarUnresolved,
    /// The program was located, but its process did not start.
    SpawnFailed,
    /// The sidecar exited before any daemon answered on the port.
    ExitedDuringBoot,
    /// The sidecar that was serving the app exited.
    ExitedAfterReady,
    /// The daemon stopped answering the health check.
    HealthCheckFailed,
    /// The health monitor used up its restarts and no longer tries.
    RestartsExhausted,
}

/// The most recent reason the daemon failed to start or to stay up, as
/// `BackendStatus::last_error` reports it. Cleared when a daemon becomes
/// ready.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartFailure {
    pub kind: StartFailureKind,
    /// What went wrong, for a person to read. For an exit it is the daemon's
    /// own last error line when it printed one (see [`exit_reason`]), else
    /// the exit code or signal.
    pub message: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    /// When it happened, in milliseconds since the Unix epoch.
    pub at_ms: i64,
}

impl StartFailure {
    fn new(kind: StartFailureKind, message: String) -> Self {
        Self {
            kind,
            message,
            exit_code: None,
            signal: None,
            at_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    /// The failure a sidecar's exit amounts to.
    ///
    /// The exit code cannot say why: every error the daemon's `run` returns
    /// exits with 1, whether another daemon holds the role, another program
    /// holds the port, or the config does not load. The daemon says why in
    /// its last log line, so that line is the message whenever there is one.
    fn from_exit(kind: StartFailureKind, exit: &SidecarExit) -> Self {
        let message = exit.reason.clone().unwrap_or_else(|| {
            let what = if kind == StartFailureKind::ExitedDuringBoot {
                "The daemon exited during startup"
            } else {
                "The daemon exited"
            };
            match (exit.code, exit.signal) {
                (Some(code), _) => format!("{what} (exit code {code})"),
                (None, Some(signal)) => format!("{what} (signal {signal})"),
                (None, None) => what.to_string(),
            }
        });
        Self {
            exit_code: exit.code,
            signal: exit.signal,
            ..Self::new(kind, message)
        }
    }

    /// The monitor has given up. The failure before it stays in the message
    /// (and its exit details in the fields): giving up is the news, but the
    /// reason the restarts failed is what a person can act on.
    fn restarts_exhausted(max_restarts: u32, previous: &Self) -> Self {
        Self {
            exit_code: previous.exit_code,
            signal: previous.signal,
            ..Self::new(
                StartFailureKind::RestartsExhausted,
                format!(
                    "Stopped restarting the daemon after {max_restarts} failed restarts · {}",
                    previous.message
                ),
            )
        }
    }
}

// =============================================================================
// Boot log: the output of the current sidecar
// =============================================================================

/// The output lines of the longest boot on record, from "Starting Nanna
/// daemon…" to "Daemon ready": 457, on 2026-09-18 on the operator's machine
/// (204 tool-registry lines, 184 skill lines, and embedding-congestion
/// warnings from a busy provider). The 16 boots in that week's daemon logs
/// that reached ready printed 204 to 457. A boot with an empty data dir and a
/// scratch config prints 270 (measured the same day).
const LONGEST_MEASURED_BOOT_LINES: usize = 457;

/// How many output lines of one sidecar the boot log keeps: twice the
/// longest boot on record. Any boot that reaches ready is then held whole,
/// from its first line, with room for a setup with twice the tools and
/// skills. A boot that hangs keeps logging (the 2026-09-18 hang printed 5247
/// lines before it was restarted); the log then keeps the newest lines, which
/// hold the current phase and any fatal error.
const BOOT_LOG_LINES: usize = 2 * LONGEST_MEASURED_BOOT_LINES;

/// The longest line the boot log keeps whole, in bytes. The longest of the
/// ~35,000 lines in the operator's daemon logs for 2026-09-14..18 is 1,104
/// bytes, so a real line fits nearly four times over. The cap bounds the log
/// at about [`BOOT_LOG_LINES`] × 4 KiB (3.7 MB) whatever the daemon prints.
const BOOT_LOG_LINE_BYTES: usize = 4096;

/// Which output stream of the sidecar a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BootStream {
    Stdout,
    Stderr,
}

/// One output line of the sidecar, as `get_boot_log` returns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BootLine {
    pub stream: BootStream,
    /// Plain text: no terminal escapes, no line ending.
    pub line: String,
}

/// The newest output lines of one sidecar, at most [`BOOT_LOG_LINES`].
#[derive(Debug, Default)]
struct BootLog {
    lines: VecDeque<BootLine>,
}

impl BootLog {
    fn push(&mut self, line: BootLine) {
        if self.lines.len() >= BOOT_LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }
}

/// `raw` as plain text: ANSI escape sequences removed, and every other
/// control character except tab (the line ending among them).
///
/// The sidecar is spawned with `NO_COLOR`, so the daemon's log should carry
/// no escapes. This keeps them out anyway: an older daemon, or a library
/// that writes to the terminal itself, shows up in the splash as text.
fn plain_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI (colours, cursor moves): parameter and intermediate
                // bytes, then one final byte in `@`..=`~`.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC (titles, links): up to BEL or the string terminator
                // `ESC \`.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' {
                            chars.next_if_eq(&'\\');
                            break;
                        }
                    }
                }
                // nF escapes (`ESC ( B` selects a character set):
                // intermediate bytes, then one final byte.
                Some(' '..='/') => {
                    while chars.next_if(|c| (' '..='/').contains(c)).is_some() {}
                    chars.next();
                }
                // Any other escape is ESC and one more character.
                _ => {}
            },
            '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// `line`, cut to [`BOOT_LOG_LINE_BYTES`] on a character boundary. A cut
/// line says so and how much it lost.
fn fit_line(mut line: String) -> String {
    use std::fmt::Write as _;

    if line.len() <= BOOT_LOG_LINE_BYTES {
        return line;
    }
    let mut end = BOOT_LOG_LINE_BYTES;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    let cut = line.len() - end;
    line.truncate(end);
    let _ = write!(line, "… ({cut} more bytes)");
    line
}

/// The next whitespace-separated token of `s`, and what follows it.
fn next_token(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    s.split_once(char::is_whitespace).unwrap_or((s, ""))
}

/// Whether `s` is a `tracing` target: a Rust module path.
fn is_log_target(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

/// The message of a daemon log line at ERROR level, or `None` for any other
/// line.
///
/// The daemon logs to stdout in `tracing`'s default format,
/// `<timestamp> <LEVEL> <target>: <message>`, e.g.
/// `2026-09-18T17:21:44.209179Z ERROR nanna_daemon: Error: Already running`.
/// The message is what follows the target: `Error: Already running`.
fn error_message(line: &str) -> Option<&str> {
    let (first, rest) = next_token(line);
    let rest = if first == "ERROR" {
        rest
    } else {
        let (second, rest) = next_token(rest);
        if second != "ERROR" {
            return None;
        }
        rest
    };
    let rest = rest.trim();
    let message = match rest.split_once(": ") {
        Some((target, message)) if is_log_target(target) => message,
        _ => rest,
    };
    (!message.is_empty()).then_some(message)
}

/// Why the sidecar exited, in its own words: the last thing it said, when
/// that was an error. `None` when its output ends in anything else, for
/// example a process killed mid-work by a signal.
///
/// The daemon reports every fatal error through its log: `run` failing
/// logs `Error: <what>` at ERROR level and exits 1 (nanna-daemon `main.rs`),
/// and its panic hook logs `PANIC: …`. So the last stdout line decides.
/// Output that bypasses the log goes to stderr (the default panic message,
/// a loader error before `main`): stderr lines written after the last
/// stdout line are the reason when that line is not an error. The two
/// streams are read separately, so their order is only approximate; the
/// daemon's own ERROR line therefore wins over stderr. Rust's `note: run
/// with RUST_BACKTRACE=1 …` hint is never the reason.
fn exit_reason(lines: &VecDeque<BootLine>) -> Option<String> {
    let mut stderr_last: Option<&str> = None;
    for line in lines.iter().rev() {
        match line.stream {
            BootStream::Stderr => {
                if stderr_last.is_none() && !line.line.starts_with("note: ") {
                    stderr_last = Some(line.line.as_str());
                }
            }
            BootStream::Stdout => {
                return error_message(&line.line).or(stderr_last).map(str::to_string);
            }
        }
    }
    stderr_last.map(str::to_string)
}

// =============================================================================
// One spawned sidecar
// =============================================================================

/// How a sidecar ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SidecarExit {
    code: Option<i32>,
    signal: Option<i32>,
    /// See [`exit_reason`].
    reason: Option<String>,
}

/// What one spawned sidecar's event task records, shared with the manager.
/// A new one per spawn, so a sidecar's late output or exit can only ever
/// land in its own record.
#[derive(Default)]
struct SpawnWatch {
    /// The sidecar has exited. Distinguishes a live sidecar we own (stop =
    /// graceful shutdown, then tree-kill) from a dead one whose PID may have
    /// been recycled — e.g. the `AlreadyRunning` exit when we merely attached
    /// to a standalone daemon, which is not ours to stop.
    exited: AtomicBool,
    /// How it exited. Written before `exited` is set, so whoever sees the
    /// flag finds the details.
    exit: Mutex<Option<SidecarExit>>,
    log: Mutex<BootLog>,
}

/// The sidecar process in the manager's child slot, with its record.
struct Sidecar {
    child: CommandChild,
    watch: Arc<SpawnWatch>,
}

/// What a sidecar's exit does to the manager's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitEffect {
    /// Nothing changes: a stop caused the exit, a newer start owns the state,
    /// or the state already says crashed.
    Nothing,
    /// The exit ends a boot. The ready-wait decides what it means: a daemon
    /// our sidecar deferred to may be answering.
    EndsBoot,
    /// The daemon the app was attached to is gone.
    Crash,
    /// A daemon still answers on the port, so the one the app attached to was
    /// never this sidecar: it deferred (the `AlreadyRunning` exit) after the
    /// attach. The app stays attached.
    StayAttached,
}

/// The effect of a sidecar's exit. `current` says whether the sidecar
/// belongs to the current start attempt, and `still_answering` whether a
/// daemon answered on the port after the exit (probed only while `Running`).
///
/// Crashing on every exit while `Running` showed the app "crashed" while it
/// was attached to a healthy daemon, until the next health check 30 s later.
const fn exit_effect(current: bool, state: DaemonState, still_answering: bool) -> ExitEffect {
    if !current {
        return ExitEffect::Nothing;
    }
    match state {
        DaemonState::Starting => ExitEffect::EndsBoot,
        DaemonState::Running if still_answering => ExitEffect::StayAttached,
        DaemonState::Running => ExitEffect::Crash,
        DaemonState::Stopping | DaemonState::Stopped | DaemonState::Crashed => ExitEffect::Nothing,
    }
}

/// The event task of one spawned sidecar: it relays the output into the log
/// and the boot log, and records the exit.
struct SidecarEvents {
    /// The start attempt that spawned this sidecar.
    attempt: u64,
    watch: Arc<SpawnWatch>,
    state: Arc<RwLock<DaemonState>>,
    current_attempt: Arc<AtomicU64>,
    last_failure: Arc<RwLock<Option<StartFailure>>>,
    url: String,
}

impl SidecarEvents {
    async fn run(self, mut events: Receiver<CommandEvent>) {
        while let Some(event) = events.recv().await {
            match event {
                CommandEvent::Stdout(line) => self.relay(BootStream::Stdout, &line).await,
                CommandEvent::Stderr(line) => self.relay(BootStream::Stderr, &line).await,
                CommandEvent::Terminated(payload) => {
                    self.on_exit(payload.code, payload.signal).await;
                    break;
                }
                CommandEvent::Error(err) => {
                    error!("daemon error event: {}", err);
                }
                _ => {}
            }
        }
    }

    /// Log one output line (info level so it is visible in production) and
    /// keep it in the boot log. A blank line says nothing and is dropped.
    async fn relay(&self, stream: BootStream, raw: &[u8]) {
        let line = plain_text(&String::from_utf8_lossy(raw));
        if line.trim().is_empty() {
            return;
        }
        match stream {
            BootStream::Stdout => info!("daemon stdout: {}", line),
            BootStream::Stderr => error!("daemon stderr: {}", line),
        }
        self.watch.log.lock().await.push(BootLine {
            stream,
            line: fit_line(line),
        });
    }

    /// Record the exit so any in-flight ready-wait bails out now and `stop()`
    /// can tell a graceful exit from a hang (and never tree-kills a recycled
    /// PID), then apply its [`ExitEffect`].
    ///
    /// The plugin delivers the exit only after both output streams reached
    /// their end, so the boot log already holds the last line.
    async fn on_exit(&self, code: Option<i32>, signal: Option<i32>) {
        if let Some(code) = code {
            if code != 0 {
                error!("daemon terminated with exit code: {}", code);
            } else {
                info!("daemon terminated normally (code 0)");
            }
        } else if let Some(signal) = signal {
            warn!("daemon terminated by signal: {}", signal);
        } else {
            warn!("daemon terminated (unknown reason)");
        }

        let exit = SidecarExit {
            code,
            signal,
            reason: exit_reason(&self.watch.log.lock().await.lines),
        };
        *self.watch.exit.lock().await = Some(exit.clone());
        // Set before the probe below: a stop that runs meanwhile must see a
        // dead sidecar, and leave alone the daemon that may be answering.
        self.watch.exited.store(true, Ordering::SeqCst);

        let (current, state) = {
            let state = self.state.read().await;
            (self.current_attempt.load(Ordering::SeqCst) == self.attempt, *state)
        };
        let still_answering = current
            && state == DaemonState::Running
            && probe_occupant(&self.url).await != PortOccupant::Nobody;
        match exit_effect(current, state, still_answering) {
            ExitEffect::Crash => {
                let mut state = self.state.write().await;
                if *state == DaemonState::Running
                    && self.current_attempt.load(Ordering::SeqCst) == self.attempt
                {
                    error!("The daemon on {} exited and nothing answers there now", self.url);
                    *self.last_failure.write().await =
                        Some(StartFailure::from_exit(StartFailureKind::ExitedAfterReady, &exit));
                    *state = DaemonState::Crashed;
                }
            }
            ExitEffect::StayAttached => info!(
                "Our sidecar exited, but a daemon still answers on {}: the app stays attached to it",
                self.url
            ),
            ExitEffect::EndsBoot | ExitEffect::Nothing => {}
        }
    }
}

/// Wait (bounded) for a sidecar's exit, which its event task records.
async fn wait_for_terminated(watch: &SpawnWatch, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if watch.exited.load(Ordering::SeqCst) {
            return true;
        }
        sleep(Duration::from_millis(100)).await;
    }
    false
}

/// `start`'s error when a stop took over.
const START_CANCELLED: &str = "Daemon start cancelled by a stop request";

/// Manages the daemon sidecar process
pub struct DaemonManager {
    config: DaemonManagerConfig,
    state: Arc<RwLock<DaemonState>>,
    restart_count: Arc<RwLock<u32>>,
    child: Arc<RwLock<Option<Sidecar>>>,
    /// When the current start began. Read only while the state is `Starting`.
    starting_since: RwLock<Option<tokio::time::Instant>>,
    /// The current start attempt. Each move to `Starting` opens a new one,
    /// under the state lock. A sidecar's event task changes the state only
    /// while its own attempt is still the current one, so a sidecar that a
    /// restart killed cannot mark the next boot crashed.
    attempt: Arc<AtomicU64>,
    /// How many stops have been requested. A start carries the count its
    /// caller saw when it began, and is refused once a stop has come since
    /// (see [`Self::start`]).
    stops: AtomicU64,
    /// The record of the most recent spawn, kept after the sidecar exits and
    /// after a stop: its boot log is what `get_boot_log` returns.
    latest_spawn: RwLock<Option<Arc<SpawnWatch>>>,
    /// Why the daemon last failed to start or to stay up. Cleared when a
    /// daemon becomes ready.
    last_failure: Arc<RwLock<Option<StartFailure>>>,
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
            starting_since: RwLock::new(None),
            attempt: Arc::new(AtomicU64::new(0)),
            stops: AtomicU64::new(0),
            latest_spawn: RwLock::new(None),
            last_failure: Arc::new(RwLock::new(None)),
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

    /// How long the daemon has been starting, or `None` when it is not
    /// `Starting`.
    pub async fn starting_for(&self) -> Option<Duration> {
        if *self.state.read().await != DaemonState::Starting {
            return None;
        }
        self.starting_since.read().await.map(|since| since.elapsed())
    }

    /// How many stops have been requested so far. Read it when a start is
    /// decided on, and pass it to [`Self::start`].
    #[must_use]
    pub fn stop_epoch(&self) -> u64 {
        self.stops.load(Ordering::SeqCst)
    }

    /// Why the daemon last failed to start or to stay up, until a daemon
    /// becomes ready.
    pub async fn last_failure(&self) -> Option<StartFailure> {
        self.last_failure.read().await.clone()
    }

    /// The output lines of the current (or most recent) sidecar, oldest
    /// first. At most [`BOOT_LOG_LINES`]; each spawn starts a new log.
    pub async fn boot_log(&self) -> Vec<BootLine> {
        let Some(watch) = self.latest_spawn.read().await.clone() else {
            return Vec::new();
        };
        let log = watch.log.lock().await;
        log.lines.iter().cloned().collect()
    }

    /// Give automatic restarts a fresh budget. A restart the user asks for
    /// starts the daemon's story over; without this, a monitor that had used
    /// up its restarts earlier would report that it gave up instead of what
    /// the new boot did.
    pub async fn reset_restarts(&self) {
        *self.restart_count.write().await = 0;
    }

    /// Start the daemon sidecar
    ///
    /// Returns `Ok` straight away when the daemon is already `Running` or
    /// `Starting`. Otherwise it waits until a daemon answers on the port, for
    /// as long as the spawned sidecar stays alive. There is no deadline: a
    /// live sidecar whose port is still closed is booting (see
    /// [`boot_wait_verdict`]). [`Self::starting_for`] reports how long it has
    /// been.
    ///
    /// `since_stop` is the [`Self::stop_epoch`] the caller read when it
    /// decided to start. A stop requested after that wins: the start is
    /// refused, or cancelled if it is under way. Without it, an init or a
    /// health-monitor restart that was already on its way when a stop landed
    /// (a restart, an update, quit) booted a daemon right after the stop.
    ///
    /// # Errors
    ///
    /// Each failure leaves the manager `Crashed` and recorded in
    /// [`Self::last_failure`], except a cancelled start.
    ///
    /// - `"Failed to create sidecar command: …"` when the bundled
    ///   `nanna-daemon` sidecar cannot be resolved;
    /// - `"Failed to spawn daemon: …"` when its process fails to start;
    /// - `"Daemon exited during startup"` when the sidecar exits and nothing
    ///   answers on the daemon port;
    /// - `"Daemon start cancelled by a stop request"` when [`Self::stop`] was
    ///   requested after `since_stop`, or while the start was stopping. The
    ///   state is then the one `stop` set.
    pub async fn start(&self, app: &AppHandle, since_stop: u64) -> Result<(), String> {
        let Some(attempt) = self.claim_start(since_stop).await? else {
            return Ok(());
        };
        self.evict_stale_daemon(env!("CARGO_PKG_VERSION"), attempt).await;
        info!("Starting nanna-daemon sidecar...");
        let watch = self.spawn_sidecar(app, attempt).await?;

        // No deadline and no kill. The kill used to keep an embedded
        // in-process backend's fallback able to open nanna.db, and that
        // backend no longer exists. A live sidecar is still booting, and
        // killing it is what turned a slow boot into one that never finished.
        // The only way this wait fails is a sidecar that has exited. A dead
        // process holds no port and no lock, so there is nothing to kill.
        match self.wait_for_ready(attempt, &watch).await {
            BootWait::Ready => {
                if self.mark_ready(attempt).await {
                    info!("Daemon started successfully on {}", self.ws_url());
                    Ok(())
                } else {
                    info!("Daemon start abandoned: a stop request took over");
                    Err(START_CANCELLED.to_string())
                }
            }
            BootWait::Cancelled => {
                info!("Daemon start abandoned: a stop request took over");
                Err(START_CANCELLED.to_string())
            }
            // `wait_for_ready` returns only a final verdict, never `KeepWaiting`.
            BootWait::Failed | BootWait::KeepWaiting => {
                let exit = watch.exit.lock().await.clone().unwrap_or_default();
                error!("Daemon exited during startup and nothing answers on {}", self.ws_url());
                let failure = StartFailure::from_exit(StartFailureKind::ExitedDuringBoot, &exit);
                self.fail_attempt(attempt, failure).await;
                Err("Daemon exited during startup".to_string())
            }
        }
    }

    /// Move to `Starting` and open a new start attempt, whose number is
    /// returned. `None` when a start is already under way or the daemon is
    /// running.
    ///
    /// The check and the move happen under one state lock, so two callers
    /// (an init and the health monitor, say) can never both spawn a sidecar.
    ///
    /// # Errors
    ///
    /// [`START_CANCELLED`] when a stop was requested after `since_stop`, or
    /// one is in progress.
    async fn claim_start(&self, since_stop: u64) -> Result<Option<u64>, String> {
        let mut state = self.state.write().await;
        if self.stops.load(Ordering::SeqCst) != since_stop || *state == DaemonState::Stopping {
            return Err(START_CANCELLED.to_string());
        }
        if matches!(*state, DaemonState::Running | DaemonState::Starting) {
            return Ok(None);
        }
        let attempt = self.attempt.fetch_add(1, Ordering::SeqCst) + 1;
        *self.starting_since.write().await = Some(tokio::time::Instant::now());
        *state = DaemonState::Starting;
        drop(state);
        Ok(Some(attempt))
    }

    /// Open a start attempt the way `start` does before its spawn, for tests
    /// of what runs around a boot (an init in flight, a restart).
    #[cfg(test)]
    pub(crate) async fn begin_start_for_test(&self) -> u64 {
        self.claim_start(self.stop_epoch())
            .await
            .expect("no stop since")
            .expect("nothing under way")
    }

    /// Whether `attempt` still owns a start in progress: no stop, and no
    /// newer start, has come since it began.
    async fn is_starting(&self, attempt: u64) -> bool {
        let state = self.state.read().await;
        *state == DaemonState::Starting && self.attempt.load(Ordering::SeqCst) == attempt
    }

    /// The state as `attempt` sees it: `Stopped` once a newer start has
    /// replaced it, since a stop came in between.
    async fn state_of(&self, attempt: u64) -> DaemonState {
        let state = self.state.read().await;
        if self.attempt.load(Ordering::SeqCst) == attempt {
            *state
        } else {
            DaemonState::Stopped
        }
    }

    /// Start a new boot log and spawn record. The previous sidecar's record
    /// stays with its event task, so its late output cannot mix into the
    /// new log.
    async fn new_spawn_watch(&self) -> Arc<SpawnWatch> {
        let watch = Arc::new(SpawnWatch::default());
        *self.latest_spawn.write().await = Some(Arc::clone(&watch));
        watch
    }

    /// Spawn the sidecar for `attempt` and start its event task. Returns the
    /// spawn's record, which the ready-wait watches.
    ///
    /// The child slot is held from the check to the store. `stop()` takes
    /// that slot after it sets `Stopping`, so a stop either lands before the
    /// check and cancels the spawn, or waits for the new child and stops it.
    /// A stop during the eviction used to miss the child spawned after it,
    /// and that sidecar then ran with no one to stop it.
    async fn spawn_sidecar(
        &self,
        app: &AppHandle,
        attempt: u64,
    ) -> Result<Arc<SpawnWatch>, String> {
        let mut slot = self.child.write().await;
        if !self.is_starting(attempt).await {
            info!("Daemon start abandoned before the spawn: a stop request took over");
            return Err(START_CANCELLED.to_string());
        }
        let watch = self.new_spawn_watch().await;

        // Spawn the sidecar. A failure must not leave the state `Starting`:
        // `start` returns early on `Starting`, so every later call (the Retry
        // button, the health monitor) would report success without spawning.
        let shell = app.shell();
        info!("Creating sidecar command for nanna-daemon...");
        let sidecar = match shell.sidecar("nanna-daemon") {
            Ok(sidecar) => sidecar,
            Err(e) => {
                error!("Failed to create sidecar command: {}", e);
                let failure = StartFailure::new(
                    StartFailureKind::SidecarUnresolved,
                    format!("Could not locate the bundled nanna-daemon program: {e}"),
                );
                self.fail_attempt(attempt, failure).await;
                return Err(format!("Failed to create sidecar command: {e}"));
            }
        };

        let args = self.sidecar_args();
        info!("Spawning daemon with args: {:?}", args);
        // NO_COLOR: the daemon's console log otherwise colours every line
        // (tracing-subscriber turns ANSI on unless NO_COLOR is set, terminal
        // or not), and the relayed lines reached the log view and the boot
        // log full of escape codes.
        let (events, child) = match sidecar.args(args).env("NO_COLOR", "1").spawn() {
            Ok(spawned) => spawned,
            Err(e) => {
                error!("Failed to spawn daemon: {}", e);
                let failure = StartFailure::new(
                    StartFailureKind::SpawnFailed,
                    format!("Could not start nanna-daemon: {e}"),
                );
                self.fail_attempt(attempt, failure).await;
                return Err(format!("Failed to spawn daemon: {e}"));
            }
        };
        *slot = Some(Sidecar {
            child,
            watch: Arc::clone(&watch),
        });
        drop(slot);

        // The event task also records the termination, which is what ends
        // the ready-wait for a sidecar that dies while booting.
        tokio::spawn(self.sidecar_events(attempt, Arc::clone(&watch)).run(events));
        Ok(watch)
    }

    /// The event task for the sidecar `attempt` spawned.
    fn sidecar_events(&self, attempt: u64, watch: Arc<SpawnWatch>) -> SidecarEvents {
        SidecarEvents {
            attempt,
            watch,
            state: Arc::clone(&self.state),
            current_attempt: Arc::clone(&self.attempt),
            last_failure: Arc::clone(&self.last_failure),
            url: self.ws_url(),
        }
    }

    /// `attempt` reached a daemon: `Running`, with a fresh restart budget and
    /// no failure on record. `false` when a stop took over first.
    async fn mark_ready(&self, attempt: u64) -> bool {
        let mut state = self.state.write().await;
        if *state != DaemonState::Starting || self.attempt.load(Ordering::SeqCst) != attempt {
            return false;
        }
        *self.restart_count.write().await = 0;
        *self.last_failure.write().await = None;
        *state = DaemonState::Running;
        true
    }

    /// `attempt` failed: `Crashed`, with `failure` on record. Nothing changes
    /// when a stop took over first: a stop is not a failure.
    async fn fail_attempt(&self, attempt: u64, failure: StartFailure) {
        let mut state = self.state.write().await;
        if *state == DaemonState::Starting && self.attempt.load(Ordering::SeqCst) == attempt {
            *self.last_failure.write().await = Some(failure);
            *state = DaemonState::Crashed;
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
        // A killed GUI must not leave its daemon behind (Unix; on Windows
        // the Job Object already covers it, and the daemon ignores the flag).
        args.push("--exit-with-parent".into());
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
    ///
    /// A stop ends the eviction early, so the start of `attempt` that runs it
    /// unwinds promptly: a restart waits for exactly that.
    async fn evict_stale_daemon(&self, ours: &str, attempt: u64) {
        let url = self.ws_url();
        let occupant = probe_occupant(&url).await;
        let Some(theirs) = stale_version(&occupant, ours) else {
            return;
        };
        if !self.is_starting(attempt).await {
            return;
        }
        warn!(
            "A v{theirs} daemon is running on {url}, but this app is v{ours} — asking it to shut down so the matching daemon can start"
        );
        if !self.request_graceful_shutdown().await {
            error!("Could not deliver a shutdown request to the v{theirs} daemon on {url}");
            return;
        }
        let deadline = tokio::time::Instant::now() + EVICTION_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if !self.is_starting(attempt).await {
                return;
            }
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

    /// Poll the daemon port until [`boot_wait_verdict`] is final: a daemon
    /// answered, the sidecar exited with nobody answering, or a stop took
    /// over. There is no deadline while the sidecar is alive. A slow boot is
    /// logged at [`SLOW_START_NOTICE`], then at each doubling of the elapsed
    /// time.
    async fn wait_for_ready(&self, attempt: u64, watch: &SpawnWatch) -> BootWait {
        let url = self.ws_url();
        let mut evicted = false;
        let mut next_notice = SLOW_START_NOTICE;
        loop {
            // Read before the probe. An exit seen here means the probe below
            // ran after the exit, so a daemon that was already up when our
            // sidecar deferred to it (the AlreadyRunning exit) still gets
            // attached.
            let exited = watch.exited.load(Ordering::SeqCst);
            let occupant = probe_occupant(&url).await;
            // A daemon from another release can win the port between the
            // pre-spawn check and here. Evict it once; answering is not the
            // same as being ours.
            if stale_version(&occupant, env!("CARGO_PKG_VERSION")).is_some() && !evicted {
                evicted = true;
                self.evict_stale_daemon(env!("CARGO_PKG_VERSION"), attempt).await;
                continue;
            }
            let answered = occupant != PortOccupant::Nobody;
            let state = self.state_of(attempt).await;
            match boot_wait_verdict(state, exited, answered) {
                BootWait::KeepWaiting => {}
                BootWait::Ready if exited => {
                    info!("Attached to an existing daemon instance on {}", url);
                    return BootWait::Ready;
                }
                verdict => return verdict,
            }
            if let Some(elapsed) = self.starting_for().await
                && elapsed >= next_notice
            {
                let pid = self
                    .child
                    .read()
                    .await
                    .as_ref()
                    .map_or_else(
                        || "unknown".to_string(),
                        |sidecar| sidecar.child.pid().to_string(),
                    );
                warn!(
                    "Daemon still starting after {}s (PID {pid}): the process is alive and {url} is not open yet — waiting",
                    elapsed.as_secs()
                );
                next_notice = elapsed * 2;
            }
            sleep(Duration::from_millis(200)).await;
        }
    }
    
    /// Stop the daemon
    ///
    /// # Errors
    ///
    /// Never returns `Err` today: an undeliverable or ignored shutdown request
    /// falls back to a tree-kill, a failed kill is logged, and the manager
    /// always ends `Stopped`.
    pub async fn stop(&self) -> Result<(), String> {
        // Counted before anything else, even when there is nothing to stop:
        // from here on, every start decided on before this stop is refused
        // (see [`Self::start`]), including one that has not reached the state
        // lock yet.
        self.stops.fetch_add(1, Ordering::SeqCst);
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
        if let Some(Sidecar { child, watch }) = child_slot.take() {
            if watch.exited.load(Ordering::SeqCst) {
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
                    && wait_for_terminated(&watch, Duration::from_secs(5)).await;
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

    /// Start health monitoring (call once after start)
    pub fn start_health_monitor(self: Arc<Self>, app: AppHandle) {
        let manager = self;
        let config = manager.config.clone();

        tokio::spawn(async move {
            loop {
                sleep(config.health_check_interval).await;
                // A stop from here on cancels this tick's restart.
                let since_stop = manager.stop_epoch();

                // `Crashed` is watched too: a daemon the app attached to may
                // answer again (flip back to `Running`), or a first boot that
                // failed stays down (restart it).
                let state = *manager.state.read().await;
                if !matches!(state, DaemonState::Running | DaemonState::Crashed) {
                    continue;
                }

                // Health check: try to connect, within the same ceiling a
                // version probe gets for a connect plus a request.
                let url = manager.ws_url();
                match check_health(&url, PROBE_TIMEOUT).await {
                    Ok(()) => {
                        if state == DaemonState::Crashed {
                            // Our sidecar is gone, but a daemon is alive and
                            // answering.
                            manager.recovered(since_stop).await;
                        }
                        debug!("Daemon health check: OK");
                    }
                    Err(e) => {
                        warn!("Daemon health check failed: {}", e);
                        let failure = StartFailure::new(
                            StartFailureKind::HealthCheckFailed,
                            format!("The daemon stopped answering on {url} ({e})"),
                        );
                        let restart_count = *manager.restart_count.read().await;
                        if restart_count >= config.max_restarts {
                            error!("Max daemon restarts exceeded, giving up");
                            manager
                                .give_up_restarts(since_stop, config.max_restarts, failure)
                                .await;
                            continue;
                        }
                        if !manager.health_check_failed(since_stop, failure).await {
                            // A stop or someone else's start took over while
                            // the check ran: the restart is not ours to make.
                            continue;
                        }

                        // Try to restart
                        *manager.restart_count.write().await = restart_count + 1;
                        warn!("Attempting daemon restart ({}/{})", restart_count + 1, config.max_restarts);

                        sleep(config.restart_delay).await;
                        if let Err(e) = manager.start(&app, since_stop).await {
                            error!("Daemon restart failed: {}", e);
                        }
                    }
                }
            }
        });
    }

    /// Whether the health monitor's tick that began at `since_stop` still
    /// owns the state: no stop since, and nobody else started a boot.
    fn monitor_owns(&self, since_stop: u64, state: DaemonState) -> bool {
        self.stops.load(Ordering::SeqCst) == since_stop
            && matches!(state, DaemonState::Running | DaemonState::Crashed)
    }

    /// A daemon answers again after a crash: `Running`, nothing on record.
    async fn recovered(&self, since_stop: u64) {
        let mut state = self.state.write().await;
        if *state == DaemonState::Crashed && self.stops.load(Ordering::SeqCst) == since_stop {
            info!("A daemon answers on {} again", self.ws_url());
            *self.last_failure.write().await = None;
            *state = DaemonState::Running;
        }
    }

    /// Record a failed health check as a crash. `false`, with nothing
    /// changed, when the state moved on while the check ran.
    ///
    /// A daemon that was already down keeps the failure on record: it says
    /// why (a boot that exited, say), and a check that cannot connect to a
    /// daemon that is down adds nothing to it.
    async fn health_check_failed(&self, since_stop: u64, failure: StartFailure) -> bool {
        let mut state = self.state.write().await;
        if !self.monitor_owns(since_stop, *state) {
            return false;
        }
        {
            let mut last = self.last_failure.write().await;
            if *state == DaemonState::Running || last.is_none() {
                *last = Some(failure);
            }
        }
        *state = DaemonState::Crashed;
        true
    }

    /// The monitor is out of restarts: record that once, around the failure
    /// before it (or `failure`, this tick's check, when there was none). The
    /// monitor logs every tick; the record keeps the first time and reason.
    async fn give_up_restarts(&self, since_stop: u64, max_restarts: u32, failure: StartFailure) {
        let mut state = self.state.write().await;
        if !self.monitor_owns(since_stop, *state) {
            return;
        }
        {
            let mut last = self.last_failure.write().await;
            match last.take() {
                Some(previous) if previous.kind == StartFailureKind::RestartsExhausted => {
                    *last = Some(previous);
                }
                previous => {
                    let previous = previous.unwrap_or(failure);
                    *last = Some(StartFailure::restarts_exhausted(max_restarts, &previous));
                }
            }
        }
        *state = DaemonState::Crashed;
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

    /// The 2026-09-18 case: the sidecar is alive and its port is still closed
    /// minutes into the boot. That means wait, not fail. Elapsed time is
    /// deliberately not an input to this function.
    #[test]
    fn a_live_sidecar_with_a_closed_port_is_still_booting() {
        assert_eq!(boot_wait_verdict(DaemonState::Starting, false, false), BootWait::KeepWaiting);
    }

    #[test]
    fn an_answer_on_the_port_is_ready_whether_or_not_our_sidecar_lives() {
        assert_eq!(boot_wait_verdict(DaemonState::Starting, false, true), BootWait::Ready);
        // The AlreadyRunning exit: our sidecar deferred to a daemon that is up.
        assert_eq!(boot_wait_verdict(DaemonState::Crashed, true, true), BootWait::Ready);
    }

    #[test]
    fn only_an_exited_sidecar_with_nobody_answering_is_a_failure() {
        // The exit handler leaves a boot's state to the ready-wait, so it
        // sees the exit while `Starting`; `Crashed` gets the same verdict.
        assert_eq!(boot_wait_verdict(DaemonState::Crashed, true, false), BootWait::Failed);
        assert_eq!(boot_wait_verdict(DaemonState::Starting, true, false), BootWait::Failed);
    }

    /// `stop()` during a boot owns the state it leaves behind. Neither a
    /// late answer nor the sidecar's exit (which the stop itself caused) may
    /// overwrite `Stopped` with `Running` or `Crashed`.
    #[test]
    fn a_stop_during_the_wait_cancels_it() {
        for state in [DaemonState::Stopping, DaemonState::Stopped] {
            for exited in [false, true] {
                for answered in [false, true] {
                    assert_eq!(boot_wait_verdict(state, exited, answered), BootWait::Cancelled);
                }
            }
        }
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

    // -------------------------------------------------------------------------
    // Start attempts, failures, the boot log
    // -------------------------------------------------------------------------

    /// What a sidecar printed when another daemon held the PID file
    /// (captured 2026-09-18 from a real second daemon on one data dir).
    const PID_FILE_TAKEN: [&str; 3] = [
        r#"2026-09-18T17:21:44.208585Z  INFO nanna_daemon::server: Data directory: "/tmp/s/dataA""#,
        "2026-09-18T17:21:44.208739Z ERROR nanna_daemon::server: Another daemon instance is already running (PID: 298915)",
        "2026-09-18T17:21:44.209179Z ERROR nanna_daemon: Error: Already running",
    ];

    /// What a sidecar printed when another program held its port (captured
    /// the same way).
    const PORT_TAKEN: [&str; 3] = [
        "2026-09-18T17:21:44.224718Z ERROR nanna_daemon::server: IPC port 127.0.0.1:51993 unavailable: Address already in use (os error 98) — another daemon (or another program) holds it; exiting before touching storage",
        "2026-09-18T17:21:44.224786Z  INFO nanna_daemon::health: PID file removed",
        "2026-09-18T17:21:44.225193Z ERROR nanna_daemon: Error: IPC error: IPC port 127.0.0.1:51993 unavailable: Address already in use (os error 98)",
    ];

    /// A daemon's first log line as it reached the GUI before the sidecar
    /// got `NO_COLOR` (captured from a real boot).
    const COLOURED: &str = "\u{1b}[2m2026-09-18T17:21:21.595409Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m \u{1b}[2mnanna_daemon\u{1b}[0m\u{1b}[2m:\u{1b}[0m Starting Nanna daemon...\n";

    fn line(stream: BootStream, text: &str) -> BootLine {
        BootLine { stream, line: text.to_string() }
    }

    fn stdout(lines: &[&str]) -> VecDeque<BootLine> {
        lines.iter().map(|text| line(BootStream::Stdout, text)).collect()
    }

    /// A manager for a daemon on `port`.
    fn manager_on(port: u16) -> DaemonManager {
        DaemonManager::new(DaemonManagerConfig { port, ..DaemonManagerConfig::default() })
    }

    /// A port nothing listens on.
    async fn free_port() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    }

    /// Serve a daemon's handshake and version reply on a free port, on every
    /// connection, for as long as the test runs. Returns the port.
    async fn serve_daemon(version: &'static str) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else { return };
                    while let Some(Ok(Message::Text(request))) = ws.next().await {
                        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
                        let reply = serde_json::json!({
                            "id": request["id"],
                            "result": { "status": "success", "data": { "version": version } }
                        });
                        let _ = ws.send(Message::Text(reply.to_string().into())).await;
                    }
                });
            }
        });
        port
    }

    /// Open a start attempt the way `start` does, and return it.
    async fn claimed(manager: &DaemonManager) -> u64 {
        manager
            .claim_start(manager.stop_epoch())
            .await
            .expect("no stop since")
            .expect("nothing under way")
    }

    /// Two callers never both spawn, and a stop wins over every start that
    /// was decided on before it.
    #[tokio::test]
    async fn a_start_is_claimed_once_and_an_older_start_loses_to_a_stop() {
        let manager = manager_on(free_port().await);
        let before_stop = manager.stop_epoch();
        assert_eq!(manager.claim_start(before_stop).await, Ok(Some(1)));
        assert_eq!(
            manager.claim_start(before_stop).await,
            Ok(None),
            "a start under way is not claimed again"
        );
        assert_eq!(manager.state().await, DaemonState::Starting);

        manager.stop().await.unwrap();
        assert_eq!(manager.state().await, DaemonState::Stopped);
        assert!(!manager.is_starting(1).await, "the stop cancels the boot in progress");
        assert_eq!(
            manager.claim_start(before_stop).await,
            Err(START_CANCELLED.to_string()),
            "a start decided on before the stop must not undo it"
        );
        assert_eq!(manager.claim_start(manager.stop_epoch()).await, Ok(Some(2)));
    }

    /// 2026-09-18: why a boot failed was only a log line. The exit is kept,
    /// with the daemon's own reason, until a daemon is ready.
    #[tokio::test]
    async fn a_boot_exit_is_the_last_error_until_a_daemon_is_ready() {
        let manager = manager_on(free_port().await);
        let attempt = claimed(&manager).await;
        let watch = manager.new_spawn_watch().await;
        let events = manager.sidecar_events(attempt, Arc::clone(&watch));
        for text in PID_FILE_TAKEN {
            events.relay(BootStream::Stdout, format!("{text}\n").as_bytes()).await;
        }
        events.on_exit(Some(1), None).await;

        // During a boot the exit decides nothing by itself: the ready-wait
        // does, since a daemon our sidecar deferred to may be answering.
        assert!(watch.exited.load(Ordering::SeqCst));
        assert_eq!(manager.state().await, DaemonState::Starting);
        assert_eq!(manager.last_failure().await, None);

        // The ready-wait found nobody answering: what `start` does next.
        let exit = watch.exit.lock().await.clone().expect("the exit is recorded");
        let failure = StartFailure::from_exit(StartFailureKind::ExitedDuringBoot, &exit);
        manager.fail_attempt(attempt, failure).await;
        assert_eq!(manager.state().await, DaemonState::Crashed);
        let failure = manager.last_failure().await.expect("the failure is recorded");
        assert_eq!(failure.kind, StartFailureKind::ExitedDuringBoot);
        assert_eq!(failure.message, "Error: Already running");
        assert_eq!((failure.exit_code, failure.signal), (Some(1), None));

        // A retry keeps it on show while it boots, and a daemon that answers
        // clears it.
        let retry = claimed(&manager).await;
        assert_eq!(manager.last_failure().await, Some(failure));
        assert!(manager.mark_ready(retry).await);
        assert_eq!(manager.state().await, DaemonState::Running);
        assert_eq!(manager.last_failure().await, None);
    }

    /// The contract the splash reads: `snake_case` kinds, nullable exit fields.
    #[test]
    fn a_failure_serializes_as_the_status_reports_it() {
        let exit = SidecarExit { code: None, signal: Some(9), reason: None };
        let failure = StartFailure::from_exit(StartFailureKind::ExitedAfterReady, &exit);
        let json = serde_json::to_value(&failure).unwrap();
        assert_eq!(json["kind"], "exited_after_ready");
        assert_eq!(json["message"], "The daemon exited (signal 9)");
        assert_eq!(json["exit_code"], serde_json::Value::Null);
        assert_eq!(json["signal"], 9);
        assert!(json["at_ms"].as_i64().is_some_and(|at| at > 0));
        let kinds = [
            (StartFailureKind::SidecarUnresolved, "sidecar_unresolved"),
            (StartFailureKind::SpawnFailed, "spawn_failed"),
            (StartFailureKind::ExitedDuringBoot, "exited_during_boot"),
            (StartFailureKind::HealthCheckFailed, "health_check_failed"),
            (StartFailureKind::RestartsExhausted, "restarts_exhausted"),
        ];
        for (kind, name) in kinds {
            assert_eq!(serde_json::to_value(kind).unwrap(), name);
        }
    }

    #[test]
    fn without_a_reason_the_message_is_the_exit_status() {
        let exit = |code, signal| SidecarExit { code, signal, reason: None };
        let message = |kind, exit| StartFailure::from_exit(kind, &exit).message;
        assert_eq!(
            message(StartFailureKind::ExitedDuringBoot, exit(Some(1), None)),
            "The daemon exited during startup (exit code 1)"
        );
        assert_eq!(message(StartFailureKind::ExitedAfterReady, exit(None, Some(9))), "The daemon exited (signal 9)");
        assert_eq!(message(StartFailureKind::ExitedAfterReady, exit(None, None)), "The daemon exited");
    }

    /// The late `AlreadyRunning` exit (fix (a) of the splash work): the app
    /// attached to a daemon it did not spawn, then its own sidecar deferred.
    /// The state used to read `crashed` while connected, for up to 30 s.
    #[test]
    fn only_an_exit_while_running_with_nobody_answering_is_a_crash() {
        assert_eq!(exit_effect(true, DaemonState::Running, false), ExitEffect::Crash);
        assert_eq!(exit_effect(true, DaemonState::Running, true), ExitEffect::StayAttached);
        for answering in [false, true] {
            assert_eq!(exit_effect(true, DaemonState::Starting, answering), ExitEffect::EndsBoot);
            for state in [DaemonState::Stopping, DaemonState::Stopped, DaemonState::Crashed] {
                assert_eq!(exit_effect(true, state, answering), ExitEffect::Nothing);
            }
        }
        for state in [
            DaemonState::Stopped,
            DaemonState::Starting,
            DaemonState::Running,
            DaemonState::Stopping,
            DaemonState::Crashed,
        ] {
            assert_eq!(
                exit_effect(false, state, false),
                ExitEffect::Nothing,
                "a sidecar from an older attempt owns nothing"
            );
        }
    }

    #[tokio::test]
    async fn our_sidecar_deferring_after_the_attach_leaves_the_app_attached() {
        let manager = manager_on(serve_daemon(env!("CARGO_PKG_VERSION")).await);
        let attempt = claimed(&manager).await;
        let watch = manager.new_spawn_watch().await;
        assert!(manager.mark_ready(attempt).await);

        manager.sidecar_events(attempt, Arc::clone(&watch)).on_exit(Some(1), None).await;

        assert_eq!(manager.state().await, DaemonState::Running);
        assert_eq!(manager.last_failure().await, None);
        assert!(watch.exited.load(Ordering::SeqCst), "stop() must leave that daemon alone");
    }

    #[tokio::test]
    async fn the_serving_sidecar_exiting_is_a_crash_with_its_exit_status() {
        let manager = manager_on(free_port().await);
        let attempt = claimed(&manager).await;
        let watch = manager.new_spawn_watch().await;
        assert!(manager.mark_ready(attempt).await);
        let events = manager.sidecar_events(attempt, Arc::clone(&watch));
        events
            .relay(BootStream::Stdout, b"2026-09-18T17:30:00.000000Z  INFO nanna_core::scheduler: tick\n")
            .await;

        events.on_exit(None, Some(9)).await;

        assert_eq!(manager.state().await, DaemonState::Crashed);
        let failure = manager.last_failure().await.expect("the crash is recorded");
        assert_eq!(failure.kind, StartFailureKind::ExitedAfterReady);
        assert_eq!(failure.message, "The daemon exited (signal 9)");
        assert_eq!((failure.exit_code, failure.signal), (None, Some(9)));
    }

    /// A restart kills the old sidecar and begins the next boot at once. The
    /// old sidecar's exit and late output arrive during that boot and must
    /// not fail it or mix into its log.
    #[tokio::test]
    async fn an_older_sidecar_cannot_touch_the_next_boot() {
        let manager = manager_on(free_port().await);
        let old = claimed(&manager).await;
        let old_events = manager.sidecar_events(old, manager.new_spawn_watch().await);
        manager.stop().await.unwrap();
        let new = claimed(&manager).await;
        let new_watch = manager.new_spawn_watch().await;

        old_events.relay(BootStream::Stdout, b"a late line from the old sidecar\n").await;
        old_events.on_exit(None, Some(9)).await;

        assert!(manager.is_starting(new).await);
        assert_eq!(manager.last_failure().await, None);
        assert!(!new_watch.exited.load(Ordering::SeqCst));
        assert_eq!(manager.boot_log().await, Vec::new());
    }

    #[tokio::test]
    async fn the_boot_log_keeps_the_newest_lines_of_the_latest_spawn() {
        let manager = manager_on(free_port().await);
        assert_eq!(manager.boot_log().await, Vec::new(), "nothing spawned yet");

        let events = manager.sidecar_events(1, manager.new_spawn_watch().await);
        for n in 0..BOOT_LOG_LINES + 3 {
            events.relay(BootStream::Stdout, format!("line {n}\n").as_bytes()).await;
        }
        events.relay(BootStream::Stderr, b"\r\n").await;
        let log = manager.boot_log().await;
        assert_eq!(log.len(), BOOT_LOG_LINES, "bounded, and a blank line takes no slot");
        assert_eq!(log[0].line, "line 3");
        assert_eq!(log[BOOT_LOG_LINES - 1].line, format!("line {}", BOOT_LOG_LINES + 2));

        manager.new_spawn_watch().await;
        assert_eq!(manager.boot_log().await, Vec::new(), "each spawn starts a new log");
    }

    #[tokio::test]
    async fn relayed_lines_are_plain_text_tagged_with_their_stream() {
        let manager = manager_on(free_port().await);
        let events = manager.sidecar_events(1, manager.new_spawn_watch().await);
        events.relay(BootStream::Stdout, COLOURED.as_bytes()).await;
        events
            .relay(BootStream::Stderr, b"thread 'main' panicked at src/main.rs:1:2:\r\n")
            .await;
        assert_eq!(
            manager.boot_log().await,
            [
                line(
                    BootStream::Stdout,
                    "2026-09-18T17:21:21.595409Z  INFO nanna_daemon: Starting Nanna daemon..."
                ),
                line(BootStream::Stderr, "thread 'main' panicked at src/main.rs:1:2:"),
            ]
        );
    }

    #[test]
    fn plain_text_drops_escape_sequences_and_control_characters() {
        assert_eq!(plain_text("a\u{1b}[1;31mred\u{1b}[0m b"), "ared b");
        // OSC, ended by BEL and by the string terminator.
        assert_eq!(plain_text("\u{1b}]0;title\u{7}x\u{1b}]8;;http://a\u{1b}\\y"), "xy");
        // Two-character escapes, and a lone ESC at the end.
        assert_eq!(plain_text("\u{1b}(Bz\u{1b}"), "z");
        // Tabs stay; line endings, BEL and C1 controls go; text is untouched.
        assert_eq!(plain_text("k\tv\u{7}\u{85}é — ok\r\n"), "k\tvé — ok");
    }

    #[test]
    fn an_overlong_line_is_cut_on_a_character_boundary_and_says_so() {
        let whole = "x".repeat(BOOT_LOG_LINE_BYTES);
        assert_eq!(fit_line(whole.clone()), whole);

        // "é" is two bytes, and this one straddles the cut.
        let head = "x".repeat(BOOT_LOG_LINE_BYTES - 1);
        let long = format!("{head}é{}", "y".repeat(10));
        assert_eq!(fit_line(long), format!("{head}… (12 more bytes)"));
    }

    #[test]
    fn the_reason_is_the_daemons_own_last_error_line() {
        assert_eq!(exit_reason(&stdout(&PID_FILE_TAKEN)).as_deref(), Some("Error: Already running"));
        assert_eq!(
            exit_reason(&stdout(&PORT_TAKEN)).as_deref(),
            Some("Error: IPC error: IPC port 127.0.0.1:51993 unavailable: Address already in use (os error 98)")
        );
    }

    #[test]
    fn a_panic_is_reported_by_the_daemons_panic_line_over_stderr() {
        let mut lines = stdout(&[
            "2026-09-18T17:30:00.000000Z ERROR nanna_daemon::server: PANIC: boom location=src/server.rs:1:2",
        ]);
        lines.push_back(line(BootStream::Stderr, "thread 'main' panicked at src/server.rs:1:2:"));
        lines.push_back(line(BootStream::Stderr, "boom"));
        lines.push_back(line(
            BootStream::Stderr,
            "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
        ));
        assert_eq!(
            exit_reason(&lines).as_deref(),
            Some("PANIC: boom location=src/server.rs:1:2")
        );

        // Without the daemon's line, the panic message on stderr, never the
        // hint after it.
        lines.pop_front();
        lines.push_front(line(BootStream::Stdout, "2026-09-18T17:30:00.000000Z  INFO nanna: working"));
        assert_eq!(exit_reason(&lines).as_deref(), Some("boom"));
    }

    #[test]
    fn output_that_ends_in_ordinary_work_gives_no_reason() {
        // A signal mid-work: the exit status is the whole story.
        let mut lines = VecDeque::from([line(BootStream::Stderr, "an early warning")]);
        lines.push_back(line(BootStream::Stdout, "2026-09-18T17:30:00.000000Z  INFO nanna: ERROR is just a word here"));
        assert_eq!(exit_reason(&lines), None);
        assert_eq!(exit_reason(&VecDeque::new()), None);

        // Only stderr: a program that never reached its log.
        let loader = "nanna-daemon: error while loading shared libraries: libx.so: cannot open shared object file";
        assert_eq!(
            exit_reason(&VecDeque::from([line(BootStream::Stderr, loader)])).as_deref(),
            Some(loader)
        );
    }

    #[test]
    fn an_error_line_is_read_with_or_without_its_timestamp() {
        assert_eq!(error_message("ERROR nanna_daemon: Error: x"), Some("Error: x"));
        assert_eq!(error_message("2026-09-18T17:30:00Z ERROR no target here"), Some("no target here"));
        assert_eq!(error_message("2026-09-18T17:30:00Z  WARN nanna: ERROR soon"), None);
        assert_eq!(error_message("2026-09-18T17:30:00Z ERROR"), None);
    }

    // -------------------------------------------------------------------------
    // Health monitor
    // -------------------------------------------------------------------------

    /// A port that accepts TCP but never completes the handshake used to
    /// hang the health monitor for good.
    #[tokio::test]
    async fn a_health_check_that_gets_no_handshake_times_out() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                // Hold the connection open and never answer.
                tokio::spawn(async move {
                    let _stream = stream;
                    std::future::pending::<()>().await;
                });
            }
        });

        let started = std::time::Instant::now();
        let result = check_health(&url, Duration::from_millis(300)).await;
        assert_eq!(result, Err("no answer within 300ms".to_string()));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn a_health_check_passes_against_a_daemon_and_fails_on_a_closed_port() {
        let serving = serve_daemon(env!("CARGO_PKG_VERSION")).await;
        assert_eq!(check_health(&format!("ws://127.0.0.1:{serving}"), PROBE_TIMEOUT).await, Ok(()));
        let closed = free_port().await;
        assert!(check_health(&format!("ws://127.0.0.1:{closed}"), PROBE_TIMEOUT).await.is_err());
    }

    /// A tick whose check outlived a stop must neither overwrite the stop's
    /// state nor restart the daemon behind it.
    #[tokio::test]
    async fn a_health_check_that_outlived_a_stop_changes_nothing() {
        let manager = manager_on(free_port().await);
        let attempt = claimed(&manager).await;
        assert!(manager.mark_ready(attempt).await);
        let tick = manager.stop_epoch();
        manager.stop().await.unwrap();

        let failure = StartFailure::new(StartFailureKind::HealthCheckFailed, "x".to_string());
        assert!(!manager.health_check_failed(tick, failure).await);
        assert_eq!(manager.state().await, DaemonState::Stopped);
        assert_eq!(manager.last_failure().await, None);
        assert!(manager.claim_start(tick).await.is_err(), "its restart is refused");
    }

    #[tokio::test]
    async fn a_failed_check_is_recorded_and_an_answer_clears_it() {
        let manager = manager_on(free_port().await);
        let attempt = claimed(&manager).await;
        assert!(manager.mark_ready(attempt).await);
        let tick = manager.stop_epoch();

        let failure = StartFailure::new(StartFailureKind::HealthCheckFailed, "x".to_string());
        assert!(manager.health_check_failed(tick, failure.clone()).await);
        assert_eq!(manager.state().await, DaemonState::Crashed);
        assert_eq!(manager.last_failure().await, Some(failure));

        manager.recovered(tick).await;
        assert_eq!(manager.state().await, DaemonState::Running);
        assert_eq!(manager.last_failure().await, None);
    }

    /// A boot that failed stays explained while the monitor retries it.
    #[tokio::test]
    async fn a_failed_check_on_a_crashed_daemon_keeps_the_reason_on_record() {
        let manager = manager_on(free_port().await);
        let attempt = claimed(&manager).await;
        let exit = SidecarExit {
            code: Some(1),
            signal: None,
            reason: Some("Error: Already running".to_string()),
        };
        let boot = StartFailure::from_exit(StartFailureKind::ExitedDuringBoot, &exit);
        manager.fail_attempt(attempt, boot.clone()).await;

        let check = StartFailure::new(StartFailureKind::HealthCheckFailed, "x".to_string());
        assert!(manager.health_check_failed(manager.stop_epoch(), check).await, "still a restart");
        assert_eq!(manager.last_failure().await, Some(boot));
    }

    #[tokio::test]
    async fn giving_up_is_recorded_once_with_the_failure_before_it() {
        let manager = manager_on(free_port().await);
        let attempt = claimed(&manager).await;
        let spawn_failed = StartFailure::new(
            StartFailureKind::SpawnFailed,
            "Could not start nanna-daemon: No such file or directory (os error 2)".to_string(),
        );
        manager.fail_attempt(attempt, spawn_failed).await;
        let tick = manager.stop_epoch();
        let check = || StartFailure::new(StartFailureKind::HealthCheckFailed, "check".to_string());

        manager.give_up_restarts(tick, 3, check()).await;
        let gave_up = manager.last_failure().await.expect("recorded");
        assert_eq!(gave_up.kind, StartFailureKind::RestartsExhausted);
        assert_eq!(
            gave_up.message,
            "Stopped restarting the daemon after 3 failed restarts · Could not start nanna-daemon: No such file or directory (os error 2)"
        );

        manager.give_up_restarts(tick, 3, check()).await;
        assert_eq!(manager.last_failure().await, Some(gave_up), "the first record stays");
    }
}
