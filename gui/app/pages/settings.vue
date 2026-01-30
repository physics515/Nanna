<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <header class="px-4 sm:px-6 py-3 sm:py-4 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
      <div class="flex items-center gap-3 sm:gap-4">
        <NuxtLink to="/" class="text-nanna-text-muted hover:text-nanna-text transition-colors">
          <ArrowLeft class="w-5 h-5" />
        </NuxtLink>
        <h2 class="text-base sm:text-lg font-semibold text-nanna-text">Settings</h2>
        <div class="ml-auto flex gap-2">
          <UiButton v-if="hasChanges" @click="saveAllSettings" size="sm" :disabled="saving">
            <Save class="w-4 h-4 mr-1" />
            {{ saving ? 'Saving...' : 'Save' }}
          </UiButton>
        </div>
      </div>
    </header>
    
    <!-- Tabs -->
    <div class="px-4 sm:px-6 pt-4">
      <UiTabs v-model="activeTab" :tabs="tabs" />
    </div>
    
    <!-- Tab Content -->
    <div class="flex-1 overflow-y-auto p-4 sm:p-6">
      <div class="max-w-2xl mx-auto">
        
        <!-- Models Tab -->
        <UiTabPanel :active="activeTab === 'models'">
          <div class="space-y-6">
            <!-- API Keys -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-accent mb-4 flex items-center gap-2">
                <Key class="w-4 h-4" />
                API Keys
              </h3>
              <div class="space-y-4">
                <ApiKeyInput
                  label="Anthropic"
                  provider="anthropic"
                  placeholder="sk-ant-..."
                  :is-set="settings?.anthropic_key_set"
                  hint="For Claude models"
                  @save="saveApiKey"
                />
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
              </div>
            </UiCard>
            
            <!-- Provider & Model Selection -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-accent mb-4 flex items-center gap-2">
                <Brain class="w-4 h-4" />
                Model Configuration
              </h3>
              <div class="space-y-4">
                <!-- Provider -->
                <div>
                  <label class="block text-sm font-medium text-nanna-text-muted mb-2">Provider</label>
                  <div class="flex flex-wrap gap-2">
                    <UiButton
                      v-for="p in ['anthropic', 'openai', 'openrouter', 'ollama']"
                      :key="p"
                      @click="setProvider(p)"
                      :variant="settings?.provider === p ? 'default' : 'secondary'"
                      size="sm"
                    >
                      {{ formatProvider(p) }}
                    </UiButton>
                  </div>
                </div>
                
                <!-- Model -->
                <div>
                  <div class="flex items-center justify-between mb-2">
                    <label class="text-sm font-medium text-nanna-text-muted">Model</label>
                    <UiButton @click="refreshModels" :disabled="loadingModels" variant="ghost" size="sm">
                      <RefreshCw :class="['w-3 h-3', loadingModels && 'animate-spin']" />
                    </UiButton>
                  </div>
                  <UiSelect 
                    v-model="selectedModel" 
                    @update:model-value="updateModel"
                    :options="availableModels.map(m => ({ value: m.id, label: m.name }))"
                    :placeholder="loadingModels ? 'Loading...' : 'Select model'"
                  />
                </div>
                
                <!-- Ollama Host (if ollama selected) -->
                <div v-if="settings?.provider === 'ollama'">
                  <label class="block text-sm font-medium text-nanna-text-muted mb-1">Ollama Server</label>
                  <div class="flex gap-2">
                    <UiInput v-model="ollamaHostInput" placeholder="http://localhost:11434" class="flex-1" />
                    <UiButton @click="saveOllamaHost" size="sm">Save</UiButton>
                  </div>
                </div>
              </div>
            </UiCard>
          </div>
        </UiTabPanel>
        
        <!-- Memory Tab -->
        <UiTabPanel :active="activeTab === 'memory'">
          <div class="space-y-6">
            <!-- Embedding Configuration -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-accent mb-4 flex items-center gap-2">
                <Link class="w-4 h-4" />
                Embedding Configuration
              </h3>
              <div class="space-y-4">
                <!-- Provider -->
                <div>
                  <label class="block text-sm font-medium text-nanna-text-muted mb-2">Embedding Provider</label>
                  <div class="flex flex-wrap gap-2">
                    <UiButton
                      v-for="p in ['openai', 'ollama', 'disabled']"
                      :key="p"
                      @click="setEmbeddingProvider(p)"
                      :variant="settings?.embedding_provider === p ? 'accent' : 'secondary'"
                      size="sm"
                    >
                      {{ formatEmbeddingProvider(p) }}
                    </UiButton>
                  </div>
                </div>
                
                <!-- Model -->
                <div v-if="settings?.embedding_provider !== 'disabled'">
                  <label class="block text-sm font-medium text-nanna-text-muted mb-2">Embedding Model</label>
                  <UiSelect 
                    v-if="settings?.embedding_provider === 'openai'"
                    v-model="selectedEmbeddingModel" 
                    @update:model-value="updateEmbeddingModel"
                    :options="[
                      { value: 'text-embedding-3-small', label: 'text-embedding-3-small (1536 dims)' },
                      { value: 'text-embedding-3-large', label: 'text-embedding-3-large (3072 dims)' },
                    ]"
                  />
                  <UiSelect 
                    v-else-if="settings?.embedding_provider === 'ollama'"
                    v-model="selectedEmbeddingModel" 
                    @update:model-value="updateEmbeddingModel"
                    :options="ollamaModelOptions"
                    placeholder="Select embedding model"
                  />
                </div>
                
                <!-- Status -->
                <div class="flex items-center gap-2">
                  <UiBadge v-if="settings?.embedding_enabled" variant="success">✓ Memory recall enabled</UiBadge>
                  <UiBadge v-else variant="warning">⚠ Memory recall disabled</UiBadge>
                </div>
              </div>
            </UiCard>
            
            <!-- Memory Settings -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-accent mb-4 flex items-center gap-2">
                <BrainCircuit class="w-4 h-4" />
                Cognitive Memory
                <UiBadge variant="secondary" class="ml-auto">FSRS-6</UiBadge>
              </h3>
              <div class="space-y-4">
                <!-- Stats Grid -->
                <div class="grid grid-cols-4 gap-2">
                  <div class="p-2 rounded-lg bg-nanna-bg-elevated/50 text-center">
                    <div class="text-lg font-bold text-nanna-success">{{ memoryStats?.active || 0 }}</div>
                    <div class="text-xs text-nanna-text-dim">Active</div>
                  </div>
                  <div class="p-2 rounded-lg bg-nanna-bg-elevated/50 text-center">
                    <div class="text-lg font-bold text-nanna-warning">{{ memoryStats?.dormant || 0 }}</div>
                    <div class="text-xs text-nanna-text-dim">Dormant</div>
                  </div>
                  <div class="p-2 rounded-lg bg-nanna-bg-elevated/50 text-center">
                    <div class="text-lg font-bold text-nanna-text-muted">{{ memoryStats?.silent || 0 }}</div>
                    <div class="text-xs text-nanna-text-dim">Silent</div>
                  </div>
                  <div class="p-2 rounded-lg bg-nanna-bg-elevated/50 text-center">
                    <div class="text-lg font-bold text-nanna-error">{{ memoryStats?.unavailable || 0 }}</div>
                    <div class="text-xs text-nanna-text-dim">Faded</div>
                  </div>
                </div>
                
                <!-- Similarity Threshold -->
                <div class="p-3 rounded-lg bg-nanna-bg-elevated/50">
                  <div class="flex items-center justify-between mb-2">
                    <span class="text-sm font-medium text-nanna-text">Recall Threshold</span>
                    <span class="text-sm text-nanna-accent font-mono">{{ (similarityThreshold * 100).toFixed(0) }}%</span>
                  </div>
                  <input 
                    type="range" min="0" max="100" step="5"
                    :value="similarityThreshold * 100"
                    @change="setSimilarityThreshold(Number(($event.target as HTMLInputElement).value) / 100)"
                    class="w-full h-2 bg-nanna-bg-deep rounded-lg appearance-none cursor-pointer accent-nanna-primary"
                  >
                  <p class="text-xs text-nanna-text-dim mt-1">Lower = more results, higher = more precise</p>
                </div>
                
                <!-- Toggles -->
                <div class="space-y-3">
                  <div class="flex items-center justify-between">
                    <div>
                      <div class="text-sm font-medium text-nanna-text">Enable Dreaming</div>
                      <div class="text-xs text-nanna-text-dim">Memory consolidation</div>
                    </div>
                    <UiSwitch :model-value="settings?.dreaming_enabled" @update:model-value="setDreamingEnabled" />
                  </div>
                </div>
                
                <!-- Dream Button -->
                <UiButton @click="triggerConsolidation" :disabled="consolidating || !settings?.dreaming_enabled" class="w-full">
                  <UiSpinner v-if="consolidating" size="sm" class="mr-2" />
                  <Moon v-else class="w-4 h-4 mr-2" />
                  {{ consolidating ? 'Dreaming...' : 'Dream Now' }}
                </UiButton>
              </div>
            </UiCard>
          </div>
        </UiTabPanel>
        
        <!-- Tools Tab -->
        <UiTabPanel :active="activeTab === 'tools'">
          <div class="space-y-6">
            <!-- Tool API Keys -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-accent mb-4 flex items-center gap-2">
                <Key class="w-4 h-4" />
                Tool API Keys
              </h3>
              <div class="space-y-4">
                <ApiKeyInput
                  label="Brave Search"
                  provider="brave"
                  placeholder="BSA..."
                  :is-set="settings?.brave_key_set"
                  hint="For web_search tool"
                  @save="saveApiKey"
                />
              </div>
            </UiCard>
            
            <!-- Available Tools -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-accent mb-4 flex items-center gap-2">
                <Wrench class="w-4 h-4" />
                Available Tools
                <UiBadge variant="outline" class="ml-auto">{{ settings?.tools?.length || 0 }}</UiBadge>
              </h3>
              <div class="space-y-2">
                <div
                  v-for="tool in settings?.tools || []"
                  :key="tool.name"
                  class="flex items-center justify-between gap-2 p-3 rounded-lg bg-nanna-bg-elevated/50"
                >
                  <div class="flex items-center gap-2 min-w-0">
                    <span class="text-lg">{{ getToolIcon(tool.name) }}</span>
                    <div class="min-w-0">
                      <span class="text-sm font-medium text-nanna-text font-mono">{{ tool.name }}</span>
                      <p class="text-xs text-nanna-text-dim truncate">{{ tool.description }}</p>
                    </div>
                  </div>
                  <UiBadge :variant="tool.enabled ? 'success' : 'outline'" class="shrink-0">
                    {{ tool.enabled ? 'Active' : 'Off' }}
                  </UiBadge>
                </div>
              </div>
            </UiCard>
          </div>
        </UiTabPanel>
        
        <!-- Scheduler Tab -->
        <UiTabPanel :active="activeTab === 'scheduler'">
          <div class="space-y-6">
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-accent mb-4 flex items-center gap-2">
                <Clock class="w-4 h-4" />
                Scheduler Settings
              </h3>
              <div class="space-y-4">
                <div class="flex items-center justify-between">
                  <div>
                    <div class="text-sm font-medium text-nanna-text">Enable Scheduler</div>
                    <div class="text-xs text-nanna-text-dim">Background tasks</div>
                  </div>
                  <UiSwitch :model-value="settings?.scheduler_enabled" @update:model-value="setSchedulerEnabled" />
                </div>
                
                <div class="flex items-center justify-between">
                  <div>
                    <div class="text-sm font-medium text-nanna-text">Enable Heartbeats</div>
                    <div class="text-xs text-nanna-text-dim">Periodic self-checks</div>
                  </div>
                  <UiSwitch :model-value="settings?.heartbeat_enabled" @update:model-value="setHeartbeatEnabled" />
                </div>
                
                <!-- Heartbeat Interval -->
                <div class="p-3 rounded-lg bg-nanna-bg-elevated/50">
                  <div class="flex items-center justify-between mb-2">
                    <span class="text-sm font-medium text-nanna-text">Heartbeat Interval</span>
                    <span class="text-sm text-nanna-accent font-mono">{{ formatInterval(settings?.heartbeat_interval_seconds || 300) }}</span>
                  </div>
                  <input 
                    type="range" min="60" max="1800" step="60"
                    :value="settings?.heartbeat_interval_seconds || 300"
                    @change="setHeartbeatInterval(Number(($event.target as HTMLInputElement).value))"
                    class="w-full h-2 bg-nanna-bg-deep rounded-lg appearance-none cursor-pointer accent-nanna-primary"
                  >
                  <div class="flex justify-between text-xs text-nanna-text-dim mt-1">
                    <span>1 min</span>
                    <span>30 min</span>
                  </div>
                </div>
              </div>
            </UiCard>
          </div>
        </UiTabPanel>
        
        <!-- Data Tab -->
        <UiTabPanel :active="activeTab === 'data'">
          <div class="space-y-6">
            <!-- Sessions -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-accent mb-4 flex items-center gap-2">
                <Database class="w-4 h-4" />
                Data Management
              </h3>
              <div class="space-y-4">
                <div class="flex items-center justify-between p-3 rounded-lg bg-nanna-bg-elevated/50">
                  <div>
                    <div class="text-sm font-medium text-nanna-text">Chat Sessions</div>
                    <div class="text-xs text-nanna-text-dim">{{ sessionCount }} sessions stored</div>
                  </div>
                  <UiButton @click="confirmClearSessions" variant="destructive" size="sm">
                    <Trash2 class="w-4 h-4 mr-1" />
                    Clear All
                  </UiButton>
                </div>
                
                <div class="flex items-center justify-between p-3 rounded-lg bg-nanna-bg-elevated/50">
                  <div>
                    <div class="text-sm font-medium text-nanna-text">Memories</div>
                    <div class="text-xs text-nanna-text-dim">{{ memoryStats?.total_memories || 0 }} memories stored</div>
                  </div>
                  <UiButton @click="confirmClearMemories" variant="destructive" size="sm">
                    <Trash2 class="w-4 h-4 mr-1" />
                    Clear All
                  </UiButton>
                </div>
              </div>
            </UiCard>
            
            <!-- Import/Export -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-accent mb-4 flex items-center gap-2">
                <FileDown class="w-4 h-4" />
                Configuration
              </h3>
              <div class="space-y-3">
                <p class="text-sm text-nanna-text-muted">
                  Config file location:
                </p>
                <code class="block text-xs bg-nanna-bg-deep text-nanna-accent p-2 rounded font-mono break-all">
                  {{ configPath }}
                </code>
                <div class="flex gap-2">
                  <UiButton @click="exportConfig" variant="secondary" size="sm" class="flex-1">
                    <FileDown class="w-4 h-4 mr-1" />
                    Export
                  </UiButton>
                  <UiButton @click="importConfig" variant="secondary" size="sm" class="flex-1">
                    <FileUp class="w-4 h-4 mr-1" />
                    Import
                  </UiButton>
                </div>
              </div>
            </UiCard>
            
            <!-- About -->
            <UiCard>
              <h3 class="text-base font-semibold text-nanna-accent mb-4 flex items-center gap-2">
                <Moon class="w-4 h-4" />
                About Nanna
              </h3>
              <p class="text-sm text-nanna-text-muted italic mb-3">
                "I am the light that finds you in darkness, the memory that outlives the flesh."
              </p>
              <div class="space-y-2 text-sm">
                <div class="flex justify-between">
                  <span class="text-nanna-text-muted">Version</span>
                  <span class="text-nanna-text font-mono">0.1.0</span>
                </div>
                <div class="flex justify-between">
                  <span class="text-nanna-text-muted">Stack</span>
                  <span class="text-nanna-text">Tauri v2 + Nuxt v4 + Rust</span>
                </div>
              </div>
            </UiCard>
          </div>
        </UiTabPanel>
        
      </div>
    </div>
    
    <!-- Toast -->
    <Transition name="toast">
      <div 
        v-if="toast" 
        :class="[
          'fixed bottom-4 right-4 left-4 sm:left-auto px-4 py-3 rounded-lg shadow-lg flex items-center gap-2 max-w-sm mx-auto sm:mx-0 z-50',
          toast.type === 'success' ? 'bg-nanna-success text-nanna-bg-deep' : 'bg-nanna-error text-white'
        ]"
      >
        <CheckCircle v-if="toast.type === 'success'" class="w-4 h-4 shrink-0" />
        <XCircle v-else class="w-4 h-4 shrink-0" />
        <span class="text-sm">{{ toast.message }}</span>
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { 
  ArrowLeft, Key, Brain, Link, Wrench, BrainCircuit, Database, Moon, 
  RefreshCw, Trash2, CheckCircle, XCircle, Save, Clock, FileDown, FileUp
} from 'lucide-vue-next'

interface ToolInfo {
  name: string
  description: string
  enabled: boolean
}

interface ExtendedSettings {
  anthropic_key_set: boolean
  openai_key_set: boolean
  openrouter_key_set: boolean
  brave_key_set: boolean
  provider: string
  available_providers: string[]
  model: string
  embedding_provider: string
  embedding_model: string
  embedding_enabled: boolean
  ollama_host: string
  tools: ToolInfo[]
  dreaming_enabled: boolean
  scheduler_enabled: boolean
  heartbeat_enabled: boolean
  heartbeat_interval_seconds: number
}

interface CognitiveMemoryStats {
  total_memories: number
  active: number
  dormant: number
  silent: number
  unavailable: number
}

interface ModelInfo {
  id: string
  name: string
}

interface OllamaModelInfo {
  name: string
  size_mb: number
  is_embedding_model: boolean
}

const tabs = [
  { id: 'models', label: 'Models', icon: Brain },
  { id: 'memory', label: 'Memory', icon: BrainCircuit },
  { id: 'tools', label: 'Tools', icon: Wrench },
  { id: 'scheduler', label: 'Scheduler', icon: Clock },
  { id: 'data', label: 'Data', icon: Database },
]

const activeTab = ref('models')
const settings = ref<ExtendedSettings | null>(null)
const selectedModel = ref('')
const selectedEmbeddingModel = ref('')
const sessionCount = ref(0)
const ollamaModels = ref<OllamaModelInfo[]>([])
const loadingOllamaModels = ref(false)
const ollamaHostInput = ref('')
const availableModels = ref<ModelInfo[]>([])
const loadingModels = ref(false)
const memoryStats = ref<CognitiveMemoryStats | null>(null)
const consolidating = ref(false)
const toast = ref<{ message: string; type: 'success' | 'error' } | null>(null)
const similarityThreshold = ref(0.4)
const hasChanges = ref(false)
const saving = ref(false)

const configPath = computed(() => {
  if (navigator.platform.includes('Win')) {
    return '%APPDATA%\\clawd\\Nanna\\config\\config.toml'
  } else if (navigator.platform.includes('Mac')) {
    return '~/Library/Application Support/clawd.Nanna/config.toml'
  } else {
    return '~/.config/nanna/config.toml'
  }
})

const ollamaModelOptions = computed(() => {
  return ollamaModels.value.map(m => ({
    value: m.name,
    label: `${m.name} (${m.size_mb}MB)${m.is_embedding_model ? ' ★' : ''}`
  }))
})

onMounted(async () => {
  await loadSettings()
  await loadSessions()
  await loadMemoryStats()
  await loadSimilarityThreshold()
})

async function loadSettings() {
  try {
    settings.value = await invoke<ExtendedSettings>('get_extended_settings')
    selectedModel.value = settings.value.model
    selectedEmbeddingModel.value = settings.value.embedding_model
    ollamaHostInput.value = settings.value.ollama_host
    await refreshModels()
    if (settings.value.embedding_provider === 'ollama') {
      await refreshOllamaModels()
    }
  } catch (e) {
    console.error('Failed to load settings:', e)
    showToast('Failed to load settings', 'error')
  }
}

async function refreshModels() {
  if (!settings.value) return
  loadingModels.value = true
  try {
    const provider = settings.value.provider
    let models: ModelInfo[] = []
    
    if (provider === 'anthropic') {
      models = await invoke<ModelInfo[]>('get_anthropic_models')
    } else if (provider === 'openai') {
      models = await invoke<ModelInfo[]>('get_openai_models')
    } else if (provider === 'ollama') {
      const ollamaList = await invoke<OllamaModelInfo[]>('get_ollama_models')
      models = ollamaList.filter(m => !m.is_embedding_model).map(m => ({ id: m.name, name: `${m.name} (${m.size_mb}MB)` }))
    } else if (provider === 'openrouter') {
      models = [
        { id: 'anthropic/claude-sonnet-4-20250514', name: 'Claude Sonnet 4' },
        { id: 'deepseek/deepseek-chat', name: 'DeepSeek Chat' },
        { id: 'google/gemini-2.5-flash-preview-05-20', name: 'Gemini 2.5 Flash' },
      ]
    }
    availableModels.value = models
    if (models.length > 0 && !models.find(m => m.id === selectedModel.value)) {
      availableModels.value.unshift({ id: selectedModel.value, name: `${selectedModel.value} (current)` })
    }
  } catch (e: any) {
    availableModels.value = [{ id: settings.value.model, name: `${settings.value.model} (current)` }]
  } finally {
    loadingModels.value = false
  }
}

async function refreshOllamaModels() {
  loadingOllamaModels.value = true
  try {
    ollamaModels.value = await invoke<OllamaModelInfo[]>('get_ollama_models')
  } catch (e: any) {
    showToast(`Ollama: ${e.message || e}`, 'error')
    ollamaModels.value = []
  } finally {
    loadingOllamaModels.value = false
  }
}

async function loadSessions() {
  try {
    const sessions = await invoke<{ id: string }[]>('list_sessions')
    sessionCount.value = sessions.length
  } catch (e) {
    console.error('Failed to load sessions:', e)
  }
}

async function loadMemoryStats() {
  try {
    memoryStats.value = await invoke<CognitiveMemoryStats>('get_cognitive_memory_stats')
  } catch (e) {
    console.error('Failed to load memory stats:', e)
  }
}

async function loadSimilarityThreshold() {
  try {
    similarityThreshold.value = await invoke<number>('get_similarity_threshold')
  } catch (e) {
    console.error('Failed to load similarity threshold:', e)
  }
}

async function saveApiKey(provider: string, apiKey: string) {
  try {
    await invoke('set_provider_api_key', { provider, apiKey })
    showToast(`${formatProvider(provider)} API key saved`, 'success')
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

async function setSimilarityThreshold(value: number) {
  try {
    await invoke<string>('set_similarity_threshold', { threshold: value })
    similarityThreshold.value = value
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setDreamingEnabled(enabled: boolean) {
  try {
    await invoke('set_dreaming_enabled', { enabled })
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setSchedulerEnabled(enabled: boolean) {
  try {
    await invoke('set_scheduler_enabled', { enabled })
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setHeartbeatEnabled(enabled: boolean) {
  try {
    await invoke('set_heartbeat_enabled', { enabled })
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function setHeartbeatInterval(seconds: number) {
  try {
    await invoke('set_heartbeat_interval', { seconds })
    await loadSettings()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function triggerConsolidation() {
  consolidating.value = true
  try {
    const result = await invoke<{ memories_processed: number; memories_merged: number }>('trigger_consolidation')
    showToast(`Dreaming complete: ${result.memories_processed} processed`, 'success')
    await loadMemoryStats()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  } finally {
    consolidating.value = false
  }
}

async function confirmClearSessions() {
  if (!confirm('Delete all chat sessions? This cannot be undone.')) return
  try {
    const sessions = await invoke<{ id: string }[]>('list_sessions')
    for (const session of sessions) {
      await invoke('delete_session', { sessionId: session.id })
    }
    showToast('All sessions cleared', 'success')
    sessionCount.value = 0
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function confirmClearMemories() {
  if (!confirm('Delete all memories? This cannot be undone.')) return
  try {
    await invoke('clear_all_memories')
    showToast('All memories cleared', 'success')
    await loadMemoryStats()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function saveAllSettings() {
  saving.value = true
  try {
    await invoke('save_config')
    showToast('Settings saved', 'success')
    hasChanges.value = false
  } catch (e: any) {
    showToast(`Failed to save: ${e.message || e}`, 'error')
  } finally {
    saving.value = false
  }
}

function exportConfig() {
  showToast('Export coming soon', 'success')
}

function importConfig() {
  showToast('Import coming soon', 'success')
}

function formatInterval(seconds: number): string {
  if (seconds < 60) return `${seconds}s`
  return `${Math.floor(seconds / 60)} min`
}

function formatProvider(provider: string): string {
  const names: Record<string, string> = {
    anthropic: 'Anthropic', openai: 'OpenAI', openrouter: 'OpenRouter', ollama: 'Ollama',
  }
  return names[provider] || provider
}

function formatEmbeddingProvider(provider: string): string {
  const names: Record<string, string> = {
    openai: 'OpenAI', ollama: 'Ollama', disabled: 'Disabled',
  }
  return names[provider] || provider
}

function getToolIcon(name: string): string {
  const icons: Record<string, string> = {
    read_file: '📄', write_file: '✏️', list_dir: '📁', exec: '⚡',
    web_fetch: '🌐', web_search: '🔍', echo: '💬', analyze_image: '👁️',
    ocr: '📝', describe_image: '🖼️', read_pdf: '📑',
  }
  return icons[name] || '🔧'
}

function showToast(message: string, type: 'success' | 'error') {
  toast.value = { message, type }
  setTimeout(() => { toast.value = null }, 3000)
}
</script>

<style scoped>
.toast-enter-active,
.toast-leave-active {
  transition: all 0.3s ease;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(10px);
}
</style>
