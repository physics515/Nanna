export default {
  name: "project_structure",
  version: "0.2.1",
  timeout: 120,
  output: "memory",
  description: "Show the directory tree of a project: names, nesting and file sizes. Reads no file contents, so it reports no line counts - use code_search or read_file for what is inside a file. Noise dirs (node_modules, .git, target, ...) are shown but never descended into.",
  parameters: {
    type: "object",
    properties: {
      path: { type: "string", description: "Root path to display. Default: current directory" },
      max_depth: { type: "integer", description: "How many directory levels below the root to descend. Default: 3" }
    },
    required: []
  },
  execute: function(input) {
    var rootPath = String(input.path || input.dir || input.directory || ".");
    // Trim trailing separators so joined child paths never double up.
    while (rootPath.length > 1 && (rootPath.charAt(rootPath.length - 1) === "/" ||
           rootPath.charAt(rootPath.length - 1) === "\\")) {
      rootPath = rootPath.slice(0, rootPath.length - 1);
    }

    var maxDepth = parseInt(input.max_depth, 10);
    if (!isFinite(maxDepth) || maxDepth < 1) {
      maxDepth = parseInt(input.depth, 10);
    }
    if (!isFinite(maxDepth) || maxDepth < 1) {
      maxDepth = 3;
    }

    // -----------------------------------------------------------------------
    // Budgets. Each is derived from a measured constraint; none is a magic cap.
    // The throughput numbers come from the ignored measurement test in
    // nanna-scripting/tests/code_search_skill.rs, taken through the real Boa
    // bridge on a debug build - the build the daemon actually runs.
    //
    // OUTPUT - this tool declares output:"memory", so its result is not cut at
    // the ~2KB context boundary the context-routed tools face. It is split into
    // MEMORY_CHUNK_MAX_CHARS (3200-char) chunks and EVERY chunk becomes its own
    // memory row: one embedding, one similarity search, one write. So result
    // size has a cost denominated in chunks. The per-call ceiling is set once
    // across the memory-routed listing tools and taken from the largest
    // DESIGNED result among them - code_search's default invocation, ~29,000
    // chars = 9.1 chunks, rounded up to 10 whole chunks. A directory tree has
    // no reason to cost more memory writes than a full code search does.
    var MEMORY_CHUNK_CHARS = 3200;
    var MAX_OUTPUT_CHARS = 10 * MEMORY_CHUNK_CHARS;

    // ENTRIES - every directory entry crosses the bridge as its own JS object,
    // and marshalling a whole workspace's worth (hundreds of thousands under
    // node_modules/.git/target) is what blew the deadline before a single line
    // was rendered. The bound follows from the output budget: the shortest line
    // this tool can emit is a one-character directory at the top level - the
    // 4-char connector, the name, "/" and a newline = 7 chars - so no tree that
    // could fit MAX_OUTPUT_CHARS can carry more than MAX_OUTPUT_CHARS/7
    // entries. Asking the bridge for more than could ever be printed is pure
    // marshalling waste.
    var MIN_LINE_CHARS = 4 + 1 + 1 + 1;
    var MAX_ENTRIES = Math.floor(MAX_OUTPUT_CHARS / MIN_LINE_CHARS);

    // TIME - this skill declares `timeout: 120`, so the engine kills it at
    // 120s and an overrun returns NOTHING: the failure is total, not degraded.
    // The budget is checked once per directory (the per-entry work is string
    // building, no IO), so the most that can be spent after the last check is
    // one bounded listing - MAX_ENTRIES entries at the measured 8,800
    // entries/s marshal rate = ~0.52s - plus the render pass, which is bounded
    // by MAX_OUTPUT_CHARS of string concatenation (milliseconds, and it never
    // splits). Reserve 10s, ~19x that worst single step: stopping early costs
    // a partial tree, overrunning costs the entire answer.
    var SCRIPT_DEADLINE_MS = 120000;
    var WORK_BUDGET_MS = 110000;

    // Heavy/noise directories are counted and shown but never descended into -
    // in a real workspace node_modules/.git/target alone hold hundreds of
    // thousands of entries. Mirrors the bridge's recursive-walk ignore list and
    // the explore/code_search skills'. Every skip is announced in the output.
    var SKIP_DIRS = [
      "node_modules", ".git", "target", "dist", "build", ".venv", "venv",
      "__pycache__", ".next", ".nuxt", ".cache"
    ];

    // NOTE ON LINE COUNTS (dropped in 0.2.0, deliberately): this tool used to
    // read every text file under 1 MB and split("\n") it to report a line
    // count. Splitting a file into lines measured ~99% of all scan cost and
    // grows SUPER-linearly in line count (11.6 MB of source: 1.5s to read,
    // 112s to split), so on any real repo that one column spent the whole 120s
    // deadline and the tool returned nothing at all. A bounded version would be
    // worse than none: it would spend the budget meant for the tree on a column
    // that then covers an arbitrary prefix of the files. The sizes below come
    // from directory metadata, so this tool now reads zero file bytes.

    var MARK_SKIPPED = " (skipped)";
    var MARK_DEPTH = " ...";
    var MARK_UNREADABLE = " (unreadable)";
    var MARK_NOT_WALKED = " (not walked)";
    var MARK_PARTIAL = " (partial)";

    var startedAt = Date.now();
    // If the engine ever serves a non-numeric clock the time budget silently
    // becomes a no-op - which is exactly the unbounded behaviour being fixed.
    // Detect it here so the note below tells the truth about which bound
    // applied; the entry and output budgets still carry the load.
    var clockOk = isFinite(startedAt);

    var rootParts = rootPath.split(/[\/\\]/);
    var root = {
      name: rootParts[rootParts.length - 1] || rootPath,
      kind: "dir",
      size: 0,
      mark: "",
      children: []
    };

    var visited = 0;
    var dirs = 0;
    var files = 0;
    var entriesCapped = false;
    var timeCapped = false;
    var unreadableDirs = 0;
    var depthLimited = 0;
    var skippedCounts = {};
    var skippedAny = false;

    // Level-by-level walk with bounded, NON-recursive listings. The FIFO queue
    // finishes each depth before starting the next, so whichever budget trips
    // first, what drops is the deepest - least likely to be the layout the
    // caller meant - rather than an arbitrary slice of a depth-first walk.
    //
    // Consumed with a head index rather than shift(): shift() is O(n) per call,
    // and a wide tree can queue thousands of directories, which would make the
    // walk itself quadratic in the entry budget.
    var queue = [{ node: root, path: rootPath, depth: 1 }];
    var head = 0;
    var stop = false;
    var partialNode = null;

    while (head < queue.length && !stop) {
      if (clockOk && Date.now() - startedAt > WORK_BUDGET_MS) {
        timeCapped = true;
        break;
      }
      var cur = queue[head];
      head++;

      var entries;
      try {
        // Request one more than the remaining budget: an overflow-length
        // return proves the directory holds more than we will visit.
        entries = Nanna.listDir(cur.path, false, MAX_ENTRIES - visited + 1);
      } catch (e) {
        if (cur.depth === 1) {
          // The root itself is unreadable: a structured failure, never a throw.
          // A thrown script error reaches the model under an "Execution
          // failed:" prefix, which reads as breakage rather than as a fixable
          // call. The commonest cause is a FILE path, whose os error ("The
          // directory name is invalid", ENOTDIR) teaches nothing — one stat,
          // on an already-failed call, turns it into the correction, and that
          // stat has its own try so this error path can never itself throw.
          if (pathIsFile(rootPath)) {
            return {
              content: "project_structure: \"" + rootPath + "\" exists but is a FILE, not a " +
                "directory — use read_file to see its contents (or search_file to search it). " +
                "Nothing was walked.",
              success: false
            };
          }
          return {
            content: "project_structure failed: cannot list \"" + rootPath + "\": " + String(e) +
              ". Nothing was walked. Check that the path exists, is a directory, and is " +
              "readable.",
            success: false
          };
        }
        cur.node.mark = MARK_UNREADABLE;
        unreadableDirs++;
        continue;
      }

      entries.sort(function(a, b) {
        if (a.entry_type === "dir" && b.entry_type !== "dir") return -1;
        if (a.entry_type !== "dir" && b.entry_type === "dir") return 1;
        if (a.name < b.name) return -1;
        if (a.name > b.name) return 1;
        return 0;
      });

      for (var i = 0; i < entries.length; i++) {
        if (visited >= MAX_ENTRIES) {
          entriesCapped = true;
          // This directory was listed, but not all of it made it into the
          // tree. Saying nothing here would render it as though it were whole.
          partialNode = cur.node;
          stop = true;
          break;
        }
        visited++;

        var e = entries[i];
        var name = String(e.name);
        var isDir = e.entry_type === "dir";
        var child = {
          name: name,
          kind: isDir ? "dir" : "file",
          size: e.size || 0,
          mark: "",
          children: []
        };
        cur.node.children.push(child);

        if (!isDir) {
          files++;
          continue;
        }

        dirs++;
        if (SKIP_DIRS.indexOf(name) >= 0) {
          child.mark = MARK_SKIPPED;
          skippedCounts[name] = (skippedCounts[name] || 0) + 1;
          skippedAny = true;
        } else if (cur.depth < maxDepth) {
          queue.push({ node: child, path: joinPath(cur.path, name), depth: cur.depth + 1 });
        } else {
          child.mark = MARK_DEPTH;
          depthLimited++;
        }
      }
    }

    // Directories that were queued but never reached must say so: an empty
    // child list is otherwise indistinguishable from an empty directory.
    var notWalked = 0;
    for (var q = head; q < queue.length; q++) {
      if (!queue[q].node.mark) {
        queue[q].node.mark = MARK_NOT_WALKED;
        notWalked++;
      }
    }
    if (partialNode) {
      // The budget can trip on the first entry of a directory, in which case
      // nothing of it was taken and "partial" would overstate what we saw.
      if (partialNode.children.length > 0) {
        partialNode.mark = MARK_PARTIAL;
      } else {
        partialNode.mark = MARK_NOT_WALKED;
        partialNode = null;
        notWalked++;
      }
    }

    if (visited === 0) {
      return { content: "Empty directory: " + rootPath, success: true };
    }

    var rendered = renderTree(root, MAX_OUTPUT_CHARS);
    var truncated = entriesCapped || timeCapped || rendered.capped;

    var lines = [];
    lines.push("# " + rootPath + " (depth " + maxDepth + ")");
    lines.push(
      dirs + " director" + (dirs === 1 ? "y" : "ies") + ", " + files + " file" +
      (files === 1 ? "" : "s") + (truncated ? " (PARTIAL - see notes at the end)" : "")
    );
    lines.push("");
    lines.push(rendered.text);

    // Every bound that shaped the answer says so, in the answer.
    var notes = [];
    if (skippedAny) {
      var skipParts = [];
      for (var s = 0; s < SKIP_DIRS.length; s++) {
        var sk = SKIP_DIRS[s];
        if (skippedCounts[sk]) {
          skipParts.push(skippedCounts[sk] > 1 ? sk + " x" + skippedCounts[sk] : sk);
        }
      }
      notes.push("\"" + trimMark(MARK_SKIPPED) + "\" marks noise dirs that exist but were never " +
        "descended into: " + skipParts.join(", ") + ". They hold no project layout worth " +
        "mapping and would swamp every budget below.");
    }
    if (depthLimited > 0) {
      notes.push("\"" + trimMark(MARK_DEPTH) + "\" marks " + depthLimited + " director" +
        (depthLimited === 1 ? "y that holds" : "ies that hold") + " more, not shown because " +
        "max_depth is " + maxDepth + ". Raise max_depth, or pass that path as `path`.");
    }
    if (unreadableDirs > 0) {
      notes.push("\"" + trimMark(MARK_UNREADABLE) + "\" marks " + unreadableDirs +
        " subdirector" + (unreadableDirs === 1 ? "y" : "ies") + " that could not be listed.");
    }
    if (notWalked > 0 || partialNode) {
      notes.push("\"" + trimMark(MARK_NOT_WALKED) + "\" / \"" + trimMark(MARK_PARTIAL) +
        "\" mark directories the walk never reached, or listed only in part, because a " +
        "budget below stopped it first.");
    }
    if (entriesCapped) {
      notes.push("STOPPED: hit the " + MAX_ENTRIES + "-entry discovery budget (the most " +
        "entries that could ever fit a " + MAX_OUTPUT_CHARS + "-char result, so walking " +
        "further could not have printed anything). The walk is shallowest-first, so the " +
        "deepest directories were never reached. The call SUCCEEDED over what it reached " +
        "and more exists beyond it - point `path` at a subdirectory for a complete view.");
    }
    if (timeCapped) {
      notes.push("STOPPED: spent the " + WORK_BUDGET_MS + "ms work budget (of this skill's " +
        SCRIPT_DEADLINE_MS + "ms script deadline, so a partial tree comes back instead of " +
        "the whole call timing out) after " + visited + " entries. The call SUCCEEDED over " +
        "what it reached - narrow with path/max_depth.");
    }
    if (rendered.capped) {
      notes.push("STOPPED: the tree filled the " + MAX_OUTPUT_CHARS + "-char result budget (" +
        (MAX_OUTPUT_CHARS / MEMORY_CHUNK_CHARS) + " memory chunks, one embedding and one " +
        "write each) after " + rendered.shown + " of " + (visited + 1) + " lines. Every line " +
        "shown is complete; the rest were not appended. The call SUCCEEDED - lower " +
        "max_depth, or point `path` at a subdirectory.");
    }
    if (!clockOk) {
      notes.push("Note: this engine served no usable clock, so the time budget was not " +
        "enforced; the entry and result budgets still were.");
    }

    var body = lines.join("\n");
    if (notes.length > 0) {
      body += "\n\n" + notes.join("\n");
    }
    return { content: body, success: true };
  }
}

// Render the tree pre-order with an explicit stack, stopping at `maxChars`.
//
// Explicit stack rather than recursion-plus-reassembly: the 0.1.0 renderer
// recursed and then re-split("\n") the rendered subtree at every level, and
// split() is the one operation that is pathologically slow in this engine. This
// one builds each line once, never splits, and joins exactly once at the end.
function renderTree(root, maxChars) {
  var lines = [];
  // The root carries a mark too when a budget cut its own listing short; it is
  // rendered here rather than through nodeLabel, so it has to be repeated.
  var rootLine = root.name + "/" + root.mark;
  lines.push(rootLine);
  var chars = rootLine.length + 1;
  var capped = false;

  var stack = [];
  pushChildren(stack, root, "");
  while (stack.length > 0) {
    var item = stack.pop();
    var node = item.node;
    var line = item.prefix + (item.isLast ? "└── " : "├── ") +
      nodeLabel(node);
    if (chars + line.length + 1 > maxChars) {
      capped = true;
      break;
    }
    lines.push(line);
    chars += line.length + 1;
    if (node.children.length > 0) {
      pushChildren(stack, node, item.prefix + (item.isLast ? "    " : "│   "));
    }
  }

  return { text: lines.join("\n"), capped: capped, shown: lines.length };
}

// Push in reverse so pop() yields the children top-to-bottom.
function pushChildren(stack, node, prefix) {
  var last = node.children.length - 1;
  for (var i = last; i >= 0; i--) {
    stack.push({ node: node.children[i], prefix: prefix, isLast: i === last });
  }
}

function nodeLabel(node) {
  if (node.kind === "dir") {
    return node.name + "/" + node.mark;
  }
  return node.name + "  " + formatSize(node.size) + node.mark;
}

// Is this path a file? Asked only on a path that already failed as a directory,
// so it costs one metadata call on a path the tool was going to give up on
// anyway. The stat sits in its OWN try: an error path that throws is worse than
// the error it was explaining.
function pathIsFile(path) {
  try {
    var st = Nanna.stat(path);
    return !!(st && st.is_file);
  } catch (e) {
    return false;
  }
}

// Marks carry a leading space so they concatenate onto a label; the notes quote
// them without it.
function trimMark(mark) {
  return mark.slice(1);
}

// Join a directory path with a child name, keeping a "." root implicit so the
// reported paths stay usable verbatim in a follow-up read_file call.
function joinPath(base, name) {
  if (base === "" || base === ".") {
    return name;
  }
  return base + "/" + name;
}

function formatSize(bytes) {
  if (bytes < 1024) return bytes + "B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + "KB";
  return (bytes / (1024 * 1024)).toFixed(1) + "MB";
}
