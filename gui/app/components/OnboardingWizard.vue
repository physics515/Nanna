<script setup lang="ts">
/**
 * Minimal 3-step first-run wizard.
 * Persists `nanna.onboarding.done=1` on finish/skip.
 */
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ArrowRight, Check, HeartPulse, KeyRound, Sparkles, X } from '@lucide/vue'
import { isLoopbackUrl, sameOllamaServer, sendsTokenInClear } from '~/lib/ollamaServer'
import { describeBackend, type BackendStatusLike } from '~/lib/backendLabels'

const props = defineProps<{
  open: boolean
  /** True when a key is already configured (skip step 2 detail). */
  hasApiKey?: boolean
}>()

const emit = defineEmits<{
  close: []
  finished: []
}>()

const STORAGE_KEY = 'nanna.onboarding.done'

const step = ref(1)
const provider = ref('anthropic')
const saving = ref(false)
const checking = ref(false)
const keySaved = ref(false)
/** Each path's own error: a refused API key is not the Ollama step's, and
 *  switching provider clears both. */
const keyError = ref<string | null>(null)
const ollamaError = ref<string | null>(null)
const healthOk = ref(false)
const healthDetail = ref('')

/** The daemon's answer to "is Ollama running, and is a model pulled?" */
interface OllamaProbeResult {
  base_url: string
  reachable: boolean
  reason: string | null
  models: { name: string; size_mb: number; is_embedding_model: boolean }[]
  wanted: string[]
  missing: { name: string; pull: string }[]
}
const ollamaProbe = ref<OllamaProbeResult | null>(null)

const providers = [
  { value: 'anthropic', label: 'Anthropic' },
  { value: 'openai', label: 'OpenAI' },
  { value: 'openrouter', label: 'OpenRouter' },
  { value: 'ollama', label: 'Ollama (local or remote)' },
]

/** Ollama server address and optional bearer token, for the Ollama choice. */
const OLLAMA_DEFAULT_HOST = 'http://localhost:11434'
const ollamaHost = ref(OLLAMA_DEFAULT_HOST)
const ollamaToken = ref('')
/** Set once the user types an address: the prefill never overwrites it, and
 *  only then does Continue save one. */
const ollamaHostEdited = ref(false)
/** Whether a token is saved, and for which server — never the token. */
const ollamaTokenSaved = ref(false)
const ollamaTokenHost = ref<string | null>(null)
/** `OLLAMA_API_KEY` is set: its token goes to whatever address is set. */
const ollamaTokenFromEnv = ref(false)
const ollamaPrefill = ref<'idle' | 'loading' | 'loaded' | 'failed'>('idle')
let ollamaPrefillRun: Promise<void> | null = null

/** Prefill from the saved config — a re-run of onboarding must not show the
 *  default over an address the user already set. `get_extended_settings` is
 *  slow while the daemon starts: this returns the one in-flight read, and a
 *  failed read can be asked again. */
function loadOllamaSettings(): Promise<void> {
  if (ollamaPrefill.value === 'loaded') return Promise.resolve()
  if (ollamaPrefillRun) return ollamaPrefillRun
  ollamaPrefill.value = 'loading'
  ollamaPrefillRun = (async () => {
    try {
      const s = await invoke<{
        ollama_host?: string
        ollama_token_saved?: boolean
        ollama_token_host?: string | null
        ollama_token_from_env?: boolean
      }>('get_extended_settings')
      // Typed while this was in flight: the user's, not the saved value's.
      if (s?.ollama_host && !ollamaHostEdited.value) ollamaHost.value = s.ollama_host
      ollamaTokenSaved.value = !!s?.ollama_token_saved
      ollamaTokenHost.value = s?.ollama_token_host ?? null
      ollamaTokenFromEnv.value = !!s?.ollama_token_from_env
      ollamaPrefill.value = 'loaded'
    } catch {
      ollamaPrefill.value = 'failed'
    } finally {
      ollamaPrefillRun = null
    }
  })()
  return ollamaPrefillRun
}

/** The saved token goes to the address in the field. */
const ollamaTokenIsForHost = computed(
  () => ollamaTokenSaved.value && !!ollamaTokenHost.value && sameOllamaServer(ollamaTokenHost.value, ollamaHost.value),
)
/** The server the saved token belongs to, when it is not this one. */
const ollamaTokenElsewhere = computed(() =>
  ollamaTokenSaved.value && ollamaTokenHost.value && !ollamaTokenIsForHost.value ? ollamaTokenHost.value : null,
)
const ollamaTokenPlaceholder = computed(() =>
  ollamaTokenIsForHost.value ? 'A token is saved for this server — leave empty to keep it' : 'Only if the server requires one',
)
/** A token that would go to this address over plain http to another machine:
 *  one being typed, the one saved for it, or the environment's. */
const ollamaTokenInClear = computed(
  () =>
    sendsTokenInClear(ollamaHost.value) &&
    (ollamaToken.value.trim() !== '' || ollamaTokenIsForHost.value || ollamaTokenFromEnv.value),
)

const needsKey = computed(() => provider.value !== 'ollama')

watch(provider, (p) => {
  keyError.value = null
  ollamaError.value = null
  if (p === 'ollama') void loadOllamaSettings()
})

watch(
  () => props.open,
  (open) => {
    if (open) {
      step.value = 1
      keyError.value = null
      ollamaError.value = null
      keySaved.value = !!props.hasApiKey
      healthOk.value = false
      healthDetail.value = ''
      ollamaProbe.value = null
      saving.value = false
      checking.value = false
    }
  },
)

function markDone() {
  try {
    localStorage.setItem(STORAGE_KEY, '1')
  } catch {
    /* ignore */
  }
}

function finish() {
  markDone()
  emit('finished')
  emit('close')
}

function skip() {
  markDone()
  emit('close')
}

function goStep2() {
  step.value = 2
  if (props.hasApiKey) keySaved.value = true
}

async function onKeySave(p: string, key: string) {
  saving.value = true
  keyError.value = null
  try {
    await invoke('set_provider_api_key', { provider: p, apiKey: key })
    try { await invoke('set_provider', { provider: p }) } catch { /* non-fatal */ }
    keySaved.value = true
    step.value = 3
    await runHealthCheck()
  } catch (e: any) {
    keyError.value = e?.message || String(e) || "Couldn't save that key."
  } finally {
    saving.value = false
  }
}

async function continueWithoutKey() {
  keyError.value = null
  ollamaError.value = null
  if (provider.value === 'ollama') {
    saving.value = true
    // The saved address has to be known before anything is saved over it.
    await loadOllamaSettings()
    try {
      // Address and token first: the health check probes the saved server.
      // A bad address stops here with the reason, instead of probing the old one.
      // Only what was typed is saved: an untouched address keeps the saved
      // one (even one the prefill could not read), and an empty token field
      // keeps the saved token. The token goes after the address it is for.
      if (ollamaHostEdited.value) {
        await invoke('set_ollama_host', { host: ollamaHost.value.trim() || OLLAMA_DEFAULT_HOST })
      }
      const token = ollamaToken.value.trim()
      if (token) await invoke('set_ollama_api_key', { key: token })
    } catch (e: any) {
      ollamaError.value = e?.message || String(e)
      saving.value = false
      return
    }
    try {
      await invoke('set_provider', { provider: 'ollama' })
    } catch {
      /* non-fatal — health check will surface issues */
    } finally {
      saving.value = false
    }
  }
  step.value = 3
  await runHealthCheck()
}

async function runHealthCheck() {
  checking.value = true
  healthOk.value = false
  healthDetail.value = ''
  ollamaProbe.value = null
  try {
    // `connected` is the one field that says the daemon answers. This used to
    // read a `running` field the status has never had, so the check passed
    // whenever the call returned, and showed the app's own version as the
    // daemon's: `version` is this GUI's build; the daemon reports its own.
    const status = await invoke<BackendStatusLike | null>('get_backend_status')
    if (status?.connected === true) {
      healthOk.value = true
      const daemonVersion = await invoke<string | null>('get_daemon_version').catch(() => null)
      healthDetail.value = daemonVersion
        ? `Backend ready · ${daemonVersion}`
        : 'Backend is reachable.'
    } else {
      healthOk.value = false
      healthDetail.value = `${describeBackend(status).short}. You can finish setup now; chats work once the daemon connects.`
    }
  } catch (e: any) {
    healthOk.value = false
    healthDetail.value = e?.message || "Couldn't reach the backend. You can still start and fix this in Settings."
  }
  // Ollama is keyless, so "the backend answers" says nothing about whether a
  // model can actually run. Ask the daemon's probe instead of assuming a local
  // server is up: down and missing-model are different next steps.
  if (provider.value === 'ollama' && healthOk.value) {
    try {
      const probe = await invoke<OllamaProbeResult>('probe_ollama', {})
      ollamaProbe.value = probe
      if (!probe.reachable) {
        healthOk.value = false
      } else if (probe.missing.length > 0) {
        healthOk.value = false
      }
    } catch (e: any) {
      ollamaProbe.value = null
      healthOk.value = false
      healthDetail.value = `Backend is reachable, but the Ollama check failed: ${e?.message || String(e)}`
    }
  }
  checking.value = false
}

/** What the wizard says about Ollama, from the probe — one sentence, plus the fix. */
const ollamaSummary = computed(() => {
  const p = ollamaProbe.value
  if (!p) return null
  if (!p.reachable) {
    return {
      ok: false,
      text: `Ollama is not answering at ${p.base_url}${p.reason ? ` (${p.reason})` : ''}.`,
      // "Install and start it" only makes sense for this machine; for a remote
      // server the usual fix is the address (its path) or the token.
      fix: isLoopbackUrl(p.base_url)
        ? 'Install Ollama and start it (`ollama serve`), then recheck.'
        : 'Check the server URL (including any path, e.g. /ollama) and the bearer token, then go back and recheck.',
      pulls: [] as string[],
    }
  }
  if (p.missing.length > 0) {
    return {
      ok: false,
      text: `Ollama is running at ${p.base_url}, but ${p.missing.length === 1 ? 'a configured model is' : `${p.missing.length} configured models are`} not pulled yet.`,
      fix: 'Run this in a terminal, then recheck:',
      pulls: p.missing.map((m) => m.pull),
    }
  }
  const count = p.models.length
  return {
    ok: true,
    text: p.wanted.length > 0
      ? `Ollama is running at ${p.base_url} with every configured model pulled (${count} installed).`
      : `Ollama is running at ${p.base_url} with ${count} model${count === 1 ? '' : 's'} installed.`,
    fix: count === 0 ? 'Pull a model to chat with, e.g. `ollama pull qwen3.5:9b`.' : null,
    pulls: [] as string[],
  }
})
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="fixed inset-0 z-[100] flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="onboarding-title"
    >
      <div class="absolute inset-0 bg-black/55 backdrop-blur-sm" @click="skip" />
      <div
        class="relative w-full max-w-md rounded-2xl glass-strong overflow-hidden"
      >
        <!-- Progress -->
        <div class="flex items-center gap-1.5 px-6 pt-5">
          <div
            v-for="n in 3"
            :key="n"
            class="h-1 flex-1 rounded-full transition-colors"
            :class="n <= step ? 'bg-nanna-primary' : 'bg-white/[0.08]'"
          />
          <button
            type="button"
            class="ml-3 p-1 rounded-md text-nanna-text-dim hover:text-nanna-text hover:bg-white/[0.06] transition-colors"
            aria-label="Close onboarding"
            @click="skip"
          >
            <X class="w-4 h-4" />
          </button>
        </div>

        <div class="p-6 space-y-5">
          <!-- Step 1: What Nanna is -->
          <div v-if="step === 1" class="space-y-4">
            <div class="flex items-center gap-3">
              <div class="w-10 h-10 rounded-xl bg-nanna-primary/15 flex items-center justify-center">
                <Sparkles class="w-5 h-5 text-nanna-primary" />
              </div>
              <h2 id="onboarding-title" class="text-lg font-semibold text-nanna-text">Welcome to Nanna</h2>
            </div>
            <p class="text-sm text-nanna-text-muted leading-relaxed">
              Nanna is a calm personal agent — chat, tools, and memory that stay on your machine.
              It can use cloud models or local ones; you’re always in control of keys and data.
            </p>
            <p class="text-sm text-nanna-text-muted leading-relaxed">
              A short setup gets you talking. You can change everything later in Settings.
            </p>
            <div class="flex justify-between pt-2">
              <UiButton variant="ghost" size="sm" @click="skip">Skip</UiButton>
              <UiButton size="sm" @click="goStep2">
                Continue
                <ArrowRight class="w-4 h-4" />
              </UiButton>
            </div>
          </div>

          <!-- Step 2: Backend / API key -->
          <div v-else-if="step === 2" class="space-y-4">
            <div class="flex items-center gap-3">
              <div class="w-10 h-10 rounded-xl bg-nanna-primary/15 flex items-center justify-center">
                <KeyRound class="w-5 h-5 text-nanna-primary" />
              </div>
              <div>
                <h2 class="text-lg font-semibold text-nanna-text">Connect a model</h2>
                <p class="text-xs text-nanna-text-muted">Pick a provider and add a key, or connect to an Ollama server.</p>
              </div>
            </div>

            <div class="space-y-2">
              <label class="text-xs font-medium text-nanna-text-muted">Provider</label>
              <select
                v-model="provider"
                class="w-full bg-white/[0.04] border border-white/[0.08] rounded-lg px-3 py-2 text-sm text-nanna-text focus:outline-none focus:border-nanna-primary/50"
              >
                <option v-for="p in providers" :key="p.value" :value="p.value">{{ p.label }}</option>
              </select>
            </div>

            <ApiKeyInput
              v-if="needsKey"
              :label="providers.find((p) => p.value === provider)?.label || 'API key'"
              :provider="provider"
              placeholder="Paste API key"
              :is-set="keySaved || hasApiKey"
              :saving="saving"
              :external-error="keyError"
              :hint="hasApiKey ? 'A key is already saved. You can replace it or continue.' : undefined"
              @save="onKeySave"
            />
            <div v-else class="space-y-3" data-testid="ollama-connection">
              <div>
                <label class="block text-xs text-nanna-text-dim mb-1" for="onboarding-ollama-host">Server URL</label>
                <input
                  id="onboarding-ollama-host"
                  v-model="ollamaHost"
                  data-testid="onboarding-ollama-host"
                  type="url"
                  :placeholder="OLLAMA_DEFAULT_HOST"
                  class="w-full bg-white/[0.04] border border-white/[0.08] rounded-lg px-3 py-2 text-sm text-nanna-text focus:outline-none focus:border-nanna-primary/50"
                  @input="ollamaHostEdited = true"
                >
                <p v-if="ollamaPrefill === 'failed'" class="text-[11px] text-nanna-text-muted mt-1">
                  Couldn't read the saved address yet — Continue keeps it unless you type one.
                  <button
                    type="button"
                    data-testid="onboarding-ollama-prefill-retry"
                    class="underline hover:text-nanna-text"
                    @click="loadOllamaSettings"
                  >Try again</button>
                </p>
                <p class="text-[11px] text-nanna-text-muted mt-1">
                  Leave the default for Ollama on this machine. For a remote or Ollama-compatible server, use the address
                  that answers <code>/api/tags</code>, including any path it lives under (e.g. <code>https://host/ollama</code>).
                </p>
              </div>
              <div>
                <label class="block text-xs text-nanna-text-dim mb-1" for="onboarding-ollama-token">
                  Bearer token <span class="text-nanna-text-dim/60">(optional)</span>
                </label>
                <input
                  id="onboarding-ollama-token"
                  v-model="ollamaToken"
                  data-testid="onboarding-ollama-token"
                  type="password"
                  autocomplete="off"
                  :placeholder="ollamaTokenPlaceholder"
                  class="w-full bg-white/[0.04] border border-white/[0.08] rounded-lg px-3 py-2 text-sm text-nanna-text focus:outline-none focus:border-nanna-primary/50"
                >
                <p v-if="ollamaTokenElsewhere" data-testid="onboarding-ollama-token-elsewhere" class="text-[11px] text-nanna-text-muted mt-1">
                  The saved token is for <code>{{ ollamaTokenElsewhere }}</code> and is not sent to this address.
                  Enter one here if this server needs it.
                </p>
                <p v-if="ollamaTokenFromEnv" data-testid="onboarding-ollama-token-env" class="text-[11px] text-nanna-text-muted mt-1">
                  <code>OLLAMA_API_KEY</code> is set in Nanna's environment, so its token is sent to this address — to
                  whatever address is set here — instead of a saved one.
                </p>
                <p v-if="ollamaTokenInClear" data-testid="onboarding-ollama-token-cleartext" class="text-[11px] text-amber-300 mt-1">
                  This address is plain http:// to another machine, so the token would cross the network unencrypted.
                </p>
              </div>
              <p v-if="ollamaError" class="text-xs text-red-400" data-testid="onboarding-ollama-error">{{ ollamaError }}</p>
              <p class="text-xs text-nanna-text-muted">The next step checks the server is answering and has a model.</p>
            </div>

            <div class="flex justify-between pt-2">
              <UiButton variant="ghost" size="sm" @click="step = 1">Back</UiButton>
              <div class="flex gap-2">
                <UiButton
                  v-if="hasApiKey || keySaved || !needsKey"
                  variant="secondary"
                  size="sm"
                  :disabled="saving"
                  @click="continueWithoutKey"
                >
                  {{ needsKey ? 'Use existing' : 'Continue' }}
                  <ArrowRight v-if="!needsKey" class="w-4 h-4" />
                </UiButton>
              </div>
            </div>
          </div>

          <!-- Step 3: Health check -->
          <div v-else class="space-y-4">
            <div class="flex items-center gap-3">
              <div class="w-10 h-10 rounded-xl bg-nanna-primary/15 flex items-center justify-center">
                <HeartPulse class="w-5 h-5 text-nanna-primary" />
              </div>
              <div>
                <h2 class="text-lg font-semibold text-nanna-text">Ready check</h2>
                <p class="text-xs text-nanna-text-muted">Confirming the backend can hear you.</p>
              </div>
            </div>

            <div
              class="rounded-lg border px-3 py-3 text-sm"
              :class="
                checking
                  ? 'border-white/10 bg-white/[0.03] text-nanna-text-muted'
                  : healthOk
                    ? 'border-emerald-500/25 bg-emerald-500/10 text-emerald-100'
                    : 'border-amber-500/25 bg-amber-500/10 text-amber-100'
              "
            >
              <span v-if="checking">Checking backend…</span>
              <span v-else class="flex items-start gap-2" data-testid="onboarding-health" :data-ok="healthOk">
                <Check v-if="healthOk" class="w-4 h-4 shrink-0 mt-0.5" />
                {{ healthDetail || 'Status unknown.' }}
              </span>
            </div>

            <div
              v-if="!checking && ollamaSummary"
              data-testid="ollama-probe"
              class="rounded-lg border px-3 py-3 text-sm space-y-2"
              :class="
                ollamaSummary.ok
                  ? 'border-emerald-500/25 bg-emerald-500/10 text-emerald-100'
                  : 'border-amber-500/25 bg-amber-500/10 text-amber-100'
              "
            >
              <span class="flex items-start gap-2">
                <Check v-if="ollamaSummary.ok" class="w-4 h-4 shrink-0 mt-0.5" />
                {{ ollamaSummary.text }}
              </span>
              <p v-if="ollamaSummary.fix" class="text-xs opacity-80">{{ ollamaSummary.fix }}</p>
              <ul v-if="ollamaSummary.pulls.length" class="space-y-1">
                <li v-for="pull in ollamaSummary.pulls" :key="pull">
                  <code class="block rounded bg-black/30 px-2 py-1 text-xs font-mono select-all">{{ pull }}</code>
                </li>
              </ul>
            </div>

            <div class="flex justify-between pt-2">
              <UiButton variant="ghost" size="sm" :disabled="checking" @click="step = 2">Back</UiButton>
              <div class="flex gap-2">
                <UiButton variant="secondary" size="sm" :disabled="checking" @click="runHealthCheck">
                  Recheck
                </UiButton>
                <UiButton size="sm" :disabled="checking" @click="finish">
                  Start chatting
                  <ArrowRight class="w-4 h-4" />
                </UiButton>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </Teleport>
</template>
