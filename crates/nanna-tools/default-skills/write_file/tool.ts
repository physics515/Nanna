export default {
  name: "write_file",
  version: "0.1.9",
  output: "context",
  description: "Write content to a file. BOTH parameters are REQUIRED on every call: file_path AND content (the complete file text). A call without content does nothing and fails. Creates the file if it doesn't exist, overwrites if it does. For files too long to write in one call, use file_buffer (append chunks, then commit) instead. SAFETY: blocked if new content is under 30% of the existing file size (likely truncation), if a .py file would not parse, or if the filename looks like a versioned copy.",
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
    var fileContent = input.content || input.text || input.data || input.new_content || input.file_content;
    if (!filePath && (fileContent === undefined || fileContent === null)) {
      return fail("write_file failed: you must pass BOTH file_path AND content. Call it again like: write_file(file_path=\"D:/path/to/file.py\", content=\"<the complete file text>\")");
    }
    if (!filePath) {
      return fail("write_file failed: missing file_path. Nothing was written. Call it again with BOTH file_path (the destination path) AND content.");
    }
    if (fileContent === undefined || fileContent === null) {
      return fail("write_file failed: missing content. Nothing was written. Call write_file again with file_path=\"" + filePath + "\" AND content set to the COMPLETE file text.");
    }

    var bytes = fileContent.length;

    // Safety check BEFORE writing: block if new content is drastically smaller
    // than existing file. This prevents the model from accidentally overwriting
    // a large file with truncated/compressed content when it lost context.
    if (!input.force) {
      var existingSize = 0;
      try {
        var existing = Nanna.readFile(filePath);
        if (existing) existingSize = existing.length;
      } catch (e) {
        // File doesn't exist yet, no check needed
      }

      if (existingSize > 500 && bytes < existingSize * 0.3) {
        return {
          content: "WRITE REFUSED — the file was NOT modified and is fully intact. " +
            "You tried to write only " + bytes + " bytes over " + filePath +
            " which currently holds " + existingSize + " bytes (" +
            Math.round(bytes / existingSize * 100) + "% of it). That usually means " +
            "you sent a fragment instead of the whole file. To proceed: " +
            "(1) read_file " + filePath + ", (2) merge your change into the FULL text, " +
            "(3) call write_file again with the complete content.",
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
        return fail("WRITE REFUSED — '" + filePath + "' looks like a versioned copy ('" + copyHit + "'). Nothing was written. Keep ONE real file: change the ORIGINAL in place with edit_file, or write the full corrected content directly to the original path (a complete valid rewrite is always accepted).");
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
          return fail("WRITE BLOCKED — this content has a SYNTAX ERROR (" + syntaxDetail + ") and a parked draft for " + filePath + " already exists at " + railBufPath + " (" + railParked.length + " chars). Repair THAT draft: edit_file(file_path=\"" + railBufPath + "\", old_string=<the broken line>, new_string=<the fix>), then file_buffer(action=\"commit\", file_path=\"" + filePath + "\"). A fully VALID rewrite of " + filePath + " would also be accepted.");
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

    // Deliberately NO echo of the written content: echoing the whole file
    // made the result exceed the context threshold, and the model read the
    // resulting truncation stub as "my write was discarded" (observed
    // live, round 7). The file on disk is the source of truth.
    return { content: "Wrote " + bytes + " bytes to " + filePath + ". The file on disk now holds exactly this content." };
  }
}
