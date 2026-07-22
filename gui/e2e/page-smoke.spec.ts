import { expect, test } from './fixtures/test-base'

/**
 * Page smoke matrix — each route loads primary content (no white/blank shell).
 * Prefer semantic landmarks; fall back to distinctive copy if headings lag client-side load.
 */
const PAGES: Array<{ path: string; hit: RegExp }> = [
  { path: '/', hit: /Start a new conversation|Welcome|Nanna|type a message/i },
  { path: '/agents', hit: /Agent Overview|Agents|No agents/i },
  { path: '/channels', hit: /Channels|Telegram|Discord|Slack/i },
  { path: '/memory', hit: /Memory|No memories|memories/i },
  { path: '/model-stats', hit: /Model Stats|No model|tokens/i },
  { path: '/scheduler', hit: /Scheduler|No tasks|cron|heartbeat/i },
  { path: '/settings', hit: /Settings|Models|Providers/i },
  { path: '/tool-stats', hit: /Tool Stats|No tool|calls/i },
  { path: '/tools', hit: /Tools|skills|No tools/i },
  { path: '/workspaces', hit: /Workspaces|No workspace|Open Workspace/i },
  { path: '/logs', hit: /Logs|Live|Copy/i },
  { path: '/tasks', hit: /Tasks|No tasks|goal/i },
]

for (const { path, hit } of PAGES) {
  test(`smoke: ${path}`, async ({ page, mock }) => {
    await mock.gotoWithMock(path)
    await expect(page.locator('#__nuxt')).toBeAttached({ timeout: 20_000 })
    await expect(page.getByText(hit).first()).toBeVisible({ timeout: 25_000 })
    const textLen = await page.locator('body').innerText().then((t) => t.trim().length)
    expect(textLen).toBeGreaterThan(20)
  })
}
