export default {
  name: "explore",
  version: "0.1.0",
  output: "context",
  description: "Explore a directory and summarize its contents. Provides a quick overview of project structure, file types, and sizes.",
  parameters: {
    type: "object",
    properties: {
      path: { type: "string", description: "Directory path to explore. Default: current directory" },
      depth: { type: "integer", description: "How deep to traverse. Default: 2" }
    },
    required: []
  },
  execute: function(input) {
    var dirPath = input.path || ".";
    var maxDepth = input.depth || 2;

    var entries = Nanna.listDir(dirPath, true);
    if (!entries || entries.length === 0) {
      return "Empty or inaccessible directory: " + dirPath;
    }

    var dirs = 0;
    var files = 0;
    var totalSize = 0;
    var extCounts = {};

    for (var i = 0; i < entries.length; i++) {
      var e = entries[i];
      // Approximate depth by counting path separators
      var parts = e.name.replace(/\\/g, "/").split("/");
      if (parts.length > maxDepth + 1) continue;

      if (e.entry_type === "dir") {
        dirs++;
      } else {
        files++;
        totalSize += e.size || 0;
        var ext = e.name.split(".").pop().toLowerCase();
        if (ext && ext !== e.name) {
          extCounts[ext] = (extCounts[ext] || 0) + 1;
        }
      }
    }

    var lines = [];
    lines.push("# " + dirPath);
    lines.push(dirs + " directories, " + files + " files, " + formatSize(totalSize) + " total");
    lines.push("");

    // Top file types
    var extList = [];
    for (var ext in extCounts) {
      extList.push({ ext: ext, count: extCounts[ext] });
    }
    extList.sort(function(a, b) { return b.count - a.count; });

    if (extList.length > 0) {
      lines.push("File types:");
      var top = extList.slice(0, 10);
      for (var j = 0; j < top.length; j++) {
        lines.push("  ." + top[j].ext + ": " + top[j].count + " files");
      }
    }

    return lines.join("\n");
  }
}

function formatSize(bytes) {
  if (bytes < 1024) return bytes + "B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + "KB";
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + "MB";
  return (bytes / (1024 * 1024 * 1024)).toFixed(1) + "GB";
}
