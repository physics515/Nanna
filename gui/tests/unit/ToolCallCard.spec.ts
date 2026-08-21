import { mount } from '@vue/test-utils'
import ToolCallCard from '~/components/ToolCallCard.vue'

vi.mock('~/composables/useSplatter', () => ({
  useSplatter: () => ({ splatterBg: '', onEnter: vi.fn(), onLeave: vi.fn() }),
}))

/**
 * P22 Tier 4 breaker replays reach the GUI marked `short_circuited`: the
 * harness answered the call and the tool never ran. They report success=false
 * only because there is no tool result. Rendering them with the red failure
 * chip is what made ordinary steering read as a wall of breakage.
 */
const call = {
  id: 'toolu_00000001',
  name: 'read_file',
  input: { file_path: 'notes.md' },
  output: 'This identical call already succeeded twice with byte-identical results.',
  success: false,
  duration_ms: 0,
  data: { short_circuited: true },
}

function mountCard(status: 'completed' | 'error' | 'steering') {
  return mount(ToolCallCard, { props: { toolCall: call, status } })
}

describe('ToolCallCard', () => {
  it('marks a steering replay as steering, not an error', () => {
    const wrapper = mountCard('steering')
    expect(wrapper.classes()).toContain('tool-card--steering')
    expect(wrapper.classes()).not.toContain('tool-card--error')
    expect(wrapper.find('.tool-status--error').exists()).toBe(false)
    expect(wrapper.get('.tool-status').text()).toBe('⇄')
    expect(wrapper.text()).toContain('steering')
  })

  it('explains what a steering replay means once expanded', async () => {
    const wrapper = mountCard('steering')
    await wrapper.get('.tool-header').trigger('click')
    const text = wrapper.text()
    expect(text).toContain('answered by the harness, not executed')
    expect(text).toContain('nothing changed on disk')
    // The notice body is not styled as error output.
    expect(wrapper.find('.tool-code--error').exists()).toBe(false)
  })

  it('still renders a genuine failure as an error', () => {
    const wrapper = mountCard('error')
    expect(wrapper.classes()).toContain('tool-card--error')
    expect(wrapper.get('.tool-status').text()).toBe('✗')
  })
})

/**
 * P24.19: the daemon replaces a write call's bytes with a placeholder before
 * storing the record. Until today that placeholder asserted the bytes had
 * landed whatever happened, so a card marked failed showed an Input claiming
 * success next to an Output reading "WRITE HELD — nothing was written". The
 * source is fixed, but every record already persisted carries the old wording
 * and this card is what re-reads them.
 */
describe('ToolCallCard write placeholders', () => {
  const heldWrite = (placeholder: string) => ({
    id: 'toolu_00000002',
    name: 'write_file',
    input: { file_path: 'ROADMAP.md', content: placeholder },
    output: 'WRITE HELD — nothing was written and nothing is lost. You are shrinking ROADMAP.md',
    success: false,
    duration_ms: 12,
  })

  const expandedInput = async (
    toolCall: ReturnType<typeof heldWrite>,
    status: 'completed' | 'error' | 'steering',
  ) => {
    const wrapper = mount(ToolCallCard, { props: { toolCall, status } })
    await wrapper.get('.tool-header').trigger('click')
    return wrapper.findAll('.tool-code')[0]!.text()
  }

  // The wording persisted before P24.19.
  const OLD_CLAIM = '[content omitted from context — 3718 bytes were written successfully to disk]'
  // The wording a success writes today.
  const NEW_CLAIM = '[content omitted from context — 3718 bytes were written to disk]'

  it.each([
    ['the pre-P24.19 wording', OLD_CLAIM],
    ['the current success wording', NEW_CLAIM],
  ])('does not repeat %s on a failed card', async (_label, placeholder) => {
    const input = await expandedInput(heldWrite(placeholder), 'error')
    expect(input).not.toContain('were written')
    expect(input).toContain('3718 bytes were sent to this tool')
    expect(input).toContain('authoritative record of what happened on disk')
  })

  // A breaker replay never reached the tool either, so the same rule holds.
  it('does not repeat the claim on a steering replay', async () => {
    const input = await expandedInput(heldWrite(OLD_CLAIM), 'steering')
    expect(input).not.toContain('were written')
  })

  // The byte count is a fact about the REQUEST and survives either way.
  it('leaves the placeholder alone on a call that actually succeeded', async () => {
    const call = { ...heldWrite(NEW_CLAIM), success: true, output: 'Wrote 3718 bytes' }
    expect(await expandedInput(call, 'completed')).toContain('3718 bytes were written to disk')
  })

  // Only the placeholder may match — never a file whose real content happens
  // to talk about writing.
  it('never rewrites real content that merely mentions a write', async () => {
    const call = heldWrite('The build step reports how many bytes were written to disk.')
    const input = await expandedInput(call, 'error')
    expect(input).toContain('The build step reports how many bytes were written to disk.')
  })

  // The green "Written Content" pane claims the bytes are on disk.
  it('does not show written content for a write that did not land', async () => {
    const call = { ...heldWrite(OLD_CLAIM), data: { written: '# ROADMAP\n' } }
    const wrapper = mount(ToolCallCard, { props: { toolCall: call, status: 'error' } })
    await wrapper.get('.tool-header').trigger('click')
    expect(wrapper.find('.tool-code--written').exists()).toBe(false)
  })

  it('still shows written content for a write that landed', async () => {
    const call = { ...heldWrite(NEW_CLAIM), success: true, data: { written: '# ROADMAP\n' } }
    const wrapper = mount(ToolCallCard, { props: { toolCall: call, status: 'completed' } })
    await wrapper.get('.tool-header').trigger('click')
    expect(wrapper.get('.tool-code--written').text()).toContain('# ROADMAP')
  })
})
