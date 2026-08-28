//! Message generation utilities extracted from loop_runner.rs
//! 
//! Contains all message formatting functions for agent communication,
//! nudge messages, mission control messages, and wrapup messages.

use serde::{Deserialize, Serialize};

/// Generate a wrapup message after tool use completes
pub fn step_wrapup_message(task_anchor: Option<&str>, reason: &str) -> String {
    format!(
        "{}Tool use for this step is over ({reason}). Reply in plain text NOW — this is \
         the last chance to act before the next iteration.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a mission continuation message when rounds stall
pub fn mission_continue_message(
    round: usize,
    stall_rounds: usize,
) -> String {
    format!(
        "[MISSION CONTROL] Round {round} stalled for {stall_rounds} iterations. \
         Assess the situation and act decisively.",
    )
}

/// Generate a mission verification message
pub fn mission_verify_message() -> String {
    "[MISSION CONTROL verification] You declared MISSION COMPLETE. Prove it with real \
     commands NOW: exec the full test suite (or every key command) and show the actual \
     results, not assertions."
}

/// Generate a mission convergence message
pub fn mission_convergence_message(round: usize, repeats: usize, digest: &str) -> String {
    let anchor = if digest.is_empty() {
        String::new()
    } else {
        format!("Digest: {digest}\n")
    };
    format!(
        "{}[MISSION CONTROL] Round {round} converged after {repeats} repeats.\n{}",
        anchor,
        "Continue with the next step or declare completion."
    )
}

/// Generate a wrapup nudge message based on level
pub fn wrapup_nudge_message(
    level: NudgeLevel,
    iteration: usize,
) -> String {
    match level {
        NudgeLevel::Gentle => format!(
            "[Nudge {iteration}] This is a gentle reminder to proceed. \
             The agent loop is waiting for your next action.",
        ),
        NudgeLevel::Moderate => format!(
            "[Nudge {iteration}] Moderate nudge: you're approaching a decision point. \
             Choose the next step or declare completion.",
        ),
        NudgeLevel::Urgent => format!(
            "[Nudge {iteration}] URGENT: You've been looping. Act now or I'll intervene.",
        ),
    }
}

/// Generate a claim nudge message
pub fn claim_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You have already executed successful write/exec work this step. \
         Continue with the next task or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a claim failure nudge message
pub fn claim_nudge_failure_message(
    task_anchor: Option<&str>,
    subject: &str,
) -> String {
    format!(
        "{}Failed to execute work on {subject}. This is a critical failure. \
         You must act now or the mission will fail.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a tool loop nudge message
pub fn tool_loop_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You called the same tool with the same arguments twice and got the identical \
         result. This is a loop. Change your approach or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a narration nudge message
pub fn narration_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You narrated tool calls instead of actually executing them — NOTHING you \
         wrote changed the world. Execute the tools or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a repetition nudge message
pub fn repetition_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}Your last response repeated the same line(s) over and over — you are stuck in a \
         loop. Break the pattern or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a thinking spiral nudge message
pub fn thinking_spiral_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You were stuck in a reasoning loop — STOP deliberating and act: call \
         the next tool or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a budget warning message
pub fn budget_warning_message(cumulative: u64, budget: u64, task_anchor: Option<&str>) -> String {
    format!(
        "{}Token budget status: {cumulative} of {budget} tokens used (over 80%) — finish \
         the current task or declare completion NOW.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a memory nudge message
pub fn memory_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You're not using your memory effectively. Recall relevant context before \
         proceeding.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a tool selection nudge message
pub fn tool_selection_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You're hesitating on which tool to call. Choose one or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a fallback nudge message
pub fn fallback_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}Fallback triggered: your primary approach failed. Try an alternative or \
         declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a reconnect nudge message
pub fn reconnect_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}Connection lost. Reconnect to the agent loop or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a persistence nudge message
pub fn persistence_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}Your work must persist across failures. Ensure state is saved before \
         proceeding.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a conversation nudge message
pub fn conversation_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You're in a multi-turn conversation. Maintain context and respond \
         appropriately.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a connection nudge message
pub fn connection_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}Connection established. Proceed with your task.",
        task_anchor.unwrap_or("")
    )
}

/// Generate an assertion nudge message
pub fn assertion_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You made an assertion without evidence. Prove it or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a hallucination nudge message
pub fn hallucination_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You're hallucinating. Stick to facts and available information.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a reasoning nudge message
pub fn reasoning_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}Your reasoning is sound but incomplete. Continue or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a planning nudge message
pub fn planning_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You're planning well but not executing. Take action or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate an exploration nudge message
pub fn exploration_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You're exploring options. Narrow down or commit to one path.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a synthesis nudge message
pub fn synthesis_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You've gathered information. Synthesize it or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a reflection nudge message
pub fn reflection_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}Reflect on your progress. What's working? What needs adjustment?",
        task_anchor.unwrap_or("")
    )
}

/// Generate an optimization nudge message
pub fn optimization_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}You can optimize your approach. Consider efficiency or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a validation nudge message
pub fn validation_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}Validate your work before proceeding. Check for errors.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a debugging nudge message
pub fn debugging_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}Debugging detected. Fix the issue or declare completion.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a recovery nudge message
pub fn recovery_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}Recovery mode: your primary approach failed. Try an alternative.",
        task_anchor.unwrap_or("")
    )
}

/// Generate a completion nudge message
pub fn completion_nudge_message(task_anchor: Option<&str>) -> String {
    format!(
        "{}Task complete. Declare completion or move to the next task.",
        task_anchor.unwrap_or("")
    )
}

/// Nudge level enum for categorizing message urgency
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NudgeLevel {
    /// Gentle reminder
    Gentle,
    /// Moderate urgency
    Moderate,
    /// Urgent intervention needed
    Urgent,
}

impl Default for NudgeLevel {
    fn default() -> Self {
        Self::Gentle
    }
}