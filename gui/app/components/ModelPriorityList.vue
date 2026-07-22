<template>
  <div class="space-y-3">
    <div class="flex items-center justify-between">
      <label class="text-sm font-medium text-nanna-text-muted">{{ label }}</label>
      <UiButton @click="showAddModel = true" variant="ghost" size="sm" v-if="availableToAdd.length > 0">
        <Plus class="w-3 h-3 mr-1" />
        Add
      </UiButton>
    </div>

    <!-- Active Models (Drag & Drop) -->
    <div
      class="min-h-[60px] p-2 rounded-lg bg-nanna-bg-elevated/30 border border-dashed border-white/[0.06]"
    >
      <div v-if="localModels.length === 0" class="text-center py-4 text-sm text-nanna-text-dim">
        No models selected. Click "Add" to enable models.
      </div>

      <draggable
        v-model="localModels"
        :item-key="(item: string) => item"
        handle=".drag-handle"
        ghost-class="opacity-50"
        :animation="150"
        :force-fallback="true"
        fallback-class="dragging"
        class="space-y-1"
        @end="onDragEnd"
      >
        <template #item="{ element: modelId, index }">
          <div
            class="flex items-center gap-2 p-2 rounded-lg transition-all select-none glass border-2 border-white/[0.06]"
          >
            <!-- Drag Handle -->
            <GripVertical class="drag-handle w-4 h-4 text-nanna-text-dim shrink-0 cursor-grab active:cursor-grabbing" />

            <!-- Priority Badge -->
            <span class="w-5 h-5 rounded-full flex items-center justify-center text-xs font-bold shrink-0 bg-nanna-bg-elevated text-nanna-text-muted">
              {{ index + 1 }}
            </span>

            <!-- Provider Icon -->
            <span class="text-base">{{ getProviderIcon(getModel(modelId)?.provider) }}</span>

            <!-- Model Info -->
            <div class="flex-1 min-w-0">
              <div class="text-sm font-medium text-nanna-text truncate">{{ getModel(modelId)?.name || modelId }}</div>
              <div class="text-xs text-nanna-text-dim">{{ getModel(modelId)?.provider }}</div>
            </div>

            <!-- Status Indicators -->
            <div class="flex items-center gap-1 shrink-0">
              <span v-if="!getModel(modelId)?.available" class="text-xs text-nanna-warning" :title="getModel(modelId)?.provider === 'ollama' ? 'Ollama not connected' : 'No API key'">⚠️</span>
            </div>

            <!-- Remove Button -->
            <button
              type="button"
              aria-label="Remove model from priority list"
              title="Remove model"
              @click.stop="removeModel(index)"
              class="p-1 rounded hover:bg-nanna-error/20 text-nanna-text-dim hover:text-nanna-error transition-colors"
            >
              <X class="w-3 h-3" />
            </button>
          </div>
        </template>
      </draggable>
    </div>
    
    <!-- Hint -->
    <p class="text-xs text-nanna-text-dim">
      {{ hint || 'Drag to reorder. First available model is used; others are fallbacks.' }}
    </p>
    
    <!-- Excluded Models -->
    <details v-if="excludedModels.length > 0" class="text-sm">
      <summary class="cursor-pointer text-nanna-text-muted hover:text-nanna-text">
        {{ excludedModels.length }} excluded model{{ excludedModels.length > 1 ? 's' : '' }}
      </summary>
      <div class="mt-2 space-y-1 p-2 rounded-lg bg-nanna-bg-deep/50">
        <div
          v-for="model in excludedModels"
          :key="model.id"
          class="flex items-center gap-2 p-2 rounded bg-nanna-bg-elevated/40 opacity-60"
        >
          <span class="text-sm">{{ getProviderIcon(model.provider) }}</span>
          <span class="text-sm text-nanna-text-muted flex-1 truncate">{{ model.name }}</span>
          <button 
            @click="restoreModel(model)"
            class="text-xs text-nanna-accent hover:underline"
          >
            Restore
          </button>
        </div>
      </div>
    </details>
    
    <!-- Add Model Dialog -->
    <Teleport to="body">
      <div
        v-if="showAddModel"
        class="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4"
        @click.self="closeAddDialog"
      >
        <div class="bg-nanna-bg-surface rounded-xl p-4 w-full max-w-sm border border-white/[0.06] shadow-xl">
          <h3 class="font-semibold text-nanna-text mb-3">Add Model</h3>

          <!-- Search Input -->
          <div class="relative mb-3">
            <Search class="absolute left-2.5 top-1/2 -translate-y-1/2 w-4 h-4 text-nanna-text-dim" />
            <input
              ref="searchInputRef"
              v-model="searchQuery"
              type="text"
              placeholder="Search models..."
              class="w-full pl-8 pr-3 py-2 text-sm bg-nanna-bg-elevated rounded-lg border border-nanna-border/30 text-nanna-text placeholder:text-nanna-text-dim/50 focus:outline-none focus:border-nanna-primary"
              @keydown.escape="closeAddDialog"
            />
          </div>

          <div class="space-y-1 max-h-64 overflow-y-auto">
            <div v-if="filteredModelsToAdd.length === 0" class="text-center py-4 text-sm text-nanna-text-dim">
              No models found
            </div>
            <button
              v-for="model in filteredModelsToAdd"
              :key="model.id"
              @click="addModel(model)"
              class="w-full flex items-center gap-2 p-2 rounded-lg hover:bg-white/[0.06] text-left transition-colors"
            >
              <span>{{ getProviderIcon(model.provider) }}</span>
              <div class="flex-1 min-w-0">
                <div class="text-sm text-nanna-text truncate">{{ model.name }}</div>
                <div class="text-xs text-nanna-text-dim">{{ model.provider }}</div>
              </div>
              <span v-if="!model.available" class="text-xs text-nanna-warning">{{ model.provider === 'ollama' ? 'Not connected' : 'No key' }}</span>
            </button>
          </div>

          <div class="flex justify-end mt-4">
            <UiButton @click="closeAddDialog" variant="secondary" size="sm">
              Cancel
            </UiButton>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, nextTick } from 'vue'
import draggable from 'vuedraggable'
import { GripVertical, X, Plus, Search } from 'lucide-vue-next'

export interface ModelOption {
  id: string
  name: string
  provider: string
  available: boolean
}

const props = defineProps<{
  label: string
  hint?: string
  allModels: ModelOption[]
  modelValue: string[]
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string[]]
}>()

const showAddModel = ref(false)
const searchQuery = ref('')
const searchInputRef = ref<HTMLInputElement | null>(null)

// Local copy of models for vuedraggable
const localModels = ref<string[]>([...props.modelValue])

// Watch for external changes to modelValue
watch(() => props.modelValue, (newVal) => {
  if (JSON.stringify(newVal) !== JSON.stringify(localModels.value)) {
    localModels.value = [...newVal]
  }
}, { deep: true })

// Emit changes after drag ends
function onDragEnd() {
  emit('update:modelValue', [...localModels.value])
}

// Helper to get model info by ID
function getModel(id: string): ModelOption | undefined {
  return props.allModels.find(m => m.id === id)
}

// Models not in the active list
const excludedModels = computed(() => {
  return props.allModels.filter(m => !localModels.value.includes(m.id))
})

const availableToAdd = computed(() => excludedModels.value)

// Filtered models based on search query
const filteredModelsToAdd = computed(() => {
  const query = searchQuery.value.toLowerCase().trim()
  if (!query) return availableToAdd.value

  return availableToAdd.value.filter(m =>
    m.name.toLowerCase().includes(query) ||
    m.id.toLowerCase().includes(query) ||
    m.provider.toLowerCase().includes(query)
  )
})

function closeAddDialog() {
  showAddModel.value = false
  searchQuery.value = ''
}

// Focus search input when dialog opens
watch(showAddModel, async (open) => {
  if (open) {
    await nextTick()
    searchInputRef.value?.focus()
  }
})

function getProviderIcon(provider: string | undefined): string {
  if (!provider) return '⚪'
  const icons: Record<string, string> = {
    anthropic: '🟣',
    openai: '🟢',
    openrouter: '🌐',
    ollama: '🦙',
    google: '🔵',
    github: '🐙',
    'claude-proxy': '🔀',
  }
  return icons[provider.toLowerCase()] || '⚪'
}

function removeModel(index: number) {
  const newList = [...localModels.value]
  newList.splice(index, 1)
  localModels.value = newList
  emit('update:modelValue', [...localModels.value])
}

function restoreModel(model: ModelOption) {
  localModels.value = [...localModels.value, model.id]
  emit('update:modelValue', [...localModels.value])
}

function addModel(model: ModelOption) {
  localModels.value = [...localModels.value, model.id]
  emit('update:modelValue', [...localModels.value])
  closeAddDialog()
}
</script>

<style scoped>
.dragging {
  opacity: 0.8;
  background: var(--nanna-bg-surface);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
}
</style>
