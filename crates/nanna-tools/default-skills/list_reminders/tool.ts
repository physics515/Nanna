export default {
  name: "list_reminders",
  requires: ["schedule.list"],
  version: "0.2.0",
  description: "List all pending reminders, soonest first, showing their messages and remaining time.",
  parameters: {
    type: "object",
    properties: {},
    required: []
  },
  execute: function(input) {
    try {
      var reminders = Nanna.service("schedule.list", {});
      if (!reminders || reminders.length === 0) {
        return "No pending reminders.";
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

      return "Pending reminders (" + reminders.length + "):\n\n" + lines.join("\n");
    } catch (e) {
      var msg = "" + (e && e.message ? e.message : e);
      return { content: "list_reminders: " + msg, success: false };
    }
  }
}
