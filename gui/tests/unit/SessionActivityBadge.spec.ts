import { mount } from '@vue/test-utils'
import SessionActivityBadge from '~/components/SessionActivityBadge.vue'
import { getSessionStateMap, useSessionState } from '~/composables/useSessionState'
import { ref } from 'vue'

/**
 * The badge's four labels are all inferences from latched local flags:
 * `isStreaming` latches on the first text chunk and clears only when the whole
 * run ends, so a turn that streamed once and then went silent kept pulsing
 * "Streaming..." for as long as the silence lasted. A wedged turn and a
 * working one were pixel-identical.
 *
 * The daemon's liveness beat is the one OBSERVED statement about a running
 * turn, so it has to qualify every branch — not just the idle one.
 */
const SESSION = 'session-badge'

function startTurn() {
  const state = useSessionState(ref(SESSION))
  state.isLoading.value = true
  return state
}

function beat(awaiting: string, quietS: number | null = 41) {
  useSessionState(ref(SESSION)).liveness.value = {
    elapsedS: 300,
    quietS,
    phase: 'streaming',
    awaiting,
    beat: 10,
  }
}

const mountBadge = (compact = false) =>
  mount(SessionActivityBadge, { props: { sessionId: SESSION, compact } })

describe('SessionActivityBadge', () => {
  beforeEach(() => {
    getSessionStateMap().clear()
  })

  it('says nothing at all when the session has no active work', () => {
    const wrapper = mountBadge()
    expect(wrapper.find('span').exists()).toBe(false)
  })

  it('still names the activity when no beat has arrived', () => {
    startTurn()
    expect(mountBadge().text()).toContain('Thinking...')
  })

  /**
   * One case per branch: the beat has to reach the tool branch, the streaming
   * branch, the thinking branch and the catch-all, because a turn can wedge in
   * any of them and the badge looked identical in all four.
   */
  it('qualifies the running-tool branch with what the turn is waiting on', () => {
    const state = startTurn()
    state.activeToolCalls.value = [{
      id: 'c1', name: 'exec', input: {}, output: '', success: false,
      duration_ms: 0, status: 'started',
    }]
    beat('tool exec: no output for 41s')

    const text = mountBadge().text()
    expect(text).toContain('Running exec...')
    expect(text).toContain('no output for 41s')
  })

  it('qualifies the streaming branch', () => {
    const state = startTurn()
    state.isStreaming.value = true
    beat('model output (ollama/qwen3.5:9b): last token 41s ago')

    const text = mountBadge().text()
    expect(text).toContain('Streaming...')
    expect(text).toContain('last token 41s ago')
  })

  it('qualifies the thinking branch', () => {
    startTurn()
    beat('model output (ollama/qwen3.5:9b): last token 41s ago')

    const text = mountBadge().text()
    expect(text).toContain('Thinking...')
    expect(text).toContain('last token 41s ago')
  })

  it('qualifies the catch-all branch', () => {
    const state = useSessionState(ref(SESSION))
    // Neither loading nor streaming, but a tool call is on the books — the
    // "Working..." branch, which had nothing to say at all.
    state.activeToolCalls.value = [{
      id: 'c1', name: 'exec', input: {}, output: '', success: false,
      duration_ms: 0, status: 'completed',
    }]
    beat('planning the next step')

    const text = mountBadge().text()
    expect(text).toContain('Working...')
    expect(text).toContain('planning the next step')
  })

  it('falls back to the quiet time when the beat carries no prose', () => {
    startTurn()
    beat('', 305)
    expect(mountBadge().text()).toContain('305s since last output')
  })

  /** Absent quiet time is "nothing observed to time from yet", not zero. */
  it('says nothing about quiet time when the beat reports none', () => {
    startTurn()
    beat('', null)
    expect(mountBadge().text()).not.toContain('since last output')
  })

  /**
   * The sidebar's dot has no room for words and is exactly where a user asks
   * whether a background chat is still alive.
   */
  it('carries the beat in the hover text even when compact', () => {
    startTurn()
    beat('model output (ollama/qwen3.5:9b): last token 41s ago')

    const title = mountBadge(true).get('span').attributes('title') ?? ''
    expect(title).toContain('last token 41s ago')
    expect(title).toContain('elapsed 300s')
    expect(title).toContain('quiet 41s')
  })

  /** Beats belong to the turn that produced them. */
  it('drops the beat when the turn ends', async () => {
    const state = startTurn()
    state.isStreaming.value = true
    beat('model output: last token 41s ago')
    const wrapper = mountBadge()
    expect(wrapper.text()).toContain('last token 41s ago')

    state.clearStreamingState()
    state.isLoading.value = true
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).not.toContain('last token 41s ago')
  })
})
