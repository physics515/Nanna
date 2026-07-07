export default {
  name: "transcribe",
  version: "0.1.0",
  output: "context",
  description: "Transcribe audio to text using a speech-to-text service.",
  parameters: {
    type: "object",
    properties: {
      path: { type: "string", description: "Path to audio file (mp3, wav, m4a, etc.)" },
      language: { type: "string", description: "Language code (e.g. 'en'). Default: auto-detect" }
    },
    required: ["path"]
  },
  execute: function(input) {
    try {
      var result = Nanna.service("audio.transcribe", {
        path: input.path,
        language: input.language
      });
      return "Transcription:\n\n" + (result.text || "(empty)");
    } catch (e) {
      return "Error: Transcription service not available. " + e;
    }
  }
}
