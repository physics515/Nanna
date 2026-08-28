export default {
  name: "figma",
  version: "0.1.0",
  output: "memory",
  description: "Bridge to Figma via the Figma API (MCP-compatible endpoint). Fetch file/document data, specific nodes, export assets as images, or list comments. Requires FIGMA_API_KEY in the daemon's environment; set FIGMA_MCP_ENDPOINT to route through a Figma MCP server instead of api.figma.com.",
  parameters: {
    type: "object",
    properties: {
      action: {
        type: "string",
        description: "What to do: 'file' (fetch document tree), 'nodes' (fetch specific nodes), 'export' (render nodes to images), 'comments' (list comments on a file)",
        enum: ["file", "nodes", "export", "comments"]
      },
      file_key: {
        type: "string",
        description: "Figma file key — the segment after /file/ or /design/ in a Figma URL"
      },
      node_ids: {
        type: "string",
        description: "Comma-separated node ids (e.g. '1:2,1:3'). Required for 'nodes' and 'export'."
      },
      format: {
        type: "string",
        description: "Export image format for 'export': png, jpg, svg, or pdf. Default: png",
        enum: ["png", "jpg", "svg", "pdf"]
      },
      scale: {
        type: "number",
        description: "Export scale between 0.01 and 4 for 'export'. Default: 1"
      },
      depth: {
        type: "integer",
        description: "For 'file': how many levels deep to traverse the document tree. Omit for the full tree."
      }
    },
    required: ["action", "file_key"]
  },
  execute: function(input) {
    var action = input.action;
    var fileKey = input.file_key;

    if (!action) {
      return "Error: figma: missing required parameter 'action' (file | nodes | export | comments). Nothing was fetched.";
    }
    if (!fileKey) {
      return "Error: figma: missing required parameter 'file_key' — the segment after /file/ or /design/ in a Figma URL. Nothing was fetched.";
    }

    var apiKey = Nanna.getEnv("FIGMA_API_KEY");
    if (!apiKey) {
      return "Error: figma is unavailable in this session: no FIGMA_API_KEY is set in the daemon's environment. Nothing was fetched. Ask the user to set FIGMA_API_KEY (a Figma personal access token) before starting the daemon.";
    }

    // Optional MCP endpoint override: a Figma MCP server exposing the same
    // REST surface (e.g. a local figma-mcp bridge). Falls back to the public API.
    var base = Nanna.getEnv("FIGMA_MCP_ENDPOINT") || "https://api.figma.com";
    // strip a trailing slash so path joining stays predictable
    if (base.charAt(base.length - 1) === "/") {
      base = base.substring(0, base.length - 1);
    }

    var url;
    if (action === "file") {
      url = base + "/v1/files/" + encodeURIComponent(fileKey);
      if (input.depth) {
        url += "?depth=" + input.depth;
      }
    } else if (action === "nodes") {
      if (!input.node_ids) {
        return "Error: figma: action 'nodes' requires 'node_ids' (comma-separated, e.g. '1:2,1:3'). Nothing was fetched.";
      }
      url = base + "/v1/files/" + encodeURIComponent(fileKey) + "/nodes?ids=" + encodeURIComponent(input.node_ids);
    } else if (action === "export") {
      if (!input.node_ids) {
        return "Error: figma: action 'export' requires 'node_ids' (comma-separated, e.g. '1:2,1:3'). Nothing was exported.";
      }
      var format = input.format || "png";
      var scale = input.scale || 1;
      url = base + "/v1/images/" + encodeURIComponent(fileKey) +
        "?ids=" + encodeURIComponent(input.node_ids) +
        "&format=" + encodeURIComponent(format) +
        "&scale=" + scale;
    } else if (action === "comments") {
      url = base + "/v1/files/" + encodeURIComponent(fileKey) + "/comments";
    } else {
      return "Error: figma: unknown action '" + action + "' (expected file | nodes | export | comments). Nothing was fetched.";
    }

    var response = Nanna.fetch(url, {
      headers: {
        "Accept": "application/json",
        "X-Figma-Token": apiKey
      }
    });

    if (response.status === 403) {
      return "Error: Figma returned 403 (forbidden) for " + url + " — the FIGMA_API_KEY is invalid or lacks access to this file. Nothing was fetched.";
    }
    if (response.status === 404) {
      return "Error: Figma returned 404 for file '" + fileKey + "' — check the file key (the segment after /file/ or /design/ in the URL). Nothing was fetched.";
    }
    if (response.status !== 200) {
      return "Error: Figma API returned status " + response.status + ": " + response.body.substring(0, 200);
    }

    var data;
    try {
      data = JSON.parse(response.body);
    } catch (e) {
      return "Error: figma: failed to parse Figma response as JSON (first 200 chars): " + response.body.substring(0, 200);
    }

    if (action === "file") {
      var doc = data.document || {};
      var pages = doc.children || [];
      var lines = [];
      lines.push("Figma file: " + (data.name || fileKey));
      lines.push("Last modified: " + (data.lastModified || "unknown"));
      lines.push("Pages (" + pages.length + "):");
      for (var i = 0; i < pages.length; i++) {
        var page = pages[i];
        var kids = (page.children || []).length;
        lines.push("  - " + page.name + " [id " + page.id + ", " + kids + " top-level node(s)]");
      }
      lines.push("");
      lines.push("Full document JSON follows:");
      lines.push(JSON.stringify(data));
      return lines.join("\n");
    }

    if (action === "nodes") {
      var nodes = data.nodes || {};
      var ids = Object.keys(nodes);
      if (ids.length === 0) {
        return "No nodes found in file '" + fileKey + "' for ids: " + input.node_ids;
      }
      var out = [];
      out.push("Figma nodes from " + (data.name || fileKey) + " (" + ids.length + "):");
      for (var j = 0; j < ids.length; j++) {
        var entry = nodes[ids[j]];
        if (!entry || !entry.document) {
          out.push("  - " + ids[j] + ": (not found)");
          continue;
        }
        var n = entry.document;
        out.push("  - " + ids[j] + ": " + n.type + " \"" + n.name + "\"");
      }
      out.push("");
      out.push("Full nodes JSON follows:");
      out.push(JSON.stringify(data));
      return out.join("\n");
    }

    if (action === "export") {
      if (data.err) {
        return "Error: Figma export failed: " + data.err;
      }
      var images = data.images || {};
      var keys = Object.keys(images);
      if (keys.length === 0) {
        return "Figma export returned no images for ids: " + input.node_ids;
      }
      var rows = [];
      rows.push("Figma export (" + keys.length + " image(s)) — URLs are temporary (~30 days):");
      for (var k = 0; k < keys.length; k++) {
        var imgUrl = images[keys[k]];
        rows.push("  - " + keys[k] + ": " + (imgUrl || "(render failed for this node)"));
      }
      return rows.join("\n");
    }

    // comments
    var comments = data.comments || [];
    if (comments.length === 0) {
      return "No comments on file '" + fileKey + "'.";
    }
    var clines = [];
    clines.push("Comments on " + fileKey + " (" + comments.length + "):");
    for (var c = 0; c < comments.length; c++) {
      var cm = comments[c];
      var who = (cm.user && cm.user.handle) || "unknown";
      var when = cm.created_at || "";
      clines.push("  - [" + who + " " + when + "] " + (cm.message || ""));
    }
    return clines.join("\n");
  }
}
