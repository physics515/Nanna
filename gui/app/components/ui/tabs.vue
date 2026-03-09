<template>
  <div class="w-full">
    <!-- Tab List -->
    <div 
      class="flex border-b border-white/[0.06] overflow-x-auto scrollbar-none"
      role="tablist"
    >
      <button
        v-for="tab in tabs"
        :key="tab.id"
        role="tab"
        :aria-selected="modelValue === tab.id"
        :class="[
          'flex items-center gap-2 px-4 py-2.5 text-sm font-medium whitespace-nowrap transition-all border-b-2 -mb-px',
          modelValue === tab.id
            ? 'text-nanna-primary border-nanna-primary'
            : 'text-nanna-text-muted border-transparent hover:text-nanna-text hover:border-white/[0.08]'
        ]"
        @click="$emit('update:modelValue', tab.id)"
      >
        <component :is="tab.icon" v-if="tab.icon" class="w-4 h-4" />
        {{ tab.label }}
        <UiBadge v-if="tab.badge" variant="accent" class="ml-1 text-xs">
          {{ tab.badge }}
        </UiBadge>
      </button>
    </div>
    
    <!-- Tab Panels -->
    <div class="mt-4">
      <slot />
    </div>
  </div>
</template>

<script setup lang="ts">
import type { Component } from 'vue'

export interface Tab {
  id: string
  label: string
  icon?: Component
  badge?: string | number
}

defineProps<{
  tabs: Tab[]
  modelValue: string
}>()

defineEmits<{
  'update:modelValue': [value: string]
}>()
</script>

<style scoped>
.scrollbar-none::-webkit-scrollbar {
  display: none;
}
.scrollbar-none {
  -ms-overflow-style: none;
  scrollbar-width: none;
}
</style>
