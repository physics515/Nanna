import { ref, readonly } from 'vue'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { getVersion } from '@tauri-apps/api/app'
import { toast } from 'vue-sonner'
import { useNotifications } from '~/composables/useNotifications'

// Six hours between background checks: releases are infrequent and the check
// is a network fetch of the manifest — anything tighter is noise.
const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000
// OS-notify each version only once across app restarts (a balloon on every
// launch until the user updates would train them to ignore it). In-app
// announcements still fire once per session.
const ANNOUNCED_KEY = 'nanna.update.announced'

const currentVersion = ref('')
const updateVersion = ref<string | null>(null)
const checking = ref(false)
const updating = ref(false)
const updateError = ref<string | null>(null)

let pending: Update | null = null
let announcedThisSession: string | null = null
let timer: ReturnType<typeof setInterval> | null = null
let started = false

function announce(version: string) {
  if (announcedThisSession === version) return
  announcedThisSession = version
  toast.info(`Nanna v${version} is available`, {
    description: 'Click Update in the footer to install and restart.',
    duration: 8000,
  })
  let osNotified: string | null = null
  try {
    osNotified = localStorage.getItem(ANNOUNCED_KEY)
  } catch {
    // Storage unavailable: fall through and notify.
  }
  if (osNotified !== version) {
    try {
      localStorage.setItem(ANNOUNCED_KEY, version)
    } catch {
      // Best-effort.
    }
    const { notify } = useNotifications()
    notify({
      title: `Nanna v${version} is available`,
      body: 'Open Nanna and click Update in the footer to install.',
    })
  }
}

/**
 * Check the update endpoint. Manual checks (the footer button) always get
 * feedback; background checks stay silent unless an update is found.
 */
async function checkForUpdates(manual = false) {
  if (checking.value || updating.value) return
  checking.value = true
  try {
    const update = await check()
    if (update) {
      pending = update
      updateVersion.value = update.version
      announce(update.version)
    } else if (manual) {
      toast.success(`You're up to date — v${currentVersion.value} is the latest.`)
    }
  } catch (e) {
    if (manual) {
      toast.error('Update check failed', {
        description: e instanceof Error ? e.message : String(e),
      })
    }
    // Background checks fail silently: offline, endpoint unreachable, or a
    // dev build with no signature — never a scary error for those.
  } finally {
    checking.value = false
  }
}

/**
 * Applies the pending update: downloads, installs, and relaunches the app.
 * Deliberately user-initiated — an always-on assistant must never restart
 * itself out from under a running mission.
 */
async function applyUpdate() {
  if (updating.value || checking.value) return
  if (!pending) {
    // Persistent footer button with no known update: this click IS the
    // check-for-updates flow.
    await checkForUpdates(true)
    return
  }
  updating.value = true
  updateError.value = null
  const version = pending.version
  toast.info(`Downloading Nanna v${version}…`, { duration: 6000 })
  try {
    await pending.downloadAndInstall()
    toast.success(`Nanna v${version} installed — restarting…`)
    await relaunch()
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e)
    updateError.value = message
    updating.value = false
    toast.error('Update failed', {
      description: `${message} — click Update to retry.`,
    })
  }
}

export function useAppUpdater() {
  if (!started) {
    started = true
    getVersion()
      .then(v => { currentVersion.value = v })
      .catch(() => { currentVersion.value = '' })
    checkForUpdates(false)
    timer = setInterval(() => checkForUpdates(false), CHECK_INTERVAL_MS)
    if (import.meta.hot) {
      import.meta.hot.dispose(() => { if (timer) clearInterval(timer) })
    }
  }
  return {
    currentVersion: readonly(currentVersion),
    updateVersion: readonly(updateVersion),
    checking: readonly(checking),
    updating: readonly(updating),
    updateError: readonly(updateError),
    checkForUpdates,
    applyUpdate,
  }
}
