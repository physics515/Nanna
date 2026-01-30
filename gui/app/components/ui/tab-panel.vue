<template>
  <div 
    v-show="active"
    role="tabpanel"
    class="focus:outline-none"
  >
    <slot />
  </div>
</template>

<script setup lang="ts">
defineProps<{
  active: boolean
}>()
</script>
