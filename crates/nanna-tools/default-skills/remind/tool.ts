export default {
  name: "remind",
  requires: ["schedule.add"],
  version: "0.2.0",
  description: "Set a reminder that will fire after a specified delay. When it comes due, the message is posted into this conversation (checked every 30 seconds, so it may arrive up to 30s late). It survives a restart of Nanna.",
  parameters: {
    type: "object",
    properties: {
      message: { type: "string", description: "Reminder message" },
      delay_secs: { type: "integer", description: "Delay in seconds before the reminder fires" }
    },
    required: ["message", "delay_secs"]
  },
  execute: function(input) {
    try {
      // The session comes from the run's own binding, never from the model:
      // it is where the reminder will be delivered.
      var result = Nanna.service("schedule.add", {
        message: input.message,
        delay_secs: input.delay_secs,
        session_id: Nanna.sessionId()
      });
      var mins = Math.floor(result.delay_secs / 60);
      var secs = result.delay_secs % 60;
      var timeStr = mins > 0 ? mins + "m " + secs + "s" : secs + "s";
      return "Reminder set (id: " + result.id + "): \"" + input.message + "\" in " + timeStr +
        " (due " + result.fire_at + "). It will be posted into this conversation.";
    } catch (e) {
      // The service's refusal already says what went wrong and that nothing
      // was set; pass it through rather than guessing at a cause.
      var msg = "" + (e && e.message ? e.message : e);
      return { content: "remind: " + msg, success: false };
    }
  }
}
