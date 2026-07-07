export default {
  name: "python",
  version: "0.1.0",
  output: "context",
  description: "Execute Python code or manage saved scripts. Embedded interpreter — no system Python required. Supports standard library (os, json, re, pathlib, collections, math, etc). No pip/third-party packages. Use for file manipulation, data processing, batch edits, text transforms, and scripting.",
  parameters: {
    type: "object",
    properties: {
      code: {
        type: "string",
        description: "Python code to execute. Multi-line supported. Use print() for output. Not needed if running a saved script."
      },
      action: {
        type: "string",
        description: "Script management action: 'run' (default), 'save', 'load', 'list', 'delete'. 'run' executes code directly. 'save' stores code as a named script. 'load' runs a saved script. 'list' shows all saved scripts. 'delete' removes a saved script.",
        enum: ["run", "save", "load", "list", "delete"]
      },
      name: {
        type: "string",
        description: "Script name for save/load/delete actions. Use short descriptive names like 'fix_imports' or 'batch_rename'."
      },
      args: {
        type: "string",
        description: "JSON string of arguments to pass to the script. Available as `args` dict in the Python code. Example: '{\"pattern\": \"*.rs\", \"dry_run\": true}'"
      },
      workdir: {
        type: "string",
        description: "Working directory for the script. Defaults to the session workspace."
      },
      timeout: {
        type: "integer",
        description: "Timeout in seconds. Default: 30"
      }
    },
    required: []
  },
  execute: function(input) {
    var action = input.action || "run";
    var sessionId = Nanna.sessionId();
    var scriptFile = sessionId ? ".nanna-scripts-" + sessionId + ".json" : ".nanna-scripts.json";

    // Load saved scripts
    var scripts = {};
    try {
      var content = Nanna.readFile(scriptFile);
      if (content) {
        scripts = JSON.parse(content);
      }
    } catch (e) {
      scripts = {};
    }

    switch (action) {
      case "save": {
        if (!input.name) {
          return { content: "Error: 'name' is required for save action", success: false };
        }
        if (!input.code) {
          return { content: "Error: 'code' is required for save action", success: false };
        }
        scripts[input.name] = {
          code: input.code,
          saved_at: new Date().toISOString(),
          description: input.description || ""
        };
        Nanna.writeFile(scriptFile, JSON.stringify(scripts, null, 2));
        var lineCount = input.code.split("\n").length;
        return { content: "Saved script '" + input.name + "' (" + lineCount + " lines). Run with: python(action='load', name='" + input.name + "')", success: true };
      }

      case "list": {
        var names = Object.keys(scripts);
        if (names.length === 0) {
          return { content: "No saved scripts in this session.", success: true };
        }
        var lines = ["Saved scripts (" + names.length + "):"];
        for (var i = 0; i < names.length; i++) {
          var s = scripts[names[i]];
          var lineCount = s.code.split("\n").length;
          var desc = s.description ? " — " + s.description : "";
          lines.push("  • " + names[i] + " (" + lineCount + " lines, saved " + s.saved_at + ")" + desc);
        }
        return { content: lines.join("\n"), success: true };
      }

      case "delete": {
        if (!input.name) {
          return { content: "Error: 'name' is required for delete action", success: false };
        }
        if (!scripts[input.name]) {
          return { content: "Error: script '" + input.name + "' not found", success: false };
        }
        delete scripts[input.name];
        Nanna.writeFile(scriptFile, JSON.stringify(scripts, null, 2));
        return { content: "Deleted script '" + input.name + "'.", success: true };
      }

      case "load": {
        if (!input.name) {
          return { content: "Error: 'name' is required for load action", success: false };
        }
        if (!scripts[input.name]) {
          var available = Object.keys(scripts);
          var hint = available.length > 0 ? " Available: " + available.join(", ") : " No scripts saved.";
          return { content: "Error: script '" + input.name + "' not found." + hint, success: false };
        }
        // Fall through to execution with the saved code
        input.code = scripts[input.name].code;
        // Fall through to run
      }

      case "run":
      default: {
        if (!input.code) {
          return { content: "Error: 'code' is required. Provide Python code to execute, or use action='load' with a script name.", success: false };
        }

        // Prepend args injection if provided
        var codeToRun = input.code;
        if (input.args) {
          codeToRun = "import json\nargs = json.loads(" + JSON.stringify(input.args) + ")\n" + codeToRun;
        }

        var result;
        try {
          result = Nanna.service("python.exec", {
            code: codeToRun,
            workdir: input.workdir,
            timeout: input.timeout || 30
          });
        } catch (e) {
          return { content: "Error: Python service unavailable: " + e, success: false };
        }

        var output = "";
        if (result.stdout) {
          output = result.stdout;
        }
        if (result.stderr) {
          output += (output ? "\n--- stderr ---\n" : "") + result.stderr;
        }
        if (result.error) {
          output += (output ? "\n--- error ---\n" : "") + result.error;
        }
        if (!output) {
          output = "(no output)";
        }

        if (result.duration_ms !== undefined) {
          output += "\n(" + result.duration_ms + "ms)";
        }

        return { content: output, success: result.success };
      }
    }
  }
}
