export default {
  name: "cancel_reminder",
  requires: ["schedule.cancel"],
  version: "0.2.0",
  description: "Cancel a pending reminder by its ID (list_reminders shows the IDs).",
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
      return {
        content: "cancel_reminder: no pending reminder has id " + input.id + ", so nothing was cancelled. list_reminders shows the ids that exist.",
        success: false
      };
    } catch (e) {
      var msg = "" + (e && e.message ? e.message : e);
      return { content: "cancel_reminder: " + msg, success: false };
    }
  }
}
