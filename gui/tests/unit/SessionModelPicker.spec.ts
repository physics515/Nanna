import { mount } from '@vue/test-utils'
import SessionModelPicker from '~/components/SessionModelPicker.vue'
import { getSessionStateMap, seedChatModel } from '~/composables/useSessionState'

/**
 * The pin is per CHAT, so the two failures worth guarding are both about
 * addressing: sending the change for a session other than the one on screen,
 * and showing one session's pin while another session is open. The picker
 * writes optimistically to module-global state, which is exactly where a
 * cross-session leak would hide.
 */

const invoke = vi.fn()
const toastError = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('~/composables/useToast', () => ({
  useToast: () => ({ error: toastError, success: vi.fn(), info: vi.fn(), warning: vi.fn(), show: vi.fn(), copy: vi.fn() }),
}))

/** What Settings → Chat Models has vetted; the picker offers exactly these. */
const CHAT_MODELS = ['ollama/qwen3:14b', 'claude-opus-4-5-20251101']

const mountPicker = async (sessionId: string) => {
  const wrapper = mount(SessionModelPicker, { props: { sessionId } })
  await vi.waitFor(() => expect(wrapper.findAll('option').length).toBe(CHAT_MODELS.length + 1))
  return wrapper
}

const selectEl = (wrapper: Awaited<ReturnType<typeof mountPicker>>) =>
  wrapper.get('select').element as HTMLSelectElement

/**
 * Let a rejected `invoke` reach its `catch` and the pill re-render. Asserting
 * that NOTHING was written needs this — with no state change to wait for,
 * `waitFor` would pass before the handler had a chance to write anything.
 */
const flushRejection = async (wrapper: Awaited<ReturnType<typeof mountPicker>>) => {
  await Promise.resolve()
  await wrapper.vm.$nextTick()
}

describe('SessionModelPicker', () => {
  beforeEach(() => {
    getSessionStateMap().clear()
    toastError.mockReset()
    invoke.mockReset()
    invoke.mockImplementation(async (command: string) => {
      if (command === 'get_chat_model_priority') return CHAT_MODELS
      return undefined
    })
  })

  it('offers the global default plus the vetted chat models, nothing else', async () => {
    const wrapper = await mountPicker('session-offer')
    expect(wrapper.findAll('option').map(o => o.attributes('value')))
      .toEqual(['', ...CHAT_MODELS])
  })

  it('pins the picked model on the session it is showing', async () => {
    const wrapper = await mountPicker('session-a')
    await wrapper.get('select').setValue('ollama/qwen3:14b')

    expect(invoke).toHaveBeenCalledWith('set_session_model', {
      sessionId: 'session-a',
      model: 'ollama/qwen3:14b',
    })
  })

  it('clears the pin back to the global default', async () => {
    const wrapper = await mountPicker('session-clear')
    await wrapper.get('select').setValue('claude-opus-4-5-20251101')
    invoke.mockClear()

    await wrapper.get('select').setValue('')

    expect(invoke).toHaveBeenCalledWith('set_session_model', { sessionId: 'session-clear', model: null })
  })

  it('shows each session its own selection when the chat switches', async () => {
    const wrapper = await mountPicker('session-pinned')
    await wrapper.get('select').setValue('ollama/qwen3:14b')
    expect(selectEl(wrapper).value).toBe('ollama/qwen3:14b')

    await wrapper.setProps({ sessionId: 'session-unpinned' })
    expect(selectEl(wrapper).value).toBe('')

    await wrapper.setProps({ sessionId: 'session-pinned' })
    expect(selectEl(wrapper).value).toBe('ollama/qwen3:14b')
  })

  it('names both the pin and the global default it overrides', async () => {
    const wrapper = await mountPicker('session-title')
    await wrapper.get('select').setValue('claude-opus-4-5-20251101')

    const title = wrapper.get('.chat-model-pill').attributes('title') ?? ''
    expect(title).toContain('pinned to claude-opus-4-5-20251101')
    expect(title).toContain('global default is ollama/qwen3:14b')
  })

  it('keeps naming a pin the chat-model list no longer carries', async () => {
    const wrapper = mount(SessionModelPicker, { props: { sessionId: 'session-stale' } })
    getSessionStateMap().get('session-stale')!.chatModel = 'ollama/retired:7b'
    await vi.waitFor(() => expect(wrapper.findAll('option').length).toBe(CHAT_MODELS.length + 2))

    expect(selectEl(wrapper).value).toBe('ollama/retired:7b')
  })

  it('reverts on the session that was picked, not the one now on screen', async () => {
    const wrapper = await mountPicker('session-inflight')
    let refuse: (reason: Error) => void = () => {}
    invoke.mockImplementationOnce(() => new Promise((_resolve, reject) => { refuse = reject }))

    await wrapper.get('select').setValue('ollama/qwen3:14b')
    await wrapper.setProps({ sessionId: 'session-bystander' })
    refuse(new Error('session_not_found'))

    await vi.waitFor(() => expect(toastError).toHaveBeenCalled())
    expect(getSessionStateMap().get('session-inflight')!.chatModel).toBeNull()
    expect(getSessionStateMap().get('session-bystander')!.chatModel).toBeNull()
  })

  it('puts the old model back when the daemon refuses the change', async () => {
    const wrapper = await mountPicker('session-reject')
    invoke.mockRejectedValueOnce(new Error('session_not_found'))

    await wrapper.get('select').setValue('ollama/qwen3:14b')

    await vi.waitFor(() => expect(toastError).toHaveBeenCalled())
    expect(selectEl(wrapper).value).toBe('')
  })

  /**
   * The revert is the only thing in the picker that writes a value the user
   * did not just pick, so it is the only thing that can leave the header
   * naming a model the chat is not using. It may write only while it is still
   * the newest word on the subject.
   */
  it('lets a newer successful pin stand when an older request fails late', async () => {
    const wrapper = await mountPicker('session-race')
    let refuse: (reason: Error) => void = () => {}
    invoke.mockImplementationOnce(() => new Promise((_resolve, reject) => { refuse = reject }))

    await wrapper.get('select').setValue('ollama/qwen3:14b')
    await wrapper.get('select').setValue('claude-opus-4-5-20251101')
    refuse(new Error('timed out'))
    await flushRejection(wrapper)

    expect(selectEl(wrapper).value).toBe('claude-opus-4-5-20251101')
    expect(toastError).not.toHaveBeenCalled()
  })

  // Reverting to the value on screen is not good enough while an earlier
  // request is still out: that value is itself a guess. The pin goes back to
  // what the daemon last agreed to.
  it('reverts past an unconfirmed pick to the last model the daemon agreed to', async () => {
    const wrapper = await mountPicker('session-double')
    let refuseFirst: (reason: Error) => void = () => {}
    let refuseSecond: (reason: Error) => void = () => {}
    invoke.mockImplementationOnce(() => new Promise((_r, reject) => { refuseFirst = reject }))
    invoke.mockImplementationOnce(() => new Promise((_r, reject) => { refuseSecond = reject }))

    await wrapper.get('select').setValue('ollama/qwen3:14b')
    await wrapper.get('select').setValue('claude-opus-4-5-20251101')
    refuseFirst(new Error('timed out'))
    refuseSecond(new Error('timed out'))

    await vi.waitFor(() => expect(toastError).toHaveBeenCalledTimes(1))
    expect(selectEl(wrapper).value).toBe('')
  })

  /**
   * The mirror of the case above, and the half the ticket guard never covered:
   * an ACCEPTED change is a fact about the daemon, so a refusal that arrives
   * afterwards must not revert past it. The daemon applies requests in the
   * order they were sent and a refusal changes nothing there, so what it holds
   * after the burst is the newest request that succeeded — not whichever
   * request happened to settle last.
   */
  it('keeps a pin the daemon accepted when a later change is refused', async () => {
    const wrapper = await mountPicker('session-accepted')
    let accept: () => void = () => {}
    invoke.mockImplementationOnce(() => new Promise<void>((resolve) => { accept = () => resolve() }))
    invoke.mockRejectedValueOnce(new Error('session_not_found'))

    await wrapper.get('select').setValue('ollama/qwen3:14b')
    await wrapper.get('select').setValue('claude-opus-4-5-20251101')
    await vi.waitFor(() => expect(toastError).toHaveBeenCalledTimes(1))

    accept()

    await vi.waitFor(() => expect(selectEl(wrapper).value).toBe('ollama/qwen3:14b'))
  })

  /**
   * A session LIST is the daemon's answer as of when it was fetched. The
   * layout reloads it on workspace-tab changes and on `sessions-cleared`, and
   * a reload that overlaps an unanswered pin change cannot carry that change —
   * seeding from it put the superseded model back on screen AND marked it
   * known, so both the row and the header asserted a pin the user had just
   * replaced.
   */
  it('ignores a session list that predates an unanswered pin change', async () => {
    const wrapper = await mountPicker('session-seed')
    let accept: () => void = () => {}
    invoke.mockImplementationOnce(() => new Promise<void>((resolve) => { accept = () => resolve() }))

    await wrapper.get('select').setValue('ollama/qwen3:14b')
    // The layout's `loadSessions()` lands mid-flight with the pre-change list.
    seedChatModel('session-seed', null)
    await wrapper.vm.$nextTick()

    expect(selectEl(wrapper).value).toBe('ollama/qwen3:14b')

    // Once the request is answered the list is fair game again.
    accept()
    await vi.waitFor(() => expect(selectEl(wrapper).value).toBe('ollama/qwen3:14b'))
    seedChatModel('session-seed', 'claude-opus-4-5-20251101')
    await wrapper.vm.$nextTick()
    expect(selectEl(wrapper).value).toBe('claude-opus-4-5-20251101')
  })

  describe('a pin changed while a turn is running', () => {
    const startTurn = (sessionId: string) => {
      getSessionStateMap().get(sessionId)!.isStreaming = true
    }

    it('says the change lands on the next message', async () => {
      const wrapper = await mountPicker('session-midturn')
      startTurn('session-midturn')

      await wrapper.get('select').setValue('ollama/qwen3:14b')

      expect(wrapper.get('.chat-model-pill__pending').text()).toBe('next turn')
      expect(wrapper.get('.chat-model-pill').attributes('title'))
        .toContain('Takes effect on your next message')
    })

    it('stops saying so once that turn ends', async () => {
      const wrapper = await mountPicker('session-turnend')
      startTurn('session-turnend')
      await wrapper.get('select').setValue('ollama/qwen3:14b')

      getSessionStateMap().get('session-turnend')!.isStreaming = false
      await wrapper.vm.$nextTick()

      expect(wrapper.find('.chat-model-pill__pending').exists()).toBe(false)
    })

    it('says nothing of the sort when the chat is idle', async () => {
      const wrapper = await mountPicker('session-idle')
      await wrapper.get('select').setValue('ollama/qwen3:14b')

      expect(wrapper.find('.chat-model-pill__pending').exists()).toBe(false)
    })

    // A refused change never reached the daemon, so there is nothing waiting
    // on the running turn either.
    it('drops the note when the daemon refuses the change', async () => {
      const wrapper = await mountPicker('session-midturn-reject')
      startTurn('session-midturn-reject')
      invoke.mockRejectedValueOnce(new Error('session_not_found'))

      await wrapper.get('select').setValue('ollama/qwen3:14b')

      await vi.waitFor(() => expect(toastError).toHaveBeenCalled())
      expect(wrapper.find('.chat-model-pill__pending').exists()).toBe(false)
    })
  })
})
