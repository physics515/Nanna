<template>
  <div class="space-y-6">
    <!-- Sessions -->
    <UiCard>
      <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
        <Database class="w-4 h-4" />
        Data Management
      </h3>
      <div class="space-y-4">
        <div class="flex items-center justify-between p-3 rounded-lg bg-nanna-bg-elevated/40">
          <div>
            <div class="text-sm font-medium text-nanna-text">Chat Sessions</div>
            <div class="text-xs text-nanna-text-dim">{{ sessionCount }} sessions stored</div>
          </div>
          <UiButton @click="confirmClearSessions" variant="destructive" size="sm">
            <Trash2 class="w-4 h-4 mr-1" />
            Clear All
          </UiButton>
        </div>

        <div class="flex items-center justify-between p-3 rounded-lg bg-nanna-bg-elevated/40">
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
      <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
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
      <h3 class="text-base font-semibold text-nanna-primary mb-4 flex items-center gap-2">
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
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Database, Trash2, FileDown, FileUp, Moon } from 'lucide-vue-next'
import { useConfirm } from '~/composables/useConfirm'
import { useSettingsPage } from '~/composables/useSettingsPage'

const store = useSettingsPage()
const { memoryStats, loadMemoryStats, loadSettings, showToast } = store

const { confirm } = useConfirm()

const sessionCount = ref(0)

const configPath = computed(() => {
  if (navigator.platform.includes('Win')) {
    return '%APPDATA%\\clawd\\Nanna\\config\\config.toml'
  } else if (navigator.platform.includes('Mac')) {
    return '~/Library/Application Support/clawd.Nanna/config.toml'
  } else {
    return '~/.config/nanna/config.toml'
  }
})

onMounted(async () => {
  await loadSessions()
})

async function loadSessions() {
  try {
    const sessions = await invoke<{ id: string }[]>('list_sessions')
    sessionCount.value = sessions.length
  } catch (e) {
    console.error('Failed to load sessions:', e)
  }
}

async function confirmClearSessions() {
  const confirmed = await confirm({
    title: 'Delete All Sessions',
    message: 'Delete all chat sessions? This cannot be undone.',
    confirmText: 'Delete All',
    destructive: true
  })

  if (!confirmed) return

  try {
    const count = await invoke<number>('clear_all_sessions')
    showToast(`Cleared ${count} sessions`, 'success')
    sessionCount.value = 0
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function confirmClearMemories() {
  const confirmed = await confirm({
    title: 'Delete All Memories',
    message: 'Delete all memories? This cannot be undone.',
    confirmText: 'Delete All',
    destructive: true
  })

  if (!confirmed) return

  try {
    await invoke('clear_all_memories')
    showToast('All memories cleared', 'success')
    await loadMemoryStats()
  } catch (e: any) {
    showToast(`Failed: ${e.message || e}`, 'error')
  }
}

async function exportConfig() {
  try {
    const config = await invoke<string>('export_config')

    // Create a downloadable blob
    const blob = new Blob([config], { type: 'text/plain' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = 'nanna-config.toml'
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)

    showToast('Configuration exported', 'success')
  } catch (e: any) {
    showToast(`Export failed: ${e.message || e}`, 'error')
  }
}

async function importConfig() {
  try {
    // Create file input and trigger it
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = '.toml'

    input.onchange = async (e) => {
      const file = (e.target as HTMLInputElement).files?.[0]
      if (!file) return

      if (!confirm('This will replace your current configuration. Continue?')) return

      const content = await file.text()
      await invoke('import_config', { config: content })
      showToast('Configuration imported', 'success')
      await loadSettings()
    }

    input.click()
  } catch (e: any) {
    showToast(`Import failed: ${e.message || e}`, 'error')
  }
}
</script>
