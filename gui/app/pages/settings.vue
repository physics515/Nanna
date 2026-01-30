<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <header class="px-6 py-4 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
      <div class="flex items-center gap-4">
        <NuxtLink to="/" class="text-nanna-text-muted hover:text-nanna-text">
          ← Back
        </NuxtLink>
        <h2 class="text-lg font-semibold text-nanna-text">Settings</h2>
      </div>
    </header>
    
    <!-- Settings content -->
    <div class="flex-1 overflow-y-auto p-6">
      <div class="max-w-2xl mx-auto space-y-8">
        
        <!-- API Keys Section -->
        <section class="card">
          <h3 class="text-lg font-semibold text-nanna-accent mb-4 flex items-center gap-2">
            🔑 API Keys
          </h3>
          
          <div class="space-y-4">
            <!-- Anthropic -->
            <ApiKeyInput
              label="Anthropic"
              provider="anthropic"
              placeholder="sk-ant-..."
              :is-set="settings?.anthropic_key_set"
              hint="For Claude models. Get key from console.anthropic.com"
              @save="saveApiKey"
            />
            
            <!-- OpenAI -->
            <ApiKeyInput
              label="OpenAI"
              provider="openai"
              placeholder="sk-..."
              :is-set="settings?.openai_key_set"
              hint="For GPT models. Get key from platform.openai.com"
              @save="saveApiKey"
            />
            
            <!-- Brave Search -->
            <ApiKeyInput
              label="Brave Search"
              provider="brave"
              placeholder="BSA..."
              :is-set="settings?.brave_key_set"
              hint="For web search tool. Get key from brave.com/search/api"
              @save="saveApiKey"
            />
          </div>
        </section>
        
        <!-- Provider & Model Section -->
        <section class="card">
          <h3 class="text-lg font-semibold text-nanna-accent mb-4 flex items-center gap-2">
            🧠 Model Configuration
          </h3>
          
          <div class="space-y-4">
            <!-- Provider Selection -->
            <div>
              <label class="block text-sm font-medium text-nanna-text-muted mb-2">
                Provider
              </label>
              <div class="flex gap-2">
                <button
                  v-for="p in settings?.available_providers || []"
                  :key="p"
                  @click="setProvider(p)"
                  :class="[
                    'px-4 py-2 rounded-lg text-sm font-medium transition-colors',
                    settings?.provider === p
                      ? 'bg-nanna-primary text-white'
                      : 'bg-nanna-bg-elevated text-nanna-text-muted hover:text-nanna-text'
                  ]"
                >
                  {{ formatProvider(p) }}
                </button>
              </div>
            </div>
            
            <!-- Model Selection -->
            <div>
              <label class="block text-sm font-medium text-nanna-text-muted mb-2">
                Model
              </label>
              <select 
                v-model="selectedModel" 
                @change="updateModel"
                class="input"
              >
                <optgroup v-for="group in groupedModels" :key="group.provider" :label="group.label">
                  <option v-for="model in group.models" :key="model" :value="model">
                    {{ formatModelName(model) }}
                  </option>
                </optgroup>
              </select>
              <p class="text-xs text-nanna-text-dim mt-2">
                Current: <span class="text-nanna-accent">{{ formatModelName(settings?.model || 'Loading...') }}</span>
              </p>
            </div>
          </div>
        </section>
        
        <!-- Tools Section -->
        <section class="card">
          <h3 class="text-lg font-semibold text-nanna-accent mb-4 flex items-center gap-2">
            🛠️ Available Tools
            <span class="text-sm font-normal text-nanna-text-dim">
              ({{ settings?.tools?.length || 0 }} registered)
            </span>
          </h3>
          
          <div class="space-y-2">
            <div
              v-for="tool in settings?.tools || []"
              :key="tool.name"
              class="flex items-center justify-between p-3 rounded-lg bg-nanna-bg-elevated/50 hover:bg-nanna-bg-elevated transition-colors"
            >
              <div class="flex-1">
                <div class="flex items-center gap-2">
                  <span class="text-lg">{{ getToolIcon(tool.name) }}</span>
                  <span class="text-sm font-medium text-nanna-text font-mono">{{ tool.name }}</span>
                </div>
                <p class="text-xs text-nanna-text-dim mt-0.5">{{ tool.description }}</p>
              </div>
              <div :class="tool.enabled ? 'text-nanna-success' : 'text-nanna-text-dim'">
                {{ tool.enabled ? '✓ Active' : '○ Disabled' }}
              </div>
            </div>
          </div>
        </section>
        
        <!-- Data Management -->
        <section class="card">
          <h3 class="text-lg font-semibold text-nanna-accent mb-4 flex items-center gap-2">
            💾 Data Management
          </h3>
          
          <div class="space-y-4">
            <div class="flex items-center justify-between p-3 rounded-lg bg-nanna-bg-elevated/50">
              <div>
                <div class="text-sm font-medium text-nanna-text">Chat Sessions</div>
                <div class="text-xs text-nanna-text-dim">{{ sessionCount }} sessions stored</div>
              </div>
              <button 
                @click="confirmClearSessions"
                class="btn-ghost text-nanna-error hover:bg-nanna-error/10 text-sm"
              >
                Clear All
              </button>
            </div>
            
            <div class="flex items-center justify-between p-3 rounded-lg bg-nanna-bg-elevated/50">
              <div>
                <div class="text-sm font-medium text-nanna-text">Database</div>
                <div class="text-xs text-nanna-text-dim font-mono">~/.local/share/Nanna/nanna.db</div>
              </div>
            </div>
          </div>
        </section>
        
        <!-- About Section -->
        <section class="card">
          <h3 class="text-lg font-semibold text-nanna-accent mb-4 flex items-center gap-2">
            🌙 About Nanna
          </h3>
          
          <div class="space-y-3">
            <p class="text-sm text-nanna-text-muted italic">
              "I am the light that finds you in darkness, the memory that outlives the flesh."
            </p>
            
            <div class="pt-2 space-y-2 text-sm">
              <div class="flex justify-between">
                <span class="text-nanna-text-muted">Version</span>
                <span class="text-nanna-text font-mono">0.1.0</span>
              </div>
              <div class="flex justify-between">
                <span class="text-nanna-text-muted">Stack</span>
                <span class="text-nanna-text">Tauri v2 + Nuxt v4 + Rust</span>
              </div>
              <div class="flex justify-between">
                <span class="text-nanna-text-muted">Etymology</span>
                <span class="text-nanna-text">Sumerian moon god, patron of Ur</span>
              </div>
            </div>
          </div>
        </section>
        
      </div>
    </div>
    
    <!-- Toast notification -->
    <Transition name="toast">
      <div 
        v-if="toast" 
        :class="[
          'fixed bottom-4 right-4 px-4 py-3 rounded-lg shadow-lg flex items-center gap-2',
          toast.type === 'success' ? 'bg-nanna-success text-nanna-bg-deep' : 'bg-nanna-error text-white'
        ]"
      >
        <span>{{ toast.type === 'success' ? '✓' : '✗' }}</span>
        {{ toast.message }}
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

interface ToolInfo {
  name: string
  description: string
  enabled: boolean
}

interface ExtendedSettings {
  anthropic_key_set: boolean
  openai_key_set: boolean
  brave_key_set: boolean
  provider: string
  available_providers: string[]
  model: string
  available_models: string[]
  temperature: number
  top_p: number
  max_tokens: number
  tools: ToolInfo[]
}

interface SessionInfo {
  id: string
  name: string
}

const settings = ref<ExtendedSettings | null>(null)
const selectedModel = ref('')
const sessionCount = ref(0)
const toast = ref<{ message: string; type: 'success' | 'error' } | null>(null)

onMounted(async () => {
  await loadSettings()
  await loadSessions()
})

async function loadSettings() {
  try {
    settings.value = await invoke<ExtendedSettings>('get_extended_settings')
    selectedModel.value = settings.value.model
  } catch (e) {
    console.error('Failed to load settings:', e)
    showToast('Failed to load settings', 'error')
  }
}

async function loadSessions() {
  try {
    const sessions = await invoke<SessionInfo[]>('list_sessions')
    sessionCount.value = sessions.length
  } catch (e) {
    console.error('Failed to load sessions:', e)
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

async function confirmClearSessions() {
  if (!confirm('Delete all chat sessions? This cannot be undone.')) return
  
  try {
    const sessions = await invoke<SessionInfo[]>('list_sessions')
    for (const session of sessions) {
      await invoke('delete_session', { sessionId: session.id })
    }
    showToast('All sessions cleared', 'success')
    sessionCount.value = 0
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

// Group models by provider for the dropdown
const groupedModels = computed(() => {
  const models = settings.value?.available_models || []
  return [
    {
      provider: 'anthropic',
      label: 'Anthropic (Claude)',
      models: models.filter(m => m.includes('claude'))
    },
    {
      provider: 'openai',
      label: 'OpenAI (GPT)',
      models: models.filter(m => m.includes('gpt'))
    },
    {
      provider: 'other',
      label: 'Other',
      models: models.filter(m => !m.includes('claude') && !m.includes('gpt'))
    }
  ].filter(g => g.models.length > 0)
})

function formatProvider(provider: string): string {
  const names: Record<string, string> = {
    anthropic: 'Anthropic',
    openai: 'OpenAI',
    openrouter: 'OpenRouter',
  }
  return names[provider] || provider
}

function formatModelName(model: string): string {
  const names: Record<string, string> = {
    'claude-sonnet-4-20250514': 'Claude Sonnet 4',
    'claude-opus-4-20250514': 'Claude Opus 4',
    'claude-3-5-sonnet-20241022': 'Claude 3.5 Sonnet',
    'claude-3-5-haiku-20241022': 'Claude 3.5 Haiku',
    'gpt-4o': 'GPT-4o',
    'gpt-4o-mini': 'GPT-4o Mini',
    'gpt-4-turbo': 'GPT-4 Turbo',
    'deepseek/deepseek-chat': 'DeepSeek Chat',
    'google/gemini-2.0-flash-exp': 'Gemini 2.0 Flash',
  }
  return names[model] || model
}

function getToolIcon(name: string): string {
  const icons: Record<string, string> = {
    read_file: '📄',
    write_file: '✏️',
    list_dir: '📁',
    exec: '⚡',
    web_fetch: '🌐',
    web_search: '🔍',
    echo: '💬',
  }
  return icons[name] || '🔧'
}

function showToast(message: string, type: 'success' | 'error') {
  toast.value = { message, type }
  setTimeout(() => {
    toast.value = null
  }, 3000)
}
</script>

<style scoped>
.card {
  @apply bg-nanna-bg-surface/50 border border-nanna-primary/10 rounded-xl p-6;
}

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
