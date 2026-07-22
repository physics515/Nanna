<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <header class="px-4 sm:px-6 py-3 sm:py-4 border-b border-white/[0.04] bg-nanna-bg-surface/80">
      <div class="flex items-center gap-3 sm:gap-4">
        <NuxtLink
          to="/"
          class="text-nanna-text-muted hover:text-nanna-text transition-colors"
          aria-label="Back to chat"
          title="Back to chat"
        >
          <ArrowLeft class="w-5 h-5" aria-hidden="true" />
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
      <div
        v-if="!isOnline"
        class="mb-3 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-100 flex items-center justify-between gap-3"
        role="status"
      >
        <span>Daemon not reachable — settings still open locally; saves that need the daemon will retry when it returns.</span>
      </div>

      <UiTabs v-model="activeTab" :tabs="tabs" />

      <div class="mt-3 flex items-center justify-between gap-3">
        <p class="text-[11px] text-nanna-text-muted">
          Advanced reveals rarely-used knobs (iteration floors, compression ratios, host details).
        </p>
        <UiSwitch
          :model-value="showAdvanced"
          label="Show advanced settings"
          class="shrink-0"
          @update:model-value="showAdvanced = $event"
        />
      </div>
    </div>

    <!-- Tab Content -->
    <div ref="tabScrollEl" class="flex-1 overflow-y-auto p-4 sm:p-6 page-shell__body">
      <div class="max-w-2xl mx-auto">

        <!-- Models Tab -->
        <UiTabPanel :active="activeTab === 'models'">
          <SettingsModelsTab />
        </UiTabPanel>

        <!-- Agent Tab -->
        <UiTabPanel :active="activeTab === 'agent'">
          <SettingsAgentTab />
        </UiTabPanel>

        <!-- Memory Tab -->
        <UiTabPanel :active="activeTab === 'memory'">
          <SettingsMemoryTab />
        </UiTabPanel>

        <!-- Tools Tab -->
        <UiTabPanel :active="activeTab === 'tools'">
          <SettingsToolsTab />
        </UiTabPanel>

        <!-- Scheduler Tab -->
        <UiTabPanel :active="activeTab === 'scheduler'">
          <SettingsSchedulerTab />
        </UiTabPanel>

        <!-- Data Tab -->
        <UiTabPanel :active="activeTab === 'data'">
          <SettingsDataTab />
          <div class="mt-8 pt-6 border-t border-white/[0.06]">
            <ShortcutsHelp />
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
import { ref, watch, onMounted, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  ArrowLeft, Save, CheckCircle, XCircle,
  Brain, Bot, BrainCircuit, Wrench, Clock, Database
} from 'lucide-vue-next'
import { provideSettingsPage } from '~/composables/useSettingsPage'
import { useBackend } from '~/composables/useBackend'

const route = useRoute()
const router = useRouter()

const tabs = [
  { id: 'models', label: 'Models', icon: Brain },
  { id: 'agent', label: 'Agent', icon: Bot },
  { id: 'memory', label: 'Memory', icon: BrainCircuit },
  { id: 'tools', label: 'Tools', icon: Wrench },
  { id: 'scheduler', label: 'Scheduler', icon: Clock },
  { id: 'data', label: 'Data', icon: Database },
]

const validTabIds = new Set(tabs.map(t => t.id))

// Shared settings-page store, consumed by the tab components via useSettingsPage()
const store = provideSettingsPage()
const { toast, showToast, showAdvanced } = store

const { isOnline } = useBackend()
const activeTab = ref('models')
const hasChanges = ref(false)
const saving = ref(false)
const tabScrollEl = ref<HTMLElement | null>(null)
/** Per-tab scroll offsets so switching tabs doesn't jump. */
const tabScrollPos = ref<Record<string, number>>({})

function tabFromQuery(raw: unknown): string | null {
  const v = Array.isArray(raw) ? raw[0] : raw
  if (typeof v !== 'string') return null
  return validTabIds.has(v) ? v : null
}

watch(activeTab, (next, prev) => {
  if (prev && tabScrollEl.value) {
    tabScrollPos.value[prev] = tabScrollEl.value.scrollTop
  }
  nextTick(() => {
    if (tabScrollEl.value) {
      tabScrollEl.value.scrollTop = tabScrollPos.value[next] ?? 0
    }
  })
  // Keep ?tab= in sync without stacking history noise
  if (route.query.tab !== next) {
    router.replace({ query: { ...route.query, tab: next } })
  }
})

watch(
  () => route.query.tab,
  (q) => {
    const t = tabFromQuery(q)
    if (t && t !== activeTab.value) activeTab.value = t
  },
)

onMounted(async () => {
  const t = tabFromQuery(route.query.tab)
  if (t) activeTab.value = t
  try {
    showAdvanced.value = localStorage.getItem('nanna.settings.showAdvanced') === '1'
  } catch { /* ignore */ }
  await store.loadSettings()
  await store.loadMemoryStats()
})

watch(showAdvanced, (v) => {
  try {
    localStorage.setItem('nanna.settings.showAdvanced', v ? '1' : '0')
  } catch { /* ignore */ }
})

async function saveAllSettings() {
  saving.value = true
  try {
    await invoke('save_config')
    showToast('Settings saved', 'success')
    hasChanges.value = false
  } catch (e: any) {
    showToast(`Couldn't save settings: ${e.message || e}`, 'error')
  } finally {
    saving.value = false
  }
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

