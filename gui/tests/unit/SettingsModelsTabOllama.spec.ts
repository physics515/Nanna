import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'
import SettingsModelsTab from '~/components/settings/SettingsModelsTab.vue'
import { provideSettingsPage, type SettingsPageStore } from '~/composables/useSettingsPage'

/**
 * Settings → Models, the Ollama block: a server address and an optional
 * bearer token.
 *
 * Every settings (re)load — after any save, and on each `config-changed`
 * event — ran a hook that reset both fields from the saved settings, so
 * saving one field wiped an unsaved edit in the other. There is now one Save
 * for the block, and a reload never overwrites a field being edited. The
 * page is not sent the token itself: only whether one is saved, and for
 * which server.
 */

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

const listeners = new Map<string, () => void>()
vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: () => void) => {
    listeners.set(event, handler)
    return Promise.resolve(() => listeners.delete(event))
  },
}))

const UiInputStub = defineComponent({
  name: 'UiInput',
  props: {
    modelValue: { type: String, default: '' },
    type: { type: String, default: 'text' },
    placeholder: { type: String, default: '' },
  },
  emits: ['update:modelValue'],
  setup(props, { emit, attrs }) {
    return () =>
      h('input', {
        ...attrs,
        type: props.type,
        value: props.modelValue,
        placeholder: props.placeholder,
        onInput: (e: Event) => emit('update:modelValue', (e.target as HTMLInputElement).value),
      })
  },
})

const UiButtonStub = defineComponent({
  name: 'UiButton',
  props: { disabled: { type: Boolean, default: false } },
  emits: ['click'],
  setup(props, { slots, emit, attrs }) {
    return () =>
      h('button', { ...attrs, type: 'button', disabled: props.disabled, onClick: () => emit('click') }, slots.default?.())
  },
})

const SectionStub = defineComponent({
  name: 'SettingsSection',
  setup(_props, { slots }) {
    return () => h('section', slots.default?.())
  },
})

interface Saved {
  ollama_host: string
  ollama_token_saved: boolean
  ollama_token_host: string | null
}

let saved: Saved

function answer(command: string, args?: Record<string, unknown>) {
  switch (command) {
    case 'get_extended_settings':
      return Promise.resolve({
        provider: 'ollama',
        model: 'qwen3.5:9b',
        embedding_provider: 'ollama',
        embedding_model: 'nomic-embed-text',
        claude_proxy_url: '',
        ...saved,
      })
    case 'set_ollama_host':
      saved.ollama_host = (args?.host as string).trim().replace(/\/+$/, '')
      return Promise.resolve('saved')
    case 'set_ollama_api_key': {
      const key = (args?.key as string).trim()
      saved.ollama_token_saved = key !== ''
      saved.ollama_token_host = key ? saved.ollama_host : null
      return Promise.resolve('saved')
    }
    case 'get_ollama_models':
      return Promise.resolve([])
    case 'get_use_embedded_ocr':
      return Promise.resolve(true)
    default:
      return Promise.resolve([])
  }
}

let store: SettingsPageStore

async function mountTab() {
  const Host = defineComponent({
    setup() {
      store = provideSettingsPage()
      return () => h(SettingsModelsTab)
    },
  })
  const wrapper = mount(Host, {
    global: {
      stubs: {
        UiInput: UiInputStub,
        UiButton: UiButtonStub,
        SettingsSection: SectionStub,
        UiBadge: true,
        UiSwitch: true,
        ApiKeyInput: true,
        ModelPriorityList: true,
      },
    },
  })
  await store.loadSettings()
  await flushPromises()
  return wrapper
}

const hostField = (w: Awaited<ReturnType<typeof mountTab>>) =>
  w.find('[data-testid="ollama-host"]').element as HTMLInputElement
const tokenField = (w: Awaited<ReturnType<typeof mountTab>>) =>
  w.find('[data-testid="ollama-token"]').element as HTMLInputElement
const calls = () => invoke.mock.calls.map((c) => c[0] as string)

/** Something else changed the config: the tab's listener reloads settings. */
function configChanged() {
  const reload = listeners.get('config-changed')
  expect(reload, 'the tab listens for config-changed').toBeTypeOf('function')
  reload!()
}

describe('SettingsModelsTab — Ollama server and token', () => {
  beforeEach(() => {
    saved = { ollama_host: 'http://localhost:11434', ollama_token_saved: false, ollama_token_host: null }
    invoke.mockReset()
    invoke.mockImplementation(answer)
    listeners.clear()
  })

  it('a reload keeps what is typed in both fields', async () => {
    const wrapper = await mountTab()
    expect(hostField(wrapper).value).toBe('http://localhost:11434')
    await wrapper.find('[data-testid="ollama-host"]').setValue('https://mummu.example/ollama')
    await wrapper.find('[data-testid="ollama-token"]').setValue('s3cret')

    // Something else changed the config: the page reloads its settings.
    configChanged()
    await flushPromises()

    expect(hostField(wrapper).value).toBe('https://mummu.example/ollama')
    expect(tokenField(wrapper).value).toBe('s3cret')
  })

  it('an address saved elsewhere still shows while the field is untouched', async () => {
    const wrapper = await mountTab()
    saved.ollama_host = 'https://other-window.example/ollama'
    configChanged()
    await flushPromises()
    expect(hostField(wrapper).value).toBe('https://other-window.example/ollama')
  })

  it('one Save stores the address, then the token for it, then re-checks the server', async () => {
    const wrapper = await mountTab()
    await wrapper.find('[data-testid="ollama-host"]').setValue(' https://mummu.example/ollama/ ')
    await wrapper.find('[data-testid="ollama-token"]').setValue(' s3cret ')
    invoke.mockClear()
    await wrapper.find('[data-testid="ollama-save"]').trigger('click')
    await flushPromises()

    expect(invoke).toHaveBeenCalledWith('set_ollama_host', { host: 'https://mummu.example/ollama/' })
    expect(invoke).toHaveBeenCalledWith('set_ollama_api_key', { key: 's3cret' })
    const order = calls()
    expect(order.indexOf('set_ollama_host')).toBeLessThan(order.indexOf('set_ollama_api_key'))
    expect(order.lastIndexOf('get_ollama_models')).toBeGreaterThan(order.indexOf('set_ollama_api_key'))

    // Saved: the address shows as stored, the token field empties, and its
    // placeholder says a token is saved — without showing it.
    expect(hostField(wrapper).value).toBe('https://mummu.example/ollama')
    expect(tokenField(wrapper).value).toBe('')
    expect(tokenField(wrapper).placeholder).toMatch(/saved for this server/i)
  })

  it('an empty or blank token field keeps the saved token', async () => {
    saved.ollama_token_saved = true
    saved.ollama_token_host = 'http://localhost:11434'
    const wrapper = await mountTab()
    await wrapper.find('[data-testid="ollama-token"]').setValue('   ')
    await wrapper.find('[data-testid="ollama-save"]').trigger('click')
    await flushPromises()
    expect(calls()).not.toContain('set_ollama_api_key')
    expect(saved.ollama_token_saved).toBe(true)
  })

  it('Remove token clears the saved token', async () => {
    saved.ollama_token_saved = true
    saved.ollama_token_host = 'http://localhost:11434'
    const wrapper = await mountTab()
    await wrapper.find('[data-testid="ollama-token-remove"]').trigger('click')
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('set_ollama_api_key', { key: '' })
    expect(wrapper.find('[data-testid="ollama-token-remove"]').exists()).toBe(false)
  })

  it('says plainly when the saved token belongs to another server', async () => {
    saved.ollama_token_saved = true
    saved.ollama_token_host = 'https://mummu.example/ollama'
    const wrapper = await mountTab()
    const note = wrapper.find('[data-testid="ollama-token-elsewhere"]')
    expect(note.exists()).toBe(true)
    expect(note.text()).toContain('https://mummu.example/ollama')
    expect(note.text()).toMatch(/not sent/i)
    expect(tokenField(wrapper).placeholder).not.toMatch(/saved/i)

    // Typing that server's address (spelled differently) is the same server.
    await wrapper.find('[data-testid="ollama-host"]').setValue('https://MUMMU.example:443/ollama/')
    expect(wrapper.find('[data-testid="ollama-token-elsewhere"]').exists()).toBe(false)
  })

  it('warns that a token to a plain-http remote server travels unencrypted', async () => {
    const wrapper = await mountTab()
    await wrapper.find('[data-testid="ollama-host"]').setValue('http://gpu-box:11434')
    expect(wrapper.find('[data-testid="ollama-token-cleartext"]').exists()).toBe(false)
    await wrapper.find('[data-testid="ollama-token"]').setValue('s3cret')
    expect(wrapper.find('[data-testid="ollama-token-cleartext"]').exists()).toBe(true)

    // This machine, or https: nothing crosses the network in the clear.
    await wrapper.find('[data-testid="ollama-host"]').setValue('http://localhost:11434')
    expect(wrapper.find('[data-testid="ollama-token-cleartext"]').exists()).toBe(false)
    await wrapper.find('[data-testid="ollama-host"]').setValue('https://gpu-box/ollama')
    expect(wrapper.find('[data-testid="ollama-token-cleartext"]').exists()).toBe(false)
  })
})
