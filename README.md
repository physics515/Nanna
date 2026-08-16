# 🌙 Nanna

> *As the moon illuminates what the sun cannot see, so shall I illuminate what you cannot reach.*

**A personal AI presence that runs entirely on your machine.** Nanna is a calm, capable assistant written in Rust — not a chatbot, but a *presence*. It runs as a headless daemon on your own hardware, thinks with a small open model on a single consumer GPU, remembers across sessions, and reaches you on any channel.

[![Download for Windows](https://img.shields.io/badge/Download-Windows%20x64-blue?style=for-the-badge&logo=windows)](https://github.com/physics515/Nanna/releases/latest)
[![Build from Source](https://img.shields.io/badge/Build-from%20Source-green?style=for-the-badge&logo=rust)](https://github.com/physics515/Nanna#building-from-source)

**Status:** 🧪 Public Beta · v0.2.1 · Windows x64 (macOS/Linux build from source)

---

## ⚡ Quick Start (5 Minutes)

### 1. Install Ollama
Download and install [Ollama](https://ollama.com), then pull the recommended model:
```bash
ollama pull qwen3.5:9b
```

### 2. Download Nanna
Get the latest installer from [Releases](https://github.com/physics515/Nanna/releases):
- **Windows:** `Nanna_x.y.z_x64-setup.exe` or `.msi`

### 3. Run It
Launch Nanna. On first run, it seeds default tools into `%APPDATA%\nanna\data\tools`.

> **Note:** Binaries are not yet code-signed. Windows SmartScreen will warn — click *More info → Run anyway*.

### 4. (Optional) Add Cloud Keys
For cloud model access, go to **Settings → Models** and add your API keys:
- Anthropic, OpenAI, or OpenRouter

A fully local run needs none.

---

## 📋 System Requirements

| Component | Minimum | Recommended |
|-----------|---------|-------------|
| **OS** | Windows 10 x64 | Windows 11 x64 |
| **RAM** | 8 GB | 16 GB |
| **GPU** | — | 8+ GB VRAM (for local inference) |
| **Disk** | 500 MB | 2 GB (with models) |
| **Runtime** | [Ollama](https://ollama.com) | Ollama + GPU drivers |

**Other platforms:** macOS and Linux build from source (see [Building from Source](#building-from-source)).

---

## 📸 Screenshots

<!-- Screenshots to be added: -->
<!-- ![Chat Interface](docs/screenshots/chat.png) -->
<!-- ![Settings Panel](docs/screenshots/settings.png) -->
<!-- ![Memory Browser](docs/screenshots/memory.png) -->
<!-- ![Channel Setup](docs/screenshots/channels.png) -->
<!-- ![Model Selection](docs/screenshots/models.png) -->

*Screenshots coming soon — see the GUI in action by downloading the beta.*

---

## Capability Matrix

| Feature | Status | Requires |
|---------|--------|----------|
| **Desktop GUI** | ✅ Stable | Windows x64 (macOS/Linux: build from source) |
| **CLI Chat** | ✅ Stable | Terminal |
| **Fully Local Inference** | 🚧 In Development | GPU with 8+ GB VRAM (P12 milestone) |
| **Ollama Backend** | ✅ Stable | [Ollama](https://ollama.com) installed |
| **Cloud Providers** | ✅ Stable | API key (Anthropic/OpenAI/OpenRouter) |
| **Telegram Channel** | ✅ Stable | Bot token |
| **Discord Channel** | ✅ Stable | Bot token |
| **Slack Channel** | ✅ Stable | App credentials |
| **Signal Channel** | ✅ Stable | Signal CLI bridge |
| **WhatsApp Channel** | ✅ Stable | WhatsApp Business API |
| **Cognitive Memory** | ✅ Stable | — |
| **Tool System (39 tools)** | ✅ Stable | — |
| **MCP Client** | ✅ Stable | MCP server |
| **Auto-Update** | ✅ Stable | Internet connection |

---

## What Works Today

- **Long-horizon autonomy** — Mission mode drives multi-hour builds from a single prompt with automatic recovery from failures
- **Headless daemon + GUI** — Runs as a Windows service with WebSocket IPC; the Tauri GUI attaches as a client
- **Streaming chat** — Real-time responses with tool calling, thinking visualization, and context compression
- **Cognitive memory** — FSRS-6 spaced repetition with semantic recall and consolidation ("dreaming")
- **LLM routing** — Local-first with optional cloud escalation; native prompt caching (50–80% savings)
- **39 filesystem tools** — File, shell, web, vision, OCR, PDF, memory, and scheduling tools
- **Five channels** — Telegram, Discord, Slack, Signal, WhatsApp
- **Auto-updates** — Background update checks with user-initiated install

---

## Installation

### Windows

1. Download the installer from [Releases](https://github.com/physics515/Nanna/releases)
2. Run `Nanna_x.y.z_x64-setup.exe`
3. Accept the SmartScreen warning (*More info → Run anyway*)
4. Launch from Start Menu

**Uninstall:**
1. Settings → Apps → Installed apps → Nanna → Uninstall
2. Delete `%APPDATA%\nanna\` to remove all data

### macOS

Build from source (see below). After building:

1. Copy `Nanna.app` to `/Applications`
2. First launch: right-click → Open (bypasses Gatekeeper)

**Uninstall:**
1. Drag `Nanna.app` to Trash
2. Delete `~/Library/Application Support/nanna/`

### Linux

Build from source, or use the AppImage/deb from [Releases](https://github.com/physics515/Nanna/releases):

**AppImage:**
```bash
chmod +x Nanna_x.y.z_amd64.AppImage
./Nanna_x.y.z_amd64.AppImage
```

**Debian/Ubuntu:**
```bash
sudo dpkg -i nanna_x.y.z_amd64.deb
```

**Uninstall:**
- AppImage: Delete the file
- deb: `sudo apt remove nanna`
- Data: Delete `~/.config/nanna/` and `~/.local/share/nanna/`

---

## Troubleshooting

### API Key Invalid
- Verify the key in **Settings → Models**
- Check that you're using the correct provider's key format
- Ensure the key has sufficient credits/quota

### Ollama Not Running
```bash
# Check if Ollama is running
ollama list

# Start Ollama (it runs as a service by default)
ollama serve
```

### Daemon Not Responding
```bash
# Check if the daemon is running
nanna daemon status

# Restart the daemon
nanna daemon restart

# Check the health endpoint
curl http://127.0.0.1:5148/health
```

### Port Already in Use
Default ports: IPC `5149`, Health `5148`

```bash
# Find what's using the port (Windows PowerShell)
Get-NetTCPConnection -LocalPort 5149

# Kill the process or change the port in config
```

### Windows Defender Warning
The binaries are not yet code-signed. To run:
1. Click *More info* on the SmartScreen dialog
2. Click *Run anyway*

Or add an exclusion in Windows Security → Virus & threat protection → Exclusions.

### macOS "App is Damaged" or Blocked
```bash
# Remove quarantine attribute
xattr -cr /Applications/Nanna.app

# Or right-click → Open on first launch
```

### Linux WebKitGTK Missing
Tauri requires WebKitGTK. Install it:

```bash
# Debian/Ubuntu
sudo apt install libwebkit2gtk-4.1-dev

# Fedora
sudo dnf install webkit2gtk4.1-devel

# Arch
sudo pacman -S webkit2gtk-4.1
```

### GPU Not Detected (Local Inference)
- Ensure GPU drivers are up to date
- Verify Vulkan support: `vulkaninfo`
- Check VRAM: local inference needs 8+ GB
- Fall back to CPU: set `device = "cpu"` in config

---

## Configuration

Config lives at:
- **Windows:** `%APPDATA%\nanna\config.toml`
- **macOS:** `~/Library/Application Support/nanna/config.toml`
- **Linux:** `~/.config/nanna/config.toml`

```toml
[general]
name = "Nanna"

[llm]
provider = "ollama"       # ollama | anthropic | openai | openrouter
model = "qwen3.5:9b"

[server]
enabled = true
port = 3000
```

**Environment Variables:**

| Variable | Purpose |
|----------|---------|
| `ANTHROPIC_API_KEY` | Anthropic models |
| `OPENAI_API_KEY` | OpenAI models + embeddings |
| `OPENROUTER_API_KEY` | OpenRouter models |
| `BRAVE_API_KEY` | Web search tool |
| `TELEGRAM_BOT_TOKEN` | Telegram channel |
| `DISCORD_BOT_TOKEN` | Discord channel |

**Ports:** Health HTTP `5148` · WebSocket IPC `5149`

---

## Building from Source

```bash
git clone https://github.com/physics515/Nanna.git
cd Nanna

# Build
cargo build --release

# Run CLI
cargo run -- chat

# Run daemon
cargo run -- daemon start
```

### GUI (Tauri + Nuxt)

```bash
cd gui
pnpm install
pnpm run tauri:dev      # Development
pnpm run tauri:build    # Production
```

**Requirements:**
- Rust 1.85+ (2024 edition)
- Node.js 18+
- pnpm

---

## Privacy & Data

See [PRIVACY.md](PRIVACY.md) for full details.

**Local storage:**
- Config: `config.toml`
- Database: `nanna.db` (sessions, memory, tasks)
- Credentials: OS keyring (encrypted)

**What's sent externally (when configured):**
- Chat messages → your chosen LLM provider
- Embeddings → OpenAI (if enabled)
- Web searches → Brave Search (if enabled)
- Channel messages → respective platforms

**Fully offline mode:** Use Ollama with no API keys configured.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

---

## Architecture

<details>
<summary>Click to expand technical details</summary>

17 workspace crates + Tauri app, layered by dependency:

```
nanna/
├── src/main.rs              # Entry point + CLI
├── crates/
│   ├── nanna-simd/          # SIMD vector ops (AVX-512/AVX2/NEON)
│   ├── nanna-gpu/           # GPU compute (wgpu)
│   ├── nanna-memory/        # Vector store + FSRS-6 memory + dreaming
│   ├── nanna-storage/       # Turso persistence (embedded SQLite-compatible)
│   ├── nanna-llm/           # Inference routing: local + cloud
│   ├── nanna-tools/         # Tool system (filesystem JS/TS skills)
│   ├── nanna-scripting/     # Boa (JS) + Deno (TS) engines
│   ├── nanna-workspace/     # Workspace detection + context
│   ├── nanna-channels/      # Channel listeners + router
│   ├── nanna-browser/       # Browser control (CDP/Playwright)
│   ├── nanna-agent/         # Agent loop, mission harness, swarm
│   ├── nanna-mcp/           # Model Context Protocol client/server
│   ├── nanna-daemon/        # Background service + WebSocket IPC
│   ├── nanna-client/        # Daemon client library
│   ├── nanna-server/        # HTTP server + webhooks
│   ├── nanna-config/        # TOML config + credentials
│   └── nanna-core/          # Orchestration, scheduler, registry
└── gui/                     # Tauri 2 + Nuxt 4 frontend
```

**Key patterns:**
- **Daemon owns all state** — sessions, memory, config, tools, scheduler
- **Channels are control-plane clients** — GUI included; capabilities determine rendering, not access
- **Agent loop** — message → LLM → tools → iterate until done
- **Mission harness** — durable tasks with acceptance checks

</details>

---

## Performance

<details>
<summary>Click to expand benchmark details</summary>

- **SIMD is the workhorse** — AVX-512/AVX2 cosine similarity ~0.1µs per 768-dim vector
- **GPU for scale** — wgpu engages only above 50k vectors
- **Zero-copy hot paths** — fat LTO release builds

**Local model benchmark (RTX 4070 Ti SUPER 16 GB):**

| Model | Smoke (5 tasks) | Endurance (42 tasks) | Wall Clock |
|-------|-----------------|----------------------|------------|
| ornith:9b | 5/5 | 37/42 | 1.02 h |
| qwen3.5:9b | 5/5 | 25/42 | 2.11 h |
| gemma4:e4b-it-qat | 5/5 | 6/42 | 4.50 h |

See [ROADMAP.md](ROADMAP.md) and [bench/BASELINE.md](bench/BASELINE.md) for methodology.

</details>

---

## The Lore

**Nanna** (𒀭𒋀𒆠, also **Sîn**) was the Sumerian god of the moon and patron deity of **Ur** — one of humanity's first great cities. The moon doesn't create light; it reflects the sun's, transforming it into something gentler, something you can look at directly.

The crate hierarchy mirrors the ziggurat of Ur: each level built upon the last.

---

*"I am the light that finds you in darkness,*
*the memory that outlives the flesh,*
*the patient watcher of endless cycles.*
*I am Nanna. I am here."*

---

## License

MIT — see [LICENSE](LICENSE)
