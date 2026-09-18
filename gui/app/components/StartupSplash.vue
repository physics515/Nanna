<script setup lang="ts">
/**
 * The window's first screen, shown by app.vue until the startup gate releases
 * (see useStartupGate). It never waits on init_backend, which lasts the whole
 * boot: Rust's own setup already runs the init, and this only reads its status.
 */
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { computed, nextTick, onBeforeUpdate, onMounted, onUnmounted, onUpdated, ref, useId, watch } from 'vue'
import { useAppUpdater } from '~/composables/useAppUpdater'
import { useBackend } from '~/composables/useBackend'
import { useCloseHandler } from '~/composables/useCloseHandler'
import { useStartupGate } from '~/composables/useStartupGate'
import { describeSplash, type BootLogLine, type SplashTone } from '~/lib/startupSplash'

/**
 * get_backend_status is an in-process read of the GUI's own state, with no
 * daemon round trip, and nothing else runs in the window during a boot.
 * Half a second is the longest a finished boot should sit behind the splash
 * before the window notices; the shared poll's 2 s would be visible. Reads
 * go through useBackend's poll, one at a time: the read waits on the app
 * state's and the daemon manager's locks, as long as their holders take.
 */
const STATUS_POLL_MS = 500
/**
 * The tail is a bounded in-process buffer. Once a second keeps pace with a
 * person reading it, without re-rendering on every line a busy boot prints.
 */
const LOG_POLL_MS = 1000
/**
 * One line of the tail (12 px text at leading-normal). A reader parked within
 * a line of the bottom is still following it; one who scrolled further up is
 * reading and keeps their place.
 */
const FOLLOW_SLACK_PX = 18

/** The nui live dot, per tone: pulsing while work is under way, still otherwise. */
const DOT_CLASS: Record<SplashTone, string> = {
  working: 'bg-nui-yellow motion-safe:animate-pulse',
  failed: 'bg-nui-pink',
  idle: 'bg-nui-muted',
}

/** Quiet secondary actions: text only, muted until hovered, with a visible keyboard focus. */
const QUIET_BUTTON =
  'rounded-lg px-2 py-1 text-xs leading-normal transition-colors hover:text-nui-fg ' +
  'disabled:pointer-events-none disabled:opacity-50 ' +
  'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-nui-accent'

const { status, refresh, poll } = useBackend()
const { continueOffline } = useStartupGate()
const { updateVersion, updating, checking, applyUpdate } = useAppUpdater()
const { showCloseDialog, handleClose, performQuit } = useCloseHandler()

const restarting = ref(false)
const actionError = ref<string | null>(null)

const view = computed(() =>
  describeSplash(status.value, { restarting: restarting.value, updating: updating.value }),
)

const reason = computed(() =>
  actionError.value ? "Couldn't restart the daemon: " + actionError.value : view.value.reason,
)
// A refused restart is news only until the daemon's state moves on.
watch(() => status.value?.daemon_state, () => { actionError.value = null })

const updateLabel = computed(() =>
  updating.value ? 'Updating…' : 'Update to v' + (updateVersion.value ?? ''),
)

/** A slow boot is a process still starting; a daemon that does not answer is one already running. */
const restartTitle = computed(() =>
  view.value.phase === 'slow' ? 'Stop this boot and start the daemon again' : 'Stop the daemon and start it again',
)

// ═══ Status poll ═══
let statusTimer: ReturnType<typeof setInterval> | null = null

// ═══ Actions ═══
const primaryButton = ref<HTMLButtonElement | null>(null)

/**
 * Restart (or first start) through restart_daemon. It stops a boot that is
 * still in flight, even a hung one, and returns once a fresh init has begun
 * in the background, so progress is read from the status as usual.
 */
async function restartDaemon() {
  if (restarting.value) return
  restarting.value = true
  actionError.value = null
  try {
    await invoke('restart_daemon')
    // Read the status the fresh init produced before dropping "Restarting",
    // so a poll that raced the command cannot flash the state it replaced.
    await refresh()
  } catch (e) {
    actionError.value = e instanceof Error ? e.message : String(e)
  } finally {
    restarting.value = false
  }
}

/**
 * "Open Nanna anyway" mounts the shell now, and the layout's init then starts
 * the daemon if it is not running. Not while an update installs: the updater
 * stopped the daemon on purpose, and that init would start the old one again
 * mid-install.
 */
const canOpenAnyway = computed(() => !updating.value)

function openAnyway() {
  if (canOpenAnyway.value) continueOffline()
}

async function quit() {
  await performQuit()
}

// ═══ Boot log ═══
const logOpen = ref(false)
const logLines = ref<BootLogLine[]>([])
const logUnavailable = ref(false)
const logEl = ref<HTMLElement | null>(null)
const logId = useId()
let logTimer: ReturnType<typeof setInterval> | null = null
/**
 * Whether the view follows the newest line. Only the reader's own scrolling
 * changes it, so a window resize that leaves the view short of the bottom
 * does not count as scrolling away.
 */
let followTail = true

function onLogScroll() {
  const el = logEl.value
  if (el) followTail = el.scrollTop + el.clientHeight >= el.scrollHeight - FOLLOW_SLACK_PX
}

async function readLog() {
  try {
    const lines = await invoke<BootLogLine[]>('get_boot_log')
    logLines.value = Array.isArray(lines) ? lines : []
    logUnavailable.value = false
    if (followTail) {
      await nextTick()
      if (logEl.value) logEl.value.scrollTop = logEl.value.scrollHeight
    }
  } catch {
    // Before the app's state is managed there is no tail to read yet.
    logUnavailable.value = true
  }
}

function stopLogPoll() {
  if (logTimer !== null) {
    clearInterval(logTimer)
    logTimer = null
  }
}

watch(logOpen, (open) => {
  stopLogPoll()
  if (!open) return
  followTail = true
  void readLog()
  logTimer = setInterval(() => { void readLog() }, LOG_POLL_MS)
})

// ═══ Window (frameless: this bar is the title bar until the shell mounts) ═══
const isMaximized = ref(false)
const unlisteners: UnlistenFn[] = []
let unmounted = false

/** Keep a window listener until unmount; one that lands after the gate released is dropped at once. */
function adopt(unlisten: UnlistenFn) {
  if (unmounted) unlisten()
  else unlisteners.push(unlisten)
}

async function minimizeWindow() {
  try { await getCurrentWindow().minimize() } catch (e) { console.error('minimize failed:', e) }
}
async function toggleMaximizeWindow() {
  try { await getCurrentWindow().toggleMaximize() } catch (e) { console.error('maximize failed:', e) }
}

/**
 * Close honours the saved close preference, as it does in the shell:
 * handleClose hides the window to the tray, quits, or opens the CloseDialog
 * mounted below (the layout's is not mounted yet), and does all of it
 * itself. Before the app's state is managed it cannot read the preference,
 * and it quits, so the button always ends in something.
 */
async function closeWindow() {
  await handleClose()
}

/**
 * Esc opens Nanna anyway (not while an update installs; see openAnyway).
 * Heard in the capture phase, ahead of the close dialog's own Esc handling,
 * so a press that closes the dialog is not also read as "open anyway" once
 * the dialog is gone.
 */
function onKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape' || showCloseDialog.value) return
  openAnyway()
}

// ═══ Focus: the action that addresses the state, when there is one ═══
const splashEl = ref<HTMLElement | null>(null)
const statusEl = ref<HTMLElement | null>(null)

async function focusPrimary() {
  await nextTick()
  primaryButton.value?.focus()
}
watch(() => view.value.primary, (primary) => {
  if (primary) void focusPrimary()
})

/**
 * An action the new state removes (Restart, once clicked) takes focus with
 * it to <body>, where a keyboard user loses their place, and "Restarting…"
 * has no action to take it. The status line, which says what happened, gets
 * it then. Checked around every render, since any render can remove the
 * focused element; the next primary action takes focus from there as usual.
 */
let focusWasInside = false
onBeforeUpdate(() => {
  focusWasInside = splashEl.value?.contains(document.activeElement) ?? false
})
onUpdated(() => {
  if (focusWasInside && !splashEl.value?.contains(document.activeElement)) statusEl.value?.focus()
})

onMounted(async () => {
  window.addEventListener('keydown', onKeydown, true)
  void poll()
  statusTimer = setInterval(() => { void poll() }, STATUS_POLL_MS)
  if (view.value.primary) void focusPrimary()

  try {
    const appWindow = getCurrentWindow()
    // The window's own close (Alt+F4, the taskbar) takes the same route as
    // the close button. Prevented, since the JS API would otherwise destroy
    // the window itself, which the capabilities do not grant.
    adopt(await appWindow.onCloseRequested(async (event) => {
      event.preventDefault()
      await closeWindow()
    }))
    isMaximized.value = await appWindow.isMaximized()
    adopt(await appWindow.onResized(async () => {
      try { isMaximized.value = await appWindow.isMaximized() } catch { /* browser dev */ }
    }))
  } catch { /* browser dev — no Tauri window */ }
})

onUnmounted(() => {
  unmounted = true
  window.removeEventListener('keydown', onKeydown, true)
  if (statusTimer !== null) clearInterval(statusTimer)
  stopLogPoll()
  for (const unlisten of unlisteners.splice(0)) unlisten()
})
</script>

<template>
  <!-- Full window, with the shell's 32px radius on the transparent window
       (dropped while maximized: a maximized window has no corners). -->
  <main
    ref="splashEl"
    data-testid="startup-splash"
    class="nui-root relative h-screen w-screen overflow-hidden text-xs leading-normal"
    :class="!isMaximized && 'rounded-[32px]'"
  >
    <div class="absolute inset-x-0 top-0 z-10 flex items-start" data-tauri-drag-region>
      <div class="min-w-0 flex-1 self-stretch" data-tauri-drag-region />
      <NuiWindowControls
        @minimize="minimizeWindow"
        @maximize="toggleMaximizeWindow"
        @close="closeWindow"
      />
    </div>

    <!-- The logo and status line sit in the middle row, exactly where the
         pre-mount frame (spa-loading-template.html) drew them, so nothing
         moves when Vue takes over, and a reason or a detail appearing below
         cannot push them. Opening the log is the one thing that moves them:
         it asks for room, and the lower row takes three quarters. -->
    <div
      class="grid h-full px-8"
      :class="logOpen ? 'grid-rows-[minmax(0,1fr)_auto_minmax(0,3fr)]' : 'grid-rows-[minmax(0,1fr)_auto_minmax(0,1fr)]'"
    >
      <div />

      <div class="flex flex-col items-center gap-4">
        <h1 class="splash-breathe">
          <NuiLogo :height="40" />
        </h1>
        <!-- tabindex="-1": focus lands here when the focused action goes away. -->
        <p
          ref="statusEl"
          role="status"
          aria-live="polite"
          tabindex="-1"
          class="flex items-center gap-2 rounded-lg text-sm leading-normal text-nui-fg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-nui-accent"
        >
          <span
            aria-hidden="true"
            class="h-2 w-2 shrink-0 rounded-full"
            :class="DOT_CLASS[view.tone]"
          />
          <span>{{ view.headline }}<span v-if="view.elapsed" aria-live="off">{{ ' · ' + view.elapsed }}</span></span>
        </p>
      </div>

      <div class="splash-enter flex min-h-0 flex-col items-center gap-4 pt-4">
        <div v-if="reason" role="alert" class="flex max-w-lg flex-col items-center gap-1 text-center">
          <p class="break-words text-xs leading-normal text-nui-fg">{{ reason }}</p>
          <p v-if="view.exit && !actionError" class="text-xs leading-normal text-nui-muted">{{ view.exit }}</p>
        </div>
        <p v-if="view.detail" class="max-w-md text-center text-xs leading-normal text-nui-muted">
          {{ view.detail }}
        </p>

        <button
          v-if="view.primary"
          ref="primaryButton"
          type="button"
          class="flex items-center gap-2 rounded-lg bg-nui-accent px-4 py-2 text-xs leading-normal text-nui-fg transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-nui-fg"
          @click="restartDaemon"
        >
          <NuiIcon name="update" :size="16" />
          {{ view.primary === 'start' ? 'Start the daemon' : 'Restart the daemon' }}
        </button>

        <div class="flex flex-wrap items-center justify-center gap-x-2 gap-y-1">
          <button
            v-if="view.offerRestart"
            type="button"
            :class="[QUIET_BUTTON, 'text-nui-muted']"
            :title="restartTitle"
            :disabled="restarting"
            @click="restartDaemon"
          >
            Restart the daemon
          </button>
          <button
            type="button"
            :class="[QUIET_BUTTON, 'flex items-center gap-2 text-nui-muted']"
            title="Settings and logs work now. Nanna starts the daemon if it isn't running, and chats work once it answers."
            aria-keyshortcuts="Escape"
            :disabled="!canOpenAnyway"
            @click="openAnyway"
          >
            Open Nanna anyway
            <NuiKbd keys="Esc" aria-hidden="true" />
          </button>
          <button
            type="button"
            :class="[QUIET_BUTTON, 'text-nui-muted']"
            :aria-expanded="logOpen"
            :aria-controls="logOpen ? logId : undefined"
            @click="logOpen = !logOpen"
          >
            {{ logOpen ? 'Hide log' : 'Show log' }}
          </button>
          <button
            v-if="updateVersion"
            type="button"
            :class="[QUIET_BUTTON, 'text-nui-info']"
            :title="'Download v' + updateVersion + ' and restart Nanna'"
            :disabled="updating || checking"
            @click="applyUpdate"
          >
            {{ updateLabel }}
          </button>
          <button
            type="button"
            :class="[QUIET_BUTTON, 'text-nui-muted']"
            title="Quit Nanna and stop the daemon"
            @click="quit"
          >
            Quit
          </button>
        </div>

        <div
          v-if="logOpen"
          :id="logId"
          ref="logEl"
          role="log"
          aria-live="off"
          aria-label="Daemon output"
          tabindex="0"
          @scroll="onLogScroll"
          class="nui-scroll mb-8 min-h-0 w-full max-w-3xl overflow-y-auto rounded-lg bg-black/20 px-4 py-2 font-nui text-xs leading-normal focus-visible:outline-2 focus-visible:outline-nui-accent"
        >
          <p v-if="logLines.length === 0" class="text-nui-muted">
            {{ logUnavailable ? "The daemon's output isn't available yet." : 'No output yet.' }}
          </p>
          <p
            v-for="(entry, index) in logLines"
            :key="index"
            class="whitespace-pre-wrap break-words"
            :class="entry.stream === 'stderr' ? 'text-nui-pink' : 'text-nui-fg'"
          >{{ entry.line }}</p>
        </div>
      </div>
    </div>

    <!-- The shell's close dialog lives in its layout, which is not mounted yet. -->
    <CloseDialog />
  </main>
</template>

<style scoped>
/* Motion only for those who have not asked for less. The logo breathes
   slowly while the app waits; the actions settle in once. Neither gates
   anything: the splash leaves by v-if, never at the end of an animation.
   Each starts fully opaque. A window that renders no frames (the
   tauri-webdriver one) holds every animation at its first keyframe, and a
   fade in from 0 left the reason and every action invisible there. */
@media (prefers-reduced-motion: no-preference) {
  .splash-breathe {
    animation: splash-breathe 4s ease-in-out infinite;
  }
  .splash-enter {
    animation: splash-enter 360ms ease-out both;
  }
}

@keyframes splash-breathe {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.8; transform: scale(0.98); }
}

@keyframes splash-enter {
  from { transform: translateY(4px); }
  to { transform: none; }
}
</style>
