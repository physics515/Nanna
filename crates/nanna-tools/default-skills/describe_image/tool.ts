export default {
  name: "describe_image",
  description: "Get a concise description of an image. Useful for accessibility, captioning, or quick understanding of visual content.",
  parameters: {
    type: "object",
    properties: {
      path: { type: "string", description: "Path to the image file" }
    },
    required: ["path"]
  },
  execute: function(input) {
    try {
      var result = Nanna.service("vision.analyze", {
        path: input.path,
        prompt: "Provide a brief, factual description of this image in 1-3 sentences."
      });
      return result.text || result.description || "(no description returned)";
    } catch (e) {
      return "Error: Vision service not available. " + e;
    }
  }
}
