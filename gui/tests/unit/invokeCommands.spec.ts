import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

/**
 * Every `invoke('name')` in the frontend must name a command Tauri actually registers.
 *
 * An unregistered name is not a compile error on either side — it fails at runtime with
 * "Command not found", which the call sites swallow into a `catch` and a toast. And until the
 * toaster was fixed (see `componentResolution.spec.ts`) that toast never rendered, so the button
 * simply did nothing. That is how Settings → Data's destructive "Delete All Memories" shipped
 * calling `clear_all_memories`, a command that has never existed: the user confirmed the dialog
 * and nothing happened, silently.
 *
 * The authority is the `tauri::generate_handler![...]` list, not the set of `#[tauri::command]`
 * functions — a command that exists but is not in that list is equally unreachable.
 */

const GUI_ROOT = process.cwd()
const APP_DIR = join(GUI_ROOT, 'app')
const LIB_RS = join(GUI_ROOT, 'src-tauri/src/lib.rs')

/** Bounds: the frontend is ~120 files; these sit far above it and cap a runaway walk. */
const DIRECTORY_DEPTH_MAX = 12
const SOURCE_FILES_MAX = 2000

const SOURCE_EXTENSIONS = ['.vue', '.ts']

function collectSourceFiles(directory: string, depth = 0, found: string[] = []): string[] {
  if (depth > DIRECTORY_DEPTH_MAX) return found
  for (const name of readdirSync(directory)) {
    if (found.length >= SOURCE_FILES_MAX) return found
    const path = join(directory, name)
    if (statSync(path).isDirectory()) collectSourceFiles(path, depth + 1, found)
    else if (SOURCE_EXTENSIONS.some((extension) => path.endsWith(extension))) found.push(path)
  }
  return found
}

/**
 * Names inside `tauri::generate_handler![ ... ]`, read by matching brackets rather than by
 * finding the first `]` — the list spans hundreds of lines and contains comments.
 */
function readRegisteredCommands(source: string): Set<string> {
  const macroIndex = source.indexOf('tauri::generate_handler![')
  if (macroIndex < 0) throw new Error('generate_handler! not found in lib.rs')

  const open = source.indexOf('[', macroIndex)
  let depth = 0
  let close = -1
  for (let index = open; index < source.length; index += 1) {
    if (source[index] === '[') depth += 1
    else if (source[index] === ']') {
      depth -= 1
      if (depth === 0) {
        close = index
        break
      }
    }
  }
  if (close < 0) throw new Error('generate_handler! list is unterminated')

  return new Set(
    source
      .slice(open + 1, close)
      .replace(/\/\/[^\n]*/g, '')
      .split(',')
      .map((entry) => entry.trim())
      .filter(Boolean)
      // entries are paths like `commands::memory::clear_memories`
      .map((entry) => entry.split('::').pop()!),
  )
}

const registeredCommands = readRegisteredCommands(readFileSync(LIB_RS, 'utf8'))
const sourceFiles = collectSourceFiles(APP_DIR)

describe('tauri invoke commands', () => {
  // Negative space: an empty registry or file list would make the check below pass vacuously.
  it('reads a non-empty registered-command list', () => {
    expect(registeredCommands.size).toBeGreaterThan(0)
    expect(registeredCommands.has('clear_memories')).toBe(true)
  })

  it('never treats an unregistered name as registered', () => {
    expect(registeredCommands.has('clear_all_memories')).toBe(false)
    expect(registeredCommands.has('update_setting')).toBe(false)
  })

  it('finds the frontend sources to check', () => {
    expect(sourceFiles.length).toBeGreaterThan(0)
    expect(sourceFiles.length).toBeLessThanOrEqual(SOURCE_FILES_MAX)
  })

  it('registers every command the frontend invokes', () => {
    const unregistered: string[] = []
    for (const file of sourceFiles) {
      const source = readFileSync(file, 'utf8')
      for (const call of source.matchAll(/\binvoke\s*(?:<[^>]*>)?\s*\(\s*['"]([A-Za-z_]\w*)['"]/g)) {
        const command = call[1]!
        if (registeredCommands.has(command)) continue
        unregistered.push(`${relative(GUI_ROOT, file).replace(/\\/g, '/')}: invoke('${command}')`)
      }
    }
    expect(unregistered).toEqual([])
  })
})
