<script setup lang="ts">
import type { ClassValue } from 'clsx'

import { cn } from '~/lib/utils'

interface Props {
  /**
   * Anything `cn` accepts, not just a string. Callers legitimately pass the
   * object form (`:class="{ 'opacity-50': !enabled }"`), which clsx handles —
   * the `string` type was narrower than the runtime, so correct usage read as
   * a type error.
   */
  class?: ClassValue
}

const props = defineProps<Props>()
</script>

<template>
  <div
    :class="cn('rounded-xl glass-panel p-4', props.class)"
  >
    <slot />
  </div>
</template>
