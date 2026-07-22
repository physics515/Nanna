import { spawn } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const outPath = path.resolve(root, '..', 'playwright-run', 'focused.log')
const out = createWriteStream(outPath, { flags: 'w' })

const args = [
  'exec',
  'playwright',
  'test',
  'e2e/critical-path.spec.ts',
  'e2e/a11y.spec.ts',
  'e2e/page-smoke.spec.ts',
  'e2e/error-boundary.spec.ts',
  '--reporter=line',
  '--workers=1',
  '--retries=0',
]

const child = spawn('pnpm', args, {
  cwd: root,
  env: {
    ...process.env,
    CI: '1',
    FORCE_COLOR: '0',
    PYTHONIOENCODING: 'utf-8',
  },
  shell: true,
  stdio: ['ignore', 'pipe', 'pipe'],
})

function pipe(stream) {
  stream.on('data', (chunk) => {
    process.stdout.write(chunk)
    out.write(chunk)
  })
}

pipe(child.stdout)
pipe(child.stderr)

child.on('exit', (code) => {
  out.end()
  console.log(`\n[run_focused_e2e] exit=${code} log=${outPath}`)
  process.exit(code ?? 1)
})
