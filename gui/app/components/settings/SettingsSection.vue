<template>
  <section
    v-if="shouldShow"
    :class="[
      'rounded-xl border bg-nanna-bg-surface/60 p-4 sm:p-5 space-y-3',
      danger
        ? 'border-nanna-error/25'
        : 'border-white/[0.06]',
    ]"
  >
    <header class="space-y-1">
      <h3
        :class="[
          'text-sm font-medium flex items-center gap-2',
          danger ? 'text-nanna-error' : 'text-nanna-text',
        ]"
      >
        <slot name="icon" />
        {{ title }}
      </h3>
      <p v-if="description" class="text-xs text-nanna-text-muted leading-relaxed">
        {{ description }}
      </p>
    </header>
    <div class="space-y-3">
      <slot />
    </div>
  </section>
</template>

<script setup lang="ts">
/**
 * Consistent settings chrome.
 * When `advanced` is true, only renders if the settings page has Show advanced on.
 */
import { computed } from 'vue'
import { useSettingsPage } from '~/composables/useSettingsPage'

const props = withDefaults(
  defineProps<{
    title: string
    description?: string
    danger?: boolean
    /** Hide until "Show advanced settings" is on. */
    advanced?: boolean
  }>(),
  {
    danger: false,
    advanced: false,
  }
)

const store = useSettingsPage()

const shouldShow = computed(() => {
  if (!props.advanced) return true
  return store.showAdvanced.value
})
</script>
