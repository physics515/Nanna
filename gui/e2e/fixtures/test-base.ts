import { test as base, expect, type Page } from '@playwright/test'
import {
  e2eEmit,
  e2eGetState,
  e2eSetBackendStatus,
  installTauriMock,
  type MockOptions,
} from './tauri-mock'
import { normalizeMockOptions } from './mock-state'

export type MockHandle = {
  install: (config?: MockOptions) => Promise<void>
  gotoWithMock: (path?: string, config?: MockOptions) => Promise<void>
  emit: (event: string, payload?: unknown) => Promise<void>
  setDisconnected: (message?: string) => Promise<void>
  getState: () => Promise<Record<string, unknown>>
}

type NannaFixtures = {
  mock: MockHandle
}

/**
 * Playwright test with a hermetic Tauri/daemon mock.
 */
export const test = base.extend<NannaFixtures>({
  mock: async ({ page }, use) => {
    let last: MockOptions = {}

    const handle: MockHandle = {
      async install(config: MockOptions = {}) {
        last = normalizeMockOptions({ ...last, ...config })
        await installTauriMock(page, last)
      },
      async gotoWithMock(path = '/', config: MockOptions = {}) {
        last = normalizeMockOptions({ ...last, ...config })
        await installTauriMock(page, last)

        let target = path
        // Deep-link into a seeded session so chat tests don't depend on the
        // collapsible side panel. Skip when the caller seeds an empty list.
        if (path === '/' || path === '') {
          if (Array.isArray(last.sessions) && last.sessions.length === 0) {
            target = '/'
          } else if (Array.isArray(last.sessions) && last.sessions[0]?.id) {
            target = '/?session=' + last.sessions[0].id
          } else {
            target = '/?session=sess-seed-1'
          }
        }

        await page.goto(target, { waitUntil: 'domcontentloaded' })
        await page.waitForFunction(() => Boolean((window as any).__NANNA_E2E_READY__), null, {
          timeout: 30_000,
        })
        await page.waitForSelector('#__nuxt', { state: 'attached', timeout: 30_000 })
        await page.waitForTimeout(300)
      },
      async emit(event: string, payload?: unknown) {
        await e2eEmit(page, event, payload)
      },
      async setDisconnected(message = 'Daemon not reachable on 5149') {
        await e2eSetBackendStatus(page, {
          connected: false,
          mode: 'daemon',
          daemon_state: 'stopped',
          daemon_url: null,
          message,
        })
        await page.waitForTimeout(200)
        await page
          .getByText(/DISCONNECTED|Disconnected|not reachable|unreachable/i)
          .first()
          .waitFor({ state: 'visible', timeout: 8_000 })
          .catch(() => {})
      },
      async getState() {
        return e2eGetState(page)
      },
    }

    await use(handle)
  },
})

export { expect }

export async function expectPrimaryContent(page: Page, heading: string | RegExp) {
  const root = page.locator('#__nuxt')
  await expect(root).toBeAttached()
  await expect(page.getByRole('heading', { name: heading }).first()).toBeVisible({
    timeout: 15_000,
  })
  const text = await root.innerText()
  expect(text.trim().length).toBeGreaterThan(5)
}
