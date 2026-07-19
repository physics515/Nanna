export default {
  name: "exec",
  version: "0.1.0",
  output: "context",
  // Script-engine deadline (seconds). This is only a backstop for a hung script:
  // the shell bridge owns the real per-command timeout — the `timeout` parameter
  // below, or an auto-detected 30s/120s — and kills the child when it fires. This
  // ceiling must stay ABOVE the bridge's widest auto-detect (120s) so a long
  // build/VCS command is never preempted by the engine (which would orphan the
  // child). A larger explicit `timeout` extends this deadline automatically.
  timeout: 180,
  description: "Execute a shell command in a POSIX shell (Git Bash on Windows, sh on Unix) and return its output. Use bash syntax: pipes, &&, [ -f x ], cat/grep/tail, forward-slash paths. Do not use cmd.exe idioms (no 'cd /d'; prefer forward slashes over backslashes). To search code, use the code_search tool — rg/ripgrep is not guaranteed on PATH. Use for build commands, scripts, git operations, etc.",
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
