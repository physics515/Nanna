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
