<script setup lang="ts">
import { useGroundGlass } from '~/composables/useGroundGlass'

export type ButtonVariant = 'solid' | 'outline' | 'ghost' | 'text'
export type ButtonSize = 'xs' | 'sm' | 'md' | 'lg'
export type ButtonColor = 'default' | 'accent' | 'danger' | 'muted'

const props = withDefaults(defineProps<{
  variant?: ButtonVariant
  size?: ButtonSize
  color?: ButtonColor
  pill?: boolean
  disabled?: boolean
  type?: 'button' | 'submit' | 'reset'
  to?: string
  class?: string
}>(), {
  variant: 'solid',
  size: 'sm',
  color: 'default',
  pill: false,
  disabled: false,
  type: 'button',
})

// ── Mesh gradient colors per color prop ──
const meshColors: Record<string, [string, string, string]> = {
  default: ['148, 163, 184', '100, 116, 139', '120, 140, 160'],
  accent:  ['139, 92, 246', '167, 139, 250', '139, 92, 246'],
  danger:  ['239, 68, 68', '220, 38, 38', '248, 113, 113'],
  muted:   ['100, 116, 139', '71, 85, 105', '100, 116, 139'],
}

const hasMesh = computed(() => (props.variant === 'solid' || props.pill) && !props.disabled)

const colors = computed(() => meshColors[props.color] || meshColors.default)

// Splatter for non-pill solid buttons
const { splatterBg, onEnter: splatterEnter, onLeave: splatterLeave } = useSplatter({
  colors,
})

// Ground glass for pill buttons
const { meshBg, containerStyle: glassStyle, onEnter: glassEnter, onLeave: glassLeave } = useGroundGlass({
  colors,
  opacity: 2.2,
  sizes: ['55%', '50%', '45%'],
  lerpSpeed: 0.008,
  interval: 2000,
  blur: 8,
})

const ready = ref(false)
onMounted(() => { setTimeout(() => { ready.value = true }, 200) })

function onEnter() {
  if (!hasMesh.value) return
  if (props.pill) {
    if (ready.value) glassEnter()
  } else {
    splatterEnter()
  }
}
function onLeave() {
  if (props.pill) {
    glassLeave()
  } else {
    splatterLeave()
  }
}
</script>

<template>
  <NuxtLink
    v-if="props.to"
    :to="props.to"
    :class="[
      'ui-btn',
      `ui-btn--${props.variant}`,
      `ui-btn--${props.size}`,
      `ui-btn--${props.color}`,
      { 'ui-btn--disabled': props.disabled, 'ui-btn--pill': props.pill },
      props.class,
    ]"
    :style="props.pill ? glassStyle : undefined"
    @mouseenter="onEnter"
    @mouseleave="onLeave"
  >
    <!-- Splatter mesh (non-pill) -->
    <span
      v-if="hasMesh && !props.pill"
      class="ui-btn__mesh"
      :style="{ background: splatterBg }"
    />
    <!-- Ground glass mesh (pill) -->
    <span
      v-if="hasMesh && props.pill"
      class="ui-btn__glass-mesh"
      :style="{ background: meshBg }"
    />
    <!-- Noise overlay (pill) -->
    <span v-if="props.pill" class="ui-btn__noise" />
    <span class="ui-btn__label"><slot /></span>
  </NuxtLink>
  <button
    v-else
    :type="props.type"
    :disabled="props.disabled || undefined"
    :class="[
      'ui-btn',
      `ui-btn--${props.variant}`,
      `ui-btn--${props.size}`,
      `ui-btn--${props.color}`,
      { 'ui-btn--disabled': props.disabled, 'ui-btn--pill': props.pill },
      props.class,
    ]"
    :style="props.pill ? glassStyle : undefined"
    @mouseenter="onEnter"
    @mouseleave="onLeave"
  >
    <!-- Splatter mesh (non-pill) -->
    <span
      v-if="hasMesh && !props.pill"
      class="ui-btn__mesh"
      :style="{ background: splatterBg }"
    />
    <!-- Ground glass mesh (pill) -->
    <span
      v-if="hasMesh && props.pill"
      class="ui-btn__glass-mesh"
      :style="{ background: meshBg }"
    />
    <!-- Noise overlay (pill) -->
    <span v-if="props.pill" class="ui-btn__noise" />
    <span class="ui-btn__label"><slot /></span>
  </button>
</template>

<style scoped>
/* ════ Base ════ */
.ui-btn {
  position: relative;
  isolation: isolate;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  font-weight: 500;
  white-space: nowrap;
  cursor: pointer;
  outline: none;
  overflow: hidden;
  border: none;
  border-radius: 0;
  text-decoration: none;
  transition: color 0.2s ease, border-color 0.2s ease, background-color 0.2s ease;
}

.ui-btn--disabled {
  pointer-events: none;
  opacity: 0.4;
}

/* ════ Pill shape (ground glass slab) ════ */
.ui-btn--pill {
  border-radius: 9999px;
  background: rgba(30, 41, 59, 0.30);
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  border-left: 1px solid rgba(255, 255, 255, 0.04);
  border-bottom: 1.5px solid rgba(71, 85, 105, 0.18);
  border-right: 1px solid rgba(71, 85, 105, 0.10);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.04),
    0 1.5px 1px -0.5px rgba(0, 0, 0, 0.18),
    0 3px 8px -3px rgba(0, 0, 0, 0.12);
  transition: color 0.2s ease, border-color 0.2s ease, box-shadow 0.2s ease;
}
.ui-btn--pill:hover {
  border-top-color: rgba(255, 255, 255, 0.10);
  border-left-color: rgba(255, 255, 255, 0.07);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.06),
    0 2px 4px -1px rgba(0, 0, 0, 0.20),
    0 4px 12px -4px rgba(0, 0, 0, 0.15);
}
/* Pill sizes: tighter padding, fit content */
.ui-btn--pill.ui-btn--xs { padding: 4px 10px; font-size: 11px; }
.ui-btn--pill.ui-btn--sm { padding: 5px 14px; font-size: 12px; }
.ui-btn--pill.ui-btn--md { padding: 7px 18px; font-size: 13px; }
.ui-btn--pill.ui-btn--lg { padding: 9px 22px; font-size: 14px; height: auto; }

/* Ground glass animated mesh layer */
.ui-btn__glass-mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
}

/* Noise overlay for pill */
.ui-btn__noise {
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

/* Splatter mesh (non-pill solid buttons) */
.ui-btn__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  -webkit-mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch' result='noise'/%3E%3CfeColorMatrix type='saturate' values='0' in='noise' result='gray'/%3E%3CfeColorMatrix type='matrix' in='gray' values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 1 0 0 0 0' result='a'/%3E%3CfeComponentTransfer in='a'%3E%3CfeFuncA type='linear' slope='0.9' intercept='0.05'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch' result='noise'/%3E%3CfeColorMatrix type='saturate' values='0' in='noise' result='gray'/%3E%3CfeColorMatrix type='matrix' in='gray' values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 1 0 0 0 0' result='a'/%3E%3CfeComponentTransfer in='a'%3E%3CfeFuncA type='linear' slope='0.9' intercept='0.05'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  -webkit-mask-size: 200px 200px;
  mask-size: 200px 200px;
}

.ui-btn__label {
  position: relative;
  z-index: 3;
  display: inline-flex;
  align-items: center;
}


/* ════ Sizes (non-pill) ════ */
.ui-btn--xs { padding: 7px 16px; font-size: 11px; }
.ui-btn--sm { padding: 9px 22px; font-size: 12px; }
.ui-btn--md { padding: 12px 28px; font-size: 13px; }
.ui-btn--lg { padding: 14px 34px; font-size: 14px; height: 2.75rem; }

/* ════ Variant: solid (mesh background) ════ */
.ui-btn--solid.ui-btn--default { color: rgba(226, 232, 240, 0.9); background: rgba(71, 85, 105, 0.25); }
.ui-btn--solid.ui-btn--accent  { color: #e2e8f0; background: rgba(139, 92, 246, 0.2); }
.ui-btn--solid.ui-btn--danger  { color: #e2e8f0; background: rgba(239, 68, 68, 0.2); }
.ui-btn--solid.ui-btn--muted   { color: rgba(196, 205, 214, 0.85); background: rgba(71, 85, 105, 0.2); }

/* ════ Variant: outline ════ */
.ui-btn--outline {
  background: transparent;
  border-radius: 0.375rem;
}
.ui-btn--outline.ui-btn--default {
  border: 1px solid rgba(71, 85, 105, 0.4);
  color: #94a3b8;
}
.ui-btn--outline.ui-btn--default:hover {
  border-color: rgba(71, 85, 105, 0.7);
  color: #e2e8f0;
}
.ui-btn--outline.ui-btn--accent {
  border: 1px solid rgba(139, 92, 246, 0.6);
  color: #8b5cf6;
}
.ui-btn--outline.ui-btn--accent:hover {
  background: rgba(139, 92, 246, 0.10);
  color: #a78bfa;
}
.ui-btn--outline.ui-btn--danger {
  border: 1px solid rgba(71, 85, 105, 0.4);
  color: rgba(148, 163, 184, 0.6);
}
.ui-btn--outline.ui-btn--danger:hover {
  border-color: rgba(248, 113, 113, 0.4);
  color: rgba(248, 113, 113, 0.8);
}
.ui-btn--outline.ui-btn--muted {
  border: 1px solid rgba(71, 85, 105, 0.4);
  color: rgba(148, 163, 184, 0.6);
}
.ui-btn--outline.ui-btn--muted:hover {
  border-color: rgba(71, 85, 105, 0.6);
  color: rgba(148, 163, 184, 0.8);
}

/* ════ Variant: ghost (subtle hover) ════ */
.ui-btn--ghost {
  background: transparent;
}
.ui-btn--ghost.ui-btn--default {
  color: rgba(196, 205, 214, 0.5);
}
.ui-btn--ghost.ui-btn--default:hover {
  color: rgba(196, 205, 214, 0.8);
  background: rgba(255, 255, 255, 0.04);
}
.ui-btn--ghost.ui-btn--accent {
  color: rgba(139, 92, 246, 0.7);
}
.ui-btn--ghost.ui-btn--accent:hover {
  color: #a78bfa;
  background: rgba(139, 92, 246, 0.08);
}
.ui-btn--ghost.ui-btn--danger {
  color: rgba(196, 205, 214, 0.5);
}
.ui-btn--ghost.ui-btn--danger:hover {
  color: rgba(248, 113, 113, 0.8);
  background: rgba(239, 68, 68, 0.08);
}
.ui-btn--ghost.ui-btn--muted {
  color: rgba(196, 205, 214, 0.4);
}
.ui-btn--ghost.ui-btn--muted:hover {
  color: rgba(196, 205, 214, 0.7);
}

/* ════ Variant: text (minimal, no background) ════ */
.ui-btn--text {
  background: transparent;
  padding-left: 0;
  padding-right: 0;
}
.ui-btn--text.ui-btn--default {
  color: rgba(196, 205, 214, 0.5);
}
.ui-btn--text.ui-btn--default:hover {
  color: rgba(196, 205, 214, 0.8);
}
.ui-btn--text.ui-btn--accent {
  color: #8b5cf6;
}
.ui-btn--text.ui-btn--accent:hover {
  color: rgba(139, 92, 246, 0.8);
}
.ui-btn--text.ui-btn--danger {
  color: rgba(196, 205, 214, 0.5);
}
.ui-btn--text.ui-btn--danger:hover {
  color: rgba(248, 113, 113, 0.8);
}
.ui-btn--text.ui-btn--muted {
  color: rgba(196, 205, 214, 0.35);
}
.ui-btn--text.ui-btn--muted:hover {
  color: rgba(196, 205, 214, 0.6);
}
</style>
