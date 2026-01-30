<template>
  <div class="flex items-center gap-2">
    <!-- Active Model Badge -->
    <div 
      class="flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium transition-all"
      :class="statusClass"
      :title="statusTitle"
    >
      <span class="w-1.5 h-1.5 rounded-full animate-pulse" :class="dotClass" />
      <span class="truncate max-w-[120px]">{{ displayModel }}</span>
      <span v-if="isUsingFallback" class="text-nanna-warning">↓</span>
    </div>
    
    <!-- Rate Limited Indicator -->
    <div 
      v-if="rateLimitedCount > 0"
      class="flex items-center gap-1 px-2 py-1 rounded-full bg-nanna-warning/20 text-nanna-warning text-xs"
      :title="`${rateLimitedCount} model(s) rate limited`"
    >
      <span>⏱️ {{ rateLimitedCount }}</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

interface ModelStatus {
  active_model: string
  fallback_reason: string | null
  rate_limited_models: string[]
}

const status = ref<ModelStatus | null>(null)
const isLoading = ref(true)
let unlisten: UnlistenFn | null = null

const displayModel = computed(() => {
  if (!status.value) return 'Loading...'
  const model = status.value.active_model
  // Simplify display (remove provider prefix, shorten name)
  const name = model.includes('/') ? (model.split('/')[1] ?? model) : model
  return formatModelName(name)
})

const isUsingFallback = computed(() => {
  return status.value?.fallback_reason !== null
})

const rateLimitedCount = computed(() => {
  return status.value?.rate_limited_models.length ?? 0
})

const statusClass = computed(() => {
  if (isUsingFallback.value) {
    return 'bg-nanna-warning/20 text-nanna-warning border border-nanna-warning/30'
  }
  return 'bg-nanna-bg-elevated/80 text-nanna-text-muted border border-nanna-primary/20'
})

const dotClass = computed(() => {
  if (isUsingFallback.value) {
    return 'bg-nanna-warning'
  }
  return 'bg-nanna-success'
})

const statusTitle = computed(() => {
  if (!status.value) return 'Loading model status...'
  let title = `Active: ${status.value.active_model}`
  if (status.value.fallback_reason) {
    title += `\nFallback reason: ${status.value.fallback_reason}`
  }
  if (status.value.rate_limited_models.length > 0) {
    title += `\nRate limited: ${status.value.rate_limited_models.join(', ')}`
  }
  return title
})

function formatModelName(model: string): string {
  // Shorten common model names
  const replacements: Record<string, string> = {
    'claude-opus-4-5-20250514': 'Opus 4.5',
    'claude-sonnet-4-20250514': 'Sonnet 4',
    'claude-3-5-sonnet-20241022': 'Sonnet 3.5',
    'claude-3-5-haiku-20241022': 'Haiku 3.5',
    'gpt-4o': 'GPT-4o',
    'gpt-4o-mini': 'GPT-4o Mini',
    'gpt-4-turbo': 'GPT-4 Turbo',
  }
  return replacements[model] || model
}

async function loadStatus() {
  try {
    status.value = await invoke<ModelStatus>('get_model_status')
  } catch (e) {
    console.error('Failed to load model status:', e)
  } finally {
    isLoading.value = false
  }
}

onMounted(async () => {
  await loadStatus()
  
  // Listen for model status updates
  unlisten = await listen<ModelStatus>('model-status', (event) => {
    status.value = event.payload
  })
})

onUnmounted(() => {
  if (unlisten) {
    unlisten()
  }
})
</script>
