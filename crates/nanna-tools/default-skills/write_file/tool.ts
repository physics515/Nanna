export default {
  name: "write_file",
  description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does.",
  parameters: {
    type: "object",
    properties: {
      file_path: { type: "string", description: "The absolute path to the file to write" },
      content: { type: "string", description: "Content to write to the file" }
    },
    required: ["file_path", "content"]
  },
  execute: function(input) {
    // Accept both file_path (Claude Code convention) and path (legacy)
    var filePath = input.file_path || input.path;
    if (!filePath) throw "Missing required parameter: file_path";
    var fileContent = input.content;
    if (fileContent === undefined || fileContent === null) throw "Missing required parameter: content";
    Nanna.writeFile(filePath, fileContent);
    var bytes = fileContent.length;
    return "Wrote " + bytes + " bytes to " + filePath;
  }
}
