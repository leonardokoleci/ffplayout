<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

import { createFilePreviewUrl, storagePreviewPath } from '@/composables/fileAccess'
import { useConfig } from '@/stores/config'
import { useIndex } from '@/stores/index'

const props = defineProps<{
  src: string
  duration?: number
}>()

const inTime = defineModel<number>('in', { default: 0 })
const outTime = defineModel<number>('out', { default: 0 })
const emit = defineEmits<{ validity: [value: boolean] }>()

const configStore = useConfig()
const indexStore = useIndex()

/* ───────────────────────── constants ───────────────────────── */
const GUTTER = 12 // space left/right of the timeline where the trim handles sit
const RULER_H = 22
const STRIP_H = 44
const WAVE_H = 30
const TILE_W = 78 // one filmstrip tile, ~16:9 at STRIP_H
const THUMB_H = 88 // captured thumbnail height (2x for sharp screens)
const THUMB_CACHE_MAX = 400
const MIN_TRIM = 0.1
const MAX_PPS = 1000 // max zoom: pixels per second
const WAVE_MAX_SEC = 300 // waveform is only built for clips up to 5 min
const WAVE_MAX_BYTES = 32 * 1024 * 1024
const PEAKS_PER_SEC = 100
const SNAPS_DESC = [3600, 1800, 600, 300, 120, 60, 30, 15, 10, 5, 2, 1, 0.5, 0.25, 0.1, 0.05]
const RULER_STEPS = [0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600]

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))
const round3 = (v: number) => Math.round(v * 1000) / 1000

/* ───────────────────────── state ───────────────────────── */
const rootEl = ref<HTMLElement>()
const videoEl = ref<HTMLVideoElement>()
const probeEl = ref<HTMLVideoElement>()
const trackEl = ref<HTMLElement>()
const rulerCanvas = ref<HTMLCanvasElement>()
const bodyCanvas = ref<HTMLCanvasElement>()
const playheadEl = ref<HTMLElement>()
const overviewEl = ref<HTMLElement>()

const previewUrl = ref('')
const loadError = ref('')
const metadataDuration = ref(0)
const isPlaying = ref(false)
const shownTime = ref(0)
const showWave = ref(false)

const trackW = ref(600) // drawable timeline width in px (without gutters)
const pps = ref(0) // pixels per second
const viewStart = ref(0) // time (s) at the left edge of the timeline
const fitMode = ref(true) // true = whole video fits, no zoom

let curT = 0 // playhead time, plain variable on purpose (updated every frame)
let urlRequest = 0

/* ───────────────────────── derived ───────────────────────── */
const fileDuration = computed(() => (metadataDuration.value > 0 ? metadataDuration.value : props.duration || 0))
const effectiveOut = computed(() => (outTime.value > 0 ? outTime.value : fileDuration.value))
const validTrim = computed(() => effectiveOut.value > inTime.value)
const trimLength = computed(() => Math.max(0, effectiveOut.value - inTime.value))
const fitPps = computed(() => (fileDuration.value > 0 ? trackW.value / fileDuration.value : 0))
const viewDur = computed(() => (pps.value > 0 ? trackW.value / pps.value : 0))
const bodyH = computed(() => STRIP_H + (showWave.value ? WAVE_H : 0))

const toX = (t: number) => GUTTER + (t - viewStart.value) * pps.value
const clampX = (x: number) => clamp(x, GUTTER, GUTTER + trackW.value)
const inX = computed(() => toX(inTime.value))
const outX = computed(() => toX(effectiveOut.value))
const inVisible = computed(() => inX.value >= GUTTER - 0.5 && inX.value <= GUTTER + trackW.value + 0.5)
const outVisible = computed(() => outX.value >= GUTTER - 0.5 && outX.value <= GUTTER + trackW.value + 0.5)

const bandStyle = (left: number, right: number) => ({
  left: `${left}px`,
  width: `${Math.max(0, right - left)}px`,
  top: `${RULER_H}px`,
  height: `${bodyH.value}px`,
})
const dimLeftStyle = computed(() => bandStyle(GUTTER, clampX(inX.value)))
const dimRightStyle = computed(() => bandStyle(clampX(outX.value), GUTTER + trackW.value))
const selectionStyle = computed(() => bandStyle(clampX(inX.value), clampX(outX.value)))

const ovLeft = computed(() => (fileDuration.value ? (viewStart.value / fileDuration.value) * 100 : 0))
const ovWidth = computed(() => (fileDuration.value ? Math.min(100, (viewDur.value / fileDuration.value) * 100) : 100))
const ovTrimLeft = computed(() => (fileDuration.value ? (inTime.value / fileDuration.value) * 100 : 0))
const ovTrimWidth = computed(() => (fileDuration.value ? (trimLength.value / fileDuration.value) * 100 : 0))

watch(validTrim, (v) => emit('validity', v), { immediate: true })

watch(fileDuration, (d) => {
  if (d <= 0) return
  if (outTime.value <= 0 || outTime.value > d + 0.001) outTime.value = round3(d)
})

/* ───────────────────────── view / zoom ───────────────────────── */
function clampView() {
  viewStart.value = clamp(viewStart.value, 0, Math.max(0, fileDuration.value - viewDur.value))
}

function applyFit() {
  pps.value = fitPps.value
  viewStart.value = 0
  fitMode.value = true
}

watch(
  fitPps,
  (fit) => {
    if (fit <= 0) return
    if (fitMode.value || pps.value < fit) applyFit()
    else clampView()
  },
  { immediate: true },
)

function zoomAt(factor: number, anchorPx: number) {
  if (fitPps.value <= 0) return
  const anchor = clamp(anchorPx, 0, trackW.value)
  const anchorT = viewStart.value + anchor / pps.value
  const next = clamp(pps.value * factor, fitPps.value, Math.max(fitPps.value, MAX_PPS))
  pps.value = next
  viewStart.value = anchorT - anchor / next
  if (next <= fitPps.value * 1.001) applyFit()
  else {
    fitMode.value = false
    clampView()
  }
}

function zoomButton(factor: number) {
  const px = (curT - viewStart.value) * pps.value
  zoomAt(factor, px >= 0 && px <= trackW.value ? px : trackW.value / 2)
}

function revealTime(t: number) {
  if (fitMode.value) return
  if (t < viewStart.value || t > viewStart.value + viewDur.value) {
    viewStart.value = t - viewDur.value / 2
    clampView()
  }
}

function onWheel(e: WheelEvent) {
  if (fitPps.value <= 0 || !trackEl.value) return
  if (e.ctrlKey || e.metaKey) {
    e.preventDefault()
    const rect = trackEl.value.getBoundingClientRect()
    zoomAt(Math.exp(-e.deltaY * 0.004), e.clientX - rect.left - GUTTER)
  } else if (!fitMode.value) {
    e.preventDefault()
    const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY
    viewStart.value += d / pps.value
    clampView()
  }
}

/* ───────────────────────── canvas drawing ───────────────────────── */
let renderRaf = 0
function invalidate() {
  if (renderRaf) return
  renderRaf = requestAnimationFrame(() => {
    renderRaf = 0
    drawRuler()
    drawBody()
  })
}

function prepCanvas(c: HTMLCanvasElement, w: number, h: number) {
  const dpr = window.devicePixelRatio || 1
  const pw = Math.max(1, Math.round(w * dpr))
  const ph = Math.max(1, Math.round(h * dpr))
  if (c.width !== pw || c.height !== ph) {
    c.width = pw
    c.height = ph
  }
  const ctx = c.getContext('2d')
  if (!ctx) return null
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.clearRect(0, 0, w, h)
  return ctx
}

function fmtTick(t: number, step: number) {
  const dec = step < 0.1 ? 2 : step < 1 ? 1 : 0
  const h = Math.floor(t / 3600)
  const m = Math.floor((t % 3600) / 60)
  const sec = (t % 60).toFixed(dec).padStart(dec ? dec + 3 : 2, '0')
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`
}

function drawRuler() {
  const c = rulerCanvas.value
  if (!c || pps.value <= 0) return
  const w = trackW.value
  const ctx = prepCanvas(c, w, RULER_H)
  if (!ctx) return

  const step = RULER_STEPS.find((s) => s * pps.value >= 70) ?? 3600
  const sub = step / 5
  const t0 = viewStart.value
  const t1 = Math.min(t0 + w / pps.value, fileDuration.value)

  ctx.font = '10px ui-sans-serif, system-ui, sans-serif'
  ctx.textBaseline = 'top'
  for (let i = Math.max(0, Math.floor(t0 / sub)); i * sub <= t1; i++) {
    const t = round3(i * sub)
    const x = Math.round((t - t0) * pps.value) + 0.5
    const major = i % 5 === 0
    ctx.fillStyle = major ? 'rgba(255,255,255,0.5)' : 'rgba(255,255,255,0.15)'
    ctx.fillRect(x - 0.5, RULER_H - (major ? 9 : 5), 1, major ? 9 : 5)
    if (major) {
      ctx.fillStyle = 'rgba(255,255,255,0.55)'
      ctx.fillText(fmtTick(t, step), x + 4, 3)
    }
  }
}

function drawCover(ctx: CanvasRenderingContext2D, th: HTMLCanvasElement, x: number, w: number) {
  const tileAspect = TILE_W / STRIP_H
  let sw = th.width
  let sh = th.height
  if (sw / sh > tileAspect) sw = sh * tileAspect
  else sh = sw / tileAspect
  const f = Math.min(1, w / TILE_W)
  ctx.drawImage(th, (th.width - sw) / 2, (th.height - sh) / 2, sw * f, sh, x, 0, w, STRIP_H)
}

function drawBody() {
  const c = bodyCanvas.value
  if (!c || pps.value <= 0 || fileDuration.value <= 0) return
  const w = trackW.value
  const ctx = prepCanvas(c, w, bodyH.value)
  if (!ctx) return

  const dur = fileDuration.value
  const t0 = viewStart.value
  const viewPx = t0 * pps.value
  const endX = Math.round((dur - t0) * pps.value)
  const tileSec = TILE_W / pps.value
  const snap = SNAPS_DESC.find((s) => s <= tileSec) ?? 0.05
  const maxKey = Math.max(0, Math.floor((dur - 0.05) * 100))

  // Filmstrip: fixed pixel grid anchored to the timeline, only the visible tiles
  const wanted = new Set<number>()
  for (let m = Math.floor(viewPx / TILE_W); m * TILE_W - viewPx < w; m++) {
    const x0 = Math.round(m * TILE_W - viewPx)
    const x1 = Math.min(Math.round((m + 1) * TILE_W - viewPx), endX)
    if (x1 <= x0) continue
    const tc = (m * TILE_W + TILE_W / 2) / pps.value
    const key = clamp(Math.round(Math.round(tc / snap) * snap * 100), 0, maxKey)
    let thumb = thumbCache.get(key)
    if (!thumb) {
      wanted.add(key)
      thumb = nearestThumb(key, tileSec * 200)
    }
    if (thumb) drawCover(ctx, thumb, x0, x1 - x0)
    else {
      ctx.fillStyle = '#1f2937'
      ctx.fillRect(x0, 0, x1 - x0, STRIP_H)
    }
  }

  // Waveform (only for short clips)
  if (showWave.value) {
    const drawW = Math.min(w, endX)
    ctx.fillStyle = 'rgba(255,255,255,0.05)'
    ctx.fillRect(0, STRIP_H, drawW, WAVE_H)
    if (peaks) {
      const mid = STRIP_H + WAVE_H / 2
      const inT = inTime.value
      const outT = effectiveOut.value
      for (let x = 0; x < drawW; x += 3) {
        const ta = t0 + x / pps.value
        const i0 = Math.max(0, Math.floor(ta * PEAKS_PER_SEC))
        const i1 = Math.min(peaks.length, Math.max(i0 + 1, Math.ceil((t0 + (x + 3) / pps.value) * PEAKS_PER_SEC)))
        let p = 0
        for (let i = i0; i < i1; i++) if (peaks[i] > p) p = peaks[i]
        const bh = Math.max(1, p * (WAVE_H - 4))
        ctx.fillStyle = ta >= inT && ta <= outT ? 'rgba(250,204,21,0.8)' : 'rgba(255,255,255,0.15)'
        ctx.fillRect(x, mid - bh / 2, 2, bh)
      }
    }
  }

  const centerKey = Math.round((t0 + w / pps.value / 2) * 100)
  requestThumbs([...wanted].sort((a, b) => Math.abs(a - centerKey) - Math.abs(b - centerKey)))
}

/* ───────────────────────── thumbnails (lazy, visible only) ───────────────────────── */
const thumbCache = new Map<number, HTMLCanvasElement>() // key = time in centiseconds
let thumbQueue: number[] = []
let thumbPumping = false
let thumbGen = 0
let thumbTimer: ReturnType<typeof setTimeout> | undefined

function nearestThumb(key: number, maxDist: number) {
  let best: HTMLCanvasElement | undefined
  let bestD = maxDist
  for (const [k, v] of thumbCache) {
    const d = Math.abs(k - key)
    if (d < bestD) {
      bestD = d
      best = v
    }
  }
  return best
}

function requestThumbs(keys: number[]) {
  thumbQueue = keys.filter((k) => !thumbCache.has(k))
  if (thumbQueue.length && !thumbPumping) kickThumbs()
}

// Wait until the view has settled before touching the probe video
function kickThumbs() {
  clearTimeout(thumbTimer)
  thumbTimer = setTimeout(() => void pumpThumbs(), 140)
}

function seekProbe(t: number) {
  const el = probeEl.value
  return new Promise<boolean>((resolve) => {
    if (!el || el.readyState < 1) return resolve(false)
    let done = false
    const finish = (ok: boolean) => {
      if (done) return
      done = true
      clearTimeout(timer)
      el.removeEventListener('seeked', onSeeked)
      resolve(ok)
    }
    const onSeeked = () => finish(true)
    const timer = setTimeout(() => finish(false), 2500)
    el.addEventListener('seeked', onSeeked)
    const target = clamp(t, 0, Math.max(0, fileDuration.value - 0.05))
    // fastSeek jumps to the nearest keyframe: much cheaper on long videos
    if (typeof el.fastSeek === 'function') el.fastSeek(target)
    else el.currentTime = target
  })
}

async function pumpThumbs() {
  if (thumbPumping) return
  thumbPumping = true
  const gen = thumbGen
  try {
    while (gen === thumbGen) {
      const el = probeEl.value
      // Never compete with playback or scrubbing for the decoder
      if (!el || el.readyState < 1 || isPlaying.value || dragMode) break
      const key = thumbQueue.shift()
      if (key === undefined) break
      if (thumbCache.has(key)) continue

      const ok = await seekProbe(key / 100)
      if (!ok || gen !== thumbGen || !el.videoWidth) continue

      const aspect = el.videoWidth / el.videoHeight || 16 / 9
      const c = document.createElement('canvas')
      c.width = Math.min(320, Math.round(THUMB_H * aspect))
      c.height = THUMB_H
      c.getContext('2d')?.drawImage(el, 0, 0, c.width, c.height)

      thumbCache.set(key, c)
      if (thumbCache.size > THUMB_CACHE_MAX) thumbCache.delete(thumbCache.keys().next().value as number)
      invalidate()
      await new Promise((r) => setTimeout(r, 0))
    }
  } finally {
    thumbPumping = false
  }
}

function onProbeMeta() {
  kickThumbs()
  invalidate()
}

/* ───────────────────────── waveform (short clips only) ───────────────────────── */
let peaks: Float32Array | null = null
let waveAbort: AbortController | undefined

async function loadWave(url: string, request: number) {
  waveAbort?.abort()
  const ac = (waveAbort = new AbortController())
  try {
    const res = await fetch(url, { signal: ac.signal })
    const len = Number(res.headers.get('content-length') || 0)
    if (!res.ok || !len || len > WAVE_MAX_BYTES) throw new Error('skip waveform')

    const audio = await new OfflineAudioContext(1, 1, 44100).decodeAudioData(await res.arrayBuffer())
    if (request !== urlRequest) return

    const data = audio.getChannelData(0)
    const n = Math.max(1, Math.ceil(audio.duration * PEAKS_PER_SEC))
    const stride = Math.max(1, Math.floor(data.length / n))
    const out = new Float32Array(n)
    let max = 0
    for (let i = 0; i < n; i++) {
      let p = 0
      const end = Math.min(data.length, (i + 1) * stride)
      for (let j = i * stride; j < end; j++) {
        const a = Math.abs(data[j])
        if (a > p) p = a
      }
      out[i] = p
      if (p > max) max = p
    }
    if (max > 0) for (let i = 0; i < n; i++) out[i] = Math.pow(out[i] / max, 0.7)
    peaks = out
    invalidate()
  } catch {
    // no audio track, CORS, too big... just drop the waveform row
    if (request === urlRequest) showWave.value = false
  }
}

/* ───────────────────────── playhead + playback ───────────────────────── */
let lastShown = 0
function setShown(t: number, force = false) {
  const now = performance.now()
  if (force || now - lastShown > 90) {
    lastShown = now
    shownTime.value = t
  }
}

function placePlayhead() {
  const el = playheadEl.value
  if (!el) return
  const x = (curT - viewStart.value) * pps.value
  if (x < -1 || x > trackW.value + 1) {
    el.style.display = 'none'
    return
  }
  el.style.display = ''
  el.style.transform = `translateX(${(GUTTER + x).toFixed(2)}px)`
}

// Page-flip follow: redraw once per page instead of every frame
function followPlayhead() {
  if (fitMode.value || dragMode) return
  const x = (curT - viewStart.value) * pps.value
  if (x < 0 || x > trackW.value - 24) {
    viewStart.value = curT - viewDur.value * 0.1
    clampView()
  }
}

let playRaf = 0
function tick() {
  playRaf = 0
  const v = videoEl.value
  if (!v || v.paused) return
  const t = v.currentTime
  if (validTrim.value && t >= effectiveOut.value) {
    v.pause()
    seekTo(inTime.value)
    return
  }
  curT = t
  placePlayhead()
  setShown(t)
  followPlayhead()
  playRaf = requestAnimationFrame(tick)
}

function onPlay() {
  isPlaying.value = true
  cancelAnimationFrame(playRaf)
  playRaf = requestAnimationFrame(tick)
}

function onPause() {
  isPlaying.value = false
  cancelAnimationFrame(playRaf)
  playRaf = 0
  const v = videoEl.value
  if (v && !dragMode && seekTarget === null && !seekBusy) {
    curT = v.currentTime
    placePlayhead()
  }
  setShown(curT, true)
  kickThumbs()
}

function togglePlay() {
  const v = videoEl.value
  if (!v) return
  if (!v.paused) return v.pause()
  if (validTrim.value && (curT < inTime.value || curT >= effectiveOut.value - 0.05)) seekTo(inTime.value)
  v.play().catch(() => {})
}

/* ───────────────────────── seeking (one in flight at a time) ───────────────────────── */
let seekTarget: number | null = null
let seekFast = false
let seekBusy = false
let seekTimer: ReturnType<typeof setTimeout> | undefined
let lastFastT = NaN

function runSeek() {
  const v = videoEl.value
  if (!v || seekTarget === null) return
  const t = seekTarget
  seekTarget = null
  seekBusy = true
  clearTimeout(seekTimer)
  seekTimer = setTimeout(onSeeked, 1000) // safety net so we never get stuck
  if (seekFast && typeof v.fastSeek === 'function') v.fastSeek(t)
  else v.currentTime = t
}

function seekTo(t: number, fast = false) {
  const v = videoEl.value
  const target = clamp(t, 0, fileDuration.value || t)
  if (fast && Math.abs(target - lastFastT) < 1e-4) return
  lastFastT = fast ? target : NaN
  curT = target
  placePlayhead()
  setShown(curT, true)
  if (!v || v.readyState < 1) return
  seekTarget = target
  seekFast = fast
  if (!seekBusy) runSeek()
}

function onSeeked() {
  clearTimeout(seekTimer)
  seekBusy = false
  if (seekTarget !== null) return runSeek()
  const v = videoEl.value
  if (v && !dragMode && v.paused) {
    curT = v.currentTime
    placePlayhead()
    setShown(curT, true)
  }
}

/* ───────────────────────── dragging (handles + scrubbing) ───────────────────────── */
type DragMode = 'in' | 'out' | 'scrub'
let dragMode: DragMode | null = null
let dragX = 0
let dragLeft = 0
let dragRaf = 0

function beginDrag(mode: DragMode, e: PointerEvent, captureEl: HTMLElement) {
  if (!trackEl.value) return
  dragMode = mode
  dragX = e.clientX
  dragLeft = trackEl.value.getBoundingClientRect().left
  captureEl.setPointerCapture(e.pointerId)
  rootEl.value?.focus({ preventScroll: true })
  dragLoop()
}

function dragStep() {
  const W = trackW.value
  const x = dragX - dragLeft - GUTTER

  // auto-pan when dragging near the edge while zoomed
  if (!fitMode.value) {
    const edge = 32
    const push = x < edge ? x - edge : x > W - edge ? x - (W - edge) : 0
    if (push) {
      viewStart.value += (push * 0.3) / pps.value
      clampView()
    }
  }

  const t = clamp(viewStart.value + x / pps.value, 0, fileDuration.value)
  if (dragMode === 'in') {
    const v = round3(clamp(t, 0, effectiveOut.value - MIN_TRIM))
    if (v !== inTime.value) inTime.value = v
    seekTo(v, true)
  } else if (dragMode === 'out') {
    const v = round3(clamp(t, inTime.value + MIN_TRIM, fileDuration.value))
    if (v !== outTime.value) outTime.value = v
    seekTo(v, true)
  } else {
    seekTo(t, true)
  }
}

function dragLoop() {
  if (!dragMode) return
  dragStep()
  dragRaf = requestAnimationFrame(dragLoop)
}

function onTrackDown(e: PointerEvent) {
  if (e.button !== 0 || pps.value <= 0) return
  e.preventDefault()
  beginDrag('scrub', e, trackEl.value!)
}

function onHandleDown(e: PointerEvent, side: 'in' | 'out') {
  e.preventDefault()
  videoEl.value?.pause()
  beginDrag(side, e, e.currentTarget as HTMLElement)
}

function onDragMove(e: PointerEvent) {
  if (dragMode) dragX = e.clientX
}

function onDragEnd() {
  if (!dragMode) return
  dragMode = null
  cancelAnimationFrame(dragRaf)
  seekTo(curT) // final, frame-accurate seek
  kickThumbs()
}

// Overview bar (visible only when zoomed)
let ovDrag = false
function ovCenter(e: PointerEvent) {
  const el = overviewEl.value
  if (!el) return
  const r = el.getBoundingClientRect()
  const f = clamp((e.clientX - r.left) / r.width, 0, 1)
  viewStart.value = f * fileDuration.value - viewDur.value / 2
  clampView()
}
function onOvDown(e: PointerEvent) {
  ovDrag = true
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  ovCenter(e)
}
function onOvMove(e: PointerEvent) {
  if (ovDrag) ovCenter(e)
}
function onOvUp() {
  ovDrag = false
  kickThumbs()
}

/* ───────────────────────── keyboard + helpers ───────────────────────── */
function setIn() {
  inTime.value = round3(clamp(curT, 0, effectiveOut.value - MIN_TRIM))
}
function setOut() {
  outTime.value = round3(clamp(curT, inTime.value + MIN_TRIM, fileDuration.value))
}
function goTo(t: number) {
  videoEl.value?.pause()
  seekTo(t)
  revealTime(t)
}

function onKeydown(e: KeyboardEvent) {
  if (e.ctrlKey || e.metaKey || e.altKey) return
  const onButton = (e.target as HTMLElement | null)?.tagName === 'BUTTON'
  switch (e.key) {
    case ' ':
      if (onButton) return
      e.preventDefault()
      togglePlay()
      break
    case 'i':
    case 'I':
      setIn()
      break
    case 'o':
    case 'O':
      setOut()
      break
    case 'ArrowLeft':
    case 'ArrowRight': {
      e.preventDefault()
      const step = (e.shiftKey ? 1 : 1 / 30) * (e.key === 'ArrowLeft' ? -1 : 1)
      seekTo(curT + step)
      revealTime(curT)
      break
    }
  }
}

function formatTime(s: number) {
  const cs = Math.round(Math.max(0, s || 0) * 100)
  const h = Math.floor(cs / 360000)
  const m = Math.floor(cs / 6000) % 60
  const sec = Math.floor(cs / 100) % 60
  const c = cs % 100
  const mm = String(m).padStart(2, '0')
  const ss = String(sec).padStart(2, '0')
  const cc = String(c).padStart(2, '0')
  return h > 0 ? `${h}:${mm}:${ss}.${cc}` : `${mm}:${ss}.${cc}`
}

/* ───────────────────────── media lifecycle ───────────────────────── */
function resetMedia() {
  previewUrl.value = ''
  loadError.value = ''
  metadataDuration.value = 0
  showWave.value = false
  isPlaying.value = false
  curT = 0
  shownTime.value = 0
  peaks = null
  waveAbort?.abort()
  thumbGen++
  thumbQueue = []
  thumbCache.clear()
  seekTarget = null
  seekBusy = false
  if (fitPps.value > 0) applyFit()
  fitMode.value = true
  placePlayhead()
  invalidate()
}

watch(
  () => props.src,
  async (source) => {
    const request = ++urlRequest
    resetMedia()
    if (!source) return
    try {
      const url = /^https?:\/\//i.test(source)
        ? source
        : await createFilePreviewUrl(configStore.channels[configStore.i]?.id, storagePreviewPath(source))
      if (request === urlRequest) previewUrl.value = url
    } catch (error) {
      if (request === urlRequest) {
        loadError.value = error instanceof Error ? error.message : String(error)
        indexStore.msgAlert('error', loadError.value, 4)
      }
    }
  },
  { immediate: true },
)

function onLoadedMetadata() {
  const v = videoEl.value
  if (!v || !Number.isFinite(v.duration) || v.duration <= 0) return
  metadataDuration.value = v.duration
  if (v.duration <= WAVE_MAX_SEC && previewUrl.value) {
    showWave.value = true
    void loadWave(previewUrl.value, urlRequest)
  }
}

watch([viewStart, pps, trackW, fileDuration, inTime, outTime, showWave], invalidate, { flush: 'post' })
watch([viewStart, pps, trackW], placePlayhead, { flush: 'post' })

let resizeObserver: ResizeObserver | undefined
onMounted(() => {
  if (trackEl.value) {
    resizeObserver = new ResizeObserver(([entry]) => {
      const w = Math.floor(entry.contentRect.width) - GUTTER * 2
      if (w > 20) trackW.value = w
    })
    resizeObserver.observe(trackEl.value)
  }
  placePlayhead()
  invalidate()
})

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  waveAbort?.abort()
  thumbGen++
  thumbCache.clear()
  clearTimeout(thumbTimer)
  clearTimeout(seekTimer)
  cancelAnimationFrame(renderRaf)
  cancelAnimationFrame(playRaf)
  cancelAnimationFrame(dragRaf)
})
</script>

<template>
  <div
    ref="rootEl"
    tabindex="0"
    class="w-full max-w-4xl mx-auto flex flex-col gap-3 text-base-content outline-none"
    @keydown="onKeydown"
  >
    <!-- Preview -->
    <div class="relative w-full h-[28vh] max-h-72 bg-black rounded-xl overflow-hidden shadow-md">
      <video
        v-if="previewUrl"
        ref="videoEl"
        :src="previewUrl"
        preload="metadata"
        playsinline
        class="w-full h-full object-contain cursor-pointer"
        @loadedmetadata="onLoadedMetadata"
        @play="onPlay"
        @pause="onPause"
        @seeked="onSeeked"
        @click="togglePlay"
      />
      <!-- hidden second decoder used only to grab filmstrip frames -->
      <video
        v-if="previewUrl"
        ref="probeEl"
        :src="previewUrl"
        preload="metadata"
        muted
        playsinline
        class="absolute w-px h-px opacity-0 pointer-events-none"
        @loadedmetadata="onProbeMeta"
      />
      <div
        v-if="loadError"
        class="absolute inset-0 flex items-center justify-center p-4 text-center text-sm text-error bg-error/10"
      >
        {{ loadError }}
      </div>
    </div>

    <!-- Transport + trim info -->
    <div class="flex flex-wrap items-center gap-x-3 gap-y-2">
      <button
        type="button"
        class="grid place-items-center w-9 h-9 rounded-full bg-lime-400 text-black hover:bg-lime-300 transition-colors disabled:opacity-30"
        :disabled="!previewUrl"
        :aria-label="isPlaying ? 'Pause' : 'Play'"
        @click="togglePlay"
      >
        <svg v-if="!isPlaying" class="w-4 h-4 fill-current ml-0.5" viewBox="0 0 24 24"><path d="M8 5v14l11-7z" /></svg>
        <svg v-else class="w-4 h-4 fill-current" viewBox="0 0 24 24"><path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z" /></svg>
      </button>

      <div class="font-mono text-sm tabular-nums">
        {{ formatTime(shownTime) }}
        <span class="text-base-content/40">/ {{ formatTime(fileDuration) }}</span>
      </div>

      <div class="flex flex-wrap items-center gap-1.5 ml-auto text-xs">
        <button
          type="button"
          class="h-7 px-2.5 rounded-md bg-white/5 hover:bg-white/10 transition-colors tabular-nums"
          title="Go to start of selection (I sets it)"
          @click="goTo(inTime)"
        >
          <span class="text-base-content/50">Start</span> {{ formatTime(inTime) }}
        </button>
        <button
          type="button"
          class="h-7 px-2.5 rounded-md bg-white/5 hover:bg-white/10 transition-colors tabular-nums"
          title="Go to end of selection (O sets it)"
          @click="goTo(effectiveOut)"
        >
          <span class="text-base-content/50">End</span> {{ formatTime(effectiveOut) }}
        </button>
        <span class="h-7 px-2.5 rounded-md bg-lime-400/15 text-lime-300 grid place-items-center tabular-nums">
          {{ formatTime(trimLength) }}
        </span>

        <span class="w-px h-4 bg-white/10 mx-1" />

        <button
          type="button"
          class="h-7 w-7 rounded-md bg-white/5 hover:bg-white/10 transition-colors disabled:opacity-30"
          :disabled="fitMode"
          aria-label="Zoom out"
          @click="zoomButton(1 / 1.6)"
        >
          −
        </button>
        <button
          type="button"
          class="h-7 px-2 rounded-md bg-white/5 hover:bg-white/10 transition-colors disabled:opacity-30"
          :disabled="fitMode"
          @click="applyFit"
        >
          Fit
        </button>
        <button
          type="button"
          class="h-7 w-7 rounded-md bg-white/5 hover:bg-white/10 transition-colors disabled:opacity-30"
          :disabled="fitPps <= 0"
          aria-label="Zoom in"
          @click="zoomButton(1.6)"
        >
          +
        </button>
      </div>
    </div>

    <!-- Timeline -->
    <div class="rounded-xl bg-neutral shadow-inner select-none pt-1.5 pb-2">
      <div
        ref="trackEl"
        class="relative touch-none cursor-pointer"
        :style="{ height: `${RULER_H + bodyH}px` }"
        @pointerdown="onTrackDown"
        @pointermove="onDragMove"
        @pointerup="onDragEnd"
        @pointercancel="onDragEnd"
        @wheel="onWheel"
      >
        <canvas
          ref="rulerCanvas"
          class="absolute block"
          :style="{ left: `${GUTTER}px`, top: 0, width: `${trackW}px`, height: `${RULER_H}px` }"
        />
        <div
          class="absolute overflow-hidden rounded-md"
          :style="{ left: `${GUTTER}px`, top: `${RULER_H}px`, width: `${trackW}px`, height: `${bodyH}px` }"
        >
          <canvas ref="bodyCanvas" class="block" :style="{ width: `${trackW}px`, height: `${bodyH}px` }" />
        </div>

        <!-- Overlays (clipped to the track so handles never cause overflow) -->
        <div class="absolute inset-0 overflow-hidden pointer-events-none">
          <div class="absolute bg-black/50" :style="dimLeftStyle" />
          <div class="absolute bg-black/50" :style="dimRightStyle" />
          <div class="absolute border-y-2 border-lime-400 box-border" :style="selectionStyle" />

          <div
            v-if="inVisible"
            class="absolute w-3 rounded-l-md bg-lime-400 hover:bg-lime-300 transition-colors cursor-ew-resize pointer-events-auto flex items-center justify-center shadow"
            :style="{ left: `${inX - GUTTER}px`, top: `${RULER_H}px`, height: `${bodyH}px` }"
            @pointerdown.stop="onHandleDown($event, 'in')"
          >
            <div class="absolute -inset-x-2 inset-y-0" />
            <div class="w-0.5 h-4 rounded-full bg-lime-700/50" />
          </div>
          <div
            v-if="outVisible"
            class="absolute w-3 rounded-r-md bg-lime-400 hover:bg-lime-300 transition-colors cursor-ew-resize pointer-events-auto flex items-center justify-center shadow"
            :style="{ left: `${outX}px`, top: `${RULER_H}px`, height: `${bodyH}px` }"
            @pointerdown.stop="onHandleDown($event, 'out')"
          >
            <div class="absolute -inset-x-2 inset-y-0" />
            <div class="w-0.5 h-4 rounded-full bg-lime-700/50" />
          </div>

          <!-- Playhead: moved directly via style.transform from requestAnimationFrame -->
          <div
            ref="playheadEl"
            class="absolute left-0 top-0 bottom-0 w-px bg-white will-change-transform shadow-[0_0_2px_rgba(0,0,0,0.8)]"
          >
            <div class="absolute top-0 -left-[5px] w-[11px] h-[11px] rounded-full bg-white shadow" />
          </div>
        </div>
      </div>

      <!-- Overview: where the visible window sits in the whole video (zoomed only) -->
      <div v-if="!fitMode" class="px-3 pt-2">
        <div
          ref="overviewEl"
          class="relative h-2 rounded-full bg-white/10 cursor-pointer touch-none"
          @pointerdown="onOvDown"
          @pointermove="onOvMove"
          @pointerup="onOvUp"
          @pointercancel="onOvUp"
        >
          <div
            class="absolute inset-y-0 rounded-full bg-lime-400/50"
            :style="{ left: `${ovTrimLeft}%`, width: `${ovTrimWidth}%` }"
          />
          <div
            class="absolute -inset-y-0.5 rounded-full border border-white/70 bg-white/15"
            :style="{ left: `${ovLeft}%`, width: `${ovWidth}%` }"
          />
        </div>
      </div>
    </div>

    <p class="hidden sm:block text-[11px] text-base-content/40">
      Space plays · I and O set the start and end at the playhead · Ctrl+scroll zooms · arrows step frames
    </p>
  </div>
</template>