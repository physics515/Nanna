# Tauri WebDriver smoke

Web/dev-shell Playwright covers Nuxt quickly without a packaged shell. Full-window
smoke still needs `tauri-driver` + a debug build of the GUI.

## Prerequisites

1. Install [tauri-driver](https://v2.tauri.app/develop/tests/webdriver/):

   ```bash
   cargo install tauri-driver --locked
   ```

2. Edge/Chrome WebDriver matching the host OS (Windows: Microsoft Edge Driver on PATH).
3. Build a debug / unpackaged app:

   ```bash
   cd gui
   pnpm build
   pnpm build:daemon
   pnpm tauri build --debug
   ```

## Smoke sequence (automated by `scripts/tauri-driver-smoke.mjs`)

1. Launch the packaged/debug binary under `tauri-driver`.
2. Assert main chrome (title bar / nav).
3. Open **Settings**.
4. Open **Logs**.
5. Close window — assert process exit / no orphan daemon when close-to-tray is disabled.

```bash
cd gui
pnpm test:e2e:tauri
```

CI: nightly / packaging jobs only (see `.github/workflows/gui.yml`). Artifact upload
on failure.

Until the binary path is present, `tauri-driver-smoke.mjs` exits `0` with a skipped
message so local web E2E stays green without a full Tauri toolchain.
