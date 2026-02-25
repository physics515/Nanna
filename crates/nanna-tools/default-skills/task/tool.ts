export default {
  name: "task",
  description: "Delegate a sub-task to a separate agent. The sub-agent runs independently with its own context and tools, then returns the result. Useful for complex tasks that benefit from isolated execution.",
  parameters: {
    type: "object",
    properties: {
      prompt: { type: "string", description: "The task description / prompt for the sub-agent" },
      description: { type: "string", description: "Short label for logging. Default: 'sub-task'" },
      max_iterations: { type: "integer", description: "Maximum iterations for the sub-agent. Default: 10" }
    },
    required: ["prompt"]
  },
  execute: function(input) {
    var result = Nanna.service("agent.spawn", {
      prompt: input.prompt,
      description: input.description || "sub-task",
      max_iterations: input.max_iterations || 10
    });

    var output = result.text || "(no output)";
    output += "\n\n--- Sub-agent stats: " + result.iterations + " iterations, " + result.tool_calls + " tool calls ---";
    return output;
  }
}
