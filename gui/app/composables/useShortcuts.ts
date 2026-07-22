import { onMounted, onUnmounted } from 'vue'

export type ShortcutHandler = (event: KeyboardEvent) => void | boolean | Promise<void>

export interface ShortcutBinding {
  /** Key from KeyboardEvent.key, e.g. 'k', 'Escape', 'Enter', 'n' */
  key: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
  shift?: boolean
  /** When true, match either Ctrl (Win/Linux) or Meta (macOS). */
  mod?: boolean
  /** Lower = higher priority. Dialogs should register low numbers. */
  priority?: number
  /** Skip when focus is in editable unless allowInInput. */
  allowInInput?: boolean
  description?: string
  handler: ShortcutHandler
}

interface InternalBinding extends ShortcutBinding {
  id: number
}

const bindings: InternalBinding[] = []
let nextId = 1
let listening = false
let escStack: Array<() => void> = []

function isEditableTarget(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false
  const tag = t.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true
  if (t.isContentEditable) return true
  return !!t.closest('[contenteditable="true"]')
}

function matches(b: InternalBinding, e: KeyboardEvent): boolean {
  if (e.key.toLowerCase() !== b.key.toLowerCase() && e.key !== b.key) return false
  const mod = e.ctrlKey || e.metaKey
  if (b.mod) {
    if (!mod) return false
  } else {
    if (!!b.ctrl !== e.ctrlKey) return false
    if (!!b.meta !== e.metaKey) return false
  }
  if (!!b.alt !== e.altKey) return false
  if (!!b.shift !== e.shiftKey) return false
  return true
}

function onKeyDown(e: KeyboardEvent) {
  // Escape stack first (topmost dialog/menu).
  if (e.key === 'Escape' && escStack.length > 0) {
    e.preventDefault()
    const top = escStack[escStack.length - 1]
    top()
    return
  }

  const editable = isEditableTarget(e.target)
  const candidates = bindings
    .filter((b) => matches(b, e))
    .filter((b) => b.allowInInput || !editable)
    .sort((a, b) => (a.priority ?? 100) - (b.priority ?? 100))

  for (const b of candidates) {
    const result = b.handler(e)
    if (result !== false) {
      e.preventDefault()
      break
    }
  }
}

function ensureListen() {
  if (listening || typeof window === 'undefined') return
  window.addEventListener('keydown', onKeyDown, true)
  listening = true
}

export function registerShortcut(binding: ShortcutBinding): () => void {
  ensureListen()
  const item: InternalBinding = { ...binding, id: nextId++ }
  bindings.push(item)
  return () => {
    const i = bindings.findIndex((b) => b.id === item.id)
    if (i >= 0) bindings.splice(i, 1)
  }
}

/** Push a dismisser for Escape; pop on cleanup. Topmost wins. */
export function pushEscapeHandler(fn: () => void): () => void {
  ensureListen()
  escStack.push(fn)
  return () => {
    escStack = escStack.filter((f) => f !== fn)
  }
}

export function listShortcuts(): Array<{ key: string; description: string }> {
  return bindings
    .filter((b) => b.description)
    .map((b) => {
      const parts: string[] = []
      if (b.mod) parts.push('Ctrl/Cmd')
      else {
        if (b.ctrl) parts.push('Ctrl')
        if (b.meta) parts.push('Cmd')
      }
      if (b.alt) parts.push('Alt')
      if (b.shift) parts.push('Shift')
      parts.push(b.key.length === 1 ? b.key.toUpperCase() : b.key)
      return { key: parts.join('+'), description: b.description || '' }
    })
}

/** Component-scoped registration + Escape helpers. */
export function useShortcuts() {
  const cleanups: Array<() => void> = []

  function bind(binding: ShortcutBinding) {
    cleanups.push(registerShortcut(binding))
  }

  function onEscape(fn: () => void) {
    cleanups.push(pushEscapeHandler(fn))
  }

  onMounted(() => ensureListen())
  onUnmounted(() => {
    while (cleanups.length) cleanups.pop()?.()
  })

  return { bind, onEscape, listShortcuts, registerShortcut, pushEscapeHandler }
}
