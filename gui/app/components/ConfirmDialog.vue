<template>
  <Teleport to="body">
    <Transition name="confirm">
      <div v-if="state.isOpen" class="fixed inset-0 z-[100] flex items-center justify-center">
        <!-- Backdrop -->
        <div
          class="absolute inset-0 bg-black/60 backdrop-blur-sm"
          @click="handleCancel"
        />

        <!-- Dialog -->
        <div class="relative glass border border-white/[0.06] rounded-xl p-6 w-96 max-w-[90vw] shadow-xl">
          <h3 v-if="state.options.title" class="text-lg font-semibold text-nanna-text mb-2">
            {{ state.options.title }}
          </h3>
          <p class="text-sm text-nanna-text-muted mb-6">
            {{ state.options.message }}
          </p>
          <div class="flex justify-end gap-3">
            <button
              @click="handleCancel"
              class="px-4 py-2 text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-white/[0.06] rounded-lg transition-colors"
            >
              {{ state.options.cancelText || 'Cancel' }}
            </button>
            <button
              @click="handleConfirm"
              :class="[
                'px-4 py-2 text-sm font-medium rounded-lg transition-colors',
                state.options.destructive
                  ? 'bg-nanna-error/20 text-nanna-error border border-nanna-error/30 hover:bg-nanna-error/30'
                  : 'bg-nanna-primary text-white hover:bg-nanna-primary-hover'
              ]"
            >
              {{ state.options.confirmText || 'Confirm' }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { useConfirm } from '~/composables/useConfirm'

const { state, handleConfirm, handleCancel } = useConfirm()
</script>

<style scoped>
.confirm-enter-active,
.confirm-leave-active {
  transition: all 0.2s ease;
}

.confirm-enter-from,
.confirm-leave-to {
  opacity: 0;
}

.confirm-enter-from > div:last-child,
.confirm-leave-to > div:last-child {
  transform: scale(0.95);
}
</style>
