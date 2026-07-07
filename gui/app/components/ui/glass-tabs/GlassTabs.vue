<script setup lang="ts">
import type { Component } from 'vue'
import { useGroundGlass } from '~/composables/useGroundGlass'

export interface GlassTab {
  id: string
  label: string
  icon?: Component
  badge?: string | number
}

export type GlassTabSize = 'xs' | 'sm' | 'md' | 'lg'
export type GlassTabColor = 'default' | 'accent' | 'danger' | 'muted'

const props = withDefaults(defineProps<{
  tabs: GlassTab[]
  size?: GlassTabSize
  /** Color for active tab pill (inactive tabs are always transparent) */
  activeColor?: GlassTabColor
  disabled?: boolean
}>(), {
  size: 'sm',
  activeColor: 'accent',
})

const model = defineModel<string>({ required: true })

// ── Mesh colors per active color ──
const glassConfigs: Record<string, {
  colors: [string, string, string]
  opacityRanges: [[number, number], [number, number], [number, number]]
}> = {
  default: {
    colors: ['0, 0, 0', '0, 0, 0', '0, 0, 0'],
    opacityRanges: [[0, 0], [0, 0], [0, 0]],
  },
  accent: {
    colors: ['139, 92, 246', '167, 139, 250', '139, 92, 246'],
    opacityRanges: [[0, 0.35], [0, 0.25], [0, 0.30]],
  },
  danger: {
    colors: ['239, 68, 68', '220, 38, 38', '248, 113, 113'],
    opacityRanges: [[0, 0.35], [0, 0.25], [0, 0.30]],
  },
  muted: {
    colors: ['100, 116, 139', '71, 85, 105', '100, 116, 139'],
    opacityRanges: [[0, 0.25], [0, 0.20], [0, 0.20]],
  },
}

// ── Sliding indicator: ground glass with animated mesh ──
const gc = glassConfigs[props.activeColor] || glassConfigs.accent
const showMesh = props.activeColor !== 'default'

const { meshBg, containerStyle: glassStyle, onEnter: glassEnter, onLeave: glassLeave } = useGroundGlass({
  colors: gc.colors,
  opacityRanges: gc.opacityRanges,
  opacity: 2.2,
  sizes: ['55%', '50%', '45%'],
  lerpSpeed: 0.008,
  interval: 2000,
  blur: 8,
})

// ── Indicator position tracking ──
const barRef = ref<HTMLElement | null>(null)
const tabRefs = ref<Record<string, HTMLElement | null>>({})
const indicatorStyle = ref<Record<string, string>>({ opacity: '0' })

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

// Overlay text: fade out on switch, fade back in after slide
const overlayVisible = ref(true)
const overlayKey = ref(0)
const activeLabel = computed(() => {
  const item = props.tabs.find(t => t.id === model.value)
  return item?.label ?? ''
})
const activeIcon = computed(() => {
  const item = props.tabs.find(t => t.id === model.value)
  return item?.icon
})
const activeBadge = computed(() => {
  const item = props.tabs.find(t => t.id === model.value)
  return item?.badge
})

function select(id: string) {
  if (props.disabled || id === model.value) return
  model.value = id
}

watch(model, () => {
  overlayVisible.value = false
  overlayKey.value++
  nextTick(measure)
  setTimeout(() => { overlayVisible.value = true }, 320)
})

onMounted(() => nextTick(() => setTimeout(measure, 50)))
</script>

<template>
  <div class="glass-tabs" :class="`glass-tabs--${props.size}`">
    <div
      ref="barRef"
      class="glass-tabs__bar"
      role="tablist"
      @mouseenter="glassEnter"
      @mouseleave="glassLeave"
    >
      <!-- Tab buttons (text only, indicator slides behind) -->
      <button
        v-for="tab in tabs"
        :key="tab.id"
        :ref="(el) => { if (el) tabRefs[tab.id] = (el as HTMLElement) }"
        role="tab"
        :aria-selected="model === tab.id"
        :disabled="disabled"
        class="glass-tabs__item"
        :class="{
          'glass-tabs__item--active': model === tab.id,
          'glass-tabs__item--hidden': model === tab.id && overlayVisible,
        }"
        @click="select(tab.id)"
      >
        <component :is="tab.icon" v-if="tab.icon" class="glass-tabs__icon" />
        {{ tab.label }}
        <span v-if="tab.badge != null" class="glass-tabs__badge">{{ tab.badge }}</span>
      </button>

      <!-- Sliding glass indicator -->
      <span
        class="glass-tabs__indicator"
        :style="{ ...indicatorStyle, ...glassStyle }"
      >
        <!-- Animated mesh (non-default colors only) -->
        <span
          v-if="showMesh"
          class="glass-tabs__indicator-mesh"
          :style="{ background: meshBg }"
        />
        <!-- Noise overlay -->
        <span class="glass-tabs__indicator-noise" />
        <!-- Overlay label (fades in after slide) -->
        <span
          :key="overlayKey"
          class="glass-tabs__overlay"
          :class="{ 'glass-tabs__overlay--visible': overlayVisible }"
        >
          <component :is="activeIcon" v-if="activeIcon" class="glass-tabs__icon" />
          {{ activeLabel }}
          <span v-if="activeBadge != null" class="glass-tabs__badge">{{ activeBadge }}</span>
        </span>
      </span>
    </div>

    <!-- Panels -->
    <div class="glass-tabs__panels">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.glass-tabs__bar {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 0;
}

/* ── Tab items ── */
.glass-tabs__item {
  position: relative;
  z-index: 1;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-weight: 500;
  white-space: nowrap;
  cursor: pointer;
  user-select: none;
  background: transparent;
  border: none;
  outline: none;
  color: rgba(196, 205, 214, 0.5);
  transition: color 0.25s ease;
}

.glass-tabs__item:hover:not(:disabled) {
  color: rgba(196, 205, 214, 0.8);
}

.glass-tabs__item--active {
  color: #e2e8f0;
}

.glass-tabs__item--hidden {
  color: transparent;
  transition: color 0.5s ease;
}

.glass-tabs__item:disabled {
  pointer-events: none;
  opacity: 0.4;
}

/* ── Sizes ── */
.glass-tabs--xs .glass-tabs__item { padding: 4px 10px; font-size: 11px; }
.glass-tabs--sm .glass-tabs__item { padding: 5px 14px; font-size: 12px; }
.glass-tabs--md .glass-tabs__item { padding: 7px 18px; font-size: 13px; }
.glass-tabs--lg .glass-tabs__item { padding: 9px 22px; font-size: 14px; }

/* ── Icon ── */
.glass-tabs__icon {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
}

/* ── Badge ── */
.glass-tabs__badge {
  font-size: 0.7em;
  margin-left: 4px;
  color: rgba(34, 211, 238, 0.8);
}

/* ── Sliding indicator (glass pill) ── */
.glass-tabs__indicator {
  position: absolute;
  top: 0;
  height: 100%;
  z-index: 2;
  pointer-events: none;
  isolation: isolate;
  overflow: hidden;
  border-radius: 9999px;
  background: transparent;
  /* Glass slab borders */
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

.glass-tabs__indicator:hover {
  border-top-color: rgba(255, 255, 255, 0.10);
  border-left-color: rgba(255, 255, 255, 0.07);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.06),
    0 2px 4px -1px rgba(0, 0, 0, 0.20),
    0 4px 12px -4px rgba(0, 0, 0, 0.15);
}

/* Animated mesh layer */
.glass-tabs__indicator-mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
}

/* Noise overlay */
.glass-tabs__indicator-noise {
  position: absolute;
  inset: 0;
  z-index: 2;
  pointer-events: none;
  border-radius: inherit;
  opacity: 0.14;
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

/* Overlay label (fades in after slide) */
.glass-tabs__overlay {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  font-weight: 500;
  color: #e2e8f0;
  opacity: 0;
  transition: opacity 0.5s ease;
  z-index: 3;
  pointer-events: none;
  white-space: nowrap;
}

.glass-tabs__overlay--visible {
  opacity: 1;
}

/* ── Panels ── */
.glass-tabs__panels {
  margin-top: 0;
}
</style>
