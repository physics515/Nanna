import { expect, test } from './fixtures/test-base'

test.describe('error boundary', () => {
  test('recoverable panel when a child throws', async ({ page, mock }) => {
    await mock.gotoWithMock('/')
    await expect(page.locator('[data-testid="error-boundary"]')).toBeAttached()

    // Force the boundary into its error state via the e2e hook.
    await page.evaluate(() => {
      const w = window as any
      if (typeof w.__NANNA_FORCE_ERROR__ === 'function') {
        w.__NANNA_FORCE_ERROR__()
        return
      }
      window.dispatchEvent(new CustomEvent('nanna-e2e-force-error'))
    })

    await expect(
      page.getByText(/Something went wrong|Try again|Reload/i).first(),
    ).toBeVisible({ timeout: 5_000 })

    // Recover
    await page.getByRole('button', { name: /try again/i }).click()
    await expect(page.getByText(/Something went wrong/i)).toHaveCount(0)
  })
})
