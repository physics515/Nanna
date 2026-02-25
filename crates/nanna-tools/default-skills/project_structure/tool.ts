export default {
  name: "project_structure",
  description: "Display the directory tree structure of a project. Shows files with sizes and line counts, useful for understanding project layout.",
  parameters: {
    type: "object",
    properties: {
      path: { type: "string", description: "Root path to display. Default: current directory" },
      max_depth: { type: "integer", description: "Maximum depth to traverse. Default: 3" }
    },
    required: []
  },
  execute: function(input) {
    var rootPath = input.path || ".";
    var maxDepth = input.max_depth || 3;

    var tree = buildTree(rootPath, 0, maxDepth);
    var totalFiles = countFiles(tree);
    var totalDirs = countDirs(tree);

    var output = formatTree(tree, "");
    output += "\n\n" + totalDirs + " directories, " + totalFiles + " files";

    return output;
  }
}

function buildTree(dirPath, depth, maxDepth) {
  var parts = dirPath.split(/[\/\\]/);
  var node = { name: parts[parts.length - 1] || dirPath, type: "dir", children: [] };

  if (depth >= maxDepth) {
    node.children = [{ name: "...", type: "truncated", children: [] }];
    return node;
  }

  var entries;
  try {
    entries = Nanna.listDir(dirPath, false);
  } catch (e) {
    return node;
  }

  entries.sort(function(a, b) {
    if (a.entry_type === "dir" && b.entry_type !== "dir") return -1;
    if (a.entry_type !== "dir" && b.entry_type === "dir") return 1;
    if (a.name < b.name) return -1;
    if (a.name > b.name) return 1;
    return 0;
  });

  for (var i = 0; i < entries.length; i++) {
    var entry = entries[i];
    if (entry.entry_type === "dir") {
      var childPath = dirPath + "/" + entry.name;
      var child = buildTree(childPath, depth + 1, maxDepth);
      node.children.push(child);
    } else {
      var fileNode = {
        name: entry.name,
        type: "file",
        size: entry.size,
        children: []
      };

      if (entry.size < 1024 * 1024 && isTextFile(entry.name)) {
        try {
          var content = Nanna.readFile(dirPath + "/" + entry.name);
          fileNode.lines = content.split("\n").length;
        } catch (e) {
          // ignore
        }
      }

      node.children.push(fileNode);
    }
  }

  return node;
}

function formatTree(node, prefix) {
  var lines = [];

  if (node.type === "dir") {
    lines.push(prefix + node.name + "/");
  }

  for (var i = 0; i < node.children.length; i++) {
    var child = node.children[i];
    var isLast = i === node.children.length - 1;
    var connector = isLast ? "\u2514\u2500\u2500 " : "\u251C\u2500\u2500 ";
    var childPrefix = isLast ? "    " : "\u2502   ";

    if (child.type === "truncated") {
      lines.push(prefix + connector + "...");
    } else if (child.type === "dir") {
      var subtree = formatTree(child, prefix + childPrefix);
      lines.push(prefix + connector + child.name + "/");
      var subLines = subtree.split("\n").slice(1);
      for (var j = 0; j < subLines.length; j++) {
        lines.push(subLines[j]);
      }
    } else {
      var size = formatSize(child.size);
      var lineInfo = child.lines ? " (" + child.lines + "L)" : "";
      lines.push(prefix + connector + child.name + "  " + size + lineInfo);
    }
  }

  return lines.join("\n");
}

function countFiles(node) {
  var count = 0;
  for (var i = 0; i < node.children.length; i++) {
    if (node.children[i].type === "file") count++;
    else if (node.children[i].type === "dir") count += countFiles(node.children[i]);
  }
  return count;
}

function countDirs(node) {
  var count = 0;
  for (var i = 0; i < node.children.length; i++) {
    if (node.children[i].type === "dir") {
      count++;
      count += countDirs(node.children[i]);
    }
  }
  return count;
}

function formatSize(bytes) {
  if (bytes < 1024) return bytes + "B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + "KB";
  return (bytes / (1024 * 1024)).toFixed(1) + "MB";
}

function isTextFile(name) {
  var textExts = [
    "rs", "ts", "js", "tsx", "jsx", "py", "go", "java", "c", "h", "cpp",
    "hpp", "cs", "rb", "php", "swift", "kt", "scala", "lua", "sh", "bash",
    "zsh", "fish", "ps1", "bat", "cmd", "toml", "yaml", "yml", "json",
    "xml", "html", "css", "scss", "less", "md", "txt", "cfg", "ini", "env",
    "vue", "svelte", "sql", "graphql", "proto", "dockerfile", "makefile"
  ];
  var ext = name.split(".").pop().toLowerCase();
  var basename = name.split(/[\/\\]/).pop().toLowerCase();
  return textExts.indexOf(ext) >= 0 || ["makefile", "dockerfile", "readme", "license"].indexOf(basename) >= 0;
}
