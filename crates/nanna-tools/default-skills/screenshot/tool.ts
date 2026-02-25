export default {
  name: "screenshot",
  description: "Take a screenshot of the current desktop or a specific window. Returns the screenshot as a base64-encoded PNG.",
  parameters: {
    type: "object",
    properties: {
      target: {
        type: "string",
        description: "What to capture: 'desktop' for full screen, or a window title to capture specific window. Default: 'desktop'"
      }
    },
    required: []
  },
  execute: function(input) {
    try {
      var result = Nanna.service("screenshot.capture", {
        target: input.target || "desktop"
      });
      return "Screenshot captured (" + (result.size || "unknown") + " bytes, base64 PNG)";
    } catch (e) {
      return "Error: Screenshot service not available. " + e;
    }
  }
}
