#!/usr/bin/env node
/**
 * Generate the auto-updater manifest (.updater/latest.json at the repo root)
 * after `pnpm tauri build`.
 *
 * The updater endpoint is the RAW master copy of this file — GitHub's
 * `releases/latest/download` shortcut skips pre-releases, so beta releases
 * would be invisible through it. Committing the manifest to master is the
 * publish step: installed apps poll it, verify the minisign signature, and
 * download the installer from the release tag named here.
 *
 * Usage: node scripts/make-updater-manifest.mjs <tag> [notes]
 *   <tag>   the git/GitHub release tag the assets live under, e.g. v0.2.1-beta.2
 *   [notes] optional one-line release notes shown by the updater
 */

import { readFileSync, writeFileSync, mkdirSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';
import { execSync } from 'child_process';

const __dirname = dirname(fileURLToPath(import.meta.url));
const guiDir = join(__dirname, '..');
const rootDir = join(guiDir, '..');

const tag = process.argv[2];
if (!tag) {
  console.error('Usage: node scripts/make-updater-manifest.mjs <release-tag> [notes]');
  process.exit(1);
}
const notes = process.argv[3] || `Nanna ${tag}`;

const conf = JSON.parse(readFileSync(join(guiDir, 'src-tauri', 'tauri.conf.json'), 'utf8'));
const version = conf.version;

// Same target-dir resolution as build-daemon.js: `<root>/target` is only the
// default and a global .cargo/config.toml target-dir moves it.
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
    // Fall through.
  }
  return join(rootDir, 'target');
}

const setupName = `Nanna_${version}_x64-setup.exe`;
const sigPath = join(resolveTargetDir(), 'release', 'bundle', 'nsis', `${setupName}.sig`);
const signature = readFileSync(sigPath, 'utf8').trim();

const manifest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': {
      signature,
      url: `https://github.com/physics515/Nanna/releases/download/${tag}/${setupName}`,
    },
  },
};

const outDir = join(rootDir, '.updater');
mkdirSync(outDir, { recursive: true });
const outPath = join(outDir, 'latest.json');
writeFileSync(outPath, JSON.stringify(manifest, null, 2) + '\n');
console.log(`Wrote ${outPath} for ${version} -> ${manifest.platforms['windows-x86_64'].url}`);
