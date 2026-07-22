import { describe, expect, it } from 'vitest'
import { mount, type MountingOptions } from '@vue/test-utils'
import PageState from '../../app/components/PageState.vue'

type Opts = MountingOptions<InstanceType<typeof PageState>>

/** PageState relies on Nuxt auto-imports stubbed in setup.ts. */
function mountPage(options: Opts = {}) {
  return mount(PageState, options)
}

describe('PageState', () => {
  it('renders loading copy', () => {
    const w = mountPage({ props: { state: 'loading', title: 'Loading memory' } })
    expect(w.text()).toContain('Loading memory')
    // Spinner is a Nuxt auto-import — presence of the loading tone is enough;
    // stub may or may not render depending on VTU stub strategy.
    expect(w.classes().join(' ') + ' ' + (w.find('.page-state').attributes('class') || '')).toMatch(/loading/)
  })

  it('renders offline with recovery CTA', async () => {
    const w = mountPage({
      props: {
        state: 'offline',
        title: 'Daemon offline',
        description: 'not reachable on 5149',
        primaryAction: 'Retry',
      },
    })
    expect(w.text()).toMatch(/Daemon offline|not reachable/)
    // Prefer a real button; fall back to role-based query if stubbed oddly.
    const btn =
      w.findAll('button').find((b) => b.text().includes('Retry')) ??
      w.find('[data-testid="ui-glass-button"]')
    expect(btn.exists()).toBe(true)
    await btn!.trigger('click')
    expect(w.emitted('primary')).toBeTruthy()
  })

  it('renders empty without forcing a button', () => {
    const w = mountPage({
      props: { state: 'empty', title: 'No tools yet' },
    })
    expect(w.text()).toContain('No tools yet')
    expect(w.findAll('button').length).toBe(0)
  })
})
