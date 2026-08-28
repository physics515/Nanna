import { mount } from '@vue/test-utils'
import NuiThinkingMessage from '~/components/nui/NuiThinkingMessage.vue'

/**
 * Ported from ThinkingCard.spec.ts when the chat moved to the nui design.
 *
 * The header's word count must agree with the gate that decides whether the
 * card renders at all: splitting an empty string yields [''], so a burst of
 * pure whitespace counted as "1 words" — the label on the empty cards seen
 * live 2026-08-03. Zero words means nothing to show.
 */

const stubs = {
  NuiMessage: {
    props: ['role', 'accent', 'author'],
    template: '<div><slot name="header" /><slot /></div>',
  },
  NuiCollapseTrigger: {
    props: ['label', 'meta', 'modelValue'],
    template: '<button class="trigger">{{ label }} {{ meta }}</button>',
  },
  NuiIcon: { props: ['name', 'size'], template: '<span />' },
}

function count(content: string): string {
  return mount(NuiThinkingMessage, { props: { content }, global: { stubs } }).text()
}

describe('NuiThinkingMessage word count', () => {
  it('shows no count for whitespace-only or marker-only content', () => {
    expect(count('   \n\t ')).not.toContain('words')
    expect(count('')).not.toContain('words')
    expect(count('TASK COMPLETE\n')).not.toContain('words')
  })

  it('counts the tokens of real reasoning', () => {
    expect(count('  three whole words  ')).toContain('3 words')
  })

  it('counts a lone punctuation token as the one word it is', () => {
    // Content the gate lets through must have a non-zero count, or the card
    // would render with no label at all.
    expect(count('.')).toContain('1 words')
  })

  it('shows no count while the burst is still streaming', () => {
    const wrapper = mount(NuiThinkingMessage, {
      props: { content: 'still reasoning through it', active: true },
      global: { stubs },
    })
    expect(wrapper.text()).not.toContain('words')
  })
})
