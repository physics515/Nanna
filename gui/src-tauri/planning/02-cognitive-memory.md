# 02 — Cognitive Memory (FSRS-6)

## Feature Description

Nanna uses FSRS-6 (Free Spaced Repetition Scheduler, version 6) for cognitive memory — a spaced repetition algorithm that models memory strength and retrievability over time. Memories are stored with semantic embeddings for vector search, tagged with importance scores, and organized by workspace scope. The system automatically extracts facts from conversations, consolidates related memories ("dreaming"), and strengthens memories on recall (the testing effect).

### Memory Lifecycle
1. **Extraction**: LLM analyzes conversations → extracts facts with importance (1-5) and type (stated/observed)
2. **Storage**: Facts stored with embeddings, FSRS scheduling parameters, workspace scope, tags
3. **Recall**: Semantic similarity search → returns relevant memories → FSRS records a "review" (testing effect)
4. **Consolidation**: Periodic clustering of similar memories → merge duplicates, expand insights
5. **Decay**: FSRS models forgetting curves — memories have retrievability scores that decrease over time

### Memory States (FSRS)
- **Active**: High retrievability, recently reviewed
- **Dormant**: Moderate retrievability, due for review
- **Silent**: Low retrievability, fading
- **Unavailable**: Very low retrievability, effectively forgotten (but still stored)

## Current Implementation

### MemoryServiceAdapter (lib.rs ~190)
Bridges `nanna_tools::MemoryStorage` trait to `nanna_memory::MemoryService`:
- `store()` → creates memory with tags, no workspace scope (global)
- `search()` → semantic search with limit, maps to `MemoryResult`
- `delete()` → removes by ID
- `list()` → lists recent memories with limit

### Memory Extraction (lib.rs ~1523)
`extract_and_store_memories` with `ExtractionConfig`:
- Uses a separate LLM call with structured prompt to extract facts
- Prompt asks for JSON array of `{fact, importance, fact_type, tags}`
- `fact_type`: "stated" (user said explicitly) or "observed" (model inferred)
- Importance: 1-5 scale for FSRS initial difficulty
- Extracted memories stored with workspace scope from session
- Runs as background `tokio::spawn` — fire and forget
- Uses `extraction_model` if configured, otherwise falls back to chat model

### Memory Recall in Chat (lib.rs ~1233, within send_message)
- Queries memory service with the user's message as search query
- Scoping: workspace sessions get global + workspace memories; global sessions get all
- Results injected into system prompt as "Recalled memories:" section
- FSRS testing effect applied on recall (strengthens retrieved memories)

### Consolidation (lib.rs ~4256)
`trigger_consolidation`:
- Delegates to `nanna_core::consolidation::consolidate_memories`
- Hierarchical clustering of similar memories
- Merges near-duplicates, expands related memories
- Uses configured summarization model priority
- Returns stats: merged count, expanded count, new memories created

### Persistence (lib.rs ~4340)
- `save_memories`: Serializes to JSON file at `memory_path`
- `apply_memory_updates`: Applies batch updates from consolidation results
- Auto-saves after extraction
- Loaded on startup from JSON file

### CRUD Commands
- `list_memories`: Paginated list with optional scope filter
- `get_memory`: Single memory by ID
- `update_memory`: Update content and/or tags
- `delete_memory`: Remove by ID
- `clear_all_memories`: Wipe all memories
- `get_cognitive_memory_stats`: Distribution of FSRS states (active/dormant/silent/unavailable counts)
- `get_memory_stats`: Basic stats (total count, workspace breakdown)

### Embedding Configuration
- Providers: OpenAI (text-embedding-3-small/large), Ollama (nomic-embed-text, mxbai-embed-large, etc.)
- Dimension auto-detection from model info cache
- Configurable via `set_embedding_config` command
- When disabled, memory recall returns empty results (graceful degradation)

## Issues & Bugs

### Critical
1. **No embedding dimension migration**: Changing embedding providers/models changes the vector dimension. Existing memories have embeddings in the old dimension. The system warns about incompatibility but doesn't migrate — old memories become unsearchable. Need either re-embedding on provider change or dimension-aware search.

2. **Memory extraction prompt injection**: The extraction prompt includes raw conversation content. A malicious user message could manipulate the extraction LLM into storing arbitrary "memories" or ignoring real facts. No sanitization of conversation content before extraction.

3. **JSON persistence is not crash-safe**: `save_memories` writes directly to the file. A crash mid-write corrupts the memory store. Should use write-to-temp-then-rename (atomic write) pattern.

### Moderate
4. **MemoryServiceAdapter ignores workspace scope**: The `store()` method creates memories with no workspace scope (global). Tools like `remember` always create global memories, even when the user is in a workspace session. The workspace scope is only set during extraction.

5. **No deduplication on extraction**: The extraction pipeline doesn't check if a fact already exists before storing. Repeated conversations about the same topic create duplicate memories. Consolidation eventually merges them, but this is wasteful.

6. **Consolidation blocks on LLM calls**: `trigger_consolidation` is a synchronous Tauri command that awaits the full consolidation process. For large memory stores, this can take minutes. Should run in background with progress events.

7. **Similarity threshold is global**: One threshold for all queries. Different contexts might benefit from different thresholds (e.g., strict for fact lookup, loose for creative association).

8. **No memory importance decay**: FSRS handles retrievability decay, but importance scores are static. A fact that was importance-5 a year ago might be irrelevant now.

### Minor
9. **Memory stats race condition**: `get_cognitive_memory_stats` and `get_memory_stats` make separate calls to the memory service. State can change between calls.

10. **No memory export/import**: Memories are stored in a proprietary JSON format with no export to human-readable formats or import from other systems.

11. **Extraction model fallback**: If the extraction model fails, the error is logged but no fallback to the chat model occurs within the extraction function itself.

## Improvement Suggestions

### High Priority
- **Atomic file writes**: Use `tempfile` crate → write to temp → `fs::rename` to target. Prevents corruption.
- **Re-embed on provider change**: When embedding config changes, queue a background task to re-embed all memories with the new model. Show progress in UI.
- **Add workspace scope to tool-created memories**: Pass workspace context through to `MemoryServiceAdapter::store()`.
- **Deduplication before storage**: Before storing an extracted fact, do a similarity search. If a near-duplicate exists (>0.9 similarity), update the existing memory's FSRS schedule instead of creating a new one.

### Medium Priority
- **Background consolidation**: Run consolidation as a spawned task with progress events. Add a `consolidation-progress` event stream.
- **Per-query threshold override**: Allow `search_memory` to accept an optional threshold parameter.
- **Memory export**: Add `export_memories` command that produces a Markdown or JSON file suitable for backup/transfer.
- **Extraction filtering**: Skip extraction for very short messages (<50 chars) or messages that are purely tool-call responses.

### Future
- **Memory graphs**: Instead of flat list, model relationships between memories (e.g., "Justin prefers Rust" relates to "Justin is a developer"). Enable graph-based recall.
- **Emotional valence**: Tag memories with emotional context (positive/negative/neutral) for more nuanced recall.
- **Memory narratives**: Periodically generate a narrative summary of all memories about a topic, replacing individual fragments with a coherent story.
- **Active forgetting**: Allow users to mark memories as "forget" — actively suppress rather than just delete, so the extraction pipeline doesn't re-extract them.
