<template>
  <div class="h-full flex flex-col bg-nanna-bg-deep">
    <!-- Header -->
    <div class="px-6 py-4 border-b border-white/[0.04] bg-nanna-bg-surface">
      <div class="flex items-center justify-between">
        <div>
          <h1 class="text-2xl font-bold text-nanna-text">Model Stats</h1>
          <p class="text-sm text-nanna-text-muted mt-1">Per-model performance metrics, health, and token usage</p>
        </div>
        <div class="flex items-center gap-2">
          <button
            @click="fetchStats"
            class="px-3 py-2 rounded-lg text-sm font-medium bg-nanna-bg-elevated text-nanna-text-muted hover:text-nanna-text transition-colors"
          >
            <RefreshCw class="w-4 h-4 inline mr-1" :class="isLoading ? 'animate-spin' : ''" />
            Refresh
          </button>
        </div>
      </div>
    </div>

    <!-- Content -->
    <div class="flex-1 overflow-y-auto p-6 space-y-6">
      <PageState
        v-if="isLoading || !isOnline || loadError || models.length === 0"
        :state="isLoading ? 'loading' : (!isOnline ? 'offline' : (loadError ? 'error' : 'empty'))"
        :title="isLoading ? 'Loading model stats…' : (!isOnline ? 'Daemon offline' : (loadError ? 'Could not load stats' : 'No model stats yet'))"
        :description="isLoading
          ? 'Pulling per-model counters from the daemon.'
          : (!isOnline
            ? 'Model stats live on the daemon. Reconnect to inspect latency, tokens, and cost.'
            : (loadError || 'Stats will appear after models process requests.'))"
        :primary-action="(!isOnline || !!loadError) && !isLoading ? 'Retry' : ''"
        :primary-busy="isLoading"
        @primary="fetchStats"
      />

      <!-- Model cards -->
      <div v-for="model in sortedModels" :key="model.model" class="bg-nanna-bg-surface rounded-xl border border-white/[0.04] p-5">
        <!-- Model header -->
        <div class="flex items-center justify-between mb-4">
          <div class="flex items-center gap-3">
            <div
              :class="[
                'w-3 h-3 rounded-full',
                model.is_healthy ? 'bg-nanna-success' :
                model.consecutive_failures > 0 ? 'bg-nanna-error' :
                'bg-nanna-warning'
              ]"
            />
            <h3 class="text-lg font-semibold text-nanna-text font-mono">{{ model.model }}</h3>
            <span
              :class="[
                'text-xs px-2 py-0.5 rounded-full',
                model.is_healthy
                  ? 'bg-nanna-success/20 text-nanna-success'
                  : model.consecutive_failures >= 5
                    ? 'bg-nanna-error/20 text-nanna-error'
                    : 'bg-nanna-warning/20 text-nanna-warning'
              ]"
            >
              {{ model.is_healthy ? 'Healthy' : model.consecutive_failures >= 5 ? 'Unhealthy' : 'Degraded' }}
            </span>
          </div>
          <span class="text-sm text-nanna-text-muted">
            {{ model.total_requests.toLocaleString() }} requests
          </span>
        </div>

        <!-- Stats grid -->
        <div class="grid grid-cols-2 md:grid-cols-4 gap-4 mb-4">
          <!-- Success Rate -->
          <div class="bg-nanna-bg-elevated rounded-lg p-3">
            <div class="text-xs text-nanna-text-muted mb-1">Success Rate</div>
            <div :class="[
              'text-xl font-bold font-mono',
              model.success_rate >= 0.95 ? 'text-nanna-success' :
              model.success_rate >= 0.8 ? 'text-nanna-warning' :
              'text-nanna-error'
            ]">
              {{ (model.success_rate * 100).toFixed(1) }}%
            </div>
          </div>

          <!-- Avg Latency -->
          <div class="bg-nanna-bg-elevated rounded-lg p-3">
            <div class="text-xs text-nanna-text-muted mb-1">Avg Latency</div>
            <div class="text-xl font-bold font-mono text-nanna-text">
              {{ formatLatency(model.avg_latency_ms) }}
            </div>
          </div>

          <!-- Throughput -->
          <div class="bg-nanna-bg-elevated rounded-lg p-3">
            <div class="text-xs text-nanna-text-muted mb-1">Throughput</div>
            <div class="text-xl font-bold font-mono text-nanna-accent">
              {{ model.avg_throughput_tps.toFixed(1) }} <span class="text-sm text-nanna-text-muted">tok/s</span>
            </div>
          </div>

          <!-- Cache Hit Rate -->
          <div class="bg-nanna-bg-elevated rounded-lg p-3">
            <div class="text-xs text-nanna-text-muted mb-1">Cache Hit Rate</div>
            <div class="text-xl font-bold font-mono text-nanna-primary">
              {{ (model.cache_hit_rate * 100).toFixed(1) }}%
            </div>
          </div>
        </div>

        <!-- Token usage bar -->
        <div class="mb-3">
          <div class="flex justify-between text-xs text-nanna-text-muted mb-1">
            <span>Token Usage</span>
            <span>{{ formatTokens(model.total_input_tokens + model.total_output_tokens) }} total</span>
          </div>
          <div class="h-2 bg-nanna-bg-elevated rounded-full overflow-hidden flex">
            <div
              class="bg-nanna-primary h-full"
              :style="{ width: tokenBarWidth(model, 'input') + '%' }"
              :title="'Input: ' + formatTokens(model.total_input_tokens)"
            />
            <div
              class="bg-nanna-accent h-full"
              :style="{ width: tokenBarWidth(model, 'output') + '%' }"
              :title="'Output: ' + formatTokens(model.total_output_tokens)"
            />
            <div
              class="bg-nanna-success/50 h-full"
              :style="{ width: tokenBarWidth(model, 'cache') + '%' }"
              :title="'Cached: ' + formatTokens(model.total_cache_read_tokens)"
            />
          </div>
          <div class="flex gap-4 mt-1 text-[10px] text-nanna-text-dim">
            <span class="flex items-center gap-1">
              <span class="w-2 h-2 rounded-full bg-nanna-primary inline-block" />
              Input: {{ formatTokens(model.total_input_tokens) }}
            </span>
            <span class="flex items-center gap-1">
              <span class="w-2 h-2 rounded-full bg-nanna-accent inline-block" />
              Output: {{ formatTokens(model.total_output_tokens) }}
            </span>
            <span class="flex items-center gap-1">
              <span class="w-2 h-2 rounded-full bg-nanna-success/50 inline-block" />
              Cached: {{ formatTokens(model.total_cache_read_tokens) }}
            </span>
          </div>
        </div>

        <!-- Extra details -->
        <div class="flex gap-6 text-xs text-nanna-text-muted pt-2 border-t border-white/[0.04]">
          <span>P95: {{ formatLatency(model.p95_latency_ms) }}</span>
          <span v-if="model.escalation_count > 0">Escalations: {{ model.escalation_count }}</span>
          <span v-if="model.consecutive_failures > 0" class="text-nanna-error">
            {{ model.consecutive_failures }} consecutive failures
          </span>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { RefreshCw, BarChart3 } from 'lucide-vue-next'

interface ModelStat {
const { isOnline } = useBackend()
const toast = useToast()
  model: string
  total_requests: number
  success_rate: number
  avg_latency_ms: number
  p95_latency_ms: number
  avg_throughput_tps: number
  total_input_tokens: number
  total_output_tokens: number
  total_cache_read_tokens: number
  cache_hit_rate: number
  consecutive_failures: number
  is_healthy: boolean
  escalation_count: number
}

const models = ref<ModelStat[]>([])
const isLoading = ref(false)
let refreshInterval: ReturnType<typeof setInterval> | null = null

const sortedModels = computed(() => {
  return [...models.value].sort((a, b) => b.total_requests - a.total_requests)
})

async function fetchStats() {
  isLoading.value = true
  loadError.value = null
  try {
    const result = await invoke<{ models: ModelStat[] }>('get_model_stats')
    models.value = result.models || []
  } catch (e) {
    console.error('Failed to fetch model stats:', e)
    loadError.value = e instanceof Error ? e.message : String(e)
  } finally {
    isLoading.value = false
  }
}

function formatLatency(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  return `${(ms / 1000).toFixed(1)}s`
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toString()
}

function tokenBarWidth(model: ModelStat, type: 'input' | 'output' | 'cache'): number {
  const total = model.total_input_tokens + model.total_output_tokens + model.total_cache_read_tokens
  if (total === 0) return 0
  const value = type === 'input' ? model.total_input_tokens
    : type === 'output' ? model.total_output_tokens
    : model.total_cache_read_tokens
  return (value / total) * 100
}

onMounted(async () => {
  await fetchStats()
  // Auto-refresh every 30 seconds
  refreshInterval = setInterval(fetchStats, 30_000)
})

onUnmounted(() => {
  if (refreshInterval) clearInterval(refreshInterval)
})
</script>
