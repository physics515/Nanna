<script setup lang="ts">
import { cn } from '~/lib/utils'

interface Props {
  class?: string
}

const props = defineProps<Props>()
</script>

<template>
  <div
    :class="cn('rounded-xl bg-[#0f172a]/60 border border-white/[0.06] p-4', props.class)"
  >
    <slot />
  </div>
</template>
