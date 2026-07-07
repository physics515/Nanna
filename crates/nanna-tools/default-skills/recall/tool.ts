export default {
  name: "recall",
  version: "0.1.0",
  description: "Search long-term memory for relevant information. Returns memories ranked by relevance to the query.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      query: { type: "string", description: "Search query to find relevant memories" },
      limit: { type: "integer", description: "Maximum number of results. Default: 5" }
    },
    required: ["query"]
  },
  execute: function(input) {
    var query = input.query || input.search || input.text;
    if (!query) {
      return "No query provided. Usage: recall({query: \"search terms\"})";
    }
    var results;
    try {
      results = Nanna.service("memory.search", {
        query: query,
        limit: input.limit || 5
      });
    } catch (e) {
      // Embedding model not configured — return gracefully instead of erroring
      return "Memory search unavailable (no embedding model configured). Continuing without memory context.";
    }

    if (!results || results.length === 0) {
      return "No memories found matching: " + query;
    }

    var lines = [];
    for (var i = 0; i < results.length; i++) {
      var r = results[i];
      var score = r.score !== undefined ? " (relevance: " + r.score.toFixed(2) + ")" : "";
      lines.push((i + 1) + ". [" + r.id + "]" + score + "\n   " + r.content);
    }

    return "Found " + results.length + " memories:\n\n" + lines.join("\n\n");
  }
}
