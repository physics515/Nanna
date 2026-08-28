import { mount } from '@vue/test-utils'
import ModelStatusBadge from '~/components/ModelStatusBadge.vue'

/**
 * The badge reports two different kinds of fact, and only one of them can be
 * wrong for a chat pinned to its own model. The active model is process-wide
 * and would name a model that chat is not using; whether the router is on a
 * fallback, and how many models are rate limited, are true of every chat on
 * this daemon including a pinned one. Suppressing the badge to hide the first
 * took the other two with it, which is what these guard.
 */

const invoke = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('~/composables/useGroundGlass', () => ({
  useGroundGlass: () => ({ meshBg: '', containerStyle: {}, onEnter: vi.fn(), onLeave: vi.fn() }),
}))

const HEALTHY = {
  active_model: 'openrouter/anthropic/claude-opus-4-5-20251101',
  fallback_reason: null,
  rate_limited_models: [] as string[],
}
const DEGRADED = {
  active_model: 'ollama/qwen3:14b',
  fallback_reason: 'anthropic returned 529',
  rate_limited_models: ['claude-opus-4-5-20251101', 'gpt-4o'],
}

const mountBadge = async (status: typeof HEALTHY, modelNameSuperseded = false) => {
  invoke.mockResolvedValue(status)
  const wrapper = mount(ModelStatusBadge, { props: { modelNameSuperseded } })
  await vi.waitFor(() => expect(wrapper.get('.model-pill__name').text()).not.toBe('Loading...'))
  return wrapper
}

describe('ModelStatusBadge', () => {
  beforeEach(() => { invoke.mockReset() })

  it('names the active model with the shared label rule', async () => {
    const wrapper = await mountBadge(HEALTHY)
    expect(wrapper.get('.model-pill__name').text()).toBe('Opus 4.5')
  })

  it('does not wear the fallback warning before the first status lands', () => {
    invoke.mockReturnValue(new Promise(() => {}))
    const wrapper = mount(ModelStatusBadge)
    expect(wrapper.get('.model-pill__dot').classes()).not.toContain('model-pill__dot--warn')
  })

  describe('when the model name is superseded', () => {
    it('stops naming a model this chat is not using', async () => {
      const wrapper = await mountBadge(HEALTHY, true)
      expect(wrapper.get('.model-pill__name').text()).not.toContain('Opus')
      expect(wrapper.get('.model-pill').attributes('title'))
        .toContain('does not apply here')
    })

    it('keeps showing that the router fell back, and why', async () => {
      const wrapper = await mountBadge(DEGRADED, true)
      expect(wrapper.get('.model-pill__dot').classes()).toContain('model-pill__dot--warn')
      expect(wrapper.find('.model-pill__fallback-arrow').exists()).toBe(true)
      expect(wrapper.get('.model-pill').attributes('title')).toContain('anthropic returned 529')
    })

    it('keeps counting the rate-limited models', async () => {
      const wrapper = await mountBadge(DEGRADED, true)
      expect(wrapper.text()).toContain('2 limited')
    })

    it('reads as healthy when nothing is wrong', async () => {
      const wrapper = await mountBadge(HEALTHY, true)
      expect(wrapper.get('.model-pill__dot').classes()).not.toContain('model-pill__dot--warn')
      expect(wrapper.find('.model-pill__fallback-arrow').exists()).toBe(false)
    })
  })
})
