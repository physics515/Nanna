import type { MaybeRefOrGetter } from 'vue'

export interface SplatterConfig {
  /** RGB strings for the 3 radial gradient colors */
  colors?: MaybeRefOrGetter<[string, string, string]>
  /** [min, range] per gradient opacity (default: high-contrast splatter) */
  opacityRanges?: [[number, number], [number, number], [number, number]]
  /** Gradient spread sizes (default: ['70%', '65%', '55%']) */
  sizes?: [string, string, string]
  /** Edge fade opacity (default: 0.01) */
  edgeOpacity?: number
  /** Lerp speed per frame (default: 0.006) */
  lerpSpeed?: number
  /** How often targets shift while active, in ms (default: 2500) */
  interval?: number
}

type SplatterState = {
  x1: number; y1: number; x2: number; y2: number; x3: number; y3: number
  o1: number; o2: number; o3: number
}

const KEYS: (keyof SplatterState)[] = ['x1', 'y1', 'x2', 'y2', 'x3', 'y3', 'o1', 'o2', 'o3']
const rp = () => Math.round(Math.random() * 100)

function lerp(a: number, b: number, t: number) { return a + (b - a) * t }

const DEFAULTS = {
  opacityRanges: [[0.3, 0.7], [0.3, 0.7], [0.1, 0.6]] as [[number, number], [number, number], [number, number]],
  sizes: ['70%', '65%', '55%'] as [string, string, string],
  edgeOpacity: 0.01,
  lerpSpeed: 0.006,
  interval: 2500,
}

export function useSplatter(config?: SplatterConfig) {
  const opacityRanges = config?.opacityRanges ?? DEFAULTS.opacityRanges
  const sizes = config?.sizes ?? DEFAULTS.sizes
  const edgeOpacity = config?.edgeOpacity ?? DEFAULTS.edgeOpacity
  const lerpSpeed = config?.lerpSpeed ?? DEFAULTS.lerpSpeed
  const interval = config?.interval ?? DEFAULTS.interval

  function randomState(): SplatterState {
    return {
      x1: rp(), y1: rp(), x2: rp(), y2: rp(), x3: rp(), y3: rp(),
      o1: opacityRanges[0][0] + Math.random() * opacityRanges[0][1],
      o2: opacityRanges[1][0] + Math.random() * opacityRanges[1][1],
      o3: opacityRanges[2][0] + Math.random() * opacityRanges[2][1],
    }
  }

  // Initial state: two independent random positions so the splatter is mid-motion on mount
  const state = reactive<SplatterState>(randomState())
  const rest = { ...state }
  const target = reactive<SplatterState>(randomState())
  let rafId: number | null = null
  let hovering = false
  let targetTimer: ReturnType<typeof setInterval> | null = null

  // Start animating immediately toward the initial target
  onMounted(() => { startAnim() })

  const splatterBg = computed(() => {
    const c = toValue(config?.colors ?? ['139,92,246', '34,197,94', '251,191,36'] as [string, string, string])
    const edge = (rgb: string) => edgeOpacity > 0 ? `rgba(${rgb},${edgeOpacity})` : 'transparent'

    return [
      `radial-gradient(at ${state.x1}% ${state.y1}%, rgba(${c[0]},${state.o1}), ${edge(c[0])} ${sizes[0]})`,
      `radial-gradient(at ${state.x2}% ${state.y2}%, rgba(${c[1]},${state.o2}), ${edge(c[1])} ${sizes[1]})`,
      `radial-gradient(at ${state.x3}% ${state.y3}%, rgba(${c[2]},${state.o3}), ${edge(c[2])} ${sizes[2]})`,
      `linear-gradient(rgba(${c[0]},${edgeOpacity}), rgba(${c[1]},${edgeOpacity}))`,
    ].join(', ')
  })

  function animate() {
    let settled = true
    for (const k of KEYS) {
      const diff = Math.abs(state[k] - target[k])
      if (diff > 0.1) {
        state[k] = lerp(state[k], target[k], lerpSpeed)
        settled = false
      } else {
        state[k] = target[k]
      }
    }
    if (!settled || hovering) {
      rafId = requestAnimationFrame(animate)
    } else {
      rafId = null
    }
  }

  function startAnim() {
    if (rafId === null) {
      rafId = requestAnimationFrame(animate)
    }
  }

  function newTarget() {
    Object.assign(target, randomState())
    startAnim()
  }

  function onEnter() {
    hovering = true
    newTarget()
    startAnim()
    targetTimer = setInterval(newTarget, interval)
  }

  function onLeave() {
    hovering = false
    if (targetTimer) { clearInterval(targetTimer); targetTimer = null }
    Object.assign(target, rest)
    startAnim()
  }

  onBeforeUnmount(() => {
    if (targetTimer) clearInterval(targetTimer)
    if (rafId !== null) cancelAnimationFrame(rafId)
  })

  return { splatterBg, onEnter, onLeave }
}
