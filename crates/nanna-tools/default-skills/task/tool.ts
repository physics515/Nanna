export default {
  name: "task",
  version: "0.1.0",
  timeout: 86400,
  description: "Delegate a sub-task to a separate agent. The sub-agent runs independently with its own context and tools, then returns the result. Useful for complex tasks that benefit from isolated execution.",
  parameters: {
    type: "object",
    properties: {
      prompt: { type: "string", description: "The task description / prompt for the sub-agent" },
      description: { type: "string", description: "Short label for logging. Default: 'sub-task'" },
      max_iterations: { type: "integer", description: "Optional hard cap on iterations. Omit for no limit (sub-agent will be nudged to wrap up progressively). Only set this if you want a strict cutoff." }
    },
    required: ["prompt"]
  },
  execute: function(input) {
    var params = {
      prompt: input.prompt,
      description: input.description || "sub-task"
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
