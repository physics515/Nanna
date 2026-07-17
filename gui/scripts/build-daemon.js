#!/usr/bin/env node
/**
 * Build the nanna-daemon binary for the current platform
 * and copy it to the Tauri sidecar location
 */

import { execSync } from 'child_process';
import { copyFileSync, mkdirSync, existsSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

// Paths
const guiDir = join(__dirname, '..');
const rootDir = join(guiDir, '..');
const binariesDir = join(guiDir, 'src-tauri', 'binaries');

// Platform-specific settings
const isWindows = process.platform === 'win32';
const isMac = process.platform === 'darwin';
const isLinux = process.platform === 'linux';

// Determine target triple
let targetTriple;
if (isWindows) {
  targetTriple = 'x86_64-pc-windows-msvc';
} else if (isMac) {
  // Check for ARM vs Intel
  const arch = process.arch === 'arm64' ? 'aarch64' : 'x86_64';
  targetTriple = `${arch}-apple-darwin`;
} else if (isLinux) {
  targetTriple = 'x86_64-unknown-linux-gnu';
} else {
  console.error('Unsupported platform:', process.platform);
  process.exit(1);
}

// Binary name
const binaryName = isWindows ? 'nanna-daemon.exe' : 'nanna-daemon';
const sidecarName = isWindows 
  ? `nanna-daemon-${targetTriple}.exe` 
  : `nanna-daemon-${targetTriple}`;

// Check for --debug flag
const isDebug = process.argv.includes('--debug');
const profile = isDebug ? 'debug' : 'release';

/**
 * Ask cargo where it actually puts build artifacts.
 *
 * `<root>/target` is only the default: CARGO_TARGET_DIR or a `target-dir` in any
 * .cargo/config.toml (including the user's global one) moves it elsewhere, and
 * hardcoding the default makes `cargo tauri build` fail on those machines with a
 * confusing ENOENT on copyfile after a successful compile.
 */
function resolveTargetDir() {
  try {
    const meta = execSync('cargo metadata --format-version 1 --no-deps', {
      cwd: rootDir,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    });
    const dir = JSON.parse(meta).target_directory;
    if (dir) return dir;
  } catch {
    // Fall through to the default layout below.
  }
  return join(rootDir, 'target');
}

// Source and destination paths
const targetDir = resolveTargetDir();
const sourcePath = join(targetDir, profile, binaryName);
const destPath = join(binariesDir, sidecarName);

console.log('Building nanna-daemon...');
console.log(`  Platform: ${process.platform}`);
console.log(`  Target: ${targetTriple}`);
console.log(`  Profile: ${profile}`);
console.log(`  Root dir: ${rootDir}`);
console.log(`  Target dir: ${targetDir}`);

try {
  // Build the daemon
  const buildCmd = isDebug
    ? 'cargo build --package nanna-daemon'
    : 'cargo build --package nanna-daemon --release';
  execSync(buildCmd, {
    cwd: rootDir,
    stdio: 'inherit',
  });
  
  // Ensure binaries directory exists
  if (!existsSync(binariesDir)) {
    mkdirSync(binariesDir, { recursive: true });
  }
  
  // Copy the binary
  console.log(`Copying ${sourcePath} -> ${destPath}`);
  copyFileSync(sourcePath, destPath);
  
  console.log('✓ Daemon build complete!');
} catch (error) {
  console.error('Failed to build daemon:', error.message);
  process.exit(1);
}
