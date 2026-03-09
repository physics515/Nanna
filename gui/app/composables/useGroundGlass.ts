import type { MaybeRefOrGetter } from 'vue'

export interface GroundGlassConfig {
  /** RGB strings for the 3 radial gradient colors (default: purple, green, amber) */
  colors?: MaybeRefOrGetter<[string, string, string]>
  /** [min, range] per gradient opacity — final = min + random * range */
  opacityRanges?: [[number, number], [number, number], [number, number]]
  /** Gradient spread sizes (default: ['50%', '50%', '45%']) */
  sizes?: [string, string, string]
  /** Lerp speed per frame (default: 0.006) */
  lerpSpeed?: number
  /** How often targets shift while active, in ms (default: 2500) */
  interval?: number
  /** Backdrop blur in px (default: 9) */
  blur?: number
  /** Multiplier for mesh gradient opacities (default: 1, higher = more opaque) */
  opacity?: number
}

type MeshState = {
  x1: number; y1: number; x2: number; y2: number; x3: number; y3: number
  o1: number; o2: number; o3: number
}

const MESH_KEYS: (keyof MeshState)[] = ['x1', 'y1', 'x2', 'y2', 'x3', 'y3', 'o1', 'o2', 'o3']
const rp = () => Math.round(Math.random() * 100)

function lerp(a: number, b: number, t: number) { return a + (b - a) * t }

const DEFAULTS = {
  colors: ['139,92,246', '34,197,94', '251,191,36'] as [string, string, string],
  opacityRanges: [[0.20, 0.20], [0.12, 0.15], [0.08, 0.10]] as [[number, number], [number, number], [number, number]],
  sizes: ['50%', '50%', '45%'] as [string, string, string],
  lerpSpeed: 0.006,
  interval: 2500,
  blur: 6,
  opacity: 1,
}

export function useGroundGlass(config?: GroundGlassConfig) {
  const opacityRanges = config?.opacityRanges ?? DEFAULTS.opacityRanges
  const sizes = config?.sizes ?? DEFAULTS.sizes
  const lerpSpeed = config?.lerpSpeed ?? DEFAULTS.lerpSpeed
  const interval = config?.interval ?? DEFAULTS.interval
  const blur = config?.blur ?? DEFAULTS.blur
  const opacity = config?.opacity ?? DEFAULTS.opacity

  function randomMesh(): MeshState {
    return {
      x1: rp(), y1: rp(), x2: rp(), y2: rp(), x3: rp(), y3: rp(),
      o1: opacityRanges[0][0] + Math.random() * opacityRanges[0][1],
      o2: opacityRanges[1][0] + Math.random() * opacityRanges[1][1],
      o3: opacityRanges[2][0] + Math.random() * opacityRanges[2][1],
    }
  }

  // Initial state: two independent random positions so the mesh is mid-motion on mount
  const state = reactive<MeshState>(randomMesh())
  const rest = { ...state }
  const target = reactive<MeshState>(randomMesh())
  let rafId: number | null = null
  let hovering = false
  let targetTimer: ReturnType<typeof setInterval> | null = null

  // Start animating immediately toward the initial target
  onMounted(() => { startAnim() })

  const meshBg = computed(() => {
    const c = toValue(config?.colors ?? DEFAULTS.colors)
    const o1 = Math.min(state.o1 * opacity, 1)
    const o2 = Math.min(state.o2 * opacity, 1)
    const o3 = Math.min(state.o3 * opacity, 1)
    const gradients = [
      `radial-gradient(at ${state.x1}% ${state.y1}%, rgba(${c[0]},${o1}), transparent ${sizes[0]})`,
      `radial-gradient(at ${state.x2}% ${state.y2}%, rgba(${c[1]},${o2}), transparent ${sizes[1]})`,
      `radial-gradient(at ${state.x3}% ${state.y3}%, rgba(${c[2]},${o3}), transparent ${sizes[2]})`,
    ]
    return gradients.join(', ')
  })

  const containerStyle = computed(() => ({
    backdropFilter: `blur(${blur}px)`,
    WebkitBackdropFilter: `blur(${blur}px)`,
  }))

  function animate() {
    let settled = true
    for (const k of MESH_KEYS) {
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
    Object.assign(target, randomMesh())
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

  return { meshBg, containerStyle, onEnter, onLeave }
}
