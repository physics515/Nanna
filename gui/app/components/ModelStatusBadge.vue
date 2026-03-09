<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useGroundGlass } from '~/composables/useGroundGlass'

interface ModelStatus {
  active_model: string
  fallback_reason: string | null
  rate_limited_models: string[]
}

const status = ref<ModelStatus | null>(null)
const isLoading = ref(true)
let unlisten: UnlistenFn | null = null

// Ground glass for the pill
const { meshBg, containerStyle, onEnter, onLeave } = useGroundGlass({
  opacity: 2.0,
  sizes: ['60%', '55%', '50%'],
  lerpSpeed: 0.008,
  interval: 2000,
})

// Prevent animation on mount when cursor is already over the pill
const ready = ref(false)

function handleEnter() {
  if (ready.value) onEnter()
}
function handleLeave() {
  if (ready.value) onLeave()
}

const displayModel = computed(() => {
  if (!status.value) return 'Loading...'
  const model = status.value.active_model
  const name = model.includes('/') ? (model.split('/')[1] ?? model) : model
  return formatModelName(name)
})

const isUsingFallback = computed(() => {
  return status.value?.fallback_reason !== null
})

const rateLimitedCount = computed(() => {
  return status.value?.rate_limited_models.length ?? 0
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
  const replacements: Record<string, string> = {
    'claude-opus-4-5-20251101': 'Opus 4.5',
    'claude-opus-4-20250514': 'Opus 4',
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
  unlisten = await listen<ModelStatus>('model-status', (event) => {
    status.value = event.payload
  })
  setTimeout(() => { ready.value = true }, 200)
})

onUnmounted(() => {
  if (unlisten) unlisten()
})
</script>

<template>
  <div class="flex items-center gap-2">
    <!-- Active Model Pill (Ground Glass) -->
    <div
      class="model-pill"
      :class="{ 'model-pill--fallback': isUsingFallback }"
      :style="containerStyle"
      :title="statusTitle"
      @mouseenter="handleEnter"
      @mouseleave="handleLeave"
    >
      <!-- Layer 0: animated mesh gradient -->
      <span class="model-pill__mesh" :style="{ background: meshBg }" />

      <!-- Layer 1: content -->
      <span class="model-pill__content">
        <span class="model-pill__dot" :class="{ 'model-pill__dot--warn': isUsingFallback }" />
        <span class="model-pill__name">{{ displayModel }}</span>
        <span v-if="isUsingFallback" class="model-pill__fallback-arrow">&#8595;</span>
      </span>
    </div>

    <!-- Rate Limited Indicator -->
    <div
      v-if="rateLimitedCount > 0"
      class="flex items-center gap-1 px-2 py-1 rounded-full bg-nanna-warning/20 text-nanna-warning text-xs"
      :title="`${rateLimitedCount} model(s) rate limited`"
    >
      <span>{{ rateLimitedCount }} limited</span>
    </div>
  </div>
</template>

<style scoped>
.model-pill {
  position: relative;
  isolation: isolate;
  overflow: hidden;
  display: inline-flex;
  align-items: center;
  border-radius: 9999px;
  padding: 4px 10px;
  font-size: 12px;
  font-weight: 500;
  cursor: default;
  background: rgba(30, 41, 59, 0.30);
  /* Glass slab borders */
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  border-left: 1px solid rgba(255, 255, 255, 0.04);
  border-bottom: 1.5px solid rgba(71, 85, 105, 0.18);
  border-right: 1px solid rgba(71, 85, 105, 0.10);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.04),
    0 1.5px 1px -0.5px rgba(0, 0, 0, 0.18),
    0 3px 8px -3px rgba(0, 0, 0, 0.12);
  transition: box-shadow 0.2s ease, border-color 0.2s ease;
}
.model-pill:hover {
  border-top-color: rgba(255, 255, 255, 0.10);
  border-left-color: rgba(255, 255, 255, 0.07);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.06),
    0 2px 4px -1px rgba(0, 0, 0, 0.20),
    0 4px 12px -4px rgba(0, 0, 0, 0.15);
}

.model-pill--fallback {
  border-color: rgba(251, 191, 36, 0.25);
}

/* Layer 0: animated mesh gradient */
.model-pill__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
}

/* Noise overlay */
.model-pill::after {
  content: '';
  position: absolute;
  inset: 0;
  z-index: 2;
  pointer-events: none;
  border-radius: inherit;
  opacity: 0.14;
  background-blend-mode: soft-light;
  background: repeating-radial-gradient(
    circle,
    #1a2035,
    #1a2035 2px,
    #253050 2px 4px,
    #1a2035 4px 6px,
    #253050 6px 8px,
    #1a2035 8px 10px,
    #253050 10px 12px
  ) 0 0 / 100% 100%;
}

/* Layer 1: content */
.model-pill__content {
  position: relative;
  z-index: 1;
  display: flex;
  align-items: center;
  gap: 6px;
}

.model-pill__dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #34d399;
  animation: pill-pulse 2s ease-in-out infinite;
  flex-shrink: 0;
}
.model-pill__dot--warn {
  background: #fbbf24;
}

@keyframes pill-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.5; }
}

.model-pill__name {
  color: rgba(255, 255, 255, 0.95);
  white-space: nowrap;
  max-width: 140px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.model-pill__fallback-arrow {
  color: #fbbf24;
  font-size: 11px;
}
</style>
