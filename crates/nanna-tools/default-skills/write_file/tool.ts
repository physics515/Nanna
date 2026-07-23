export default {
  name: "write_file",
  version: "0.1.1",
  output: "context",
  description: "Write content to a file. BOTH parameters are REQUIRED on every call: file_path AND content (the complete file text). A call without content does nothing and fails. Creates the file if it doesn't exist, overwrites if it does. SAFETY: the write is blocked if new content is less than 30% of the existing file size (likely truncation). Use force=true to override.",
  parameters: {
    type: "object",
    properties: {
      file_path: { type: "string", description: "REQUIRED. Path to the file to write. Relative paths are resolved against the workspace directory." },
      content: { type: "string", description: "REQUIRED. The complete text to write into the file. Never omit this — a write_file call without content always fails." },
      force: { type: "boolean", description: "If true, skip the truncation safety check. Use when you intentionally want to write a smaller file." }
    },
    required: ["file_path", "content"]
  },
  execute: function(input) {
    // Accept multiple parameter name variants from different models
    var filePath = input.file_path || input.filePath || input.path || input.file || input.filename;
    var fileContent = input.content || input.text || input.data || input.new_content || input.file_content;
    if (!filePath && (fileContent === undefined || fileContent === null)) {
      throw "write_file failed: you must pass BOTH file_path AND content. Call it again like: write_file(file_path=\"D:\\\\path\\\\to\\\\file.py\", content=\"<the complete file text>\")";
    }
    if (!filePath) {
      throw "write_file failed: missing file_path. Call it again with BOTH file_path (the destination path) AND content.";
    }
    if (fileContent === undefined || fileContent === null) {
      throw "write_file failed: missing content. Nothing was written. Call write_file again with file_path=\"" + filePath + "\" AND content set to the COMPLETE file text.";
    }

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
