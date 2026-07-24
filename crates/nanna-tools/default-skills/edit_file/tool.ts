export default {
  name: "edit_file",
  version: "0.1.6",
  output: "context",
  description: "Replace one exact text snippet in a file with new text — an in-place edit for small changes. Use this instead of rewriting the whole file with write_file. ALL THREE main parameters are REQUIRED: file_path, old_string, new_string. old_string must be text that exists in the file (copy it verbatim; indentation differences are tolerated) — include 2-3 surrounding lines to make it unique. Only the matched snippet changes; the rest of the file is untouched. Use write_file only for new files or full rewrites.",
  parameters: {
    type: "object",
    properties: {
      file_path: { type: "string", description: "REQUIRED. Path to the file to edit. Relative paths are resolved against the workspace directory." },
      old_string: { type: "string", description: "REQUIRED. The exact text currently in the file to be replaced. Copy it verbatim (read_file first if unsure) and include 2-3 surrounding lines so it matches exactly once." },
      new_string: { type: "string", description: "REQUIRED. The replacement text. May be empty to delete the snippet. Must differ from old_string." },
      replace_all: { type: "boolean", description: "If true, replace EVERY occurrence of old_string. Default: false." },
      occurrence: { type: "integer", description: "Replace only the Nth match of old_string, 1-based. Alternative to replace_all when old_string appears more than once." }
    },
    required: ["file_path", "old_string", "new_string"]
  },
  execute: function(input) {
    // Errors are returned as { content, success: false } instead of thrown:
    // a thrown script error reaches the model wrapped in five stacked
    // "Execution failed:" prefixes, which small models read as corruption
    // and spiral on. A structured failure surfaces as clean corrective text.
    function fail(message) {
      return { content: message, success: false };
    }

    // Anti-erosion ratchet state, shared with write_file v0.1.11 / file_buffer
    // v0.1.4 (the design comment lives in write_file). All state I/O is
    // best-effort and fails OPEN — it can never block an edit.
    var HIWATER_STATE = ".nanna/write_hiwater.json";
    var HIWATER_MAX_ENTRIES = 200;
    function hiwaterKey(path) {
      var k = path.split("\\").join("/").toLowerCase();
      while (k.indexOf("./") === 0) k = k.substring(2);
      while (k.indexOf("//") !== -1) k = k.split("//").join("/");
      return k;
    }
    function hiwaterIsBuffer(key) {
      var buf = ".__buffer__";
      return key.length >= buf.length && key.lastIndexOf(buf) === key.length - buf.length;
    }
    function hiwaterIsState(key) {
      if (key === ".nanna/write_hiwater.json") return true;
      var tail = "/.nanna/write_hiwater.json";
      return key.length > tail.length && key.lastIndexOf(tail) === key.length - tail.length;
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
    function hiwaterRecord(path, newSize, prevSize) {
      try {
        var key = hiwaterKey(path);
        if (hiwaterIsBuffer(key) || hiwaterIsState(key)) return;
        var map = hiwaterLoad();
        var entry = map[key];
        var hi = newSize > prevSize ? newSize : prevSize;
        if (entry && typeof entry.hi === "number" && isFinite(entry.hi) && entry.hi > hi &&
            typeof entry.last === "number" && entry.last === prevSize) {
          hi = entry.hi;
        }
        map[key] = { hi: hi, last: newSize, at: Date.now() };
        hiwaterSave(map);
      } catch (e) {
        // Best-effort.
      }
    }

    // Collapse whitespace runs in one line: leading/trailing dropped,
    // internal runs become a single space. The unit of indentation-tolerant
    // comparison.
    function normLine(line) {
      var out = "";
      var pendingWs = false;
      for (var i = 0; i < line.length; i++) {
        var c = line.charAt(i);
        if (c === " " || c === "\t" || c === "\r") {
          pendingWs = true;
          continue;
        }
        if (pendingWs && out !== "") out += " ";
        pendingWs = false;
        out += c;
      }
      return out;
    }

    // Dice bigram similarity of two normalized lines (0..1). Cheap, no
    // regex, good enough to point the model at the right neighborhood.
    function diceSim(a, b) {
      if (a === b) return 1;
      if (a.length < 2 || b.length < 2) return 0;
      var counts = {};
      var i;
      for (i = 0; i < a.length - 1; i++) {
        var bg = "k" + a.substring(i, i + 2);
        counts[bg] = (counts[bg] || 0) + 1;
      }
      var hits = 0;
      for (i = 0; i < b.length - 1; i++) {
        var bg2 = "k" + b.substring(i, i + 2);
        if (counts[bg2] > 0) { counts[bg2]--; hits++; }
      }
      return (2 * hits) / (a.length - 1 + b.length - 1);
    }

    // The file line most similar to old_string's first substantial line,
    // quoted with up to 3 following lines — a re-anchoring gift: every
    // failed match hands the model REAL text to copy into its next call.
    function closestSnippet(content, oldStr) {
      var target = "";
      var oldLines = oldStr.split("\n");
      for (var i = 0; i < oldLines.length; i++) {
        var t = normLine(oldLines[i]);
        if (t !== "") { target = t; break; }
      }
      if (target === "") return "";
      var lines = content.split("\n");
      var scan = lines.length < 500 ? lines.length : 500;
      var bestIdx = -1;
      var bestScore = 0.3; // below this it's noise, not an anchor
      for (var j = 0; j < scan; j++) {
        var s = diceSim(normLine(lines[j]), target);
        if (s > bestScore) { bestScore = s; bestIdx = j; }
      }
      if (bestIdx < 0) return "";
      var out = [];
      for (var k = bestIdx; k < lines.length && k < bestIdx + 4; k++) out.push(lines[k]);
      var snip = out.join("\n");
      if (snip.length > 240) snip = snip.substring(0, 240);
      return snip;
    }

    // Whitespace-tolerant match: find spans of whole file lines whose
    // normalized forms equal old_string's normalized lines. Replaces the
    // exact ORIGINAL span, so surrounding bytes (and their line endings)
    // are untouched. Observed live: a 9B model's old_string is composed
    // from compressed memory — content right, indentation wrong.
    function findLooseSpans(content, oldStr) {
      var oldLines = oldStr.split("\n");
      while (oldLines.length > 0 && normLine(oldLines[0]) === "") oldLines.shift();
      while (oldLines.length > 0 && normLine(oldLines[oldLines.length - 1]) === "") oldLines.pop();
      if (oldLines.length === 0) return [];
      var normOld = [];
      for (var i = 0; i < oldLines.length; i++) normOld.push(normLine(oldLines[i]));

      var starts = [0];
      for (var p = 0; p < content.length; p++) {
        if (content.charAt(p) === "\n") starts.push(p + 1);
      }
      var spans = [];
      for (var li = 0; li + normOld.length <= starts.length; li++) {
        var okAll = true;
        for (var lj = 0; lj < normOld.length; lj++) {
          var ls = starts[li + lj];
          var le = (li + lj + 1 < starts.length) ? starts[li + lj + 1] - 1 : content.length;
          if (normLine(content.substring(ls, le)) !== normOld[lj]) { okAll = false; break; }
        }
        if (okAll) {
          var endLine = li + normOld.length - 1;
          var spanEnd = (endLine + 1 < starts.length) ? starts[endLine + 1] - 1 : content.length;
          if (spanEnd > starts[li] && content.charAt(spanEnd - 1) === "\r") spanEnd -= 1;
          spans.push({ start: starts[li], end: spanEnd });
        }
      }
      return spans;
    }

    // Refuse ANY resulting .py content that does not parse. Round-6
    // lesson: gating only valid->invalid transitions let a file that was
    // BORN broken stay broken through repeated equally-broken "repairs".
    // The error names the line; if the file carries several errors they
    // must be fixed in one edit (or force=true saves partial progress).
    // ANY checker failure fails OPEN. Returns an error string or null.
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
          return "REFUSED — after this edit " + path + " would NOT be valid Python (" + detail + "). The file is UNCHANGED. Fix new_string so the whole file parses (if the file has several errors, fix them all in this one edit), then retry.";
        }
        return null;
      } catch (e) {
        return null;
      }
    }

    // Accept multiple parameter name variants from different models
    var filePath = input.file_path || input.filePath || input.path || input.file;

    // old/new variants accept ONLY string values, so a boolean flag like
    // replace=true can never be mistaken for the replacement text.
    var oldStr;
    var oldNames = ["old_string", "old_str", "old_text", "search", "find", "target"];
    for (var oi = 0; oi < oldNames.length; oi++) {
      if (typeof input[oldNames[oi]] === "string") { oldStr = input[oldNames[oi]]; break; }
    }
    var newStr;
    var newNames = ["new_string", "new_str", "new_text", "replacement", "replace_with", "replace"];
    for (var ni = 0; ni < newNames.length; ni++) {
      if (typeof input[newNames[ni]] === "string") { newStr = input[newNames[ni]]; break; }
    }

    if (!filePath && oldStr === undefined && newStr === undefined) {
      return fail("edit_file failed: you must pass file_path, old_string AND new_string. Nothing was changed. Call edit_file again like: edit_file(file_path=\"D:/path/to/file.py\", old_string=\"<exact text currently in the file>\", new_string=\"<the replacement text>\")");
    }
    if (!filePath) {
      return fail("edit_file failed: missing file_path. Nothing was changed. Call edit_file again with file_path (the file to edit) plus old_string AND new_string.");
    }
    if (oldStr === undefined) {
      return fail("edit_file failed: missing old_string. Nothing was changed. Call edit_file again with old_string set to the EXACT text currently in " + filePath + " (include 2-3 surrounding lines to make it unique) and new_string set to its replacement.");
    }
    if (newStr === undefined) {
      return fail("edit_file failed: missing new_string. Nothing was changed. Call edit_file again with the same old_string and new_string set to the replacement text (it may be empty to delete the snippet).");
    }
    if (oldStr === "") {
      return fail("edit_file failed: old_string is empty. Nothing was changed. edit_file replaces an existing snippet; to create a file or replace its entire content, use write_file with the complete text.");
    }

    var content;
    try {
      content = Nanna.readFile(filePath);
    } catch (e) {
      var readErr = String(e);
      if (readErr.length > 120) readErr = readErr.substring(0, 120) + "...";
      return fail("edit_file failed: could not read " + filePath + " (" + readErr + "). Nothing was changed. Check the path, or use write_file to create a new file.");
    }

    // Identical old/new: if the file ALREADY contains the text, the desired
    // state holds — succeed as a no-op so the model moves on instead of
    // spiraling (observed live: it "confirms" content instead of diffing).
    // If the text is absent, its memory of the file is stale — say so.
    if (oldStr === newStr) {
      var lfSame = oldStr.split("\r\n").join("\n");
      var present = content.indexOf(oldStr) >= 0
        || content.indexOf(lfSame) >= 0
        || content.indexOf(lfSame.split("\n").join("\r\n")) >= 0
        || findLooseSpans(content, oldStr).length > 0;
      if (present) {
        return { content: "No change needed — " + filePath + " already contains exactly that text. Continue to the next step.", success: true };
      }
      var nearSame = closestSnippet(content, oldStr);
      return fail("edit_file failed: old_string and new_string are identical AND that text is not in " + filePath + " — the file's real content differs from your memory. The file is UNCHANGED." + (nearSame === "" ? "" : "\nClosest ACTUAL text in the file:\n" + nearSame + "\n") + "Call read_file, copy the real text as old_string, and set new_string to your fix.");
    }

    // Try the exact text first, then retry with line-ending normalization in
    // BOTH directions (LF old_string vs CRLF file, and CRLF old_string vs LF
    // file). Only the matched snippet is touched — the rest of the file keeps
    // its own line endings. The replacement is converted to the matched
    // flavor so the edit does not introduce mixed endings.
    var needle = oldStr;
    var replacement = newStr;
    if (content.indexOf(needle) < 0) {
      var oldLf = oldStr.split("\r\n").join("\n");
      var oldCrlf = oldLf.split("\n").join("\r\n");
      if (content.indexOf(oldCrlf) >= 0) {
        needle = oldCrlf;
        replacement = newStr.split("\r\n").join("\n").split("\n").join("\r\n");
      } else if (content.indexOf(oldLf) >= 0) {
        needle = oldLf;
        replacement = newStr.split("\r\n").join("\n");
      }
    }

    var updated;
    var replaced;

    if (content.indexOf(needle) >= 0) {
      var count = 0;
      var pos = content.indexOf(needle);
      while (pos >= 0) {
        count++;
        pos = content.indexOf(needle, pos + needle.length);
      }

      // Accept string "true" as well: small models often stringify booleans,
      // and rejecting it here would loop them on the ambiguity error below.
      function flagSet(v) { return v === true || v === "true"; }
      var replaceAll = flagSet(input.replace_all) || flagSet(input.replaceAll) || flagSet(input.all) || input.replace === true;
      var occurrence = input.occurrence;
      if (occurrence === undefined) occurrence = input.occurence;
      if (occurrence === undefined) occurrence = input.nth;

      if (occurrence !== undefined) {
        occurrence = Math.floor(occurrence);
        if (!(occurrence >= 1 && occurrence <= count)) {
          return fail("edit_file failed: occurrence=" + occurrence + " is out of range — old_string matches " + count + " time(s) in " + filePath + ". The file is UNCHANGED. Pass occurrence between 1 and " + count + ".");
        }
      }

      if (count > 1 && !replaceAll && occurrence === undefined) {
        return fail("edit_file failed: found " + count + " matches for old_string in " + filePath + ". The file is UNCHANGED. Either include more surrounding lines in old_string to make it unique, or pass replace_all=true, or pass occurrence=<1.." + count + "> to pick one match.");
      }

      if (occurrence !== undefined) {
        var at = content.indexOf(needle);
        for (var k = 1; k < occurrence; k++) {
          at = content.indexOf(needle, at + needle.length);
        }
        updated = content.substring(0, at) + replacement + content.substring(at + needle.length);
        replaced = 1;
      } else if (replaceAll) {
        updated = content.split(needle).join(replacement);
        replaced = count;
      } else {
        var first = content.indexOf(needle);
        updated = content.substring(0, first) + replacement + content.substring(first + needle.length);
        replaced = 1;
      }
    } else {
      // Exact and line-ending matches failed: whitespace-tolerant fallback.
      var spans = findLooseSpans(content, oldStr);
      if (spans.length === 1) {
        var spanText = content.substring(spans[0].start, spans[0].end);
        var looseReplacement = newStr;
        if (spanText.indexOf("\r\n") >= 0) {
          looseReplacement = newStr.split("\r\n").join("\n").split("\n").join("\r\n");
        }
        updated = content.substring(0, spans[0].start) + looseReplacement + content.substring(spans[0].end);
        replaced = 1;
      } else if (spans.length > 1) {
        return fail("edit_file failed: " + spans.length + " places in " + filePath + " match old_string once indentation differences are ignored. The file is UNCHANGED. Include 1-2 more surrounding lines in old_string to make it unique.");
      } else {
        var head = oldStr.split("\r\n").join("\n").split("\n").slice(0, 3).join("\n");
        if (head.length > 120) head = head.substring(0, 120) + "...";
        var near = closestSnippet(content, oldStr);
        return fail("edit_file failed: old_string not found in " + filePath + " — the file's real content differs from your memory. The file is UNCHANGED and intact. You searched for:\n" + head + (near === "" ? "" : "\nClosest ACTUAL text in the file:\n" + near) + "\nCall read_file, copy the exact current text, then retry edit_file.");
      }
    }

    if (input.force !== true) {
      var syntaxRefusal = pythonSyntaxRefusal(filePath, updated);
      if (syntaxRefusal) return fail(syntaxRefusal);
    }

    try {
      Nanna.writeFile(filePath, updated);
    } catch (e2) {
      var writeErr = String(e2);
      if (writeErr.length > 120) writeErr = writeErr.substring(0, 120) + "...";
      return fail("edit_file failed writing " + filePath + " (" + writeErr + "). Retry the same edit_file call; if it fails again, read the file to verify its current state before editing.");
    }

    // Anti-erosion ratchet sync, shared with write_file v0.1.11 (full design
    // comment there). edit_file is a TRUSTED in-band mutator: recording
    // {hi, last} after each successful edit keeps write_file's high-water
    // guard armed across surgical edits — otherwise every edit looks
    // out-of-band and hands the next rewrite a fresh current-size floor (the
    // 2-call nibble+rewrite erosion loop from the verify round). Best-effort,
    // fails open; deliberately NO shrink refusal here (deletion is this
    // tool's job — the guard's own refusals steer deletions to edit_file).
    hiwaterRecord(filePath, updated.length, content.length);

    return { content: "Edited " + filePath + ": replaced " + replaced + " occurrence(s). File is now " + updated.length + " characters.", success: true };
  }
}
