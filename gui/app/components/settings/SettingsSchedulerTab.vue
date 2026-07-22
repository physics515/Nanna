<template>
  <div class="space-y-6">
    <SettingsSection
      title="Scheduler"
      description="Background tasks and periodic self-checks while Nanna is running."
    >
      <div class="space-y-4">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm font-medium text-nanna-text">Enable Scheduler</div>
            <div class="text-xs text-nanna-text-dim">Background tasks</div>
          </div>
          <UiSwitch :model-value="settings?.scheduler_enabled" label="Scheduler enabled" @update:model-value="setSchedulerEnabled" />
        </div>

        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm font-medium text-nanna-text">Enable Heartbeats</div>
            <div class="text-xs text-nanna-text-dim">Periodic self-checks</div>
          </div>
          <UiSwitch :model-value="settings?.heartbeat_enabled" label="Heartbeat enabled" @update:model-value="setHeartbeatEnabled" />
        </div>

        <div class="p-3 rounded-lg bg-nanna-bg-elevated/40">
          <div class="flex items-center justify-between mb-2">
            <span class="text-sm font-medium text-nanna-text">Heartbeat Interval</span>
            <span class="text-sm text-nanna-accent font-mono">{{ formatInterval(settings?.heartbeat_interval_seconds || 300) }}</span>
          </div>
          <input
            type="range" min="60" max="1800" step="60"
            :value="settings?.heartbeat_interval_seconds || 300"
            @change="setHeartbeatInterval(Number(($event.target as HTMLInputElement).value))"
            class="w-full h-2 bg-nanna-bg-deep rounded-lg appearance-none cursor-pointer accent-nanna-primary"
          >
          <div class="flex justify-between text-xs text-nanna-text-dim mt-1">
            <span>1 min</span>
            <span>30 min</span>
          </div>
        </div>
      </div>
    </SettingsSection>
  </div>
</template>

<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { useSettingsPage } from '~/composables/useSettingsPage'

const store = useSettingsPage()
const { settings, loadSettings, showToast } = store

async function setSchedulerEnabled(enabled: boolean) {
  try {
    await invoke('set_scheduler_enabled', { enabled })
    await loadSettings()
  } catch (e: any) {
    showToast(`Couldn't update scheduler: ${e.message || e}`, 'error')
  }
}

async function setHeartbeatEnabled(enabled: boolean) {
  try {
    await invoke('set_heartbeat_enabled', { enabled })
    await loadSettings()
  } catch (e: any) {
    showToast(`Couldn't update heartbeats: ${e.message || e}`, 'error')
  }
}

async function setHeartbeatInterval(seconds: number) {
  try {
    await invoke('set_heartbeat_interval', { seconds })
    await loadSettings()
  } catch (e: any) {
    showToast(`Couldn't update heartbeat interval: ${e.message || e}`, 'error')
  }
}

function formatInterval(seconds: number): string {
  if (seconds < 60) return `${seconds}s`
  return `${Math.floor(seconds / 60)} min`
}
</script>
