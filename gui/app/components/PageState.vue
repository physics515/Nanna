<template>
  <div class="page-state" :class="toneClass" role="status" :aria-live="state === 'loading' ? 'polite' : 'assertive'">
    <div class="page-state__icon" aria-hidden="true">
      <UiSpinner v-if="state === 'loading'" class="w-8 h-8" />
      <slot v-else name="icon">
        <component :is="defaultIcon" class="w-10 h-10 opacity-70" />
      </slot>
    </div>
    <h3 v-if="title" class="page-state__title">{{ title }}</h3>
    <p v-if="description" class="page-state__desc">{{ description }}</p>
    <div v-if="$slots.actions || primaryAction || secondaryAction" class="page-state__actions">
      <slot name="actions">
        <UiGlassButton
          v-if="primaryAction"
          size="sm"
          color="accent"
          pill
          :disabled="primaryBusy"
          @click="$emit('primary')"
        >
          {{ primaryBusy ? (primaryBusyLabel || 'Working…') : primaryAction }}
        </UiGlassButton>
        <UiGlassButton
          v-if="secondaryAction"
          size="sm"
          color="default"
          pill
          @click="$emit('secondary')"
        >
          {{ secondaryAction }}
        </UiGlassButton>
      </slot>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { AlertCircle, Inbox, WifiOff, Loader2 } from 'lucide-vue-next'

export type PageStateKind = 'loading' | 'empty' | 'error' | 'offline'

const props = withDefaults(defineProps<{
  state: PageStateKind
  title?: string
  description?: string
  primaryAction?: string
  secondaryAction?: string
  primaryBusy?: boolean
  primaryBusyLabel?: string
}>(), {
  title: '',
  description: '',
  primaryAction: '',
  secondaryAction: '',
  primaryBusy: false,
  primaryBusyLabel: '',
})

defineEmits<{
  primary: []
  secondary: []
}>()

const defaultIcon = computed(() => {
  switch (props.state) {
    case 'error': return AlertCircle
    case 'offline': return WifiOff
    case 'loading': return Loader2
    default: return Inbox
  }
})

const toneClass = computed(() => `page-state--${props.state}`)
</script>

<style scoped>
.page-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  gap: 0.5rem;
  min-height: 280px;
  padding: 2rem 1.25rem;
  max-width: 28rem;
  margin-inline: auto;
}
.page-state__icon {
  color: var(--color-nanna-text-muted, #94a3b8);
  margin-bottom: 0.25rem;
}
.page-state--error .page-state__icon { color: var(--color-nanna-error, #f87171); }
.page-state--offline .page-state__icon { color: var(--color-nanna-warning, #fbbf24); }
.page-state__title {
  font-size: 1.05rem;
  font-weight: 600;
  color: var(--color-nanna-text, #e2e8f0);
  margin: 0;
}
.page-state__desc {
  font-size: 0.875rem;
  line-height: 1.45;
  color: var(--color-nanna-text-muted, #94a3b8);
  margin: 0;
}
.page-state__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  justify-content: center;
  margin-top: 0.75rem;
}
</style>
