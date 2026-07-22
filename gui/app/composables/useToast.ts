import { toast } from 'vue-sonner'

export type ToastKind = 'success' | 'error' | 'info' | 'warning'

/**
 * Thin wrapper over vue-sonner so pages stop rolling ad-hoc toast dismissal.
 * Prefer this for copy/save/delete/clear feedback.
 */
export function useToast() {
  function show(message: string, kind: ToastKind = 'info', description?: string) {
    const opts = description ? { description } : undefined
    switch (kind) {
      case 'success':
        toast.success(message, opts)
        break
      case 'error':
        toast.error(message, opts)
        break
      case 'warning':
        toast.warning(message, opts)
        break
      default:
        toast.info(message, opts)
    }
  }

  function success(message: string, description?: string) {
    show(message, 'success', description)
  }
  function error(message: string, description?: string) {
    show(message, 'error', description)
  }
  function info(message: string, description?: string) {
    show(message, 'info', description)
  }
  function warning(message: string, description?: string) {
    show(message, 'warning', description)
  }

  /** Clipboard helper with success/error toasts. */
  async function copy(text: string, successMsg = 'Copied', errorMsg = 'Copy failed') {
    try {
      await navigator.clipboard.writeText(text)
      success(successMsg)
      return true
    } catch (e) {
      error(errorMsg, e instanceof Error ? e.message : String(e))
      return false
    }
  }

  return { show, success, error, info, warning, copy, toast }
}
