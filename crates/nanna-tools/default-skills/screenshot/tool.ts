export default {
  name: "screenshot",
  requires: ["screenshot.capture"],
  version: "0.1.0",
  output: "memory",
  description: "Take a screenshot of the whole desktop and save it as a PNG, returning the file path. Capturing a single window is not supported — use browser_screenshot for a specific web page.",
  parameters: {
    type: "object",
    properties: {
      target: {
        type: "string",
        output: "context",
  description: "Only 'desktop' (the whole screen) is supported, which is the default. A window title is refused rather than served as a full-screen grab.",
        enum: ["desktop"]
      }
    },
    required: []
  },
  execute: function(input) {
    try {
      var result = Nanna.service("screenshot.capture", {
        target: input.target || "desktop"
      });
      // The service writes the PNG and reports where. Returning only a byte
      // count meant the daemon took a screenshot nobody could look at.
      if (result && result.path) {
        return "Captured the desktop with " + (result.tool || "a capture tool") +
          " and saved it to " + result.path + " (" + (result.size || "unknown") + " bytes).";
      }
      return "Screenshot captured (" + ((result && result.size) || "unknown") +
        " bytes) but the service did not report where it was saved.";
    } catch (e) {
      return "Error: Screenshot service not available. " + e;
    }
  }
}
