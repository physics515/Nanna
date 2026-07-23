export default {
  name: "edit_file",
  version: "0.1.1",
  output: "context",
  description: "Replace one exact text snippet in a file with new text — an in-place edit for small changes. Use this instead of rewriting the whole file with write_file. ALL THREE main parameters are REQUIRED: file_path, old_string, new_string. old_string must be text that exists in the file EXACTLY as written — include 2-3 surrounding lines to make it unique. Only the matched snippet changes; the rest of the file is untouched. Use write_file only for new files or full rewrites.",
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

    // Accept multiple parameter name variants from different models
    var filePath = input.file_path || input.filePath || input.path || input.file;

    // old/new variants accept ONLY string values, so a boolean flag like
    // replace=true can never be mistaken for the replacement text.
    var oldStr;
    var oldNames = ["old_string", "old_str", "old_text", "search", "find", "target"];
    for (var i = 0; i < oldNames.length; i++) {
      if (typeof input[oldNames[i]] === "string") { oldStr = input[oldNames[i]]; break; }
    }
    var newStr;
    var newNames = ["new_string", "new_str", "new_text", "replacement", "replace_with", "replace"];
    for (var i = 0; i < newNames.length; i++) {
      if (typeof input[newNames[i]] === "string") { newStr = input[newNames[i]]; break; }
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
    if (oldStr === newStr) {
      return fail("edit_file failed: old_string and new_string are identical, so there is nothing to do. Nothing was changed. Set new_string to the modified text you want in " + filePath + ".");
    }

    var content;
    try {
      content = Nanna.readFile(filePath);
    } catch (e) {
      var readErr = String(e);
      if (readErr.length > 120) readErr = readErr.substring(0, 120) + "...";
      return fail("edit_file failed: could not read " + filePath + " (" + readErr + "). Nothing was changed. Check the path, or use write_file to create a new file.");
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

    if (content.indexOf(needle) < 0) {
      var head = oldStr.split("\r\n").join("\n").split("\n").slice(0, 3).join("\n");
      if (head.length > 120) head = head.substring(0, 120) + "...";
      return fail("edit_file failed: old_string not found in " + filePath + ". The file is UNCHANGED and intact. Searched for text starting with:\n" + head + "\nCall read_file first, copy the exact current text, then retry edit_file.");
    }

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

    var updated;
    var replaced;
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

    try {
      Nanna.writeFile(filePath, updated);
    } catch (e2) {
      var writeErr = String(e2);
      if (writeErr.length > 120) writeErr = writeErr.substring(0, 120) + "...";
      return fail("edit_file failed writing " + filePath + " (" + writeErr + "). Retry the same edit_file call; if it fails again, read the file to verify its current state before editing.");
    }

    return { content: "Edited " + filePath + ": replaced " + replaced + " occurrence(s). File is now " + updated.length + " characters.", success: true };
  }
}
