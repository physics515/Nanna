export default {
  name: "list_user_tools",
  version: "0.1.0",
  description: "List all user-created tools showing their names, descriptions, and status.",
  parameters: {
    type: "object",
    properties: {},
    required: []
  },
  execute: function(input) {
    try {
      var tools = Nanna.service("tools.list", {});
      if (!tools || tools.length === 0) {
        return "No user-created tools found. Use create_tool to make one.";
      }

      var lines = [];
      for (var i = 0; i < tools.length; i++) {
        var t = tools[i];
        var status = t.enabled ? "enabled" : "disabled";
        lines.push("- " + t.name + " [" + status + "]: " + (t.description || "(no description)"));
      }

      return "User tools (" + tools.length + "):\n\n" + lines.join("\n");
    } catch (e) {
      return "Error listing tools: " + e;
    }
  }
}
