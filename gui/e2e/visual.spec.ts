import { expect, test } from './fixtures/test-base'

/**
 * Lightweight visual / theme regression.
 * Goldens: gui/e2e/__snapshots__/
 * Update: pnpm test:e2e:update
 */
test.describe('visual regression', () => {
  test('chat empty state', async ({ page, mock }) => {
    await mock.gotoWithMock('/', { sessions: [] })
    await expect(
      page.getByText(/Start a new conversation|New Chat|Nanna/i).first(),
    ).toBeVisible({ timeout: 20_000 })
    await expect(page).toHaveScreenshot('chat-empty.png', {
      maxDiffPixelRatio: 0.03,
      animations: 'disabled',
    })
  })

  test('settings shell', async ({ page, mock }) => {
    await mock.gotoWithMock('/settings')
    await expect(page.getByText('Settings').first()).toBeVisible({ timeout: 20_000 })
    await expect(page).toHaveScreenshot('settings-shell.png', {
      maxDiffPixelRatio: 0.03,
      animations: 'disabled',
    })
  })

  test('logs toolbar', async ({ page, mock }) => {
    await mock.gotoWithMock('/logs')
    await expect(page.getByText('Logs').first()).toBeVisible({ timeout: 20_000 })
    // Prefer capturing the header toolbar when present.
    const toolbar = page.locator('header').first()
    const target = (await toolbar.count()) > 0 ? toolbar : page.locator('body')
    await expect(target).toHaveScreenshot('logs-toolbar.png', {
      maxDiffPixelRatio: 0.04,
      animations: 'disabled',
    })
  })
})
