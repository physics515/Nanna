export default {
  name: "create_tool",
  description: "Create a new user-defined tool by writing a JavaScript tool file. The tool will be saved and available for future use.",
  parameters: {
    type: "object",
    properties: {
      name: { type: "string", description: "Tool name (lowercase, no spaces)" },
      description: { type: "string", description: "What the tool does" },
      source: { type: "string", description: "Complete JavaScript source code for the tool (must export default an object with name, description, parameters, and execute function)" }
    },
    required: ["name", "description", "source"]
  },
  execute: function(input) {
    try {
      var result = Nanna.service("tools.create", {
        name: input.name,
        description: input.description,
        source: input.source
      });
      return "Created tool: " + input.name + "\n" + (result.message || "Tool saved successfully.");
    } catch (e) {
      return "Error creating tool: " + e;
    }
  }
}
