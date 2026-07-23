export default {
  name: "discover_tools",
  version: "0.1.1",
  description: "Activate tools for file access, shell commands, web browsing, code analysis, and more. Call with no arguments to see all available tools, or with a query (e.g. 'file', 'exec', 'web', 'code') to filter. Activated tools persist for the rest of this conversation. You MUST call this before using any tool beyond remember/recall/reflect.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      query: {
        type: "string",
        description: "Optional search query to filter tools by name or description (case-insensitive)"
      }
    }
  },
  execute({ query }) {
    var allTools = Nanna.listTools();
    if (!allTools || !allTools.length) {
      return { content: "No tools available.", data: { activate_tools: [] } };
    }

    var CORE = { remember: true, recall: true, reflect: true, discover_tools: true };

    // Filter out core tools (already always available)
    var discoverable = [];
    for (var i = 0; i < allTools.length; i++) {
      if (!CORE[allTools[i].name]) {
        discoverable.push(allTools[i]);
      }
    }

    // Apply query filter if provided. The query is TOKENIZED and matched
    // per word: "file write" must find the file tools exactly like
    // "write file" does — the old whole-phrase substring match returned
    // nothing unless the words happened to appear in that order. Tools
    // matching more query words rank first.
    var matched = discoverable;
    if (query && query.length > 0) {
      var terms = [];
      var cur = "";
      var ql = query.toLowerCase();
      for (var ci = 0; ci < ql.length; ci++) {
        var ch = ql.charAt(ci);
        var isWord = (ch >= "a" && ch <= "z") || (ch >= "0" && ch <= "9") || ch === "_";
        if (isWord) {
          cur += ch;
        } else if (cur.length > 0) {
          terms.push(cur);
          cur = "";
        }
      }
      if (cur.length > 0) {
        terms.push(cur);
      }

      if (terms.length > 0) {
        var scored = [];
        for (var i = 0; i < discoverable.length; i++) {
          var tool = discoverable[i];
          var hay = (tool.name + " " + (tool.description || "")).toLowerCase();
          var hits = 0;
          for (var t = 0; t < terms.length; t++) {
            if (hay.indexOf(terms[t]) >= 0) {
              hits++;
            }
          }
          if (hits > 0) {
            scored.push({ tool: tool, hits: hits });
          }
        }
        scored.sort(function (a, b) { return b.hits - a.hits; });
        matched = [];
        for (var i = 0; i < scored.length; i++) {
          matched.push(scored[i].tool);
        }
      }
    }

    if (matched.length === 0) {
      return {
        content: "No tools found matching '" + query + "'. Try a broader query or call discover_tools() with no query to see all.",
        data: { activate_tools: [] }
      };
    }

    // Format output
    var lines = [];
    var names = [];
    for (var i = 0; i < matched.length; i++) {
      var tool = matched[i];
      names.push(tool.name);
      var entry = "## " + tool.name + "\n" + (tool.description || "(no description)");
      if (tool.parameters && tool.parameters.length > 0) {
        var paramParts = [];
        for (var j = 0; j < tool.parameters.length; j++) {
          var p = tool.parameters[j];
          var part = p.name + " (" + (p.type || "string");
          if (p.required) {
            part = part + ", required";
          }
          part = part + ")";
          paramParts.push(part);
        }
        entry = entry + "\nParams: " + paramParts.join(", ");
      }
      lines.push(entry);
    }

    var content = "Found " + matched.length + " tool(s):\n\n" + lines.join("\n\n");

    return { content: content, data: { activate_tools: names } };
  }
}
