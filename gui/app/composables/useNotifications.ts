import { ref } from 'vue'
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification'

export interface NotificationOptions {
  title: string
  body?: string
  icon?: string
}

const permissionGranted = ref<boolean | null>(null)

export function useNotifications() {
  /**
   * Check and request notification permissions
   */
  async function checkPermission(): Promise<boolean> {
    // Check if permission is already granted
    let granted = await isPermissionGranted()
    
    if (!granted) {
      // Request permission
      const permission = await requestPermission()
      granted = permission === 'granted'
    }
    
    permissionGranted.value = granted
    return granted
  }
  
  /**
   * Send a native notification
   */
  async function notify(options: NotificationOptions): Promise<boolean> {
    try {
      // Check permission first
      if (permissionGranted.value === null) {
        await checkPermission()
      }
      
      if (!permissionGranted.value) {
        console.warn('Notification permission not granted')
        return false
      }
      
      await sendNotification({
        title: options.title,
        body: options.body,
        icon: options.icon,
      })
      
      return true
    } catch (error) {
      console.error('Failed to send notification:', error)
      return false
    }
  }
  
  /**
   * Send a success notification
   */
  async function notifySuccess(title: string, body?: string) {
    return notify({ title: `✅ ${title}`, body })
  }
  
  /**
   * Send an error notification
   */
  async function notifyError(title: string, body?: string) {
    return notify({ title: `❌ ${title}`, body })
  }
  
  /**
   * Send an info notification
   */
  async function notifyInfo(title: string, body?: string) {
    return notify({ title: `ℹ️ ${title}`, body })
  }
  
  /**
   * Notify when a tool completes
   */
  async function notifyToolComplete(toolName: string, success: boolean) {
    if (success) {
      return notify({
        title: '🔧 Tool Complete',
        body: `${toolName} finished successfully`,
      })
    } else {
      return notify({
        title: '⚠️ Tool Failed',
        body: `${toolName} encountered an error`,
      })
    }
  }
  
  /**
   * Notify when a message is received (useful for background)
   */
  async function notifyMessage(preview: string) {
    return notify({
      title: '🌙 Nanna',
      body: preview.substring(0, 100) + (preview.length > 100 ? '...' : ''),
    })
  }
  
  return {
    permissionGranted,
    checkPermission,
    notify,
    notifySuccess,
    notifyError,
    notifyInfo,
    notifyToolComplete,
    notifyMessage,
  }
}
