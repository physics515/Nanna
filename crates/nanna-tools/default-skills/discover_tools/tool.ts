export default {
  name: "discover_tools",
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

    // Apply query filter if provided
    var matched = discoverable;
    if (query && query.length > 0) {
      var q = query.toLowerCase();
      matched = [];
      for (var i = 0; i < discoverable.length; i++) {
        var tool = discoverable[i];
        var nameMatch = tool.name.toLowerCase().indexOf(q) >= 0;
        var descMatch = tool.description && tool.description.toLowerCase().indexOf(q) >= 0;
        if (nameMatch || descMatch) {
          matched.push(tool);
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
