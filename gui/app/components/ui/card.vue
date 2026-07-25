<script setup lang="ts">
import { cn } from '~/lib/utils'

interface Props {
  class?: string
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
