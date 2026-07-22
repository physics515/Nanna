#!/usr/bin/env node
/**
 * Tauri WebDriver smoke scaffold.
 * Full automation needs tauri-driver + a debug build. When those are missing we
 * exit 0 (skip) so web E2E CI stays hermetic.
 *
 * Sequence when armed:
 *   launch → main chrome → Settings → Logs → window close hygiene
 */
import { existsSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const guiRoot = path.resolve(__dirname, '..')

function findDebugBinary() {
  const candidates = [
    path.join(guiRoot, 'src-tauri', 'target', 'debug', 'nanna-gui.exe'),
    path.join(guiRoot, 'src-tauri', 'target', 'debug', 'nanna-gui'),
    path.join(guiRoot, 'src-tauri', 'target', 'release', 'nanna-gui.exe'),
    path.join(guiRoot, 'src-tauri', 'target', 'release', 'nanna-gui'),
  ]
  return candidates.find((p) => existsSync(p)) ?? null
}

function hasTauriDriver() {
  const r = spawnSync('tauri-driver', ['--help'], { encoding: 'utf8' })
  return r.status === 0 || (r.stdout || r.stderr || '').length > 0
}

const bin = findDebugBinary()
if (!bin || !hasTauriDriver()) {
  console.log(
    '[tauri-driver-smoke] skip — need debug binary under src-tauri/target and tauri-driver on PATH.\n' +
      'See gui/e2e/tauri-driver.md',
  )
  process.exit(0)
}

console.log(`[tauri-driver-smoke] binary: ${bin}`)
console.log('[tauri-driver-smoke] launch → Settings → Logs → close (manual WebDriver wiring next).')
// Placeholder for WebDriverIO/selenium bindings once CI hosts a display + driver pair.
// Exit non-zero only when explicitly armed via NANNA_TAURI_E2E=1 and the session fails.
if (process.env.NANNA_TAURI_E2E === '1') {
  console.error(
    '[tauri-driver-smoke] NANNA_TAURI_E2E=1 set but WebDriver session is not yet wired.',
  )
  process.exit(1)
}
console.log('[tauri-driver-smoke] soft-pass (binary present; session wiring TBD).')
process.exit(0)
