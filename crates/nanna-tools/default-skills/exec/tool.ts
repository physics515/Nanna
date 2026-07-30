export default {
  name: "exec",
  version: "0.1.3",
  output: "memory",
  // Script-engine deadline (seconds). This is only a backstop for a hung script:
  // the shell bridge owns the real per-command timeout — the `timeout` parameter
  // below, or an auto-detected 30s/120s — and kills the child when it fires. This
  // ceiling must stay ABOVE the bridge's widest auto-detect (120s) so a long
  // build/VCS command is never preempted by the engine (which would orphan the
  // child). A larger explicit `timeout` extends this deadline automatically.
  timeout: 180,
  description: "Execute a shell command in a POSIX bash shell (Git Bash on Windows, sh on Unix) and return its output. ALWAYS bash syntax: pipes, &&, ||, [ -f x ] / [ -d x ], ls, cat/grep/tail, mkdir -p, 2>/dev/null, forward-slash paths. NEVER cmd.exe syntax — 'if exist', '2>nul', 'cd /d', 'errorlevel' all FAIL here. To search code, use the code_search tool — rg/ripgrep is not guaranteed on PATH. Use for build commands, scripts, git operations, etc.",
  parameters: {
    type: "object",
    properties: {
      command: { type: "string", description: "Shell command to execute. Runs in the workspace directory by default." },
      workdir: { type: "string", description: "Working directory for the command. Defaults to the workspace directory if omitted." },
      timeout: { type: "integer", description: "Timeout in seconds. Default: 30 for simple commands, 120 for git/cargo/npm/build tools (auto-detected). Override with a specific value if needed." }
    },
    required: ["command"]
  },
  execute: function(input) {
    // Accept multiple parameter name variants from different models
    if (!input.command) {
      input.command = input.cmd || input.script || input.shell || input.bash_command || input.shell_command;
    }
    if (!input.command) {
      return { content: "Error: Missing required parameter: command", success: false };
    }

    // --- anti-erosion: shell redirection over a ratchet-protected file ---
    //
    // Mirrors write_file/edit_file's key normalization exactly, or the
    // lookup misses and the guard silently does nothing. (The ratchet keys
    // on "./minidb" and "minidb" as the same file; so must this.)
    var HIWATER_STATE = ".nanna/write_hiwater.json";
    function hiwaterKey(path) {
      var k = path.split("\\").join("/").toLowerCase();
      while (k.indexOf("./") === 0) k = k.substring(2);
      while (k.indexOf("//") !== -1) k = k.split("//").join("/");
      return k;
    }
    function hiwaterMap() {
      try {
        var raw = Nanna.readFile(HIWATER_STATE);
        var parsed = JSON.parse(raw);
        if (parsed && typeof parsed === "object") return parsed;
      } catch (e) {
        // No state, or unreadable: nothing is protected yet. Fail OPEN.
      }
      return {};
    }

    // Collect targets of CLOBBERING redirects only. `>>` appends and is
    // safe; `>` and `>|` truncate, and `tee` without -a truncates.
    function clobberTargets(command) {
      var targets = [];
      var i = 0;
      while (i < command.length) {
        var ch = command.charAt(i);
        if (ch === ">") {
          // Skip `>>` (append) entirely, both characters.
          if (command.charAt(i + 1) === ">") { i += 2; continue; }
          var j = i + 1;
          if (command.charAt(j) === "|") j++;          // `>|` force-clobber
          while (j < command.length && (command.charAt(j) === " " || command.charAt(j) === "\t")) j++;
          var tok = "";
          while (j < command.length) {
            var c = command.charAt(j);
            if (c === " " || c === "\t" || c === "\n" || c === ";" || c === "&" ||
                c === "|" || c === "<" || c === ">" || c === ")") break;
            tok += c;
            j++;
          }
          // Strip surrounding quotes the model may have added.
          if (tok.length >= 2) {
            var f = tok.charAt(0), l = tok.charAt(tok.length - 1);
            if ((f === '"' && l === '"') || (f === "'" && l === "'")) tok = tok.substring(1, tok.length - 1);
          }
          if (tok.length > 0) targets.push(tok);
          i = j;
          continue;
        }
        i++;
      }
      // `tee FILE` (no -a/--append) truncates just like `>`.
      var teeAt = command.indexOf("tee ");
      if (teeAt !== -1) {
        var rest = command.substring(teeAt + 4);
        if (rest.indexOf("-a") !== 0 && rest.indexOf("--append") !== 0) {
          var tok2 = rest.split(" ")[0].split("|")[0].split(";")[0].trim();
          if (tok2.length > 0 && tok2.charAt(0) !== "-") targets.push(tok2);
        }
      }
      return targets;
    }

    function redirectClobberRefusal(command) {
      var targets = clobberTargets(command);
      if (targets.length === 0) return null;
      var map = hiwaterMap();
      for (var t = 0; t < targets.length; t++) {
        var raw = targets[t];
        // A redirect into a variable or process substitution is not a path
        // we can reason about; leave it alone.
        if (raw.indexOf("$") !== -1 || raw.indexOf("/dev/") === 0) continue;
        var entry = map[hiwaterKey(raw)];
        var hi = entry && typeof entry.hi === "number" && isFinite(entry.hi) ? entry.hi : 0;
        if (hi > 500) {
          return "NOT EXECUTED — the file was NOT modified and is fully intact. " +
            "This command redirects over " + raw + ", which is a file you built with " +
            "write_file/edit_file and which has held " + hi + " bytes. Shell redirection " +
            "replaces the WHOLE file with no safety check, and that is how a working " +
            "script gets silently replaced by a shorter one that drops features you " +
            "already had passing.\n\n" +
            "Nothing failed and nothing is lost — " + raw + " is exactly as it was.\n\n" +
            "To change part of it: edit_file(file_path=\"" + raw + "\", old_string=<exact current text>, new_string=<replacement>).\n" +
            "To replace all of it on purpose: read_file(\"" + raw + "\") first, merge your change " +
            "into the FULL text, then write_file with the complete content.\n" +
            "Redirect to a DIFFERENT path if you meant to write scratch output.";
        }
      }
      return null;
    }

    var denylist = [
      "rm -rf /",
      "rm -rf /*",
      "format C:",
      "mkfs",
      "dd if=/dev/zero",
      ":(){ :|:& };:"
    ];

    var cmdLower = input.command.toLowerCase().trim();
    for (var i = 0; i < denylist.length; i++) {
      if (cmdLower.indexOf(denylist[i].toLowerCase()) === 0) {
        return "Error: Command blocked by safety check: \"" + denylist[i] + "\"";
      }
    }

    // This shell is Git Bash, not cmd.exe. Catch unambiguous cmd.exe syntax
    // BEFORE running it so the model gets a correction instead of a cryptic
    // bash parse error (observed live: small models mix shells). Markers are
    // chosen to avoid matching legitimate bash that merely mentions the
    // words (e.g. grep for "errorlevel" in code is fine; "if errorlevel "
    // is the cmd.exe conditional).
    var cmdisms = ["if exist ", "if not exist ", "cd /d ", "if errorlevel "];
    var cmdism = null;
    for (var j = 0; j < cmdisms.length; j++) {
      if (cmdLower.indexOf(cmdisms[j]) !== -1) { cmdism = cmdisms[j].trim(); break; }
    }
    if (!cmdism) {
      // ">nul" only counts as cmd.exe when it stands alone — not as a
      // prefix of a real filename like ">nul_check.txt".
      var nulAt = cmdLower.indexOf(">nul");
      while (nulAt !== -1) {
        var afterNul = cmdLower.charAt(nulAt + 4);
        if (afterNul === "" || afterNul === " " || afterNul === "\t" || afterNul === "&" || afterNul === "2") {
          cmdism = ">nul";
          break;
        }
        nulAt = cmdLower.indexOf(">nul", nulAt + 4);
      }
    }
    // Buffer files are MANAGED state: deleting one with the shell defeats
    // the parked-draft repair loop (observed live: rm -f '*.__buffer__'
    // then regenerate, forever). The legal doors are edit_file + commit,
    // or file_buffer action="clear".
    if ((cmdLower.indexOf("rm ") !== -1 || cmdLower.indexOf("del ") !== -1 || cmdLower.indexOf("unlink") !== -1) && cmdLower.indexOf(".__buffer__") !== -1) {
      return {
        content: "NOT EXECUTED — buffer files (*.__buffer__) are managed drafts; do not delete them with the shell. Either REPAIR the draft (edit_file the broken line, then file_buffer action=\"commit\") or discard it properly with file_buffer(action=\"clear\", file_path=<the real file>).",
        success: false
      };
    }
    if (!cmdism && cmdLower.indexOf("type ") === 0 && (cmdLower.indexOf(":\\") !== -1 || cmdLower.indexOf(":/") !== -1)) {
      // cmd.exe `type <file>` prints a file; bash `type` describes a
      // command, so `type D:\...` just echoes the path back (observed
      // live twice — the model believed it had read the file).
      return {
        content: "NOT EXECUTED — in bash, 'type' does not print files (that is cmd.exe). Use: cat \"" + (input.command.split(" ").slice(1).join(" ").split("|")[0].trim()) + "\" instead. Then call exec again.",
        success: false
      };
    }
    if (cmdism) {
      return {
        content: "NOT EXECUTED — exec runs Git Bash (POSIX), not cmd.exe, and your command contains cmd.exe syntax ('" + cmdism + "'). Rewrite with bash: '[ -d path ]' / '[ -f path ]' to test existence, 'ls' to list, '2>/dev/null' to silence errors, 'mkdir -p' to create dirs. Then call exec again.",
        success: false
      };
    }

    // Shell redirection is an UNGUARDED WRITE PATH, and it silently defeats
    // the anti-erosion ratchet that write_file/edit_file enforce.
    //
    // Observed live 2026-07-28, qwen3.5:9b on the 42-feature ladder: the
    // model built ./minidb up to 8620 bytes through write_file/edit_file,
    // then ran `cat > ./minidb << 'EOF'` with a ~1.7 KB script. That write
    // took no 30% floor check AND left the ratchet's `last` stale, so the
    // next write_file saw disk != last, re-based to disk truth, and the
    // 8620-byte high-water collapsed to 1719. From there every subsequent
    // shrink was "acceptable" relative to the laundered peak, and the run
    // fell from 19/42 to 6/42 while looking healthy the whole way.
    //
    // Scoped deliberately: only files the ratchet is ALREADY protecting are
    // refused. A redirect to out.txt, a temp file, or anything the agent has
    // never built with write_file has no ratchet entry and passes straight
    // through — including the ladder's own `sh tests/test_NN.sh >out.txt`.
    // So this closes the erosion hole without touching ordinary shell work.
    var clobber = redirectClobberRefusal(input.command);
    if (clobber) {
      return { content: clobber, success: false };
    }

    // Bridge failures (spawn errors, bad workdir, missing files) must be
    // RETURNED, not thrown — a thrown error reaches the model under five
    // stacked "Execution failed:" prefixes (observed live: os error 267 in
    // a retry loop). Name the likely cause so the retry is a correction.
    var result;
    try {
      result = Nanna.exec(input.command, input.workdir, input.timeout);
    } catch (e) {
      var bridgeErr = String(e && e.message ? e.message : e);
      if (bridgeErr.length > 200) bridgeErr = bridgeErr.substring(0, 200);
      var hint = "";
      if (bridgeErr.indexOf("directory name is invalid") !== -1 || bridgeErr.indexOf("os error 267") !== -1) {
        hint = " The workdir you passed ('" + (input.workdir || "") + "') is not a valid directory — pass an existing DIRECTORY (not a file path), or omit workdir to use the workspace default.";
      } else if (bridgeErr.indexOf("cannot find the file") !== -1 || bridgeErr.indexOf("os error 2") !== -1) {
        hint = " A path in the command or workdir does not exist — check it with ls, then retry.";
      }
      return {
        content: "exec could not start the command (" + bridgeErr + ")." + hint + " Nothing ran; retry with the correction.",
        success: false
      };
    }

    var output = result.stdout;
    if (result.stderr) {
      output += output ? "\n" : "";
      output += "--- stderr ---\n" + result.stderr;
    }

    if (!result.success) {
      output = "Command failed (exit code " + result.code + ")\n" + output;
    }

    return { content: output || "(no output)", success: result.success };
  }
}
