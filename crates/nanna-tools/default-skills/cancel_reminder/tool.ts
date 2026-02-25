export default {
  name: "cancel_reminder",
  description: "Cancel an active reminder by its ID.",
  parameters: {
    type: "object",
    properties: {
      id: { type: "string", description: "ID of the reminder to cancel" }
    },
    required: ["id"]
  },
  execute: function(input) {
    try {
      var result = Nanna.service("schedule.cancel", { id: input.id });
      if (result && result.cancelled) {
        return "Cancelled reminder: " + input.id;
      }
      return "Reminder not found: " + input.id;
    } catch (e) {
      return "Error: Schedule service not available. " + e;
    }
  }
}
