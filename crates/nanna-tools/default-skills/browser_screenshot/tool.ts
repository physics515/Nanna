export default {
  name: "browser_screenshot",
  description: "Take a screenshot of a web page. Returns the screenshot as a base64-encoded PNG.",
  parameters: {
    type: "object",
    properties: {
      url: { type: "string", description: "URL of the page to screenshot" },
      full_page: { type: "boolean", description: "Capture full page (scroll). Default: false" },
      width: { type: "integer", description: "Viewport width in pixels. Default: 1280" },
      height: { type: "integer", description: "Viewport height in pixels. Default: 720" }
    },
    required: ["url"]
  },
  execute: function(input) {
    try {
      var result = Nanna.service("browser.screenshot", {
        url: input.url,
        full_page: input.full_page || false,
        width: input.width || 1280,
        height: input.height || 720
      });
      return "Screenshot captured (" + (result.size || "unknown") + " bytes, base64 PNG)";
    } catch (e) {
      return "Error: Browser service not available. " + e;
    }
  }
}
