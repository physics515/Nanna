export default {
  name: "list_dir",
  version: "0.2.1",
  output: "memory",
  description: "List files and directories in a path. Supports recursive listing.",
  parameters: {
    type: "object",
    properties: {
      path: { type: "string", description: "Directory path to list" },
      recursive: { type: "boolean", description: "List recursively. Default: false" }
    },
    required: []
  },
  execute: function(input) {
    var dirPath = String(
      input.path || input.dir || input.directory || input.file_path || input.filePath || "."
    );
    var recursive = input.recursive || false;

    // ---------------------------------------------------------------------
    // Budgets. Derived, not picked.
    //
    // OUTPUT — this tool declares output:"memory", so its result is split
    // into MEMORY_CHUNK_MAX_CHARS (3200-char) chunks and every chunk becomes
    // its own memory row: one embedding, one similarity search, one write.
    // The per-call ceiling is set once across the memory-routed listing tools
    // and taken from the largest DESIGNED result among them — code_search's
    // default invocation, ~29,000 chars ≈ 9.1 chunks, rounded up to 10 whole
    // chunks. A directory listing has no reason to cost more memory writes
    // than a full code search does.
    var MEMORY_CHUNK_CHARS = 3200;
    var MAX_OUTPUT_CHARS = 10 * MEMORY_CHUNK_CHARS;

    // ENTRIES — every entry crosses the bridge as its own JS object, and an
    // unbounded listing of a real tree (or one flat directory holding tens of
    // thousands of files) marshals all of them before the script runs a line;
    // that is what blew the 30s deadline in the explore failure. The bound
    // follows from the output budget: the shortest line this tool can emit is
    // the 6-char type prefix, a space, a one-character name and a newline —
    // 9 chars — so no listing that could fit in the budget can hold more than
    // MAX_OUTPUT_CHARS / 9 entries. Asking the bridge for more than could
    // ever be printed is pure marshalling waste.
    var MIN_LINE_CHARS = 6 + 1 + 1 + 1;
    var MAX_ENTRIES = Math.floor(MAX_OUTPUT_CHARS / MIN_LINE_CHARS);

    var entries;
    try {
      // One over the budget: an overflow-length return proves more exist.
      entries = Nanna.listDir(dirPath, recursive, MAX_ENTRIES + 1);
    } catch (e) {
      // The commonest reason a listing fails is that the path is a FILE, and
      // the os error for that ("The directory name is invalid", ENOTDIR)
      // teaches nothing — it reads as a broken tool rather than as a wrong
      // call. One stat, on a call that has ALREADY failed, turns it into the
      // correction. Structured, never thrown — a thrown script error reaches
      // the model under an "Execution failed:" prefix and reads as breakage.
      if (pathIsFile(dirPath)) {
        return {
          content: "list_dir: \"" + dirPath + "\" exists but is a FILE, not a directory — " +
            "use read_file to see its contents (or search_file to search it). Nothing was listed.",
          success: false
        };
      }
      return {
        content: "list_dir failed: cannot list \"" + dirPath + "\": " + String(e) +
          ". Check that the path exists, is a directory, and is readable.",
        success: false
      };
    }

    var entriesCapped = entries.length > MAX_ENTRIES;
    if (entriesCapped) {
      entries = entries.slice(0, MAX_ENTRIES);
    }

    if (entries.length === 0) {
      // A recursive walk over a FILE does not throw — walkdir yields the file
      // itself at depth 0 and the bridge skips the root, so the listing comes
      // back empty and "Empty directory: notes.md" is a flat falsehood about a
      // file that has contents. Same one stat, same teaching sentence, and it
      // only runs when a listing came back with nothing in it.
      if (pathIsFile(dirPath)) {
        return {
          content: "list_dir: \"" + dirPath + "\" exists but is a FILE, not a directory — " +
            "use read_file to see its contents (or search_file to search it). Nothing was listed.",
          success: false
        };
      }
      return { content: "Empty directory: " + dirPath, success: true };
    }

    entries.sort(function(a, b) {
      if (a.entry_type === "dir" && b.entry_type !== "dir") return -1;
      if (a.entry_type !== "dir" && b.entry_type === "dir") return 1;
      if (a.name < b.name) return -1;
      if (a.name > b.name) return 1;
      return 0;
    });

    var lines = [];
    var chars = 0;
    var outputCapped = false;
    for (var i = 0; i < entries.length; i++) {
      var e = entries[i];
      var type_prefix = e.entry_type === "dir" ? "[dir] " : e.entry_type === "link" ? "[link]" : "      ";
      var size = e.entry_type === "file" ? " (" + formatSize(e.size) + ")" : "";
      var line = type_prefix + " " + e.name + size;
      if (chars + line.length + 1 > MAX_OUTPUT_CHARS) {
        outputCapped = true;
        break;
      }
      lines.push(line);
      chars += line.length + 1;
    }

    // Every bound that shaped the answer says so, in the answer.
    var notes = [];
    if (recursive) {
      // The bridge's recursive walk prunes these and stops at depth 10 on its
      // own. Silent omission reads as "the tree is smaller than it is", so it
      // is stated here rather than left for the model to discover.
      notes.push("Recursive listing: node_modules, .git, target, dist, build, .venv, venv, " +
        "__pycache__, .next, .nuxt and .cache are never descended into, and the walk stops " +
        "at 10 levels deep.");
    }
    if (entriesCapped || outputCapped) {
      var total = entriesCapped ? "more than " + MAX_ENTRIES : String(entries.length);
      notes.push("LISTING TRUNCATED: showing " + lines.length + " of " + total +
        " entries — this path holds more than one listing can carry (" + MAX_ENTRIES +
        " entries, " + MAX_OUTPUT_CHARS + " chars = " +
        (MAX_OUTPUT_CHARS / MEMORY_CHUNK_CHARS) + " memory chunks). The listing SUCCEEDED " +
        "and the directory is intact; what you see is the first slice the filesystem " +
        "returned, then sorted. List a subdirectory directly for a complete view.");
    }

    var body = lines.join("\n");
    if (notes.length > 0) {
      body += "\n\n" + notes.join("\n");
    }
    return { content: body, success: true };
  }
}

// Is this path a file? Asked only on a path that already failed (or answered
// nothing) as a directory, so it costs one metadata call on a path the tool
// was going to give up on anyway. The stat sits in its OWN try: an error path
// that throws is worse than the error it was explaining.
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
  return (bytes / (1024 * 1024)).toFixed(1) + "MB";
}
