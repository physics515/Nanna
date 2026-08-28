<script setup lang="ts">
import type { NuiIconName } from './NuiIcon.vue'

export interface NuiRailItem {
  id: string
  icon: NuiIconName
  label: string
  /** Small pink counter over the icon (e.g. unread notifications). */
  badge?: number
}

const props = defineProps<{
  items: NuiRailItem[]
  /** Items pinned to the bottom of the rail (e.g. settings). */
  bottomItems?: NuiRailItem[]
  activeId?: string
}>()

const emit = defineEmits<{
  select: [id: string]
}>()
</script>

<template>
  <!-- Scrolls (with a hidden scrollbar) rather than clipping: on a short
       window the bottom items must stay reachable, not vanish under the
       status bar. -->
  <nav class="nui-rail flex w-24 shrink-0 flex-col items-center gap-4 overflow-y-auto overflow-x-clip pl-8 pr-4 pt-8 pb-4">
    <div class="pb-4">
      <NuiLogo :height="20" />
    </div>
    <NuiRailButton
      v-for="item in props.items"
      :key="item.id"
      :icon="item.icon"
      :label="item.label"
      :active="item.id === props.activeId"
      :badge="item.badge"
      @click="emit('select', item.id)"
    />
    <div class="flex-1" />
    <NuiRailButton
      v-for="item in props.bottomItems ?? []"
      :key="item.id"
      :icon="item.icon"
      :label="item.label"
      :active="item.id === props.activeId"
      @click="emit('select', item.id)"
    />
  </nav>
</template>

<style scoped>
.nui-rail {
  scrollbar-width: none;
}
.nui-rail::-webkit-scrollbar {
  display: none;
}
</style>
