//! Transport layer for MCP communication
//!
//! Supports:
//! - stdio: Spawn a process and communicate via stdin/stdout
//! - HTTP/SSE: Connect to an HTTP server with Server-Sent Events

use crate::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpError, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Transport trait for MCP communication
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a request and wait for a response
    async fn request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse>;
    
    /// Send a notification (no response expected)
    async fn notify(&self, notification: JsonRpcNotification) -> Result<()>;
    
    /// Close the transport
    async fn close(&self) -> Result<()>;
}

// ============================================================================
// Stdio Transport
// ============================================================================

#[cfg(feature = "stdio")]
pub mod stdio {
    use super::{async_trait, Arc, Mutex, JsonRpcResponse, Result, McpError, JsonRpcNotification, Transport, JsonRpcRequest};
    use std::collections::HashMap;
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::{Child, ChildStdin, ChildStdout, Command};
    use tokio::sync::{mpsc, oneshot};
    use tracing::{debug, error, trace, warn};

    /// Stdio transport - spawns a process and communicates via stdin/stdout
    pub struct StdioTransport {
        /// Child process
        child: Arc<Mutex<Child>>,
        /// Stdin writer
        stdin: Arc<Mutex<ChildStdin>>,
        /// Pending requests waiting for responses
        pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
        /// Shutdown signal
        shutdown_tx: mpsc::Sender<()>,
    }

    impl StdioTransport {
        /// Spawn a new process and create a transport
        ///
        /// # Errors
        ///
        /// Returns error if process fails to spawn
        pub async fn spawn(program: &str, args: &[&str]) -> Result<Self> {
            Self::spawn_with_env(program, args, &[]).await
        }

        /// Spawn with environment variables
        ///
        /// # Errors
        ///
        /// Returns error if process fails to spawn
        #[allow(clippy::unused_async)] // Async for API consistency
        pub async fn spawn_with_env(
            program: &str,
            args: &[&str],
            env: &[(&str, &str)],
        ) -> Result<Self> {
            debug!(program, ?args, "Spawning MCP server process");

            let mut cmd = Command::new(program);
            cmd.args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit()) // Let stderr pass through for debugging
                .kill_on_drop(true);

            for (key, value) in env {
                cmd.env(key, value);
            }

            let mut child = cmd.spawn()?;

            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| McpError::Transport("Failed to open stdin".into()))?;

            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| McpError::Transport("Failed to open stdout".into()))?;

            let pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>> =
                Arc::new(Mutex::new(HashMap::new()));
            let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);

            // Spawn reader task
            let pending_clone = pending.clone();
            tokio::spawn(Self::reader_task(stdout, pending_clone, shutdown_rx));

            Ok(Self {
                child: Arc::new(Mutex::new(child)),
                stdin: Arc::new(Mutex::new(stdin)),
                pending,
                shutdown_tx,
            })
        }

        /// Background task that reads responses from stdout
        async fn reader_task(
            stdout: ChildStdout,
            pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
            mut shutdown_rx: mpsc::Receiver<()>,
        ) {
            let mut reader = BufReader::new(stdout).lines();

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        debug!("Reader task shutting down");
                        break;
                    }
                    line = reader.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                trace!(line, "Received from MCP server");
                                
                                // Try to parse as response
                                match serde_json::from_str::<JsonRpcResponse>(&line) {
                                    Ok(response) => {
                                        let id = response.id.to_string();
                                        let mut pending = pending.lock().await;
                                        if let Some(tx) = pending.remove(&id) {
                                            let _ = tx.send(response);
                                        } else {
                                            warn!(id, "Received response for unknown request");
                                        }
                                    }
                                    Err(e) => {
                                        // Might be a notification, try to parse that
                                        match serde_json::from_str::<JsonRpcNotification>(&line) {
                                            Ok(notif) => {
                                                debug!(method = notif.method, "Received notification from server");
                                                // TODO: Handle server notifications (logging, etc.)
                                            }
                                            Err(_) => {
                                                warn!(error = %e, line, "Failed to parse response");
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                debug!("MCP server closed stdout");
                                break;
                            }
                            Err(e) => {
                                error!(error = %e, "Error reading from MCP server");
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    #[async_trait]
    impl Transport for StdioTransport {
        async fn request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
            let id = request.id.to_string();
            
            // Register pending request
            let (tx, rx) = oneshot::channel();
            {
                let mut pending = self.pending.lock().await;
                pending.insert(id.clone(), tx);
            }

            // Send request
            let line = serde_json::to_string(&request)?;
            trace!(line, "Sending to MCP server");
            
            {
                let mut stdin = self.stdin.lock().await;
                stdin.write_all(line.as_bytes()).await?;
                stdin.write_all(b"\n").await?;
                stdin.flush().await?;
            }

            // Wait for response with timeout
            match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
                Ok(Ok(response)) => Ok(response),
                Ok(Err(_)) => Err(McpError::ConnectionClosed),
                Err(_) => {
                    // Clean up pending request
                    let mut pending = self.pending.lock().await;
                    pending.remove(&id);
                    Err(McpError::Timeout)
                }
            }
        }

        async fn notify(&self, notification: JsonRpcNotification) -> Result<()> {
            let line = serde_json::to_string(&notification)?;
            trace!(line, "Sending notification to MCP server");
            
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(line.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await?;
            
            Ok(())
        }

        async fn close(&self) -> Result<()> {
            let _ = self.shutdown_tx.send(()).await;
            
            let mut child = self.child.lock().await;
            let _ = child.kill().await;
            
            Ok(())
        }
    }
}

#[cfg(feature = "stdio")]
pub use stdio::StdioTransport;

// ============================================================================
// HTTP Transport
// ============================================================================

#[cfg(feature = "http")]
pub mod http {
    use super::{async_trait, Arc, Mutex, JsonRpcResponse, Result, McpError, Transport, JsonRpcRequest, JsonRpcNotification};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::{mpsc, oneshot};
    use tracing::{debug, error, trace, warn};

    /// HTTP transport - connects to an MCP server over HTTP with SSE
    pub struct HttpTransport {
        /// Base URL of the server
        base_url: String,
        /// HTTP client
        client: reqwest::Client,
        /// Pending requests
        pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
        /// SSE connection active
        connected: AtomicBool,
        /// Message endpoint (typically /message or from SSE endpoint)
        message_endpoint: Arc<Mutex<Option<String>>>,
        /// Shutdown signal
        shutdown_tx: mpsc::Sender<()>,
    }

    impl HttpTransport {
        /// Connect to an HTTP MCP server
        ///
        /// # Errors
        ///
        /// Returns error if connection fails
        pub async fn connect(base_url: impl Into<String>) -> Result<Self> {
            let base_url = base_url.into();
            debug!(url = %base_url, "Connecting to MCP HTTP server");

            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| McpError::Transport(e.to_string()))?;

            let pending = Arc::new(Mutex::new(HashMap::new()));
            let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);
            let message_endpoint = Arc::new(Mutex::new(None));

            // Start SSE listener
            let transport = Self {
                base_url: base_url.clone(),
                client,
                pending,
                connected: AtomicBool::new(false),
                message_endpoint,
                shutdown_tx,
            };

            // Spawn SSE connection task
            let pending_clone = transport.pending.clone();
            let client_clone = transport.client.clone();
            let base_url_clone = base_url.clone();
            let message_endpoint_clone = transport.message_endpoint.clone();
            let connected_ptr = &raw const transport.connected as usize;
            
            tokio::spawn(async move {
                // Safety: we know the transport outlives this task due to the shutdown channel
                let connected = unsafe { &*(connected_ptr as *const AtomicBool) };
                Self::sse_task(
                    client_clone,
                    base_url_clone,
                    pending_clone,
                    message_endpoint_clone,
                    connected,
                    shutdown_rx,
                ).await;
            });

            // Wait for connection
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            Ok(transport)
        }

        /// SSE listener task
        async fn sse_task(
            client: reqwest::Client,
            base_url: String,
            pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
            message_endpoint: Arc<Mutex<Option<String>>>,
            connected: &AtomicBool,
            mut shutdown_rx: mpsc::Receiver<()>,
        ) {
            let sse_url = format!("{base_url}/sse");
            debug!(url = %sse_url, "Connecting to SSE endpoint");

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        debug!("SSE task shutting down");
                        break;
                    }
                    result = client.get(&sse_url).send() => {
                        match result {
                            Ok(response) => {
                                if !response.status().is_success() {
                                    error!(status = %response.status(), "SSE connection failed");
                                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                    continue;
                                }

                                connected.store(true, Ordering::SeqCst);
                                debug!("SSE connection established");

                                // Process SSE events
                                let mut stream = response.bytes_stream();
                                use futures::StreamExt;
                                
                                let mut buffer = String::new();
                                while let Some(chunk) = stream.next().await {
                                    match chunk {
                                        Ok(bytes) => {
                                            if let Ok(text) = std::str::from_utf8(&bytes) {
                                                buffer.push_str(text);
                                                
                                                // Process complete SSE events
                                                while let Some(pos) = buffer.find("\n\n") {
                                                    let event = buffer[..pos].to_string();
                                                    buffer = buffer[pos + 2..].to_string();
                                                    
                                                    Self::process_sse_event(
                                                        &event,
                                                        &pending,
                                                        &message_endpoint,
                                                    ).await;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            error!(error = %e, "SSE stream error");
                                            break;
                                        }
                                    }
                                }

                                connected.store(false, Ordering::SeqCst);
                                warn!("SSE connection closed, reconnecting...");
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to connect to SSE endpoint");
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            }
                        }
                    }
                }
            }
        }

        /// Process an SSE event
        async fn process_sse_event(
            event: &str,
            pending: &Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
            message_endpoint: &Arc<Mutex<Option<String>>>,
        ) {
            let mut event_type = "message";
            let mut data = String::new();

            for line in event.lines() {
                if let Some(value) = line.strip_prefix("event: ") {
                    event_type = value.trim();
                } else if let Some(value) = line.strip_prefix("data: ") {
                    data = value.to_string();
                }
            }

            match event_type {
                "endpoint" => {
                    // Server is telling us where to POST messages
                    debug!(endpoint = %data, "Received message endpoint");
                    let mut ep = message_endpoint.lock().await;
                    *ep = Some(data);
                }
                "message" => {
                    // JSON-RPC response
                    trace!(data, "Received SSE message");
                    if let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&data) {
                        let id = response.id.to_string();
                        let mut pending = pending.lock().await;
                        if let Some(tx) = pending.remove(&id) {
                            let _ = tx.send(response);
                        }
                    }
                }
                _ => {
                    trace!(event_type, "Unknown SSE event type");
                }
            }
        }
    }

    #[async_trait]
    impl Transport for HttpTransport {
        async fn request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
            let id = request.id.to_string();

            // Get message endpoint
            let endpoint = {
                let ep = self.message_endpoint.lock().await;
                ep.clone().unwrap_or_else(|| format!("{}/message", self.base_url))
            };

            // Register pending request
            let (tx, rx) = oneshot::channel();
            {
                let mut pending = self.pending.lock().await;
                pending.insert(id.clone(), tx);
            }

            // Send request
            trace!(endpoint, "Sending HTTP request");
            let response = self
                .client
                .post(&endpoint)
                .json(&request)
                .send()
                .await
                .map_err(|e| McpError::Transport(e.to_string()))?;

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                return Err(McpError::Transport(format!("{status}: {text}")));
            }

            // For HTTP, we might get the response directly or via SSE
            // Try to get from response body first
            if let Ok(text) = response.text().await
                && !text.is_empty()
                    && let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&text) {
                        // Clean up pending
                        let mut pending = self.pending.lock().await;
                        pending.remove(&id);
                        return Ok(resp);
                    }

            // Wait for response via SSE
            match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
                Ok(Ok(response)) => Ok(response),
                Ok(Err(_)) => Err(McpError::ConnectionClosed),
                Err(_) => {
                    let mut pending = self.pending.lock().await;
                    pending.remove(&id);
                    Err(McpError::Timeout)
                }
            }
        }

        async fn notify(&self, notification: JsonRpcNotification) -> Result<()> {
            let endpoint = {
                let ep = self.message_endpoint.lock().await;
                ep.clone().unwrap_or_else(|| format!("{}/message", self.base_url))
            };

            self.client
                .post(&endpoint)
                .json(&notification)
                .send()
                .await
                .map_err(|e| McpError::Transport(e.to_string()))?;

            Ok(())
        }

        async fn close(&self) -> Result<()> {
            let _ = self.shutdown_tx.send(()).await;
            self.connected.store(false, Ordering::SeqCst);
            Ok(())
        }
    }
}

#[cfg(feature = "http")]
pub use http::HttpTransport;
