// Standalone regression test for the explore tool's depth handling.
// Run with: node test-explore-depth.mjs
//
// Recursive Nanna.listDir returns FULL paths (walkdir entry.path()), e.g.
// "D:\Development\nanna-bench\run-x\tests\test_01.sh". The original tool
// counted raw path separators against maxDepth, so every entry under a
// nested root was skipped and explore(".") reported "0 directories, 0
// files, 0B total". These tests stub Nanna.listDir with the real return
// shapes and assert non-zero counts and correct depth cutoffs.

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

function run(entries, input) {
  const tool = loadTool(() => entries);
  return tool.execute(input || {});
}

function summary(output) {
  const m = output.match(/(\d+) directories, (\d+) files, (\S+) total/);
  if (!m) throw new Error("no summary line in output:\n" + output);
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

// 1. The observed live failure: explore(".") over a workspace whose resolved
// root is several directories deep; listDir returns absolute backslash paths.
{
  const root = "D:\\Development\\nanna-bench\\run-x";
  const entries = [d(root + "\\tests")];
  for (let i = 1; i <= 42; i++) {
    entries.push(f(root + "\\tests\\test_" + String(i).padStart(2, "0") + ".sh"));
  }
  const out = run(entries, { path: "." });
  check("dot root, absolute backslash returns", summary(out), {
    dirs: 1,
    files: 42,
    size: "4.1KB",
  });
  check("dot root counts .sh extension", out.includes(".sh: 42 files"), true);
}

// 2. Absolute root passed explicitly: prefix strip, depth-3 entry excluded
// at the default depth of 2.
{
  const entries = [
    d("D:/proj/src"),
    f("D:/proj/src/main.rs"),
    d("D:/proj/src/deep"),
    f("D:/proj/src/deep/excluded.rs"),
  ];
  const out = run(entries, { path: "D:/proj" });
  check("absolute root, default depth cutoff", summary(out), {
    dirs: 2,
    files: 1,
    size: "100B",
  });
}

// 3. Same tree, root given with backslashes and a trailing separator.
{
  const entries = [
    d("D:\\proj\\src"),
    f("D:\\proj\\src\\main.rs"),
    d("D:\\proj\\src\\deep"),
    f("D:\\proj\\src\\deep\\excluded.rs"),
  ];
  const out = run(entries, { path: "D:\\proj\\" });
  check("trailing-backslash root", summary(out), {
    dirs: 2,
    files: 1,
    size: "100B",
  });
}

// 4. depth: 1 keeps only the root's direct children.
{
  const entries = [
    d("D:/proj/src"),
    f("D:/proj/top.md"),
    f("D:/proj/src/main.rs"),
  ];
  const out = run(entries, { path: "D:/proj", depth: 1 });
  check("depth 1 keeps direct children only", summary(out), {
    dirs: 1,
    files: 1,
    size: "100B",
  });
}

// 5. Root-relative returns (tolerated even though the current bridge sends
// absolute paths).
{
  const entries = [d("tests"), f("tests/a.sh"), f("tests/b.sh")];
  const out = run(entries, { path: "." });
  check("relative returns", summary(out), { dirs: 1, files: 2, size: "200B" });
}

// 6. Root that doesn't literally prefix the entries (case-mismatched drive
// letter): falls back to the shallowest-entry baseline instead of skipping
// everything.
{
  const entries = [
    d("D:/proj/src"),
    f("D:/proj/src/main.rs"),
    f("D:/proj/src/deep/excluded.rs"),
  ];
  const out = run(entries, { path: "d:/proj" });
  check("case-mismatched root falls back to baseline", summary(out), {
    dirs: 1,
    files: 1,
    size: "100B",
  });
}

// 7. Empty listing still reports inaccessible, not a zero summary.
{
  const out = run([], { path: "." });
  check(
    "empty listing",
    out.startsWith("Empty or inaccessible directory"),
    true
  );
}

if (failures > 0) {
  console.error(failures + " test(s) failed");
  process.exit(1);
}
console.log("all tests passed");
