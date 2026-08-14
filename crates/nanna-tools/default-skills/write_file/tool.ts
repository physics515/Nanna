export default {
  name: "write_file",
  version: "0.1.15",
  output: "memory",
  description: "Write content to a file. BOTH parameters are REQUIRED on every call: file_path AND content (the complete file text). A call without content does nothing and fails. Creates the file if it doesn't exist, overwrites if it does. For files too long to write in one call, use file_buffer (append chunks, then commit) instead. SAFETY: a shrinking rewrite of a file that changed since you last read it returns the file's CURRENT content to merge (not a refusal); blocked if new content is under 30% of the last known-good size (likely truncation), if a .py file would not parse, or if the filename looks like a versioned copy. After each write the cheapest structural check (sh -n / node --check / JSON.parse) runs and its verdict is appended; a full overwrite parks the previous version at <file>.__prev__ for recovery.",
  parameters: {
    type: "object",
    properties: {
      file_path: { type: "string", description: "REQUIRED. Path to the file to write. Relative paths are resolved against the workspace directory." },
      content: { type: "string", description: "REQUIRED. The complete text to write into the file. Never omit this — a write_file call without content always fails." },
    },
    required: ["file_path", "content"]
  },
  execute: function(input) {
    // Guidance errors are RETURNED, not thrown: a thrown script error reaches
    // the model under five stacked "Execution failed:" prefixes, which small
    // models read as corruption and spiral on.
    function fail(message) {
      return { content: message, success: false };
    }

    // Guard events are logged at INFO so they can be audited from the daemon
    // log after a run. P22 evidence: `grep 'rewrite REMOVED'` over a 4-hour
    // leg returned 0 hits with no way to tell "never fired" from "fired,
    // unlogged" — tool results reach the log only as a ~25-char memory
    // prefix. A safety net you cannot audit is not a safety net. Best-effort.
    function glog(msg) {
      try { Nanna.log("info", msg); } catch (e) { /* logging is optional */ }
    }

    // A refusal the model keeps re-earning has stopped being information.
    //
    // Observed live 2026-07-28 (qwen3.5:9b, 42-feature ladder): the fork guard
    // correctly refused ./minidb.sh because ./minidb already existed, and named
    // the right path and the exact call to make. The model read it and tried
    // ./minidb.sh again — 74 times, 45% of every write attempt in the run. The
    // guard prevented the damage and none of the waste.
    //
    // The explanation is not the problem; its SHAPE is. Each step re-anchors
    // context, so every attempt is effectively the model's first, and a
    // paragraph of reasoning is what a small model skims. So after a couple of
    // identical refusals the message stops explaining and starts commanding:
    // short, imperative, the correct call and nothing else to weigh against it.
    //
    // Counted per path, per workspace, in the same .nanna state dir as the
    // ratchet. Fails OPEN in every direction — a missing or corrupt counter
    // just yields the normal message.
    var REFUSAL_STATE = ".nanna/write_refusals.json";
    var REFUSAL_ESCALATE_AT = 2;
    var REFUSAL_MAX_ENTRIES = 200;
    function refusalBump(key) {
      try {
        var map;
        try { map = JSON.parse(Nanna.readFile(REFUSAL_STATE)); } catch (e2) { map = {}; }
        if (!map || typeof map !== "object") map = {};
        var next = (typeof map[key] === "number" && isFinite(map[key]) ? map[key] : 0) + 1;
        map[key] = next;
        var keys = Object.keys(map);
        if (keys.length > REFUSAL_MAX_ENTRIES) {
          for (var i = 0; i < keys.length - REFUSAL_MAX_ENTRIES; i++) delete map[keys[i]];
        }
        Nanna.writeFile(REFUSAL_STATE, JSON.stringify(map));
        return next;
      } catch (e) {
        return 0;
      }
    }
    // `blunt` replaces `normal` once this path has been refused repeatedly.
    // Keyed "fork:<path>" and SHARED with edit_file on purpose: the model
    // alternated between the two tools on the same doomed path, so a
    // per-tool counter would let it spend the full quota twice over.
    function failEscalating(pathKey, normal, blunt) {
      var n = refusalBump("fork:" + pathKey);
      var msg = n > REFUSAL_ESCALATE_AT ? blunt : normal;
      glog("write_file guard refused (fork, attempt " + n + "): " + pathKey);
      return fail(msg);
    }

    // Anti-erosion ratchet (round-17 lesson): the 30% shrink floor used to be
    // relative to the CURRENT size, so repeated 60-80% rewrites during fault
    // storms compounded (0.7^n) and slowly hollowed files out without ever
    // tripping the guard. The floor base is tracked per workspace in
    // .nanna/write_hiwater.json (.nanna/ is the non-markdown local-state dir —
    // never beside user files, so no sidecar clutter). Each entry stores
    // {hi, last, at, good?, goodAt?, chk?}: `hi` is the high-water mark,
    // `last` is the size write_file ITSELF last left on disk, `good` is the
    // size of the newest version that passed a structural check and `chk` the
    // latest check verdict. While the file EXISTS, `hi` is MONOTONE: an
    // out-of-band change (edit_file, file_buffer, exec, the user) is more
    // evidence of held mass, folded in as max(hi, disk) — never a license to
    // restart the history downward. The 2026-08-08 ornith endurance log
    // showed why the old rule (out-of-band change re-bases hi to disk truth)
    // was a laundering hole: an exec append re-shaped the file, the next
    // write re-based hi 3794→1566, and an 875-byte rewrite that the true
    // history would have refused sailed through — the final artifact kept 8
    // of 37 verified commands.
    //
    // The SHRINK FLOOR, however, anchors on `good` — the LAST EVIDENCED-GOOD
    // size — not on `hi` (P22 Tier 3). Size is not quality: in a long session
    // the largest byte count is usually the most bloated, least-correct
    // draft. Observed 2026-08-10 (ornith leg): a 9768-byte draft that scored
    // 2/42 latched the high-water, and the 30% floor it set (2930 B) refused
    // a legitimate 2830-byte version and left the leg's eventual 16/42 peak
    // (3335 B = 34%) one small edit away from being refused — while the
    // 2952-byte write that actually cost two tests cleared it by 22 bytes.
    // When no structural check has ever passed (no checker applies), the
    // floor falls back to the old hi anchor, so nothing loses protection.
    //
    // Grow-writes are never refused regardless (the refusal requires bytes <
    // current disk size); a deliberate whole-file re-shape has honest doors:
    // delete + recreate (creation re-arms from the new size) or force. Every
    // state operation fails OPEN: a missing or corrupt state file degrades to
    // the old current-size behavior, never blocks a write.
    var HIWATER_STATE = ".nanna/write_hiwater.json";
    // Bound: the state file must stay trivially small over an unbounded
    // daemon lifetime; missions touch tens of files, so 200 entries with
    // least-recently-updated eviction loses nothing real.
    var HIWATER_MAX_ENTRIES = 200;
    // Slash/case normalization plus "./" stripping. Lowercase is correct
    // here because this daemon targets Windows paths.
    function hiwaterNormKey(path) {
      var k = path.split("\\").join("/").toLowerCase();
      while (k.indexOf("./") === 0) k = k.substring(2);
      while (k.indexOf("//") !== -1) k = k.split("//").join("/");
      return k;
    }
    // Canonical ledger key (P22 Tier 3): one file, ONE entry. The old key
    // kept relative and absolute spellings of the same file as separate
    // entries, and the split-brain was observed live (2026-08-10 ornith:
    // 'minidb' hi=9768 next to 'd:/development/.../minidb' hi=3195 — one
    // absolute-path edit silently cut the effective floor from 2930 to 958
    // bytes). Absolute spellings under the workspace root now collapse to
    // the relative form; hiwaterEntryFor still consults the old spelling so
    // pre-existing entries are found and folded forward (healed on the next
    // save). Fails open: no workdir → the old per-spelling behavior.
    function hiwaterKey(path) {
      var k = hiwaterNormKey(path);
      try {
        var wd = Nanna.workdir();
        if (wd) {
          var w = hiwaterNormKey(String(wd));
          if (w.charAt(w.length - 1) !== "/") w += "/";
          if (k.indexOf(w) === 0 && k.length > w.length) k = k.substring(w.length);
        }
      } catch (e) {
        // No workdir — spellings keep their own entries, as before.
      }
      return k;
    }
    // Transient park buffers and recovery copies must never be judged by (or
    // recorded in) cross-call history — they are rewritten wholesale — and
    // the ratchet's own state file guards itself specially (below).
    function hiwaterIsBuffer(key) {
      var buf = ".__buffer__";
      return key.length >= buf.length && key.lastIndexOf(buf) === key.length - buf.length;
    }
    function hiwaterIsPrev(key) {
      var p = ".__prev__";
      return key.length >= p.length && key.lastIndexOf(p) === key.length - p.length;
    }
    // Exact path only (root or any /.nanna/ dir), so a real work file with a
    // similar name keeps full ratchet protection.
    function hiwaterIsState(key) {
      if (key === ".nanna/write_hiwater.json") return true;
      var tail = "/.nanna/write_hiwater.json";
      return key.length > tail.length && key.lastIndexOf(tail) === key.length - tail.length;
    }
    function hiwaterExempt(key) {
      return hiwaterIsBuffer(key) || hiwaterIsPrev(key) || hiwaterIsState(key);
    }
    function hiwaterLoad() {
      try {
        var raw = Nanna.readFile(HIWATER_STATE);
        if (raw) {
          var parsed = JSON.parse(raw);
          if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) return parsed;
        }
      } catch (e) {
        // Missing or corrupt state: start fresh.
      }
      return {};
    }
    function hiwaterHi(entry) {
      if (entry && typeof entry.hi === "number" && isFinite(entry.hi) && entry.hi > 0) return entry.hi;
      return 0;
    }
    function hiwaterGood(entry) {
      if (entry && typeof entry.good === "number" && isFinite(entry.good) && entry.good > 0) return entry.good;
      return 0;
    }
    function hiwaterSave(map) {
      try {
        var keys = Object.keys(map);
        if (keys.length > HIWATER_MAX_ENTRIES) {
          keys.sort(function(a, b) {
            return ((map[a] && map[a].at) || 0) - ((map[b] && map[b].at) || 0);
          });
          var evict = keys.length - HIWATER_MAX_ENTRIES;
          for (var i = 0; i < evict; i++) delete map[keys[i]];
        }
        Nanna.writeFile(HIWATER_STATE, JSON.stringify(map));
      } catch (e) {
        // State persistence is best-effort.
      }
    }
    // The entry for a path under BOTH its canonical and its legacy spelling,
    // folded into one: hi takes the max (monotone evidence), everything else
    // follows the fresher entry. When the map is later saved through, the
    // alias entry is dropped — the split-brain heals on first touch.
    function hiwaterEntryFor(map, path) {
      var canon = hiwaterKey(path);
      var alias = hiwaterNormKey(path);
      var a = map[canon] || null;
      var b = alias !== canon ? (map[alias] || null) : null;
      if (b) {
        var merged;
        if (!a) {
          merged = b;
        } else {
          merged = ((b.at || 0) > (a.at || 0)) ? b : a;
          var other = merged === a ? b : a;
          var hiM = hiwaterHi(merged);
          var hiO = hiwaterHi(other);
          if (hiO > hiM) merged.hi = hiO;
          if (hiwaterGood(merged) === 0 && hiwaterGood(other) > 0) {
            merged.good = other.good;
            merged.goodAt = other.goodAt || 0;
            if (merged.chk !== "ok" && merged.chk !== "bad") merged.chk = other.chk;
          }
        }
        delete map[alias];
        map[canon] = merged;
        return merged;
      }
      return a;
    }

    // Read-recency marks (P22 Tier 3): when did this session last SEE this
    // file's content? Recorded by read_file, by a successful whole-file
    // write here (at that instant the file holds exactly what the model
    // sent), and by the stale-shrink echo below (the echo IS a read). The
    // mark is compared against the file's mtime: any mutation — edit_file,
    // exec redirect, the user — moves mtime past the mark and invalidates
    // it. All I/O fails OPEN.
    var READMARK_STATE = ".nanna/read_marks.json";
    var READMARK_MAX_ENTRIES = 200;
    function readmarkLoad() {
      try {
        var raw = Nanna.readFile(READMARK_STATE);
        if (raw) {
          var parsed = JSON.parse(raw);
          if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) return parsed;
        }
      } catch (e) {
        // Missing or corrupt state: start fresh.
      }
      return {};
    }
    function readmarkPut(path) {
      try {
        var map = readmarkLoad();
        map[hiwaterKey(path)] = { at: Date.now() };
        var keys = Object.keys(map);
        if (keys.length > READMARK_MAX_ENTRIES) {
          keys.sort(function(a, b) {
            return ((map[a] && map[a].at) || 0) - ((map[b] && map[b].at) || 0);
          });
          var evict = keys.length - READMARK_MAX_ENTRIES;
          for (var i = 0; i < evict; i++) delete map[keys[i]];
        }
        Nanna.writeFile(READMARK_STATE, JSON.stringify(map));
      } catch (e) {
        // Best-effort.
      }
    }
    // True when the file's content has NOT changed since the session last
    // saw it. Unknown → true (fail open). stat.modified is whole seconds
    // and truncates, which biases sub-second races toward "seen" — the open
    // direction.
    function seenSinceLastChange(path) {
      try {
        var st = Nanna.stat(path);
        if (!st || typeof st.modified !== "number" || !isFinite(st.modified)) return true;
        var entry = readmarkLoad()[hiwaterKey(path)];
        var at = entry && typeof entry.at === "number" && isFinite(entry.at) ? entry.at : 0;
        if (at === 0) return false; // never recorded as seen
        return at >= st.modified * 1000;
      } catch (e) {
        return true;
      }
    }

    // Refuse ANY .py content that does not parse — new file or overwrite.
    // Round-6 lesson: gating only valid->invalid transitions let the model
    // create a file BORN broken and then "repair" it with equally broken
    // content forever. Invalid Python on disk is never useful; the error
    // names the line so the write call becomes a fast syntax feedback
    // loop. force=true overrides; ANY checker failure fails OPEN (a
    // missing python interpreter must never block writes). Returns
    // {ran, ok, detail} — `ran` distinguishes a real verdict from a
    // fail-open non-answer, so only genuine passes feed the evidenced-good
    // anchor below.
    function pythonSyntaxCheck(path, nextText) {
      var lower = path.toLowerCase();
      if (lower.length < 3 || lower.lastIndexOf(".py") !== lower.length - 3) return { ran: false, ok: false, detail: "" };
      try {
        var chk = path + ".__chk.py";
        var newTmp = path + ".__chk_new.py";
        Nanna.writeFile(newTmp, nextText);
        Nanna.writeFile(chk,
          "import ast, sys\n" +
          "try:\n" +
          "    ast.parse(open(sys.argv[1], encoding='utf-8').read())\n" +
          "    print('NEW_OK')\n" +
          "except SyntaxError as e:\n" +
          "    print('NEW_BAD line ' + str(e.lineno) + ': ' + str(e.msg))\n");
        var cmd = "python '" + chk + "' '" + newTmp + "'; rc=$?; rm -f '" + chk + "' '" + newTmp + "'; exit $rc";
        var result = Nanna.exec(cmd, null, 30);
        var out = result && result.stdout ? result.stdout : "";
        var bad = out.indexOf("NEW_BAD");
        if (bad !== -1) {
          var detail = out.substring(bad + 8);
          var nl = detail.indexOf("\n");
          if (nl !== -1) detail = detail.substring(0, nl);
          if (detail.length > 160) detail = detail.substring(0, 160);
          return { ran: true, ok: false, detail: detail };
        }
        if (out.indexOf("NEW_OK") !== -1) return { ran: true, ok: true, detail: "" };
        return { ran: false, ok: false, detail: "" };
      } catch (e) {
        return { ran: false, ok: false, detail: "" };
      }
    }

    // Post-mutation structural check (P22 Tier 3): after any write, the
    // cheapest applicable check runs on what actually landed and its verdict
    // is appended to the result AS A SENTENCE — never a gate. Evidence
    // (gemma, 2026-08-10): a partial-overlap edit left a dangling `else`;
    // the broken file exits 2, which is exactly what the first test asserts,
    // so the surviving failure said "should print usage" and the model spent
    // 31 minutes editing help text on a file that had not parsed for any of
    // them. The truth was one `sh -n` away. A pass also records the size as
    // the ratchet's evidenced-good anchor. Every failure path (no checker,
    // checker missing, timeout) yields NO verdict at all — fail OPEN.
    function structuralCheckKind(path, contentText) {
      var p = path.split("\\").join("/").toLowerCase();
      var base = p.split("/").pop();
      function hasExt(e) { return base.length > e.length && base.lastIndexOf(e) === base.length - e.length; }
      if (hasExt(".json") || hasExt(".geojson")) return "json";
      // .bash (and bash shebangs below) route to bash -n: on hosts where
      // /bin/sh is dash, valid bash ([[ ]], arrays, process substitution)
      // fails sh -n and the verdict would cry wolf on a correct file
      // (ultrareview on PR #224).
      if (hasExt(".bash")) return "bash";
      if (hasExt(".sh")) return "sh";
      if (hasExt(".js") || hasExt(".mjs") || hasExt(".cjs")) return "node";
      if (hasExt(".py")) return "py";
      // Extensionless artifacts (./minidb) are the common mission shape:
      // classify by shebang instead of the name. "sh" must not match fish/
      // pwsh/zsh/csh — those are not POSIX sh and sh -n would cry wolf.
      if (base.indexOf(".") === -1 && typeof contentText === "string" && contentText.indexOf("#!") === 0) {
        var nl = contentText.indexOf("\n");
        var line1 = nl === -1 ? contentText : contentText.substring(0, nl);
        if (line1.indexOf("python") !== -1) return "py";
        if (line1.indexOf("node") !== -1) return "node";
        if (line1.indexOf("bash") !== -1) return "bash";
        if (line1.indexOf("fish") === -1 && line1.indexOf("pwsh") === -1 &&
            line1.indexOf("zsh") === -1 && line1.indexOf("csh") === -1 &&
            line1.indexOf("sh") !== -1) return "sh";
      }
      return null;
    }
    // Runs the check. Returns {ok, tool, detail} or null for no verdict.
    // 15s cap: these checks parse without executing, so they are near-
    // instant; the cap only bounds interpreter cold start and must stay far
    // under exec's 180s engine deadline so a check can never orphan a child.
    function runStructuralCheck(kind, path, contentText) {
      if (kind === "json") {
        if (typeof contentText !== "string") return null;
        try {
          JSON.parse(contentText);
          return { ok: true, tool: "JSON.parse", detail: "" };
        } catch (eJ) {
          var jd = String(eJ && eJ.message ? eJ.message : eJ);
          if (jd.length > 160) jd = jd.substring(0, 160);
          return { ok: false, tool: "JSON.parse", detail: jd };
        }
      }
      if (path.indexOf("'") !== -1) return null; // unquotable — no verdict
      var cmd = null;
      var toolName = null;
      if (kind === "sh") { cmd = "sh -n '" + path + "'"; toolName = "sh -n"; }
      else if (kind === "bash") { cmd = "bash -n '" + path + "'"; toolName = "bash -n"; }
      else if (kind === "node") { cmd = "node --check '" + path + "'"; toolName = "node --check"; }
      else if (kind === "py") { cmd = "python -c 'import ast,sys; ast.parse(open(sys.argv[1], encoding=\"utf-8\").read())' '" + path + "'"; toolName = "python ast"; }
      if (!cmd) return null;
      try {
        var r = Nanna.exec(cmd, null, 15);
        if (!r) return null;
        if (r.success) return { ok: true, tool: toolName, detail: "" };
        var err = (r.stderr || r.stdout || "");
        if (r.code === 127 || err.indexOf("command not found") !== -1) return null; // checker absent
        err = err.split("\r").join("").split("\n").join(" ");
        while (err.indexOf("  ") !== -1) err = err.split("  ").join(" ");
        if (err.length > 200) err = err.substring(0, 200);
        if (err === "" || err === " ") err = "exit code " + r.code;
        return { ok: false, tool: toolName, detail: err };
      } catch (eX) {
        return null;
      }
    }
    function structSentence(path, verdict, prevChk) {
      if (!verdict) return "";
      if (verdict.ok) {
        if (prevChk === "bad") return " STRUCTURE: " + path + " parses again (" + verdict.tool + ") — the earlier syntax break is fixed.";
        return " STRUCTURE: the file parses (" + verdict.tool + ").";
      }
      var history = "";
      if (prevChk === "ok") history = " It parsed BEFORE this write — this write introduced the break.";
      else if (prevChk === "bad") history = " It did not parse before this write either.";
      return " STRUCTURE: " + path + " does NOT parse (" + verdict.tool + "): " + verdict.detail + "." + history +
        " This is information, not a block — the file holds exactly what you sent. Fix that line with edit_file.";
    }

    // Accept multiple parameter name variants from different models
    var filePath = input.file_path || input.filePath || input.path || input.file || input.filename;
    // Undefined-chain, NOT || — an explicit content:"" is a legitimate
    // empty-file write (e.g. a package __init__.py) and must not be
    // misreported as "missing content" (verify-round finding).
    var fileContent = input.content;
    if (fileContent === undefined || fileContent === null) fileContent = input.text;
    if (fileContent === undefined || fileContent === null) fileContent = input.data;
    if (fileContent === undefined || fileContent === null) fileContent = input.new_content;
    if (fileContent === undefined || fileContent === null) fileContent = input.file_content;
    if (!filePath && (fileContent === undefined || fileContent === null)) {
      return fail("write_file failed: you must pass BOTH file_path AND content. Call it again like: write_file(file_path=\"D:/path/to/file.py\", content=\"<the complete file text>\")");
    }
    if (!filePath) {
      return fail("write_file failed: missing file_path. Nothing was written. Call it again with BOTH file_path (the destination path) AND content.");
    }
    if (fileContent === undefined || fileContent === null) {
      return fail("write_file failed: missing content. Nothing was written. Call write_file again with file_path=\"" + filePath + "\" AND content set to the COMPLETE file text.");
    }
    if (typeof filePath !== "string") filePath = String(filePath);
    if (typeof fileContent !== "string") fileContent = String(fileContent);

    // Writing the ratchet's own bookkeeping is always confusion — and wiping
    // it would silently disarm the erosion guard. Calm, self-describing
    // refusal so the model moves on instead of "repairing" internals.
    if (!input.force && hiwaterIsState(hiwaterKey(filePath))) {
      return fail("write_file skipped: " + filePath + " is write_file's internal bookkeeping. It maintains itself, is healthy, and never needs manual repair. Your own files are unaffected. Continue with your actual task.");
    }

    // Double-escaped content — REPAIRED, not refused. Observed live
    // (lfm2.5): the model emitted "#!/bin/sh\ncase $1 in" with the
    // backslash-n as TWO LITERAL CHARACTERS, so the "script" landed as one
    // physical line and could never execute; every downstream acceptance
    // check then failed on behaviour, hiding the real cause.
    //
    // Refusing was tried first and is WORSE: the model cannot see its own
    // escaping (the defect is in how its output is serialized, not in its
    // intent), so it resent byte-identical content three times, then began
    // SHRINKING the file to appease the guard — heading straight for the
    // truncation ratchet. A guard the model cannot satisfy is a wedge.
    //
    // The signature is unambiguous and needs no size threshold: content
    // carrying two or more literal \n sequences describes three or more
    // lines, so a complete absence of real newlines means the escaping was
    // lost in transit, not that the author wanted one line. JSON-family
    // files are exempt — \n inside a string is exactly how JSON encodes a
    // newline, by spec. force=true writes bytes through untouched.
    function escapedNewlineCount(path, text) {
      if (text.indexOf("\n") !== -1) return 0; // real newlines present — fine
      var lower = path.toLowerCase();
      if (/\.(json|jsonl|ndjson|geojson)$/.test(lower)) return 0;
      return text.split("\\n").length - 1;
    }
    var unescapedNewlines = 0;
    if (!input.force) {
      unescapedNewlines = escapedNewlineCount(filePath, fileContent);
      if (unescapedNewlines >= 2) {
        fileContent = fileContent.split("\\n").join("\n");
      } else {
        unescapedNewlines = 0;
      }
    }

    var bytes = fileContent.length;

    // Safety checks BEFORE writing. Three invariants from the adversarial
    // verify round still hold: a write that does not shrink the CURRENT
    // disk file can never erode it, so it is never refused; a file that no
    // longer exists has nothing left to protect — creation is always allowed
    // and re-arms the ratchet from the new size; and every state failure
    // degrades to the old behavior, never to a block.
    var existingSize = 0;
    var fileExists = false;
    var existsUnknown = false;
    var existing;
    if (!input.force) {
      try {
        existing = Nanna.readFile(filePath);
        if (existing !== undefined && existing !== null) {
          fileExists = true;
          existingSize = existing.length;
        }
      } catch (e) {
        // "os error 2"/"os error 3" = the file genuinely doesn't exist. Any
        // OTHER read failure (sharing violation, non-UTF-8) means the file is
        // probably THERE but unreadable: fail open on the guard, but flag it
        // so the ratchet state is left untouched (verify finding: a transient
        // lock must not reset the mark to the new small size).
        // Parenthesized form ONLY: the bridge embeds io::Error display as
        // "...(os error N)", and a bare "os error 3" substring also matches
        // "(os error 32)" — the sharing-violation case this flag exists for.
        var readErr = String(e);
        if (readErr.indexOf("(os error 2)") === -1 && readErr.indexOf("(os error 3)") === -1) {
          existsUnknown = true;
        }
      }

      var hwKeyGuard = hiwaterKey(filePath);

      // P22 Tier 3: you cannot shrink what you have not seen. A shrinking
      // whole-file write over a file whose content CHANGED since the session
      // last read it is a rewrite from a stale context copy — the commonest
      // way an assistant destroys a file: it saw the file twenty messages
      // ago, the history got compacted, and it confidently rewrites from
      // memory (2026-08-10 qwen: a fresh step wrote 2449 bytes over a
      // 5598-byte artifact with 22 checks passing, zero reads in the step;
      // the score fell to 1/42). Not a refusal: the reply carries the file's
      // CURRENT content to merge against, counts itself as the read, and the
      // next attempt proceeds. One bounce, with the missing information.
      if (fileExists && !hiwaterExempt(hwKeyGuard) && bytes < existingSize &&
          typeof existing === "string" && !seenSinceLastChange(filePath)) {
        // Echo bound (ultrareview on PR #224): the echo is MERGE MATERIAL,
        // and merge material the model cannot hold is not material — 64 KiB
        // is a small local model's entire 16k-token window, so anything
        // bigger only burns the context this guard exists to respect. Under
        // the bound the full content ships and counts as the read; over it,
        // a head ships with a loud truncation notice (WHAT dropped, WHY,
        // disk unaffected), the mark is NOT recorded, and the next attempt
        // still bounces — into an explicit ranged read_file, never into a
        // merge against a partial view.
        var ECHO_MAX = 65536;
        if (existingSize <= ECHO_MAX) {
          readmarkPut(filePath);
          glog("write_file guard: stale-shrink echo for " + filePath + " (attempted " + bytes + " over " + existingSize + " bytes; file changed since last recorded read)");
          return fail(
            "WRITE HELD — nothing was written and nothing is lost. You are shrinking " + filePath +
            " from " + existingSize + " to " + bytes + " bytes, but the file has CHANGED since you last read it — " +
            "your context copy is stale, so this rewrite would silently destroy parts of the current file. " +
            "Here is the CURRENT content of " + filePath + ":\n\n" + existing +
            "\n\nMerge your change INTO this current text and call write_file again with the full merged content " +
            "(or use edit_file for a targeted change). This reply counts as your read — the file will not be held for this reason again."
          );
        }
        glog("write_file guard: stale-shrink hold (truncated echo) for " + filePath + " (attempted " + bytes + " over " + existingSize + " bytes)");
        return fail(
          "WRITE HELD — nothing was written and nothing is lost. You are shrinking " + filePath +
          " from " + existingSize + " to " + bytes + " bytes, but the file has CHANGED since you last read it — " +
          "your context copy is stale, so this rewrite would silently destroy parts of the current file. " +
          "The file is too large (" + existingSize + " bytes) to echo here; its first lines are:\n\n" + existing.substring(0, 4096) +
          "\n\n[TRUNCATED — only the first 4096 of " + existingSize + " bytes shown; the file on disk is complete and unaffected.] " +
          "This truncated preview does NOT count as reading the file. Call read_file(\"" + filePath + "\") " +
          "(with offset/limit for ranges) to see the current content, then merge your change into it — " +
          "after a real read this write will be accepted, or use edit_file for a targeted change."
        );
      }

      var hwBase = existingSize;
      var hwGoodBase = 0;
      if (fileExists && !hiwaterExempt(hwKeyGuard)) {
        var hwEntry = hiwaterEntryFor(hiwaterLoad(), filePath);
        var hwHi = hiwaterHi(hwEntry);
        // Monotone while the file exists: an out-of-band change since our
        // last write is folded in as evidence, never a reason to forget the
        // mass this file has held (the re-base rule was a laundering hole —
        // see the ratchet design comment above).
        if (hwHi > hwBase) {
          hwBase = hwHi;
        }
        hwGoodBase = hiwaterGood(hwEntry);
      }

      // Shrink floor, anchored on the last evidenced-good size when one
      // exists (see the ratchet design comment above).
      var floorBase = hwGoodBase > 0 ? hwGoodBase : hwBase;
      var floorAnchor = hwGoodBase > 0 ? "good" : "hi";
      if (!hiwaterExempt(hwKeyGuard) && floorBase > 500 && bytes < existingSize && bytes < floorBase * 0.3) {
        var sizeStory;
        if (floorAnchor === "good") {
          sizeStory = "holds " + existingSize + " bytes now, and its last version that passed a structural check held " + floorBase + " bytes";
        } else if (hwBase > existingSize) {
          sizeStory = "holds " + existingSize + " bytes now and has held " + hwBase + " bytes before";
        } else {
          sizeStory = "currently holds " + existingSize + " bytes";
        }
        glog("write_file guard: WRITE REFUSED (shrink floor) " + filePath + " attempted=" + bytes + " existing=" + existingSize + " floorBase=" + floorBase + " anchor=" + floorAnchor);
        return {
          content: "WRITE REFUSED — the file was NOT modified and is fully intact. " +
            "You tried to write only " + bytes + " bytes over " + filePath +
            " which " + sizeStory + " (" +
            Math.round(bytes / floorBase * 100) + "% of that). That usually means " +
            "you sent a fragment instead of the whole file. For a small change, use " +
            "edit_file instead: edit_file(file_path=\"" + filePath + "\", old_string=<the exact current text>, " +
            "new_string=<your replacement>) — it changes just that snippet and leaves the rest untouched. " +
            "To remove a section, edit_file with new_string=\"\". " +
            "Only if you truly mean to replace the WHOLE file: (1) read_file " + filePath + ", " +
            "(2) merge your change into the FULL text, (3) call write_file again with the complete content.",
          success: false
        };
      }
    }

    var pyGate = { ran: false, ok: false, detail: "" };
    if (!input.force) {
      // Versioned-copy REFUSAL (observed live: models fork foo.py.new2,
      // new_foo.py, foo_fixed_v1.txt instead of fixing the real file, then
      // lose track of which copy is real — an advisory did not stop it).
      var baseName = filePath.split("\\").join("/").split("/").pop().toLowerCase();
      var copyMarkers = [".new", "_v1", "_v2", "_v3", "_v4", "_v5", "_fixed", "_backup", "_temp", "_copy", "_part", "_old", "_final", "_clean", "_scrubbed", "scratch"];
      var copyPrefixes = ["new_", "copy_", "old_", "temp_", "backup_"];
      var copyHit = null;
      for (var m = 0; m < copyMarkers.length; m++) {
        if (baseName.indexOf(copyMarkers[m]) !== -1) { copyHit = copyMarkers[m]; break; }
      }
      if (!copyHit) {
        for (var p = 0; p < copyPrefixes.length; p++) {
          if (baseName.indexOf(copyPrefixes[p]) === 0) { copyHit = copyPrefixes[p]; break; }
        }
      }
      if (copyHit) {
        glog("write_file guard: WRITE REFUSED (versioned copy '" + copyHit + "') " + filePath);
        return fail("WRITE REFUSED — '" + filePath + "' looks like a versioned copy ('" + copyHit + "'). Nothing was written. Keep ONE real file: change the ORIGINAL in place with edit_file, or write the full corrected content directly to the original path (a complete valid rewrite at or above the file's current size is always accepted).");
      }

      // SUFFIXED-COPY REFUSAL: writing `minidb.sh` while `minidb` exists.
      //
      // The marker list above catches renames that ANNOUNCE themselves
      // (_v2, _backup, .new). This catches the quieter fork: keep the name,
      // add an extension. Observed live 2026-07-27 — a run built ./minidb to
      // 8/42 passing, then drifted onto ./minidb.sh and spent its remaining
      // time improving a file the acceptance tests never read.
      //
      // Deliberately narrow: it fires only when the target's STEM is itself
      // an existing file, i.e. the original carries no extension of its own.
      // That is the copy instinct. Sibling formats keep working, because
      // their stems are not files: config.json next to config.yaml, tool.ts
      // next to tool.js, index.css next to index.html.
      var lastDot = baseName.lastIndexOf(".");
      if (lastDot > 0) {
        var stem = baseName.substring(0, lastDot);
        var dirPart = filePath.split("\\").join("/");
        var slashAt = dirPart.lastIndexOf("/");
        var stemPath = slashAt >= 0 ? dirPart.substring(0, slashAt + 1) + stem : stem;
        // Nanna.stat THROWS when the path is absent, and its result carries
        // {size, is_file, is_dir, modified} — there is no `exists` flag. The
        // throw IS the existence check.
        var originalStat = null;
        try { originalStat = Nanna.stat(stemPath); } catch (e) { originalStat = null; }
        if (originalStat && originalStat.is_file) {
          return failEscalating(
            hiwaterKey(filePath),
            "WRITE REFUSED — '" + filePath + "' is '" + stem + "' with an extension added, and '" +
            stem + "' already exists (" + (originalStat.size || 0) + " bytes). Nothing was written. " +
            "That fork leaves two files and the real one stops improving — whatever reads '" + stem +
            "' (tests, callers) will not see this content. Work on '" + stemPath + "' itself: " +
            "edit_file(file_path=\"" + stemPath + "\", old_string=<exact current text>, new_string=<replacement>) " +
            "for a targeted change, or write the complete content to '" + stemPath + "'.",
            "STOP — '" + filePath + "' WILL NEVER BE ACCEPTED. You have tried it repeatedly. " +
            "The only file that counts is '" + stemPath + "'. Send this exact call instead:\n" +
            "    write_file(file_path=\"" + stemPath + "\", content=<the complete script>)\n" +
            "Do not add an extension. Do not try '" + filePath + "' again."
          );
        }
      }

      // VALID CONTENT ALWAYS WINS (round-13 lesson): the earlier rail
      // blocked whole-file writes whenever a parked draft existed — even
      // when the new content was perfectly valid, bouncing the model's one
      // reliable move (fresh generation) and wedging it between an empty
      // real file and a draft it would not repair. Order is now: check
      // validity FIRST; a parsing .py write is accepted outright and any
      // stale draft/markers are swept; only INVALID content meets the
      // park/rail machinery.
      pyGate = pythonSyntaxCheck(filePath, fileContent);
      var syntaxDetail = pyGate.ran && !pyGate.ok ? pyGate.detail : null;
      if (syntaxDetail === null) {
        var sweepBufPath = filePath + ".__buffer__";
        try {
          Nanna.exec("rm -f '" + sweepBufPath + "' '" + filePath + ".__cleared__' '" + sweepBufPath + ".__cleared__'", null, 15);
        } catch (eSweep) {
          // Stale draft leftovers are harmless.
        }
      } else {
        // Existing parked draft + ANOTHER invalid regeneration: keep the
        // parked draft authoritative and steer to the repair loop.
        var railBufPath = filePath + ".__buffer__";
        var railParked = null;
        try {
          railParked = Nanna.readFile(railBufPath);
        } catch (eRail) {
          // No parked draft.
        }
        if (railParked !== null && railParked !== undefined && railParked !== "") {
          glog("write_file guard: WRITE BLOCKED (invalid .py over parked draft) " + filePath);
          return fail("WRITE BLOCKED — this content has a SYNTAX ERROR (" + syntaxDetail + ") and a parked draft for " + filePath + " already exists at " + railBufPath + " (" + railParked.length + " chars). Repair THAT draft: edit_file(file_path=\"" + railBufPath + "\", old_string=<the broken line>, new_string=<the fix>), then file_buffer(action=\"commit\", file_path=\"" + filePath + "\"). A fully VALID rewrite of " + filePath + " at or above its current size would also be accepted.");
        }
      }

      // Draft PARKING (round-8 lesson): refusing a broken whole-file write
      // outright sends the model into a regeneration lottery — each retry
      // regenerates everything and rolls new errors (observed live: 21
      // refusals, every one a different line). Instead the rejected draft
      // is SAVED to the buffer beside the target, where the model repairs
      // the one named error with a small edit_file delta and commits.
      if (syntaxDetail) {
        var parkPath = filePath + ".__buffer__";
        var parked = false;
        try {
          Nanna.writeFile(parkPath, fileContent);
          parked = true;
        } catch (ePark) {
          // Fall through to the plain refusal below.
        }
        if (parked) {
          // Quote the offending line verbatim: it is a ready-made
          // old_string, removing the last excuse to regenerate (observed
          // live: three parks in a row, each a fresh full regeneration
          // with a brand-new error line).
          var lineQuote = "";
          if (syntaxDetail.indexOf("line ") === 0) {
            var colonAt = syntaxDetail.indexOf(":");
            if (colonAt > 5) {
              var lineNo = parseInt(syntaxDetail.substring(5, colonAt), 10);
              if (lineNo >= 1) {
                var draftLines = fileContent.split("\n");
                if (lineNo <= draftLines.length) {
                  var lq = draftLines[lineNo - 1];
                  if (lq.length > 80) lq = lq.substring(0, 80);
                  lineQuote = " Line " + lineNo + " of your draft is: `" + lq + "` — use exactly that as old_string.";
                }
              }
            }
          }
          glog("write_file guard: WRITE PARKED (invalid .py) " + filePath + " draft=" + bytes + " bytes");
          return fail("WRITE PARKED — your content for " + filePath + " has a SYNTAX ERROR (" + syntaxDetail + "), so the file was NOT changed. Nothing was lost: the draft IS SAVED at " + parkPath + "." + lineQuote + " Your NEXT call must be edit_file(file_path=\"" + parkPath + "\", old_string=<the broken line>, new_string=<the fixed line>), then file_buffer(action=\"commit\", file_path=\"" + filePath + "\"). Do NOT call write_file again for this file and do NOT regenerate it.");
        }
        return fail("WRITE REFUSED — the content you sent for " + filePath + " is NOT valid Python (" + syntaxDetail + "). The file is UNCHANGED. Fix the syntax and call write_file again with the corrected COMPLETE text.");
      }
    }

    // P22 Tier 3: displaced content stays recoverable. A full overwrite
    // parks the outgoing version at <file>.__prev__ (one slot, overwritten
    // each time) so a destructive rewrite is one read_file away from
    // recovery instead of gone — three bench legs ended with their peak
    // artifact overwritten and nothing to restore. Best-effort, never
    // blocks the write. Skipped under force (the pre-image was never read;
    // force is an explicit re-shape) and for managed sidecars.
    var prevPath = filePath + ".__prev__";
    var prevParked = false;
    if (fileExists && !input.force && typeof existing === "string" &&
        existing !== fileContent && !hiwaterExempt(hiwaterKey(filePath))) {
      try {
        Nanna.writeFile(prevPath, existing);
        prevParked = true;
      } catch (ePrev) {
        // Recovery copy is best-effort.
      }
    }

    try {
      Nanna.writeFile(filePath, fileContent);
    } catch (e2) {
      var writeErr = String(e2);
      if (writeErr.length > 120) writeErr = writeErr.substring(0, 120) + "...";
      return fail("write_file failed writing " + filePath + " (" + writeErr + "). Retry the same call; if it fails again, read_file to verify the file state.");
    }

    // Structural verdict on what landed. The .py gate above already ran the
    // real check — adopt its answer instead of spawning the interpreter a
    // second time; under force (gate skipped) the generic check runs.
    var hwKeyPost = hiwaterKey(filePath);
    var verdict = null;
    var prevChk = null;
    if (!hiwaterExempt(hwKeyPost)) {
      try {
        var peek = hiwaterEntryFor(hiwaterLoad(), filePath);
        if (peek && (peek.chk === "ok" || peek.chk === "bad")) prevChk = peek.chk;
      } catch (ePeek) {
        // No history — the sentence just has no before/after clause.
      }
      var checkKind = structuralCheckKind(filePath, fileContent);
      if (checkKind === "py" && pyGate.ran) {
        verdict = { ok: pyGate.ok, tool: "python ast", detail: pyGate.detail };
      } else if (checkKind) {
        verdict = runStructuralCheck(checkKind, filePath, fileContent);
      }
    }
    if (input.force || !fileExists) prevChk = null;

    // Ratchet update AFTER a successful write. While the file exists, the
    // high-water mark only ever RISES — max of the previous mark, the disk
    // size we just observed, and this write — so fluctuating rewrites stay
    // pinned to the peak and out-of-band changes fold in as evidence
    // instead of restarting the history (the old re-base-on-mismatch rule
    // was the laundering hole documented in the ratchet design comment).
    // Force and creation-over-missing RESET it — both are deliberate
    // re-shapes with nothing stale worth protecting, and that includes the
    // evidenced-good anchor. A passing structural check records this
    // write's size as the new good anchor. When the pre-write read failed
    // for unknown reasons, the state is left completely alone.
    if (!existsUnknown) {
      try {
        var hwMap = hiwaterLoad();
        var hwKey = hiwaterKey(filePath);
        if (!hiwaterExempt(hwKey)) {
          var hwPrevEntry = hiwaterEntryFor(hwMap, filePath);
          var hwNext = bytes > existingSize ? bytes : existingSize;
          if (!input.force && fileExists) {
            var hwPrevHi = hiwaterHi(hwPrevEntry);
            if (hwPrevHi > hwNext) hwNext = hwPrevHi;
          }
          if (input.force || !fileExists) hwNext = bytes;
          var hwNew = { hi: hwNext, last: bytes, at: Date.now() };
          if (!input.force && fileExists && hwPrevEntry) {
            if (hiwaterGood(hwPrevEntry) > 0) {
              hwNew.good = hwPrevEntry.good;
              hwNew.goodAt = hwPrevEntry.goodAt || 0;
            }
            if (hwPrevEntry.chk === "ok" || hwPrevEntry.chk === "bad") hwNew.chk = hwPrevEntry.chk;
          }
          if (verdict) {
            hwNew.chk = verdict.ok ? "ok" : "bad";
            if (verdict.ok) {
              hwNew.good = bytes;
              hwNew.goodAt = Date.now();
            }
          }
          hwMap[hwKey] = hwNew;
          hiwaterSave(hwMap);
        }
      } catch (eHw) {
        // Best-effort; the user's write already succeeded.
      }
      // The file now holds exactly what the model sent — its knowledge of
      // the content is perfect at this instant, so the write counts as a
      // read for the stale-shrink guard.
      if (!hiwaterExempt(hwKeyPost)) readmarkPut(filePath);
    }

    var structNote = structSentence(filePath, verdict, prevChk);
    if (verdict && !verdict.ok) {
      glog("write_file structure: " + filePath + " does NOT parse after write (" + verdict.tool + "): " + verdict.detail);
    } else if (verdict && verdict.ok && prevChk === "bad") {
      glog("write_file structure: " + filePath + " parses again after write (" + verdict.tool + ")");
    }

    // Rewrite-delta announcement (P22 Tier 3, bidirectional). The original
    // 2026-08-08 ornith lesson: full-file rewrites from a stale in-context
    // copy silently deleted working sections all run long — every write
    // reported plain success while the artifact kept 8 of 37 verified
    // commands. The 2026-08-10 follow-up showed the symmetric hole: BOTH
    // destructive writes that leg GREW the file (3333→6986 bytes, a
    // superset of function names with rewritten bodies) and the removal-
    // only note was structurally blind to them. The note now announces
    // (a) sections REMOVED, (b) pre-existing sections whose BODIES changed
    // (cheap per-symbol content hash), and (c) a rewrite that more than
    // doubled the file — i.e. added more new mass than the whole file
    // previously held. Refusing is still wrong (a complete rewrite is the
    // model's one reliable move); the note informs, is logged at INFO, and
    // names the .__prev__ recovery copy. Detection is a single whole-string
    // regex pass per side (no per-line split — the Boa split cost lesson):
    // shell functions `name() {`, case arms `name)`, and def/class/function
    // declarations at line starts. Informational only; never blocks.
    function topSymbolSpans(text) {
      var out = [];
      var re = /^[ \t]*(?:([A-Za-z_][A-Za-z0-9_]*)[ \t]*\(\)[ \t]*\{?[ \t]*$|(?:def|class|function)[ \t]+([A-Za-z_$][A-Za-z0-9_$]*)|([A-Za-z_][A-Za-z0-9_-]*)\)[ \t]*$)/gm;
      var m;
      while ((m = re.exec(text)) !== null) {
        var n = m[1] || m[2] || m[3];
        if (n) out.push({ name: n, at: m.index });
        if (m.index === re.lastIndex) re.lastIndex++;
      }
      return out;
    }
    function collapseWs(s) {
      var out = "";
      var pend = false;
      for (var ci = 0; ci < s.length; ci++) {
        var c = s.charAt(ci);
        if (c === " " || c === "\t" || c === "\r" || c === "\n") { pend = true; continue; }
        if (pend && out !== "") out += " ";
        pend = false;
        out += c;
      }
      return out;
    }
    function hashStr(s) {
      var h = 5381;
      for (var hi2 = 0; hi2 < s.length; hi2++) h = (((h << 5) + h) ^ s.charCodeAt(hi2)) >>> 0;
      return h;
    }
    // Where a symbol's body ends. The span is bounded by the next
    // declaration, then trimmed back past trailing TOP-LEVEL code — a bare
    // `usage` call after the closing brace must not leak into the hash, or
    // appending anything after the LAST function marks that function
    // "changed": a false positive that teaches the model to ignore the
    // note (caught by the P22 harness). A brace declaration runs through
    // its first column-0 closer regardless of body indentation (small
    // models write unindented shell); anything else (def/class, case arms)
    // follows the indentation rule.
    function symbolBodyEnd(text, declStart, limit) {
      var nl = text.indexOf("\n", declStart);
      var declEnd = (nl === -1 || nl > limit) ? limit : nl;
      var braced = text.substring(declStart, declEnd).indexOf("{") !== -1;
      var end = declEnd;
      var lineStart = declEnd + 1;
      while (lineStart < limit) {
        nl = text.indexOf("\n", lineStart);
        var lineEnd = (nl === -1 || nl > limit) ? limit : nl;
        var raw = text.substring(lineStart, lineEnd);
        var ti = 0;
        while (ti < raw.length && (raw.charAt(ti) === " " || raw.charAt(ti) === "\t")) ti++;
        var trimmed = raw.substring(ti);
        if (trimmed.charAt(trimmed.length - 1) === "\r") trimmed = trimmed.substring(0, trimmed.length - 1);
        var isCloser = trimmed === "}" || trimmed === "};" || trimmed === "fi" ||
          trimmed === "esac" || trimmed === "done" || trimmed === "end" || trimmed === ";;";
        if (braced) {
          end = lineEnd;
          if (ti === 0 && isCloser) break;
        } else {
          if (trimmed === "" || ti > 0 || isCloser) { end = lineEnd; }
          else break;
        }
        if (nl === -1 || nl >= limit) break;
        lineStart = nl + 1;
      }
      return end;
    }
    // Each symbol's body hash (whitespace-insensitive). First occurrence of
    // a duplicated name wins.
    function symbolBodies(text) {
      var spans = topSymbolSpans(text);
      var map = {};
      for (var si = 0; si < spans.length; si++) {
        if (map[spans[si].name] !== undefined) continue;
        var limit = si + 1 < spans.length ? spans[si + 1].at : text.length;
        var end = symbolBodyEnd(text, spans[si].at, limit);
        map[spans[si].name] = hashStr(collapseWs(text.substring(spans[si].at, end)));
      }
      return map;
    }
    function nameList(arr) {
      var shown = arr.slice(0, 10);
      var more = arr.length - shown.length;
      return "`" + shown.join("`, `") + "`" + (more > 0 ? " (+" + more + " more)" : "");
    }
    var lossNote = "";
    if (fileExists && !input.force && typeof existing === "string" && existing !== fileContent) {
      try {
        var oldBodies = symbolBodies(existing);
        var newBodies = symbolBodies(fileContent);
        var dropped = [];
        var changed = [];
        for (var sName in oldBodies) {
          if (newBodies[sName] === undefined) dropped.push(sName);
          else if (newBodies[sName] !== oldBodies[sName]) changed.push(sName);
        }
        // "More than doubled" = this rewrite added more new mass than the
        // entire previous file held — a from-scratch superset, the shape of
        // both destructive ornith writes. The 500-byte gate matches the
        // ratchet's own smallness line.
        var grewPastDouble = existingSize > 500 && bytes > existingSize * 2;
        if (dropped.length > 0 || changed.length > 0 || grewPastDouble) {
          var parts = "";
          if (dropped.length > 0) {
            parts += " It REMOVED sections that existed on disk before: " + nameList(dropped) + ".";
          }
          if (changed.length > 0) {
            parts += " It CHANGED the bodies of pre-existing sections: " + nameList(changed) + ".";
          }
          if (grewPastDouble) {
            parts += dropped.length === 0 && changed.length === 0
              ? " It more than doubled the file while keeping every pre-existing section's body intact — the growth is purely additive."
              : " It also more than doubled the file — most of what is on disk now is new text.";
          }
          lossNote = " NOTE: this whole-file rewrite replaced " + filePath + " (" + existingSize + " → " + bytes + " bytes)." + parts +
            " The write SUCCEEDED — the file holds exactly what you sent. If any of those sections were verified working, re-verify them now" +
            (prevParked ? "; the previous version is preserved at " + prevPath + " (read_file it to recover anything)" : "") + ".";
          glog("write_file rewrite-note for " + filePath + " (" + existingSize + "->" + bytes + " bytes): removed=[" + dropped.join(",") + "] changed=[" + changed.join(",") + "]" + (grewPastDouble ? " grew>2x" : ""));
        }
      } catch (eLoss) {
        // Informational only — never affects the write.
      }
    }

    // Deliberately NO echo of the written content: echoing the whole file
    // made the result exceed the context threshold, and the model read the
    // resulting truncation stub as "my write was discarded" (observed
    // live, round 7). The file on disk is the source of truth.
    // A repair the model cannot see is a repair it will "fix" back. Announce
    // WHAT changed, WHY, and that the write SUCCEEDED — an unexplained
    // difference between what was sent and what landed reads as corruption.
    var repairNote = "";
    if (unescapedNewlines > 0) {
      repairNote =
        " NOTE: your content arrived with " + unescapedNewlines +
        " literal backslash-n sequences and no real line breaks, which would have made the file one unusable line;" +
        " they were converted to real newlines before writing. The write SUCCEEDED and the file is correct —" +
        " nothing to redo. Send real line breaks next time and this note goes away.";
    }
    return { content: "Wrote " + bytes + " bytes to " + filePath + ". The file on disk now holds exactly this content." + structNote + repairNote + lossNote };
  }
}
