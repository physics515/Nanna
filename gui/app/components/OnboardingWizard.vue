<script setup lang="ts">
/**
 * Minimal 3-step first-run wizard.
 * Persists `nanna.onboarding.done=1` on finish/skip.
 */
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ArrowRight, Check, HeartPulse, KeyRound, Sparkles, X } from 'lucide-vue-next'

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
const error = ref<string | null>(null)
const healthOk = ref(false)
const healthDetail = ref('')

const providers = [
  { value: 'anthropic', label: 'Anthropic' },
  { value: 'openai', label: 'OpenAI' },
  { value: 'openrouter', label: 'OpenRouter' },
  { value: 'ollama', label: 'Ollama (local)' },
]

const needsKey = computed(() => provider.value !== 'ollama')

watch(
  () => props.open,
  (open) => {
    if (open) {
      step.value = 1
      error.value = null
      keySaved.value = !!props.hasApiKey
      healthOk.value = false
      healthDetail.value = ''
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
  error.value = null
  try {
    await invoke('set_provider_api_key', { provider: p, apiKey: key })
    try { await invoke('set_provider', { provider: p }) } catch { /* non-fatal */ }
    keySaved.value = true
    step.value = 3
    await runHealthCheck()
  } catch (e: any) {
    error.value = e?.message || String(e) || "Couldn't save that key."
  } finally {
    saving.value = false
  }
}

async function continueWithoutKey() {
  error.value = null
  if (provider.value === 'ollama') {
    try {
      saving.value = true
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
  try {
    const status = await invoke<{ running?: boolean; version?: string; error?: string } | string>('get_backend_status')
    if (typeof status === 'string') {
      healthOk.value = true
      healthDetail.value = status
    } else if (status && (status.running !== false)) {
      healthOk.value = true
      healthDetail.value = status.version
        ? `Backend ready · ${status.version}`
        : 'Backend is reachable.'
    } else {
      healthOk.value = false
      healthDetail.value = status?.error || 'Backend is not ready yet — you can still start chatting.'
    }
  } catch (e: any) {
    healthOk.value = false
    healthDetail.value = e?.message || "Couldn't reach the backend. You can still start and fix this in Settings."
  } finally {
    checking.value = false
  }
}
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
        class="relative w-full max-w-md rounded-2xl border border-white/[0.08] bg-nanna-bg-elevated shadow-2xl shadow-black/40 overflow-hidden"
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
                <p class="text-xs text-nanna-text-muted">Pick a provider and add a key, or use Ollama locally.</p>
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
              :external-error="error"
              :hint="hasApiKey ? 'A key is already saved. You can replace it or continue.' : undefined"
              @save="onKeySave"
            />
            <p v-else class="text-xs text-nanna-text-muted">
              Ollama runs locally — no API key needed. Ensure Ollama is running on this machine.
            </p>

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
              <span v-else class="flex items-start gap-2">
                <Check v-if="healthOk" class="w-4 h-4 shrink-0 mt-0.5" />
                {{ healthDetail || 'Status unknown.' }}
              </span>
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
