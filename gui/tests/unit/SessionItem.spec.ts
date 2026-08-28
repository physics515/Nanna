import { mount } from '@vue/test-utils'
import SessionItem from '~/components/SessionItem.vue'
import { getSessionStateMap, seedChatModel, useSessionState } from '~/composables/useSessionState'
import { ref } from 'vue'

const invoke = vi.fn()
const confirm = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('~/composables/useConfirm', () => ({ useConfirm: () => ({ confirm }) }))
vi.mock('~/composables/useSplatter', () => ({ useSplatter: () => ({ splatterBg: '', onEnter: vi.fn(), onLeave: vi.fn() }) }))

const session = { id: 'session-1', name: 'Moon notes', created_at: '2026-01-01T00:00:00Z', updated_at: new Date().toISOString(), message_count: 4, workspace_id: null, workspace_name: null }
const mountItem = (overrides: Partial<typeof session> & { chat_model?: string | null } = {}) =>
  mount(SessionItem, { props: { session: { ...session, ...overrides }, isActive: false }, global: { stubs: { SessionActivityBadge: true } } })
const openMenu = async (wrapper: ReturnType<typeof mountItem>) => wrapper.get('.session-menu-btn').trigger('click')

describe('SessionItem', () => {
  beforeEach(() => { invoke.mockReset(); confirm.mockReset(); getSessionStateMap().clear() })

  it('selects the session from the main row', async () => {
    const wrapper = mountItem(); await wrapper.get('.session-btn').trigger('click')
    expect(wrapper.emitted('select')?.[0]).toEqual([session])
  })

  it('renames through the daemon and emits the changed session', async () => {
    invoke.mockResolvedValue(undefined)
    const wrapper = mountItem(); await openMenu(wrapper); await wrapper.get('.ctx-item').trigger('click')
    const input = wrapper.get('input'); await input.setValue('Night journal'); await input.trigger('keydown.enter')
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('rename_session', { sessionId: 'session-1', name: 'Night journal' }))
    expect(wrapper.emitted('renamed')?.[0]?.[0]).toMatchObject({ id: 'session-1', name: 'Night journal' })
  })

  it('does not invoke rename for blank input', async () => {
    const wrapper = mountItem(); await openMenu(wrapper); await wrapper.get('.ctx-item').trigger('click')
    const input = wrapper.get('input'); await input.setValue('   '); await input.trigger('keydown.enter')
    expect(invoke).not.toHaveBeenCalled()
  })

  it('deletes only after confirmation', async () => {
    confirm.mockResolvedValue(true); invoke.mockResolvedValue(undefined)
    const wrapper = mountItem(); await openMenu(wrapper); await wrapper.findAll('.ctx-item')[1].trigger('click')
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('delete_session', { sessionId: 'session-1' }))
    expect(wrapper.emitted('deleted')?.[0]).toEqual(['session-1'])
  })

  /**
   * The row's `session` comes from a list the layout reloads only at mount, on
   * `sessions-cleared`, and on a workspace-tab change — so on its own the chip
   * outlives the pin it names. The chat header's picker is what changes a pin,
   * and this is what makes the sidebar agree with it.
   */
  describe('the pin chip', () => {
    const chip = (wrapper: ReturnType<typeof mountItem>) => wrapper.find('.session-model')

    it('names the pin the session list carries', () => {
      expect(chip(mountItem({ chat_model: 'ollama/qwen3:14b' })).text()).toBe('qwen3:14b')
    })

    it('drops the chip once the pin is removed', async () => {
      const wrapper = mountItem({ chat_model: 'ollama/qwen3:14b' })
      useSessionState(ref('session-1')).chatModel.value = null
      await wrapper.vm.$nextTick()
      expect(chip(wrapper).exists()).toBe(false)
    })

    it('names a pin added after the list was loaded', async () => {
      const wrapper = mountItem()
      useSessionState(ref('session-1')).chatModel.value = 'claude-opus-4-5-20251101'
      await wrapper.vm.$nextTick()
      expect(chip(wrapper).text()).toBe('Opus 4.5')
    })

    // A stream event for a chat this window never opened creates state for it.
    // That entry has never been told anything about the pin, and must not be
    // mistaken for "this chat has none".
    it('keeps the listed pin when state exists but was never told the pin', async () => {
      const wrapper = mountItem({ chat_model: 'ollama/qwen3:14b' })
      useSessionState(ref('session-1')).isStreaming.value = true
      await wrapper.vm.$nextTick()
      expect(chip(wrapper).text()).toBe('qwen3:14b')
    })

    // The same staleness the other way round: a pin set in another window
    // arrives with the next list reload, and the daemon's answer wins.
    it('takes the daemon answer back when the session list reloads', async () => {
      const wrapper = mountItem({ chat_model: 'ollama/qwen3:14b' })
      useSessionState(ref('session-1')).chatModel.value = null
      await wrapper.vm.$nextTick()

      seedChatModel('session-1', 'claude-opus-4-5-20251101')
      await wrapper.vm.$nextTick()

      expect(chip(wrapper).text()).toBe('Opus 4.5')
    })

    // The seed runs over every listed session on every reload, so it must not
    // mint state for chats this window has never touched.
    it('records nothing for an unpinned chat with no state of its own', () => {
      seedChatModel('session-never-opened', null)
      expect(getSessionStateMap().has('session-never-opened')).toBe(false)
    })
  })
})
