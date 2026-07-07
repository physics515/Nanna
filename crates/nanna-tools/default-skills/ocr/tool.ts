export default {
  name: "ocr",
  version: "0.1.0",
  output: "context",
  description: "Extract text from an image using optical character recognition. Works on screenshots, photos of documents, handwriting, etc.",
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
        prompt: "Extract ALL text visible in this image. Reproduce the text exactly as it appears, preserving layout and formatting where possible. If no text is found, say 'No text detected'."
      });
      return result.text || "(no text detected)";
    } catch (e) {
      return "Error: Vision/OCR service not available. " + e;
    }
  }
}
