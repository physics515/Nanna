<template>
  <div class="slash-menu" v-if="items.length">
    <button
      v-for="(item, index) in items"
      :key="item.name"
      class="slash-menu-item"
      :class="{ 'slash-menu-item--active': index === selectedIndex }"
      @click="selectItem(index)"
      @mouseenter="selectedIndex = index"
    >
      <span class="slash-menu-item__icon">{{ item.icon }}</span>
      <div class="slash-menu-item__text">
        <span class="slash-menu-item__label">{{ item.label }}</span>
        <span class="slash-menu-item__desc">{{ item.description }}</span>
      </div>
    </button>
  </div>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import type { SlashCommandItem } from '~/extensions/SlashCommands'

const props = defineProps<{
  items: SlashCommandItem[]
  command: (item: SlashCommandItem) => void
}>()

const selectedIndex = ref(0)

watch(() => props.items, () => {
  selectedIndex.value = 0
})

function selectItem(index: number) {
  const item = props.items[index]
  if (item) props.command(item)
}

function onKeyDown(event: KeyboardEvent): boolean {
  if (event.key === 'ArrowUp') {
    selectedIndex.value = (selectedIndex.value + props.items.length - 1) % props.items.length
    return true
  }
  if (event.key === 'ArrowDown') {
    selectedIndex.value = (selectedIndex.value + 1) % props.items.length
    return true
  }
  if (event.key === 'Enter') {
    selectItem(selectedIndex.value)
    return true
  }
  return false
}

defineExpose({ onKeyDown })
</script>

<style>
@reference "../assets/css/main.css";

.slash-menu {
  @apply flex flex-col gap-0.5 p-1.5 rounded-lg min-w-[220px] max-h-[300px] overflow-y-auto;
  background: rgba(15, 23, 42, 0.95);
  border: 1px solid rgba(71, 85, 105, 0.3);
  backdrop-filter: blur(12px);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
}

.slash-menu-item {
  @apply flex items-center gap-2.5 px-2.5 py-2 rounded-md text-left transition-colors cursor-pointer;
  color: rgba(203, 213, 225, 0.85);
}

.slash-menu-item:hover,
.slash-menu-item--active {
  background: rgba(99, 102, 241, 0.15);
  color: #e2e8f0;
}

.slash-menu-item__icon {
  @apply flex items-center justify-center w-7 h-7 rounded text-xs font-bold shrink-0;
  background: rgba(99, 102, 241, 0.1);
  color: rgba(165, 180, 252, 0.9);
}

.slash-menu-item__text {
  @apply flex flex-col;
}

.slash-menu-item__label {
  @apply text-xs font-medium;
}

.slash-menu-item__desc {
  @apply text-[10px];
  color: rgba(148, 163, 184, 0.7);
}

/* Tippy theme override */
.tippy-box[data-theme~='nanna-slash'] {
  background: transparent;
  border: none;
  box-shadow: none;
}

.tippy-box[data-theme~='nanna-slash'] > .tippy-content {
  padding: 0;
}
</style>
