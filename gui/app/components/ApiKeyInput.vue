<template>
  <div class="p-4 rounded-lg bg-nanna-bg-elevated/50 border border-nanna-primary/5">
    <div class="flex items-center justify-between mb-3">
      <div class="flex items-center gap-2">
        <span class="text-sm font-medium text-nanna-text">{{ label }}</span>
        <span 
          :class="[
            'text-xs px-2 py-0.5 rounded-full',
            isSet 
              ? 'bg-nanna-success/20 text-nanna-success' 
              : 'bg-nanna-warning/20 text-nanna-warning'
          ]"
        >
          {{ isSet ? '✓ Configured' : '○ Not set' }}
        </span>
      </div>
    </div>
    
    <div class="flex gap-2">
      <div class="relative flex-1">
        <input
          v-model="apiKey"
          :type="showKey ? 'text' : 'password'"
          :placeholder="placeholder"
          class="input w-full font-mono text-sm pr-10"
          @keydown.enter="save"
        />
        <button 
          @click="showKey = !showKey"
          class="absolute right-2 top-1/2 -translate-y-1/2 text-nanna-text-dim hover:text-nanna-text transition-colors"
          type="button"
        >
          {{ showKey ? '🙈' : '👁️' }}
        </button>
      </div>
      <button 
        @click="save"
        class="btn-primary text-sm"
        :disabled="!apiKey.trim()"
      >
        Save
      </button>
    </div>
    
    <p v-if="hint" class="text-xs text-nanna-text-dim mt-2">
      {{ hint }}
    </p>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'

const props = defineProps<{
  label: string
  provider: string
  placeholder: string
  isSet?: boolean
  hint?: string
}>()

const emit = defineEmits<{
  (e: 'save', provider: string, apiKey: string): void
}>()

const apiKey = ref('')
const showKey = ref(false)

function save() {
  if (!apiKey.value.trim()) return
  emit('save', props.provider, apiKey.value.trim())
  apiKey.value = ''
}
</script>
