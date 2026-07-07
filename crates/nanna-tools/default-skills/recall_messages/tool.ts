export default {
  name: "recall_messages",
  version: "0.1.0",
  description: "Recall recent conversation messages that may have been summarized or compressed away. Use this when you've lost track of the original user request, need to review what was discussed earlier, or want to refresh your context about the conversation. Returns the last N messages from the current session.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      limit: {
        type: "integer",
        description: "Number of recent messages to recall. Default: 10. Max: 50."
      },
      role: {
        type: "string",
        description: "Filter by role: 'user', 'assistant', or 'all'. Default: 'all'",
        enum: ["user", "assistant", "all"]
      }
    },
    required: []
  },
  execute: function(input) {
    var limit = Math.min(input.limit || 10, 50);
    var roleFilter = input.role || "all";

    var result = Nanna.service("session.history", {
      limit: limit
    });

    if (!result || result.length === 0) {
      return "No conversation history available.";
    }

    // Filter by role if requested
    var messages = result;
    if (roleFilter !== "all") {
      var filtered = [];
      for (var i = 0; i < messages.length; i++) {
        if (messages[i].role === roleFilter) {
          filtered.push(messages[i]);
        }
      }
      messages = filtered;
    }

    if (messages.length === 0) {
      return "No " + roleFilter + " messages found in recent history.";
    }

    var lines = [];
    lines.push("📜 Recalled " + messages.length + " recent messages:");
    lines.push("");

    for (var i = 0; i < messages.length; i++) {
      var msg = messages[i];
      var roleIcon = msg.role === "user" ? "👤" : "🤖";
      var content = msg.content;

      // Truncate very long messages
      if (content.length > 500) {
        content = content.substring(0, 500) + "... [truncated]";
      }

      lines.push(roleIcon + " [" + msg.role + "] " + msg.timestamp);
      lines.push(content);
      lines.push("");
    }

    return lines.join("\n");
  }
}
