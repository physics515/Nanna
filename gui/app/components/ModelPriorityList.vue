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
      class="space-y-1 min-h-[60px] p-2 rounded-lg bg-nanna-bg-elevated/30 border border-dashed border-nanna-primary/20"
      @dragover.prevent
      @drop="onDrop($event, 'active')"
    >
      <div v-if="activeModels.length === 0" class="text-center py-4 text-sm text-nanna-text-dim">
        Drag models here to enable
      </div>
      
      <div
        v-for="(model, index) in activeModels"
        :key="model.id"
        :draggable="true"
        @dragstart="onDragStart($event, model, index, 'active')"
        @dragend="onDragEnd"
        @dragover.prevent="onDragOver($event, index)"
        @drop.prevent="onDropOnItem($event, index)"
        :class="[
          'flex items-center gap-2 p-2 rounded-lg cursor-grab active:cursor-grabbing transition-all select-none',
          'bg-nanna-bg-surface border border-nanna-primary/20',
          dragOverIndex === index && 'border-nanna-primary border-dashed bg-nanna-primary/10',
          draggingModel?.id === model.id && 'opacity-50',
          index === 0 && 'ring-1 ring-nanna-accent/50'
        ]"
      >
        <!-- Drag Handle -->
        <GripVertical class="w-4 h-4 text-nanna-text-dim shrink-0 cursor-grab" />
        
        <!-- Priority Badge -->
        <span :class="[
          'w-5 h-5 rounded-full flex items-center justify-center text-xs font-bold shrink-0',
          index === 0 ? 'bg-nanna-accent text-white' : 'bg-nanna-bg-elevated text-nanna-text-muted'
        ]">
          {{ index + 1 }}
        </span>
        
        <!-- Provider Icon -->
        <span class="text-base">{{ getProviderIcon(model.provider) }}</span>
        
        <!-- Model Info -->
        <div class="flex-1 min-w-0">
          <div class="text-sm font-medium text-nanna-text truncate">{{ model.name }}</div>
          <div class="text-xs text-nanna-text-dim">{{ model.provider }}</div>
        </div>
        
        <!-- Status Indicators -->
        <div class="flex items-center gap-1 shrink-0">
          <span v-if="!model.available" class="text-xs text-nanna-warning" title="No API key">⚠️</span>
          <span v-if="index === 0" class="text-xs text-nanna-accent">Primary</span>
        </div>
        
        <!-- Remove Button -->
        <button 
          @click="removeModel(index)"
          class="p-1 rounded hover:bg-nanna-error/20 text-nanna-text-dim hover:text-nanna-error transition-colors"
        >
          <X class="w-3 h-3" />
        </button>
      </div>
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
      <div 
        class="mt-2 space-y-1 p-2 rounded-lg bg-nanna-bg-deep/50"
        @dragover.prevent
        @drop="onDrop($event, 'excluded')"
      >
        <div
          v-for="model in excludedModels"
          :key="model.id"
          draggable="true"
          @dragstart="onDragStart($event, model, -1, 'excluded')"
          @dragend="onDragEnd"
          class="flex items-center gap-2 p-2 rounded bg-nanna-bg-elevated/50 opacity-60 cursor-grab"
        >
          <GripVertical class="w-3 h-3 text-nanna-text-dim" />
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
        @click.self="showAddModel = false"
      >
        <div class="bg-nanna-bg-surface rounded-xl p-4 w-full max-w-sm border border-nanna-primary/20 shadow-xl">
          <h3 class="font-semibold text-nanna-text mb-3">Add Model</h3>
          
          <div class="space-y-2 max-h-64 overflow-y-auto">
            <button
              v-for="model in availableToAdd"
              :key="model.id"
              @click="addModel(model)"
              class="w-full flex items-center gap-2 p-2 rounded-lg hover:bg-nanna-bg-elevated/50 text-left transition-colors"
            >
              <span>{{ getProviderIcon(model.provider) }}</span>
              <div class="flex-1 min-w-0">
                <div class="text-sm text-nanna-text truncate">{{ model.name }}</div>
                <div class="text-xs text-nanna-text-dim">{{ model.provider }}</div>
              </div>
              <span v-if="!model.available" class="text-xs text-nanna-warning">No key</span>
            </button>
          </div>
          
          <div class="flex justify-end mt-4">
            <UiButton @click="showAddModel = false" variant="secondary" size="sm">
              Cancel
            </UiButton>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { GripVertical, X, Plus } from 'lucide-vue-next'

export interface ModelOption {
  id: string
  name: string
  provider: string
  available: boolean // Has API key configured
}

const props = defineProps<{
  label: string
  hint?: string
  models: ModelOption[]
  modelValue: string[] // Ordered list of model IDs
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string[]]
}>()

const showAddModel = ref(false)
const draggedModel = ref<ModelOption | null>(null)
const draggingModel = ref<ModelOption | null>(null)
const dragSource = ref<'active' | 'excluded'>('active')
const dragOverIndex = ref<number | null>(null)
const draggedFromIndex = ref<number>(-1)

// Active models in priority order
const activeModels = computed(() => {
  return props.modelValue
    .map(id => props.models.find(m => m.id === id))
    .filter((m): m is ModelOption => m !== undefined)
})

// Models not in the active list
const excludedModels = computed(() => {
  return props.models.filter(m => !props.modelValue.includes(m.id))
})

// Models available to add (excluded ones)
const availableToAdd = computed(() => excludedModels.value)

function getProviderIcon(provider: string): string {
  const icons: Record<string, string> = {
    anthropic: '🟣',
    openai: '🟢',
    openrouter: '🌐',
    ollama: '🦙',
    google: '🔵',
  }
  return icons[provider.toLowerCase()] || '⚪'
}

function onDragStart(event: DragEvent, model: ModelOption, index: number, source: 'active' | 'excluded') {
  draggedModel.value = model
  draggingModel.value = model
  dragSource.value = source
  draggedFromIndex.value = index
  event.dataTransfer!.effectAllowed = 'move'
  event.dataTransfer!.setData('text/plain', model.id)
  // Set drag image
  if (event.dataTransfer) {
    event.dataTransfer.setDragImage(event.target as HTMLElement, 10, 10)
  }
}

function onDragEnd() {
  draggedModel.value = null
  draggingModel.value = null
  dragOverIndex.value = null
  draggedFromIndex.value = -1
}

function onDragOver(event: DragEvent, index: number) {
  event.preventDefault()
  if (draggedModel.value && draggedFromIndex.value !== index) {
    dragOverIndex.value = index
  }
}

function onDropOnItem(event: DragEvent, targetIndex: number) {
  event.preventDefault()
  if (!draggedModel.value || dragSource.value !== 'active') return
  
  const newList = [...props.modelValue]
  const currentIndex = newList.indexOf(draggedModel.value.id)
  
  if (currentIndex !== -1 && currentIndex !== targetIndex) {
    // Remove from current position
    newList.splice(currentIndex, 1)
    // Insert at target position (adjust if we removed before target)
    const adjustedTarget = currentIndex < targetIndex ? targetIndex - 1 : targetIndex
    newList.splice(adjustedTarget, 0, draggedModel.value.id)
    emit('update:modelValue', newList)
  }
  
  dragOverIndex.value = null
}

function onDrop(event: DragEvent, target: 'active' | 'excluded') {
  event.preventDefault()
  event.stopPropagation()
  
  if (!draggedModel.value) return
  
  const newList = [...props.modelValue]
  
  if (target === 'active') {
    // Remove from current position if in active list
    const currentIndex = newList.indexOf(draggedModel.value.id)
    if (currentIndex !== -1) {
      newList.splice(currentIndex, 1)
    }
    
    // Insert at end (dropped on container, not specific item)
    newList.push(draggedModel.value.id)
  } else {
    // Remove from active list (exclude)
    const index = newList.indexOf(draggedModel.value.id)
    if (index !== -1) {
      newList.splice(index, 1)
    }
  }
  
  emit('update:modelValue', newList)
  dragOverIndex.value = null
  draggingModel.value = null
}

function removeModel(index: number) {
  const newList = [...props.modelValue]
  newList.splice(index, 1)
  emit('update:modelValue', newList)
}

function restoreModel(model: ModelOption) {
  emit('update:modelValue', [...props.modelValue, model.id])
}

function addModel(model: ModelOption) {
  emit('update:modelValue', [...props.modelValue, model.id])
  showAddModel.value = false
}
</script>
