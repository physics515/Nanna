export default {
  name: "create_tool",
  requires: ["tools.create"],
  version: "0.2.0",
  description: "Author a NEW tool and make it callable immediately, no daemon restart. Give it a name, a description, a parameter schema, and JavaScript/TypeScript source. Pass either a complete module ('export default {...}') as source, or just the BODY of the execute function and the module is assembled around it from the other arguments. The tool is written into the daemon's tools directory and registered live. This refuses to overwrite an existing tool — use edit_tool to change one.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      name: {
        type: "string",
        description: "Tool name: a lowercase letter first, then lowercase letters, digits or underscores, max 64 chars (^[a-z][a-z0-9_]{0,63}$). No dots, slashes, spaces or other separators — the name becomes a directory on disk."
      },
      description: {
        type: "string",
        description: "One or two sentences: what the tool does and when to use it. This is what the model reads when choosing tools."
      },
      parameters: {
        type: "object",
        description: "JSON Schema for the tool's input, e.g. {\"type\":\"object\",\"properties\":{\"url\":{\"type\":\"string\"}},\"required\":[\"url\"]}. Optional when 'source' is a complete module that declares its own."
      },
      source: {
        type: "string",
        description: "The tool's code. EITHER a complete module: 'export default { name, description, parameters, execute: function(input) {...} }' — OR just the body of the execute function (it receives 'input'; return a string or an object). In a body you can use Nanna.exec, Nanna.readFile, Nanna.writeFile, Nanna.fetch, Nanna.service, etc."
      }
    },
    required: ["name", "description", "source"]
  },
  execute: function(input) {
    var name = (input.name || "").trim();
    var description = (input.description || "").trim();
    var source = input.source || "";
    var schema = input.parameters;

    // Mirror the daemon's validate_tool_name rules exactly (^[a-z][a-z0-9_]{0,63}$).
    // The name becomes a directory under the tools dir, so separators, dots and
    // anything that could traverse paths must never pass — reject here first so
    // the model gets the rule spelled out instead of a server-side error.
    if (!/^[a-z][a-z0-9_]{0,63}$/.test(name)) {
      return "create_tool refused: invalid name " + JSON.stringify(name) +
        ". Names must match ^[a-z][a-z0-9_]{0,63}$ — a lowercase letter first, then only lowercase letters, digits or underscores; no dots, slashes, spaces or path tricks. Nothing was created.";
    }
    if (!description) {
      return "create_tool refused: 'description' is required so the model knows when to pick the tool. Nothing was created.";
    }
    if (!source || !String(source).trim()) {
      return "create_tool refused: 'source' is empty. Pass a complete module ('export default {...}') or the body of the execute function. Nothing was created.";
    }

    // The schema may arrive as a JSON string from a model that stringified it.
    if (schema !== undefined && schema !== null) {
      if (typeof schema === "string") {
        try {
          schema = JSON.parse(schema);
        } catch (e) {
          return "create_tool refused: 'parameters' is a string that is not valid JSON (" + e + "). Pass the schema as a JSON object. Nothing was created.";
        }
      }
      if (typeof schema !== "object" || Array.isArray(schema)) {
        return "create_tool refused: 'parameters' must be a JSON Schema object like {\"type\":\"object\",\"properties\":{...}}. Nothing was created.";
      }
    }

    // Refuse to shadow ANY currently registered tool (builtin or skill):
    // two tools sharing a name means one silently stops being callable.
    // The tools.create service also refuses an on-disk duplicate, so losing
    // this check (listTools unavailable) narrows the net, never removes it.
    try {
      var existing = Nanna.listTools();
      if (existing && existing.length) {
        for (var i = 0; i < existing.length; i++) {
          var t = existing[i];
          var tn = typeof t === "string" ? t : t && t.name;
          if (tn === name) {
            return "create_tool refused: a tool named '" + name + "' is already registered. Pick another name, or use edit_tool to change an existing tool. Nothing was created.";
          }
        }
      }
    } catch (e) {
      // listTools not available in this context; the service still enforces
      // no-overwrite on disk.
    }

    // Assemble a full module when only an execute body was given, so agents
    // can author simple tools without hand-writing module boilerplate.
    var moduleSource = String(source);
    if (moduleSource.indexOf("export default") === -1) {
      moduleSource =
        "export default {\n" +
        "  name: " + JSON.stringify(name) + ",\n" +
        "  version: \"0.1.0\",\n" +
        "  description: " + JSON.stringify(description) + ",\n" +
        "  parameters: " + JSON.stringify(schema || { type: "object", properties: {}, required: [] }, null, 2) + ",\n" +
        "  execute: function(input) {\n" + moduleSource + "\n  }\n" +
        "};\n";
    }

    // The daemon owns the tools_dir and the live registry, so creation is
    // delegated to the tools.create service: it re-validates the name, refuses
    // an existing tool, writes <tools_dir>/<name>/tool.ts, compiles the source
    // and registers it live. The result names the exact path it wrote.
    try {
      var result = Nanna.service("tools.create", {
        name: name,
        description: description,
        parameters: schema || null,
        source: moduleSource
      });
      if (result && result.error) {
        return "create_tool failed: " + result.error + " Nothing was registered.";
      }
      var path = result && result.path;
      var lines = [];
      lines.push("Created tool '" + name + "'" + (path ? " at " + path : "") + ".");
      if (result && result.registered === false) {
        lines.push("NOTE: the file was written but live registration failed" + (result && result.message ? ": " + result.message : ".") + " It will load on the next daemon restart.");
      } else {
        lines.push("It is registered and callable right now — no restart needed. Call it as '" + name + "'.");
      }
      if (!path) {
        lines.push("(The tools.create service did not report the file path it wrote.)");
      }
      return lines.join("\n");
    } catch (e) {
      return "create_tool failed: " + e + ". Nothing was created. (If this says 'Service not found: tools.create', this daemon does not expose the tool-authoring service — the skill stays hidden until it does.)";
    }
  }
};
