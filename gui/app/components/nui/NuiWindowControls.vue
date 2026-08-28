<script setup lang="ts">
const emit = defineEmits<{
  minimize: []
  maximize: []
  close: []
}>()
</script>

<template>
  <div class="flex items-center">
    <NuiIconButton icon="minimize" label="Minimize" class="p-4" @click="emit('minimize')" />
    <NuiIconButton icon="maximize" label="Maximize" class="p-4" @click="emit('maximize')" />
    <NuiIconButton icon="close" label="Close" class="py-4 pl-4 pr-8 hover:text-nui-pink" @click="emit('close')" />
  </div>
</template>
