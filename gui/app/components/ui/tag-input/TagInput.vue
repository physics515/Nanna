<script setup lang="ts">
const props = withDefaults(defineProps<{
  placeholder?: string
  disabled?: boolean
}>(), {
  placeholder: 'Add a tag...',
  disabled: false,
})

const model = defineModel<string>({ default: '' })

const tags = computed(() =>
  model.value ? model.value.split(',').map(t => t.trim()).filter(Boolean) : []
)

const inputValue = ref('')

function addTag() {
  const val = inputValue.value.trim()
  if (!val || props.disabled) return
  const current = tags.value
  if (!current.includes(val)) {
    model.value = [...current, val].join(', ')
  }
  inputValue.value = ''
}

function removeTag(index: number) {
  if (props.disabled) return
  const current = [...tags.value]
  current.splice(index, 1)
  model.value = current.join(', ')
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter' || e.key === ',') {
    e.preventDefault()
    addTag()
  }
  if (e.key === 'Backspace' && !inputValue.value && tags.value.length > 0) {
    removeTag(tags.value.length - 1)
  }
}
</script>

<template>
  <div class="tag-input" :class="{ 'tag-input--disabled': disabled }">
    <div class="tag-input__inner">
      <span
        v-for="(tag, i) in tags"
        :key="tag + i"
        class="tag-input__tag"
      >
        <span class="tag-input__tag-mesh" />
        <span class="tag-input__tag-content">
          <span class="tag-input__tag-text">{{ tag }}</span>
          <button
            v-if="!disabled"
            class="tag-input__tag-remove"
            tabindex="-1"
            @click="removeTag(i)"
          >
            <svg class="w-2.5 h-2.5" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="3" y1="3" x2="9" y2="9" />
              <line x1="9" y1="3" x2="3" y2="9" />
            </svg>
          </button>
        </span>
      </span>
      <input
        v-model="inputValue"
        type="text"
        class="tag-input__field"
        :placeholder="tags.length === 0 ? placeholder : ''"
        :disabled="disabled"
        @keydown="onKeydown"
        @blur="addTag"
      >
    </div>
  </div>
</template>

<style scoped>
/* Plain text input container */
.tag-input {
  border: none;
  border-bottom: 1px solid rgba(71, 85, 105, 0.3);
  background: transparent;
  transition: border-color 0.2s ease;
}

.tag-input:focus-within {
  border-bottom-color: rgba(139, 92, 246, 0.5);
}

.tag-input--disabled {
  pointer-events: none;
  opacity: 0.4;
}

.tag-input__inner {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  padding: 6px 0;
  min-height: 36px;
}

/* ════ Tag chip — glass slab ════ */
.tag-input__tag {
  position: relative;
  isolation: isolate;
  overflow: hidden;
  display: inline-flex;
  align-items: center;
  border-radius: 0.375rem;
  font-size: 12px;
  color: #e2e8f0;
  white-space: nowrap;
  /* Glass slab borders */
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  border-left: 1px solid rgba(255, 255, 255, 0.04);
  border-bottom: 1px solid rgba(71, 85, 105, 0.18);
  border-right: 1px solid rgba(71, 85, 105, 0.10);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.04),
    0 1px 1px -0.5px rgba(0, 0, 0, 0.12),
    0 2px 4px -2px rgba(0, 0, 0, 0.10);
  backdrop-filter: blur(18px);
  -webkit-backdrop-filter: blur(18px);
}

/* Mesh gradient on each tag */
.tag-input__tag-mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
  filter: blur(6px);
  background:
    radial-gradient(at 10% 20%, rgba(139, 92, 246, 0.35), transparent 55%),
    radial-gradient(at 90% 80%, rgba(34, 197, 94, 0.25), transparent 55%),
    radial-gradient(at 80% 10%, rgba(100, 116, 139, 0.12), transparent 45%);
}

/* Ground glass overlay on each tag */
.tag-input__tag::after {
  content: '';
  position: absolute;
  inset: 0;
  z-index: 1;
  pointer-events: none;
  border-radius: inherit;
  opacity: 0.18;
  background-blend-mode: soft-light;
  background: repeating-radial-gradient(
    circle,
    #1a2035,
    #1a2035 2px,
    #253050 2px 4px,
    #1a2035 4px 6px,
    #253050 6px 8px,
    #1a2035 8px 10px,
    #253050 10px 12px
  ) 0 0 / 100% 100%;
}

.tag-input__tag-content {
  position: relative;
  z-index: 2;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
}

.tag-input__tag-text {
  line-height: 1.4;
}

.tag-input__tag-remove {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  margin: 0;
  background: transparent;
  border: none;
  color: rgba(196, 205, 214, 0.5);
  cursor: pointer;
  outline: none;
  transition: color 0.15s ease;
}
.tag-input__tag-remove:hover {
  color: rgba(248, 113, 113, 0.8);
}

/* Text input */
.tag-input__field {
  flex: 1;
  min-width: 80px;
  background: transparent;
  border: none;
  outline: none;
  padding: 2px 0;
  font-size: 13px;
  color: #e2e8f0;
}
.tag-input__field::placeholder {
  color: rgba(148, 163, 184, 0.3);
}
</style>
