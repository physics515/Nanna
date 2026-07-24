export default {
  name: "write_file",
  version: "0.1.11",
  output: "context",
  description: "Write content to a file. BOTH parameters are REQUIRED on every call: file_path AND content (the complete file text). A call without content does nothing and fails. Creates the file if it doesn't exist, overwrites if it does. For files too long to write in one call, use file_buffer (append chunks, then commit) instead. SAFETY: blocked if new content is under 30% of the largest size the file has held (likely truncation), if a .py file would not parse, or if the filename looks like a versioned copy.",
  parameters: {
    type: "object",
    properties: {
      file_path: { type: "string", description: "REQUIRED. Path to the file to write. Relative paths are resolved against the workspace directory." },
      content: { type: "string", description: "REQUIRED. The complete text to write into the file. Never omit this — a write_file call without content always fails." },
    },
    required: ["file_path", "content"]
  },
  execute: function(input) {
    // Guidance errors are RETURNED, not thrown: a thrown script error reaches
    // the model under five stacked "Execution failed:" prefixes, which small
    // models read as corruption and spiral on.
    function fail(message) {
      return { content: message, success: false };
    }

    // Anti-erosion ratchet (round-17 lesson): the 30% shrink floor used to be
    // relative to the CURRENT size, so repeated 60-80% rewrites during fault
    // storms compounded (0.7^n) and slowly hollowed files out without ever
    // tripping the guard. The floor is now 30% of the LARGEST size the file
    // has held, tracked per workspace in .nanna/write_hiwater.json (.nanna/
    // is the non-markdown local-state dir — never beside user files, so no
    // sidecar clutter). Each entry stores {hi, last, at}: `hi` is the
    // high-water mark, `last` is the size write_file ITSELF last left on
    // disk. The history is only trusted while write_file is the sole
    // mutator: if the disk size no longer equals `last`, another actor
    // (edit_file, file_buffer, exec, the user) changed the file
    // deliberately, and the guard RE-BASES to disk truth instead of judging
    // against a size that no longer exists — a stale mark must never refuse
    // a write that the disk state itself would allow (verify-round blocker:
    // grow-writes after an out-of-band shrink looped forever). Every state
    // operation fails OPEN: a missing or corrupt state file degrades to the
    // old current-size behavior, never blocks a write.
    var HIWATER_STATE = ".nanna/write_hiwater.json";
    // Bound: the state file must stay trivially small over an unbounded
    // daemon lifetime; missions touch tens of files, so 200 entries with
    // least-recently-updated eviction loses nothing real.
    var HIWATER_MAX_ENTRIES = 200;
    function hiwaterKey(path) {
      // Slash/case normalization plus "./" stripping only. Deliberately NO
      // workspace-root resolution (that lives on the Rust side): an aliased
      // spelling of the same file gets an independent entry whose `last`
      // never matches disk, so it re-bases to current-size behavior — it
      // costs a little ratchet strength, never a false refusal. Lowercase is
      // correct here because this daemon targets Windows paths.
      var k = path.split("\\").join("/").toLowerCase();
      while (k.indexOf("./") === 0) k = k.substring(2);
      while (k.indexOf("//") !== -1) k = k.split("//").join("/");
      return k;
    }
    // Transient park buffers must never be judged by (or recorded in)
    // cross-call history — they are rewritten wholesale every park cycle —
    // and the ratchet's own state file guards itself specially (below).
    function hiwaterIsBuffer(key) {
      var buf = ".__buffer__";
      return key.length >= buf.length && key.lastIndexOf(buf) === key.length - buf.length;
    }
    // Exact path only (root or any /.nanna/ dir), so a real work file with a
    // similar name keeps full ratchet protection.
    function hiwaterIsState(key) {
      if (key === ".nanna/write_hiwater.json") return true;
      var tail = "/.nanna/write_hiwater.json";
      return key.length > tail.length && key.lastIndexOf(tail) === key.length - tail.length;
    }
    function hiwaterExempt(key) {
      return hiwaterIsBuffer(key) || hiwaterIsState(key);
    }
    function hiwaterLoad() {
      try {
        var raw = Nanna.readFile(HIWATER_STATE);
        if (raw) {
          var parsed = JSON.parse(raw);
          if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) return parsed;
        }
      } catch (e) {
        // Missing or corrupt state: start fresh.
      }
      return {};
    }
    function hiwaterHi(entry) {
      if (entry && typeof entry.hi === "number" && isFinite(entry.hi) && entry.hi > 0) return entry.hi;
      return 0;
    }
    function hiwaterLast(entry) {
      if (entry && typeof entry.last === "number" && isFinite(entry.last) && entry.last >= 0) return entry.last;
      return -1;
    }
    function hiwaterSave(map) {
      try {
        var keys = Object.keys(map);
        if (keys.length > HIWATER_MAX_ENTRIES) {
          keys.sort(function(a, b) {
            return ((map[a] && map[a].at) || 0) - ((map[b] && map[b].at) || 0);
          });
          var evict = keys.length - HIWATER_MAX_ENTRIES;
          for (var i = 0; i < evict; i++) delete map[keys[i]];
        }
        Nanna.writeFile(HIWATER_STATE, JSON.stringify(map));
      } catch (e) {
        // State persistence is best-effort.
      }
    }

    // Refuse ANY .py content that does not parse — new file or overwrite.
    // Round-6 lesson: gating only valid->invalid transitions let the model
    // create a file BORN broken and then "repair" it with equally broken
    // content forever. Invalid Python on disk is never useful; the error
    // names the line so the write call becomes a fast syntax feedback
    // loop. force=true overrides; ANY checker failure fails OPEN (a
    // missing python interpreter must never block writes).
    function pythonSyntaxRefusal(path, nextText) {
      var lower = path.toLowerCase();
      if (lower.length < 3 || lower.lastIndexOf(".py") !== lower.length - 3) return null;
      try {
        var chk = path + ".__chk.py";
        var newTmp = path + ".__chk_new.py";
        Nanna.writeFile(newTmp, nextText);
        Nanna.writeFile(chk,
          "import ast, sys\n" +
          "try:\n" +
          "    ast.parse(open(sys.argv[1], encoding='utf-8').read())\n" +
          "    print('NEW_OK')\n" +
          "except SyntaxError as e:\n" +
          "    print('NEW_BAD line ' + str(e.lineno) + ': ' + str(e.msg))\n");
        var cmd = "python '" + chk + "' '" + newTmp + "'; rc=$?; rm -f '" + chk + "' '" + newTmp + "'; exit $rc";
        var result = Nanna.exec(cmd, null, 30);
        var out = result && result.stdout ? result.stdout : "";
        var bad = out.indexOf("NEW_BAD");
        if (bad !== -1) {
          var detail = out.substring(bad + 8);
          var nl = detail.indexOf("\n");
          if (nl !== -1) detail = detail.substring(0, nl);
          if (detail.length > 160) detail = detail.substring(0, 160);
          return detail;
        }
        return null;
      } catch (e) {
        return null;
      }
    }

    // Accept multiple parameter name variants from different models
    var filePath = input.file_path || input.filePath || input.path || input.file || input.filename;
    // Undefined-chain, NOT || — an explicit content:"" is a legitimate
    // empty-file write (e.g. a package __init__.py) and must not be
    // misreported as "missing content" (verify-round finding).
    var fileContent = input.content;
    if (fileContent === undefined || fileContent === null) fileContent = input.text;
    if (fileContent === undefined || fileContent === null) fileContent = input.data;
    if (fileContent === undefined || fileContent === null) fileContent = input.new_content;
    if (fileContent === undefined || fileContent === null) fileContent = input.file_content;
    if (!filePath && (fileContent === undefined || fileContent === null)) {
      return fail("write_file failed: you must pass BOTH file_path AND content. Call it again like: write_file(file_path=\"D:/path/to/file.py\", content=\"<the complete file text>\")");
    }
    if (!filePath) {
      return fail("write_file failed: missing file_path. Nothing was written. Call it again with BOTH file_path (the destination path) AND content.");
    }
    if (fileContent === undefined || fileContent === null) {
      return fail("write_file failed: missing content. Nothing was written. Call write_file again with file_path=\"" + filePath + "\" AND content set to the COMPLETE file text.");
    }
    if (typeof filePath !== "string") filePath = String(filePath);
    if (typeof fileContent !== "string") fileContent = String(fileContent);

    // Writing the ratchet's own bookkeeping is always confusion — and wiping
    // it would silently disarm the erosion guard. Calm, self-describing
    // refusal so the model moves on instead of "repairing" internals.
    if (!input.force && hiwaterIsState(hiwaterKey(filePath))) {
      return fail("write_file skipped: " + filePath + " is write_file's internal bookkeeping. It maintains itself, is healthy, and never needs manual repair. Your own files are unaffected. Continue with your actual task.");
    }

    var bytes = fileContent.length;

    // Safety check BEFORE writing: block if new content is drastically smaller
    // than the file has ever been WHILE write_file was its only mutator. This
    // prevents the model from overwriting a large file with truncated content
    // when it lost context, AND (via the high-water base) from eroding it
    // across many individually-"acceptable" shrinks. Three invariants from
    // the adversarial verify round: a write that does not shrink the CURRENT
    // disk file can never erode it, so it is never refused; a file that was
    // changed out-of-band since our last write re-bases to disk truth; a file
    // that no longer exists has nothing left to protect — creation is always
    // allowed and re-arms the ratchet from the new size.
    var existingSize = 0;
    var fileExists = false;
    var existsUnknown = false;
    if (!input.force) {
      try {
        var existing = Nanna.readFile(filePath);
        if (existing !== undefined && existing !== null) {
          fileExists = true;
          existingSize = existing.length;
        }
      } catch (e) {
        // "os error 2"/"os error 3" = the file genuinely doesn't exist. Any
        // OTHER read failure (sharing violation, non-UTF-8) means the file is
        // probably THERE but unreadable: fail open on the guard, but flag it
        // so the ratchet state is left untouched (verify finding: a transient
        // lock must not reset the mark to the new small size).
        // Parenthesized form ONLY: the bridge embeds io::Error display as
        // "...(os error N)", and a bare "os error 3" substring also matches
        // "(os error 32)" — the sharing-violation case this flag exists for.
        var readErr = String(e);
        if (readErr.indexOf("(os error 2)") === -1 && readErr.indexOf("(os error 3)") === -1) {
          existsUnknown = true;
        }
      }

      var hwKeyGuard = hiwaterKey(filePath);
      var hwBase = existingSize;
      if (fileExists && !hiwaterExempt(hwKeyGuard)) {
        var hwEntry = hiwaterLoad()[hwKeyGuard];
        var hwHi = hiwaterHi(hwEntry);
        // History is live only if the disk still holds exactly what OUR last
        // write left there; otherwise another tool/user re-shaped the file
        // deliberately and the stale mark must not judge this write.
        if (hwHi > hwBase && hiwaterLast(hwEntry) === existingSize) {
          hwBase = hwHi;
        }
      }

      if (!hiwaterExempt(hwKeyGuard) && hwBase > 500 && bytes < existingSize && bytes < hwBase * 0.3) {
        var sizeStory = "currently holds " + existingSize + " bytes";
        if (hwBase > existingSize) {
          sizeStory = "holds " + existingSize + " bytes now and has held " + hwBase + " bytes before";
        }
        return {
          content: "WRITE REFUSED — the file was NOT modified and is fully intact. " +
            "You tried to write only " + bytes + " bytes over " + filePath +
            " which " + sizeStory + " (" +
            Math.round(bytes / hwBase * 100) + "% of that). That usually means " +
            "you sent a fragment instead of the whole file. For a small change, use " +
            "edit_file instead: edit_file(file_path=\"" + filePath + "\", old_string=<the exact current text>, " +
            "new_string=<your replacement>) — it changes just that snippet and leaves the rest untouched. " +
            "To remove a section, edit_file with new_string=\"\". " +
            "Only if you truly mean to replace the WHOLE file: (1) read_file " + filePath + ", " +
            "(2) merge your change into the FULL text, (3) call write_file again with the complete content.",
          success: false
        };
      }
    }

    if (!input.force) {
      // Versioned-copy REFUSAL (observed live: models fork foo.py.new2,
      // new_foo.py, foo_fixed_v1.txt instead of fixing the real file, then
      // lose track of which copy is real — an advisory did not stop it).
      var baseName = filePath.split("\\").join("/").split("/").pop().toLowerCase();
      var copyMarkers = [".new", "_v1", "_v2", "_v3", "_v4", "_v5", "_fixed", "_backup", "_temp", "_copy", "_part", "_old", "_final", "_clean", "_scrubbed", "scratch"];
      var copyPrefixes = ["new_", "copy_", "old_", "temp_", "backup_"];
      var copyHit = null;
      for (var m = 0; m < copyMarkers.length; m++) {
        if (baseName.indexOf(copyMarkers[m]) !== -1) { copyHit = copyMarkers[m]; break; }
      }
      if (!copyHit) {
        for (var p = 0; p < copyPrefixes.length; p++) {
          if (baseName.indexOf(copyPrefixes[p]) === 0) { copyHit = copyPrefixes[p]; break; }
        }
      }
      if (copyHit) {
        return fail("WRITE REFUSED — '" + filePath + "' looks like a versioned copy ('" + copyHit + "'). Nothing was written. Keep ONE real file: change the ORIGINAL in place with edit_file, or write the full corrected content directly to the original path (a complete valid rewrite at or above the file's current size is always accepted).");
      }

      // VALID CONTENT ALWAYS WINS (round-13 lesson): the earlier rail
      // blocked whole-file writes whenever a parked draft existed — even
      // when the new content was perfectly valid, bouncing the model's one
      // reliable move (fresh generation) and wedging it between an empty
      // real file and a draft it would not repair. Order is now: check
      // validity FIRST; a parsing .py write is accepted outright and any
      // stale draft/markers are swept; only INVALID content meets the
      // park/rail machinery.
      var syntaxDetail = pythonSyntaxRefusal(filePath, fileContent);
      if (syntaxDetail === null) {
        var sweepBufPath = filePath + ".__buffer__";
        try {
          Nanna.exec("rm -f '" + sweepBufPath + "' '" + filePath + ".__cleared__' '" + sweepBufPath + ".__cleared__'", null, 15);
        } catch (eSweep) {
          // Stale draft leftovers are harmless.
        }
      } else {
        // Existing parked draft + ANOTHER invalid regeneration: keep the
        // parked draft authoritative and steer to the repair loop.
        var railBufPath = filePath + ".__buffer__";
        var railParked = null;
        try {
          railParked = Nanna.readFile(railBufPath);
        } catch (eRail) {
          // No parked draft.
        }
        if (railParked !== null && railParked !== undefined && railParked !== "") {
          return fail("WRITE BLOCKED — this content has a SYNTAX ERROR (" + syntaxDetail + ") and a parked draft for " + filePath + " already exists at " + railBufPath + " (" + railParked.length + " chars). Repair THAT draft: edit_file(file_path=\"" + railBufPath + "\", old_string=<the broken line>, new_string=<the fix>), then file_buffer(action=\"commit\", file_path=\"" + filePath + "\"). A fully VALID rewrite of " + filePath + " at or above its current size would also be accepted.");
        }
      }

      // Draft PARKING (round-8 lesson): refusing a broken whole-file write
      // outright sends the model into a regeneration lottery — each retry
      // regenerates everything and rolls new errors (observed live: 21
      // refusals, every one a different line). Instead the rejected draft
      // is SAVED to the buffer beside the target, where the model repairs
      // the one named error with a small edit_file delta and commits.
      if (syntaxDetail) {
        var parkPath = filePath + ".__buffer__";
        var parked = false;
        try {
          Nanna.writeFile(parkPath, fileContent);
          parked = true;
        } catch (ePark) {
          // Fall through to the plain refusal below.
        }
        if (parked) {
          // Quote the offending line verbatim: it is a ready-made
          // old_string, removing the last excuse to regenerate (observed
          // live: three parks in a row, each a fresh full regeneration
          // with a brand-new error line).
          var lineQuote = "";
          if (syntaxDetail.indexOf("line ") === 0) {
            var colonAt = syntaxDetail.indexOf(":");
            if (colonAt > 5) {
              var lineNo = parseInt(syntaxDetail.substring(5, colonAt), 10);
              if (lineNo >= 1) {
                var draftLines = fileContent.split("\n");
                if (lineNo <= draftLines.length) {
                  var lq = draftLines[lineNo - 1];
                  if (lq.length > 80) lq = lq.substring(0, 80);
                  lineQuote = " Line " + lineNo + " of your draft is: `" + lq + "` — use exactly that as old_string.";
                }
              }
            }
          }
          return fail("WRITE PARKED — your content for " + filePath + " has a SYNTAX ERROR (" + syntaxDetail + "), so the file was NOT changed. Nothing was lost: the draft IS SAVED at " + parkPath + "." + lineQuote + " Your NEXT call must be edit_file(file_path=\"" + parkPath + "\", old_string=<the broken line>, new_string=<the fixed line>), then file_buffer(action=\"commit\", file_path=\"" + filePath + "\"). Do NOT call write_file again for this file and do NOT regenerate it.");
        }
        return fail("WRITE REFUSED — the content you sent for " + filePath + " is NOT valid Python (" + syntaxDetail + "). The file is UNCHANGED. Fix the syntax and call write_file again with the corrected COMPLETE text.");
      }
    }

    try {
      Nanna.writeFile(filePath, fileContent);
    } catch (e2) {
      var writeErr = String(e2);
      if (writeErr.length > 120) writeErr = writeErr.substring(0, 120) + "...";
      return fail("write_file failed writing " + filePath + " (" + writeErr + "). Retry the same call; if it fails again, read_file to verify the file state.");
    }

    // Ratchet update AFTER a successful write. In-band writes only ever
    // raise the high-water mark (so fluctuating rewrites stay pinned to the
    // peak — a grow-write must NOT re-base, or shrink/grow alternation
    // launders the ratchet). Force and creation-over-missing RESET it: both
    // are deliberate re-shapes with nothing stale worth protecting. A write
    // over a file that changed out-of-band re-bases to disk truth. And when
    // the pre-write read failed for unknown reasons, the state is left
    // completely alone — the next successful write re-bases naturally via
    // the last-mismatch path.
    if (!existsUnknown) {
      try {
        var hwMap = hiwaterLoad();
        var hwKey = hiwaterKey(filePath);
        if (!hiwaterExempt(hwKey)) {
          var hwPrevEntry = hwMap[hwKey];
          var hwNext = bytes > existingSize ? bytes : existingSize;
          if (!input.force && fileExists && hiwaterLast(hwPrevEntry) === existingSize) {
            var hwPrevHi = hiwaterHi(hwPrevEntry);
            if (hwPrevHi > hwNext) hwNext = hwPrevHi;
          }
          if (input.force || !fileExists) hwNext = bytes;
          hwMap[hwKey] = { hi: hwNext, last: bytes, at: Date.now() };
          hiwaterSave(hwMap);
        }
      } catch (eHw) {
        // Best-effort; the user's write already succeeded.
      }
    }

    // Deliberately NO echo of the written content: echoing the whole file
    // made the result exceed the context threshold, and the model read the
    // resulting truncation stub as "my write was discarded" (observed
    // live, round 7). The file on disk is the source of truth.
    return { content: "Wrote " + bytes + " bytes to " + filePath + ". The file on disk now holds exactly this content." };
  }
}
