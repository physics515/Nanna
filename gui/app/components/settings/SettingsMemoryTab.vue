<template>
  <div class="space-y-6">
    <SettingsSection
      title="Cognitive Memory"
      description="How Nanna recalls and consolidates what it learns over time."
    >
      <template #icon>
        <BrainCircuit class="w-4 h-4 text-nanna-primary" />
      </template>

      <div class="flex items-center justify-end -mt-1">
        <UiBadge variant="secondary">FSRS-6</UiBadge>
      </div>

      <!-- Stats Grid -->
      <div class="grid grid-cols-4 gap-2">
        <div class="p-2 rounded-lg bg-nanna-bg-elevated/40 text-center">
          <div class="text-lg font-bold text-nanna-success">{{ memoryStats?.active || 0 }}</div>
          <div class="text-xs text-nanna-text-dim">Active</div>
        </div>
        <div class="p-2 rounded-lg bg-nanna-bg-elevated/40 text-center">
          <div class="text-lg font-bold text-nanna-warning">{{ memoryStats?.dormant || 0 }}</div>
          <div class="text-xs text-nanna-text-dim">Dormant</div>
        </div>
        <div class="p-2 rounded-lg bg-nanna-bg-elevated/40 text-center">
          <div class="text-lg font-bold text-nanna-text-muted">{{ memoryStats?.silent || 0 }}</div>
          <div class="text-xs text-nanna-text-dim">Silent</div>
        </div>
        <div class="p-2 rounded-lg bg-nanna-bg-elevated/40 text-center">
          <div class="text-lg font-bold text-nanna-error">{{ memoryStats?.unavailable || 0 }}</div>
          <div class="text-xs text-nanna-text-dim">Faded</div>
        </div>
      </div>

      <!-- Similarity Threshold -->
      <div class="p-3 rounded-lg bg-nanna-bg-elevated/40">
        <div class="flex items-center justify-between mb-2">
          <span class="text-sm font-medium text-nanna-text">Recall Threshold</span>
          <span class="text-sm text-nanna-accent font-mono">{{ (similarityThreshold * 100).toFixed(0) }}%</span>
        </div>
        <input
          type="range" min="0" max="100" step="5"
          :value="similarityThreshold * 100"
          @change="setSimilarityThreshold(Number(($event.target as HTMLInputElement).value) / 100)"
          class="w-full h-2 bg-nanna-bg-deep rounded-lg appearance-none cursor-pointer accent-nanna-primary"
        >
        <p class="text-xs text-nanna-text-dim mt-1">Lower = more results, higher = more precise</p>
      </div>

      <!-- Toggles -->
      <div class="flex items-center justify-between">
        <div>
          <div class="text-sm font-medium text-nanna-text">Enable Dreaming</div>
          <div class="text-xs text-nanna-text-dim">Memory consolidation</div>
        </div>
        <UiSwitch :model-value="settings?.dreaming_enabled" label="Dreaming enabled" @update:model-value="setDreamingEnabled" />
      </div>

      <!-- Advanced consolidation knobs -->
      <div v-if="showAdvanced" class="space-y-3">
        <div class="p-3 rounded-lg bg-nanna-bg-elevated/40">
          <div class="flex items-center justify-between mb-2">
            <span class="text-sm font-medium text-nanna-text">Max Compression</span>
            <span class="text-sm text-nanna-accent font-mono">{{ ((settings?.max_compression_ratio ?? 0.5) * 100).toFixed(0) }}%</span>
          </div>
          <input
            type="range" min="10" max="90" step="5"
            :value="(settings?.max_compression_ratio ?? 0.5) * 100"
            @change="setMaxCompressionRatio(Number(($event.target as HTMLInputElement).value) / 100)"
            class="w-full h-2 bg-nanna-bg-deep rounded-lg appearance-none cursor-pointer accent-nanna-primary"
          >
          <p class="text-xs text-nanna-text-dim mt-1">Max fraction of memories that can be merged per consolidation run</p>
        </div>
        <div class="p-3 rounded-lg bg-nanna-bg-elevated/40">
          <div class="flex items-center justify-between mb-2">
            <span class="text-sm font-medium text-nanna-text">Min Memories Floor</span>
            <span class="text-sm text-nanna-accent font-mono">{{ settings?.min_remaining_memories ?? 20 }}</span>
          </div>
          <input
            type="range" min="5" max="200" step="5"
            :value="settings?.min_remaining_memories ?? 20"
            @change="setMinRemainingMemories(Number(($event.target as HTMLInputElement).value))"
            class="w-full h-2 bg-nanna-bg-deep rounded-lg appearance-none cursor-pointer accent-nanna-primary"
          >
          <p class="text-xs text-nanna-text-dim mt-1">Never consolidate below this many memories</p>
        </div>
      </div>

      <!-- Dream Button -->
      <UiButton @click="triggerConsolidation" :disabled="consolidating || !settings?.dreaming_enabled" class="w-full">
        <UiSpinner v-if="consolidating" size="sm" class="mr-2" />
        <Moon v-else class="w-4 h-4 mr-2" />
        {{ consolidating ? 'Dreaming...' : 'Dream Now' }}
      </UiButton>
    </SettingsSection>
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { BrainCircuit, Moon } from 'lucide-vue-next'
import { useSettingsPage } from '~/composables/useSettingsPage'

const store = useSettingsPage()
const { settings, showAdvanced, memoryStats, loadMemoryStats, loadSettings, showToast } = store

const similarityThreshold = ref(0.4)
const consolidating = ref(false)

onMounted(async () => {
  await loadSimilarityThreshold()
})

async function loadSimilarityThreshold() {
  try {
    similarityThreshold.value = await invoke<number>('get_similarity_threshold')
  } catch (e) {
    console.error('Could not load similarity threshold:', e)
  }
}

async function setSimilarityThreshold(value: number) {
  try {
    await invoke<string>('set_similarity_threshold', { threshold: value })
    similarityThreshold.value = value
  } catch (e: any) {
    showToast(`Could not update recall threshold: ${e.message || e}`, 'error')
  }
}

async function setDreamingEnabled(enabled: boolean) {
  try {
    await invoke('set_dreaming_enabled', { enabled })
    await loadSettings()
  } catch (e: any) {
    showToast(`Could not update dreaming: ${e.message || e}`, 'error')
  }
}

async function setMaxCompressionRatio(value: number) {
  try {
    await invoke('set_max_compression_ratio', { ratio: value })
    if (settings.value) settings.value.max_compression_ratio = value
  } catch (e: any) {
    showToast(`Could not update compression: ${e.message || e}`, 'error')
  }
}

async function setMinRemainingMemories(value: number) {
  try {
    await invoke('set_min_remaining_memories', { count: value })
    if (settings.value) settings.value.min_remaining_memories = value
  } catch (e: any) {
    showToast(`Could not update memory floor: ${e.message || e}`, 'error')
  }
}

async function triggerConsolidation() {
  consolidating.value = true
  try {
    const result = await invoke<{ memories_processed: number; memories_merged: number }>('trigger_consolidation')
    showToast(`Dreaming complete: ${result.memories_processed} processed`, 'success')
    await loadMemoryStats()
  } catch (e: any) {
    showToast(`Dreaming did not finish: ${e.message || e}`, 'error')
  } finally {
    consolidating.value = false
  }
}
</script>
