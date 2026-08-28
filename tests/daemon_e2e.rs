// End-to-end daemon lifecycle tests
// Covers: start -> connect -> conversation -> persistence -> fallback -> reconnect

use std::time::Duration;
use tokio::time::sleep;

#[cfg(test)]
mod daemon_e2e {
    use super::*;
    use nanna_daemon::{Daemon, DaemonConfig};
    use nanna_client::{Client, ClientConfig};
    use serde_json::json;

    const TEST_PORT: u16 = 5149;
    const HEALTH_PORT: u16 = 5148;
    const TEST_TIMEOUT: Duration = Duration::from_secs(30);

    /// Test daemon startup and health check
    #[tokio::test]
    async fn test_daemon_startup_health() {
        let config = DaemonConfig {
            port: TEST_PORT,
            health_port: HEALTH_PORT,
            ..Default::default()
        };

        let daemon = Daemon::new(config).await.unwrap();
        
        // Health check should succeed immediately after start
        let response = reqwest::get(format!("http://127.0.0.1:{}/health", HEALTH_PORT))
            .await
            .expect("Failed to connect to health endpoint");
        
        assert!(response.status().is_success());
        daemon.stop().await.unwrap();
    }

    /// Test client connection to daemon
    #[tokio::test]
    async fn test_client_connection() {
        let config = DaemonConfig {
            port: TEST_PORT,
            health_port: HEALTH_PORT,
            ..Default::default()
        };

        let daemon = Daemon::new(config).await.unwrap();
        
        let client_config = ClientConfig {
            url: format!("ws://127.0.0.1:{}/ipc", TEST_PORT),
            ..Default::default()
        };

        let client = Client::new(client_config).await.unwrap();
        
        // Test basic ping
        let response = client.ping().await.expect("Ping failed");
        assert_eq!(response, "pong");
        
        client.close().await.unwrap();
        daemon.stop().await.unwrap();
    }

    /// Test conversation flow
    #[tokio::test]
    async fn test_conversation_flow() {
        let config = DaemonConfig {
            port: TEST_PORT,
            health_port: HEALTH_PORT,
            ..Default::default()
        };

        let daemon = Daemon::new(config).await.unwrap();
        
        let client_config = ClientConfig {
            url: format!("ws://127.0.0.1:{}/ipc", TEST_PORT),
            ..Default::default()
        };

        let client = Client::new(client_config).await.unwrap();
        
        // Test chat message
        let response = client.chat("Hello, Nanna".to_string()).await.expect("Chat failed");
        assert!(response.contains("Hello"));
        
        // Test tool invocation
        let response = client.execute(json!({
            "name": "file_read",
            "arguments": {"path": "README.md"}
        })).await.expect("Tool execution failed");
        assert!(response["success"].as_bool().unwrap());
        
        client.close().await.unwrap();
        daemon.stop().await.unwrap();
    }

    /// Test message persistence across restarts
    #[tokio::test]
    async fn test_message_persistence() {
        let config = DaemonConfig {
            port: TEST_PORT,
            health_port: HEALTH_PORT,
            ..Default::default()
        };

        // Start daemon and create conversation
        let daemon1 = Daemon::new(config.clone()).await.unwrap();
        let client_config = ClientConfig {
            url: format!("ws://127.0.0.1:{}/ipc", TEST_PORT),
            ..Default::default()
        };
        
        let client = Client::new(client_config).await.unwrap();
        let _response = client.chat("Test persistence message".to_string()).await.unwrap();
        client.close().await.unwrap();
        
        // Stop and restart daemon
        daemon1.stop().await.unwrap();
        sleep(Duration::from_secs(2)).await;
        
        let daemon2 = Daemon::new(config).await.unwrap();
        let client2 = Client::new(client_config).await.unwrap();
        
        // Previous messages should be retrievable
        let response = client2.chat("What did we talk about?".to_string()).await.unwrap();
        assert!(response.contains("persistence"));
        
        client2.close().await.unwrap();
        daemon2.stop().await.unwrap();
    }

    /// Test fallback to backup model/provider
    #[tokio::test]
    async fn test_model_fallback() {
        let config = DaemonConfig {
            port: TEST_PORT,
            health_port: HEALTH_PORT,
            primary_model: "nonexistent_model".to_string(),
            fallback_model: "qwen3.5:9b".to_string(),
            ..Default::default()
        };

        let daemon = Daemon::new(config).await.unwrap();
        
        let client_config = ClientConfig {
            url: format!("ws://127.0.0.1:{}/ipc", TEST_PORT),
            ..Default::default()
        };

        let client = Client::new(client_config).await.unwrap();
        
        // Primary model fails, should fallback automatically
        let response = client.chat("Test fallback".to_string()).await.expect("Fallback failed");
        assert!(response.contains("fallback"));
        
        client.close().await.unwrap();
        daemon.stop().await.unwrap();
    }

    /// Test automatic reconnection after disconnect
    #[tokio::test]
    async fn test_auto_reconnect() {
        let config = DaemonConfig {
            port: TEST_PORT,
            health_port: HEALTH_PORT,
            ..Default::default()
        };

        let daemon = Daemon::new(config).await.unwrap();
        
        let client_config = ClientConfig {
            url: format!("ws://127.0.0.1:{}/ipc", TEST_PORT),
            reconnect_interval: Duration::from_secs(5),
            max_retries: 3,
            ..Default::default()
        };

        let client = Client::new(client_config).await.unwrap();
        
        // Establish connection and send message
        let _response = client.chat("Before disconnect".to_string()).await.unwrap();
        
        // Simulate disconnect by stopping daemon
        daemon.stop().await.unwrap();
        sleep(Duration::from_secs(7)).await; // Wait for reconnect
        
        // Client should auto-reconnect
        let response = client.chat("After reconnect".to_string()).await.expect("Reconnect failed");
        assert!(response.contains("reconnect"));
        
        client.close().await.unwrap();
    }

    /// Test full lifecycle: start -> connect -> chat -> disconnect -> restart -> recover
    #[tokio::test]
    async fn test_full_lifecycle() {
        let config = DaemonConfig {
            port: TEST_PORT,
            health_port: HEALTH_PORT,
            ..Default::default()
        };

        // Phase 1: Start daemon
        let daemon = Daemon::new(config.clone()).await.unwrap();
        
        // Phase 2: Connect and chat
        let client_config = ClientConfig {
            url: format!("ws://127.0.0.1:{}/ipc", TEST_PORT),
            ..Default::default()
        };
        let client = Client::new(client_config).await.unwrap();
        
        let response1 = client.chat("Phase 1: Initial message".to_string()).await.unwrap();
        assert!(response1.contains("Phase 1"));
        
        // Phase 3: Disconnect and restart
        daemon.stop().await.unwrap();
        sleep(Duration::from_secs(2)).await;
        
        let daemon2 = Daemon::new(config).await.unwrap();
        
        // Phase 4: Recover connection
        let client2 = Client::new(client_config).await.unwrap();
        let response2 = client2.chat("Phase 2: After restart".to_string()).await.unwrap();
        assert!(response2.contains("Phase 2"));
        
        client2.close().await.unwrap();
        daemon2.stop().await.unwrap();
    }

    /// Test health endpoint resilience
    #[tokio::test]
    async fn test_health_resilience() {
        let config = DaemonConfig {
            port: TEST_PORT,
            health_port: HEALTH_PORT,
            ..Default::default()
        };

        let daemon = Daemon::new(config).await.unwrap();
        
        // Health should be available immediately
        for _ in 0..5 {
            let response = reqwest::get(format!("http://127.0.0.1:{}/health", HEALTH_PORT))
                .await
                .expect("Health check failed");
            assert!(response.status().is_success());
            sleep(Duration::from_millis(100)).await;
        }
        
        daemon.stop().await.unwrap();
    }

    /// Test concurrent connections
    #[tokio::test]
    async fn test_concurrent_connections() {
        let config = DaemonConfig {
            port: TEST_PORT,
            health_port: HEALTH_PORT,
            ..Default::default()
        };

        let daemon = Daemon::new(config).await.unwrap();
        
        let client_config = ClientConfig {
            url: format!("ws://127.0.0.1:{}/ipc", TEST_PORT),
            ..Default::default()
        };

        // Create multiple concurrent clients
        let mut handles = vec![];
        for i in 0..5 {
            let cfg = client_config.clone();
            let handle = tokio::spawn(async move {
                let client = Client::new(cfg).await.unwrap();
                let response = client.chat(format!("Client {} message", i)).await.unwrap();
                assert!(response.contains(&format!("Client {}", i)));
                client.close().await.unwrap();
            });
            handles.push(handle);
        }

        // Wait for all to complete
        for handle in handles {
            handle.await.expect("Client task failed").unwrap();
        }
        
        daemon.stop().await.unwrap();
    }
}
