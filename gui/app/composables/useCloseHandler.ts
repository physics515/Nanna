import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'

export type CloseMode = 'ask' | 'minimize_to_tray' | 'quit_completely'

const showCloseDialog = ref(false)
const closeMode = ref<CloseMode>('ask')
const rememberChoice = ref(false)

export function useCloseHandler() {
  /**
   * Load the saved close mode preference
   */
  async function loadCloseMode(): Promise<CloseMode> {
    try {
      const mode = await invoke<string>('get_close_mode')
      closeMode.value = mode as CloseMode
      return mode as CloseMode
    } catch (e) {
      console.error('Failed to load close mode:', e)
      return 'ask'
    }
  }
  
  /**
   * Save the close mode preference
   */
  async function setCloseMode(mode: CloseMode): Promise<void> {
    try {
      await invoke('set_close_mode', { mode })
      closeMode.value = mode
    } catch (e) {
      console.error('Failed to set close mode:', e)
    }
  }
  
  /**
   * Handle the window close request
   * Returns true if close should proceed, false if handled (minimized or dialog shown)
   */
  async function handleClose(): Promise<boolean> {
    console.log('[useCloseHandler] handleClose called')
    try {
      console.log('[useCloseHandler] Invoking handle_window_close...')
      const action = await invoke<string>('handle_window_close')
      console.log('[useCloseHandler] Backend returned action:', action)
      
      switch (action) {
        case 'ask':
          // Show the close dialog
          console.log('[useCloseHandler] Showing close dialog')
          showCloseDialog.value = true
          console.log('[useCloseHandler] showCloseDialog is now:', showCloseDialog.value)
          return false
          
        case 'minimized':
          // Already minimized to tray
          console.log('[useCloseHandler] Window minimized to tray')
          return false
          
        case 'quit':
          // Proceed with quit
          console.log('[useCloseHandler] Proceeding with quit')
          await performQuit()
          return true
          
        default:
          console.log('[useCloseHandler] Unknown action, returning true')
          return true
      }
    } catch (e) {
      console.error('[useCloseHandler] Failed to handle close:', e)
      return true
    }
  }
  
  /**
   * User chose to minimize to tray
   */
  async function minimizeToTray(): Promise<void> {
    showCloseDialog.value = false
    
    if (rememberChoice.value) {
      await setCloseMode('minimize_to_tray')
    }
    
    // Hide the window
    const window = getCurrentWindow()
    await window.hide()
  }
  
  /**
   * User chose to quit completely
   */
  async function quitCompletely(): Promise<void> {
    showCloseDialog.value = false
    
    if (rememberChoice.value) {
      await setCloseMode('quit_completely')
    }
    
    await performQuit()
  }
  
  /**
   * Perform the actual quit
   */
  async function performQuit(): Promise<void> {
    try {
      await invoke('perform_quit')
    } catch (e) {
      console.error('Failed to perform quit:', e)
      // Force exit if perform_quit fails
      const window = getCurrentWindow()
      await window.destroy()
    }
  }
  
  /**
   * Cancel the close dialog
   */
  function cancelClose(): void {
    showCloseDialog.value = false
    rememberChoice.value = false
  }
  
  return {
    showCloseDialog,
    closeMode: readonly(closeMode),
    rememberChoice,
    loadCloseMode,
    setCloseMode,
    handleClose,
    minimizeToTray,
    quitCompletely,
    cancelClose,
  }
}
