# Phase 9: Multi-Device Swarm (Tor P2P)

**Status:** ❌ Not Started — Entirely greenfield. No crates, no code, no dependencies.

## Overview

Every Nanna daemon becomes a node with its own `.onion` address. Nodes can invoke tools on each other — phone camera from desktop, GPU compute from phone, sensors from anywhere. Communication happens over Tor hidden services for zero-configuration networking and built-in encryption.

**Vision:** A personal mesh of devices, all running Nanna, all able to collaborate. Your phone's camera is a tool your desktop can call. Your desktop's GPU is a resource your phone can use. No port forwarding, no DNS, no firewall configuration. Just Tor.

## Architecture

```
┌─────────────────────┐  Tor Hidden Service  ┌─────────────────────┐
│    Phone Daemon     │◄════════════════════►│   Desktop Daemon    │
│     (Android)       │   .onion ↔ .onion    │   (Windows/Linux)   │
│                     │                      │                     │
│  Tools:             │  "Run camera_snap    │  Tools:             │
│  - Camera           │   on your phone"     │  - File system      │
│  - GPS              │◄─────────────────────│  - Browser          │
│  - Notifications    │                      │  - GPU compute      │
│  - Sensors          │                      │  - exec             │
└─────────────────────┘                      └─────────────────────┘
          │                                            │
          └────────────────────┬───────────────────────┘
                               ▼
                 ┌─────────────────────────┐
                 │    Optional Registry    │
                 │  (Rendezvous or DHT)    │
                 └─────────────────────────┘
```

## Current State

**Nothing exists.** No crates, no dependencies, no code references.

The only tangentially related code:
- `ed25519-dalek = "2.1"` in `nanna-server/Cargo.toml` — used for Discord webhook signature verification, not identity
- The daemon already has a WebSocket IPC server (port 5149) and HTTP health server — these patterns can inform the Tor-exposed service design
- `nanna-client` crate provides a WebSocket client pattern that could be adapted for peer connections

## Proposed Crate Structure

```
crates/
├── nanna-identity/       # Cryptographic identity management
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs        # Public API
│       ├── keypair.rs    # Ed25519 keypair generation/storage
│       ├── onion.rs      # Onion address derivation
│       └── fingerprint.rs # Human-readable fingerprints
│
├── nanna-tor/            # Tor integration
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs        # Public API
│       ├── embedded.rs   # Embedded arti Tor client
│       ├── system.rs     # System Tor daemon fallback
│       ├── hidden.rs     # Hidden service publishing
│       └── client.rs     # Outbound requests to .onion addresses
│
├── nanna-mesh/           # Peer-to-peer coordination (NEW — not in roadmap)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── peer.rs       # Peer management, trust model
│       ├── discovery.rs  # Pairing, QR codes, registry
│       ├── protocol.rs   # Remote tool protocol messages
│       ├── relay.rs      # Tool request/response relay
│       └── audit.rs      # Audit logging for remote invocations
```

**Rationale for `nanna-mesh`:** The roadmap describes peer discovery, remote tool protocol, and trust model as separate items, but they're tightly coupled. A single `nanna-mesh` crate that depends on `nanna-identity` and `nanna-tor` keeps the peer-to-peer logic cohesive.

---

## nanna-identity Crate

### Purpose
Persistent cryptographic identity for each daemon. Every Nanna instance has a unique Ed25519 keypair that determines its `.onion` address and serves as its identity for peer authentication.

### Dependencies
```toml
[dependencies]
ed25519-dalek = { version = "2.1", features = ["rand_core"] }
rand = "0.8"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
base64 = "0.22"
sha2 = "0.10"
aes-gcm = "0.10"          # Encrypt secret key at rest
argon2 = "0.5"             # KDF for passphrase-based encryption
zeroize = { version = "1", features = ["derive"] }  # Zero secret key memory
thiserror = "2"
```

### Design

```rust
/// A Nanna node identity
pub struct Identity {
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub keypair: SigningKey,          // Ed25519 (zeroize on drop)
    pub onion_address: String,        // Derived from public key
    pub fingerprint: String,          // Human-readable "A1B2-C3D4-E5F6-G7H8"
}

impl Identity {
    /// Generate a new random identity
    pub fn generate() -> Self;
    
    /// Load from file (decrypting if passphrase-protected)
    pub fn load(path: &Path, passphrase: Option<&str>) -> Result<Self>;
    
    /// Save to file (encrypting with passphrase if provided)
    pub fn save(&self, path: &Path, passphrase: Option<&str>) -> Result<()>;
    
    /// Derive .onion address from public key
    /// v3 onion: base32(pubkey + checksum + version)
    pub fn derive_onion(public_key: &VerifyingKey) -> String;
    
    /// Generate human-readable fingerprint
    /// SHA-256 of public key, formatted as "XXXX-XXXX-XXXX-XXXX"
    pub fn derive_fingerprint(public_key: &VerifyingKey) -> String;
    
    /// Sign a message
    pub fn sign(&self, message: &[u8]) -> Signature;
    
    /// Verify a signature from a peer
    pub fn verify(public_key: &VerifyingKey, message: &[u8], signature: &Signature) -> bool;
    
    /// Export public identity (safe to share)
    pub fn public_info(&self) -> PublicIdentity;
}

/// Public identity info (safe to share with peers)
pub struct PublicIdentity {
    pub onion_address: String,
    pub public_key: VerifyingKey,
    pub fingerprint: String,
}
```

### Storage

Default location: `~/.nanna/identity.json` (or platform-specific config dir)

```json
{
  "version": 1,
  "created_at": "2026-02-01T12:00:00Z",
  "public_key": "base64...",
  "secret_key_encrypted": "base64...",  // AES-256-GCM encrypted
  "encryption_salt": "base64...",        // Argon2 salt
  "onion_address": "abc123xyz789.onion",
  "fingerprint": "A1B2-C3D4-E5F6-G7H8"
}
```

### Issues & Considerations

- **Key rotation:** If the keypair changes, the `.onion` address changes, breaking all peer connections. Need a rotation protocol that announces the new address to known peers before switching.
- **Backup:** If the identity file is lost, the node gets a new address. All peer trust must be re-established. Consider a recovery seed (BIP-39 mnemonic).
- **Multi-device identity:** Should a user have one identity across all devices, or one per device? Per-device is simpler and more secure (compromise of one device doesn't compromise all).

**Recommendation:** One identity per device. A "device group" concept can link them later.

---

## nanna-tor Crate

### Purpose
Tor integration for hidden service publishing and outbound requests to other `.onion` addresses.

### Dependencies
```toml
[dependencies]
arti-client = "0.26"         # Embedded Tor client (pure Rust)
arti-hyper = "0.26"          # Hyper integration for HTTP over Tor
tor-rtcompat = "0.26"        # Runtime compatibility
tor-hsservice = "0.26"       # Hidden service support
tokio = { version = "1", features = ["full"] }
hyper = { version = "1", features = ["full"] }
axum = "0.8"                 # HTTP server for hidden service
tracing = "0.1"
thiserror = "2"

[features]
default = ["embedded"]
embedded = ["arti-client", "tor-hsservice"]
system = []  # Use system Tor daemon via control port
```

### Design

```rust
/// Tor client managing connections and hidden services
pub struct TorClient {
    client: arti_client::TorClient<PreferredRuntime>,
    hidden_service: Option<RunningHiddenService>,
    bootstrap_progress: watch::Sender<f32>,
}

impl TorClient {
    /// Create with embedded Tor (zero-config, ~30s bootstrap)
    pub async fn embedded(data_dir: &Path) -> Result<Self>;
    
    /// Connect to system Tor daemon via control port
    pub async fn system(control_port: u16, auth: Option<&str>) -> Result<Self>;
    
    /// Publish a hidden service exposing a local port
    pub async fn publish_hidden_service(
        &mut self,
        identity: &Identity,
        local_port: u16,
    ) -> Result<String>;  // Returns .onion address
    
    /// Make an HTTP request to a .onion address
    pub async fn request(&self, url: &str) -> Result<Response>;
    
    /// Get bootstrap progress (0.0 to 1.0)
    pub fn bootstrap_progress(&self) -> watch::Receiver<f32>;
    
    /// Shutdown Tor and hidden services
    pub async fn shutdown(&mut self) -> Result<()>;
}
```

### Bootstrap UX

Tor bootstrap takes 15-60 seconds. The GUI needs to show progress:

```
Connecting to Tor network... [████████░░░░░░░░] 52%
```

The `bootstrap_progress` watch channel lets the GUI subscribe to updates.

### Issues & Considerations

- **`arti` maturity:** Arti is the official Rust Tor implementation but hidden service support (`tor-hsservice`) is still experimental. May need to fall back to system Tor for production use.
- **Binary size:** Arti adds significant binary size (~10-20MB). Consider making it a feature flag.
- **Bootstrap time:** 15-60 seconds is a long startup delay. Consider:
  - Caching Tor state between runs (consensus, descriptors)
  - Starting Tor bootstrap in background on daemon startup
  - Allowing the daemon to function without Tor until bootstrap completes
- **Android:** Arti should work on Android but hasn't been widely tested. May need `orbot` integration as fallback.
- **iOS:** Tor is technically allowed on iOS but Apple has historically been hostile. May not be viable.

**Recommendation:** Start with `embedded` feature using arti. Add `system` feature for users who prefer their own Tor instance. Cache Tor state aggressively.

---

## nanna-mesh Crate

### Purpose
Peer-to-peer coordination: discovery, pairing, trust, remote tool invocation, and audit logging.

### Peer Discovery & Pairing

**Manual pairing via QR code or link:**

```
Device A                          Device B
   │                                  │
   │──── Display QR ──────────────────►
   │     (onion + nonce + pubkey)     │
   │                                  │
   │◄─── Connect to A's .onion ──────│
   │     + send PairingRequest       │
   │     (B's pubkey + nonce sig)    │
   │                                  │
   │──── Verify nonce signature ──────│
   │     + send PairingAccept        │
   │     (A's pubkey + peer config)  │
   │                                  │
   └──── Mutual trust established ────┘
```

**Pairing link format:**
```
nanna://pair?onion=abc123.onion&nonce=xyz789&pk=base64...
```

The GUI shows a QR code encoding this link. The other device scans it (or clicks the link on desktop).

### Trust Model

```rust
/// A known peer
pub struct Peer {
    pub name: String,
    pub onion_address: String,
    pub public_key: VerifyingKey,
    pub fingerprint: String,
    pub paired_at: DateTime<Utc>,
    pub last_seen: Option<DateTime<Utc>>,
    pub trust: TrustConfig,
}

/// What a peer is allowed to do
pub struct TrustConfig {
    /// Tools this peer can invoke on us
    pub allowed_tools: ToolPolicy,
    /// Whether to prompt user before executing
    pub require_approval: HashSet<String>,
    /// Rate limit (requests per minute)
    pub rate_limit: Option<u32>,
    /// Whether this peer can see our tool list
    pub discoverable: bool,
}

pub enum ToolPolicy {
    /// All tools allowed
    All,
    /// Only these tools
    Allowlist(HashSet<String>),
    /// All except these
    Blocklist(HashSet<String>),
    /// No tools (connection only, for messaging)
    None,
}
```

**Storage:** `~/.nanna/peers.toml`

```toml
[[peers]]
name = "My Phone"
onion = "abc123.onion"
public_key = "base64..."
fingerprint = "A1B2-C3D4-E5F6-G7H8"
paired_at = "2026-03-01T12:00:00Z"

[peers.trust]
allowed_tools = { type = "allowlist", tools = ["camera_snap", "location_get", "notify"] }
require_approval = ["location_get"]
rate_limit = 10
discoverable = true

[[peers]]
name = "Work Laptop"
onion = "xyz789.onion"
public_key = "base64..."
fingerprint = "I9J0-K1L2-M3N4-O5P6"

[peers.trust]
allowed_tools = { type = "all" }
require_approval = []
rate_limit = 60
discoverable = true
```

### Remote Tool Protocol

JSON messages over HTTP (via Tor hidden service). Each request is signed with the sender's Ed25519 key.

**Request:**
```json
{
  "type": "tool_request",
  "id": "uuid-v4",
  "from": "abc123.onion",
  "to": "xyz789.onion",
  "tool": "camera_snap",
  "input": { "facing": "front", "resolution": "1080p" },
  "timestamp": "2026-03-01T12:00:00Z",
  "signature": "base64..."
}
```

**Response:**
```json
{
  "type": "tool_response",
  "id": "uuid-v4",
  "from": "xyz789.onion",
  "to": "abc123.onion",
  "status": "success",
  "result": { "image": "base64...", "timestamp": "2026-03-01T12:00:01Z" },
  "duration_ms": 1200,
  "signature": "base64..."
}
```

**Error:**
```json
{
  "type": "tool_response",
  "id": "uuid-v4",
  "from": "xyz789.onion",
  "to": "abc123.onion",
  "status": "error",
  "error": { "code": "permission_denied", "message": "Tool 'exec' not in allowlist" },
  "signature": "base64..."
}
```

**Additional message types:**
- `tool_discovery` — Request list of available tools from peer
- `tool_discovery_response` — Peer's tool list (filtered by trust config)
- `ping` / `pong` — Keepalive and latency measurement
- `peer_update` — Notify of identity/capability changes

### Remote Tool Integration

Remote tools appear in the local tool registry with a `remote:` prefix:

```
Local tools:     read_file, write_file, exec, web_search, ...
Remote tools:    remote:phone:camera_snap, remote:phone:location_get, remote:laptop:exec
```

The agent sees all tools (local + remote) and can use them naturally:

> "Take a photo with my phone's front camera and save it to my desktop"

The agent calls `remote:phone:camera_snap` → Nanna routes the request over Tor → phone daemon executes → result returns.

### Audit Logging

Every remote tool invocation is logged:

```rust
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub direction: Direction,          // Inbound or Outbound
    pub peer: String,                  // Peer onion address
    pub tool: String,
    pub input_summary: String,         // Truncated input for privacy
    pub status: AuditStatus,           // Success, Denied, Error, Timeout
    pub duration_ms: Option<u64>,
    pub approval_required: bool,
    pub approved_by: Option<String>,   // "user" or "auto"
}
```

Stored in SQLite. Browsable in GUI. Exportable for compliance.

---

## Claude Code / External Agent Attachment

### Purpose
Connect external AI agents (like Claude Code) to Nanna's tool ecosystem.

### Approach

Nanna already has an MCP server mode (`nanna-server` crate). External agents can connect via MCP to use Nanna's tools. The mesh extension adds:

1. **Remote tool exposure via MCP** — Remote peer tools appear in the MCP tool list
2. **Session handoff** — External agent can take over a Nanna conversation
3. **Capability negotiation** — External agent discovers available tools (local + remote)

### Implementation

The existing MCP server in `nanna-server` already exposes tools via stdio transport. To support Claude Code:

1. Add HTTP/SSE transport to MCP server (currently stdio only)
2. Register remote peer tools in the MCP tool list
3. Add authentication for MCP connections (API key or peer identity)

This is relatively lightweight given the existing MCP infrastructure.

---

## GUI Integration

### Peer Management Page

New page: `pages/peers.vue`

- **Pair new device** — Show QR code, or enter pairing link
- **Peer list** — All paired devices with status (online/offline/last seen)
- **Per-peer settings** — Edit trust config, tool allowlist, rate limits
- **Remote tool browser** — View tools available on each peer
- **Audit log viewer** — Browse remote tool invocations
- **Revoke peer** — Remove trust instantly

### Tor Status Widget

In the sidebar or status bar:
- Tor bootstrap progress
- Connection status (Connected / Bootstrapping / Offline)
- Number of peers online
- Own `.onion` address (copyable)

### Identity Management

In settings:
- View identity fingerprint
- Export public identity (for sharing)
- Backup identity (encrypted export)
- Rotate identity (with peer notification)

---

## Security Considerations

### Threat Model

1. **Network adversary** — Tor provides transport encryption and anonymity. The adversary cannot see which nodes are communicating.
2. **Compromised peer** — A compromised peer can only invoke tools in its allowlist. `require_approval` forces user confirmation for sensitive tools.
3. **Stolen identity** — If a device's identity file is stolen, the attacker can impersonate that node. Mitigation: passphrase encryption, hardware key support (future).
4. **Replay attacks** — Each request includes a timestamp. Reject requests older than 5 minutes.
5. **Man-in-the-middle** — Tor hidden services provide end-to-end encryption. Pairing verifies public keys out-of-band (QR code).

### Mitigations

- **All requests signed** — Ed25519 signatures on every message
- **Timestamp validation** — Reject stale requests (>5 min)
- **Rate limiting** — Per-peer rate limits prevent abuse
- **Audit logging** — All remote invocations logged
- **User approval** — Sensitive tools require explicit user confirmation
- **Allowlists** — Default deny; peers must be explicitly granted tool access
- **Identity encryption** — Secret key encrypted at rest with Argon2 + AES-256-GCM

---

## Implementation Order

1. **nanna-identity** — Keypair generation, onion derivation, persistence, fingerprints
2. **nanna-tor** — Embedded arti, hidden service publishing, outbound requests
3. **nanna-mesh: discovery** — Manual pairing via QR/link, peer storage
4. **nanna-mesh: protocol** — Remote tool request/response messages
5. **nanna-mesh: trust** — Peer allowlists, tool permissions, rate limiting
6. **nanna-mesh: relay** — Wire remote tools into local tool registry
7. **nanna-mesh: audit** — Audit logging for all remote invocations
8. **GUI: peer management** — Pair devices, manage peers, view remote tools
9. **GUI: Tor status** — Bootstrap progress, connection status
10. **Claude Code bridge** — MCP HTTP transport + remote tool exposure

## Estimated Effort

| Component | Estimated Lines | Effort |
|-----------|----------------|--------|
| nanna-identity | ~500 | 1-2 days |
| nanna-tor | ~800 | 3-5 days (arti integration is complex) |
| nanna-mesh | ~1,500 | 5-7 days |
| GUI integration | ~800 | 2-3 days |
| Testing & hardening | — | 3-5 days |
| **Total** | **~3,600** | **~2-3 weeks** |

## Open Questions

1. **Should Tor be required or optional?** If optional, peers on the same LAN could use mDNS + direct WebSocket. Tor adds latency (~500ms-2s per request).
2. **How to handle offline peers?** Queue requests? Fail immediately? Configurable timeout?
3. **Should there be a central registry?** A rendezvous server could help peers find each other without exchanging addresses manually. But it's a single point of failure and reduces privacy.
4. **What about NAT traversal without Tor?** For users who don't want Tor, consider WebRTC or STUN/TURN as alternatives. But this significantly increases complexity.
5. **Mobile battery impact?** Running a Tor hidden service on a phone will consume battery. Consider "sleep mode" where the phone only connects when needed (pull-based via the desktop).

## Dependencies to Add to Workspace

```toml
# In workspace Cargo.toml [workspace.dependencies]
ed25519-dalek = { version = "2.1", features = ["rand_core"] }
arti-client = "0.26"
arti-hyper = "0.26"
tor-rtcompat = "0.26"
tor-hsservice = "0.26"
aes-gcm = "0.10"
argon2 = "0.5"
zeroize = { version = "1", features = ["derive"] }
qrcode = "0.14"           # QR code generation for pairing
image = "0.25"             # QR code rendering
```
