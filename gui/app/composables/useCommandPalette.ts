import { ref } from 'vue'

const open = ref(false)

/** Module-level singleton so layout + shortcuts share palette state. */
export function useCommandPalette() {
  function toggle() {
    open.value = !open.value
  }
  function show() {
    open.value = true
  }
  function hide() {
    open.value = false
  }
  return { open, toggle, show, hide }
}
