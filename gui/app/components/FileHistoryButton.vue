<template>
  <div class="relative shrink-0">
    <button
      type="button"
      class="flex items-center gap-1 whitespace-nowrap rounded px-1.5 py-0.5 text-xs leading-normal text-nui-muted hover:text-nui-text"
      :aria-expanded="open"
      title="Files changed in this chat — restore an earlier version"
      @click="toggle"
    >
      <History class="h-3.5 w-3.5" />
      Files
    </button>
    <div
      v-if="open"
      class="absolute right-0 z-20 mt-1 w-96 max-w-[80vw] rounded-lg border border-white/10 bg-nui-surface p-2 shadow-lg"
      role="dialog"
      aria-label="File history"
    >
      <p v-if="loading" class="px-2 py-1 text-xs text-nui-muted">Loading…</p>
      <p v-else-if="loadError" class="px-2 py-1 text-xs text-nui-pink">{{ loadError }}</p>
      <p v-else-if="history.checkpoints.length === 0" class="px-2 py-1 text-xs text-nui-muted">
        No file changes in this chat yet.
      </p>
      <ul v-else class="max-h-80 space-y-1 overflow-y-auto">
        <li
          v-for="c in history.checkpoints"
          :key="c.checkpoint"
          class="flex items-center gap-2 rounded px-2 py-1 text-xs hover:bg-white/5"
          :data-checkpoint="c.checkpoint"
        >
          <span class="min-w-0 flex-1 truncate font-mono text-nui-text" :title="c.path">{{ fileName(c.path) }}</span>
          <span class="shrink-0 text-nui-muted">{{ c.existed ? `${c.bytes.toLocaleString()} B` : 'new file' }}</span>
          <button
            type="button"
            class="shrink-0 rounded px-1.5 py-0.5 text-nui-accent hover:bg-nui-accent/10 disabled:opacity-50"
            :disabled="restoring !== null"
            @click="restore(c)"
          >
            Restore
          </button>
        </li>
      </ul>
      <p v-if="history.total > history.checkpoints.length" class="px-2 pt-1 text-[10px] text-nui-muted">
        {{ history.total - history.checkpoints.length }} older not shown
      </p>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { History } from '@lucide/vue'
import { useConfirm } from '~/composables/useConfirm'
import { useToast } from '~/composables/useToast'
import { fileName, parseFileHistory, restoreEffect, type FileCheckpoint, type FileHistory } from '~/lib/fileHistory'

const props = defineProps<{ sessionId: string }>()

/** Rows fetched per open — the daemon's own recent window. */
const LIST_LIMIT = 50

const open = ref(false)
const loading = ref(false)
const loadError = ref<string | null>(null)
const restoring = ref<number | null>(null)
const history = ref<FileHistory>({ checkpoints: [], total: 0 })
const { confirm } = useConfirm()
const toast = useToast()

async function load() {
  loading.value = true
  loadError.value = null
  try {
    history.value = parseFileHistory(
      await invoke<unknown>('get_file_history', { sessionId: props.sessionId, limit: LIST_LIMIT }),
    )
  } catch (e) {
    history.value = { checkpoints: [], total: 0 }
    loadError.value = `Could not read the file history: ${e}`
  } finally {
    loading.value = false
  }
}

async function toggle() {
  open.value = !open.value
  if (open.value) await load()
}

async function restore(c: FileCheckpoint) {
  const ok = await confirm({
    title: 'Restore file?',
    message: `This will ${restoreEffect(c)}. Its current content is saved first, so you can undo this.`,
    confirmLabel: 'Restore',
  })
  if (!ok) return
  restoring.value = c.checkpoint
  try {
    await invoke('restore_file_checkpoint', { sessionId: props.sessionId, checkpoint: c.checkpoint })
    toast.success(`Restored ${fileName(c.path)}`)
    await load()
  } catch (e) {
    toast.error(`Could not restore ${fileName(c.path)}`, String(e))
  } finally {
    restoring.value = null
  }
}

// Another chat's history must never show under this one.
watch(() => props.sessionId, () => {
  open.value = false
  history.value = { checkpoints: [], total: 0 }
})
</script>
