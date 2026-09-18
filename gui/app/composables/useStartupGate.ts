import { computed, readonly, ref } from 'vue'

/**
 * The startup gate: app.vue shows the splash until a daemon has answered once,
 * or until the person chose to open Nanna without one, and only then mounts
 * the shell.
 *
 * Every page loads its data once, on mount, and none of them watches the
 * connection. Mounting them after the first attach is what lets that single
 * load succeed, instead of each page reading "Daemon offline" through a boot.
 *
 * The latch is per process and one-way. Later disconnects (a crash, the health
 * monitor's restart, the updater stopping the daemon to install) belong to the
 * footer, the chat banner and the layout's attach watcher. A splash that came
 * back would hide the shell and its Update and Logs with it.
 */
const everConnected = ref(false)
const userContinued = ref(false)
/** Released by the person while nothing was connected, so the pages mounted offline. */
const releasedOffline = ref(false)
/** Bumped once, on the first attach after an offline release, to remount the page. */
const pageEpoch = ref(0)

export function useStartupGate() {
  const released = computed(() => everConnected.value || userContinued.value)

  /**
   * Record an attach. Only the first one matters, and it never re-arms the gate.
   *
   * `page.worksOffline` is the route's own `definePageMeta({ worksOffline: true })`:
   * the page on screen works without a daemon and holds what the person
   * typed into it (Settings' fields, the Logs filters).
   */
  function noteConnected(page: { worksOffline?: boolean } = {}) {
    if (everConnected.value) return
    everConnected.value = true
    // The pages that mounted offline fetched once, got nothing, and have no
    // reason to ask again. Remount the page so it does its load now. Only
    // this once: after a later reconnect the page already holds real data.
    // A page that works offline is kept: a remount would throw away its
    // unsaved input, and a page opened after this mounts fresh anyway.
    if (releasedOffline.value && !page.worksOffline) pageEpoch.value += 1
  }

  /** "Open Nanna anyway": the shell mounts without a daemon. */
  function continueOffline() {
    if (released.value) return
    userContinued.value = true
    releasedOffline.value = true
  }

  return {
    released,
    releasedOffline: readonly(releasedOffline),
    pageEpoch: readonly(pageEpoch),
    noteConnected,
    continueOffline,
  }
}
