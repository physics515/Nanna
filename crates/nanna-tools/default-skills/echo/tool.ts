export default {
  name: "echo",
  description: "Echo text back. Useful for testing or returning computed values to the user.",
  parameters: {
    type: "object",
    properties: {
      text: { type: "string", description: "Text to echo back" }
    },
    required: ["text"]
  },
  execute({ text }) {
    return text;
  }
}
