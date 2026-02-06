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

// Source and destination paths
const sourcePath = join(rootDir, 'target', profile, binaryName);
const destPath = join(binariesDir, sidecarName);

console.log('Building nanna-daemon...');
console.log(`  Platform: ${process.platform}`);
console.log(`  Target: ${targetTriple}`);
console.log(`  Profile: ${profile}`);
console.log(`  Root dir: ${rootDir}`);

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
