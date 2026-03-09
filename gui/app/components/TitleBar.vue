<script setup lang="ts">
const maximized = ref(false)
let appWindow: any = null

onMounted(async () => {
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  appWindow = getCurrentWindow()
  maximized.value = await appWindow.isMaximized()
})

async function minimize() {
  await appWindow?.minimize()
}

async function toggleMaximize() {
  await appWindow?.toggleMaximize()
  maximized.value = await appWindow?.isMaximized()
}

async function close() {
  await appWindow?.close()
}
</script>

<template>
  <div class="h-8 flex items-center select-none shrink-0" data-tauri-drag-region>
    <div class="flex-1" data-tauri-drag-region></div>

    <!-- Window controls -->
    <button class="titlebar-btn" @click="minimize">
      <svg class="w-[10px] h-[10px]" viewBox="0 0 10 10" fill="currentColor">
        <rect x="1" y="5" width="8" height="1" />
      </svg>
    </button>
    <button class="titlebar-btn" @click="toggleMaximize">
      <svg v-if="!maximized" class="w-[10px] h-[10px]" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1">
        <rect x="1" y="1" width="8" height="8" />
      </svg>
      <svg v-else class="w-[10px] h-[10px]" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1">
        <rect x="2.5" y="0.5" width="7" height="7" />
        <rect x="0.5" y="2.5" width="7" height="7" />
      </svg>
    </button>
    <button class="titlebar-btn titlebar-close" @click="close">
      <svg class="w-[10px] h-[10px]" viewBox="0 0 10 10" stroke="currentColor" stroke-width="1.2">
        <line x1="1" y1="1" x2="9" y2="9" />
        <line x1="9" y1="1" x2="1" y2="9" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.titlebar-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 46px;
  height: 32px;
  color: #94a3b8;
  transition: background-color 0.1s, color 0.1s;
}

.titlebar-btn:hover {
  background-color: rgba(255, 255, 255, 0.06);
  color: #e2e8f0;
}

.titlebar-close:hover {
  background-color: #ef4444;
  color: #ffffff;
}
</style>
