<script setup lang="ts">
const props = withDefaults(defineProps<{
  placeholder?: string
  disabled?: boolean
  mono?: boolean
}>(), {
  disabled: false,
  mono: false,
})

const model = defineModel<string>({ default: '' })

const glassRef = ref<{ onEnter: () => void; onLeave: () => void } | null>(null)

function onFocusIn() {
  if (!props.disabled) glassRef.value?.onEnter()
}

function onFocusOut() {
  glassRef.value?.onLeave()
}
</script>

<template>
  <GroundGlass
    ref="glassRef"
    class="glass-input"
    :class="{ 'glass-input--disabled': disabled }"
    @focusin="onFocusIn"
    @focusout="onFocusOut"
  >
    <input
      v-model="model"
      type="text"
      class="glass-input__field"
      :class="{ 'font-mono': mono }"
      :placeholder="placeholder"
      :disabled="disabled"
    >
  </GroundGlass>
</template>

<style scoped>
.glass-input {
  display: flex;
  align-items: center;
  border-radius: 0.5rem;
  transition: border-color 0.2s ease;
}

.glass-input:focus-within {
  border-color: rgba(139, 92, 246, 0.4);
}

.glass-input--disabled {
  pointer-events: none;
  opacity: 0.4;
}

.glass-input__field {
  position: relative;
  z-index: 1;
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  padding: 8px 12px;
  font-size: 16px;
  color: #e2e8f0;
}

.glass-input__field::placeholder {
  color: rgba(255, 255, 255, 0.35);
}
</style>
