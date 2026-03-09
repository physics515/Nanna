<script setup lang="ts">
const props = withDefaults(defineProps<{
  disabled?: boolean
  label?: string
}>(), {
  disabled: false,
})

const model = defineModel<boolean>({ default: false })

const { meshBg, containerStyle, onEnter, onLeave } = useGroundGlass()

function toggle() {
  if (props.disabled) return
  model.value = !model.value
}

watch(model, (on) => {
  if (on) onEnter()
  else onLeave()
}, { immediate: true })
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="model"
    :disabled="disabled"
    class="switch"
    :class="{
      'switch--on': model,
      'switch--disabled': disabled,
    }"
    :style="model ? containerStyle : undefined"
    @click="toggle"
  >
    <!-- Mesh gradient (visible when on) -->
    <span class="switch__mesh" :style="model ? { background: meshBg } : undefined" />
    <!-- Ground glass overlay -->
    <span class="switch__glass" />
    <!-- Thumb -->
    <span class="switch__thumb" />
  </button>
</template>

<style scoped>
.switch {
  position: relative;
  isolation: isolate;
  display: inline-flex;
  align-items: center;
  width: 36px;
  height: 20px;
  padding: 0;
  margin: 0;
  border-radius: 999px;
  cursor: pointer;
  outline: none;
  overflow: hidden;
  background: rgba(71, 85, 105, 0.25);
  /* Glass slab borders */
  border-top: 1px solid rgba(255, 255, 255, 0.04);
  border-left: 1px solid rgba(255, 255, 255, 0.03);
  border-bottom: 1px solid rgba(71, 85, 105, 0.18);
  border-right: 1px solid rgba(71, 85, 105, 0.10);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.02),
    0 1px 1px -0.5px rgba(0, 0, 0, 0.12);
  transition: background 0.25s ease, border-color 0.25s ease, box-shadow 0.25s ease;
}

.switch:focus-visible {
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.04),
    0 0 0 2px rgba(139, 92, 246, 0.3);
}

.switch--on {
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  border-left: 1px solid rgba(255, 255, 255, 0.04);
  border-bottom: 1.5px solid rgba(71, 85, 105, 0.18);
  border-right: 1px solid rgba(71, 85, 105, 0.10);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.04),
    0 1.5px 1px -0.5px rgba(0, 0, 0, 0.18),
    0 3px 8px -3px rgba(0, 0, 0, 0.12);
}

.switch--disabled {
  pointer-events: none;
  opacity: 0.4;
}

/* Mesh gradient — fades in when on */
.switch__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
  opacity: 0;
  transition: opacity 0.3s ease;
}

.switch--on .switch__mesh {
  opacity: 1;
}

/* Ground glass overlay */
.switch__glass {
  position: absolute;
  inset: 0;
  z-index: 1;
  pointer-events: none;
  border-radius: inherit;
  opacity: 0;
  transition: opacity 0.3s ease;
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

.switch--on .switch__glass {
  opacity: 0.18;
}

/* Thumb */
.switch__thumb {
  position: relative;
  z-index: 2;
  width: 14px;
  height: 14px;
  border-radius: 999px;
  background: rgba(148, 163, 184, 0.5);
  box-shadow:
    0 1px 3px rgba(0, 0, 0, 0.3),
    inset 0 1px 0 rgba(255, 255, 255, 0.15);
  margin-left: 3px;
  transition: transform 0.25s cubic-bezier(0.4, 0, 0.2, 1),
              background 0.25s ease,
              box-shadow 0.25s ease;
}

.switch--on .switch__thumb {
  transform: translateX(16px);
  background: #f1f5f9;
  box-shadow:
    0 1px 4px rgba(0, 0, 0, 0.4),
    0 0 10px rgba(139, 92, 246, 0.5),
    0 0 20px rgba(139, 92, 246, 0.2),
    inset 0 1px 0 rgba(255, 255, 255, 0.4);
}
</style>
