<template>
  <Teleport to="body">
    <Transition name="confirm-fade">
      <div
        v-if="state.open"
        class="confirm-overlay"
        role="presentation"
        @click.self="handleCancel"
      >
        <div
          class="confirm-dialog"
          role="alertdialog"
          aria-modal="true"
          :aria-labelledby="titleId"
          :aria-describedby="descId"
        >
          <h3 :id="titleId" class="confirm-title">{{ state.title }}</h3>
          <p :id="descId" class="confirm-message">{{ state.message }}</p>
          <div class="confirm-actions">
            <button
              type="button"
              class="confirm-btn confirm-btn--ghost min-h-8"
              @click="handleCancel"
            >
              {{ state.cancelLabel || 'Cancel' }}
            </button>
            <button
              ref="confirmBtnRef"
              type="button"
              class="confirm-btn min-h-8"
              :class="state.danger ? 'confirm-btn--danger' : 'confirm-btn--primary'"
              @click="handleConfirm"
            >
              {{ state.confirmLabel || 'Confirm' }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { nextTick, onUnmounted, ref, watch } from 'vue'
import { useConfirm } from '~/composables/useConfirm'
import { pushEscapeHandler } from '~/composables/useShortcuts'

const titleId = 'nanna-confirm-title'
const descId = 'nanna-confirm-desc'
const confirmBtnRef = ref<HTMLButtonElement | null>(null)

const { state, handleConfirm, handleCancel } = useConfirm()

let popEscape: (() => void) | null = null

watch(
  () => state.open,
  async (open) => {
    if (open) {
      popEscape?.()
      popEscape = pushEscapeHandler(() => handleCancel())
      await nextTick()
      confirmBtnRef.value?.focus()
    } else {
      popEscape?.()
      popEscape = null
    }
  },
)

onUnmounted(() => {
  popEscape?.()
  popEscape = null
})
</script>

<style scoped>
.confirm-overlay {
  position: fixed;
  inset: 0;
  z-index: 10000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 1rem;
  background: rgba(7, 11, 20, 0.62);
  backdrop-filter: blur(6px);
}
.confirm-dialog {
  width: min(420px, 100%);
  border-radius: 0.75rem;
  border: 1px solid rgba(255, 255, 255, 0.08);
  background: rgba(22, 28, 42, 0.96);
  box-shadow: 0 18px 50px rgba(0, 0, 0, 0.45);
  padding: 1.25rem 1.35rem;
}
.confirm-title {
  margin: 0 0 0.5rem;
  font-size: 1.05rem;
  font-weight: 600;
  color: #e2e8f0;
}
.confirm-message {
  margin: 0 0 1.15rem;
  font-size: 0.9rem;
  line-height: 1.45;
  color: #94a3b8;
  white-space: pre-wrap;
}
.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
}
.confirm-btn {
  border-radius: 0.5rem;
  padding: 0.45rem 0.9rem;
  font-size: 0.85rem;
  font-weight: 500;
  border: 1px solid transparent;
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease;
}
.confirm-btn--ghost {
  background: transparent;
  border-color: rgba(148, 163, 184, 0.35);
  color: #cbd5e1;
}
.confirm-btn--ghost:hover {
  background: rgba(148, 163, 184, 0.12);
}
.confirm-btn--primary {
  background: rgba(129, 140, 248, 0.25);
  border-color: rgba(129, 140, 248, 0.5);
  color: #c7d2fe;
}
.confirm-btn--primary:hover {
  background: rgba(129, 140, 248, 0.4);
}
.confirm-btn--danger {
  background: rgba(248, 113, 113, 0.2);
  border-color: rgba(248, 113, 113, 0.45);
  color: #fecaca;
}
.confirm-btn--danger:hover {
  background: rgba(248, 113, 113, 0.35);
}
.confirm-fade-enter-active,
.confirm-fade-leave-active {
  transition: opacity 0.18s ease;
}
.confirm-fade-enter-from,
.confirm-fade-leave-to {
  opacity: 0;
}
</style>
