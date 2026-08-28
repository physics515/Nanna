import { defineConfig, devices } from '@playwright/test'

/**
 * Web/dev-shell E2E against `pnpm dev` (or a built preview) — no Tauri shell.
 * Use for fast iteration; Tauri WebDriver stays in scripts/tauri-driver-smoke.mjs.
 *
 * Snapshots: gui/e2e/__snapshots__ (via snapshotPathTemplate below).
 */
const PORT = Number(process.env.PLAYWRIGHT_PORT ?? 3456)
const BASE = process.env.PLAYWRIGHT_BASE_URL ?? `http://127.0.0.1:${PORT}`
const useExisting = Boolean(process.env.PLAYWRIGHT_BASE_URL)

export default defineConfig({
  testDir: './e2e',
  testMatch: '**/*.spec.ts',
  testIgnore: ['**/_*.spec.ts', '**/tauri/**'],
  fullyParallel: false,
  forbidOnly: process.env.CI === 'true' || process.env.CI === '1',
  retries: process.env.CI === 'true' || process.env.CI === '1' ? 1 : 0,
  workers: 1,
  timeout: 60_000,
  expect: {
    timeout: 12_000,
    toHaveScreenshot: {
      // Tolerate font antialias / subpixel variance across CI agents.
      maxDiffPixelRatio: 0.03,
      animations: 'disabled',
    },
  },
  // Goldens under gui/e2e/__snapshots__
  snapshotPathTemplate: '{testDir}/__snapshots__/{testFilePath}/{arg}{ext}',
  reporter: (process.env.CI === 'true' || process.env.CI === '1')
    ? [
        ['github'],
        ['html', { open: 'never', outputFolder: 'playwright-report' }],
        ['list'],
      ]
    : [['list']],
  use: {
    baseURL: BASE,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'off',
    ...devices['Desktop Chrome'],
    viewport: { width: 1440, height: 900 },
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: useExisting
    ? undefined
    : {
        command: `pnpm exec nuxi dev --host 127.0.0.1 --port ${PORT}`,
        url: BASE,
        reuseExistingServer: !process.env.CI,
        timeout: 180_000,
        stdout: 'pipe',
        stderr: 'pipe',
      },
  outputDir: 'test-results',
})
