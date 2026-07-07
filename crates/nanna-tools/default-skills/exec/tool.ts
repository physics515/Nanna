export default {
  name: "exec",
  version: "0.1.0",
  output: "context",
  description: "Execute a shell command and return its output. Use for running build commands, scripts, git operations, etc.",
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
