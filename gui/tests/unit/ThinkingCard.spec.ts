import { mount } from '@vue/test-utils'
import ThinkingCard from '~/components/ThinkingCard.vue'

/**
 * The header's word count must agree with the gate that decides whether the
 * card renders at all: splitting an empty string yields [''], so a burst of
 * pure whitespace counted as "1 words" — the label on the empty cards seen
 * live 2026-08-03. Zero words means nothing to show.
 */

// The splatter background drives a rAF loop off Nuxt auto-imports; this spec
// is about the count, so give the component an inert background.
vi.mock('~/composables/useSplatter', async () => {
  const { ref } = await import('vue')
  return {
    useSplatter: () => ({ splatterBg: ref(''), onEnter: () => {}, onLeave: () => {} }),
  }
})

function count(content: string): string {
  return mount(ThinkingCard, { props: { content } }).text()
}

describe('ThinkingCard word count', () => {
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
})
