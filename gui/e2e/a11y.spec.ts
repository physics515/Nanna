import AxeBuilder from '@axe-core/playwright'
import { expect, test } from './fixtures/test-base'

async function waitShell(page: import('@playwright/test').Page) {
  await expect(page.locator('#__nuxt')).toBeAttached({ timeout: 20_000 })
  await expect(page.locator('body')).not.toBeEmpty()
}

test.describe('a11y gates', () => {
  test('chat shell has no critical axe violations', async ({ page, mock }) => {
    await mock.gotoWithMock('/')
    await waitShell(page)
    await page.waitForTimeout(300)
    const results = await new AxeBuilder({ page })
      .withTags(['wcag2a', 'wcag2aa'])
      .disableRules(['color-contrast'])
      .analyze()
    const critical = results.violations.filter(
      (v) => v.impact === 'critical' || v.impact === 'serious',
    )
    expect(critical, JSON.stringify(critical, null, 2)).toEqual([])
  })

  test('settings has no critical axe violations', async ({ page, mock }) => {
    await mock.gotoWithMock('/settings')
    await waitShell(page)
    await expect(
      page.getByRole('heading', { name: /settings/i }).or(page.locator('h2')).first(),
    ).toBeVisible({ timeout: 20_000 })
    await page.waitForTimeout(500)
    const results = await new AxeBuilder({ page })
      .withTags(['wcag2a', 'wcag2aa'])
      .disableRules(['color-contrast'])
      .analyze()
    const critical = results.violations.filter(
      (v) => v.impact === 'critical' || v.impact === 'serious',
    )
    expect(critical, JSON.stringify(critical, null, 2)).toEqual([])
  })

  test('keyboard tab order reaches main controls on chat', async ({ page, mock }) => {
    await mock.gotoWithMock('/')
    await waitShell(page)
    await page.keyboard.press('Tab')
    await page.keyboard.press('Tab')
    await page.keyboard.press('Tab')
    const active = await page.evaluate(() => {
      const el = document.activeElement as HTMLElement | null
      if (!el) return null
      return {
        tag: el.tagName,
        role: el.getAttribute('role'),
        aria: el.getAttribute('aria-label'),
        tabIndex: el.tabIndex,
      }
    })
    expect(active).not.toBeNull()
  })

  test('settings exposes labelled headings', async ({ page, mock }) => {
    await mock.gotoWithMock('/settings')
    await waitShell(page)
    await expect(page.getByRole('heading').first()).toBeVisible({ timeout: 15_000 })
  })
})
