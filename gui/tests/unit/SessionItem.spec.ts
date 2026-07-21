import { mount } from '@vue/test-utils'
import SessionItem from '~/components/SessionItem.vue'

const invoke = vi.fn()
const confirm = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('~/composables/useConfirm', () => ({ useConfirm: () => ({ confirm }) }))
vi.mock('~/composables/useSplatter', () => ({ useSplatter: () => ({ splatterBg: '', onEnter: vi.fn(), onLeave: vi.fn() }) }))

const session = { id: 'session-1', name: 'Moon notes', created_at: '2026-01-01T00:00:00Z', updated_at: new Date().toISOString(), message_count: 4, workspace_id: null, workspace_name: null }
const mountItem = () => mount(SessionItem, { props: { session, isActive: false }, global: { stubs: { SessionActivityBadge: true } } })
const openMenu = async (wrapper: ReturnType<typeof mountItem>) => wrapper.get('.session-menu-btn').trigger('click')

describe('SessionItem', () => {
  beforeEach(() => { invoke.mockReset(); confirm.mockReset() })

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
})
