export default {
  name: "text_to_speech",
  version: "0.1.0",
  description: "Convert text to speech audio using a TTS service. Returns the audio as a base64-encoded string.",
  parameters: {
    type: "object",
    properties: {
      text: { type: "string", description: "Text to convert to speech" },
      voice: { type: "string", description: "Voice to use. Default: 'alloy'" }
    },
    required: ["text"]
  },
  execute: function(input) {
    try {
      var result = Nanna.service("audio.tts", {
        text: input.text,
        voice: input.voice || "alloy"
      });
      return "Generated audio (" + (result.size || "unknown size") + " bytes, base64 encoded)";
    } catch (e) {
      return "Error: TTS service not available. " + e;
    }
  }
}
