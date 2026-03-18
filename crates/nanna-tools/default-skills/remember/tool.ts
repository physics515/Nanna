export default {
  name: "remember",
  description: "Store information in long-term memory. Use this to save important facts, context, or information that should persist across conversations.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      content: { type: "string", description: "The information to remember" },
      tags: { type: "object", description: "Optional key-value tags for categorization" },
      importance: { type: "number", description: "Importance weight (0.0-1.0). Default: 1.0" }
    },
    required: ["content"]
  },
  execute: function(input) {
    var memContent = input.content || input.text || input.memory || input.fact;
    if (!memContent) throw "Missing required parameter: content";
    var params = {
      content: memContent,
      tags: input.tags || {},
      importance: input.importance || 1.0
    };
    var result = Nanna.service("memory.store", params);
    return "Remembered (id: " + result.id + "): " + memContent.substring(0, 100) + (memContent.length > 100 ? "..." : "");
  }
}
