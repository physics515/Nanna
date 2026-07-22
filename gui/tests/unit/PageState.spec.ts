import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import PageState from '../../app/components/PageState.vue'

describe('PageState', () => {
  it('renders loading copy', () => {
    const w = mount(PageState, { props: { state: 'loading', title: 'Loading memory' } })
    expect(w.text()).toContain('Loading memory')
  })

  it('renders offline with recovery CTA', async () => {
    const w = mount(PageState, {
      props: {
        state: 'offline',
        title: 'Daemon offline',
        description: 'not reachable on 5149',
        primaryAction: 'Retry',
      },
    })
    expect(w.text()).toMatch(/Daemon offline|not reachable/)
    const btn = w.find('button')
    expect(btn.exists()).toBe(true)
    await btn.trigger('click')
    expect(w.emitted('primary')).toBeTruthy()
  })

  it('renders empty without forcing a button', () => {
    const w = mount(PageState, {
      props: { state: 'empty', title: 'No tools yet' },
    })
    expect(w.text()).toContain('No tools yet')
    expect(w.findAll('button').length).toBe(0)
  })
})
