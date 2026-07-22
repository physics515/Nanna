<template>
  <div>
    <label class="block text-sm font-medium text-nanna-text mb-1">
      {{ label }}
      <UiBadge v-if="isSet" variant="success" class="ml-2">Set</UiBadge>
    </label>
    <div class="flex gap-2">
      <div class="relative flex-1">
        <UiInput
          v-model="keyInput"
          :type="showKey ? 'text' : 'password'"
          :placeholder="placeholder"
          class="pr-10"
        />
        <button
          type="button"
          :aria-label="showKey ? 'Hide API key' : 'Show API key'"
          :title="showKey ? 'Hide API key' : 'Show API key'"
          @click="showKey = !showKey"
          class="absolute right-2 top-1/2 -translate-y-1/2 text-nanna-text-dim hover:text-nanna-text transition-colors p-1"
        >
          <Eye v-if="!showKey" class="w-4 h-4" />
          <EyeOff v-else class="w-4 h-4" />
        </button>
      </div>
      <UiButton @click="handleSave" :disabled="!keyInput.trim()" size="sm">
        Save
      </UiButton>
    </div>
    <p v-if="hint" class="text-xs text-nanna-text-dim mt-1">{{ hint }}</p>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { Eye, EyeOff } from 'lucide-vue-next'

const props = defineProps<{
  label: string
  provider: string
  placeholder?: string
  isSet?: boolean
  hint?: string
}>()

const emit = defineEmits<{
  save: [provider: string, key: string]
}>()

const keyInput = ref('')
const showKey = ref(false)

function handleSave() {
  if (keyInput.value.trim()) {
    emit('save', props.provider, keyInput.value.trim())
    keyInput.value = ''
    showKey.value = false
  }
}
</script>
