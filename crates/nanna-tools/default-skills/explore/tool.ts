export default {
  name: "explore",
  version: "0.2.1",
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
    var dirPath = String(input.path || ".");
    var maxDepth = parseInt(input.depth, 10);
    if (!isFinite(maxDepth) || maxDepth < 1) {
      maxDepth = 2;
    }

    // Entry budget, derived from the output constraint (not a magic cap):
    // this tool declares output:"context", so its result is inlined into the
    // model's context and truncated at the boundary around 2KB
    // (context_result_threshold). A visited entry can only reach that 2KB as
    // an aggregate count or a listed top-level name, and counts stop being
    // decision-relevant past 3 significant digits — so once 1000 entries are
    // tallied, further walking cannot change what the model reads; it only
    // burns the 30s script deadline (each entry is a per-object Boa marshal,
    // which is what timed out the old whole-workspace recursive walk).
    var MAX_ENTRIES = 1000;

    // Heavy/noise directories are counted but never descended into — in a
    // real workspace node_modules/.git/target alone hold hundreds of
    // thousands of entries. Mirrors the bridge's recursive-walk ignore list.
    // Every skip is announced in the output.
    var SKIP_DIRS = [
      "node_modules", ".git", "target", "dist", "build", ".venv", "venv",
      "__pycache__", ".next", ".nuxt", ".cache"
    ];

    // Top-level listing bound: of the ~2KB context budget, the fixed
    // sections (header, counts, file types, notes) use roughly 600B,
    // leaving ~1400B; at ~35B per name line that is ~40 lines.
    var MAX_LISTED = 40;

    var visited = 0;
    var capped = false;
    var dirs = 0;
    var files = 0;
    var totalSize = 0;
    var extCounts = {};
    var skippedCounts = {};
    var skippedAny = false;
    var unreadable = 0;
    var topLevel = [];

    // Level-by-level walk with bounded, NON-recursive listings. The FIFO
    // queue finishes each depth before starting the next, so when the entry
    // budget trips it is the deepest (least informative) entries that drop.
    var queue = [{ path: dirPath, depth: 1 }];
    while (queue.length > 0 && !capped) {
      var cur = queue.shift();
      var entries;
      try {
        // Request one more than the remaining budget: an overflow-length
        // return proves the directory holds more than we will visit.
        entries = Nanna.listDir(cur.path, false, MAX_ENTRIES - visited + 1);
      } catch (e) {
        if (cur.depth === 1) {
          // Root itself is unreadable: structured failure, never a throw
          // (thrown script errors reach the model under scary prefixes).
          // The commonest cause is a FILE path, whose os error ("The
          // directory name is invalid", ENOTDIR) teaches nothing — one stat,
          // on an already-failed call, turns it into the correction. The stat
          // has its own try so this error path can never itself throw.
          if (pathIsFile(dirPath)) {
            return {
              content: "explore: \"" + dirPath + "\" exists but is a FILE, not a directory — " +
                "use read_file to see its contents (or search_file to search it). " +
                "Nothing was explored.",
              success: false
            };
          }
          return {
            content: "explore failed: cannot list \"" + dirPath + "\": " + String(e) +
              ". Check that the path exists, is a directory, and is readable.",
            success: false
          };
        }
        unreadable++;
        continue;
      }

      for (var i = 0; i < entries.length; i++) {
        if (visited >= MAX_ENTRIES) {
          capped = true;
          break;
        }
        visited++;
        var e = entries[i];
        var name = String(e.name);
        var isDir = e.entry_type === "dir";
        if (isDir) {
          dirs++;
          if (SKIP_DIRS.indexOf(name) >= 0) {
            skippedCounts[name] = (skippedCounts[name] || 0) + 1;
            skippedAny = true;
          } else if (cur.depth < maxDepth) {
            queue.push({ path: cur.path + "/" + name, depth: cur.depth + 1 });
          }
        } else {
          files++;
          totalSize += e.size || 0;
          var dot = name.lastIndexOf(".");
          if (dot > 0 && dot < name.length - 1) {
            var ext = name.slice(dot + 1).toLowerCase();
            extCounts[ext] = (extCounts[ext] || 0) + 1;
          }
        }
        if (cur.depth === 1) {
          topLevel.push(isDir ? name + "/" : name);
        }
      }
    }

    if (visited === 0) {
      return {
        content: "# " + dirPath + "\nEmpty directory (0 entries).",
        success: true
      };
    }

    var lines = [];
    lines.push("# " + dirPath + " (depth " + maxDepth + ")");
    lines.push(
      dirs + " directories, " + files + " files, " + formatSize(totalSize) +
      " total" + (capped ? " (partial — see note below)" : "")
    );
    lines.push("");

    if (topLevel.length > 0) {
      topLevel.sort(function(a, b) {
        var aDir = a.charAt(a.length - 1) === "/";
        var bDir = b.charAt(b.length - 1) === "/";
        if (aDir !== bDir) return aDir ? -1 : 1;
        return a < b ? -1 : a > b ? 1 : 0;
      });
      lines.push("Top-level:");
      var shown = topLevel.slice(0, MAX_LISTED);
      for (var t = 0; t < shown.length; t++) {
        lines.push("  " + shown[t]);
      }
      if (topLevel.length > MAX_LISTED) {
        lines.push("  ... +" + (topLevel.length - MAX_LISTED) + " more (listing capped to fit the context budget)");
      }
      lines.push("");
    }

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
      lines.push("");
    }

    if (skippedAny) {
      var skipParts = [];
      for (var s = 0; s < SKIP_DIRS.length; s++) {
        var sk = SKIP_DIRS[s];
        if (skippedCounts[sk]) {
          skipParts.push(skippedCounts[sk] > 1 ? sk + " x" + skippedCounts[sk] : sk);
        }
      }
      lines.push("Skipped noise dirs (counted, never descended): " + skipParts.join(", "));
    }
    if (unreadable > 0) {
      lines.push("Note: " + unreadable + " subdirector" + (unreadable === 1 ? "y" : "ies") + " could not be read and were skipped.");
    }
    if (capped) {
      lines.push(
        "NOTE: stopped at " + MAX_ENTRIES + " entries; counts above cover only the entries visited " +
        "(shallowest first) and more exist beyond them. Narrow with path/depth for a complete view."
      );
    }

    return { content: lines.join("\n"), success: true };
  }
}

// Is this path a file? Asked only on a path that already failed as a
// directory, so it costs one metadata call on a path the tool was going to
// give up on anyway. The stat sits in its OWN try: an error path that throws
// is worse than the error it was explaining.
function pathIsFile(path) {
  try {
    var st = Nanna.stat(path);
    return !!(st && st.is_file);
  } catch (e) {
    return false;
  }
}

function formatSize(bytes) {
  if (bytes < 1024) return bytes + "B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + "KB";
  if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + "MB";
  return (bytes / (1024 * 1024 * 1024)).toFixed(1) + "GB";
}
