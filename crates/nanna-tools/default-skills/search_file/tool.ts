export default {
  name: "search_file",
  version: "0.1.1",
  output: "context",
  description: "Search within a single file for a pattern and return matching lines with surrounding context. Useful for finding specific functions, variables, or text in large files without reading the entire file. Returns line numbers so you can follow up with read_file for a broader view.",
  parameters: {
    type: "object",
    properties: {
      file_path: { type: "string", description: "Path to the file to search. Relative paths are resolved against the workspace directory." },
      pattern: { type: "string", description: "Search pattern (regex supported, case-insensitive by default)" },
      context_lines: { type: "integer", description: "Number of lines to show before and after each match. Default: 3" },
      max_results: { type: "integer", description: "Maximum number of matches to return. Default: 20" },
      case_sensitive: { type: "boolean", description: "Whether the search should be case-sensitive. Default: false" }
    },
    required: ["file_path", "pattern"]
  },
  execute: function(input) {
    var filePath = input.file_path || input.filePath || input.path || input.file || input.filename;
    if (!filePath) throw "Missing required parameter: file_path";

    var searchPattern = input.pattern || input.query || input.search || input.keyword;
    if (!searchPattern) throw "Missing required parameter: pattern";

    var ctx = input.context_lines !== undefined && input.context_lines !== null ? input.context_lines : 3;
    var maxResults = input.max_results || input.maxResults || 20;
    var flags = input.case_sensitive ? "" : "i";

    // Validate regex
    var regex;
    try {
      regex = new RegExp(searchPattern, flags);
    } catch (e) {
      return "Error: Invalid regex pattern: " + e.message;
    }

    // Check file size. A missing file is a structured answer, not a throw
    // (thrown errors reach the model under "Execution failed:" prefixes).
    var MAX_SIZE = 10 * 1024 * 1024;
    var stat;
    try {
      stat = Nanna.stat(filePath);
    } catch (e) {
      return {
        content: "search_file: '" + filePath + "' does not exist (or is unreadable). Nothing was searched. Check the path with exec `ls` first.",
        success: false
      };
    }
    if (stat.size > MAX_SIZE) {
      return "Error: File is too large (" + (stat.size / 1024 / 1024).toFixed(1) + "MB). Maximum is 10MB.";
    }

    // Read and search
    var content = Nanna.readFile(filePath);

    // Skip binary files
    if (content.substring(0, 512).indexOf("\0") >= 0) {
      return "Error: File appears to be binary.";
    }

    var lines = content.split("\n");
    var totalLines = lines.length;
    var matchIndices = [];

    for (var i = 0; i < lines.length; i++) {
      if (regex.test(lines[i])) {
        matchIndices.push(i);
        if (matchIndices.length >= maxResults) break;
      }
    }

    if (matchIndices.length === 0) {
      return "No matches for \"" + searchPattern + "\" in " + filePath + " (" + totalLines + " lines searched)";
    }

    // Build output with context, merging overlapping regions
    var padLen = String(totalLines).length;
    var sections = [];
    var lastShownEnd = -1;

    for (var mi = 0; mi < matchIndices.length; mi++) {
      var matchIdx = matchIndices[mi];
      var start = Math.max(0, matchIdx - ctx);
      var end = Math.min(totalLines - 1, matchIdx + ctx);

      // If this region overlaps with the previous, merge them
      if (start <= lastShownEnd + 1 && sections.length > 0) {
        start = lastShownEnd + 1;
        if (start > end) continue; // already shown
        var section = sections[sections.length - 1];
        for (var i = start; i <= end; i++) {
          var marker = i === matchIdx ? " >" : "  ";
          var lineNum = String(i + 1);
          while (lineNum.length < padLen) lineNum = " " + lineNum;
          section.push(marker + " " + lineNum + " | " + lines[i]);
        }
      } else {
        // New section
        if (sections.length > 0) {
          sections[sections.length - 1].push("  " + repeat("·", padLen + 3));
        }
        var section = [];
        for (var i = start; i <= end; i++) {
          var marker = i === matchIdx ? " >" : "  ";
          var lineNum = String(i + 1);
          while (lineNum.length < padLen) lineNum = " " + lineNum;
          section.push(marker + " " + lineNum + " | " + lines[i]);
        }
        sections.push(section);
      }
      lastShownEnd = end;
    }

    var output = "Found " + matchIndices.length + " match" + (matchIndices.length > 1 ? "es" : "") + " in " + filePath + " (" + totalLines + " lines)\n\n";
    for (var si = 0; si < sections.length; si++) {
      output += sections[si].join("\n") + "\n";
    }

    if (matchIndices.length >= maxResults) {
      output += "\n(Showing first " + maxResults + " matches — use max_results to see more)";
    }

    return output;
  }
}

function repeat(ch, n) {
  var s = "";
  for (var i = 0; i < n; i++) s += ch;
  return s;
}
