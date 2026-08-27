<script setup lang="ts">
import type { NuiIconName } from './NuiIcon.vue'

export interface NuiRailItem {
  id: string
  icon: NuiIconName
  label: string
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
  <nav class="flex w-24 shrink-0 flex-col items-center gap-4 overflow-clip pl-8 pr-4 pt-8 pb-4">
    <div class="pb-4">
      <NuiLogo :height="20" />
    </div>
    <NuiRailButton
      v-for="item in props.items"
      :key="item.id"
      :icon="item.icon"
      :label="item.label"
      :active="item.id === props.activeId"
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
