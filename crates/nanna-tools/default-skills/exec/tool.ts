export default {
  name: "exec",
  description: "Execute a shell command and return its output. Use for running build commands, scripts, git operations, etc.",
  parameters: {
    type: "object",
    properties: {
      command: { type: "string", description: "Shell command to execute" },
      workdir: { type: "string", description: "Working directory for the command" },
      timeout: { type: "integer", description: "Timeout in seconds. Default: 30" }
    },
    required: ["command"]
  },
  execute: function(input) {
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

    var result = Nanna.exec(input.command, input.workdir);

    var output = result.stdout;
    if (result.stderr) {
      output += output ? "\n" : "";
      output += "--- stderr ---\n" + result.stderr;
    }

    if (!result.success) {
      output = "Command failed (exit code " + result.code + ")\n" + output;
    }

    return output || "(no output)";
  }
}
