export default {
  name: "edit_tool",
  requires: ["tools.update"],
  version: "0.1.0",
  description: "Modify an EXISTING tool and make the change live immediately, no daemon restart. Pass the tool's name plus EITHER a complete replacement module ('export default {...}') as source, OR an old_string/new_string pair for a targeted in-place edit (old_string must match the tool's current source exactly). The edit is applied inside the daemon's tools directory and the tool is re-registered live. This refuses to touch a tool that does not exist — use create_tool to author a new one.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      name: {
        type: "string",
        description: "Name of the existing tool to edit: a lowercase letter first, then lowercase letters, digits or underscores, max 64 chars (^[a-z][a-z0-9_]{0,63}$). No dots, slashes, spaces or other separators — the name maps to a directory on disk and anything else is rejected."
      },
      source: {
        type: "string",
        description: "Complete replacement module: 'export default { name, description, parameters, execute: function(input) {...} }'. Replaces the tool's whole source. Mutually exclusive with old_string/new_string — pass one or the other, not both."
      },
      old_string: {
        type: "string",
        description: "For a targeted edit: the exact text currently in the tool's source to be replaced. Copy it verbatim and include enough surrounding lines to match exactly once. Requires new_string."
      },
      new_string: {
        type: "string",
        description: "For a targeted edit: the replacement text. May be empty to delete the matched snippet. Must differ from old_string. Requires old_string."
      }
    },
    required: ["name"]
  },
  execute: function(input) {
    var name = (input.name || "").trim();
    var source = input.source;
    var oldString = input.old_string;
    var newString = input.new_string;

    // Mirror the daemon's validate_tool_name rules exactly (^[a-z][a-z0-9_]{0,63}$).
    // The name maps to a directory under the tools dir, so separators, dots and
    // anything that could traverse paths must never pass — rejecting here first
    // means the model gets the rule spelled out instead of a server-side error,
    // and a traversal attempt ('../x', 'a/b', symlink tricks via names) dies on
    // the regex before any path is ever built from it.
    if (!/^[a-z][a-z0-9_]{0,63}$/.test(name)) {
      return "edit_tool refused: invalid name " + JSON.stringify(name) +
        ". Names must match ^[a-z][a-z0-9_]{0,63}$ — a lowercase letter first, then only lowercase letters, digits or underscores; no dots, slashes, spaces or path tricks. Nothing was changed.";
    }

    var hasSource = source !== undefined && source !== null && String(source).trim() !== "";
    var hasOld = oldString !== undefined && oldString !== null && String(oldString) !== "";
    var hasNew = newString !== undefined && newString !== null;

    // Exactly one edit mode: whole-source replacement OR a targeted snippet edit.
    // Accepting both at once would make it ambiguous which change the caller
    // meant, and applying them in either order silently discards one of them.
    if (hasSource && (hasOld || hasNew)) {
      return "edit_tool refused: pass EITHER 'source' (whole replacement) OR 'old_string'/'new_string' (targeted edit), not both. Nothing was changed.";
    }
    if (!hasSource && !hasOld) {
      return "edit_tool refused: no edit given. Pass 'source' with the complete replacement module, or 'old_string' + 'new_string' for a targeted edit. Nothing was changed.";
    }
    if (hasOld && !hasNew) {
      return "edit_tool refused: 'old_string' was given without 'new_string'. Pass both (new_string may be an empty string to delete the snippet). Nothing was changed.";
    }
    if (hasOld && String(oldString) === String(newString)) {
      return "edit_tool refused: old_string and new_string are identical — that edit would change nothing. Nothing was changed.";
    }

    // A whole-source replacement must still be a plausible tool module: an
    // empty or truncated body would silently destroy the tool. Never truncate —
    // refuse and say so, consistent with the house write-path rules.
    if (hasSource) {
      var replacement = String(source);
      if (replacement.indexOf("export default") === -1) {
        return "edit_tool refused: 'source' does not contain 'export default' — a replacement must be the COMPLETE module, not a fragment. For a partial change use old_string/new_string instead. Nothing was changed.";
      }
    }

    // Only edit a tool that actually exists: steer creation to create_tool so
    // a typo'd name cannot quietly become a brand-new tool. The tools.update
    // service refuses a missing tool on disk too, so losing this check
    // (listTools unavailable) narrows the net, never removes it.
    try {
      var existing = Nanna.listTools();
      if (existing && existing.length) {
        var found = false;
        for (var i = 0; i < existing.length; i++) {
          var t = existing[i];
          var tn = typeof t === "string" ? t : t && t.name;
          if (tn === name) {
            found = true;
            break;
          }
        }
        if (!found) {
          return "edit_tool refused: no tool named '" + name + "' is registered. Check the name with list_tools, or use create_tool to author a new tool. Nothing was changed.";
        }
      }
    } catch (e) {
      // listTools not available in this context; the service still refuses a
      // tool that does not exist on disk.
    }

    // The daemon owns the tools_dir and the live registry, so the edit is
    // delegated to the tools.update service: it re-validates the name, resolves
    // <tools_dir>/<name>/tool.ts and rejects a path that escapes the tools dir
    // or a symlinked target, applies the whole-source replacement or the
    // old_string/new_string edit against the CURRENT file (refusing an
    // old_string that matches zero or many places), writes the result, and
    // re-registers the tool live. The result names the exact path it changed.
    try {
      var payload = { name: name };
      if (hasSource) {
        payload.source = String(source);
      } else {
        payload.old_string = String(oldString);
        payload.new_string = String(newString);
      }
      var result = Nanna.service("tools.update", payload);
      if (result && result.error) {
        return "edit_tool failed: " + result.error + " The tool is unchanged.";
      }
      var path = result && result.path;
      var lines = [];
      if (hasSource) {
        lines.push("Replaced the whole source of tool '" + name + "'" + (path ? " at " + path : "") + ".");
      } else {
        var count = result && typeof result.replacements === "number" ? result.replacements : 1;
        lines.push("Edited tool '" + name + "'" + (path ? " at " + path : "") + ": replaced " + count + " occurrence(s) of the given old_string.");
      }
      if (result && result.registered === false) {
        lines.push("NOTE: the file was written but live re-registration failed" + (result && result.message ? ": " + result.message : ".") + " The change takes effect on the next daemon restart.");
      } else {
        lines.push("The updated tool is registered and callable right now — no restart needed.");
      }
      if (!path) {
        lines.push("(The tools.update service did not report the file path it changed.)");
      }
      return lines.join("\n");
    } catch (e) {
      return "edit_tool failed: " + e + ". The tool is unchanged. (If this says 'Service not found: tools.update', this daemon does not expose the tool-editing service — the skill stays hidden until it does.)";
    }
  }
};
