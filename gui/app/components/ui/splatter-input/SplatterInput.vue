<script setup lang="ts">
import type { MaybeRefOrGetter } from 'vue'
import { useSplatter } from '~/composables/useSplatter'

const props = withDefaults(defineProps<{
  modelValue?: string
  placeholder?: string
  type?: string
  disabled?: boolean
  colors?: MaybeRefOrGetter<[string, string, string]>
  opacityRanges?: [[number, number], [number, number], [number, number]]
  sizes?: [string, string, string]
  class?: string
}>(), {
  type: 'text',
  disabled: false,
  colors: () => ['139,92,246', '99,102,241', '167,139,250'] as [string, string, string],
  opacityRanges: () => [[0.06, 0.08], [0.04, 0.06], [0.03, 0.05]] as [[number, number], [number, number], [number, number]],
  sizes: () => ['65%', '60%', '50%'] as [string, string, string],
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'input': [event: Event]
  'focus': [event: FocusEvent]
  'blur': [event: FocusEvent]
}>()

const isFocused = ref(false)

const { splatterBg, onEnter, onLeave } = useSplatter({
  colors: props.colors,
  opacityRanges: props.opacityRanges,
  sizes: props.sizes,
})

function handleFocus(e: FocusEvent) {
  isFocused.value = true
  onEnter()
  emit('focus', e)
}

function handleBlur(e: FocusEvent) {
  isFocused.value = false
  onLeave()
  emit('blur', e)
}

function handleInput(e: Event) {
  const target = e.target as HTMLInputElement
  emit('update:modelValue', target.value)
  emit('input', e)
}
</script>

<template>
  <div
    :class="[
      'splatter-input',
      { 'splatter-input--focused': isFocused, 'splatter-input--disabled': disabled },
      props.class,
    ]"
    @mouseenter="!disabled && onEnter()"
    @mouseleave="!disabled && !isFocused && onLeave()"
  >
    <span class="splatter-input__mesh" :style="{ background: splatterBg }" />
    <slot name="prefix" />
    <input
      :type="type"
      :value="modelValue"
      :placeholder="placeholder"
      :disabled="disabled"
      class="splatter-input__field"
      @input="handleInput"
      @focus="handleFocus"
      @blur="handleBlur"
    />
    <slot name="suffix" />
  </div>
</template>

<style scoped>
.splatter-input {
  position: relative;
  isolation: isolate;
  overflow: hidden;
  display: flex;
  align-items: center;
  gap: 0.5rem;
  border-radius: 0.5rem;
  border: 1px solid rgba(255, 255, 255, 0.06);
  padding: 0.375rem 0.75rem;
  transition: border-color 0.2s ease;
}

.splatter-input:hover {
  border-color: rgba(255, 255, 255, 0.08);
}

.splatter-input--focused {
  border-color: rgba(255, 255, 255, 0.12);
}

.splatter-input--disabled {
  opacity: 0.4;
  pointer-events: none;
}

.splatter-input__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
  transition: opacity 0.4s ease;
  opacity: 0.5;
  -webkit-mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch' result='noise'/%3E%3CfeColorMatrix type='saturate' values='0' in='noise' result='gray'/%3E%3CfeColorMatrix type='matrix' in='gray' values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 1 0 0 0 0' result='a'/%3E%3CfeComponentTransfer in='a'%3E%3CfeFuncA type='linear' slope='0.9' intercept='0.05'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch' result='noise'/%3E%3CfeColorMatrix type='saturate' values='0' in='noise' result='gray'/%3E%3CfeColorMatrix type='matrix' in='gray' values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 1 0 0 0 0' result='a'/%3E%3CfeComponentTransfer in='a'%3E%3CfeFuncA type='linear' slope='0.9' intercept='0.05'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  -webkit-mask-size: 200px 200px;
  mask-size: 200px 200px;
}

.splatter-input--focused .splatter-input__mesh,
.splatter-input:hover .splatter-input__mesh {
  opacity: 1;
}

.splatter-input__field {
  position: relative;
  z-index: 1;
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  color: rgba(255, 255, 255, 0.8);
  font-size: 0.875rem;
  line-height: 1.5;
}

.splatter-input__field::placeholder {
  color: rgba(255, 255, 255, 0.2);
}
</style>
