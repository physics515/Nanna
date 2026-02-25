export default {
  name: "list_reminders",
  description: "List all active reminders showing their messages and remaining time.",
  parameters: {
    type: "object",
    properties: {},
    required: []
  },
  execute: function(input) {
    try {
      var reminders = Nanna.service("schedule.list", {});
      if (!reminders || reminders.length === 0) {
        return "No active reminders.";
      }

      var lines = [];
      for (var i = 0; i < reminders.length; i++) {
        var r = reminders[i];
        var remaining = r.remaining_secs;
        var mins = Math.floor(remaining / 60);
        var secs = remaining % 60;
        var timeStr = mins > 0 ? mins + "m " + secs + "s" : secs + "s";
        lines.push("[" + r.id + "] \"" + r.message + "\" - " + timeStr + " remaining");
      }

      return "Active reminders (" + reminders.length + "):\n\n" + lines.join("\n");
    } catch (e) {
      return "Error: Schedule service not available. " + e;
    }
  }
}
