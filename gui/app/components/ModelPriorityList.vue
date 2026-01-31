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
      ref="dropZone"
      class="space-y-1 min-h-[60px] p-2 rounded-lg bg-nanna-bg-elevated/30 border border-dashed border-nanna-primary/20 transition-colors"
      :class="isDragging && !draggingFromExcluded && 'border-nanna-primary/40'"
      @dragover="handleContainerDragOver"
      @dragenter="handleContainerDragOver"
      @drop="handleContainerDrop"
    >
      <div v-if="activeModels.length === 0" class="text-center py-4 text-sm text-nanna-text-dim">
        Drag models here to enable
      </div>
      
      <div
        v-for="(model, index) in activeModels"
        :key="model.id"
        draggable="true"
        @dragstart="handleDragStart($event, model, index)"
        @dragend="handleDragEnd"
        @dragenter="handleItemDragEnter($event, index)"
        @dragover="handleItemDragOver($event, index)"
        @dragleave="handleItemDragLeave($event, index)"
        @drop="handleItemDrop($event, index)"
        :class="[
          'flex items-center gap-2 p-2 rounded-lg cursor-grab active:cursor-grabbing transition-all select-none',
          'bg-nanna-bg-surface border-2',
          getItemClasses(model, index)
        ]"
      >
        <!-- Drag Handle -->
        <GripVertical class="w-4 h-4 text-nanna-text-dim shrink-0" />
        
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
        @dragover="handleContainerDragOver"
        @dragenter="handleContainerDragOver"
        @drop="handleExcludedDrop"
      >
        <div
          v-for="model in excludedModels"
          :key="model.id"
          draggable="true"
          @dragstart="handleExcludedDragStart($event, model)"
          @dragend="handleDragEnd"
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
import { ref, computed } from 'vue'
import { GripVertical, X, Plus } from 'lucide-vue-next'

export interface ModelOption {
  id: string
  name: string
  provider: string
  available: boolean
}

const props = defineProps<{
  label: string
  hint?: string
  models: ModelOption[]
  modelValue: string[]
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string[]]
}>()

const showAddModel = ref(false)

// Drag state
const draggingId = ref<string | null>(null)
const draggingFromExcluded = ref(false)
const dropTargetIndex = ref<number | null>(null)
const dragStartIndex = ref<number>(-1)

const isDragging = computed(() => draggingId.value !== null)

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

function getItemClasses(model: ModelOption, index: number): string {
  const classes: string[] = []
  
  if (draggingId.value === model.id) {
    classes.push('opacity-40', 'scale-95', 'border-nanna-primary/20')
  } else if (dropTargetIndex.value === index) {
    classes.push('border-nanna-accent', 'bg-nanna-accent/10')
  } else if (index === 0) {
    classes.push('border-nanna-primary/30', 'ring-1', 'ring-nanna-accent/50')
  } else {
    classes.push('border-nanna-primary/20')
  }
  
  return classes.join(' ')
}

// === DRAG HANDLERS ===

function handleDragStart(event: DragEvent, model: ModelOption, index: number) {
  draggingId.value = model.id
  draggingFromExcluded.value = false
  dragStartIndex.value = index
  
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = 'move'
    event.dataTransfer.dropEffect = 'move'
    event.dataTransfer.setData('text/plain', model.id)
  }
}

function handleExcludedDragStart(event: DragEvent, model: ModelOption) {
  draggingId.value = model.id
  draggingFromExcluded.value = true
  dragStartIndex.value = -1
  
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = 'move'
    event.dataTransfer.dropEffect = 'move'
    event.dataTransfer.setData('text/plain', model.id)
  }
}

function handleDragEnd() {
  draggingId.value = null
  draggingFromExcluded.value = false
  dropTargetIndex.value = null
  dragStartIndex.value = -1
}

function handleContainerDragOver(event: DragEvent) {
  event.preventDefault()
  event.stopPropagation()
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = 'move'
  }
}

function handleItemDragEnter(event: DragEvent, index: number) {
  event.preventDefault()
  event.stopPropagation()
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = 'move'
  }
  if (draggingId.value && dragStartIndex.value !== index) {
    dropTargetIndex.value = index
  }
}

function handleItemDragOver(event: DragEvent, index: number) {
  event.preventDefault()
  event.stopPropagation()
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = 'move'
  }
  if (draggingId.value && dragStartIndex.value !== index) {
    dropTargetIndex.value = index
  }
}

function handleItemDragLeave(event: DragEvent, index: number) {
  // Only clear if we're actually leaving this item (not entering a child)
  const relatedTarget = event.relatedTarget as HTMLElement | null
  const currentTarget = event.currentTarget as HTMLElement
  if (relatedTarget && currentTarget.contains(relatedTarget)) {
    return
  }
  if (dropTargetIndex.value === index) {
    dropTargetIndex.value = null
  }
}

function handleItemDrop(event: DragEvent, targetIndex: number) {
  event.preventDefault()
  event.stopPropagation()
  
  if (!draggingId.value) return
  
  const newList = [...props.modelValue]
  
  if (draggingFromExcluded.value) {
    // Adding from excluded - insert at target position
    newList.splice(targetIndex, 0, draggingId.value)
  } else {
    // Reordering within active list
    const currentIndex = newList.indexOf(draggingId.value)
    if (currentIndex === -1 || currentIndex === targetIndex) {
      handleDragEnd()
      return
    }
    
    // Remove and reinsert
    newList.splice(currentIndex, 1)
    const insertAt = currentIndex < targetIndex ? targetIndex - 1 : targetIndex
    newList.splice(insertAt, 0, draggingId.value)
  }
  
  emit('update:modelValue', newList)
  handleDragEnd()
}

function handleContainerDrop(event: DragEvent) {
  event.preventDefault()
  event.stopPropagation()
  
  if (!draggingId.value) return
  
  const newList = [...props.modelValue]
  
  if (draggingFromExcluded.value) {
    if (!newList.includes(draggingId.value)) {
      newList.push(draggingId.value)
    }
    emit('update:modelValue', newList)
  }
  
  handleDragEnd()
}

function handleExcludedDrop(event: DragEvent) {
  event.preventDefault()
  event.stopPropagation()
  
  if (!draggingId.value || draggingFromExcluded.value) return
  
  const newList = props.modelValue.filter(id => id !== draggingId.value)
  emit('update:modelValue', newList)
  handleDragEnd()
}

// === NON-DRAG ACTIONS ===

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
