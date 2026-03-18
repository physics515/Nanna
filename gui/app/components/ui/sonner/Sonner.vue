<script setup lang="ts">
import { Toaster as Sonner, type ToasterProps } from 'vue-sonner'
import { computed } from 'vue'

const props = withDefaults(defineProps<ToasterProps & { class?: string }>(), {
  theme: 'dark',
  position: 'bottom-right',
  richColors: true,
  closeButton: true,
  class: '',
})

const toasterProps = computed(() => {
  const { class: _, ...rest } = props
  return rest
})
</script>

<template>
  <Sonner
    v-bind="toasterProps"
    :class="['nanna-sonner', props.class]"
  />
</template>

<style>
/* ═══ Nanna Palenight Sonner Theme ═══ */
.nanna-sonner [data-sonner-toaster] {
  --normal-bg: rgba(15, 23, 42, 0.95);
  --normal-border: rgba(255, 255, 255, 0.06);
  --normal-text: #e2e8f0;
  font-family: 'clother', 'Inter', system-ui, sans-serif;
}

.nanna-sonner [data-sonner-toast] {
  background: rgba(15, 23, 42, 0.95) !important;
  border: 1px solid rgba(255, 255, 255, 0.06) !important;
  color: #e2e8f0 !important;
  backdrop-filter: blur(12px);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
  font-size: 13px;
}

/* Description text */
.nanna-sonner [data-sonner-toast] [data-description] {
  color: #94a3b8 !important;
  font-size: 12px;
}

/* Close button */
.nanna-sonner [data-sonner-toast] [data-close-button] {
  background: rgba(255, 255, 255, 0.06) !important;
  border-color: rgba(255, 255, 255, 0.08) !important;
  color: #94a3b8 !important;
}
.nanna-sonner [data-sonner-toast] [data-close-button]:hover {
  background: rgba(255, 255, 255, 0.1) !important;
  color: #e2e8f0 !important;
}

/* Action button */
.nanna-sonner [data-sonner-toast] [data-button] {
  background: rgba(139, 92, 246, 0.2) !important;
  color: #a78bfa !important;
  border: none !important;
  font-size: 11px;
}
.nanna-sonner [data-sonner-toast] [data-button]:hover {
  background: rgba(139, 92, 246, 0.3) !important;
}

/* Error variant */
.nanna-sonner [data-sonner-toast][data-type="error"] {
  border-color: rgba(251, 113, 133, 0.2) !important;
  background: rgba(15, 23, 42, 0.95) !important;
}
.nanna-sonner [data-sonner-toast][data-type="error"] [data-title] {
  color: #fb7185 !important;
}

/* Warning variant */
.nanna-sonner [data-sonner-toast][data-type="warning"] {
  border-color: rgba(251, 191, 36, 0.2) !important;
  background: rgba(15, 23, 42, 0.95) !important;
}
.nanna-sonner [data-sonner-toast][data-type="warning"] [data-title] {
  color: #fbbf24 !important;
}

/* Success variant */
.nanna-sonner [data-sonner-toast][data-type="success"] {
  border-color: rgba(52, 211, 153, 0.2) !important;
  background: rgba(15, 23, 42, 0.95) !important;
}
.nanna-sonner [data-sonner-toast][data-type="success"] [data-title] {
  color: #34d399 !important;
}

/* Info variant */
.nanna-sonner [data-sonner-toast][data-type="info"] {
  border-color: rgba(34, 211, 238, 0.2) !important;
  background: rgba(15, 23, 42, 0.95) !important;
}
.nanna-sonner [data-sonner-toast][data-type="info"] [data-title] {
  color: #22d3ee !important;
}
</style>
