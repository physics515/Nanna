#!/usr/bin/env node
// The GUI typecheck gate, and the proof that it can see.
//
// Nuxt 4 writes a solution-style tsconfig (`"files": []` plus project references).
// Plain `vue-tsc --noEmit` on that checks ZERO files and exits 0, which is how CI
// reported "0 errors" for weeks while 96 real ones sat behind it (ROADMAP P4,
// 2026-08-23). `--build` follows the references and checks the app.
//
// A gate that checks nothing looks exactly like a gate that passes. So after the
// real run, this plants a file with a deliberate type error inside `app/` and
// requires the checker to reject it by name. If it does not, the gate itself is
// broken and this script fails: the typecheck equivalent of a coverage gate that
// fails at 0%.
import { spawnSync } from 'node:child_process'
import { rmSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const guiDir = join(dirname(fileURLToPath(import.meta.url)), '..')
const canaryName = '__typecheck_canary__.ts'
const canaryPath = join(guiDir, 'app', canaryName)

function vueTscBuild() {
  const run = spawnSync('pnpm', ['exec', 'vue-tsc', '--build'], {
    cwd: guiDir,
    encoding: 'utf8',
    shell: process.platform === 'win32',
  })
  return { status: run.status, output: `${run.stdout ?? ''}${run.stderr ?? ''}` }
}

const real = vueTscBuild()
process.stdout.write(real.output)
if (real.status !== 0) {
  console.error(`typecheck: vue-tsc --build failed (exit ${real.status})`)
  process.exit(real.status ?? 1)
}

writeFileSync(
  canaryPath,
  "// Planted by scripts/typecheck.mjs and deleted when it finishes.\n" +
    "export const canary: number = 'not a number'\n",
)
let canary
try {
  canary = vueTscBuild()
} finally {
  rmSync(canaryPath, { force: true })
}

if (canary.status === 0 || !canary.output.includes(canaryName)) {
  console.error('typecheck: the gate is blind: a planted type error in app/ was not reported.')
  console.error(canary.output)
  process.exit(1)
}
console.log('typecheck: 0 errors, and the planted canary proved the checker reads app/')
