export default {
  name: "reflect",
  description: "Review and manage stored memories. List recent memories, delete specific ones, or get an overview of what has been remembered.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      action: {
        type: "string",
        description: "Action: 'list' to show recent memories, 'delete' to remove a memory by id",
        enum: ["list", "delete"]
      },
      id: { type: "string", description: "Memory ID to delete (required for 'delete' action)" },
      limit: { type: "integer", description: "Number of memories to list. Default: 10" }
    },
    required: ["action"]
  },
  execute: function(input) {
    var action = input.action || "list";

    if (action === "delete") {
      if (!input.id) {
        return "Error: 'id' is required for delete action";
      }
      var result = Nanna.service("memory.delete", { id: input.id });
      if (result && result.deleted) {
        return "Deleted memory: " + input.id;
      }
      return "Memory not found: " + input.id;
    }

    // List memories
    var memories = Nanna.service("memory.list", { limit: input.limit || 10 });

    if (!memories || memories.length === 0) {
      return "No memories stored yet.";
    }

    var lines = [];
    for (var i = 0; i < memories.length; i++) {
      var m = memories[i];
      var preview = m.content.substring(0, 120);
      if (m.content.length > 120) preview += "...";
      lines.push("[" + m.id + "] " + preview);
    }

    return "Stored memories (" + memories.length + "):\n\n" + lines.join("\n\n");
  }
}
