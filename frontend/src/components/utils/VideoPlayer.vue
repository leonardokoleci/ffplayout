<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, shallowRef, watch } from 'vue'
import type { HlsJsVideoElement, HlsSource } from '@videojs/html/media/hlsjs-video'
import { Hls } from '@videojs/hlsjs-video'
import { previewLog, previewNumber } from '@/composables/previewDebug'
import '@videojs/html/video/player'
import '@videojs/html/video/neutral-skin'
import '@videojs/html/live-video/player'
import '@videojs/html/live-video/neutral-skin'
import '@videojs/html/media/hlsjs-video'

// Keep Vue's self-component resolution distinct from the <video-player> element.
defineOptions({ name: 'VideoPlayback' })

const props = withDefaults(defineProps<{ src: string; type?: string; live?: boolean }>(), { live: false })
const media = shallowRef<HTMLVideoElement | HlsJsVideoElement>()
const generation = shallowRef(0)
const hls = computed(() => /mpegurl/i.test(props.type ?? '') || /\.m3u8(?:[?#]|$)/i.test(props.src))

// hls.js otherwise starts three segments behind the live edge, so an operator
// jump stays hidden until that stale buffer drains. Keep the preview next to
// the edge and catch up quickly when it drifts.
const liveEngine = {
    liveSyncDurationCount: 1,
    liveMaxLatencyDurationCount: 2,
    maxLiveSyncPlaybackRate: 2,
} satisfies NonNullable<NonNullable<HlsSource['engine']>['hlsJs']>
const hlsSource = computed<HlsSource>(() => ({
    src: props.src,
    type: props.type,
    engine: { hlsJs: liveEngine },
}))
let retry: ReturnType<typeof setTimeout> | undefined
let disposed = false
let resumeAfterRetry = false
let attachedEngine: NonNullable<HlsJsVideoElement['engine']> | null = null
let detachPreview: (() => void) | undefined

type PreviewMedia = HTMLMediaElement | HlsJsVideoElement

function previewElement(value: PreviewMedia | undefined) {
    return value && 'engine' in value ? value : undefined
}

function previewRanges(ranges: TimeRanges) {
    return Array.from({ length: ranges.length }, (_, index) => ({
        start: previewNumber(ranges.start(index)),
        end: previewNumber(ranges.end(index)),
    }))
}

function previewState(element: PreviewMedia) {
    const hlsElement = previewElement(element)
    const engine = hlsElement?.engine ?? null
    const seekable = previewRanges(element.seekable)
    const seekableEnd = seekable.at(-1)?.end ?? null

    return {
        currentTime: previewNumber(element.currentTime),
        paused: element.paused,
        readyState: element.readyState,
        seekable,
        buffered: previewRanges(element.buffered),
        edgeGap: seekableEnd == null ? null : previewNumber(seekableEnd - element.currentTime),
        latency: engine ? previewNumber(engine.latency) : null,
        targetLatency: engine?.targetLatency == null ? null : previewNumber(engine.targetLatency),
        maxLatency: engine ? previewNumber(engine.maxLatency) : null,
        liveSyncPosition: engine?.liveSyncPosition == null ? null : previewNumber(engine.liveSyncPosition),
        liveEdgeStart: hlsElement ? previewNumber(hlsElement.liveEdgeStart) : null,
        targetLiveWindow: hlsElement ? previewNumber(hlsElement.targetLiveWindow) : null,
        streamType: hlsElement?.streamType ?? null,
    }
}

function attachEngineDiagnostics(engine: NonNullable<HlsJsVideoElement['engine']>) {
    previewLog('hls-engine', { version: Hls.version })

    engine.on(Hls.Events.MANIFEST_PARSED, (_event, data) => {
        previewLog('hls-manifest', { levels: data.levels.length })
    })

    engine.on(Hls.Events.LEVEL_LOADED, (_event, data) => {
        const { details } = data
        const last = details.fragments.at(-1)
        const media = engine.media

        previewLog('hls-playlist', {
            level: data.level,
            live: details.live,
            targetDuration: details.targetduration,
            totalDuration: previewNumber(details.totalduration),
            startSN: details.startSN,
            endSN: details.endSN,
            edge: previewNumber(details.edge),
            fragments: details.fragments.length,
            lastStart: last ? previewNumber(last.start) : null,
            lastEnd: last ? previewNumber(last.start + last.duration) : null,
            lastPdt: last?.rawProgramDateTime ?? null,
            ...(media ? previewState(media) : {}),
        })
    })

    engine.on(Hls.Events.FRAG_LOADED, (_event, data) => {
        const stats = (data as { stats?: { loading: { start: number; end: number } } }).stats
        const loadingMs = stats ? previewNumber(stats.loading.end - stats.loading.start) : null

        previewLog('hls-frag-loaded', {
            sn: data.frag.sn,
            start: previewNumber(data.frag.start),
            end: previewNumber(data.frag.start + data.frag.duration),
            duration: previewNumber(data.frag.duration),
            url: data.frag.relurl,
            pdt: data.frag.rawProgramDateTime,
            loadingMs,
        })
    })

    engine.on(Hls.Events.FRAG_CHANGED, (_event, data) => {
        const media = engine.media

        previewLog('hls-frag-changed', {
            sn: data.frag.sn,
            start: previewNumber(data.frag.start),
            end: previewNumber(data.frag.start + data.frag.duration),
            duration: previewNumber(data.frag.duration),
            url: data.frag.relurl,
            pdt: data.frag.rawProgramDateTime,
            ...(media ? previewState(media) : {}),
        })
    })

    engine.on(Hls.Events.ERROR, (_event, data) => {
        previewLog('hls-error', {
            type: data.type,
            details: data.details,
            fatal: data.fatal,
            reason: data.reason,
            error: data.error?.message ?? null,
        })
    })
}

function attachPreviewDiagnostics(element: PreviewMedia | undefined) {
    detachPreview?.()
    detachPreview = undefined
    attachedEngine = null
    if (!element || !props.live || !hls.value) return

    previewLog('media-mounted', {
        src: props.src,
        type: props.type ?? null,
        live: props.live,
        ...previewState(element),
    })

    const domEvents = [
        'loadedmetadata',
        'canplay',
        'playing',
        'waiting',
        'stalled',
        'seeking',
        'seeked',
        'pause',
    ] as const
    const listeners = domEvents.map((name) => {
        const handler = () => previewLog(`media-${name}`, previewState(element))
        element.addEventListener(name, handler)

        return [name, handler] as const
    })

    const syncEngine = () => {
        const engine = previewElement(element)?.engine ?? null
        if (!engine || engine === attachedEngine) return
        attachedEngine = engine
        attachEngineDiagnostics(engine)
    }

    syncEngine()
    const enginePoll = setInterval(syncEngine, 500)
    const snapshot = setInterval(() => previewLog('snapshot', previewState(element)), 1000)

    detachPreview = () => {
        clearInterval(enginePoll)
        clearInterval(snapshot)
        for (const [name, handler] of listeners) element.removeEventListener(name, handler)
    }
}

function restoreVolume() {
    const saved = localStorage.getItem('volume')
    const volume = saved === null ? undefined : Number(saved)
    if (media.value && volume !== undefined && Number.isFinite(volume)) {
        media.value.volume = Math.min(1, Math.max(0, volume))
    }
}

function saveVolume() {
    if (media.value) localStorage.setItem('volume', String(media.value.volume))
}

function retryStream() {
    // File previews keep their error UI; only live streams retry missing media.
    if (!props.live || retry !== undefined) return
    retry = setTimeout(() => {
        retry = undefined
        if (!disposed) {
            resumeAfterRetry = media.value?.paused === false
            generation.value++
        }
    }, 2000)
}

function resumeStream() {
    if (!resumeAfterRetry || !media.value) return
    resumeAfterRetry = false
    void media.value.play().catch(() => {
        // Browser autoplay policy may still require another explicit click.
    })
}

watch(
    media,
    (element) => {
        restoreVolume()
        attachPreviewDiagnostics(element)
    },
    { flush: 'post' },
)
watch(
    () => props.src,
    () => {
        clearTimeout(retry)
        retry = undefined
        resumeAfterRetry = false
    },
)

onMounted(restoreVolume)
onBeforeUnmount(() => {
    disposed = true
    detachPreview?.()
    detachPreview = undefined
    clearTimeout(retry)
    media.value?.pause()
    if (media.value instanceof HTMLVideoElement) {
        media.value.removeAttribute('src')
        media.value.load()
    }
    // Removing the custom element releases the HLS engine and its requests.
})
</script>
<template>
    <component :is="live ? 'live-video-player' : 'video-player'" class="block w-full">
        <component :is="live ? 'live-video-neutral-skin' : 'video-neutral-skin'" class="block w-full aspect-video">
            <hlsjs-video
                v-if="hls"
                :key="`${src}:${generation}`"
                ref="media"
                :source="hlsSource"
                preload="auto"
                playsinline
                @volumechange="saveVolume"
                @error="retryStream"
                @canplay="resumeStream"
            />
            <video v-else :key="src" ref="media" :src="src" preload="auto" playsinline @volumechange="saveVolume" />
        </component>
    </component>
</template>
