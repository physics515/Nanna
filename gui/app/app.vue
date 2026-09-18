<template>
  <ErrorBoundary>
    <!-- The startup gate (useStartupGate): the shell mounts once a daemon has
         answered, or once the person opens Nanna without one. The splash
         leaves by v-if, never by a leave transition: those wait on
         requestAnimationFrame, which can stall in a hidden or automated window.
         The page key changes once, on the first attach after opening without
         a daemon, so a page that loaded offline loads again. -->
    <NuxtLayout v-if="released">
      <NuxtPage :key="pageEpoch" />
    </NuxtLayout>
    <StartupSplash v-else />
  </ErrorBoundary>
  <ConfirmDialog />
  <UiSonner />
</template>

<script setup lang="ts">
import { watch } from 'vue'
import { useBackend } from '~/composables/useBackend'
import { useStartupGate } from '~/composables/useStartupGate'

const { status } = useBackend()
const { released, pageEpoch, noteConnected } = useStartupGate()

watch(
  () => status.value?.connected === true,
  (connected) => {
    if (connected) noteConnected()
  },
  { immediate: true },
)
</script>

<style>
html, body {
  margin: 0;
  padding: 0;
  overflow: hidden;
  height: 100%;
  background-color: #1e293b;
  color: #e2e8f0;
  font-family: 'clother', 'Inter', system-ui, sans-serif;
}

/* Glass scrollbar */
::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}
::-webkit-scrollbar-track {
  background: transparent;
}
::-webkit-scrollbar-thumb {
  background: rgba(139, 92, 246, 0.18);
  border-radius: 3px;
  border: 1px solid rgba(255, 255, 255, 0.06);
}
::-webkit-scrollbar-thumb:hover {
  background: rgba(139, 92, 246, 0.35);
}
::-webkit-scrollbar-corner {
  background: transparent;
}
</style>
