import { mount } from '@vue/test-utils'
import FileHistoryButton from '~/components/FileHistoryButton.vue'
import { fileName, parseFileHistory, restoreEffect } from '~/lib/fileHistory'

/**
 * Restoring a file is the one destructive click in the chat view, so the two
 * things pinned here are that a row means exactly what the daemon said, and
 * that Restore sends the checkpoint of the row that was clicked — for the chat
 * that is open — only after confirmation.
 */
const invoke = vi.fn()
const confirm = vi.fn()
const toastSuccess = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('~/composables/useConfirm', () => ({ useConfirm: () => ({ confirm: (...a: unknown[]) => confirm(...a) }) }))
vi.mock('~/composables/useToast', () => ({
  useToast: () => ({ success: toastSuccess, error: vi.fn(), info: vi.fn(), warning: vi.fn(), show: vi.fn() }),
}))

/** Shape captured from `session.file_history` on a live daemon. */
const DAEMON = {
  checkpoints: [
    { checkpoint: 2, path: '/w/plan.md', existed: true, bytes: 62, taken_at: '2026-09-17T11:35:22Z', baseline: false },
    { checkpoint: 0, path: '/w/plan.md', existed: false, bytes: 0, taken_at: '2026-09-17T11:35:21Z', baseline: true },
  ],
  total: 3,
}

describe('parseFileHistory', () => {
  it('reads the daemon shape', () => {
    const parsed = parseFileHistory(DAEMON)
    expect(parsed.total).toBe(3)
    expect(parsed.checkpoints[0]).toEqual({
      checkpoint: 2, path: '/w/plan.md', existed: true, bytes: 62, takenAt: '2026-09-17T11:35:22Z', baseline: false,
    })
  })

  it('drops malformed rows and never throws', () => {
    expect(parseFileHistory(null)).toEqual({ checkpoints: [], total: 0 })
    const parsed = parseFileHistory({
      checkpoints: [
        { checkpoint: -1, path: '/a', existed: true, bytes: 1, taken_at: 't' },
        { checkpoint: 1, path: '', existed: true, bytes: 1, taken_at: 't' },
        { checkpoint: 1, path: '/a', existed: 'yes', bytes: 1, taken_at: 't' },
        { checkpoint: 4, path: '/ok', existed: true, bytes: 1, taken_at: 't' },
      ],
      total: 'many',
    })
    expect(parsed.checkpoints.map(c => c.checkpoint)).toEqual([4])
    expect(parsed.total).toBe(1)
  })

  it('says what a restore will do', () => {
    const [kept, created] = parseFileHistory(DAEMON).checkpoints
    expect(restoreEffect(kept!)).toBe('put plan.md back to its 62-byte version')
    expect(restoreEffect(created!)).toBe('remove plan.md (a tool created it)')
    expect(fileName('C:\\w\\notes.txt')).toBe('notes.txt')
  })
})

describe('FileHistoryButton', () => {
  beforeEach(() => {
    invoke.mockReset()
    confirm.mockReset()
    toastSuccess.mockReset()
    invoke.mockImplementation(async (command: string) => (command === 'get_file_history' ? DAEMON : { action: 'rewrote' }))
  })

  it('restores the clicked checkpoint of this chat, only after confirming', async () => {
    const wrapper = mount(FileHistoryButton, { props: { sessionId: 'chat-1' } })
    await wrapper.get('button').trigger('click')
    await vi.waitFor(() => expect(wrapper.findAll('li')).toHaveLength(2))
    expect(invoke).toHaveBeenCalledWith('get_file_history', { sessionId: 'chat-1', limit: 50 })
    expect(wrapper.text()).toContain('1 older not shown')

    confirm.mockResolvedValueOnce(false)
    await wrapper.get('li[data-checkpoint="0"] button').trigger('click')
    await vi.waitFor(() => expect(confirm).toHaveBeenCalledTimes(1))
    expect(invoke).not.toHaveBeenCalledWith('restore_file_checkpoint', expect.anything())

    confirm.mockResolvedValueOnce(true)
    await wrapper.get('li[data-checkpoint="0"] button').trigger('click')
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('restore_file_checkpoint', { sessionId: 'chat-1', checkpoint: 0 }),
    )
    await vi.waitFor(() => expect(toastSuccess).toHaveBeenCalledWith('Restored plan.md'))
  })
})
