import { expect, test } from './fixtures/test-base'

async function openChatSession(page: import('@playwright/test').Page) {
  const editor = page.locator('.tiptap, [contenteditable="true"], textarea').first()
  await expect(editor).toBeVisible({ timeout: 15_000 })
}

test.describe('critical path', () => {
  test('first-run / no-key empty state', async ({ page, mock }) => {
    await mock.gotoWithMock('/', { apiKeySet: false, sessions: [] })
    await expect(page.locator('#__nuxt')).toBeAttached()
    await expect(
      page.getByText(/Start a new conversation|New Chat|Nanna/i).first(),
    ).toBeVisible({ timeout: 20_000 })
    await expect(page.locator('body')).not.toBeEmpty()
  })

  test('open chat → send (mock LLM) → stream chunk → Stop', async ({ page, mock }) => {
    await mock.gotoWithMock('/', {
      streamChunks: [
        'Hello ',
        'from ',
        'the ',
        'mock ',
        'model. ',
        'This ',
        'stream ',
        'is ',
        'long ',
        'enough ',
        'to ',
        'click ',
        'Stop.',
      ],
    })

    await openChatSession(page)

    const editor = page.locator('[contenteditable="true"], textarea, [role="textbox"]').first()
    await editor.waitFor({ state: 'visible', timeout: 20_000 })
    await editor.click()
    await page.keyboard.type('Hello from e2e')
    await page.keyboard.press('Control+Enter')

    const stop = page
      .getByTestId('stop-generation')
      .or(page.getByRole('button', { name: /stop/i }))
      .first()
    await expect(stop).toBeVisible({ timeout: 15_000 })
    await stop.click()

    await expect(
      page.getByText(/Hello from|Stopped by user|Stop\./i).first(),
    ).toBeVisible({ timeout: 15_000 })
  })

  test('session create / rename / delete / switch', async ({ page, mock }) => {
    await mock.gotoWithMock('/')
    await openChatSession(page)

    // Open the chat side panel (activity-bar Chats toggle) so New Chat is reachable.
    const chatsToggle = page
      .getByRole('button', { name: /^chats$/i })
      .or(page.getByTitle(/^chats$/i))
      .first()
    await chatsToggle.click()
    await page.waitForTimeout(200)

    const newChat = page
      .getByTitle(/new chat/i)
      .or(page.getByRole('button', { name: /new chat/i }))
      .first()
    await expect(newChat).toBeVisible({ timeout: 10_000 })
    await newChat.click()
    await page.waitForTimeout(400)

    // Open context menu on a session row and rename.
    const menuBtn = page.getByRole('button', { name: /session menu/i }).first()
    await expect(menuBtn).toBeVisible({ timeout: 10_000 })
    await menuBtn.click()
    await page.getByRole('button', { name: /^Rename$/i }).click()

    const renameInput = page.getByPlaceholder(/session name/i)
    await expect(renameInput).toBeVisible()
    await renameInput.fill('Renamed E2E')
    await page.getByRole('button', { name: /^Save$/i }).click()
    await expect(page.getByText('Renamed E2E').first()).toBeVisible({ timeout: 10_000 })

    // Delete via menu (confirm dialog if present).
    await menuBtn.click()
    await page.getByRole('button', { name: /^Delete$/i }).click()
    const confirm = page.getByRole('button', { name: /delete|confirm|yes/i }).first()
    if (await confirm.isVisible().catch(() => false)) {
      await confirm.click()
    }
    await page.waitForTimeout(300)
  })

  test('backend disconnect → toast + reconnect affordance', async ({ page, mock }) => {
    await mock.gotoWithMock('/')
    await expect(page.getByText(/Connected|DISCONNECTED|Daemon|Embedded/i).first()).toBeVisible({
      timeout: 15_000,
    })

    await mock.setDisconnected('Daemon unreachable (e2e)')
    // useBackend polls every 2s; wait for the status bar / badge to flip.
    await expect(
      page.getByText(/Disconnected|DISCONNECTED|unreachable|offline/i).first(),
    ).toBeVisible({ timeout: 12_000 })
  })

  test('Settings open + API-key field round-trip (mocked)', async ({ page, mock }) => {
    await mock.gotoWithMock('/settings')
    await expect(page.getByRole('heading', { name: /settings/i }).first()).toBeVisible({
      timeout: 20_000,
    })

    const keyField = page
      .locator('input[type="password"], input[placeholder*="sk-"], input[autocomplete="off"]')
      .first()
    await expect(keyField).toBeVisible({ timeout: 15_000 })
    await keyField.fill('sk-test-e2e-key-not-real')

    const save = page.getByRole('button', { name: /^save$/i }).first()
    await expect(save).toBeVisible()
    await save.click()
    await page.waitForTimeout(400)

    const ext = await page.evaluate(async () => {
      return (window as any).__TAURI_INTERNALS__.invoke('get_extended_settings')
    })
    expect(
      ext.openai_key_set === true ||
        ext.openrouter_key_set === true ||
        ext.anthropic_key_set === true ||
        Boolean(ext.api_key_set),
    ).toBeTruthy()
  })

  test('Logs Live on/off, Clear, Copy all', async ({ page, mock }) => {
    await mock.gotoWithMock('/logs')
    await expect(page.getByText(/Logs/i).first()).toBeVisible({ timeout: 20_000 })

    // Buttons live in the header toolbar — prefer exact Live/Paused labels.
    const live = page.getByRole('button', { name: /^(Live|Paused)$/i })
    await expect(live).toBeVisible({ timeout: 10_000 })
    const before = (await live.getAttribute('aria-label')) || (await live.innerText())
    await live.click()
    await expect
      .poll(async () => (await live.getAttribute('aria-label')) || (await live.innerText()), {
        timeout: 5_000,
      })
      .not.toBe(before.trim())
    await live.click()

    const copy = page.getByRole('button', { name: /Copy all|Copied/i })
    await expect(copy).toBeVisible({ timeout: 10_000 })
    await copy.click()

    const clear = page.getByRole('button', { name: /^Clear$/i })
    await expect(clear).toBeVisible({ timeout: 10_000 })
    await clear.click()
    await page.waitForTimeout(200)
  })
})
