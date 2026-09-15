<template>
  <div class="h-full flex flex-col">
    <!-- Header -->
    <div class="px-6 py-4 border-b border-white/[0.04]">
      <div class="flex items-center justify-between">
        <div>
          <h1 class="text-2xl font-bold text-nanna-text">Tool Audit</h1>
          <p class="text-sm text-nanna-text-muted mt-1">
            Every tool call Nanna made — including the ones that were refused or never existed
          </p>
        </div>
        <div class="flex items-center gap-2">
          <button
            @click="fetchAudit"
            class="px-3 py-2 rounded-lg text-sm font-medium glass-chip text-nanna-text-muted hover:text-nanna-text"
          >
            <RefreshCw class="w-4 h-4 inline mr-1" :class="isLoading ? 'animate-spin' : ''" />
            Refresh
          </button>
        </div>
      </div>
    </div>

    <!-- Content -->
    <div class="flex-1 overflow-y-auto p-6 space-y-4">
      <PageState
        v-if="isLoading || !isOnline || loadError || auditDisabled || records.length === 0"
        :state="isLoading ? 'loading' : (!isOnline ? 'offline' : (loadError ? 'error' : 'empty'))"
        :title="isLoading
          ? 'Reading the audit trail…'
          : (!isOnline
            ? 'Daemon offline'
            : (loadError
              ? 'Could not read the audit trail'
              : (auditDisabled ? 'The audit trail is off' : 'No tool calls recorded yet')))"
        :description="isLoading
          ? 'The trail lives on the daemon, so this is an IPC read.'
          : (!isOnline
            ? 'The audit trail lives in the daemon\'s data directory. Reconnect to read it.'
            : (loadError || disabledMessage || 'Records appear here as soon as Nanna runs a tool.'))"
        :primary-action="(!isOnline || !!loadError) && !isLoading ? 'Retry' : ''"
        :primary-busy="isLoading"
        @primary="fetchAudit"
      />

      <template v-if="records.length > 0">
        <!-- Filters -->
        <div class="flex flex-wrap items-center gap-2">
          <button
            v-for="opt in outcomeFilters"
            :key="opt.value"
            @click="outcomeFilter = opt.value"
            :class="[
              'px-3 py-1.5 rounded-lg text-xs font-medium transition-colors',
              outcomeFilter === opt.value
                ? 'bg-nanna-accent/20 text-nanna-accent'
                : 'glass-chip text-nanna-text-muted hover:text-nanna-text',
            ]"
          >
            {{ opt.label }}
            <span class="ml-1 opacity-60 font-mono">{{ outcomeCounts[opt.value] ?? 0 }}</span>
          </button>

          <UiInput
            v-model="nameFilter"
            size="sm"
            class="ml-auto w-56"
            placeholder="Filter by tool name…"
          />
        </div>

        <!-- Records -->
        <div class="glass-panel rounded-xl divide-y divide-white/[0.04]">
          <div
            v-for="(record, i) in visibleRecords"
            :key="`${record.ts_unix_ms}-${record.call_id}-${i}`"
            class="p-3 flex items-start gap-3 text-sm"
          >
            <span
              :class="['px-2 py-0.5 rounded text-[11px] font-medium shrink-0 mt-0.5', outcomeClass(record.outcome)]"
            >
              {{ record.outcome }}
            </span>

            <div class="min-w-0 flex-1">
              <div class="flex items-baseline gap-2 flex-wrap">
                <span class="font-mono text-nanna-text">{{ record.requested }}</span>
                <!-- The alias case the audit exists to make visible: what the
                     model typed is not always what ran. -->
                <span
                  v-if="record.resolved && record.resolved !== record.requested"
                  class="font-mono text-xs text-nanna-accent"
                >
                  → {{ record.resolved }}
                </span>
                <span
                  v-else-if="!record.resolved"
                  class="text-xs text-nanna-text-muted italic"
                >
                  resolved to nothing
                </span>
              </div>

              <div
                v-if="record.param_keys?.length"
                class="mt-1 flex flex-wrap gap-1"
              >
                <span
                  v-for="key in record.param_keys"
                  :key="key"
                  class="px-1.5 py-0.5 rounded bg-white/[0.04] text-[11px] font-mono text-nanna-text-muted"
                >
                  {{ key }}
                </span>
              </div>

              <p v-if="record.error" class="mt-1 text-xs text-nanna-error font-mono break-all">
                {{ record.error }}
              </p>
              <p v-if="record.reason" class="mt-1 text-xs text-nanna-warning">
                {{ record.reason }}
              </p>
              <p
                v-if="record.params_preview"
                class="mt-1 text-xs text-nanna-text-muted font-mono break-all"
              >
                {{ record.params_preview }}
              </p>
            </div>

            <div class="text-right shrink-0 text-xs text-nanna-text-muted">
              <div class="font-mono">{{ formatDuration(record.duration_ms) }}</div>
              <div class="mt-0.5">{{ formatTime(record.ts_unix_ms) }}</div>
            </div>
          </div>

          <div
            v-if="visibleRecords.length === 0"
            class="p-6 text-center text-sm text-nanna-text-muted"
          >
            No records match this filter.
          </div>
        </div>

        <!--
          The reader's account of itself. Without this a viewer cannot tell a
          complete history from one screenful, or a clean file from one it
          partly failed to read — which is the whole difference between a log
          and an audit.
        -->
        <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-nanna-text-muted px-1">
          <span>
            Showing {{ visibleRecords.length }} of {{ records.length }} record{{ records.length === 1 ? '' : 's' }}
          </span>
          <span v-if="reachedOldest">This is the entire retained history.</span>
          <span v-else>
            Older records exist beyond this page — raise the limit to reach further back.
          </span>
          <span v-if="unparseable > 0" class="text-nanna-warning">
            <AlertTriangle class="w-3 h-3 inline mr-0.5" />
            {{ unparseable }} line{{ unparseable === 1 ? '' : 's' }} could not be read and
            {{ unparseable === 1 ? 'was' : 'were' }} skipped.
          </span>
          <span v-if="!valuesIncluded" class="ml-auto">
            Argument values are excluded by default — only key names are recorded.
          </span>
        </div>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { RefreshCw, AlertTriangle } from '@lucide/vue'

const { isOnline } = useBackend()

/** Mirrors `nanna_tools::ToolAuditRecord`, whose outcome is serde-flattened. */
interface AuditRecord {
  ts_unix_ms: number
  call_id: string
  requested: string
  resolved: string | null
  session_id: string | null
  param_keys: string[]
  params_preview?: string
  duration_ms: number
  outcome: 'succeeded' | 'failed' | 'refused' | 'not_found'
  /** Present on `failed`. */
  error?: string
  /** Present on `refused`. */
  reason?: string
}

const PAGE_LIMIT = 200

const records = ref<AuditRecord[]>([])
const isLoading = ref(false)
const loadError = ref<string | null>(null)
const auditDisabled = ref(false)
const disabledMessage = ref<string | null>(null)
const unparseable = ref(0)
const reachedOldest = ref(true)
const valuesIncluded = ref(false)

const outcomeFilter = ref<string>('all')
const nameFilter = ref('')

const outcomeFilters = [
  { value: 'all', label: 'All' },
  { value: 'succeeded', label: 'Succeeded' },
  { value: 'failed', label: 'Failed' },
  { value: 'refused', label: 'Refused' },
  { value: 'not_found', label: 'Not found' },
] as const

const outcomeCounts = computed<Record<string, number>>(() => {
  const counts: Record<string, number> = { all: records.value.length }
  for (const record of records.value) {
    counts[record.outcome] = (counts[record.outcome] ?? 0) + 1
  }
  return counts
})

const visibleRecords = computed(() => {
  const needle = nameFilter.value.trim().toLowerCase()
  return records.value.filter((record) => {
    if (outcomeFilter.value !== 'all' && record.outcome !== outcomeFilter.value) {
      return false
    }
    if (!needle) return true
    return (
      record.requested.toLowerCase().includes(needle) ||
      (record.resolved?.toLowerCase().includes(needle) ?? false)
    )
  })
})

function outcomeClass(outcome: AuditRecord['outcome']): string {
  switch (outcome) {
    case 'succeeded':
      return 'bg-nanna-success/15 text-nanna-success'
    case 'failed':
      return 'bg-nanna-error/15 text-nanna-error'
    case 'refused':
      return 'bg-nanna-warning/15 text-nanna-warning'
    default:
      return 'bg-white/[0.06] text-nanna-text-muted'
  }
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  return `${(ms / 1000).toFixed(1)}s`
}

function formatTime(ms: number): string {
  if (!ms) return '—'
  return new Date(ms).toLocaleString()
}

async function fetchAudit() {
  isLoading.value = true
  loadError.value = null
  try {
    const result = await invoke<{
      enabled: boolean
      message?: string
      records?: AuditRecord[]
      unparseable?: number
      reached_oldest?: boolean
    }>('get_tool_audit', { limit: PAGE_LIMIT })

    auditDisabled.value = result.enabled === false
    disabledMessage.value = result.message ?? null
    records.value = result.records ?? []
    unparseable.value = result.unparseable ?? 0
    reachedOldest.value = result.reached_oldest ?? true
    // A preview only ever appears when the operator opted in, so its presence
    // IS the signal — no second config read needed.
    valuesIncluded.value = records.value.some((r) => r.params_preview != null)
  } catch (e) {
    loadError.value = String(e)
    records.value = []
  } finally {
    isLoading.value = false
  }
}

onMounted(fetchAudit)
</script>
