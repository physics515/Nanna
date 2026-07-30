export default {
  name: "explore",
  version: "0.1.1",
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

    // Recursive listDir returns FULL paths (walkdir entry.path()), not names
    // relative to the listed root, so depth must be measured against the root.
    // Counting raw separators would exceed maxDepth for every entry once the
    // root itself sits a few directories deep.

    // Normalize the requested root for prefix matching: forward slashes, no
    // trailing slash, no leading "./". A bare "." resolves daemon-side to the
    // workdir, whose absolute location this script cannot see, so it yields
    // no usable prefix.
    var rootPrefix = String(dirPath).replace(/\\/g, "/");
    while (rootPrefix.length > 1 && rootPrefix.charAt(rootPrefix.length - 1) === "/") {
      rootPrefix = rootPrefix.slice(0, rootPrefix.length - 1);
    }
    while (rootPrefix.lastIndexOf("./", 0) === 0) {
      rootPrefix = rootPrefix.slice(2);
    }
    if (rootPrefix === "." || rootPrefix === "/") {
      rootPrefix = "";
    }

    var names = [];
    var i;
    for (i = 0; i < entries.length; i++) {
      names.push(String(entries[i].name).replace(/\\/g, "/"));
    }

    // Make names root-relative. Strip the root when it literally prefixes
    // every entry (an absolute root was passed). Otherwise fall back to the
    // shallowest entry as the depth-1 baseline: the walk always yields the
    // root's direct children, so the minimum segment count is depth 1. The
    // baseline covers "." roots, relative roots resolved daemon-side, and
    // listings that already come back root-relative.
    var allPrefixed = rootPrefix.length > 0;
    for (i = 0; allPrefixed && i < names.length; i++) {
      if (names[i].lastIndexOf(rootPrefix + "/", 0) !== 0) {
        allPrefixed = false;
      }
    }

    var depths = [];
    if (allPrefixed) {
      for (i = 0; i < names.length; i++) {
        names[i] = names[i].slice(rootPrefix.length + 1);
        depths.push(names[i].split("/").length);
      }
    } else {
      var segCounts = [];
      var minSegs = -1;
      for (i = 0; i < names.length; i++) {
        segCounts.push(names[i].split("/").length);
        if (minSegs === -1 || segCounts[i] < minSegs) {
          minSegs = segCounts[i];
        }
      }
      for (i = 0; i < names.length; i++) {
        depths.push(segCounts[i] - minSegs + 1);
      }
    }

    var dirs = 0;
    var files = 0;
    var totalSize = 0;
    var extCounts = {};

    for (i = 0; i < entries.length; i++) {
      if (depths[i] > maxDepth) continue;

      var e = entries[i];
      if (e.entry_type === "dir") {
        dirs++;
      } else {
        files++;
        totalSize += e.size || 0;
        var base = names[i];
        var slash = base.lastIndexOf("/");
        if (slash >= 0) {
          base = base.slice(slash + 1);
        }
        var dot = base.lastIndexOf(".");
        if (dot > 0 && dot < base.length - 1) {
          var ext = base.slice(dot + 1).toLowerCase();
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
    for (var extName in extCounts) {
      extList.push({ ext: extName, count: extCounts[extName] });
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
