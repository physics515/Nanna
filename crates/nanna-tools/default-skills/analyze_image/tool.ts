export default {
  name: "analyze_image",
  description: "Analyze an image using a vision model. Describe contents, answer questions, or extract information from images.",
  parameters: {
    type: "object",
    properties: {
      path: { type: "string", description: "Path to the image file" },
      prompt: { type: "string", description: "What to analyze or ask about the image. Default: 'Describe this image in detail'" }
    },
    required: ["path"]
  },
  execute: function(input) {
    try {
      var result = Nanna.service("vision.analyze", {
        path: input.path,
        prompt: input.prompt || "Describe this image in detail"
      });
      return result.text || result.description || "(no analysis returned)";
    } catch (e) {
      return "Error: Vision service not available. " + e;
    }
  }
}
