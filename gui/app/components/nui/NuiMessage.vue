<script setup lang="ts">
import { computed } from 'vue'

/**
 * The message frame from the Figma chat screen: a thick colored edge on the
 * speaker's side, an avatar on the outer edge, and an author label in the
 * header. User messages hug the left gutter (avatar left, blue edge right);
 * assistant messages hug the right gutter (colored edge left, avatar right).
 */
const props = withDefaults(defineProps<{
  role: 'user' | 'assistant'
  /** Edge color. Defaults: user → 'info', assistant → 'pink'; thinking uses 'yellow'. */
  accent?: 'info' | 'pink' | 'yellow' | 'accent' | 'green'
  author?: string
  avatar?: boolean
}>(), {
  avatar: true,
})

const accentClass = computed(() => {
  const accent = props.accent ?? (props.role === 'user' ? 'info' : 'pink')
  return {
    info: 'border-nui-info',
    pink: 'border-nui-pink',
    yellow: 'border-nui-yellow',
    accent: 'border-nui-accent',
    green: 'border-nui-green',
  }[accent]
})

const authorLabel = computed(() => props.author ?? (props.role === 'user' ? 'You' : 'Nanna'))
</script>

<template>
  <div
    class="flex w-full items-start gap-4 py-2"
    :class="props.role === 'user' ? 'pl-8 pr-32' : 'pl-32 pr-8'"
  >
    <NuiAvatar v-if="props.role === 'user' && props.avatar" variant="user" />

    <div
      class="flex min-w-0 flex-1 flex-col items-start gap-4 border-solid py-4"
      :class="[
        accentClass,
        props.role === 'user' ? 'border-r-8 pr-4' : 'border-l-8 pl-4',
      ]"
    >
      <div class="flex w-full items-center gap-2.5">
        <template v-if="props.role === 'user'">
          <p class="whitespace-nowrap text-xs font-semibold leading-normal text-nui-fg">{{ authorLabel }}</p>
          <slot name="header" />
          <span class="min-w-0 flex-1" />
        </template>
        <template v-else>
          <slot name="header" />
          <span class="min-w-0 flex-1" />
          <p class="whitespace-nowrap text-xs font-semibold leading-normal text-nui-fg">{{ authorLabel }}</p>
        </template>
      </div>
      <slot />
    </div>

    <NuiAvatar v-if="props.role === 'assistant' && props.avatar" variant="nanna" />
  </div>
</template>
