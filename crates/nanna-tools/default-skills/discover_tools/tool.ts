export default {
  name: "discover_tools",
  version: "0.4.0",
  description: "Activate tools for file access, shell commands, web browsing, code analysis, and more. Search with a query describing what you need to DO (e.g. 'write file', 'run shell command', 'search code') — results are ranked best-match first and returned one page at a time. Each page's tools are activated and persist for the rest of this conversation; the reply says how to get the next page when more exist. You MUST call this before using any tool beyond remember/recall/reflect.",
  output: "context",
  parameters: {
    type: "object",
    properties: {
      query: {
        type: "string",
        description: "What you need to do, in plain words (e.g. 'write file', 'run tests'). Ranked search; leave empty or pass 'all' to page through the whole catalogue instead."
      },
      offset: {
        type: "number",
        description: "Optional: skip this many tools — pass the value the previous page's reply told you to continue with"
      },
      limit: {
        type: "number",
        description: "Optional: tools per page. Default 6, sized so a request stays within what small models handle; raise it only if you genuinely need more at once."
      }
    }
  },
  execute(input) {
    // Tolerate ANY input shape: null, undefined, {}, {query: ...}. A null
    // input must mean "list ALL tools", never a destructuring throw — a
    // thrown error reaches small models under stacked "Execution failed:"
    // prefixes and reads as corruption.
    var query = input && typeof input === "object" ? input.query : null;
    if (typeof query !== "string") {
      query = null;
    }

    // "all" MEANS all — it is not a search term.
    //
    // The description says "call with no arguments to see all available
    // tools", but a model asking for everything does not send `{}`, it sends
    // `{query: "all"}` — that is how you ask in words. The search path then
    // treated "all" as a term to match and returned the two tools whose text
    // happens to contain those letters (reCALL, instALL...), so the model asked
    // for the whole catalogue and was told the system has two tools.
    //
    // That is now the difference between a working run and a stuck one: since
    // discovery is the ONLY tool in the request, a model that cannot get the
    // catalogue cannot get anything. Observed live 2026-07-28 on gemma4:12b,
    // which asked repeatedly and kept receiving two.
    //
    // Matched on the whole trimmed query only, so a genuine search for
    // "list directory" or "install packages" still searches.
    if (query !== null) {
      var canonical = query.trim().toLowerCase();
      var MEANS_EVERYTHING = {
        "": true, "all": true, "*": true, "any": true, "all tools": true,
        "everything": true, "list": true, "list all": true, "list tools": true,
        "show all": true, "show tools": true, "available tools": true, "tools": true
      };
      if (MEANS_EVERYTHING[canonical] === true) {
        query = null;
      }
    }
    var allTools = Nanna.listTools();
    if (!allTools || !allTools.length) {
      return { content: "No tools available.", data: { activate_tools: [] } };
    }

    var CORE = { remember: true, recall: true, reflect: true, discover_tools: true };

    // Filter out core tools (already always available)
    var discoverable = [];
    for (var i = 0; i < allTools.length; i++) {
      if (!CORE[allTools[i].name]) {
        discoverable.push(allTools[i]);
      }
    }

    // Apply query filter if provided.
    //
    // Preferred path: engine-side ranked search (Nanna.searchTools — BM25 +
    // Snowball stemming + typo fallback in Rust). It handles word order,
    // morphology ("replace" matches "replacing") and typos ("wirte file").
    // The typeof guard keeps this working on engines that don't expose
    // searchTools; an empty result falls through to the local tokenizer
    // below so behavior degrades to exactly the old matching.
    var matched = discoverable;
    var usedRanked = false;
    if (query && query.length > 0 && typeof Nanna.searchTools === "function") {
      var ranked = null;
      try {
        ranked = Nanna.searchTools(query);
      } catch (e) {
        ranked = null;
      }
      if (ranked && ranked.length > 0) {
        // Map ranked names back onto the full definitions (for params),
        // preserving rank order and dropping core tools.
        var byName = {};
        for (var i = 0; i < discoverable.length; i++) {
          byName[discoverable[i].name] = discoverable[i];
        }
        var picked = [];
        for (var r = 0; r < ranked.length; r++) {
          var found = byName[ranked[r].name];
          if (found) {
            picked.push(found);
          }
        }
        if (picked.length > 0) {
          matched = picked;
          usedRanked = true;
        }
      }
    }

    // Fallback path: the query is TOKENIZED and matched per word: "file
    // write" must find the file tools exactly like "write file" does — the
    // old whole-phrase substring match returned nothing unless the words
    // happened to appear in that order. Tools matching more query words
    // rank first.
    if (query && query.length > 0 && !usedRanked) {
      var terms = [];
      var cur = "";
      var ql = query.toLowerCase();
      for (var ci = 0; ci < ql.length; ci++) {
        var ch = ql.charAt(ci);
        var isWord = (ch >= "a" && ch <= "z") || (ch >= "0" && ch <= "9") || ch === "_";
        if (isWord) {
          cur += ch;
        } else if (cur.length > 0) {
          terms.push(cur);
          cur = "";
        }
      }
      if (cur.length > 0) {
        terms.push(cur);
      }

      if (terms.length > 0) {
        var scored = [];
        for (var i = 0; i < discoverable.length; i++) {
          var tool = discoverable[i];
          var hay = (tool.name + " " + (tool.description || "")).toLowerCase();
          var hits = 0;
          for (var t = 0; t < terms.length; t++) {
            if (hay.indexOf(terms[t]) >= 0) {
              hits++;
            }
          }
          if (hits > 0) {
            scored.push({ tool: tool, hits: hits });
          }
        }
        scored.sort(function (a, b) { return b.hits - a.hits; });
        matched = [];
        for (var i = 0; i < scored.length; i++) {
          matched.push(scored[i].tool);
        }
      }
    }

    if (matched.length === 0) {
      return {
        content: "No tools found matching '" + query + "'. Try a broader query or call discover_tools() with no query to see all.",
        data: { activate_tools: [] }
      };
    }

    // Paging is the DEFAULT, and the page size is derived, not tuned.
    //
    // Every tool this call activates rides along as a full definition on every
    // later request, and the P14 scoping work measured small models degrading
    // once a request carries more than 5-10 definitions. The always-on core
    // set is 4 (remember/recall/reflect/discover_tools), so a page of 6 fills
    // a request to 10 — the top of the band. That is where 6 comes from.
    //
    // The old behavior — paging only when asked — activated the whole
    // catalogue on `query:"all"`, because a model that wants everything never
    // passes a limit. Observed 2026-07-29 (gemma4:12b): one such call put all
    // 30 definitions in every subsequent request, and the model spent two
    // hours in wonder/web_search/ask_parent without ever creating the
    // artifact. Ranking makes the default page the RIGHT page: the search
    // path returns best-match-first, so the six shown are the six it asked
    // for in words.
    //
    // An explicit `limit` still wins — how much to take at once stays the
    // model's call; the default only answers for the model that never asks.
    var PAGE_DEFAULT = 6;
    var total = matched.length;
    var offset = input && typeof input.offset === "number" && isFinite(input.offset)
      ? Math.max(0, Math.floor(input.offset)) : 0;
    var limit = input && typeof input.limit === "number" && isFinite(input.limit) && input.limit > 0
      ? Math.floor(input.limit) : PAGE_DEFAULT;
    var endAt = offset + limit;
    var paged = offset > 0 || endAt < total;
    matched = matched.slice(offset, endAt);
    if (matched.length === 0 && paged) {
      return {
        content: "No tools at offset " + offset + " — there are " + total + " in total. " +
          "Call discover_tools with a smaller offset, or with no offset to start from the top.",
        data: { activate_tools: [] }
      };
    }

    // Format output
    var lines = [];
    var names = [];
    for (var i = 0; i < matched.length; i++) {
      var tool = matched[i];
      names.push(tool.name);
      var entry = "## " + tool.name + "\n" + (tool.description || "(no description)");
      if (tool.parameters && tool.parameters.length > 0) {
        var paramParts = [];
        for (var j = 0; j < tool.parameters.length; j++) {
          var p = tool.parameters[j];
          var part = p.name + " (" + (p.type || "string");
          if (p.required) {
            part = part + ", required";
          }
          part = part + ")";
          paramParts.push(part);
        }
        entry = entry + "\nParams: " + paramParts.join(", ");
      }
      lines.push(entry);
    }

    // A PARTIAL list must say so, in full, every time.
    //
    // This is the same rule the tool-result stubs follow: an artifact that has
    // been cut must announce WHAT was cut, WHY, that the operation SUCCEEDED,
    // and exactly how to get the rest. A page that merely says "Found 10
    // tool(s)" is indistinguishable from "this system has 10 tools", and a
    // model that believes it has seen the whole catalogue stops looking — which
    // is precisely the failure `query: "all"` returning two tools produced.
    var shown = matched.length;
    var nextOffset = offset + shown;
    var remaining = total - nextOffset;
    var header = paged
      ? "Showing tools " + (offset + 1) + "-" + nextOffset + " of " + total + " total"
      : "Found " + shown + " tool(s)";
    var content = header + ":\n\n" + lines.join("\n\n");

    // Everything listed above is now usable — say that too, since the whole
    // point of the call was to be able to act.
    content += "\n\nThe " + shown + " tool(s) above are ACTIVATED and ready to use for the " +
      "rest of this run — call them directly, you do not need to discover them again.";

    if (remaining > 0) {
      // The continuation must repeat the caller's own query, or page 2 of a
      // search for "file" silently becomes page 2 of the whole catalogue.
      var queryArg = query && query.length > 0 ? "query=\"" + query + "\", " : "query=\"all\", ";
      content += "\n\n*** THIS IS A PARTIAL LIST — " + remaining + " more tool(s) matched but " +
        "are NOT shown and NOT yet activated. *** Nothing failed and nothing is missing from " +
        "the system; results come a page at a time. If what you need is not above, get the " +
        "next page with:\n" +
        "    discover_tools(" + queryArg + "offset=" + nextOffset + ")\n" +
        "Results are ranked best-match first, so the next page is a worse match than this " +
        "one — before paging on, consider whether a tool above already does the job.";
    }

    return { content: content, data: { activate_tools: names } };
  }
}
