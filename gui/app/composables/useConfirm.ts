import { readonly, ref } from 'vue'
import { pushEscapeHandler } from '~/composables/useShortcuts'

export interface ConfirmOptions {
  title: string
  message: string
  confirmLabel?: string
  cancelLabel?: string
  danger?: boolean
}

interface ConfirmState extends ConfirmOptions {
  open: boolean
  resolve: ((value: boolean) => void) | null
}

const state = ref<ConfirmState>({
  open: false,
  title: '',
  message: '',
  confirmLabel: 'Confirm',
  cancelLabel: 'Cancel',
  danger: false,
  resolve: null,
})

let escapeCleanup: (() => void) | null = null

function clearEscape() {
  if (escapeCleanup) {
    escapeCleanup()
    escapeCleanup = null
  }
}

function finish(result: boolean) {
  const resolve = state.value.resolve
  state.value = {
    ...state.value,
    open: false,
    resolve: null,
  }
  clearEscape()
  resolve?.(result)
}

export function useConfirm() {
  function confirm(options: ConfirmOptions): Promise<boolean> {
    // If a previous confirm is still open, cancel it first.
    if (state.value.open && state.value.resolve) {
      state.value.resolve(false)
      clearEscape()
    }

    return new Promise<boolean>((resolve) => {
      state.value = {
        open: true,
        title: options.title,
        message: options.message,
        confirmLabel: options.confirmLabel ?? 'Confirm',
        cancelLabel: options.cancelLabel ?? 'Cancel',
        danger: options.danger ?? false,
        resolve,
      }
      // Topmost Esc dismisses as cancel.
      escapeCleanup = pushEscapeHandler(() => finish(false))
    })
  }

  function handleConfirm() {
    finish(true)
  }

  function handleCancel() {
    finish(false)
  }

  return {
    state: readonly(state),
    confirm,
    handleConfirm,
    handleCancel,
  }
}
