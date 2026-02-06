# Phase 2: Tools & Channels

**Status:** ✅ Complete

## Tools

### File Operations (read, write, list)
**Location:** `crates/nanna-tools/src/builtin/file.rs`

**Description:**
File system operations for reading, writing, and listing files/directories.

**Current Implementation:**
- `read_file` / `read` - Read file contents with optional line limits
- `write_file` / `write` - Write content to files (creates directories)
- `list_dir` / `glob` - List directory contents with optional recursion

**Suggestions:**
- Add file watching for real-time updates
- Implement file diff/patch operations
- Add binary file support (base64 encoding)
- Consider sandboxing for security (restrict to workspace)

---

### Shell Execution
**Location:** `crates/nanna-tools/src/builtin/shell.rs`

**Description:**
Execute shell commands with timeout and output capture.

**Current Implementation:**
- `exec` / `bash` - Execute shell commands
- Configurable timeout (default 30s)
- Captures stdout/stderr
- Working directory support

**Suggestions:**
- Add command allowlist/blocklist for security
- Implement sudo/elevation handling
- Add shell history for debugging
- Consider PTY support for interactive commands

---

### Web Fetch/Search
**Location:** `crates/nanna-tools/src/builtin/web.rs`

**Description:**
HTTP requests and web search via Brave Search API.

**Current Implementation:**
- `web_fetch` - Fetch and extract readable content from URLs
- `web_search` - Search via Brave Search API
- Content extraction with readability algorithms

**Suggestions:**
- Add support for JavaScript-rendered pages (headless browser)
- Implement response caching with TTL
- Add rate limiting per domain
- Support POST/PUT/DELETE for API interactions

---

### Memory (remember, recall, reflect)
**Location:** `crates/nanna-tools/src/builtin/memory.rs`

**Description:**
Semantic memory operations for storing and retrieving facts.

**Current Implementation:**
- `remember` - Store a fact with embedding
- `recall` - Semantic search for relevant memories
- `reflect` - Store self-observations (meta-learning)
- FSRS feedback on recall (testing effect)
- Duplicate detection with similarity thresholds
- Importance scoring (1-5 scale)
- Fact source tagging (STATED vs OBSERVED)

**Suggestions:**
- Add memory graph visualization
- Implement memory decay notifications
- Add bulk import/export
- Consider memory categories/tags

---

### Scheduling (remind, list, cancel)
**Location:** `crates/nanna-tools/src/builtin/schedule.rs`

**Description:**
Create and manage scheduled reminders and tasks.

**Current Implementation:**
- `schedule_reminder` - Create one-time or recurring reminders
- `list_reminders` - List scheduled items
- `cancel_reminder` - Cancel by ID

**Suggestions:**
- Add natural language parsing ("remind me tomorrow at 3pm")
- Implement snooze functionality
- Add reminder categories
- Support recurring with exceptions (skip holidays)

---

### Browser Tools
**Location:** `crates/nanna-browser/`

**Description:**
Browser automation for screenshots, extraction, and actions.

**Current Implementation:**
- `browser_screenshot` - Capture page screenshots
- `browser_extract` - Extract content from pages
- `browser_action` - Click, type, scroll actions
- `browser_evaluate` - Execute JavaScript

**Suggestions:**
- Add session persistence (cookies, login state)
- Implement element selectors (CSS, XPath)
- Add screenshot comparison for visual testing
- Consider Playwright/Puppeteer protocol support

---

### Vision Tools
**Location:** `crates/nanna-tools/src/builtin/vision.rs`

**Description:**
Image analysis using vision-capable LLMs.

**Current Implementation:**
- `analyze_image` - Describe image contents via LLM
- Support for local files and URLs
- Base64 encoding for API calls

**Suggestions:**
- Add object detection coordinates
- Implement OCR with bounding boxes
- Support video frame extraction
- Add image comparison/diff

---

### OCR Tools
**Location:** `crates/nanna-tools/src/builtin/ocr.rs`

**Description:**
Extract text from images using vision models.

**Current Implementation:**
- Text extraction from images
- Image description generation
- Uses vision LLM for OCR

**Suggestions:**
- Add local OCR option (Tesseract)
- Implement table extraction
- Support handwriting recognition
- Add language detection

---

### Audio Tools (TTS, Transcription)
**Location:** `crates/nanna-tools/src/builtin/audio.rs`

**Description:**
Text-to-speech and speech-to-text capabilities.

**Current Implementation:**
- TTS via OpenAI/ElevenLabs
- Transcription via Whisper
- Audio file handling

**Suggestions:**
- Add local TTS option (Piper/Coqui)
- Implement streaming TTS
- Add voice cloning support
- Support real-time transcription

---

### PDF Tools
**Location:** `crates/nanna-tools/src/builtin/pdf.rs`

**Description:**
Read and extract content from PDF files.

**Current Implementation:**
- `read_pdf` - Extract text from PDFs
- `extract_pdf_images` - Get embedded images
- `analyze_pdf_images` - Describe images via vision

**Suggestions:**
- Add PDF annotation support
- Implement form field extraction
- Support PDF generation
- Add table extraction

---

### Authoring Tools (Runtime Tool Creation)
**Location:** `crates/nanna-scripting/`

**Description:**
Create tools at runtime using JavaScript/TypeScript.

**Current Implementation:**
- **Boa Engine** - Pure Rust JS engine (lightweight)
- **Deno Engine** - V8-based for full TS support
- TypeScript transpilation via `deno_ast`
- Sandboxed execution

**Suggestions:**
- Add tool testing framework
- Implement tool versioning
- Add tool marketplace/sharing
- Consider WASM support for other languages

---

## Channels

### Telegram
**Location:** `crates/nanna-channels/src/telegram.rs`

**Description:**
Full Telegram Bot API integration.

**Current Implementation:**
- Send, react, edit, delete messages
- Pin messages
- Polls
- Long-polling listener
- Webhook support (via daemon)

**Suggestions:**
- Add inline mode support
- Implement keyboard/button menus
- Support media groups
- Add channel posting (not just groups)

---

### Discord
**Location:** `crates/nanna-channels/src/discord.rs`

**Description:**
Discord REST API and Gateway integration.

**Current Implementation:**
- Send, react, edit, delete messages
- Pin messages
- Thread support
- Gateway WebSocket listener
- Slash command support

**Suggestions:**
- Add voice channel support
- Implement embed builder
- Support Discord components (buttons, selects)
- Add server management commands

---

### Slack
**Location:** `crates/nanna-channels/src/slack.rs`

**Description:**
Slack Web API and Socket Mode integration.

**Current Implementation:**
- Send, react, edit, delete messages
- Pin messages
- Thread support
- File uploads
- Socket Mode listener

**Suggestions:**
- Add Slack workflow integration
- Implement Block Kit builder
- Support Slack Connect
- Add app home tab

---

### Signal
**Location:** `crates/nanna-channels/src/signal.rs`

**Description:**
Signal messaging via signald.

**Current Implementation:**
- Send messages
- Reactions
- Group support
- REST API options

**Suggestions:**
- Add attachment support
- Implement group management
- Support disappearing messages
- Add profile management

---

### WhatsApp
**Location:** `crates/nanna-channels/src/whatsapp.rs`

**Description:**
WhatsApp Cloud API integration.

**Current Implementation:**
- Send messages
- Reactions
- Media support
- Template messages
- Web bridge mode

**Suggestions:**
- Add message templates builder
- Implement catalog/product support
- Support status updates
- Add business profile management
