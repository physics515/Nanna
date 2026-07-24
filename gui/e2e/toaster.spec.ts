import { expect, test } from './fixtures/test-base'

/**
 * The toaster must actually mount and render toasts.
 *
 * `app.vue` rendered `<UiSonnerSonner />` for months. Nuxt registers that component as `UiSonner`
 * (it collapses a filename that repeats its parent directory), so the tag resolved to nothing:
 * the `<Toaster>` never mounted and every `useToast()` call — the success/error feedback behind
 * copy, save, delete and clear across the whole app — was silently dropped.
 *
 * Vue's `Failed to resolve component` warning is **not** a usable signal here: it is emitted by
 * the dev server's render pass, not into the browser console, so a Playwright console listener
 * never sees it. Hence this test asserts the *behaviour* instead. The static name check lives in
 * `tests/unit/componentResolution.spec.ts`, which validates every `.vue` file in the app against
 * the generated Nuxt registry.
 */
test('useToast renders a toast through the mounted toaster', async ({ page, context, mock }) => {
  // copyAllLogs() writes to the clipboard first; without permission it takes the error branch,
  // which still toasts — granting it lets the success message through instead.
  await context.grantPermissions(['clipboard-read', 'clipboard-write']).catch(() => {})

  await mock.gotoWithMock('/logs')
  await expect(page.locator('#__nuxt')).toBeAttached()
  await expect(page.getByRole('heading', { name: /logs/i }).first()).toBeVisible({ timeout: 25_000 })

  // No toast has been raised yet, so nothing should be on screen.
  await expect(page.locator('[data-sonner-toast]')).toHaveCount(0)

  // The glass buttons animate their mesh background forever, so Playwright's stability check
  // never settles — assert actionability explicitly, then click past it.
  const copyAll = page.getByRole('button', { name: /copy all/i }).first()
  await expect(copyAll).toBeVisible({ timeout: 25_000 })
  await expect(copyAll).toBeEnabled()
  await copyAll.click({ force: true })

  // The invariant that was broken: the toaster mounts and renders the toast.
  await expect(page.locator('[data-sonner-toast]').first()).toBeVisible({ timeout: 10_000 })
  await expect(page.getByText(/Copied \d+ log lines|Failed to copy logs/i).first()).toBeVisible()
})
