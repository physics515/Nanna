<script setup lang="ts">
export interface TabItem {
  label: string
  value: string
}

const props = defineProps<{
  items: TabItem[]
  disabled?: boolean
}>()

const model = defineModel<string>({ required: true })

const barRef = ref<HTMLElement | null>(null)
const tabRefs = ref<Record<string, HTMLElement | null>>({})
const indicatorStyle = ref<Record<string, string>>({ opacity: '0' })
const overlayVisible = ref(true)
const overlayKey = ref(0)

const activeLabel = computed(() => {
  const item = props.items.find(i => i.value === model.value)
  return item?.label ?? ''
})

function measure() {
  const bar = barRef.value
  if (!bar || !model.value) return
  const el = tabRefs.value[model.value]
  if (!el) return
  const barRect = bar.getBoundingClientRect()
  const tabRect = el.getBoundingClientRect()
  indicatorStyle.value = {
    left: `${tabRect.left - barRect.left}px`,
    width: `${tabRect.width}px`,
    opacity: '1',
  }
}

function select(value: string) {
  if (props.disabled) return
  model.value = value
}

watch(model, () => {
  overlayVisible.value = false
  overlayKey.value++
  nextTick(measure)
  setTimeout(() => { overlayVisible.value = true }, 320)
})

onMounted(() => nextTick(() => setTimeout(measure, 50)))

const { meshBg, containerStyle, onEnter, onLeave } = useGroundGlass()
</script>

<template>
  <div ref="barRef" class="tab-bar" @mouseenter="onEnter" @mouseleave="onLeave">
    <button
      v-for="item in items"
      :key="item.value"
      :ref="(el) => { if (el) tabRefs[item.value] = (el as HTMLElement) }"
      class="tab-item"
      :class="{
        active: model === item.value,
        'text-hidden': model === item.value && overlayVisible,
      }"
      :disabled="disabled"
      @click="select(item.value)"
    >
      {{ item.label }}
    </button>
    <!-- Sliding indicator -->
    <span class="tab-indicator" :style="{ ...indicatorStyle, ...containerStyle }">
      <span class="tab-indicator__mesh" :style="{ background: meshBg }" />
      <span
        :key="overlayKey"
        class="tab-overlay-text"
        :class="{ visible: overlayVisible }"
      >{{ activeLabel }}</span>
    </span>
  </div>
</template>

<style scoped>
.tab-bar {
  position: relative;
  display: inline-flex;
  align-items: center;
  border-radius: 0.75rem;
  background: transparent;
}

.tab-item {
  position: relative;
  z-index: 1;
  padding: 6px 22px;
  font-size: 13px;
  font-weight: 500;
  color: rgba(176, 190, 201, 0.5);
  text-align: center;
  transition: color 0.25s ease;
  text-decoration: none;
  white-space: nowrap;
  cursor: pointer;
  user-select: none;
  background: transparent;
  border: none;
  outline: none;
}

.tab-item:hover {
  color: rgba(176, 190, 201, 0.8);
}

.tab-item.active {
  color: #e2e8f0;
}

.tab-item.text-hidden {
  color: transparent;
  transition: color 0.5s ease;
}

.tab-indicator {
  position: absolute;
  top: 0;
  height: 100%;
  z-index: 2;
  border-radius: 0.75rem;
  pointer-events: none;
  isolation: isolate;
  overflow: hidden;
  /* Glass slab borders (matches product card) */
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  border-left: 1px solid rgba(255, 255, 255, 0.04);
  border-bottom: 1.5px solid rgba(71, 85, 105, 0.18);
  border-right: 1px solid rgba(71, 85, 105, 0.10);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.04),
    0 1.5px 1px -0.5px rgba(0, 0, 0, 0.18),
    0 3px 8px -3px rgba(0, 0, 0, 0.12);
  transition: left 0.3s cubic-bezier(0.4, 0, 0.2, 1),
              width 0.3s cubic-bezier(0.4, 0, 0.2, 1),
              opacity 0.2s ease;
}

/* Layer 0: animated mesh gradient */
.tab-indicator__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
}

.tab-overlay-text {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 13px;
  font-weight: 500;
  color: #e2e8f0;
  opacity: 0;
  transition: opacity 0.5s ease;
  z-index: 3;
  pointer-events: none;
}

.tab-overlay-text.visible {
  opacity: 1;
}

/* Layer 2: ground glass noise (matches product card) */
.tab-indicator::after {
  content: '';
  position: absolute;
  inset: 0;
  z-index: 2;
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
</style>
