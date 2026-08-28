import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

/**
 * Guards against a template component imported from a package that does not export it.
 *
 * `componentResolution.spec.ts` already proves every PascalCase tag is *bound* — by the Nuxt
 * registry or by a local `import`. It cannot prove the binding has a value: an
 * `import { Foo } from 'pkg'` where `pkg` no longer exports `Foo` still creates the binding,
 * still typechecks (the package's `export *` re-exports make the name look present to
 * `vue-tsc`), and still renders — as `undefined`, which Vue turns into an inert unknown
 * element plus a `[Vue warn]` nobody reads. The feature just stops existing.
 *
 * That is not hypothetical and it is not one-off. It is what `<UiSonnerSonner/>` did to every
 * toast in the app, and it is what Tiptap 3 did to the editor's floating toolbar: v3 moved the
 * menu components out of the package root into `@tiptap/vue-3/menus`, so the untouched
 * `import { BubbleMenu } from '@tiptap/vue-3'` compiled, typechecked, and evaluated to
 * `undefined`. A dependency bump is exactly when this happens, and a dependency bump is exactly
 * when nobody opens the editor to look.
 *
 * So: resolve each such import for real and assert the binding has a value.
 */

/** Vitest resolves `root` to the directory holding `vitest.config.ts`, i.e. `gui/`. */
const GUI_ROOT = process.cwd()
const APP_DIR = join(GUI_ROOT, 'app')

/** Bounds: the tree is ~91 files, 4 levels deep — these sit far above it and cap a runaway walk. */
const DIRECTORY_DEPTH_MAX = 12
const VUE_FILES_MAX = 2000
/**
 * The cost of this guard is one dynamic `import()` per *distinct module* — the loader caches,
 * so each additional binding from an already-imported package is a property lookup. Bindings
 * therefore run into the hundreds honestly (every `@lucide/vue` icon a template renders is
 * one), and bounding them would be bounding the wrong number. Bound the modules instead.
 */
const PACKAGE_MODULES_MAX = 64

/** Collect every `.vue` file under `directory`, depth-bounded and count-bounded. */
function collectVueFiles(directory: string, depth = 0, found: string[] = []): string[] {
  if (depth > DIRECTORY_DEPTH_MAX) return found
  for (const name of readdirSync(directory)) {
    if (found.length >= VUE_FILES_MAX) return found
    const path = join(directory, name)
    if (statSync(path).isDirectory()) collectVueFiles(path, depth + 1, found)
    else if (path.endsWith('.vue')) found.push(path)
  }
  return found
}

/**
 * A component name a template uses, and where the script said it comes from.
 *
 * `local` is the name the template writes; `exported` is the name the package must provide
 * (they differ under `{ Toaster as Sonner }`).
 */
interface PackageComponentImport {
  file: string
  module: string
  local: string
  exported: string
}

/**
 * `true` for a bare package specifier — the only kind this guard can meaningfully resolve.
 *
 * Relative and alias imports (`./x`, `~/lib/x`, `@/x`) are project files: a missing export
 * there is a build error, not a silent `undefined`, and resolving the aliases here would mean
 * reimplementing Nuxt's resolver.
 */
function isPackageSpecifier(module: string): boolean {
  if (module.startsWith('.') || module.startsWith('/')) return false
  if (module.startsWith('~') || module.startsWith('#')) return false
  return !module.startsWith('@/')
}

/** PascalCase tags opened anywhere in the file's `<template>` blocks. */
function readTemplateComponentTags(source: string): Set<string> {
  const template = [...source.matchAll(/<template>([\s\S]*?)<\/template>/g)]
    .map(block => block[1]!)
    .join('\n')
  return new Set([...template.matchAll(/<([A-Z][A-Za-z0-9]*)/g)].map(match => match[1]!))
}

/**
 * Blank out `//` and block comments so a *quoted* import statement is not read as a real one.
 *
 * Not cosmetic: the first version of this guard reported `FloatingToolbar.vue` importing
 * `BubbleMenu` from `@tiptap/vue-3` — text that appears only inside the comment explaining why
 * the import moved. A guard that fires on prose about the bug instead of the bug is worse than
 * no guard, because the fix is to delete the explanation.
 */
function stripComments(script: string): string {
  return script.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:'"])\/\/[^\n]*/g, '$1')
}

/**
 * Named bindings a script pulls out of bare packages, as `local -> exported` per module.
 *
 * Only the braced form is read. A default import (`import Foo from 'pkg'`) cannot go missing
 * the same way — a package without a default export fails at resolution, loudly — and a
 * namespace import binds an object that is never itself a component.
 */
function readPackageNamedImports(script: string, file: string): PackageComponentImport[] {
  const imports: PackageComponentImport[] = []
  for (const statement of stripComments(script).matchAll(
    /import\s*\{([\s\S]*?)\}\s*from\s*['"]([^'"]+)['"]/g,
  )) {
    const module = statement[2]!
    if (!isPackageSpecifier(module)) continue
    for (const clause of statement[1]!.split(',')) {
      const parts = clause.trim().split(/\s+as\s+/)
      const exported = parts[0]?.trim()
      const local = (parts[1] ?? parts[0])?.trim()
      // `import { type Foo }` binds nothing at runtime, so there is nothing to resolve.
      if (!exported || !local || exported.startsWith('type ')) continue
      if (!/^[A-Za-z_$][\w$]*$/.test(exported) || !/^[A-Za-z_$][\w$]*$/.test(local)) continue
      imports.push({ file, module, local, exported })
    }
  }
  return imports
}

const vueFiles = collectVueFiles(APP_DIR)

/** Every package-imported name that a template actually renders as a component. */
const componentImports: PackageComponentImport[] = []
for (const file of vueFiles) {
  const source = readFileSync(file, 'utf8')
  const script = [...source.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)]
    .map(block => block[1]!)
    .join('\n')
  const tags = readTemplateComponentTags(source)
  for (const candidate of readPackageNamedImports(script, file))
    if (tags.has(candidate.local)) componentImports.push(candidate)
}

describe('package component exports', () => {
  // Negative space: an empty list would make the resolution test below pass vacuously — the
  // exact way a guard stops guarding without anyone noticing.
  it('finds package-imported components to check', () => {
    expect(componentImports.length).toBeGreaterThan(0)
    expect(new Set(componentImports.map(entry => entry.module)).size)
      .toBeLessThanOrEqual(PACKAGE_MODULES_MAX)
  })

  it('knows the Tiptap menu components live under the /menus subpath', async () => {
    // The regression that motivated this file, pinned as a fixture: the root export is gone
    // and the subpath has it. If Tiptap ever moves them back, this fails and the guard's own
    // premise gets re-read rather than silently rotting.
    const root: Record<string, unknown> = await import('@tiptap/vue-3')
    const menus: Record<string, unknown> = await import('@tiptap/vue-3/menus')
    expect(root.BubbleMenu).toBeUndefined()
    expect(menus.BubbleMenu).toBeDefined()
  })

  it('resolves every component a template imports from a package', async () => {
    const missing: string[] = []
    for (const entry of componentImports) {
      const where = `${relative(GUI_ROOT, entry.file).replace(/\\/g, '/')}: <${entry.local}>`
      let module: Record<string, unknown>
      try {
        module = (await import(/* @vite-ignore */ entry.module)) as Record<string, unknown>
      } catch (error) {
        missing.push(`${where} — '${entry.module}' failed to import: ${String(error)}`)
        continue
      }
      if (module[entry.exported] === undefined)
        missing.push(`${where} — '${entry.module}' exports no '${entry.exported}'`)
    }
    expect(missing).toEqual([])
  })
})
