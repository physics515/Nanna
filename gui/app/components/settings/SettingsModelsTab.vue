<template>
  <div class="space-y-6">
    <!-- API Keys -->
    <UiCard>
      <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
        <Key class="w-4 h-4" />
        Providers
      </h3>
      <div class="space-y-4">
        <!-- Anthropic (OAuth only, via claude setup-token) -->
        <div class="space-y-3">
          <div class="flex items-center justify-between">
            <label class="text-sm font-medium text-nanna-text">Anthropic</label>
            <UiBadge v-if="settings?.anthropic_oauth_logged_in" variant="success">Logged In</UiBadge>
          </div>

          <!-- Logged in state -->
          <div v-if="settings?.anthropic_oauth_logged_in" class="space-y-3">
            <div class="flex items-center justify-between p-3 rounded-lg bg-nanna-success/10 border border-nanna-success/30">
              <div class="flex items-center gap-2">
                <CheckCircle class="w-4 h-4 text-nanna-success" />
                <span class="text-sm text-nanna-text">Authenticated via Claude Code</span>
              </div>
              <UiButton @click="logoutAnthropic" variant="ghost" size="sm">
                <LogOut class="w-4 h-4 mr-1" />
                Logout
              </UiButton>
            </div>
          </div>

          <!-- Not logged in state -->
          <div v-else class="space-y-3">
            <p class="text-xs text-nanna-text-muted">
              Run <code class="bg-nanna-bg-elevated px-1 rounded">claude setup-token</code> and paste the token below.
            </p>

            <!-- Token input box -->
            <div class="flex gap-2">
              <UiInput
                v-model="oauthTokenInput"
                type="password"
                placeholder="Paste token from claude setup-token..."
                class="flex-1 font-mono text-sm"
              />
              <UiButton @click="saveOAuthToken" :disabled="!oauthTokenInput.trim() || oauthLoading" size="sm">
                <UiSpinner v-if="oauthLoading" size="sm" class="mr-1" />
                {{ oauthLoading ? 'Saving...' : 'Save' }}
              </UiButton>
            </div>

            <!-- Helper buttons -->
            <div class="flex gap-2">
              <UiButton @click="runClaudeSetupToken" :disabled="oauthLoading" variant="outline" size="sm" class="flex-1">
                <UiSpinner v-if="oauthLoading && oauthAction === 'setup'" size="sm" class="mr-1" />
                <Terminal v-else class="w-3 h-3 mr-1" />
                {{ oauthLoading && oauthAction === 'setup' ? 'Running...' : 'Run CLI' }}
              </UiButton>
              <UiButton @click="importClaudeCodeCredentials" :disabled="oauthLoading" variant="outline" size="sm" class="flex-1">
                <UiSpinner v-if="oauthLoading && oauthAction === 'import'" size="sm" class="mr-1" />
                <Download v-else class="w-3 h-3 mr-1" />
                {{ oauthLoading && oauthAction === 'import' ? 'Importing...' : 'Import' }}
              </UiButton>
            </div>
          </div>
        </div>

        <ApiKeyInput
          label="OpenAI"
          provider="openai"
          placeholder="sk-..."
          :is-set="settings?.openai_key_set"
          hint="For GPT models and embeddings"
          @save="saveApiKey"
        />
        <ApiKeyInput
          label="OpenRouter"
          provider="openrouter"
          placeholder="sk-or-..."
          :is-set="settings?.openrouter_key_set"
          hint="For multi-provider access"
          @save="saveApiKey"
        />
        <ApiKeyInput
          label="GitHub Models"
          provider="github"
          placeholder="ghp_..."
          :is-set="settings?.github_key_set"
          hint="Use Copilot models via GitHub token"
          @save="saveApiKey"
        />

        <!-- Claude Proxy (claude-max-api-proxy) -->
        <div class="space-y-2 p-3 rounded-lg bg-nanna-bg-elevated/40 border border-nanna-border/30">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-2">
              <span class="text-sm font-medium text-nanna-text">Claude Proxy</span>
              <span
                v-if="claudeProxyHealthy"
                class="px-1.5 py-0.5 text-[10px] rounded bg-nanna-success/20 text-nanna-success"
              >Online</span>
              <span
                v-else-if="settings?.claude_proxy_enabled"
                class="px-1.5 py-0.5 text-[10px] rounded bg-nanna-error/20 text-nanna-error"
              >Offline</span>
            </div>
            <label class="relative inline-flex items-center cursor-pointer">
              <input
                type="checkbox"
                :checked="settings?.claude_proxy_enabled"
                @change="toggleClaudeProxy"
                class="sr-only peer"
                aria-label="Claude OAuth proxy"
              >
              <div class="w-9 h-5 bg-nanna-bg-elevated peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-nanna-text-muted after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-nanna-primary peer-checked:after:bg-white"></div>
            </label>
          </div>
          <p class="text-xs text-nanna-text-muted">
            Route through claude-max-api-proxy to use Claude Pro/Max subscription
          </p>
          <div v-if="settings?.claude_proxy_enabled" class="flex gap-2 mt-2">
            <input
              v-model="claudeProxyUrl"
              type="text"
              placeholder="http://localhost:3456"
              class="flex-1 px-2 py-1.5 text-sm bg-nanna-bg rounded border border-nanna-border/50 text-nanna-text placeholder:text-nanna-text-muted/50 focus:outline-none focus:border-nanna-primary"
            >
            <UiButton @click="saveClaudeProxyUrl" size="sm" variant="ghost">Save</UiButton>
          </div>
        </div>

        <!-- Ollama -->
        <div class="space-y-3 p-3 rounded-lg bg-nanna-bg-elevated/40 border border-nanna-border/30">
          <div class="flex items-center justify-between">
            <span class="text-sm font-medium text-nanna-text">Ollama</span>
            <div class="flex items-center gap-2">
              <span v-if="ollamaStatus === 'connected'" class="flex items-center gap-1 text-xs text-green-500">
                <CheckCircle class="w-3 h-3" /> {{ ollamaModels.length }} model{{ ollamaModels.length !== 1 ? 's' : '' }}
              </span>
              <span v-else-if="ollamaStatus === 'error'" class="flex items-center gap-1 text-xs text-red-400">
                <XCircle class="w-3 h-3" /> Offline
              </span>
              <UiButton aria-label="Refresh Ollama models" title="Refresh Ollama models" @click="refreshOllamaModels" size="sm" variant="ghost" :disabled="loadingOllamaModels">
                <RefreshCw class="w-3 h-3" :class="{ 'animate-spin': loadingOllamaModels }" />
              </UiButton>
            </div>
          </div>
          <div>
            <label class="block text-xs text-nanna-text-dim mb-1">Server URL</label>
            <div class="flex gap-2">
              <UiInput v-model="ollamaHostInput" placeholder="http://localhost:11434" class="flex-1" />
              <UiButton @click="saveOllamaHost" size="sm">Save</UiButton>
            </div>
          </div>
          <div>
            <label class="block text-xs text-nanna-text-dim mb-1">API Key <span class="text-nanna-text-dim/60">(optional)</span></label>
            <div class="flex gap-2">
              <UiInput v-model="ollamaApiKeyInput" type="password" placeholder="For remote/authenticated instances" class="flex-1" />
              <UiButton @click="saveOllamaApiKey" size="sm">Save</UiButton>
            </div>
          </div>
        </div>
      </div>
    </UiCard>

    <!-- Model Priority (Fallback Chain) -->
    <UiCard>
      <div class="flex items-center justify-between mb-4">
        <h3 class="text-base font-semibold text-nanna-primary flex items-center gap-2">
          <Brain class="w-4 h-4" />
          Chat Models
        </h3>
        <UiButton aria-label="Refresh models" title="Refresh models" @click="refreshAllModels" :disabled="loadingModels" variant="ghost" size="sm">
          <RefreshCw :class="['w-3 h-3', loadingModels && 'animate-spin']" />
        </UiButton>
      </div>

      <ModelPriorityList
        label="Model Priority"
        hint="First working model is used. Drag to reorder fallback priority."
        :all-models="allChatModels"
        v-model="chatModelPriority"
        @update:model-value="saveChatModelPriority"
      />

      <!-- No Models Warning -->
      <div v-if="allChatModels.length === 0" class="mt-4 p-4 rounded-lg bg-nanna-warning/10 border border-nanna-warning/30">
        <div class="flex items-start gap-3">
          <AlertTriangle class="w-5 h-5 text-nanna-warning shrink-0 mt-0.5" />
          <div>
            <div class="font-medium text-nanna-warning">No models available</div>
            <p class="text-sm text-nanna-text-muted mt-1">
              Set up an API key below, or configure Ollama for local models.
            </p>
          </div>
        </div>
      </div>

      <!-- Summarization Models -->
      <div class="mt-6 pt-6 border-t border-white/[0.04]">
        <h3 class="text-sm font-semibold text-nanna-primary mb-3 flex items-center gap-2">
          <Layers class="w-4 h-4" />
          Context Summarization
        </h3>
        <p class="text-xs text-nanna-text-dim mb-3">
          When conversation context exceeds the chat model's limit, these models recursively summarize older content until it fits. Any chat model works; local Ollama models avoid API costs.
        </p>
        <ModelPriorityList
          label="Summarization Model Priority"
          hint="Used to compress context when it exceeds limits. Empty = truncate instead of summarize."
          :all-models="allSummarizationModels"
          v-model="summarizationModelPriority"
          @update:model-value="saveSummarizationModelPriority"
        />
      </div>

      <!-- Embedding Models -->
      <div class="mt-6 pt-6 border-t border-white/[0.04]">
        <h3 class="text-sm font-semibold text-nanna-primary mb-3 flex items-center gap-2">
          <Link class="w-4 h-4" />
          Embedding Models
        </h3>
        <p class="text-xs text-nanna-text-dim mb-3">
          Used for semantic memory recall. First working model is used.
        </p>
        <ModelPriorityList
          label="Embedding Priority"
          hint="Used for memory recall. First working model is used."
          :all-models="allEmbeddingModels"
          v-model="embeddingModelPriority"
          @update:model-value="saveEmbeddingModelPriority"
        />
        <div v-if="allEmbeddingModels.length === 0" class="mt-3 p-3 rounded-lg bg-nanna-warning/10 border border-nanna-warning/30">
          <div class="flex items-start gap-2">
            <AlertTriangle class="w-4 h-4 text-nanna-warning shrink-0 mt-0.5" />
            <p class="text-xs text-nanna-warning">No embedding models available. Set up an API key or install Ollama embedding models.</p>
          </div>
        </div>
        <div v-else class="flex items-center gap-2 mt-3">
          <UiBadge v-if="embeddingModelPriority.length > 0" variant="success">✓ Memory recall enabled</UiBadge>
          <UiBadge v-else variant="warning">⚠ No embedding models selected — memory recall disabled</UiBadge>
        </div>
      </div>

      <!-- OCR Models -->
      <div class="mt-6 pt-6 border-t border-white/[0.04]">
        <h3 class="text-sm font-semibold text-nanna-primary mb-3 flex items-center gap-2">
          <ScanText class="w-4 h-4" />
          OCR Models
        </h3>
        <p class="text-xs text-nanna-text-dim mb-3">
          Used to extract text from images and scanned PDFs. Tier 0 is the built-in <code>ocrs</code> engine (offline, no API cost). Tier 1+ are vision-capable models tried in order.
        </p>

        <!-- Embedded OCR toggle -->
        <div class="flex items-center justify-between mb-4 p-3 rounded-lg bg-white/[0.02] border border-white/[0.05]">
          <div>
            <div class="text-sm font-medium text-nanna-text">Use embedded OCR first (Tier 0)</div>
            <div class="text-xs text-nanna-text-dim mt-0.5">Runs offline using the <code>ocrs</code> ONNX engine — models auto-downloaded to <code>~/.cache/ocrs/</code> on first use (~50 MB). Latin script only.</div>
          </div>
          <UiSwitch
            :model-value="useEmbeddedOcr"
            label="Embedded OCR"
            @update:model-value="saveUseEmbeddedOcr"
          />
        </div>

        <!-- Vision model fallback list -->
        <ModelPriorityList
          label="Vision Model Fallback (Tier 1+)"
          hint="Only vision-capable models shown. Used when embedded OCR fails or returns no text."
          :all-models="allOcrModels"
          v-model="ocrModelPriority"
          @update:model-value="saveOcrModelPriority"
        />

        <div v-if="allOcrModels.length === 0" class="mt-3 p-3 rounded-lg bg-nanna-warning/10 border border-nanna-warning/30">
          <div class="flex items-start gap-2">
            <AlertTriangle class="w-4 h-4 text-nanna-warning shrink-0 mt-0.5" />
            <p class="text-xs text-nanna-warning">No vision-capable models available. Install a vision Ollama model (e.g. llava) or set up an Anthropic/OpenAI API key.</p>
          </div>
        </div>
        <div v-else class="flex items-center gap-2 mt-3">
          <UiBadge v-if="useEmbeddedOcr" variant="success">✓ Embedded OCR active</UiBadge>
          <UiBadge v-if="ocrModelPriority.length > 0" variant="success">✓ {{ ocrModelPriority.length }} vision model{{ ocrModelPriority.length !== 1 ? 's' : '' }} in fallback chain</UiBadge>
          <UiBadge v-if="!useEmbeddedOcr && ocrModelPriority.length === 0" variant="warning">⚠ No OCR methods configured</UiBadge>
        </div>
      </div>

    </UiCard>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  Key, CheckCircle, XCircle, LogOut, Terminal, Download, RefreshCw,
  Brain, AlertTriangle, Layers, Link, ScanText
} from 'lucide-vue-next'
import { useSettingsPage } from '~/composables/useSettingsPage'

const store = useSettingsPage()
const {
  settings,
  ollamaModels, loadingOllamaModels, ollamaStatus, loadingModels,
  claudeProxyHealthy,
  allChatModels, allEmbeddingModels, allSummarizationModels, allOcrModels,
  loadSettings, refreshModels, refreshOllamaModels,
  saveApiKey, formatProvider, showToast,
} = store

const selectedModel = ref('')
const selectedEmbeddingModel = ref('')
const ollamaHostInput = ref('')
const ollamaApiKeyInput = ref('')

// Model priority lists (fallback chains)
const chatModelPriority = ref<string[]>([])
const embeddingModelPriority = ref<string[]>([])
const summarizationModelPriority = ref<string[]>([])

// OCR settings
const ocrModelPriority = ref<string[]>([])
const useEmbeddedOcr = ref(true)

// Anthropic OAuth state
const oauthTokenInput = ref('')
const oauthLoading = ref(false)
const oauthAction = ref<'setup' | 'import' | null>(null)

// Claude Proxy state
const claudeProxyUrl = ref('http://localhost:3456')

const ollamaModelOptions = computed(() => {
  return ollamaModels.value.map(m => ({
    value: m.name,
    label: `${m.name} (${m.size_mb}MB)${m.is_embedding_model ? ' ★' : ''}`
  }))
})

// Repopulate tab state whenever settings are (re)loaded
store.onSettingsLoaded(async () => {
  if (!settings.value) return
  selectedModel.value = settings.value.model
  selectedEmbeddingModel.value = settings.value.embedding_model
  ollamaHostInput.value = settings.value.ollama_host
  ollamaApiKeyInput.value = settings.value.ollama_api_key || ''
  claudeProxyUrl.value = settings.value.claude_proxy_url || 'http://localhost:3456'

  // Load model priority lists
  try {
    chatModelPriority.value = await invoke<string[]>('get_chat_model_priority')
  } catch {
    // Default to current model if priority not set
    chatModelPriority.value = settings.value.model ? [settings.value.model] : []
  }
  try {
    embeddingModelPriority.value = await invoke<string[]>('get_embedding_model_priority')
  } catch {
    // Default based on current embedding config
    if (settings.value.embedding_provider !== 'disabled' && settings.value.embedding_model) {
      embeddingModelPriority.value = [`${settings.value.embedding_provider}/${settings.value.embedding_model}`]
    } else {
      embeddingModelPriority.value = []
    }
  }
  try {
    summarizationModelPriority.value = await invoke<string[]>('get_summarization_model_priority')
  } catch {
    // Default to empty (truncate instead of summarize)
    summarizationModelPriority.value = []
  }
  try {
    ocrModelPriority.value = await invoke<string[]>('get_ocr_model_priority')
  } catch {
    ocrModelPriority.value = []
  }
  try {
    useEmbeddedOcr.value = await invoke<boolean>('get_use_embedded_ocr')
  } catch {
    useEmbeddedOcr.value = true
  }
})

async function refreshAllModels() {
  loadingModels.value = true
  try {
    // Refresh all model sources in parallel
    await Promise.all([
      refreshOllamaModels(),
      refreshModels(), // Fetches Anthropic and OpenAI
    ])
  } finally {
    loadingModels.value = false
  }
}

async function saveChatModelPriority(priority: string[]) {
  try {
    await invoke('set_chat_model_priority', { priority })
    showToast('Chat model priority saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveEmbeddingModelPriority(priority: string[]) {
  try {
    await invoke('set_embedding_model_priority', { priority })
    showToast('Embedding model priority saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveSummarizationModelPriority(priority: string[]) {
  try {
    await invoke('set_summarization_model_priority', { priority })
    showToast('Summarization model priority saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveOcrModelPriority(priority: string[]) {
  try {
    await invoke('set_ocr_model_priority', { priority })
    showToast('OCR model priority saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveUseEmbeddedOcr(enabled: boolean) {
  useEmbeddedOcr.value = enabled
  try {
    await invoke('set_use_embedded_ocr', { enabled })
    showToast(`Embedded OCR ${enabled ? 'enabled' : 'disabled'}`, 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

// =============================================================================
// Claude Proxy (claude-max-api-proxy)
// =============================================================================

async function toggleClaudeProxy(event: Event) {
  const enabled = (event.target as HTMLInputElement).checked
  try {
    await invoke('set_claude_proxy', { enabled, url: claudeProxyUrl.value })
    await loadSettings()
    if (enabled) {
      await refreshModels()
    }
    showToast(enabled ? 'Claude Proxy enabled' : 'Claude Proxy disabled', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveClaudeProxyUrl() {
  try {
    await invoke('set_claude_proxy', { enabled: true, url: claudeProxyUrl.value })
    await refreshModels()
    showToast('Claude Proxy URL saved', 'success')
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

// =============================================================================
// Anthropic OAuth Token
// =============================================================================

async function saveOAuthToken() {
  const token = oauthTokenInput.value.trim()
  if (!token) return

  try {
    await invoke('save_anthropic_oauth_token', { token })
    oauthTokenInput.value = ''
    showToast('Anthropic token saved', 'success')
    await loadSettings()
    await refreshModels()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function runClaudeSetupToken() {
  oauthLoading.value = true
  oauthAction.value = 'setup'
  try {
    const result = await invoke<string>('run_claude_setup_token')
    showToast(result || 'Successfully authenticated via Claude Code CLI', 'success')
    await loadSettings()
    await refreshModels()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  } finally {
    oauthLoading.value = false
    oauthAction.value = null
  }
}

async function importClaudeCodeCredentials() {
  oauthLoading.value = true
  oauthAction.value = 'import'
  try {
    await invoke('import_claude_code_credentials')
    showToast('Successfully imported Claude Code credentials', 'success')
    await loadSettings()
    await refreshModels()
  } catch (e: any) {
    showToast(`Failed to import: ${e.message || e}`, 'error')
  } finally {
    oauthLoading.value = false
    oauthAction.value = null
  }
}

async function logoutAnthropic() {
  try {
    await invoke('logout_anthropic_oauth')
    showToast('Logged out of Anthropic', 'success')
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setProvider(provider: string) {
  try {
    await invoke('set_provider', { provider })
    showToast(`Switched to ${formatProvider(provider)}`, 'success')
    await loadSettings()
    await refreshModels()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function updateModel() {
  try {
    await invoke('set_model', { model: selectedModel.value })
    showToast('Model updated', 'success')
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveOllamaHost() {
  try {
    await invoke('set_ollama_host', { host: ollamaHostInput.value })
    showToast('Ollama host saved', 'success')
    await refreshOllamaModels()
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveOllamaApiKey() {
  try {
    await invoke('set_ollama_api_key', { key: ollamaApiKeyInput.value })
    showToast('Ollama API key saved', 'success')
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setEmbeddingProvider(provider: string) {
  try {
    if (provider === 'ollama') await refreshOllamaModels()
    let defaultModel = 'none'
    if (provider === 'openai') defaultModel = 'text-embedding-3-small'
    else if (provider === 'ollama') {
      const embeddingModel = ollamaModels.value.find(m => m.is_embedding_model)
      defaultModel = embeddingModel?.name || ollamaModels.value[0]?.name || 'nomic-embed-text'
    }
    await invoke<string>('set_embedding_config', { provider, model: defaultModel })
    showToast('Embedding config updated', 'success')
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function updateEmbeddingModel() {
  try {
    const provider = settings.value?.embedding_provider || 'disabled'
    await invoke<string>('set_embedding_config', { provider, model: selectedEmbeddingModel.value })
    showToast('Embedding model updated', 'success')
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

function formatEmbeddingProvider(provider: string): string {
  const names: Record<string, string> = {
    openai: 'OpenAI', ollama: 'Ollama', disabled: 'Disabled',
  }
  return names[provider] || provider
}
</script>
