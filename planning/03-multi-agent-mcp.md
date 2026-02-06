# Phase 3: Multi-Agent & MCP

**Status:** ✅ Complete

## MCP Client (External Tool Servers)
**Location:** `crates/nanna-mcp/`

**Description:**
Model Context Protocol client for connecting to external tool servers. Allows Nanna to use tools exposed by other MCP-compatible servers.

**Current Implementation:**
- JSON-RPC 2.0 protocol types
- **Stdio transport** - Spawn processes and communicate via stdin/stdout
- **HTTP/SSE transport** - Connect to HTTP-based MCP servers
- Tool discovery - List available tools from server
- Tool execution - Call tools and receive results
- Resource support - Access server-provided resources
- Prompt support - Use server-defined prompts
- Adapter for nanna-tools integration

**Code Structure:**
```
nanna-mcp/
├── src/
│   ├── lib.rs          # Module exports
│   ├── protocol.rs     # JSON-RPC types
│   ├── transport/      # Stdio, HTTP, SSE
│   ├── client.rs       # MCP client
│   └── adapter.rs      # Bridge to nanna-tools
```

**Suggestions:**
- Add connection pooling for multiple servers
- Implement health checks for connected servers
- Add server capability negotiation
- Support MCP sampling (model requests from server)
- Add automatic reconnection with backoff
- Consider WebSocket transport for bidirectional streaming

---

## MCP Server Mode (Expose Nanna Tools)
**Location:** `crates/nanna-server/src/mcp.rs`

**Description:**
Expose Nanna's tools as an MCP server, allowing external agents (like Claude Code) to use Nanna's capabilities.

**Current Implementation:**
- Stdio transport server
- Tool registration from nanna-tools
- Resource registration
- Prompt registration
- Bridge from MCP protocol to nanna-tools

**Suggestions:**
- Add HTTP server mode for remote access
- Implement tool filtering (expose subset of tools)
- Add authentication for remote connections
- Support streaming tool results
- Add rate limiting per client

---

## Background Task Spawning
**Location:** `crates/nanna-agent/src/multi.rs`

**Description:**
Spawn background tasks that run independently of the main conversation.

**Current Implementation:**
- `AgentCoordinator::spawn_task()` - Spawn async background task
- `BackgroundTask` - Task with ID, status, result
- `TaskStatus` - Pending, Running, Completed, Failed
- Task cancellation support

**Code Example:**
```rust
let coordinator = AgentCoordinator::new();
let task_id = coordinator.spawn_task("research", async {
    // Long-running research task
    Ok("Research complete".to_string())
}).await;
```

**Suggestions:**
- Add task priority levels
- Implement task dependencies (DAG execution)
- Add task progress reporting
- Support task pause/resume
- Add resource limits per task (memory, CPU time)
- Implement task queuing with concurrency limits

---

## Agent-to-Agent Communication
**Location:** `crates/nanna-agent/src/multi.rs`

**Description:**
Message passing between agents for coordination and delegation.

**Current Implementation:**
- `send_message()` - Send message to another agent
- `check_mailbox()` - Receive pending messages
- `AgentMessage` - Message with sender, recipient, content
- In-memory message queue

**Suggestions:**
- Add message acknowledgment
- Implement message persistence (survive restarts)
- Add message expiration/TTL
- Support broadcast messages (one-to-many)
- Add message priority
- Implement request/response pattern with correlation IDs

---

## Supervisor Patterns
**Location:** `crates/nanna-agent/src/supervisor.rs`

**Description:**
Erlang/OTP-inspired supervision for agent lifecycle management.

**Current Implementation:**
- **RestartPolicy**:
  - `Never` - Don't restart on failure
  - `Always` - Always restart
  - `OnFailure` - Restart only on failure
  - `ExponentialBackoff` - Restart with increasing delays
- **HealthCheckConfig**:
  - Interval and timeout settings
  - Healthy/unhealthy thresholds
  - Probe prompt for health assessment
- **SupervisionStrategy**:
  - `OneForOne` - Restart only failed agent
  - `OneForAll` - Restart all if one fails
  - `RestForOne` - Restart failed and all started after it
- **Supervisor**:
  - Lifecycle management (start, stop, restart)
  - Health monitoring
  - Event emission

**Code Example:**
```rust
let supervisor = Supervisor::new(SupervisionStrategy::OneForOne);

supervisor.add_agent(SupervisedAgentConfig {
    id: "researcher".to_string(),
    restart_policy: RestartPolicy::ExponentialBackoff {
        initial_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(60),
        multiplier: 2.0,
    },
    health_check: Some(HealthCheckConfig {
        interval: Duration::from_secs(30),
        timeout: Duration::from_secs(10),
        healthy_threshold: 2,
        unhealthy_threshold: 3,
        probe_prompt: Some("Are you operational?".to_string()),
    }),
    ..Default::default()
}).await;
```

**Suggestions:**
- Add supervisor hierarchies (supervisors of supervisors)
- Implement circuit breaker pattern
- Add resource quotas per supervised agent
- Support graceful shutdown with drain timeout
- Add supervision tree visualization
- Implement agent migration between supervisors
- Add crash dump/debugging information

---

## Potential Improvements

### Error Handling
- Standardize error types across multi-agent system
- Add error categorization (transient vs permanent)
- Implement error aggregation for swarm failures

### Observability
- Add tracing spans for agent interactions
- Implement metrics for:
  - Message latency
  - Task completion time
  - Agent health scores
  - Restart frequency
- Add distributed tracing for cross-agent requests

### Testing
- Add integration tests for multi-agent scenarios
- Implement chaos testing (random failures)
- Add performance benchmarks for message passing

### Documentation
- Document supervision strategies with examples
- Add architecture diagrams
- Create troubleshooting guide for common issues
