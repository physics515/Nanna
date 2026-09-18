import AxeBuilder from '@axe-core/playwright'
import type { Page } from '@playwright/test'
import { expect, test } from './fixtures/test-base'
import type { MockDaemonFailure } from './fixtures/mock-state'

/**
 * The startup gate: the splash holds the shell back until the daemon answers
 * once (or the person opens Nanna anyway), and never comes back afterwards.
 * Every other spec starts attached, so there the gate releases at once.
 */

const splash = (page: Page) => page.getByTestId('startup-splash')
const splashStatus = (page: Page) => splash(page).getByRole('status')
/** Only the shell has the chat menu's New chat button; the splash has none. */
const shell = (page: Page) => page.getByRole('button', { name: 'New chat', exact: true })

const ipcPortTaken: MockDaemonFailure = {
  kind: 'exited_during_boot',
  message: 'Error: IPC port 127.0.0.1:5149 unavailable: address in use',
  exit_code: 1,
  signal: null,
  at_ms: 1_758_000_000_000,
}

async function expectNoSeriousAxeViolations(page: Page) {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa'])
    .disableRules(['color-contrast'])
    .analyze()
  const serious = results.violations.filter((v) => v.impact === 'critical' || v.impact === 'serious')
  expect(serious, JSON.stringify(serious, null, 2)).toEqual([])
}

test.describe('startup splash', () => {
  test('holds the shell back while the daemon starts', async ({ page, mock }) => {
    await mock.gotoWithMock('/', { boot: { daemon_state: 'starting', starting_for_s: 3 } })
    await expect(splash(page)).toBeVisible()
    await expect(splashStatus(page)).toHaveText('Starting the daemon…')
    await expect(page.getByRole('main').getByRole('heading', { name: 'Nanna' })).toBeVisible()
    await expect(shell(page)).toHaveCount(0)
    // The error boundary still wraps whatever renders.
    await expect(page.locator('[data-testid="error-boundary"]')).toBeAttached()
    await expectNoSeriousAxeViolations(page)
  })

  test('says how long a slow boot has run, and offers a restart', async ({ page, mock }) => {
    await mock.gotoWithMock('/', { boot: { daemon_state: 'starting', starting_for_s: 65 } })
    await expect(splashStatus(page)).toHaveText('Still starting · 1m 05s')
    await expect(splash(page)).toContainText('opens by itself as soon as it answers')
    await splash(page).getByRole('button', { name: 'Restart the daemon' }).click()
    await expect.poll(async () => (await mock.getState()).restartCalls).toBe(1)
    await expect(splashStatus(page)).toHaveText('Starting the daemon…')
  })

  test('opens the app shell when the daemon answers', async ({ page, mock }) => {
    await mock.gotoWithMock('/', { boot: { daemon_state: 'starting', starting_for_s: 3 } })
    await expect(splash(page)).toBeVisible()
    await mock.attach()
    await expect(shell(page)).toBeVisible()
    await expect(splash(page)).toHaveCount(0)
    await expect(page.getByText('Connected', { exact: true })).toBeVisible()
  })

  test('shows why a boot failed, and restarts from the focused button', async ({ page, mock }) => {
    await mock.gotoWithMock('/', {
      boot: {
        daemon_state: 'crashed',
        retrying: true,
        last_error: ipcPortTaken,
        log: [
          { stream: 'stdout', line: 'Loading config from ~/.nanna/config.toml' },
          { stream: 'stderr', line: 'Error: IPC port 127.0.0.1:5149 unavailable: address in use' },
        ],
      },
    })
    await expect(splashStatus(page)).toHaveText('The daemon exited while starting')
    const alert = splash(page).getByRole('alert')
    await expect(alert).toContainText(ipcPortTaken.message)
    await expect(alert).toContainText('exit code 1')

    const restart = splash(page).getByRole('button', { name: 'Restart the daemon' })
    await expect(restart).toBeFocused()

    await splash(page).getByRole('button', { name: 'Show log' }).click()
    const log = splash(page).getByRole('log', { name: 'Daemon output' })
    await expect(log).toContainText('Loading config from ~/.nanna/config.toml')
    await expect(log.getByText('Error: IPC port 127.0.0.1:5149 unavailable')).toHaveClass(/text-nui-pink/)
    await expectNoSeriousAxeViolations(page)

    await restart.click()
    await expect.poll(async () => (await mock.getState()).restartCalls).toBe(1)
    await expect(splashStatus(page)).toHaveText('Starting the daemon…')
    // Each spawn starts its own tail.
    await expect(log).toContainText('Starting the daemon (e2e mock restart)')
    await expect(log).not.toContainText('Loading config')
  })

  test('opens anyway offline, then remounts the page once on the first attach', async ({ page, mock }) => {
    await mock.gotoWithMock('/', { boot: { daemon_state: 'starting', starting_for_s: 3 } })
    await splash(page).getByRole('button', { name: 'Open Nanna anyway' }).click()
    await expect(shell(page)).toBeVisible()
    await expect(splash(page)).toHaveCount(0)
    // Offline, and saying so: the footer reads the boot's own state.
    await expect(page.locator('footer')).toContainText('Starting')

    const page0 = page.locator('main > *').first()
    await page0.evaluate((el) => el.setAttribute('data-e2e-mounted', 'offline'))

    await mock.attach()
    await expect(page.locator('footer')).toContainText('Connected')
    // The page that mounted offline was replaced by a fresh one.
    await expect(page.locator('[data-e2e-mounted="offline"]')).toHaveCount(0)
    await expect(page.locator('main > *').first()).toBeVisible()

    // Only once: a later disconnect and reconnect keep the page as it is.
    await page.locator('main > *').first().evaluate((el) => el.setAttribute('data-e2e-mounted', 'online'))
    await mock.setDisconnected('Daemon unreachable (e2e)')
    await expect(page.locator('footer')).toContainText(/offline/i)
    await mock.attach()
    await expect(page.locator('footer')).toContainText('Connected')
    await expect(page.locator('[data-e2e-mounted="online"]')).toHaveCount(1)
  })

  test('keeps Settings as it is on the first attach, typed input and all', async ({ page, mock }) => {
    await mock.gotoWithMock('/settings', { boot: { daemon_state: 'starting', starting_for_s: 3 } })
    await splash(page).getByRole('button', { name: 'Open Nanna anyway' }).click()
    const keyField = page
      .locator('input[type="password"], input[placeholder*="sk-"], input[autocomplete="off"]')
      .first()
    await expect(keyField).toBeVisible()
    await keyField.fill('sk-typed-while-offline')
    await keyField.evaluate((el) => el.setAttribute('data-e2e-mounted', 'offline'))

    await mock.attach()
    await expect(page.locator('footer')).toContainText('Connected')
    // Nothing to wait for when nothing happens: give a remount (which lands
    // within a frame or two of the footer's change) ample time to show.
    await page.waitForTimeout(1_000)
    await expect(page.locator('[data-e2e-mounted="offline"]')).toHaveValue('sk-typed-while-offline')
  })

  test('keeps Logs as it is on the first attach, filter and all', async ({ page, mock }) => {
    await mock.gotoWithMock('/logs', { boot: { daemon_state: 'starting', starting_for_s: 3 } })
    await splash(page).getByRole('button', { name: 'Open Nanna anyway' }).click()
    const search = page.getByRole('textbox', { name: 'Search logs' })
    await expect(search).toBeVisible()
    await search.fill('Attached')
    await search.evaluate((el) => el.setAttribute('data-e2e-mounted', 'offline'))

    await mock.attach()
    await expect(page.locator('footer')).toContainText('Connected')
    await page.waitForTimeout(1_000)
    await expect(page.locator('[data-e2e-mounted="offline"]')).toHaveValue('Attached')
  })

  test('opening anyway mid-boot mounts the whole shell without waiting for the boot', async ({ page, mock }) => {
    await mock.gotoWithMock('/', { boot: { daemon_state: 'starting', starting_for_s: 3 } })
    await splash(page).getByRole('button', { name: 'Open Nanna anyway' }).click()
    await expect(shell(page)).toBeVisible()
    // The first load ran while the boot goes on: the chat list is filled.
    await expect(page.getByRole('button', { name: /^Welcome/ })).toBeVisible()
    await expect(page.locator('footer')).toContainText('Starting')
    // And the window's own close reaches the shell's handler, which asks.
    // Emitted until heard: the handler is registered a few awaits in.
    await expect(async () => {
      await mock.emit('tauri://close-requested', {})
      await expect(page.getByText('Close Nanna?')).toBeVisible({ timeout: 500 })
    }).toPass({ timeout: 12_000 })
  })

  test('stays readable in a window whose animations never advance', async ({ page, mock }) => {
    await mock.gotoWithMock('/', { boot: { daemon_state: 'crashed', retrying: true, last_error: ipcPortTaken } })
    await expect(splashStatus(page)).toHaveText('The daemon exited while starting')
    // The tauri-webdriver window renders no frames, so each CSS animation sits
    // at its first keyframe there. Hold every one of them at t=0.
    const held = await page.evaluate(() => {
      const animations = document.getAnimations()
      for (const animation of animations) {
        animation.pause()
        animation.currentTime = 0
      }
      return animations.length
    })
    expect(held).toBeGreaterThan(0)

    /** Opacity as painted: the element's own times every ancestor's. */
    const paintedOpacity = (locator: ReturnType<Page['locator']>) =>
      locator.evaluate((el) => {
        let opacity = 1
        for (let node: Element | null = el; node; node = node.parentElement) {
          opacity *= Number(getComputedStyle(node).opacity)
        }
        return opacity
      })
    const targets = [
      splash(page).getByRole('alert'),
      splash(page).getByRole('button', { name: 'Restart the daemon' }),
      splash(page).getByRole('button', { name: 'Open Nanna anyway' }),
      splash(page).getByRole('button', { name: 'Show log' }),
      splash(page).getByRole('button', { name: 'Quit' }),
    ]
    for (const target of targets) {
      expect(await paintedOpacity(target)).toBeGreaterThan(0.9)
    }
  })

  test('opens anyway on Esc', async ({ page, mock }) => {
    await mock.gotoWithMock('/', { boot: { daemon_state: 'starting', starting_for_s: 3 } })
    await expect(splash(page)).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(shell(page)).toBeVisible()
    await expect(splash(page)).toHaveCount(0)
  })

  test('never comes back after the first attach', async ({ page, mock }) => {
    await mock.gotoWithMock('/')
    await expect(shell(page)).toBeVisible()
    await expect(splash(page)).toHaveCount(0)

    // What the health monitor's restart looks like: disconnected, starting again.
    await page.evaluate(() => {
      window.__NANNA_E2E__?.setBackendStatus({ connected: false, daemon_state: 'starting', starting_for_s: 40 })
    })
    await expect(page.locator('footer')).toContainText('Still starting · 40s')
    // And the updater's own stop: stopped, not retrying.
    await page.evaluate(() => {
      window.__NANNA_E2E__?.setBackendStatus({ connected: false, daemon_state: 'stopped', retrying: false })
    })
    await expect(page.locator('footer')).toContainText('Daemon offline')
    await expect(splash(page)).toHaveCount(0)
    await expect(shell(page)).toBeVisible()
  })
})
