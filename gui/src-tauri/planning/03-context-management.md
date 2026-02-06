# 03 — Context Window Management

## Feature Description

Context window management is the system that decides what information fits into the LLM's finite context window. It handles token estimation, conversation history truncation, tool result budget allocation, smart content truncation by type, and optional LLM-based summarization of large tool outputs. This is the gatekeeper between "everything Nanna knows" and "what the model actually sees."

### Token Budget Architecture
```
Model Context Limit (e.g., 200k for Claude)
├── System Reserved: 10,000 tokens (system prompt + memory context)
├── Response Reserved: 8,000 tokens (model's reply)
└── Conversation Budget: 132,000 tokens (history + tool results)
    ├── User/assistant messages (truncated oldest-first)
    └── Tool results (budget allocated proportionally)
```

## Current Implementation

### Constants (lib.rs ~168)
- `TARGET_CONTEXT_TOKENS`: 150,000 — safety margin from 200k
- `SYSTEM_RESERVED_TOKENS`: 10,000
- `RESPONSE_RESERVED_TOKENS`: 8,000
- `MAX_CONVERSATION_TOKENS`: 132,000 (derived)
- `MAX_MESSAGE_CHARS`: 50,000 per message
- `MIN_TOOL_RESULT_CHARS`: 2,000 — floor for tool results

### Token Estimation (lib.rs ~290)
`estimate_tokens(text)`: Simple `text.len() / 4` heuristic. ~4 chars per token.

### Model Context Limits (lib.rs ~261)
`model_context_limit(model)`: Pattern-matches on model ID strings:
- Claude models: 200,000
- GPT-4o/4-turbo: 128,000
- GPT-4: 8,192
- o1/o3: 200,000
- Gemini: 1,000,000
- DeepSeek: 128,000
- Ollama default: 32,768
- Default fallback: 128,000

### Smart Truncation Functions (lib.rs ~295-443)

**`smart_truncate_tool_result(content, tool_name, budget_chars)`**
Routes to type-specific truncation based on tool name:
- `read_file`, `read` → `truncate_code_content` (head + tail, preserves structure)
- `exec`, `bash` → `truncate_command_output` (tail-heavy, 20% head + 80% tail)
- `web_fetch` → `truncate_web_content` (80% head + 20% tail)
- `web_search` → `truncate_search_results` (structure-preserving)
- Everything else → `truncate_generic` (natural break points)

**`truncate_code_content`**: Shows first 40% + last 40% with "[... N lines omitted ...]" marker. Splits on newlines to preserve line boundaries.

**`truncate_command_output`**: 20% head + 80% tail. Rationale: recent output (errors, final results) is more relevant than early output.

**`truncate_web_content`**: 80% head + 20% tail. Rationale: intro/headers contain the main content; tail catches conclusions.

**`truncate_search_results`**: Preserves structure by truncating at natural boundaries.

**`truncate_generic`**: Finds the nearest newline or space before the budget limit.

### Tool Result Budget Allocation (lib.rs ~481-565)

**`ToolResultEntry`**: Tracks each tool result's original size, tool name, and position in conversation.

**`allocate_tool_budgets(entries, total_budget_tokens)`**:
1. Calculate proportional share based on original content size
2. Apply recency bias: most recent tool results get 20% boost
3. Enforce minimum floor (`MIN_TOOL_RESULT_CHARS` = 2,000)
4. Redistribute excess from capped results to uncapped ones
5. Return per-entry character budgets

### Tool Summarization (lib.rs ~566-818)

**`ToolSummarizationConfig`**: model_priority list, ollama_url, threshold (10k chars), config reference.

**`summarize_tool_result(content, tool_name, config)`**:
- Only triggers if content exceeds threshold (10k chars)
- Tries models in priority order
- Uses hierarchical summarization for very large content
- Chunks based on model context window (leaves 3k token overhead)
- Target: 25% compression per chunk level
- Falls back to truncation if all models fail

**`try_summarize_with_model`**: Delegates to `nanna_agent::Summarizer` with `SummarizerConfig`.

**`create_summarization_client`**: Creates a fresh `LlmClient` for the summarization model.

### Request Token Estimation (lib.rs ~819)
`estimate_request_tokens(request)`: Sums tokens across system prompt, all messages (content + tool results), and tool definitions.

### Dynamic Tool Budget (lib.rs ~856)
`calculate_dynamic_tool_budget(request)`: Estimates current request size, subtracts from model limit, returns remaining budget for tool results.

### Context Truncation (lib.rs ~450)
`truncate_context(messages, max_tokens)`: Drops oldest messages first until total fits within budget. Preserves most recent messages.

## Issues & Bugs

### Critical
1. **Hardcoded constants ignore model diversity**: `TARGET_CONTEXT_TOKENS` is 150k, but for Ollama models with 32k context, this means the budget math is wrong — the system will try to fit 132k tokens of conversation into a 32k model. The dynamic budget calculation (`calculate_dynamic_tool_budget`) uses model-specific limits, but `truncate_context` uses the hardcoded `MAX_CONVERSATION_TOKENS`. These two systems are inconsistent.

2. **Token estimation is crude**: `len() / 4` is a rough heuristic. For code-heavy content, the ratio is closer to 3 chars/token. For CJK text, it can be 1-2 chars/token. This can cause context overflow or underutilization of ~20-30%.

### Moderate
3. **No system prompt token tracking**: The 10k reserved for system prompt is a guess. If workspace context + memory context + base prompt exceeds 10k tokens, it silently eats into the conversation budget. Should measure actual system prompt size.

4. **Summarization model creates a new LlmClient per call**: `create_summarization_client` instantiates a fresh client every time. No connection pooling or reuse. For Ollama, this means a new HTTP client per summarization.

5. **Budget allocation doesn't account for message framing**: Token estimation counts content but not the JSON framing, role markers, and tool_use block structure that the API adds. This overhead is ~50-100 tokens per message.

6. **Recency bias is fixed at 20%**: No configuration. The boost is always 20% for the most recent tool result, regardless of context.

7. **`model_context_limit` uses string matching**: New models require code changes. Should pull from model info cache or API metadata when available.

### Minor
8. **Truncation markers consume budget**: "[... 500 lines omitted ...]" takes space but isn't accounted for in the budget calculation.

9. **No caching of token estimates**: The same message content may be estimated multiple times across different functions.

10. **Summarization threshold is hardcoded at 10k chars**: Not configurable. Some users may want aggressive summarization (lower threshold) or minimal (higher).

## Improvement Suggestions

### High Priority
- **Use model-specific budgets everywhere**: Replace hardcoded `MAX_CONVERSATION_TOKENS` with a dynamic calculation based on `model_context_limit(current_model)`. Pass the model ID through the context management pipeline.
- **Measure actual system prompt tokens**: After building the system prompt (with workspace context and memories), estimate its tokens and subtract from the conversation budget dynamically.
- **Better token estimation**: Use `tiktoken-rs` for OpenAI models, or at minimum a model-family-aware multiplier (3.5 for code-heavy, 4 for English, 2 for CJK).

### Medium Priority
- **Pool summarization clients**: Cache `LlmClient` instances by model ID. Reuse across summarization calls.
- **Pull context limits from model info**: When `ModelInfoCache` has data for a model, use it instead of hardcoded pattern matching. Fall back to pattern matching for unknown models.
- **Make summarization threshold configurable**: Add to `ExtendedSettings`.
- **Account for message framing overhead**: Add ~100 tokens per message to estimates.

### Future
- **Adaptive context strategy**: Instead of fixed head/tail ratios, use the LLM to decide what's relevant. "Given this conversation, which parts of this tool result are most relevant?"
- **Semantic compression**: Use embeddings to identify the most semantically relevant portions of tool results relative to the current conversation.
- **Progressive context loading**: Start with a summary of history, expand recent messages in full. Only load full old messages if the model requests them.
- **Context window visualization**: Show the user how their context budget is being spent (system prompt: X%, history: Y%, tool results: Z%).
