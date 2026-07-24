export default {
  name: "read_file",
  version: "0.1.1",
  output: "context",
  description: "Read a file from the filesystem. Returns the file contents with line numbers. Supports optional offset and limit for reading portions of large files.",
  parameters: {
    type: "object",
    properties: {
      file_path: { type: "string", description: "Path to the file to read. Relative paths are resolved against the workspace directory." },
      offset: { type: "integer", description: "Line number to start reading from (1-indexed). Default: 1" },
      limit: { type: "integer", description: "Maximum number of lines to read. Default: all lines" }
    },
    required: ["file_path"]
  },
  execute: function(input) {
    // Accept multiple parameter name variants from different models.
    // All failures are RETURNED, not thrown — a thrown error reaches the
    // model under stacked "Execution failed:" prefixes and reads as
    // corruption (observed live: an unguarded stat on a missing README).
    var filePath = input.file_path || input.filePath || input.path || input.file || input.filename;
    if (!filePath) {
      return { content: "read_file failed: missing file_path. Call it again with file_path set to the file you want to read.", success: false };
    }

    var MAX_SIZE = 10 * 1024 * 1024;

    var stat;
    try {
      stat = Nanna.stat(filePath);
    } catch (e) {
      return {
        content: "read_file: '" + filePath + "' does not exist (or is unreadable). Nothing was read. Check the path with exec `ls`, or create the file first if you meant to write it.",
        success: false
      };
    }
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
