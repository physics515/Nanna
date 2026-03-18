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
    // Accept multiple parameter name variants from different models
    var filePath = input.file_path || input.filePath || input.path || input.file || input.filename;
    if (!filePath) throw "Missing required parameter: file_path";
    var fileContent = input.content;
    if (fileContent === undefined || fileContent === null) throw "Missing required parameter: content";
    Nanna.writeFile(filePath, fileContent);
    var bytes = fileContent.length;
    return "Wrote " + bytes + " bytes to " + filePath;
  }
}
