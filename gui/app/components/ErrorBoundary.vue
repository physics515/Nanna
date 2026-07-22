<script setup lang="ts">
/**
 * Recoverable error boundary for shell + chat.
 * Vue 3 has no native errorBoundary; we catch via onErrorCaptured and an e2e force hook.
 */
import { onErrorCaptured, onMounted, onUnmounted, ref } from 'vue'

const err = ref<Error | null>(null)
const info = ref<string>('')

onErrorCaptured((error, _instance, errorInfo) => {
  err.value = error instanceof Error ? error : new Error(String(error))
  info.value = errorInfo || ''
  console.error('[ErrorBoundary]', error, errorInfo)
  return false
})

function reset() {
  err.value = null
  info.value = ''
}

function forceError() {
  throw new Error('E2E forced error')
}

function onForceEvent() {
  err.value = new Error('E2E forced error')
  info.value = 'nanna-e2e-force-error'
}

onMounted(() => {
  ;(window as any).__NANNA_FORCE_ERROR__ = () => {
    err.value = new Error('E2E forced error')
    info.value = '__NANNA_FORCE_ERROR__'
  }
  window.addEventListener('nanna-e2e-force-error', onForceEvent)
})

onUnmounted(() => {
  try {
    delete (window as any).__NANNA_FORCE_ERROR__
  } catch {
    /* ignore */
  }
  window.removeEventListener('nanna-e2e-force-error', onForceEvent)
})
</script>

<template>
  <div data-error-boundary data-testid="error-boundary" class="contents">
    <div
      v-if="err"
      role="alert"
      aria-live="assertive"
      class="m-4 rounded-lg border border-rose-500/40 bg-rose-950/40 p-4 text-rose-100"
    >
      <h2 class="text-base font-semibold mb-1">Something went wrong</h2>
      <p class="text-sm opacity-90 mb-3">
        A panel crashed. Your session data is intact — reload this view to continue.
      </p>
      <pre class="text-xs opacity-70 overflow-auto max-h-32 mb-3">{{ err.message }}</pre>
      <div class="flex gap-2">
        <button
          type="button"
          class="px-3 py-1.5 rounded bg-rose-500/20 hover:bg-rose-500/30 border border-rose-400/30 text-sm"
          @click="reset"
        >
          Try again
        </button>
        <button
          type="button"
          class="px-3 py-1.5 rounded bg-white/5 hover:bg-white/10 border border-white/10 text-sm"
          @click="() => location.reload()"
        >
          Reload
        </button>
      </div>
      <button v-show="false" type="button" @click="forceError">force</button>
    </div>
    <slot v-else />
  </div>
</template>
