# Nanna Roadmap

Derived from codebase analysis. Each item has a corresponding planning file.

## Core Systems
1. **Chat & Agent Loop** — Streaming, tool execution, model fallback (`01-chat-agent-loop.md`)
2. **Cognitive Memory (FSRS-6)** — Semantic recall, extraction, consolidation (`02-cognitive-memory.md`)
3. **Context Window Management** — Token budgeting, truncation, summarization (`03-context-management.md`)
4. **Multi-Provider LLM** — Model selection, rate limiting, OAuth (`04-llm-providers.md`)
5. **Tool System** — Built-in tools, user authoring, skills (`05-tool-system.md`)
6. **Workspace System** — Multi-project context, scoped sessions/memory (`06-workspaces.md`)
7. **Daemon/Backend Architecture** — Daemon vs embedded, WebSocket protocol (`07-daemon-backend.md`)

## Automation & Integration
8. **Scheduler & Cron** — Heartbeats, consolidation, custom jobs (`08-scheduler-cron.md`)
9. **Channel Integration** — Telegram, Discord, Slack, Signal, WhatsApp (`09-channels.md`)
10. **Agent Registry** — Multi-agent visualization, lifecycle events (`10-agent-registry.md`)

## Infrastructure
11. **Configuration & Settings** — Provider keys, export/import, OAuth (`11-configuration.md`)
12. **Notifications & System Tray** — Desktop notifications, tray behavior (`12-notifications-tray.md`)
13. **Code Architecture** — lib.rs decomposition, error handling, testing (`13-code-architecture.md`)
