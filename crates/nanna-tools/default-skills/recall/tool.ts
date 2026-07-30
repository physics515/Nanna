export default {
  name: "recall",
  requires: ["memory.search", "memory.get"],
  version: "0.1.0",
  description: "Read long-term memory. Two modes: pass a HANDLE (the id printed in a [memory:xxxxxxxx ...] stub) to fetch that exact stored result back, with optional offset/limit to page through a large one; or pass a search query to find memories ranked by relevance. Tool results too large for context are stored whole and replaced by a handle stub — this is how you get the full text back.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      query: { type: "string", description: "A handle from a [memory:xxxxxxxx] stub, or search terms" },
      limit: { type: "integer", description: "Search: max results (default 5). Handle: max characters to return (default 4000)" },
      offset: { type: "integer", description: "Handle mode only: start reading at this character offset" }
    },
    required: ["query"]
  },
  execute: function(input) {
    var query = input.query || input.search || input.text;
    if (!query) {
      return "No query provided. Usage: recall({query: \"search terms\"}) or recall({query: \"<handle>\"})";
    }

    // Handle mode: a stub's id is an address, so resolve it exactly rather
    // than hoping similarity search rediscovers the same text. Only bare
    // id-shaped tokens qualify, so ordinary questions still search.
    var looksLikeHandle = /^[0-9a-fA-F][0-9a-fA-F-]{5,}$/.test(query.trim());
    if (looksLikeHandle) {
      try {
        var got = Nanna.service("memory.get", {
          id: query.trim(),
          offset: input.offset || 0,
          limit: input.limit || 4000
        });
        if (got && got.content) {
          var more = got.truncated
            ? "\n\n[" + got.returned + " of " + got.total + " chars shown. Continue with recall({query: \"" +
              query.trim() + "\", offset: " + (got.offset + got.returned) + "})]"
            : "";
          return got.content + more;
        }
      } catch (e) {
        // Not a stored handle after all — fall through to search, which is
        // the more useful answer for a token that merely looked like an id.
      }
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
