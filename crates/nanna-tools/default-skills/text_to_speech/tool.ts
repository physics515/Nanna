export default {
  name: "text_to_speech",
  requires: ["audio.tts"],
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
      // The service writes the clip and reports where. Saying only the byte
      // count told the model an API call had happened and nothing about how to
      // reach the audio, which made the whole tool a no-op from its caller's
      // point of view.
      if (result && result.path) {
        return "Spoke it in voice '" + (result.voice || "default") + "' and saved the audio to " +
          result.path + " (" + (result.size || "unknown") + " bytes).";
      }
      return "Generated audio (" + ((result && result.size) || "unknown size") +
        " bytes) but the service did not report where it was saved.";
    } catch (e) {
      return "Error: TTS service not available. " + e;
    }
  }
}
