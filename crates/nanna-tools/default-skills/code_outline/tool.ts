export default {
  name: "code_outline",
  description: "Generate a structural outline of a source file showing functions, classes, structs, and other definitions. Useful for understanding file structure without reading the entire file.",
  parameters: {
    type: "object",
    properties: {
      path: { type: "string", description: "Path to the source file" }
    },
    required: ["path"]
  },
  execute: function(input) {
    var content = Nanna.readFile(input.path);
    var lines = content.split("\n");
    var ext = input.path.split(".").pop().toLowerCase();

    var patterns = getPatterns(ext);
    var outline = [];

    for (var i = 0; i < lines.length; i++) {
      var line = lines[i];
      for (var j = 0; j < patterns.length; j++) {
        if (patterns[j].test(line)) {
          outline.push((i + 1) + ": " + line.trimEnd());
          break;
        }
      }
    }

    var pct = lines.length > 0 ? Math.round((1 - outline.length / lines.length) * 100) : 0;
    var header = "# Outline of " + input.path + " (" + lines.length + " -> " + outline.length + " lines, " + pct + "% reduction)\n";

    return header + outline.join("\n");
  }
}

function getPatterns(ext) {
  switch (ext) {
    case "rs":
      return [
        /^\s*(pub\s+)?(async\s+)?fn\s+/,
        /^\s*(pub\s+)?(struct|enum|trait|impl|type|const|static|mod|use)\s+/,
        /^\s*#\[derive/,
        /^\s*#\[cfg/
      ];
    case "py":
      return [
        /^\s*(async\s+)?def\s+/,
        /^\s*class\s+/,
        /^import\s+/,
        /^from\s+.*import\s+/,
        /^\s*@\w+/
      ];
    case "ts":
    case "tsx":
    case "js":
    case "jsx":
      return [
        /^\s*(export\s+)?(default\s+)?(async\s+)?function\s+/,
        /^\s*(export\s+)?(default\s+)?class\s+/,
        /^\s*(export\s+)?(const|let|var)\s+\w+\s*[:=]\s*(async\s+)?\(/,
        /^\s*(export\s+)?interface\s+/,
        /^\s*(export\s+)?type\s+\w+\s*=/,
        /^\s*(export\s+)?enum\s+/,
        /^\s*import\s+/
      ];
    case "go":
      return [
        /^func\s+/,
        /^type\s+\w+\s+(struct|interface)/,
        /^package\s+/,
        /^import\s+/,
        /^var\s+/,
        /^const\s+/
      ];
    default:
      return [
        /^\s*(pub\s+)?(fn|function|def|class|struct|enum|trait|impl|type|interface|module|package)\s+/,
        /^\s*(import|from|use|require)\s+/
      ];
  }
}
