export default {
  name: "day_dream",
  requires: ["memory.summarize"],
  version: "0.1.0",
  description: "Consolidate related memories on purpose. Give it memory ids, literal text, or both: they are concatenated and passed through the dreaming summarizer, which narrates them as one coherent account — drawing connections and throughlines rather than merely shortening. The narration is returned to you AND stored as a new memory, so the connection survives this conversation. Use it when you notice related fragments piling up, when several observations seem to be about the same underlying thing, or when you want to think something through and keep the result. Params: memories (array of memory ids and/or strings), focus (optional: what angle to read them through).",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      memories: {
        type: "array",
        items: { type: "string" },
        description: "Memory ids to pull from the store, and/or literal strings to include as-is. Mixed freely."
      },
      focus: {
        type: "string",
        description: "Optional angle for the narration, e.g. 'what keeps breaking the build' or 'what the user actually wants'"
      }
    },
    required: ["memories"]
  },

  execute: function (input) {
    var items = input.memories;
    if (!items || !items.length) {
      return {
        content: "day_dream needs a non-empty 'memories' array (ids, strings, or both). Nothing was consolidated.",
        success: false
      };
    }

    // An id-looking token is fetched from the store; anything else is taken as
    // literal text the model wants folded in. Fetch failures are reported, not
    // fatal — dreaming over what IS available beats refusing the whole call.
    var texts = [];
    var missing = [];
    var usedIds = [];
    for (var i = 0; i < items.length; i++) {
      var item = String(items[i]);
      var looksLikeId = /^[0-9a-fA-F-]{6,}$/.test(item);
      if (!looksLikeId) {
        texts.push(item);
        continue;
      }
      try {
        var got = Nanna.service("memory.get", { id: item, limit: 8000 });
        if (got && got.content) {
          texts.push(got.content);
          usedIds.push(item);
        } else {
          missing.push(item);
        }
      } catch (e) {
        missing.push(item);
      }
    }

    if (!texts.length) {
      return {
        content: "day_dream found none of those memories (" + missing.join(", ") +
          "). Nothing was consolidated. Use recall to find ids that exist, or pass the text directly.",
        success: false
      };
    }

    if (input.focus) {
      texts.unshift("Read these through this lens: " + input.focus);
    }

    var result;
    try {
      result = Nanna.service("memory.summarize", { texts: texts });
    } catch (e) {
      return {
        content: "day_dream could not reach the summarizer: " + e +
          ". Nothing was consolidated and no memories were changed.",
        success: false
      };
    }

    var narration = (result && result.summary) ? result.summary : "";
    if (!narration) {
      return {
        content: "day_dream got an empty narration back. Nothing was stored.",
        success: false
      };
    }

    // Store the narration so the connection outlives this conversation. The
    // sources are recorded rather than deleted: a model-invoked consolidation
    // must not destroy the raw material it drew on — scheduled dreaming is
    // what replaces originals, deliberately and with the whole store in view.
    var stored = "";
    try {
      Nanna.service("memory.store", {
        content: narration,
        tags: {
          category: "insight",
          fact_type: "observed",
          source: "day_dream",
          sources: usedIds.join(",")
        }
      });
      stored = " (stored as a memory)";
    } catch (e) {
      stored = " (NOT stored — the memory service refused: " + e + ")";
    }

    var note = "";
    if (missing.length) {
      note = "\n\n[" + missing.length + " id(s) not found and skipped: " + missing.join(", ") + "]";
    }

    return {
      content: narration + note + stored,
      data: { narration: narration, sources: usedIds, missing: missing }
    };
  }
}
