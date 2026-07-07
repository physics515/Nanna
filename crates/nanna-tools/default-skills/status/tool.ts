export default {
  name: "status",
  version: "0.1.0",
  output: "context",
  description: "Get system status information. Shows platform, working directory, git status, and environment overview.",
  parameters: {
    type: "object",
    properties: {
      verbose: { type: "boolean", description: "Include detailed environment info. Default: false" }
    },
    required: []
  },
  execute: function(input) {
    var lines = [];
    lines.push("# System Status");
    lines.push("Platform: " + Nanna.platform);

    // Working directory - prefer Nanna.workdir (set from active workspace)
    if (Nanna.workdir) {
      lines.push("Working directory: " + Nanna.workdir);
    } else {
      var pwdResult = Nanna.exec(Nanna.platform === "win32" ? "cd" : "pwd");
      if (pwdResult.success) {
        lines.push("Working directory: " + pwdResult.stdout.trim());
      }
    }

    // Git status
    var gitResult = Nanna.exec("git status --short --branch");
    if (gitResult.success) {
      lines.push("");
      lines.push("## Git");
      lines.push(gitResult.stdout.trim());
    }

    // Node/Python/Rust versions if verbose
    if (input.verbose) {
      lines.push("");
      lines.push("## Toolchain");

      var tools = [
        { cmd: "rustc --version", name: "Rust" },
        { cmd: "node --version", name: "Node" },
        { cmd: "python --version", name: "Python" },
        { cmd: "go version", name: "Go" }
      ];

      for (var i = 0; i < tools.length; i++) {
        var r = Nanna.exec(tools[i].cmd);
        if (r.success) {
          lines.push(tools[i].name + ": " + r.stdout.trim());
        }
      }
    }

    return lines.join("\n");
  }
}
