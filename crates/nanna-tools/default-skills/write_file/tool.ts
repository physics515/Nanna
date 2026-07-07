export default {
  name: "write_file",
  version: "0.1.0",
  output: "context",
  description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. SAFETY: the write is blocked if new content is less than 30% of the existing file size (likely truncation). Use force=true to override.",
  parameters: {
    type: "object",
    properties: {
      file_path: { type: "string", description: "Path to the file to write. Relative paths are resolved against the workspace directory." },
      content: { type: "string", description: "Content to write to the file" },
      force: { type: "boolean", description: "If true, skip the truncation safety check. Use when you intentionally want to write a smaller file." }
    },
    required: ["file_path", "content"]
  },
  execute: function(input) {
    // Accept multiple parameter name variants from different models
    var filePath = input.file_path || input.filePath || input.path || input.file || input.filename;
    if (!filePath) throw "Missing required parameter: file_path";
    var fileContent = input.content || input.text || input.data || input.new_content || input.file_content;
    if (fileContent === undefined || fileContent === null) throw "Missing required parameter: content";

    var bytes = fileContent.length;

    // Safety check BEFORE writing: block if new content is drastically smaller
    // than existing file. This prevents the model from accidentally overwriting
    // a large file with truncated/compressed content when it lost context.
    if (!input.force) {
      var existingSize = 0;
      try {
        var existing = Nanna.readFile(filePath);
        if (existing) existingSize = existing.length;
      } catch (e) {
        // File doesn't exist yet, no check needed
      }

      if (existingSize > 500 && bytes < existingSize * 0.3) {
        return {
          content: "❌ BLOCKED: Refusing to write " + bytes + " bytes to " + filePath +
            " — the existing file is " + existingSize + " bytes (" +
            Math.round(bytes / existingSize * 100) + "% of original). " +
            "This looks like accidental truncation. " +
            "Either re-read the file and write the COMPLETE version, " +
            "or use force=true if you intentionally want a smaller file.",
          success: false
        };
      }
    }

    Nanna.writeFile(filePath, fileContent);

    return { content: "Wrote " + bytes + " bytes to " + filePath, written: fileContent };
  }
}
