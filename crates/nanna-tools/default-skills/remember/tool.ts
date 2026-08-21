export default {
  name: "remember",
  requires: ["memory.store"],
  version: "0.1.0",
  description: "Store information in long-term memory. Use this to save important facts, context, or information that should persist across conversations. Set provenance to \"stated\" when the user actually said this, and leave it alone when you inferred it — a stated memory is kept in the user's words and is never paraphrased by a consolidation pass, so claiming it for your own inference would pin something the user never said.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      content: { type: "string", description: "The information to remember" },
      tags: { type: "object", description: "Optional key-value tags for categorization" },
      importance: { type: "number", description: "Importance weight (0.0-1.0). Default: 1.0" },
      provenance: { type: "string", enum: ["stated", "observed"], description: "\"stated\" if the user asserted this, \"observed\" if you inferred it. Default: \"observed\" — anything that is not exactly \"stated\" is treated as observed." }
    },
    required: ["content"]
  },
  execute: function(input) {
    var memContent = input.content || input.text || input.memory || input.fact;
    if (!memContent) throw "Missing required parameter: content";
    var params = {
      content: memContent,
      tags: input.tags || {},
      importance: input.importance || 1.0,
      // Classified daemon-side by the same rule the extraction path uses, so
      // an odd spelling degrades to "observed" rather than pinning a memory
      // the user never stated.
      provenance: input.provenance || "observed"
    };
    var result = Nanna.service("memory.store", params);
    return "Remembered (id: " + result.id + "): " + memContent.substring(0, 100) + (memContent.length > 100 ? "..." : "");
  }
}
