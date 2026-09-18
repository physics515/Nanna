export default {
  name: "lift_invariant",
  requires: ["invariants.lift"],
  version: "0.1.0",
  // Engine deadline in seconds: the longest ask_user wait (1800) plus margin.
  timeout: 1830,
  description: "Ask the user to lift a file rule THEY declared (e.g. \"don't touch tests/\") when it genuinely blocks the goal. Their original sentence is quoted back to them; the rule is removed only if they reply with a clear yes. Use the glob named in the refusal. Never a way around the rule: without their yes, nothing changes.",
  parameters: {
    type: "object",
    properties: {
      glob: { type: "string", description: "The protected path or glob, exactly as the refusal named it" },
      reason: { type: "string", description: "One sentence: why the rule blocks the goal" },
      wait_secs: { type: "integer", description: "How long to wait for the reply (default 600, max 1800)" }
    },
    required: ["glob", "reason"]
  },
  execute: function(input) {
    // The same relative path write_file's guard reads, resolved by the same
    // bridge — so the rule lifted here is the rule enforced there.
    var REGISTRY = ".nanna/declared_invariants.json";
    try {
      var registry = "";
      try { registry = Nanna.readFile(REGISTRY) || ""; } catch (e) { registry = ""; }
      var r = Nanna.service("invariants.lift", {
        session_id: Nanna.sessionId(),
        registry: registry,
        glob: input.glob,
        reason: input.reason,
        wait_secs: input.wait_secs
      });
      if (r.lifted) {
        Nanna.writeFile(REGISTRY, r.registry);
        return "The user said yes: the rule on `" + r.glob + "` is lifted. You may now change those files.";
      }
      if (r.reason === "no_live_turn") {
        return { content: "lift_invariant: the question was posted, but there is no running turn to receive the answer. The rule stays until the user replies and you ask again.", success: false };
      }
      if (r.reply) {
        return { content: "lift_invariant: the user replied \"" + r.reply + "\", which is not a clear yes. The rule stays in force.", success: false };
      }
      return { content: "lift_invariant: no reply came in time. The rule stays in force; continue without changing those files.", success: false };
    } catch (e) {
      var msg = "" + (e && e.message ? e.message : e);
      return { content: "lift_invariant: " + msg, success: false };
    }
  }
}
