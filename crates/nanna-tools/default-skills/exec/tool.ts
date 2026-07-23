export default {
  name: "exec",
  version: "0.1.1",
  output: "context",
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

    var result = Nanna.exec(input.command, input.workdir, input.timeout);

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
