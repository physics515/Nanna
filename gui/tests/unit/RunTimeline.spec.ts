import { mount } from '@vue/test-utils'
import RunTimeline from '~/components/RunTimeline.vue'
import type { TimelineEntry } from '~/composables/useSessionState'

/**
 * Long-horizon runs interleave many tool calls with occasional prose. Models
 * emit stray newline deltas and bare `TASK COMPLETE` marker lines between
 * tool calls; each opens its own segment (the previous journal item is a
 * tool), and those segments used to render as empty "☽ Nanna" bubbles
 * (observed live 2026-07-30, qwen3.5:9b) and empty "💭 Thinking · 1 words"
 * cards (observed live 2026-08-03). Text and thinking are gated the same
 * way: render only with visible content after stripping harness markers.
 */

const stubs = {
  ThinkingCard: {
    props: ['content', 'isActive'],
    template: '<div data-testid="thinking" />',
  },
  ToolCallCard: {
    props: ['toolCall', 'status', 'timestamp', 'tokens', 'totalTokens'],
    template: '<div data-testid="tool" />',
  },
  MessageBubble: {
    props: ['variant'],
    template: '<div data-testid="bubble"><slot /></div>',
  },
  UiAvatar: { template: '<span />' },
  MarkdownContent: {
    props: ['content'],
    template: '<div data-testid="markdown">{{ content }}</div>',
  },
}

function tool(name: string): TimelineEntry {
  return {
    kind: 'tool',
    call_id: 'toolu_00000000',
    name,
    input: null,
    output: 'ok',
    success: true,
    duration_ms: 5,
    at: '2026-07-30T08:10:00Z',
  } as TimelineEntry
}

function text(content: string): TimelineEntry {
  return { kind: 'text', content, at: '2026-07-30T08:10:00Z' } as TimelineEntry
}

function thinking(content: string): TimelineEntry {
  return { kind: 'thinking', content, at: '2026-07-30T08:10:00Z' } as TimelineEntry
}

function mountTimeline(items: TimelineEntry[], isLive = false) {
  return mount(RunTimeline, { props: { items, isLive }, global: { stubs } })
}

describe('RunTimeline', () => {
  it('renders no bubble for whitespace-only and marker-only text segments', () => {
    const wrapper = mountTimeline([
      tool('exec'),
      text('\n\n'),
      tool('exec'),
      text('TASK COMPLETE\n'),
      tool('read_file'),
    ])
    expect(wrapper.findAll('[data-testid="bubble"]')).toHaveLength(0)
    expect(wrapper.findAll('[data-testid="tool"]')).toHaveLength(3)
  })

  it('renders a bubble for a segment with real prose between tool calls', () => {
    const wrapper = mountTimeline([
      tool('exec'),
      text('Stage 3 passed, moving on.'),
      tool('exec'),
    ])
    const bubbles = wrapper.findAll('[data-testid="bubble"]')
    expect(bubbles).toHaveLength(1)
    expect(bubbles[0]!.text()).toContain('Stage 3 passed, moving on.')
  })

  it('still strips marker lines out of segments that also carry prose', () => {
    const wrapper = mountTimeline([text('All checks green.\nTASK COMPLETE\n')])
    const bubble = wrapper.get('[data-testid="bubble"]')
    expect(bubble.text()).toContain('All checks green.')
    expect(bubble.text()).not.toContain('TASK COMPLETE')
  })

  it('skips the trailing open text segment while live (streaming bubble owns it)', () => {
    const wrapper = mountTimeline([tool('exec'), text('streaming prose so far')], true)
    expect(wrapper.findAll('[data-testid="bubble"]')).toHaveLength(0)
  })

  it('renders no card for whitespace-only and marker-only thinking bursts', () => {
    const wrapper = mountTimeline([
      tool('exec'),
      thinking('\n'),
      tool('exec'),
      thinking('   \t '),
      tool('read_file'),
      thinking('TASK COMPLETE\n'),
    ])
    expect(wrapper.findAll('[data-testid="thinking"]')).toHaveLength(0)
    expect(wrapper.findAll('[data-testid="tool"]')).toHaveLength(3)
  })

  it('renders a card for a thinking burst with real reasoning', () => {
    const wrapper = mountTimeline([
      thinking('The test failed on the missing trailing newline.'),
      tool('write_file'),
    ])
    expect(wrapper.findAll('[data-testid="thinking"]')).toHaveLength(1)
  })
})
