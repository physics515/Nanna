//! Context budgeting and tool-result truncation.

#[allow(clippy::wildcard_imports)]
use crate::*;

// =============================================================================
// Context Management Constants
// =============================================================================

/// Reserved tokens for system prompt, memory context, workspace context
pub(crate) const SYSTEM_RESERVED_TOKENS: usize = 10_000;

/// Reserved tokens for model response
pub(crate) const RESPONSE_RESERVED_TOKENS: usize = 8_000;

/// Maximum characters per individual message before truncation
pub(crate) const MAX_MESSAGE_CHARS: usize = 50_000;

/// Minimum tool result chars (never truncate below this)
pub(crate) const MIN_TOOL_RESULT_CHARS: usize = 2_000;

// =============================================================================
// Intelligent Context Budget Allocation
// =============================================================================

/// Conversation-token budget for history truncation from a resolved model.
///
/// Mirrors `ModelInfo::hard_input_limit` then reserves system + response headroom.
/// Replaces the historical hardcoded `MAX_CONVERSATION_TOKENS` (132k).
pub(crate) fn conversation_token_budget_for(info: &nanna_llm::ModelInfo) -> usize {
    info.conversation_history_budget(
        SYSTEM_RESERVED_TOKENS,
        RESPONSE_RESERVED_TOKENS,
        2_000,
    )
}

/// Rough estimate: ~4 characters per token
pub(crate) fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Smart truncation for tool results based on content type
pub(crate) fn smart_truncate_tool_result(content: &str, tool_name: &str, budget_chars: usize) -> String {
    if content.len() <= budget_chars {
        return content.to_string();
    }

    // Apply tool-specific truncation strategies
    match tool_name {
        "read_file" => truncate_code_content(content, budget_chars),
        "exec" => truncate_command_output(content, budget_chars),
        "web_fetch" => truncate_web_content(content, budget_chars),
        "web_search" | "web_search_batch" => truncate_search_results(content, budget_chars),
        _ => truncate_generic(content, budget_chars),
    }
}

/// Truncate code/file content: keep head + tail with middle omitted
pub(crate) fn truncate_code_content(content: &str, budget_chars: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    // For small files, just do generic truncation
    if total_lines <= 200 || content.len() <= budget_chars * 2 {
        return truncate_generic(content, budget_chars);
    }

    // Calculate how many lines we can show (rough estimate)
    let avg_line_len = content.len() / total_lines;
    let target_lines = budget_chars / avg_line_len.max(1);
    let head_lines = target_lines * 2 / 3; // 2/3 for head
    let tail_lines = target_lines / 3;      // 1/3 for tail

    let head_lines = head_lines.min(total_lines / 2).max(20);
    let tail_lines = tail_lines.min(total_lines / 2).max(10);

    if head_lines + tail_lines >= total_lines {
        return truncate_generic(content, budget_chars);
    }

    let head = lines[..head_lines].join("\n");
    let tail = lines[total_lines - tail_lines..].join("\n");
    let omitted = total_lines - head_lines - tail_lines;

    format!(
        "{}\n\n... [{} lines omitted - showing first {} and last {} of {} total lines] ...\n\n{}",
        head, omitted, head_lines, tail_lines, total_lines, tail
    )
}

/// Truncate command output: keep recent output (tail) which is usually most relevant
pub(crate) fn truncate_command_output(content: &str, budget_chars: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    if content.len() <= budget_chars {
        return content.to_string();
    }

    // For command output, tail is usually more important
    let avg_line_len = content.len() / total_lines.max(1);
    let target_lines = budget_chars / avg_line_len.max(1);
    let head_lines = target_lines / 4;      // 1/4 for head (context)
    let tail_lines = target_lines * 3 / 4;  // 3/4 for tail (results)

    let head_lines = head_lines.min(50).max(5);
    let tail_lines = tail_lines.min(total_lines - head_lines).max(20);

    if head_lines + tail_lines >= total_lines {
        return truncate_generic(content, budget_chars);
    }

    let head = lines[..head_lines].join("\n");
    let tail = lines[total_lines - tail_lines..].join("\n");
    let omitted = total_lines - head_lines - tail_lines;

    format!(
        "{}\n\n... [{} lines of output omitted] ...\n\n{}",
        head, omitted, tail
    )
}

/// Truncate web content: extract key sections
pub(crate) fn truncate_web_content(content: &str, budget_chars: usize) -> String {
    if content.len() <= budget_chars {
        return content.to_string();
    }

    // Web content: prioritize beginning (usually has summary/intro)
    // and a bit of the end (conclusions)
    let head_budget = budget_chars * 4 / 5;
    let tail_budget = budget_chars / 5;

    let head = if content.len() > head_budget {
        &content[..head_budget]
    } else {
        content
    };

    let tail = if content.len() > budget_chars {
        let tail_start = content.len().saturating_sub(tail_budget);
        &content[tail_start..]
    } else {
        ""
    };

    let omitted = content.len().saturating_sub(head_budget + tail_budget);

    if omitted > 0 && !tail.is_empty() {
        format!(
            "{}\n\n... [{} chars omitted] ...\n\n{}",
            head, omitted, tail
        )
    } else {
        format!("{}...\n\n[Content truncated - {} chars removed]", head, content.len() - head.len())
    }
}

/// Truncate search results: keep all results but shorten snippets
pub(crate) fn truncate_search_results(content: &str, budget_chars: usize) -> String {
    // For search results, try to keep the structure but shorten each result
    if content.len() <= budget_chars {
        return content.to_string();
    }

    // Simple approach: truncate from the end but try to keep structure
    truncate_generic(content, budget_chars)
}

/// Generic truncation: head with truncation notice
pub(crate) fn truncate_generic(content: &str, budget_chars: usize) -> String {
    if content.len() <= budget_chars {
        return content.to_string();
    }

    // Find a good break point (newline or space) near the budget
    let break_point = content[..budget_chars]
        .rfind('\n')
        .or_else(|| content[..budget_chars].rfind(' '))
        .unwrap_or(budget_chars);

    let truncated = &content[..break_point];
    let removed = content.len() - break_point;

    format!(
        "{}\n\n[... {} chars truncated ...]",
        truncated, removed
    )
}

/// Truncate a single message if too long
pub(crate) fn truncate_message(content: &str, max_chars: usize) -> String {
    truncate_generic(content, max_chars)
}

/// Truncate conversation history to fit within token budget.
/// Keeps most recent messages, drops oldest when over budget.
pub(crate) fn truncate_context(
    messages: &[nanna_storage::Message],
    max_tokens: usize,
) -> Vec<nanna_storage::Message> {
    let mut result = Vec::new();
    let mut total_tokens = 0;

    // Process from newest to oldest (reverse), keeping messages that fit
    for msg in messages.iter().rev() {
        let truncated_content = truncate_message(&msg.content, MAX_MESSAGE_CHARS);
        let msg_tokens = estimate_tokens(&truncated_content);

        if total_tokens + msg_tokens > max_tokens {
            // Budget exceeded - stop adding older messages
            break;
        }

        total_tokens += msg_tokens;

        // Clone the message with potentially truncated content
        let mut truncated_msg = msg.clone();
        truncated_msg.content = truncated_content;
        result.push(truncated_msg);
    }

    // Reverse back to chronological order
    result.reverse();
    result
}

/// Tool result with metadata for intelligent allocation
#[derive(Debug)]
pub struct ToolResultEntry {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) content: String,
    pub(crate) is_error: bool,
    pub(crate) raw_tokens: usize,
    /// Index in original order (for recency bias - higher = more recent)
    pub(crate) recency_index: usize,
}

/// Intelligent budget allocation across tool results.
///
/// Instead of equal division, allocates proportionally based on:
/// 1. Original content size (larger results get proportionally more)
/// 2. Recency bias (recent tool calls get slight priority)
/// 3. Minimum floor (never truncate below MIN_TOOL_RESULT_CHARS)
pub(crate) fn allocate_tool_budgets(entries: &[ToolResultEntry], total_budget_tokens: usize) -> Vec<usize> {
    let n = entries.len();
    if n == 0 {
        return vec![];
    }

    let total_budget_chars = total_budget_tokens * 4;
    let total_raw: usize = entries.iter().map(|e| e.raw_tokens).sum();
    let total_raw_chars = total_raw * 4;

    // If everything fits, no truncation needed
    if total_raw_chars <= total_budget_chars {
        return entries.iter().map(|e| e.content.len()).collect();
    }

    // Calculate minimum floor per tool
    let floor_per_tool = MIN_TOOL_RESULT_CHARS;
    let total_floor = floor_per_tool * n;

    // If we can't even give minimums, distribute equally
    if total_budget_chars <= total_floor {
        let per_tool = total_budget_chars / n;
        return vec![per_tool.max(500); n]; // Absolute minimum 500 chars
    }

    // Distributable budget after floors
    let distributable = total_budget_chars - total_floor;

    // Calculate weights: base weight from size + recency bonus
    let max_recency = entries.iter().map(|e| e.recency_index).max().unwrap_or(0) as f64;

    let weights: Vec<f64> = entries.iter().map(|e| {
        // Base weight from original size (proportional)
        let size_weight = (e.raw_tokens as f64) / (total_raw as f64).max(1.0);

        // Recency bonus: most recent gets up to 20% boost
        let recency_factor = if max_recency > 0.0 {
            1.0 + 0.2 * (e.recency_index as f64 / max_recency)
        } else {
            1.0
        };

        size_weight * recency_factor
    }).collect();

    // Normalize weights
    let weight_sum: f64 = weights.iter().sum();
    let normalized: Vec<f64> = weights.iter().map(|w| w / weight_sum.max(0.001)).collect();

    // Allocate: floor + proportional share
    let mut allocations: Vec<usize> = normalized.iter().map(|w| {
        let extra = (distributable as f64 * w) as usize;
        floor_per_tool + extra
    }).collect();

    // Cap allocations at actual content size (don't allocate more than needed)
    for (i, entry) in entries.iter().enumerate() {
        allocations[i] = allocations[i].min(entry.content.len());
    }

    allocations
}

/// Estimate tokens used by a CompletionRequest (for dynamic budget calculation)
pub(crate) fn estimate_request_tokens(request: &nanna_llm::CompletionRequest) -> usize {
    let mut total = 0;

    // Message tokens (includes system message with Role::System)
    for msg in &request.messages {
        total += estimate_tokens(&msg.content);
    }

    // Anthropic message tokens (tool use blocks are larger)
    for msg in &request.anthropic_messages {
        for block in &msg.content {
            total += match block {
                nanna_llm::ContentBlock::Text { text } => estimate_tokens(text),
                nanna_llm::ContentBlock::ToolUse { input, .. } => {
                    // Tool use: id + name + JSON input
                    50 + estimate_tokens(&input.to_string())
                }
                nanna_llm::ContentBlock::ToolResult { content, .. } => {
                    // Tool result: id + content
                    20 + estimate_tokens(content)
                }
                nanna_llm::ContentBlock::Image { .. } => 1000, // Images ~1k tokens
                nanna_llm::ContentBlock::Thinking { thinking, .. } => {
                    // Thinking blocks are internal reasoning
                    estimate_tokens(thinking)
                }
            };
        }
    }

    // Tool definitions overhead (~100 tokens per tool)
    total += request.tools.len() * 100;

    total
}

/// Calculate dynamic tool budget from a CompletionRequest + resolved model limits.
pub(crate) fn calculate_dynamic_tool_budget(
    request: &nanna_llm::CompletionRequest,
    model_info: &nanna_llm::ModelInfo,
) -> usize {
    let total_limit = model_info.context_window;
    let used = estimate_request_tokens(request);
    let response_reserve = RESPONSE_RESERVED_TOKENS;

    let available = total_limit.saturating_sub(used).saturating_sub(response_reserve);

    debug!(
        "Dynamic tool budget: model={}, limit={}, used={}, reserve={}, available={}",
        request.model, total_limit, used, response_reserve, available
    );

    // Return at least a minimum budget to avoid degenerate cases
    available.max(10_000) // At least 10k tokens for tools
}

/// Find the largest byte index <= max_bytes that is a valid char boundary.
pub(crate) fn truncate_boundary(s: &str, max_bytes: usize) -> usize {
    if s.len() <= max_bytes {
        return s.len();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}
