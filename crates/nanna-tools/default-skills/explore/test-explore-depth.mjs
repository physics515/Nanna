// Standalone regression test for the explore tool's bounded, level-by-level
// walk. Run with: node test-explore-depth.mjs
//
// History: v0.1.x opened with Nanna.listDir(path, true) — a RECURSIVE walk of
// the whole workspace marshalled into Boa before any depth filtering. On a
// real monorepo (node_modules/.git/target = hundreds of thousands of entries)
// every call hit the 30s Boa deadline and returned success=false, which the
// model retry-looped (observed live 2026-08-02). v0.2.0 walks level-by-level
// with NON-recursive bounded listDir calls, skips noise dirs, caps visited
// entries at 1000 (derived from the ~2KB context output budget), and returns
// structured {content, success:false} on inaccessible roots. These tests stub
// Nanna.listDir with a per-directory map and assert that contract.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, "tool.ts"), "utf8");

function loadTool(listDirImpl) {
  const module = { exports: null };
  const body = source.replace("export default", "module.exports =");
  new Function("module", "Nanna", body)(module, { listDir: listDirImpl });
  return module.exports;
}

// Stub a filesystem: tree maps directory path -> entry array, keyed exactly
// as the tool constructs paths (root as given, children as parent + "/" +
// name). Records every call so tests can assert the walk's shape.
function makeStub(tree) {
  const calls = [];
  const listDir = (path, recursive, max) => {
    calls.push({ path, recursive, max });
    if (recursive) {
      throw new Error("explore must never request a recursive walk");
    }
    if (typeof max !== "number" || max < 1) {
      throw new Error("explore must always pass a positive entry bound, got: " + max);
    }
    if (!(path in tree)) {
      throw new Error("Failed to read directory: " + path);
    }
    return tree[path].slice(0, max);
  };
  return { listDir, calls };
}

function run(tree, input) {
  const stub = makeStub(tree);
  const tool = loadTool(stub.listDir);
  const result = tool.execute(input || {});
  return { result, calls: stub.calls };
}

function summary(content) {
  const m = content.match(/(\d+) directories, (\d+) files, (\S+) total/);
  if (!m) throw new Error("no summary line in output:\n" + content);
  return { dirs: Number(m[1]), files: Number(m[2]), size: m[3] };
}

let failures = 0;
function check(name, actual, expected) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a === e) {
    console.log("PASS " + name);
  } else {
    console.error("FAIL " + name + "\n  expected " + e + "\n  actual   " + a);
    failures++;
  }
}

const d = (name) => ({ name, entry_type: "dir", size: 0 });
const f = (name, size) => ({ name, entry_type: "file", size: size ?? 100 });

// 1. Default depth 2: root children + their children counted; depth-3 dirs
// are counted as dirs but never listed.
{
  const { result, calls } = run(
    {
      ".": [d("src"), f("a.md", 100)],
      "./src": [f("main.rs", 200), d("deep")],
      "./src/deep": [f("never-listed.rs", 1)],
    },
    { path: "." }
  );
  check("default depth counts", summary(result.content), {
    dirs: 2,
    files: 2,
    size: "300B",
  });
  check("depth-3 dir never listed", calls.some((c) => c.path === "./src/deep"), false);
  check("success flag", result.success, true);
  check(".rs tallied", result.content.includes(".rs: 1 files"), true);
}

// 2. depth 1 lists only the root.
{
  const { result, calls } = run(
    {
      ".": [d("src"), f("top.md")],
      "./src": [f("main.rs")],
    },
    { path: ".", depth: 1 }
  );
  check("depth 1 counts", summary(result.content), { dirs: 1, files: 1, size: "100B" });
  check("depth 1 issues a single listing", calls.length, 1);
}

// 3. Noise dirs are counted, never descended, and announced.
{
  const { result, calls } = run(
    {
      ".": [d("node_modules"), d("src"), f("a.rs", 10)],
      "./src": [f("b.rs", 10)],
      // "./node_modules" intentionally absent: listing it would throw.
    },
    { path: "." }
  );
  check("skip counts", summary(result.content), { dirs: 2, files: 2, size: "20B" });
  check(
    "node_modules never listed",
    calls.some((c) => c.path === "./node_modules"),
    false
  );
  check("skip announced", result.content.includes("node_modules"), true);
  check("skip labelled", result.content.includes("never descended"), true);
}

// 4. Entry cap: a 1100-entry directory trips the 1000-entry budget, counts
// are marked partial, and the output says so with narrowing guidance.
{
  const big = [];
  for (let i = 0; i < 1100; i++) big.push(f("f" + i + ".log", 1));
  const { result, calls } = run({ ".": big }, { path: "." });
  check("cap trips at 1000", summary(result.content).files, 1000);
  check("cap announced", result.content.includes("stopped at 1000 entries"), true);
  check("cap suggests narrowing", result.content.includes("path/depth"), true);
  check("partial marker", result.content.includes("partial"), true);
  check("bounded request (budget + 1)", calls[0].max, 1001);
}

// 5. Inaccessible root returns a STRUCTURED failure, never a throw (thrown
// script errors reach the model under scary prefixes → retry spirals).
{
  const { result } = run({}, { path: "./no-such-dir" });
  check("failure is structured", result.success, false);
  check("failure names the path", result.content.includes("no-such-dir"), true);
  check("failure is instructive", result.content.includes("exists"), true);
}

// 6. Unreadable SUBdirectory is skipped and announced, not fatal.
{
  const { result } = run(
    {
      ".": [d("ok"), d("broken"), f("a.txt", 5)],
      "./ok": [f("b.txt", 5)],
      // "./broken" absent -> listing throws mid-walk.
    },
    { path: "." }
  );
  check("subdir failure not fatal", result.success, true);
  check("subdir failure announced", result.content.includes("could not be read"), true);
}

// 7. Empty root is an observation, not an error.
{
  const { result } = run({ ".": [] }, { path: "." });
  check("empty dir success", result.success, true);
  check("empty dir stated", result.content.includes("Empty directory"), true);
}

if (failures > 0) {
  console.error(failures + " test(s) failed");
  process.exit(1);
}
console.log("all tests passed");
