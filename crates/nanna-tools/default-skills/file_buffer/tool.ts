export default {
  name: "file_buffer",
  version: "0.1.2",
  output: "context",
  description: "Write a LARGE file across MULTIPLE tool calls: append chunks of text one call at a time, then commit once to write the real file. Use this instead of write_file when a file is too long to write in one call. Sequence: file_buffer(action=\"append\", file_path, content) repeatedly in order from the top of the file, then file_buffer(action=\"commit\", file_path) to write it. action=\"show\" previews the pending buffer, action=\"clear\" discards it. The real file only changes on commit.",
  parameters: {
    type: "object",
    properties: {
      action: { type: "string", enum: ["append", "commit", "show", "clear"], description: "REQUIRED. append = add the next chunk to the pending buffer; commit = write the whole buffer to file_path and clear it; show = preview the pending buffer; clear = discard the pending buffer." },
      file_path: { type: "string", description: "REQUIRED. The REAL file being built. The pending buffer is kept beside it until commit." },
      content: { type: "string", description: "REQUIRED for append: the NEXT chunk of the file, continuing exactly where the buffer ended. A newline is inserted between chunks automatically if missing." },
      force: { type: "boolean", description: "commit only: skip the Python syntax check and the shrink safety check." }
    },
    required: ["action", "file_path"]
  },
  execute: function(input) {
    // Structured failures, never throws: thrown script errors reach the
    // model under five stacked "Execution failed:" prefixes.
    function fail(message) {
      return { content: message, success: false };
    }

    // Python syntax gate — same contract as write_file/edit_file v0.1.4:
    // refuse ANY invalid .py content (a file that never parses is never
    // useful; the error names the line so the model can fix it). Fails
    // OPEN if the checker is unavailable. Returns error string or null.
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

    function lineCount(text) {
      var n = 1;
      for (var i = 0; i < text.length; i++) {
        if (text.charAt(i) === "\n") n++;
      }
      return n;
    }

    function lastLines(text, howMany) {
      var lines = text.split("\n");
      while (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
      var start = lines.length > howMany ? lines.length - howMany : 0;
      var tail = lines.slice(start).join("\n");
      if (tail.length > 300) tail = "..." + tail.substring(tail.length - 300);
      return tail;
    }

    var filePath = input.file_path || input.filePath || input.path || input.file;
    var action = input.action || input.mode || input.op;
    var chunk;
    var chunkNames = ["content", "text", "data", "chunk"];
    for (var ci = 0; ci < chunkNames.length; ci++) {
      if (typeof input[chunkNames[ci]] === "string") { chunk = input[chunkNames[ci]]; break; }
    }

    if (!filePath) {
      return fail("file_buffer failed: missing file_path. Nothing was changed. Call it again with file_path (the real file being built) and action (append/commit/show/clear).");
    }
    // A content chunk with no action is an append — the likeliest intent.
    if (!action && chunk !== undefined) action = "append";
    if (!action) {
      return fail("file_buffer failed: missing action. Nothing was changed. Sequence: file_buffer(action=\"append\", file_path, content) repeatedly in order, then file_buffer(action=\"commit\", file_path=\"" + filePath + "\").");
    }
    action = String(action).toLowerCase();

    var bufPath = filePath + ".__buffer__";
    var buffered = null;
    try {
      buffered = Nanna.readFile(bufPath);
    } catch (eNone) {
      // No pending buffer yet.
    }

    if (action === "append" || action === "add" || action === "write") {
      if (chunk === undefined) {
        return fail("file_buffer failed: append needs content. Nothing was changed. Call file_buffer(action=\"append\", file_path=\"" + filePath + "\", content=\"<the next chunk of the file>\").");
      }
      var joined;
      if (buffered === null || buffered === "") {
        joined = chunk;
      } else if (buffered.charAt(buffered.length - 1) === "\n" || chunk.charAt(0) === "\n") {
        joined = buffered + chunk;
      } else {
        joined = buffered + "\n" + chunk;
      }
      try {
        Nanna.writeFile(bufPath, joined);
      } catch (eW) {
        var wErr = String(eW);
        if (wErr.length > 120) wErr = wErr.substring(0, 120) + "...";
        return fail("file_buffer failed writing the buffer (" + wErr + "). Retry the same append.");
      }
      return {
        content: "Buffered: " + joined.length + " chars / " + lineCount(joined) + " lines pending for " + filePath + ". The buffer now ends with:\n" + lastLines(joined, 2) + "\nContinue with the NEXT lines via file_buffer(action=\"append\", ...), or finish with file_buffer(action=\"commit\", file_path=\"" + filePath + "\").",
        success: true
      };
    }

    if (action === "commit" || action === "flush" || action === "save") {
      if (buffered === null || buffered === "") {
        return fail("file_buffer failed: nothing is buffered for " + filePath + ". Nothing was changed. Append the file content first: file_buffer(action=\"append\", file_path, content), then commit.");
      }
      if (input.force !== true) {
        var syntaxDetail = pythonSyntaxRefusal(filePath, buffered);
        if (syntaxDetail) {
          // Quote the offending line: a ready-made old_string for the
          // repair edit — regeneration is never the answer here.
          var lineQuote = "";
          if (syntaxDetail.indexOf("line ") === 0) {
            var colonAt = syntaxDetail.indexOf(":");
            if (colonAt > 5) {
              var lineNo = parseInt(syntaxDetail.substring(5, colonAt), 10);
              if (lineNo >= 1) {
                var bufLines = buffered.split("\n");
                if (lineNo <= bufLines.length) {
                  var lq = bufLines[lineNo - 1];
                  if (lq.length > 80) lq = lq.substring(0, 80);
                  lineQuote = " Line " + lineNo + " of the buffer is: `" + lq + "` — use exactly that as old_string.";
                }
              }
            }
          }
          return fail("COMMIT REFUSED — the buffered content for " + filePath + " is not valid Python (" + syntaxDetail + "). The real file is UNCHANGED and the buffer is KEPT." + lineQuote + " Your NEXT call must be edit_file(file_path=\"" + bufPath + "\", old_string=<the broken line>, new_string=<the fixed line>), then commit again. Do NOT regenerate the file.");
        }
        // Shrink guard, same 30% rule as write_file: a much-smaller commit
        // over an existing file usually means the buffer is incomplete.
        var existingLen = 0;
        try {
          var existing = Nanna.readFile(filePath);
          if (existing) existingLen = existing.length;
        } catch (eR) {
          // New file.
        }
        if (existingLen > 500 && buffered.length < existingLen * 0.3) {
          return fail("COMMIT REFUSED — the buffer holds only " + buffered.length + " chars but " + filePath + " currently holds " + existingLen + ". The file is UNCHANGED and the buffer is KEPT — it looks incomplete. Keep appending the rest, or commit with force=true to intentionally replace the file with this smaller version.");
        }
      }
      try {
        Nanna.writeFile(filePath, buffered);
      } catch (eC) {
        var cErr = String(eC);
        if (cErr.length > 120) cErr = cErr.substring(0, 120) + "...";
        return fail("file_buffer failed writing " + filePath + " (" + cErr + "). The buffer is KEPT — retry the commit.");
      }
      try {
        Nanna.exec("rm -f '" + bufPath + "' '" + filePath + ".__cleared__'", null, 15);
      } catch (eRm) {
        try { Nanna.writeFile(bufPath, ""); } catch (eZ) { /* leftovers are harmless */ }
      }
      return {
        content: "Committed " + buffered.length + " chars (" + lineCount(buffered) + " lines) to " + filePath + ". Buffer cleared. Verify the file now with exec, then continue.",
        success: true
      };
    }

    if (action === "show" || action === "preview" || action === "status") {
      if (buffered === null || buffered === "") {
        return { content: "Buffer for " + filePath + " is empty. Start with file_buffer(action=\"append\", file_path, content).", success: true };
      }
      return {
        content: "Pending buffer for " + filePath + ": " + buffered.length + " chars / " + lineCount(buffered) + " lines. Ends with:\n" + lastLines(buffered, 10),
        success: true
      };
    }

    if (action === "clear" || action === "discard" || action === "reset") {
      // Friction on serial discards (round-11 lesson: the model deleted
      // parked drafts to regenerate — first via shell rm, and clear would
      // be the same loop with a different name). The first discard per
      // file is free; the second requires force, and the refusal steers
      // back to the one-line repair.
      var clearMarker = filePath + ".__cleared__";
      if (input.force !== true) {
        var clearedBefore = null;
        try {
          clearedBefore = Nanna.readFile(clearMarker);
        } catch (eMk) {
          // First discard.
        }
        if (clearedBefore !== null && clearedBefore !== undefined) {
          var steer = "";
          if (buffered !== null && buffered !== "") {
            var clearDetail = pythonSyntaxRefusal(filePath, buffered);
            if (clearDetail) {
              steer = " The current draft has exactly one blocking error (" + clearDetail + ") — fixing that one line is faster than regenerating.";
            }
          }
          return fail("CLEAR REFUSED — you already discarded one draft for " + filePath + "; discarding again is the regeneration loop." + steer + " Repair the draft: edit_file(file_path=\"" + bufPath + "\", old_string=<the broken line>, new_string=<the fix>), then file_buffer(action=\"commit\"). Pass force=true to discard anyway.");
        }
        try { Nanna.writeFile(clearMarker, "1"); } catch (eWk) { /* best effort */ }
      }
      try {
        Nanna.exec("rm -f '" + bufPath + "'", null, 15);
      } catch (eRm2) {
        try { Nanna.writeFile(bufPath, ""); } catch (eZ2) { /* best effort */ }
      }
      return { content: "Buffer for " + filePath + " discarded. The real file was not touched. NOTE: the next discard for this file will require force=true — repair drafts instead of regenerating them.", success: true };
    }

    return fail("file_buffer failed: unknown action '" + action + "'. Use append (add the next chunk), commit (write the file), show (preview), or clear (discard). Nothing was changed.");
  }
}
