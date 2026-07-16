<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <header class="px-4 sm:px-6 py-3 sm:py-4 border-b border-white/[0.04] bg-nanna-bg-surface/80">
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
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  ArrowLeft, Save, CheckCircle, XCircle,
  Brain, Bot, BrainCircuit, Wrench, Clock, Database
} from 'lucide-vue-next'
import { provideSettingsPage } from '~/composables/useSettingsPage'

const tabs = [
  { id: 'models', label: 'Models', icon: Brain },
  { id: 'agent', label: 'Agent', icon: Bot },
  { id: 'memory', label: 'Memory', icon: BrainCircuit },
  { id: 'tools', label: 'Tools', icon: Wrench },
  { id: 'scheduler', label: 'Scheduler', icon: Clock },
  { id: 'data', label: 'Data', icon: Database },
]

// Shared settings-page store, consumed by the tab components via useSettingsPage()
const store = provideSettingsPage()
const { toast, showToast } = store

const activeTab = ref('models')
const hasChanges = ref(false)
const saving = ref(false)

onMounted(async () => {
  await store.loadSettings()
  await store.loadMemoryStats()
})

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
