import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'
import OnboardingWizard from '~/components/OnboardingWizard.vue'

/**
 * The Ollama choice in onboarding takes a server address and an optional
 * bearer token, saves both BEFORE the health check, and the health check then
 * probes that saved server. A remote, Ollama-compatible server (mummu's shim
 * behind a proxy, 2026-09-18) was unreachable from here before: the step only
 * said "Ollama runs locally" and probed whatever address the config held.
 */

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

const UiButtonStub = defineComponent({
  name: 'UiButton',
  props: { disabled: { type: Boolean, default: false } },
  emits: ['click'],
  setup(props, { slots, emit }) {
    return () =>
      h('button', { type: 'button', disabled: props.disabled, onClick: () => emit('click') }, slots.default?.())
  },
})

const probeReport = {
  reachable: true,
  base_url: 'https://mummu.example/ollama',
  reason: null,
  models: [{ name: 'qwen2.5-1.5b-instruct', size_mb: 3100, is_embedding_model: false }],
  wanted: [],
  missing: [],
}

function answer(command: string, args?: Record<string, unknown>) {
  switch (command) {
    case 'get_extended_settings':
      return Promise.resolve({ ollama_host: 'http://localhost:11434', ollama_api_key: '' })
    case 'set_ollama_host':
      return (args?.host as string)?.startsWith('http')
        ? Promise.resolve('saved')
        : Promise.reject(new Error('Ollama host must start with http:// or https://'))
    case 'get_backend_status':
      return Promise.resolve({ running: true, version: '0.3.22' })
    case 'probe_ollama':
      return Promise.resolve(probeReport)
    default:
      return Promise.resolve(null)
  }
}

async function mountAtOllamaStep() {
  const wrapper = mount(OnboardingWizard, {
    props: { open: true, hasApiKey: false },
    global: {
      stubs: { UiButton: UiButtonStub, ApiKeyInput: true, Teleport: true },
    },
    attachTo: document.body,
  })
  // Step 1 → 2, then pick Ollama.
  await wrapper.findAll('button').find((b) => b.text().includes('Continue'))!.trigger('click')
  await flushPromises()
  await wrapper.find('select').setValue('ollama')
  await flushPromises()
  return wrapper
}

describe('OnboardingWizard — Ollama server and token', () => {
  beforeEach(() => {
    invoke.mockReset()
    invoke.mockImplementation(answer)
    localStorage.clear()
  })

  it('prefills the saved address and offers a token field', async () => {
    const wrapper = await mountAtOllamaStep()
    const host = wrapper.find('[data-testid="onboarding-ollama-host"]')
    expect(host.exists()).toBe(true)
    expect((host.element as HTMLInputElement).value).toBe('http://localhost:11434')
    expect(wrapper.find('[data-testid="onboarding-ollama-token"]').exists()).toBe(true)
    expect(invoke).toHaveBeenCalledWith('get_extended_settings')
  })

  it('saves the address and token before probing, trimmed', async () => {
    const wrapper = await mountAtOllamaStep()
    await wrapper.find('[data-testid="onboarding-ollama-host"]').setValue('  https://mummu.example/ollama/  ')
    await wrapper.find('[data-testid="onboarding-ollama-token"]').setValue(' s3cret ')
    await wrapper.findAll('button').find((b) => b.text().includes('Continue'))!.trigger('click')
    await flushPromises()

    const calls = invoke.mock.calls.map((c) => c[0])
    expect(invoke).toHaveBeenCalledWith('set_ollama_host', { host: 'https://mummu.example/ollama/' })
    expect(invoke).toHaveBeenCalledWith('set_ollama_api_key', { key: 's3cret' })
    // Both saves land before the probe that checks them.
    expect(calls.indexOf('set_ollama_host')).toBeLessThan(calls.indexOf('probe_ollama'))
    expect(calls.indexOf('set_ollama_api_key')).toBeLessThan(calls.indexOf('probe_ollama'))
    expect(wrapper.find('[data-testid="ollama-probe"]').text()).toContain('https://mummu.example/ollama')
  })

  it('stops on an address the app refuses, with the reason, and does not probe', async () => {
    const wrapper = await mountAtOllamaStep()
    await wrapper.find('[data-testid="onboarding-ollama-host"]').setValue('mummu.example/ollama')
    await wrapper.findAll('button').find((b) => b.text().includes('Continue'))!.trigger('click')
    await flushPromises()

    expect(wrapper.find('[data-testid="onboarding-ollama-error"]').text()).toContain('http://')
    expect(invoke.mock.calls.map((c) => c[0])).not.toContain('probe_ollama')
  })
})
