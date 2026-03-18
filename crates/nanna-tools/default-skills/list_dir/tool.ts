export default {
  name: "list_dir",
  description: "List files and directories in a path. Supports recursive listing.",
  parameters: {
    type: "object",
    properties: {
      path: { type: "string", description: "Directory path to list" },
      recursive: { type: "boolean", description: "List recursively. Default: false" }
    },
    required: ["path"]
  },
  execute: function(input) {
    var dirPath = input.path || input.dir || input.directory || input.file_path || input.filePath;
    if (!dirPath) throw "Missing required parameter: path";
    var entries = Nanna.listDir(dirPath, input.recursive || false);

    if (entries.length === 0) {
      return "Empty directory: " + dirPath;
    }

    entries.sort(function(a, b) {
      if (a.entry_type === "dir" && b.entry_type !== "dir") return -1;
      if (a.entry_type !== "dir" && b.entry_type === "dir") return 1;
      if (a.name < b.name) return -1;
      if (a.name > b.name) return 1;
      return 0;
    });

    var lines = [];
    for (var i = 0; i < entries.length; i++) {
      var e = entries[i];
      var type_prefix = e.entry_type === "dir" ? "[dir] " : e.entry_type === "link" ? "[link]" : "      ";
      var size = e.entry_type === "file" ? " (" + formatSize(e.size) + ")" : "";
      lines.push(type_prefix + " " + e.name + size);
    }

    return lines.join("\n");
  }
}

function formatSize(bytes) {
  if (bytes < 1024) return bytes + "B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + "KB";
  return (bytes / (1024 * 1024)).toFixed(1) + "MB";
}
