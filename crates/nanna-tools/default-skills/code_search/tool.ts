export default {
  name: "code_search",
  description: "Search for a pattern across files in a directory. Returns matching lines with context. Supports regex patterns and file type filtering.",
  parameters: {
    type: "object",
    properties: {
      pattern: { type: "string", description: "Search pattern (regex supported)" },
      path: { type: "string", description: "Directory to search in. Default: current directory" },
      file_pattern: { type: "string", description: "Glob-style filter for filenames (e.g. '*.rs', '*.ts')" },
      context_lines: { type: "integer", description: "Number of context lines before/after match. Default: 2" },
      max_results: { type: "integer", description: "Maximum number of matches. Default: 50" }
    },
    required: ["pattern"]
  },
  execute: function(input) {
    var searchPattern = input.pattern || input.query || input.search || input.regex;
    if (!searchPattern) throw "Missing required parameter: pattern";
    var searchPath = input.path || input.dir || input.directory || ".";
    var ctx = input.context_lines !== undefined && input.context_lines !== null ? input.context_lines : 2;
    var maxResults = input.max_results || 50;

    var regex;
    try {
      regex = new RegExp(searchPattern, "i");
    } catch (e) {
      return "Error: Invalid regex pattern: " + e.message;
    }

    var entries = Nanna.listDir(searchPath, true);
    var files = [];
    for (var i = 0; i < entries.length; i++) {
      var e = entries[i];
      if (e.entry_type !== "file") continue;
      if (e.size >= 1024 * 1024) continue;
      if (input.file_pattern && !matchGlob(e.name, input.file_pattern)) continue;
      files.push(e);
    }

    if (files.length === 0) {
      return "No files found in \"" + searchPath + "\" (resolved from listDir). Found " + entries.length + " total entries. Try specifying an absolute path.";
    }

    var results = [];
    var totalMatches = 0;
    var readErrors = 0;

    for (var fi = 0; fi < files.length; fi++) {
      if (totalMatches >= maxResults) break;

      var content;
      try {
        content = Nanna.readFile(files[fi].name);
      } catch (err) {
        readErrors++;
        continue;
      }

      if (content.substring(0, 512).indexOf("\0") >= 0) continue;

      var lines = content.split("\n");
      var matches = [];

      for (var li = 0; li < lines.length; li++) {
        if (regex.test(lines[li])) {
          matches.push(li);
          totalMatches++;
          if (totalMatches >= maxResults) break;
        }
      }

      if (matches.length > 0) {
        results.push(formatFileMatches(files[fi].name, lines, matches, ctx));
      }
    }

    if (results.length === 0) {
      var msg = "No matches found for \"" + searchPattern + "\" in " + searchPath + " (" + files.length + " files searched)";
      if (readErrors > 0) msg += " (" + readErrors + " files failed to read)";
      return msg;
    }

    var output = results.join("\n\n");
    if (totalMatches >= maxResults) {
      output += "\n\n(Results truncated at " + maxResults + " matches)";
    }
    return output;
  }
}

function formatFileMatches(filepath, lines, matchIndices, ctx) {
  var sections = [];
  var shown = {};

  for (var mi = 0; mi < matchIndices.length; mi++) {
    var matchIdx = matchIndices[mi];
    var start = Math.max(0, matchIdx - ctx);
    var end = Math.min(lines.length - 1, matchIdx + ctx);

    if (shown[matchIdx]) continue;

    var section = [];
    for (var i = start; i <= end; i++) {
      if (shown[i]) continue;
      shown[i] = true;
      var marker = i === matchIdx ? ">" : " ";
      var lineNum = String(i + 1);
      while (lineNum.length < 4) lineNum = " " + lineNum;
      section.push(marker + lineNum + ": " + lines[i]);
    }
    sections.push(section.join("\n"));
  }

  return "=== " + filepath + " (" + matchIndices.length + " match" + (matchIndices.length > 1 ? "es" : "") + ") ===\n" + sections.join("\n  ...\n");
}

function matchGlob(name, pattern) {
  var regexStr = "^" + pattern.replace(/\./g, "\\.").replace(/\*/g, ".*").replace(/\?/g, ".") + "$";
  var regex = new RegExp(regexStr, "i");
  var basename = name.split(/[/\\]/).pop();
  return regex.test(name) || regex.test(basename);
}
