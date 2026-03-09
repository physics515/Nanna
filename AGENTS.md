# AGENTS.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

## Project Overview

Nanna is a high-performance AI assistant written in Rust. It's a modular system with SIMD/GPU acceleration, persistent memory, and multi-channel communication support.

## Build & Development Commands

```powershell
# Build
cargo build                      # Debug build
cargo build --release            # Release build (fat LTO, stripped)
cargo build -p nanna-agent       # Build single crate

# Test
cargo test                       # Run all tests
cargo test -p nanna-core         # Test single crate
cargo test test_name             # Run specific test

# Lint & Check
cargo check                      # Type check only (fast)
cargo clippy                     # Lint (pedantic + nursery enabled)

# Run
cargo run -- chat                # Interactive CLI
cargo run -- server              # HTTP server mode
cargo run -- daemon start        # Background daemon
```

### GUI (Tauri + Nuxt)

```powershell
cd gui
pnpm install                     # Install dependencies
pnpm run tauri:dev               # Development with hot reload
pnpm run tauri:build             # Production build
pnpm run build                   # Build Nuxt only
pnpm exec vue-tsc                # Type check Vue
```

## Architecture

### Crate Hierarchy (bottom-up dependencies)

```
nanna-simd         # SIMD vector ops (AVX/AVX2) - foundation
nanna-gpu          # GPU compute (wgpu/Vulkan/DX12/Metal)
    ↓
nanna-memory       # Vector store, FSRS-6 memory, consolidation/dreaming
nanna-storage      # SQLite persistence (Turso)
nanna-llm          # LLM clients (Anthropic, OpenAI, OpenRouter, OAuth)
    ↓
nanna-tools        # Tool system - exec, files, web, memory, scheduling
nanna-workspace    # Workspace detection, context files (AGENTS.md, SOUL.md)
nanna-channels     # Message routing (Telegram, Discord, Slack, Signal)
    ↓
nanna-agent        # Agentic loop, tool calling, multi-agent coordination
nanna-mcp          # Model Context Protocol client/server
nanna-scripting    # JavaScript tools (Boa engine)
    ↓
nanna-daemon       # Background service with WebSocket IPC
nanna-client       # Daemon client library
nanna-server       # HTTP server, webhooks
nanna-config       # TOML config, Claude OAuth credentials
    ↓
nanna-core         # Main orchestration, scheduler, workspace registry
```

### Key Patterns

**Agent Loop** (`nanna-agent/src/loop_runner.rs`): Processes messages → calls LLM → executes tools → iterates until complete or max iterations.

**Tool Registry** (`nanna-tools/src/registry.rs`): Tools implement `Tool` trait with `definition()` and `execute()`. Register via `ToolRegistry::register()`.

**Workspace Context** (`nanna-workspace/`): Detects `.nanna/` marker or context files (AGENTS.md, SOUL.md, etc.) to inject project-specific system prompts.

**Memory System** (`nanna-memory/`): Uses FSRS-6 spaced repetition for cognitive memory. Dreaming service consolidates memories during idle time.

## Environment Variables

```
ANTHROPIC_API_KEY    # Required for Anthropic models
OPENAI_API_KEY       # For OpenAI + embeddings (enables semantic search)
OPENROUTER_API_KEY   # For OpenRouter
BRAVE_API_KEY        # Enables web_search tool
TELEGRAM_BOT_TOKEN   # Telegram channel
DISCORD_BOT_TOKEN    # Discord channel
```

## Configuration

Default config path: `~/.config/nanna/config.toml` (or `%APPDATA%\nanna\` on Windows)

## Code Style

- All crates use `#![warn(clippy::all, clippy::pedantic, clippy::nursery)]`
- Rust 2024 edition, requires rustc 1.85+
- Async code uses `tokio` runtime
- Error handling: `thiserror` for library errors, `anyhow` for application errors
- GUI uses Vue 3 Composition API with `<script setup>` and Tailwind CSS

## Testing

Tests use `#[tokio::test]` for async. Many tests skip GPU/network by checking for API keys:
```rust
#[tokio::test]
async fn test_something() {
    let config = NannaConfig { enable_gpu: false, ..Default::default() };
    // ...
}
```

## Daemon Ports

- Health HTTP: `5148` (endpoints: `/health`, `/healthz`, `/readyz`, `/status`)
- WebSocket IPC: `5149` (`ws://127.0.0.1:5149/ws`)
