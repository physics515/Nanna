export default {
  name: "read_file",
  description: "Read a file from the filesystem. Returns the file contents with line numbers. Supports optional offset and limit for reading portions of large files.",
  parameters: {
    type: "object",
    properties: {
      file_path: { type: "string", description: "The absolute path to the file to read" },
      offset: { type: "integer", description: "Line number to start reading from (1-indexed). Default: 1" },
      limit: { type: "integer", description: "Maximum number of lines to read. Default: all lines" }
    },
    required: ["file_path"]
  },
  execute: function(input) {
    // Accept multiple parameter name variants from different models
    var filePath = input.file_path || input.filePath || input.path || input.file || input.filename;
    if (!filePath) throw "Missing required parameter: file_path";

    var MAX_SIZE = 10 * 1024 * 1024;

    var stat = Nanna.stat(filePath);
    if (stat.size > MAX_SIZE) {
      return "Error: File is too large (" + (stat.size / 1024 / 1024).toFixed(1) + "MB). Maximum is 10MB. Use offset/limit to read portions.";
    }

    var content = Nanna.readFile(filePath);
    var lines = content.split("\n");
    var totalLines = lines.length;

    var startLine = Math.max(1, input.offset || 1);
    var endLine = input.limit ? Math.min(totalLines, startLine + input.limit - 1) : totalLines;

    var numbered = [];
    for (var i = startLine - 1; i < endLine; i++) {
      var padLen = String(endLine).length;
      var lineNum = String(i + 1);
      while (lineNum.length < padLen) {
        lineNum = " " + lineNum;
      }
      numbered.push(lineNum + "\t" + lines[i]);
    }

    var result = numbered.join("\n");

    if (startLine > 1 || endLine < totalLines) {
      result += "\n\n// Showing lines " + startLine + "-" + endLine + " of " + totalLines;
    }

    return result;
  }
}
