export default {
  name: "remind",
  description: "Set a reminder that will fire after a specified delay. The message will be injected into the conversation when the timer expires.",
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
      var result = Nanna.service("schedule.add", {
        message: input.message,
        delay_secs: input.delay_secs
      });
      var mins = Math.floor(input.delay_secs / 60);
      var secs = input.delay_secs % 60;
      var timeStr = mins > 0 ? mins + "m " + secs + "s" : secs + "s";
      return "Reminder set (id: " + result.id + "): \"" + input.message + "\" in " + timeStr;
    } catch (e) {
      return "Error: Schedule service not available. " + e;
    }
  }
}
