export default {
  name: "write_file",
  version: "0.1.3",
  output: "context",
  description: "Write content to a file. BOTH parameters are REQUIRED on every call: file_path AND content (the complete file text). A call without content does nothing and fails. Creates the file if it doesn't exist, overwrites if it does. SAFETY: the write is blocked if new content is less than 30% of the existing file size (likely truncation). Use force=true to override.",
  parameters: {
    type: "object",
    properties: {
      file_path: { type: "string", description: "REQUIRED. Path to the file to write. Relative paths are resolved against the workspace directory." },
      content: { type: "string", description: "REQUIRED. The complete text to write into the file. Never omit this — a write_file call without content always fails." },
      force: { type: "boolean", description: "If true, skip the truncation safety check. Use when you intentionally want to write a smaller file." }
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

    // "Never turn valid Python into invalid Python." Observed live: the
    // model rewrote a working script into semicolon slop mid-mission. If
    // the CURRENT text parses and the NEW text does not, refuse. Repairs
    // of already-broken files pass through, non-.py files pass through,
    // and ANY checker failure fails OPEN (a missing python interpreter
    // must never block writes). Returns an error string or null.
    function pythonSyntaxRefusal(path, currentText, nextText) {
      var lower = path.toLowerCase();
      if (lower.length < 3 || lower.lastIndexOf(".py") !== lower.length - 3) return null;
      try {
        var chk = path + ".__chk.py";
        var oldTmp = path + ".__chk_old.py";
        var newTmp = path + ".__chk_new.py";
        Nanna.writeFile(oldTmp, currentText);
        Nanna.writeFile(newTmp, nextText);
        Nanna.writeFile(chk,
          "import ast, sys\n" +
          "def check(p):\n" +
          "    try:\n" +
          "        ast.parse(open(p, encoding='utf-8').read())\n" +
          "        return None\n" +
          "    except SyntaxError as e:\n" +
          "        return 'line ' + str(e.lineno) + ': ' + str(e.msg)\n" +
          "old_err = check(sys.argv[1])\n" +
          "new_err = check(sys.argv[2])\n" +
          "print('OLD_OK' if old_err is None else 'OLD_BAD')\n" +
          "print('NEW_OK' if new_err is None else 'NEW_BAD ' + new_err)\n");
        var cmd = "python '" + chk + "' '" + oldTmp + "' '" + newTmp + "'; rc=$?; rm -f '" + chk + "' '" + oldTmp + "' '" + newTmp + "'; exit $rc";
        var result = Nanna.exec(cmd, null, 30);
        var out = result && result.stdout ? result.stdout : "";
        if (out.indexOf("OLD_OK") !== -1 && out.indexOf("NEW_BAD") !== -1) {
          var detail = out.substring(out.indexOf("NEW_BAD") + 8);
          var nl = detail.indexOf("\n");
          if (nl !== -1) detail = detail.substring(0, nl);
          if (detail.length > 160) detail = detail.substring(0, 160);
          return "WRITE REFUSED — " + path + " currently contains VALID Python, but your new content has a SYNTAX ERROR (" + detail + "). The file is UNCHANGED. Fix the syntax and call write_file again with the corrected COMPLETE text. Pass force=true only to write broken code on purpose.";
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
            "(3) call write_file again with the complete content. " +
            "Only use force=true if you truly want the file replaced by this smaller version.",
          success: false
        };
      }
    }

    if (!input.force) {
      var existingText = null;
      try {
        existingText = Nanna.readFile(filePath);
      } catch (eRead) {
        // New file — nothing to protect.
      }
      if (existingText !== null && existingText !== undefined) {
        var syntaxRefusal = pythonSyntaxRefusal(filePath, existingText, fileContent);
        if (syntaxRefusal) return fail(syntaxRefusal);
      }
    }

    try {
      Nanna.writeFile(filePath, fileContent);
    } catch (e2) {
      var writeErr = String(e2);
      if (writeErr.length > 120) writeErr = writeErr.substring(0, 120) + "...";
      return fail("write_file failed writing " + filePath + " (" + writeErr + "). Retry the same call; if it fails again, read_file to verify the file state.");
    }

    var message = "Wrote " + bytes + " bytes to " + filePath;

    // Versioned-copy advisory (observed live: models fork foo.py.new2 /
    // foo_fixed_v1.txt instead of editing foo.py in place, then lose track
    // of which copy is real). The write succeeded — this only teaches.
    var baseName = filePath.split("\\").join("/").split("/").pop().toLowerCase();
    var copyMarkers = [".new", "_v1", "_v2", "_v3", "_v4", "_v5", "_fixed", "_backup", "_temp", "_copy", "_part", "_old", "_final", "_cleaned", "_scrubbed"];
    for (var m = 0; m < copyMarkers.length; m++) {
      if (baseName.indexOf(copyMarkers[m]) !== -1) {
        message += ". NOTE: this filename looks like a versioned copy. Do NOT fork versions — keep ONE real file and change it in place with edit_file (file_path + old_string + new_string). Delete stray copies when done.";
        break;
      }
    }

    return { content: message, written: fileContent };
  }
}
