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
 *
 * The saved address is prefilled from `get_extended_settings`, which is slow
 * while the daemon starts. What the user types meanwhile is theirs, and
 * Continue never saves the default over a saved address it has not read yet.
 * The page is never sent the token, so an empty token field means "keep the
 * saved one".
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

/** Emits a save of a (rejected) key, and shows the error it is handed. */
const ApiKeyInputStub = defineComponent({
  name: 'ApiKeyInput',
  props: { externalError: { type: String, default: null }, provider: { type: String, default: '' } },
  emits: ['save'],
  setup(props, { emit }) {
    return () =>
      h('div', [
        h('button', { type: 'button', 'data-testid': 'key-save', onClick: () => emit('save', props.provider, 'sk-bad') }, 'Save key'),
        props.externalError ? h('p', { 'data-testid': 'key-error' }, props.externalError) : null,
      ])
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

const savedSettings = {
  ollama_host: 'http://localhost:11434',
  ollama_token_saved: false,
  ollama_token_host: null as string | null,
  ollama_token_from_env: false,
}

function answer(command: string, args?: Record<string, unknown>) {
  switch (command) {
    case 'get_extended_settings':
      return Promise.resolve({ ...savedSettings })
    case 'set_ollama_host':
      return (args?.host as string)?.startsWith('http')
        ? Promise.resolve('saved')
        : Promise.reject(new Error('Ollama host must start with http:// or https://'))
    case 'set_provider_api_key':
      return Promise.reject(new Error('That key was refused'))
    case 'get_backend_status':
      return Promise.resolve({ running: true, version: '0.3.22' })
    case 'probe_ollama':
      return Promise.resolve(probeReport)
    default:
      return Promise.resolve(null)
  }
}

/** A promise the test settles by hand — `get_extended_settings` while the daemon starts. */
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

function mountWizard() {
  return mount(OnboardingWizard, {
    props: { open: true, hasApiKey: false },
    global: {
      stubs: { UiButton: UiButtonStub, ApiKeyInput: ApiKeyInputStub, Teleport: true },
    },
    attachTo: document.body,
  })
}

type Wizard = ReturnType<typeof mountWizard>

const continueButton = (w: Wizard) => w.findAll('button').find((b) => b.text().includes('Continue'))!
const calls = () => invoke.mock.calls.map((c) => c[0] as string)

async function toProviderStep(wrapper: Wizard) {
  await continueButton(wrapper).trigger('click')
  await flushPromises()
}

async function mountAtOllamaStep() {
  const wrapper = mountWizard()
  await toProviderStep(wrapper)
  await wrapper.find('select').setValue('ollama')
  await flushPromises()
  return wrapper
}

describe('OnboardingWizard — Ollama server and token', () => {
  beforeEach(() => {
    savedSettings.ollama_host = 'http://localhost:11434'
    savedSettings.ollama_token_saved = false
    savedSettings.ollama_token_host = null
    savedSettings.ollama_token_from_env = false
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

  it('says a token is saved for the server without showing it', async () => {
    savedSettings.ollama_host = 'https://mummu.example/ollama'
    savedSettings.ollama_token_saved = true
    savedSettings.ollama_token_host = 'https://mummu.example/ollama'
    const wrapper = await mountAtOllamaStep()
    const token = wrapper.find('[data-testid="onboarding-ollama-token"]').element as HTMLInputElement
    expect(token.value).toBe('')
    expect(token.placeholder).toMatch(/saved for this server/i)
  })

  it('says when OLLAMA_API_KEY sends a token to this address', async () => {
    const wrapper = await mountAtOllamaStep()
    expect(wrapper.find('[data-testid="onboarding-ollama-token-env"]').exists()).toBe(false)

    savedSettings.ollama_token_from_env = true
    const withEnv = await mountAtOllamaStep()
    const note = withEnv.find('[data-testid="onboarding-ollama-token-env"]')
    expect(note.exists()).toBe(true)
    expect(note.text()).toContain('OLLAMA_API_KEY')
    await withEnv.find('[data-testid="onboarding-ollama-host"]').setValue('http://gpu-box:11434')
    expect(withEnv.find('[data-testid="onboarding-ollama-token-cleartext"]').exists()).toBe(true)
  })

  it('saves the address and token before probing, trimmed', async () => {
    const wrapper = await mountAtOllamaStep()
    await wrapper.find('[data-testid="onboarding-ollama-host"]').setValue('  https://mummu.example/ollama/  ')
    await wrapper.find('[data-testid="onboarding-ollama-token"]').setValue(' s3cret ')
    await continueButton(wrapper).trigger('click')
    await flushPromises()

    const order = calls()
    expect(invoke).toHaveBeenCalledWith('set_ollama_host', { host: 'https://mummu.example/ollama/' })
    expect(invoke).toHaveBeenCalledWith('set_ollama_api_key', { key: 's3cret' })
    // Both saves land before the probe that checks them, the token after the
    // address it is bound to.
    expect(order.indexOf('set_ollama_host')).toBeLessThan(order.indexOf('set_ollama_api_key'))
    expect(order.indexOf('set_ollama_api_key')).toBeLessThan(order.indexOf('probe_ollama'))
    expect(wrapper.find('[data-testid="ollama-probe"]').text()).toContain('https://mummu.example/ollama')
  })

  it('an empty token field keeps the saved token', async () => {
    savedSettings.ollama_token_saved = true
    savedSettings.ollama_token_host = 'http://localhost:11434'
    const wrapper = await mountAtOllamaStep()
    await wrapper.find('[data-testid="onboarding-ollama-token"]').setValue('   ')
    await continueButton(wrapper).trigger('click')
    await flushPromises()
    expect(calls()).not.toContain('set_ollama_api_key')
    expect(calls()).toContain('probe_ollama')
  })

  it('stops on an address the app refuses, with the reason, and does not probe', async () => {
    const wrapper = await mountAtOllamaStep()
    await wrapper.find('[data-testid="onboarding-ollama-host"]').setValue('mummu.example/ollama')
    await continueButton(wrapper).trigger('click')
    await flushPromises()

    expect(wrapper.find('[data-testid="onboarding-ollama-error"]').text()).toContain('http://')
    expect(calls()).not.toContain('probe_ollama')
  })

  it('what is typed before the saved settings arrive is not overwritten', async () => {
    const slow = deferred<typeof savedSettings>()
    invoke.mockImplementation((command: string, args?: Record<string, unknown>) =>
      command === 'get_extended_settings' ? slow.promise : answer(command, args),
    )
    const wrapper = await mountAtOllamaStep()
    await wrapper.find('[data-testid="onboarding-ollama-host"]').setValue('https://typed.example/ollama')

    slow.resolve({
      ollama_host: 'https://saved.example/ollama',
      ollama_token_saved: false,
      ollama_token_host: null,
      ollama_token_from_env: false,
    })
    await flushPromises()

    const host = wrapper.find('[data-testid="onboarding-ollama-host"]').element as HTMLInputElement
    expect(host.value).toBe('https://typed.example/ollama')
  })

  it('Continue waits for the saved settings instead of saving the default over them', async () => {
    const slow = deferred<typeof savedSettings>()
    invoke.mockImplementation((command: string, args?: Record<string, unknown>) =>
      command === 'get_extended_settings' ? slow.promise : answer(command, args),
    )
    const wrapper = await mountAtOllamaStep()
    await continueButton(wrapper).trigger('click')
    await flushPromises()
    expect(calls()).not.toContain('set_ollama_host')
    expect(calls()).not.toContain('set_ollama_api_key')
    expect(calls()).not.toContain('probe_ollama')

    slow.resolve({
      ollama_host: 'https://saved.example/ollama',
      ollama_token_saved: true,
      ollama_token_host: 'https://saved.example/ollama',
      ollama_token_from_env: false,
    })
    await flushPromises()

    // The saved remote address and its token are left as they were, and the
    // check goes ahead against them.
    const hostSaves = invoke.mock.calls.filter((c) => c[0] === 'set_ollama_host')
    expect(hostSaves.every((c) => (c[1] as { host: string }).host !== 'http://localhost:11434')).toBe(true)
    expect(calls()).not.toContain('set_ollama_api_key')
    expect(calls()).toContain('probe_ollama')
  })

  it('a prefill that failed can be retried', async () => {
    let attempts = 0
    invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command !== 'get_extended_settings') return answer(command, args)
      attempts += 1
      return attempts === 1 ? Promise.reject(new Error('daemon not ready')) : answer(command, args)
    })
    savedSettings.ollama_host = 'https://saved.example/ollama'
    const wrapper = await mountAtOllamaStep()
    const retry = wrapper.find('[data-testid="onboarding-ollama-prefill-retry"]')
    expect(retry.exists()).toBe(true)

    await retry.trigger('click')
    await flushPromises()
    expect(attempts).toBe(2)
    const host = wrapper.find('[data-testid="onboarding-ollama-host"]').element as HTMLInputElement
    expect(host.value).toBe('https://saved.example/ollama')
    expect(wrapper.find('[data-testid="onboarding-ollama-prefill-retry"]').exists()).toBe(false)
  })

  it('an unread saved address is not overwritten by the default on Continue', async () => {
    invoke.mockImplementation((command: string, args?: Record<string, unknown>) =>
      command === 'get_extended_settings' ? Promise.reject(new Error('daemon not ready')) : answer(command, args),
    )
    const wrapper = await mountAtOllamaStep()
    await continueButton(wrapper).trigger('click')
    await flushPromises()
    expect(calls()).not.toContain('set_ollama_host')
    expect(calls()).toContain('probe_ollama')
  })

  it("an API key's error is not shown on the Ollama step", async () => {
    const wrapper = mountWizard()
    await toProviderStep(wrapper)
    await wrapper.find('[data-testid="key-save"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-testid="key-error"]').text()).toContain('refused')

    await wrapper.find('select').setValue('ollama')
    await flushPromises()
    expect(wrapper.find('[data-testid="onboarding-ollama-error"]').exists()).toBe(false)

    // And back: switching provider cleared it there too.
    await wrapper.find('select').setValue('anthropic')
    await flushPromises()
    expect(wrapper.find('[data-testid="key-error"]').exists()).toBe(false)
  })
})
