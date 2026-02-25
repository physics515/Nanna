export default {
  name: "recall",
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
    var results = Nanna.service("memory.search", {
      query: input.query,
      limit: input.limit || 5
    });

    if (!results || results.length === 0) {
      return "No memories found matching: " + input.query;
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
