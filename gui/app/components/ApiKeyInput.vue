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
          :aria-invalid="!!error"
          :class="error ? 'border-nanna-error' : ''"
          @keydown.enter.prevent="handleSave"
        />
        <button
          type="button"
          :aria-label="showKey ? 'Hide API key' : 'Show API key'"
          :title="showKey ? 'Hide API key' : 'Show API key'"
          class="absolute right-2 top-1/2 -translate-y-1/2 text-nanna-text-dim hover:text-nanna-text transition-colors p-1 min-h-8 min-w-8"
          @click="showKey = !showKey"
        >
          <Eye v-if="!showKey" class="w-4 h-4" />
          <EyeOff v-else class="w-4 h-4" />
        </button>
      </div>
      <UiButton
        size="sm"
        class="min-h-8"
        :disabled="!keyInput.trim() || saving"
        :aria-busy="saving"
        @click="handleSave"
      >
        {{ saving ? 'Saving…' : 'Save' }}
      </UiButton>
    </div>
    <p v-if="error" class="text-xs text-nanna-error mt-1" role="alert">{{ error }}</p>
    <p v-else-if="hint" class="text-xs text-nanna-text-dim mt-1">{{ hint }}</p>
  </div>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import { Eye, EyeOff } from 'lucide-vue-next'
import { validateApiKey } from '~/lib/formValidation'

const props = defineProps<{
  label: string
  provider: string
  placeholder?: string
  isSet?: boolean
  hint?: string
  /** Parent sets true while the save invoke is in flight. */
  saving?: boolean
  /** Parent-provided error (network / reject). Local validation shows first. */
  externalError?: string | null
}>()

const emit = defineEmits<{
  save: [provider: string, key: string]
}>()

const keyInput = ref('')
const showKey = ref(false)
const error = ref<string | null>(null)

watch(
  () => props.externalError,
  (v) => {
    if (v) error.value = v
  },
)

watch(keyInput, () => {
  // Clear stale validation as the user edits — don't wipe a successful key mid-type unnecessarily.
  if (error.value) error.value = null
})

function handleSave() {
  if (props.saving) return
  const raw = keyInput.value
  const result = validateApiKey(raw)
  if (!result.ok) {
    error.value = result.error
    return
  }
  error.value = null
  emit('save', props.provider, result.value)
  // Keep the field populated until parent confirms success via isSet flip
  // so a partial failure doesn't wipe a carefully pasted key.
}
</script>
