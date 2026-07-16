//! End-to-end daemon tests: start a real daemon, attach a real client over the
//! WebSocket IPC, and drive it the way the GUI/CLI do.
//!
//! This is the P8 "daemon/embedded/reconnect story is untested" gap. Everything that
//! follows exercises the actual `DaemonServer` and the actual `Client` — no mocks, no
//! in-process shortcuts around the protocol.
//!
//! The daemon under test is deliberately **hermetic**: built through `DaemonBuilder`
//! with explicit settings rather than `from_nanna_config`, so a run never reads the
//! developer's `config.toml`, never touches their `.db`, and never needs an API key or
//! a reachable model. `with_memory(false)` keeps embeddings out of it entirely — these
//! tests are about the IPC/session/persistence path, and an LLM would make them
//! non-hermetic and slow.
//!
//! This test lives in `nanna-client` rather than `nanna-daemon` because the client
//! already depends on the daemon (for the shared protocol); putting it here keeps the
//! dependency edge pointing one way.

use nanna_client::{Client, ClientConfig};
use nanna_daemon::server::DaemonBuilder;
use std::time::Duration;

/// Claim a free TCP port from the OS, then release it.
///
/// There is an unavoidable race between releasing and the daemon binding, but asking
/// the OS beats hardcoding a port that collides with a developer's real daemon (5149)
/// or with a parallel test.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("read back the bound port").port()
}

/// A daemon running on its own port and data dir for the duration of one test.
struct TestDaemon {
    port: u16,
    _data_dir: tempfile::TempDir,
    handle: tokio::task::JoinHandle<()>,
}

impl TestDaemon {
    /// Boot a daemon and wait until its IPC port actually accepts connections.
    ///
    /// Reusing a `data_dir` across calls is how the restart test proves persistence
    /// survives a full process lifecycle.
    async fn start(data_dir: tempfile::TempDir) -> Self {
        let port = free_port();
        let dir_path = data_dir.path().to_path_buf();

        let handle = tokio::spawn(async move {
            let mut server = DaemonBuilder::new()
                .with_host("127.0.0.1")
                .with_port(port)
                .with_data_dir(dir_path)
                // Keep the test hermetic and fast: no embeddings, no health/webhook
                // ports to collide on, no PID file to fight a real local daemon over.
                .with_memory(false)
                .with_health_server(false)
                .with_webhook_server(false)
                .with_pid_file(false)
                .with_log_level("warn")
                .build()
                .await;
            // A daemon that fails to run makes `wait_until_ready` time out with a
            // clear message; there is nothing useful to unwrap to here.
            let _ = server.run().await;
        });

        let daemon = Self { port, _data_dir: data_dir, handle };
        daemon.wait_until_ready().await;
        daemon
    }

    fn url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }

    /// Poll the IPC port until it accepts a TCP connection.
    ///
    /// Bounded: boot is subsecond in practice, so 10s means something is broken and the
    /// test should say so rather than hang the suite.
    async fn wait_until_ready(&self) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if tokio::net::TcpStream::connect(("127.0.0.1", self.port)).await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("daemon did not start listening on port {} within 10s", self.port);
    }

    /// Connect a client with reconnect disabled.
    ///
    /// The reconnect test drives reconnection explicitly; leaving the automatic
    /// machinery on would make it unclear which path a passing assertion exercised.
    async fn connect_client(&self) -> Client {
        Client::connect(ClientConfig {
            url: self.url(),
            auto_reconnect: false,
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            ..Default::default()
        })
        .await
        .expect("client connects to the running daemon")
    }

    /// Stop the daemon, releasing the port.
    fn stop(self) -> tempfile::TempDir {
        self.handle.abort();
        self._data_dir
    }
}

/// Pull the session id out of a `sessions.create` response without assuming which
/// envelope shape it arrived in.
fn session_id_of(value: &serde_json::Value) -> String {
    value
        .get("session_id")
        .or_else(|| value.get("id"))
        .or_else(|| value.get("session").and_then(|s| s.get("id")))
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("no session id in create response: {value}"))
        .to_string()
}

fn response_mentions(value: &serde_json::Value, needle: &str) -> bool {
    value.to_string().contains(needle)
}

/// The baseline the whole control plane rests on: a real daemon accepts a real client
/// and answers a request over the protocol.
#[tokio::test]
async fn daemon_starts_and_client_connects() {
    let daemon = TestDaemon::start(tempfile::tempdir().expect("temp dir")).await;
    let client = daemon.connect_client().await;

    assert!(client.is_connected().await, "client reports a live connection");

    let sessions = client.sessions().list().await.expect("sessions.list answers");
    assert!(sessions.is_object() || sessions.is_array(), "got a structured response: {sessions}");

    client.disconnect().await;
    daemon.stop();
}

/// A session created through the client is visible to a subsequent request — the
/// daemon is the one that owns session state, per "channels as control-plane clients".
#[tokio::test]
async fn created_session_is_visible_to_the_client() {
    let daemon = TestDaemon::start(tempfile::tempdir().expect("temp dir")).await;
    let client = daemon.connect_client().await;

    let created = client
        .sessions()
        .create(Some("e2e-session".to_string()))
        .await
        .expect("sessions.create succeeds");
    let session_id = session_id_of(&created);
    assert!(!session_id.is_empty(), "a created session has an id");

    let listed = client.sessions().list().await.expect("sessions.list succeeds");
    assert!(
        response_mentions(&listed, &session_id),
        "the created session appears in the list: {listed}"
    );

    client.disconnect().await;
    daemon.stop();
}

/// The reconnection half of the P8 gap: a client that drops and attaches again must
/// find the daemon's state intact, because the daemon — not the client — owns it.
#[tokio::test]
async fn state_survives_a_client_reconnect() {
    let daemon = TestDaemon::start(tempfile::tempdir().expect("temp dir")).await;

    let first = daemon.connect_client().await;
    let created = first
        .sessions()
        .create(Some("survives-reconnect".to_string()))
        .await
        .expect("sessions.create succeeds");
    let session_id = session_id_of(&created);
    first.disconnect().await;
    assert!(!first.is_connected().await, "the first client is really gone");

    // A fresh client, not a resumed one — this is the GUI's reattach path.
    let second = daemon.connect_client().await;
    let listed = second.sessions().list().await.expect("the daemon still answers");
    assert!(
        response_mentions(&listed, &session_id),
        "the session created before the disconnect is still there: {listed}"
    );

    second.disconnect().await;
    daemon.stop();
}

/// The persistence half: state must outlive the daemon *process*, not just a client.
/// Restarting on the same data dir has to bring the session back — this is what makes
/// the daemon a durable control plane rather than a cache.
#[tokio::test]
async fn sessions_persist_across_a_daemon_restart() {
    let daemon = TestDaemon::start(tempfile::tempdir().expect("temp dir")).await;
    let client = daemon.connect_client().await;

    let created = client
        .sessions()
        .create(Some("survives-restart".to_string()))
        .await
        .expect("sessions.create succeeds");
    let session_id = session_id_of(&created);

    client.disconnect().await;
    // Keep the data dir alive across the restart; dropping it would delete the store.
    let data_dir = daemon.stop();

    let restarted = TestDaemon::start(data_dir).await;
    let client = restarted.connect_client().await;
    let listed = client.sessions().list().await.expect("the restarted daemon answers");
    assert!(
        response_mentions(&listed, &session_id),
        "the session survived a full daemon restart: {listed}"
    );

    client.disconnect().await;
    restarted.stop();
}
