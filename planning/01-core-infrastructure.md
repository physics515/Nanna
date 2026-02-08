# Phase 1: Core Infrastructure

**Status:** ✅ Complete

## Features

### SIMD Vector Operations (AVX/AVX2)
**Location:** `crates/nanna-simd/`

**Description:**
SIMD-accelerated vector operations for fast embedding comparisons. Uses AVX/AVX2 intrinsics on x86_64 for cosine similarity calculations.

**Current Implementation:**
- `cosine_similarity_f32()` - SIMD-accelerated cosine similarity
- `normalize_f32()` - Vector normalization
- Fallback to scalar operations when SIMD unavailable

**Suggestions:**

- Consider adding AVX-512 support for newer CPUs
- Add ARM NEON support for Apple Silicon and mobile
- Benchmark against `simsimd` crate (already a dependency) to see if native Rust SIMD matches performance

**Todo**

* Consider adding AVX-512 support for newer CPUs - plan this out further and implement
* Add ARM NEON support for Apple Silicon and mobile - critical

---

### GPU Compute (wgpu)
**Location:** `crates/nanna-gpu/`

**Description:**
GPU-accelerated vector search using WebGPU (wgpu). Falls back to SIMD when GPU unavailable.

**Current Implementation:**
- `GpuContext` - Manages GPU device and queue
- `CosineSimilaritySearch` - Compute shader pipeline for batch similarity
- Automatic fallback to CPU when GPU unavailable
- Used when vector count > 1000 (threshold in `VectorStore`)

**Suggestions:**
- Consider lowering GPU threshold based on benchmarks (1000 may be conservative)
- Add GPU memory management for very large vector stores
- Implement batched GPU operations to avoid memory limits
- Add compute shader for embedding generation (if using local models) - support for ollama for local embedding

**Todo**

* Consider lowering GPU threshold based on benchmarks (1000 may be conservative) - lets create the benchmark and study this further
* Add GPU memory management for very large vector stores - lets implement
* Implement batched GPU operations to avoid memory limits - approved

---

### SQLite Persistence (Turso)
**Location:** `crates/nanna-storage/`

**Description:**
Persistent storage using Turso. Supports both Turso cloud sync.

**Current Implementation:**
- Session storage
- Conversation history
- Scheduled jobs persistence
- Configuration storage

**Suggestions:**
- Add database migrations system
- Consider WAL mode for better concurrent access
- Add backup/restore functionality
- Document Turso cloud setup for multi-device sync - will be supported be multi-agent swam system using custom tor based agent sync

**Todo**

* Add database migrations system
* Consider WAL mode for better concurrent access
* Add backup/restore functionality
* Remove SQLite support, use turso only

---

### Vector Store + Conversation Memory
**Location:** `crates/nanna-memory/`

**Description:**
In-memory vector store with semantic search and conversation context management.

**Current Implementation:**
- `VectorStore` - SIMD/GPU-accelerated vector search
- `MemoryEntry` - Stores content, embedding, metadata, FSRS state
- `ConversationMemory` - Rolling window of chat messages
- Workspace-scoped memory (global + per-workspace)
- FSRS-6 cognitive decay model

**Suggestions:**
- Add persistent vector index (currently reloads all to memory)
- Consider HNSW or IVF indexing for very large stores
- Add compression for embedding storage (f16 flag exists but verify usage)
- Implement memory compaction/garbage collection

**Todo**

* Add persistent vector index (currently reloads all to memory) - implement with turso database
* Consider HNSW or IVF indexing for very large stores - approved
* Add compression for embedding storage (f16 flag exists but verify usage) - approved add dreaming
* Implement memory compaction/garbage collection - implement in dreaming

---

### LLM Clients (Anthropic, OpenAI, OpenRouter)
**Location:** `crates/nanna-llm/`

**Description:**
Multi-provider LLM client with streaming and tool calling support.

**Current Implementation:**
- Anthropic Claude (primary)
- OpenAI (GPT-4, etc.)
- OpenRouter (proxy to multiple models)
- Ollama (local models)
- Streaming responses
- Tool/function calling

**Suggestions:**
- Add Google Gemini support
- Add Mistral API support
- Implement provider failover/fallback
- Add request caching for identical prompts
- Track token usage per session for cost estimation

**Todo**

* Add Google Gemini support - approved
* Add Mistral API support - approved
* Implement provider failover/fallback - approved
* Add grok support
* investigate github copilot api masking (similar to how we pretend to be claude code)
* Add request caching for identical prompts - approved
* Track token usage per session for cost estimation - high priority

---

### Streaming + Tool Calling
**Location:** `crates/nanna-agent/src/loop_runner.rs`

**Description:**
Agent loop that handles streaming responses and tool execution.

**Current Implementation:**
- `Agent::run()` - Main agent loop
- `StreamCallback` - Real-time token streaming
- `ToolCallRecord` - Tool execution tracking
- Parallel tool execution when tools are independent
- Tool output truncation with budget allocation

**Suggestions:**
- Add tool call caching for idempotent tools
- Implement tool call batching for efficiency
- Add circuit breaker for failing tools
- Consider tool priority/ordering hints

**Todo**

* Add tool call caching for idempotent tools
* Implement tool call batching for efficiency
* Add circuit breaker for failing tools
* Consider tool priority/ordering hints
* rebuild tool authoring tool so nanna can build her own tools
* rebuild manual tool authoring GUI
  * plan for advanced tool authoring features
* Current tools code isn't visible in the GUI
* Build and test a tool

---

### Agent Loop with Context Management
**Location:** `crates/nanna-agent/`

**Description:**
Agentic loop with context window management and summarization.

**Current Implementation:**
- Sliding window truncation
- Message truncation (50KB limit)
- Intelligent tool output truncation
- Context compression via LLM summarization
- Incremental summarization caching
- CDC deduplication for duplicate content

**Suggestions:**
- Add context budget visualization in GUI
- Implement priority-based message retention
- Add user-configurable context strategies
- Consider semantic chunking for long documents

**Todo**

* Add context budget visualization in GUI - High Priority
* Implement priority-based message retention
* Add user-configurable context strategies - plan what this would even look like.
* Consider semantic chunking for long documents

---

### Scheduler (Heartbeats, Cron)
**Location:** `crates/nanna-daemon/src/scheduler/`

**Description:**
Background task scheduling with cron expressions and heartbeat support.

**Current Implementation:**
- Cron expression parsing
- Job persistence in SQLite
- Job types: prompt, tool call, webhook
- Heartbeat intervals
- Job history tracking

**Suggestions:**
- Add timezone support per job
- Implement job dependencies (run B after A)
- Add missed job handling on startup
- Create GUI cron builder with visual schedule

**Todo**

* Add timezone support per job - use the  chrono crate for this support
* Add https://docs.rs/chrono-english/latest/chrono_english/ support for cron jobs
  * eg. "set a timer for 15 mins" and set a timer for 15 minutes
  * eg. "do this task every 3rd Thursday of the month but skip every 5th event" and schedule it correctly
* Implement job dependencies (run B after A)
* Add missed job handling on startup
* Create GUI cron builder with visual schedule - explore what a calendar interface would look like
