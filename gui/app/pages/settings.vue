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
        
        <!-- API Configuration -->
        <section class="card">
          <h3 class="text-lg font-semibold text-nanna-accent mb-4">API Configuration</h3>
          
          <div class="space-y-4">
            <!-- API Key Status -->
            <div class="flex items-center justify-between p-3 rounded-lg bg-nanna-bg-elevated/50">
              <div>
                <div class="text-sm font-medium text-nanna-text">Anthropic API Key</div>
                <div class="text-xs text-nanna-text-dim mt-0.5">
                  {{ config?.api_key_set ? 'API key is configured' : 'No API key set' }}
                </div>
              </div>
              <div :class="config?.api_key_set ? 'text-nanna-success' : 'text-nanna-error'">
                {{ config?.api_key_set ? '✓ Connected' : '✗ Missing' }}
              </div>
            </div>
            
            <!-- API Key Input -->
            <div>
              <label class="block text-sm font-medium text-nanna-text-muted mb-2">
                Set API Key
              </label>
              <div class="flex gap-2">
                <input
                  v-model="apiKey"
                  :type="showApiKey ? 'text' : 'password'"
                  placeholder="sk-ant-..."
                  class="input flex-1 font-mono text-sm"
                />
                <button 
                  @click="showApiKey = !showApiKey"
                  class="btn-ghost px-3"
                  type="button"
                >
                  {{ showApiKey ? '🙈' : '👁️' }}
                </button>
                <button 
                  @click="saveApiKey"
                  class="btn-primary"
                  :disabled="!apiKey.trim()"
                >
                  Save
                </button>
              </div>
              <p class="text-xs text-nanna-text-dim mt-2">
                Get your API key from <a href="https://console.anthropic.com" target="_blank" class="text-nanna-accent hover:underline">console.anthropic.com</a>
              </p>
            </div>
          </div>
        </section>
        
        <!-- Model Selection -->
        <section class="card">
          <h3 class="text-lg font-semibold text-nanna-accent mb-4">Model</h3>
          
          <div class="space-y-4">
            <div>
              <label class="block text-sm font-medium text-nanna-text-muted mb-2">
                Default Model
              </label>
              <select 
                v-model="selectedModel" 
                @change="updateModel"
                class="input"
              >
                <option v-for="model in config?.available_models || []" :key="model" :value="model">
                  {{ formatModelName(model) }}
                </option>
              </select>
            </div>
            
            <div class="p-3 rounded-lg bg-nanna-bg-elevated/50">
              <div class="text-sm text-nanna-text-muted">
                <strong class="text-nanna-text">Current:</strong> {{ formatModelName(config?.model || 'Loading...') }}
              </div>
            </div>
          </div>
        </section>
        
        <!-- Appearance -->
        <section class="card">
          <h3 class="text-lg font-semibold text-nanna-accent mb-4">Appearance</h3>
          
          <div class="space-y-4">
            <div class="flex items-center justify-between">
              <div>
                <div class="text-sm font-medium text-nanna-text">Theme</div>
                <div class="text-xs text-nanna-text-dim">Currently using dark theme</div>
              </div>
              <div class="text-nanna-text-muted text-sm">
                🌙 Dark (default)
              </div>
            </div>
          </div>
        </section>
        
        <!-- Data Management -->
        <section class="card">
          <h3 class="text-lg font-semibold text-nanna-accent mb-4">Data</h3>
          
          <div class="space-y-4">
            <div class="flex items-center justify-between">
              <div>
                <div class="text-sm font-medium text-nanna-text">Sessions</div>
                <div class="text-xs text-nanna-text-dim">{{ sessionCount }} chat sessions stored</div>
              </div>
              <button 
                @click="confirmClearSessions"
                class="btn-ghost text-nanna-error hover:bg-nanna-error/10"
              >
                Clear All
              </button>
            </div>
            
            <div class="flex items-center justify-between">
              <div>
                <div class="text-sm font-medium text-nanna-text">Database Location</div>
                <div class="text-xs text-nanna-text-dim font-mono">~/.local/share/Nanna/nanna.db</div>
              </div>
            </div>
          </div>
        </section>
        
        <!-- About -->
        <section class="card">
          <h3 class="text-lg font-semibold text-nanna-accent mb-4">About</h3>
          
          <div class="space-y-2 text-sm">
            <div class="flex justify-between">
              <span class="text-nanna-text-muted">Version</span>
              <span class="text-nanna-text font-mono">0.1.0</span>
            </div>
            <div class="flex justify-between">
              <span class="text-nanna-text-muted">Stack</span>
              <span class="text-nanna-text">Tauri v2 + Nuxt v4 + Rust</span>
            </div>
            <div class="flex justify-between">
              <span class="text-nanna-text-muted">Source</span>
              <a href="https://github.com/clawdbot/nanna" target="_blank" class="text-nanna-accent hover:underline">
                github.com/clawdbot/nanna
              </a>
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
          'fixed bottom-4 right-4 px-4 py-2 rounded-lg shadow-lg',
          toast.type === 'success' ? 'bg-nanna-success text-nanna-bg-deep' : 'bg-nanna-error text-white'
        ]"
      >
        {{ toast.message }}
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

interface AppConfig {
  theme: string
  model: string
  api_key_set: boolean
  available_models: string[]
}

interface SessionInfo {
  id: string
  name: string
  created_at: string
  updated_at: string
  message_count: number
}

const config = ref<AppConfig | null>(null)
const apiKey = ref('')
const showApiKey = ref(false)
const selectedModel = ref('')
const sessionCount = ref(0)
const toast = ref<{ message: string; type: 'success' | 'error' } | null>(null)

onMounted(async () => {
  await loadConfig()
  await loadSessions()
})

async function loadConfig() {
  try {
    config.value = await invoke<AppConfig>('get_config')
    selectedModel.value = config.value.model
  } catch (e) {
    console.error('Failed to load config:', e)
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

async function saveApiKey() {
  if (!apiKey.value.trim()) return
  
  try {
    // Store in environment for current session
    // Note: For persistence, this should write to config file
    await invoke('set_api_key', { apiKey: apiKey.value })
    showToast('API key saved successfully', 'success')
    apiKey.value = ''
    await loadConfig()
  } catch (e: any) {
    showToast(`Failed to save: ${e.message || e}`, 'error')
  }
}

async function updateModel() {
  try {
    await invoke('set_model', { model: selectedModel.value })
    showToast('Model updated', 'success')
    await loadConfig()
  } catch (e: any) {
    showToast(`Failed to update: ${e.message || e}`, 'error')
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
    showToast(`Failed to clear: ${e.message || e}`, 'error')
  }
}

function formatModelName(model: string): string {
  const names: Record<string, string> = {
    'claude-sonnet-4-20250514': 'Claude Sonnet 4',
    'claude-opus-4-20250514': 'Claude Opus 4',
    'claude-3-5-sonnet-20241022': 'Claude 3.5 Sonnet',
    'gpt-4o': 'GPT-4o',
    'gpt-4o-mini': 'GPT-4o Mini',
  }
  return names[model] || model
}

function showToast(message: string, type: 'success' | 'error') {
  toast.value = { message, type }
  setTimeout(() => {
    toast.value = null
  }, 3000)
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
