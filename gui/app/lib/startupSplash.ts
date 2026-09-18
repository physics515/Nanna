/**
 * What the startup splash says while the window waits for its first daemon.
 *
 * Pure, so every state has a unit test and the words live in one place. The
 * voice is backendLabels.ts's: calm and specific, sentence case, "…" for work
 * in progress and " · " for a compound state. A boot is usually a few seconds
 * long, so the ordinary states say little; the words go to the slow and the
 * failed ones, which are the ones a person has to act on.
 */
import { formatElapsed, SLOW_START_S, type BackendStatusLike, type DaemonFailure } from '~/lib/backendLabels'

/** One line of the sidecar's output, as `get_boot_log` returns it. */
export interface BootLogLine {
  stream: 'stdout' | 'stderr'
  line: string
}

export type SplashPhase =
  /** No status yet: the app's own state is not up, so nothing can be said about the daemon. */
  | 'launching'
  /** Our daemon is booting, or an init is about to start it. */
  | 'starting'
  /** Still booting past SLOW_START_S. Alive, but possibly hung. */
  | 'slow'
  /** The daemon is ready and the client is attaching. */
  | 'connecting'
  /** It crashed, and nothing will start it again by itself. */
  | 'failed'
  /** Not running, and no init is going to start it. */
  | 'stopped'
  | 'stopping'
  /** The person asked for a restart and the status has not caught up yet. */
  | 'restarting'
  /** The updater took the daemon down to install; the app relaunches next. */
  | 'updating'
  /** Attached: the gate is releasing. */
  | 'ready'
  /** A daemon_state this file does not know. */
  | 'unknown'

/** The action that addresses the state; offered as the primary button and focused. */
export type SplashAction = 'restart' | 'start'

/**
 * The live dot: 'working' pulses yellow (something is under way), 'failed'
 * is pink, 'idle' is a still, muted dot (nothing is happening, and nothing
 * went wrong either).
 */
export type SplashTone = 'working' | 'failed' | 'idle'

export interface SplashView {
  phase: SplashPhase
  /** The status line's stable words. They change only with the phase, so announcing them is not noise. */
  headline: string
  /** How long the boot has run ("1m 05s"), shown after the headline and never announced. */
  elapsed: string | null
  /** What is happening and what comes next; '' when the headline says enough. */
  detail: string
  /** Why it failed, in the daemon's own words when it printed any. */
  reason: string | null
  /** "exit code 1", "signal 9": how the process ended, when it reported it. */
  exit: string | null
  primary: SplashAction | null
  /**
   * Restart as a quiet secondary action: a boot this slow, or a running
   * daemon that does not answer, may be hung, and only a restart kills it.
   */
  offerRestart: boolean
  tone: SplashTone
}

export interface SplashContext {
  /** A restart_daemon call is in flight, or its result has not been read back yet. */
  restarting?: boolean
  /** The updater is downloading or installing. It stops the daemon on purpose. */
  updating?: boolean
}

/** The client's retry loop is alive: a daemon that answers now is attached without a restart. */
const KEEPS_TRYING = 'Nanna keeps trying and opens as soon as a daemon answers.'

function view(phase: SplashPhase, headline: string, rest: Partial<SplashView> = {}): SplashView {
  return {
    phase,
    headline,
    elapsed: null,
    detail: '',
    reason: null,
    exit: null,
    primary: null,
    offerRestart: false,
    tone: 'working',
    ...rest,
  }
}

/** The failure's headline, named for what the person sees rather than for the Rust enum. */
function failureHeadline(kind: string | undefined): string {
  switch (kind) {
    case 'sidecar_unresolved':
      return "Couldn't find the daemon"
    case 'spawn_failed':
      return "Couldn't start the daemon"
    case 'exited_during_boot':
      return 'The daemon exited while starting'
    case 'exited_after_ready':
      return 'The daemon exited'
    case 'health_check_failed':
      return 'The daemon stopped answering'
    case 'restarts_exhausted':
      return 'The daemon keeps crashing'
    default:
      return 'The daemon crashed'
  }
}

/** "exit code 1", "signal 9", both joined, or null when the process reported neither. */
export function describeExit(failure: DaemonFailure | null | undefined): string | null {
  if (!failure) return null
  const parts: string[] = []
  if (typeof failure.exit_code === 'number') parts.push('exit code ' + failure.exit_code)
  if (typeof failure.signal === 'number') parts.push('signal ' + failure.signal)
  return parts.length > 0 ? parts.join(' · ') : null
}

function failureParts(failure: DaemonFailure | null | undefined): Pick<SplashView, 'reason' | 'exit'> {
  const message = failure?.message?.trim()
  return { reason: message ? message : null, exit: describeExit(failure) }
}

/**
 * Map the polled backend status (null until the app's state is managed) to
 * the splash's words and actions.
 */
export function describeSplash(
  status: BackendStatusLike | null | undefined,
  context: SplashContext = {},
): SplashView {
  // The updater stops the daemon itself before installing. Without this the
  // splash would read that as "isn't running" and offer to start it mid-install.
  if (context.updating) {
    return view('updating', 'Updating Nanna…', {
      detail: 'Nanna restarts by itself once the update is installed.',
    })
  }

  if (!status) return view('launching', 'Starting Nanna…')

  if (status.connected === true) return view('ready', 'Opening Nanna…')

  // Until the command's own answer has been read back, the status still shows
  // the process being stopped, which is the old state and not news.
  if (context.restarting) return view('restarting', 'Restarting the daemon…')

  const state = (status.daemon_state || '').toLowerCase()
  const retrying = status.retrying === true

  if (state === 'starting') {
    const secs = typeof status.starting_for_s === 'number' ? status.starting_for_s : null
    if (secs !== null && secs >= SLOW_START_S) {
      return view('slow', 'Still starting', {
        elapsed: formatElapsed(secs),
        detail:
          'The daemon is alive and still booting. A slow provider, a migration or a cold model load ' +
          'can take minutes. Nanna opens by itself as soon as it answers.',
        offerRestart: true,
      })
    }
    return view('starting', 'Starting the daemon…')
  }

  if (state === 'running') {
    return view('connecting', 'Connecting…', {
      detail: retrying ? 'The daemon is running but has not answered yet. ' + KEEPS_TRYING : '',
      // A retrying client means the daemon is up and not answering, which a
      // hung one does for good; only a restart gets past it. Not while the
      // client is still attaching: that is a moment, not a state.
      offerRestart: retrying,
    })
  }

  if (state.includes('crash')) {
    return view('failed', failureHeadline(status.last_error?.kind), {
      ...failureParts(status.last_error),
      // Before the first connection nothing respawns a daemon that died: the
      // health monitor arms on that connection, and the client's retry loop
      // only reconnects a socket.
      detail: "It won't restart by itself." + (retrying ? ' ' + KEEPS_TRYING : ''),
      primary: 'restart',
      tone: 'failed',
    })
  }

  if (state === 'stopping') return view('stopping', 'Stopping the daemon…')

  // 'not_started' is useBackend's own fallback for an init_backend that threw.
  if (state === 'stopped' || state === 'not_started' || state === '') {
    // Stopped with an init under way is the moment just before the spawn,
    // not a daemon that is down.
    if (status.init_in_progress === true) return view('starting', 'Starting the daemon…')
    return view('stopped', "The daemon isn't running", {
      ...failureParts(status.last_error),
      // Opening anyway runs the layout's init, and that starts a stopped
      // daemon: "open Nanna without it" was not what happens.
      detail: retrying ? KEEPS_TRYING : 'Start it here, or open Nanna anyway, which starts it too. Chats work once it answers.',
      primary: 'start',
      tone: 'idle',
    })
  }

  return view('unknown', 'Waiting for the daemon…', { detail: 'Daemon state: ' + state })
}
