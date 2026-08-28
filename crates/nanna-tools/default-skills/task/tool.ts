export default {
  name: "sub_agent",
  requires: ["agent.spawn"],
  version: "0.2.0",
  timeout: 86400,
  description: "Spawn a Sub-Agent — an ordinary chat you own that runs the same chat/harness path as a normal turn, then returns its reply. Use to delegate isolated work (research, file analysis, a self-contained sub-problem) without filling your own context. Search keywords: sub-agent, subagent, delegate, spawn, task, worker, child chat. The sub-agent cannot see your conversation; give it a self-contained prompt. On failure it reports a named error instead of looping empty. Alias: task.",
  parameters: {
    type: "object",
    properties: {
      prompt: { type: "string", description: "Self-contained prompt for the sub-agent chat. Include everything it needs; it cannot see your history." },
      description: { type: "string", description: "Short label for logging. Default: 'sub-agent'" },
      max_iterations: { type: "integer", description: "Optional hard cap on iterations. Omit for no limit (the chat is nudged to wrap up progressively). Only set this if you want a strict cutoff." }
    },
    required: ["prompt"]
  },
  execute: function(input) {
    var params = {
      prompt: input.prompt,
      description: input.description || "sub-agent"
    };
    // Only pass max_iterations if explicitly set — otherwise let the sub-agent
    // run until done, with progressive nudges to wrap up.
    if (input.max_iterations) {
      params.max_iterations = input.max_iterations;
    }
    var result = Nanna.service("agent.spawn", params);

    var output = result.text || "(no output)";
    var stats = "--- Sub-agent stats: " + result.iterations + " iterations, " + result.tool_calls + " tool calls";
    if (result.model) {
      stats += ", model: " + result.model;
    }
    stats += " ---";
    output += "\n\n" + stats;
    return output;
  }
}
