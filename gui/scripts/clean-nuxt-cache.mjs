#!/usr/bin/env node
/**
 * Wipe Nuxt build/cache dirs so `nuxt generate` cannot race a stale
 * node_modules/.cache/nuxt tree against a project-local .nuxt buildDir.
 *
 * Invoked before `nuxt generate` (see package.json "generate").
 */
import { rmSync, existsSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const targets = [
  join(root, '.nuxt'),
  join(root, 'node_modules', '.cache', 'nuxt'),
]

for (const dir of targets) {
  if (!existsSync(dir)) continue
  try {
    rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 })
    console.log(`cleaned ${dir}`)
  } catch (err) {
    // Non-fatal: a locked file on Windows should not abort the build — Nuxt
    // will overwrite. Log and continue.
    console.warn(`could not fully clean ${dir}: ${err?.message ?? err}`)
  }
}
