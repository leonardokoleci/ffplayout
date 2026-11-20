<script setup lang="ts">
import dayjs from 'dayjs'
import customParseFormat from 'dayjs/plugin/customParseFormat.js'
import LocalizedFormat from 'dayjs/plugin/localizedFormat.js'
import timezone from 'dayjs/plugin/timezone.js'
import utc from 'dayjs/plugin/utc.js'
import 'dayjs/locale/de'
import 'dayjs/locale/en'
import 'dayjs/locale/es'
import 'dayjs/locale/pt-br'
import 'dayjs/locale/ru'

import VirtualList from 'vue-virtual-sortable'

import { computed, ref, nextTick, onBeforeUnmount, onMounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useWindowSize } from '@vueuse/core'
import { storeToRefs } from 'pinia'

dayjs.extend(customParseFormat)
dayjs.extend(LocalizedFormat)
dayjs.extend(timezone)
dayjs.extend(utc)

import { stringFormatter, playlistOperations } from '@/composables/helper'
import { scheduledRowIndex } from '@/composables/livePlaylistSchedule'
import { flashRows } from '@/composables/rowFlash'
import {
    MEDIA_DRAG_TYPE,
    PLAYLIST_ROW_DRAG_TYPE,
    multiDragGhost,
    nativeDropIndex,
    stopDragScroll,
    updateDragScroll,
} from '@/composables/multiDragGhost'
import { authFetch } from '@/composables/authFetch'
import { useConfig } from '@/stores/config'
import { useMedia } from '@/stores/media'
import { usePlaylist } from '@/stores/playlist'

const { t } = useI18n()
const { width } = useWindowSize({ initialWidth: 800 })
const configStore = useConfig()
const mediaStore = useMedia()
const playlistStore = usePlaylist()
const { secToHMS, filename, secondsToTime } = stringFormatter()
const { processPlaylist, genUID } = playlistOperations()

const playlistContainer = ref()
const sortContainer = ref()
const todayDate = ref(dayjs().tz(configStore.timezone).format('YYYY-MM-DD'))
const { i } = storeToRefs(useConfig())
const { currentIndex, listDate, playoutIsRunning, scrollToItem } = storeToRefs(usePlaylist())

const scrollPending = ref(false)
const activeRowIndex = computed(() => {
    if (!playoutIsRunning.value) return -1
    if (playlistStore.ingestRuns && configStore.playoutSaved.processing.mode === 'playlist') {
        return scheduledRowIndex(
            playlistStore.liveScheduledClip,
            configStore.channels[configStore.i]?.id,
            listDate.value,
            playlistStore.playlist,
        )
    }
    return listDate.value === todayDate.value ? currentIndex.value : -1
})

// The schedule and playlist arrive independently. Scroll once both identify the row,
// then leave manual scrolling alone as the countdown advances to subsequent clips.
watch(
    [activeRowIndex, () => playlistStore.isLoading, sortContainer, scrollPending],
    async () => {
        if (!scrollPending.value || playlistStore.isLoading || activeRowIndex.value < 0) return
        await nextTick()
        if (!scrollPending.value || playlistStore.isLoading || activeRowIndex.value < 0 || !sortContainer.value) return
        scrollTo(activeRowIndex.value)
        scrollPending.value = false
    },
    { flush: 'post' },
)

// Native HTML5 drag state for the insertion indicator. The hint class is
// set directly on the row element: routing it through a reactive ref would
// re-render every visible row on each dragover event and stutter the drag.
let hintRow: HTMLElement | null = null
let hintAbove = false

function clearDropHint() {
    hintRow?.classList.remove('playlist-drop-above', 'playlist-drop-below')
    hintRow = null
}

defineProps({
    editItem: {
        type: Function,
        default() {
            return ''
        },
    },
    preview: {
        type: Function,
        default() {
            return ''
        },
    },
})

onMounted(() => {
    setTimeout(() => {
        getPlaylist()
    }, 150)
})

watch([listDate, i], () => {
    playlistStore.selectedUids = []
    playlistStore.clipboardItems = []
    playlistStore.clipboardMode = ''
    setTimeout(() => {
        getPlaylist()
    }, 800)
})

watch([playoutIsRunning, scrollToItem], () => {
    if (playoutIsRunning.value || scrollToItem.value) {
        scrollPending.value = true
        scrollToItem.value = false
    }
})

defineExpose({
    getPlaylist,
})

function scrollTo(index: number) {
    sortContainer.value?.scrollToIndex(index)
}

async function getPlaylist() {
    playlistStore.isLoading = true
    await playlistStore.getPlaylist(listDate.value)
    playlistStore.isLoading = false

    scrollPending.value = true
    if (activeRowIndex.value < 0) {
        await nextTick()
        scrollTo(0)
    }
}

// Anchors the shift-range selection to the last row clicked without modifiers.
let selectionAnchorUid = ''
let pendingCollapse = false

const selectedRows = computed(() =>
    playlistStore.playlist.filter((item) => playlistStore.selectedUids.includes(item.uid)),
)

// Selection happens on mousedown, not click: on draggable rows the browser
// suppresses click when the pointer moves slightly, which would otherwise
// swallow the selection.
function selectPlaylistRow(index: number, event: MouseEvent) {
    if (event.button !== 0) return

    if ((event.target as HTMLElement).closest('button, input')) return

    const item = playlistStore.playlist[index]

    if (!item) return

    pendingCollapse = false

    if (event.shiftKey && selectionAnchorUid) {
        const from = playlistStore.playlist.findIndex((entry) => entry.uid === selectionAnchorUid)
        const to = index

        if (from >= 0) {
            const [start, end] = from <= to ? [from, to] : [to, from]
            playlistStore.selectedUids = playlistStore.playlist
                .slice(start, end + 1)
                .map((entry) => entry.uid)
        }

        return
    }

    selectionAnchorUid = item.uid

    if (event.ctrlKey || event.metaKey) {
        playlistStore.selectedUids = playlistStore.selectedUids.includes(item.uid)
            ? playlistStore.selectedUids.filter((uid) => uid !== item.uid)
            : [...playlistStore.selectedUids, item.uid]

        return
    }

    if (!playlistStore.selectedUids.includes(item.uid)) {
        playlistStore.selectedUids = [item.uid]

        return
    }

    // Pressing an already selected row keeps the whole selection so the
    // drag can carry it; only a plain click without drag collapses it.
    pendingCollapse = true
}

// Click fires only when the press did not turn into a drag.
function onRowClick(index: number) {
    if (!pendingCollapse) return

    pendingCollapse = false

    const item = playlistStore.playlist[index]

    if (item) playlistStore.selectedUids = [item.uid]
}

function clearPlaylistSelection() {
    playlistStore.selectedUids = []
    selectionAnchorUid = ''
}

function cloneItem(item: PlaylistItem): PlaylistItem {
    return JSON.parse(JSON.stringify(item))
}

function copySelected() {
    if (!selectedRows.value.length) return false

    playlistStore.clipboardItems = selectedRows.value.map((item) => cloneItem(item))
    playlistStore.clipboardMode = 'copy'

    return true
}

function cutSelected() {
    if (!selectedRows.value.length) return false

    playlistStore.clipboardItems = selectedRows.value.map((item) => cloneItem(item))
    playlistStore.clipboardMode = 'cut'

    return true
}

function deleteSelected() {
    if (!selectedRows.value.length) return false

    const uids = new Set(selectedRows.value.map((item) => item.uid))
    playlistStore.playlist = playlistStore.playlist.filter((item) => !uids.has(item.uid))
    clearPlaylistSelection()
    processPlaylist(listDate.value, playlistStore.playlist, false)

    return true
}

function pasteClipboard() {
    const items = playlistStore.clipboardItems

    if (!items.length) return false

    let insertIndex = playlistStore.playlist.length
    const lastSelected = selectedRows.value.at(-1)?.uid

    if (lastSelected) {
        const index = playlistStore.playlist.findIndex((item) => item.uid === lastSelected)

        if (index >= 0) {
            insertIndex = index + 1
        }
    }

    let list = [...playlistStore.playlist]

    if (playlistStore.clipboardMode === 'cut') {
        // Cut items stay in place until they are pasted somewhere else.
        const cutUids = new Set(items.map((item) => item.uid))
        const removedBefore = list.slice(0, insertIndex).filter((item) => cutUids.has(item.uid)).length

        list = list.filter((item) => !cutUids.has(item.uid))
        insertIndex -= removedBefore
        playlistStore.clipboardItems = []
        playlistStore.clipboardMode = ''
    }

    const pasted = items.map((item) => ({ ...cloneItem(item), uid: genUID() }))

    list.splice(insertIndex, 0, ...pasted)
    playlistStore.playlist = list
    clearPlaylistSelection()
    processPlaylist(listDate.value, playlistStore.playlist, false)
    flashRows(pasted.map((item) => item.uid))

    return true
}

function onPlaylistKeydown(event: KeyboardEvent) {
    const target = event.target as HTMLElement | null

    if (target?.closest('input, textarea, select, [contenteditable]')) return

    if (event.ctrlKey || event.metaKey) {
        const key = event.key.toLowerCase()
        const action =
            key === 'c' ? copySelected : key === 'x' ? cutSelected : key === 'v' ? pasteClipboard : null

        if (action && action()) {
            event.preventDefault()
        }
    } else if (event.key === 'Delete') {
        if (deleteSelected()) {
            event.preventDefault()
        }
    }
}

onMounted(() => {
    window.addEventListener('keydown', onPlaylistKeydown)
})

onBeforeUnmount(() => {
    window.removeEventListener('keydown', onPlaylistKeydown)
})

// A drag that starts on a selected row carries the whole selection.
function draggedRows(item: PlaylistItem) {
    return playlistStore.selectedUids.length > 1 && playlistStore.selectedUids.includes(item.uid)
        ? playlistStore.playlist.filter((entry) => playlistStore.selectedUids.includes(entry.uid))
        : [item]
}

// Same native HTML5 drag flow as the media browsers: the badge must be in
// the DOM before the browser snapshots the drag image.
function handleRowDragStart(event: DragEvent, item: PlaylistItem) {
    const selection = draggedRows(item)

    // Only a drag that carries the whole selection gets the count badge.
    multiDragGhost(event, selection.length, t('player.selectedCount', { n: selection.length }))
    event.dataTransfer?.setData(
        PLAYLIST_ROW_DRAG_TYPE,
        JSON.stringify(selection.map((entry) => entry.uid)),
    )
}

function handleRowDragEnd() {
    stopDragScroll()
    clearDropHint()
}

// Only playlist rows and media browser drags are accepted here; OS file
// drops keep the browser default behavior.
function onNativeDragOver(event: DragEvent) {
    const types = event.dataTransfer?.types

    if (!types?.includes(MEDIA_DRAG_TYPE) && !types?.includes(PLAYLIST_ROW_DRAG_TYPE)) {
        stopDragScroll()

        return
    }

    event.preventDefault()
    updateDragScroll(event)

    const target = document.elementFromPoint(event.clientX, event.clientY)
    const row = target instanceof Element ? target.closest<HTMLElement>('li') : null

    if (!row) {
        clearDropHint()

        return
    }

    const rect = row.getBoundingClientRect()
    const above = event.clientY <= rect.top + rect.height / 2

    if (row === hintRow && above === hintAbove) return

    clearDropHint()

    row.classList.add(above ? 'playlist-drop-above' : 'playlist-drop-below')
    hintRow = row
    hintAbove = above
}

function onNativeDragLeave(event: DragEvent) {
    if (event.target === event.currentTarget) {
        stopDragScroll()
        clearDropHint()
    }
}

// Move the dragged rows to the pointer position, keeping their order.
function reorderRows(event: DragEvent) {
    const uids = JSON.parse(event.dataTransfer?.getData(PLAYLIST_ROW_DRAG_TYPE) ?? '[]')

    if (!uids.length) return

    const draggedSet = new Set<string>(uids)
    const dragged = playlistStore.playlist.filter((item) => draggedSet.has(item.uid))

    if (!dragged.length) return

    const index = nativeDropIndex(event, playlistStore.playlist)
    const draggedBefore = playlistStore.playlist
        .slice(0, index)
        .filter((item) => draggedSet.has(item.uid)).length
    const rest = playlistStore.playlist.filter((item) => !draggedSet.has(item.uid))

    rest.splice(index - draggedBefore, 0, ...dragged)
    playlistStore.playlist = rest
    processPlaylist(listDate.value, playlistStore.playlist, false)
    flashRows(dragged.map((item) => item.uid))
}

function mediaObjectsFrom(sources: any[]) {
    return sources.map((file: any) => ({
        uid: genUID(),
        begin: 0,
        source: file.source ?? `${mediaStore.folderTree.source}/${file.name}`.replace(/\\/g, '/'),
        in: 0,
        out: file.duration || 10,
        duration: file.duration || 10,
        ad: false,
    }))
}

// Native HTML5 drops: playlist row drags reorder the list, media browser
// drags insert the dragged files at the pointer position.
function onNativeDrop(event: DragEvent) {
    const types = event.dataTransfer?.types
    stopDragScroll()
    clearDropHint()

    if (types?.includes(PLAYLIST_ROW_DRAG_TYPE)) {
        reorderRows(event)

        return
    }

    if (!types?.includes(MEDIA_DRAG_TYPE)) return

    const itemData = event.dataTransfer?.getData(MEDIA_DRAG_TYPE)

    if (!itemData) return

    const item = JSON.parse(itemData)

    if (item.uid !== undefined) return

    // A drag that starts on a selected row carries the whole selection.
    const multiSelect = mediaStore.selectedFiles.length > 1 && mediaStore.selectedFiles.includes(item.name)
    const sources = multiSelect
        ? mediaStore.folderTree.files.filter((file: any) => mediaStore.selectedFiles.includes(file.name))
        : [item]

    const index = nativeDropIndex(event, playlistStore.playlist)
    const mediaObjects = mediaObjectsFrom(sources)

    playlistStore.playlist.splice(index, 0, ...mediaObjects)

    for (const [offset, object] of mediaObjects.entries()) {
        if (!sources[offset].duration) {
            fetchMediaDuration(object)
        }
    }

    processPlaylist(listDate.value, playlistStore.playlist, false)
    flashRows(mediaObjects.map((object) => object.uid))
}

// Fast tree listings skip ffprobe, so dropped clips start with a default
// duration and get the real one filled in as soon as it is known.
async function fetchMediaDuration(item: any) {
    const channel = configStore.channels[configStore.i]?.id

    try {
        const info = await authFetch<{ duration: number }>(`/api/file/${channel}/media-info`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ source: item.source }),
        })

        if (info.duration && playlistStore.playlist.includes(item)) {
            item.duration = info.duration
            item.out = info.duration
            processPlaylist(listDate.value, playlistStore.playlist, false)
        }
    } catch {
        // Keep the default duration when the probe fails.
    }
}

function deletePlaylistItem(index: number) {
    playlistStore.playlist.splice(index, 1)

    processPlaylist(listDate.value, playlistStore.playlist, false)
}
</script>
<style>
#sort-container .timeHidden {
    display: none !important;
}

/*
    Native drop position indicator
*/
.playlist-drop-above {
    box-shadow: inset 0 2px 0 0 #84cc16 !important;
}

.playlist-drop-below {
    box-shadow: inset 0 -2px 0 0 #84cc16 !important;
}
</style>
<template>
    <div class="relative w-full h-full bg-base-300! rounded-e flex flex-col">
        <div v-if="playlistStore.isLoading" class="w-full h-full absolute z-2 flex justify-center bg-base-100/70">
            <span class="loading loading-spinner loading-lg" />
        </div>

        <div
            v-if="selectedRows.length || playlistStore.clipboardItems.length"
            class="absolute top-[38px] end-3 z-3 flex items-center gap-1 bg-base-100 border border-base-content/20 rounded shadow-lg p-1"
        >
            <span v-if="selectedRows.length" class="px-1 text-xs text-base-content/70 tabular-nums">
                {{ t('player.selectedCount', { n: selectedRows.length }) }}
            </span>
            <button
                class="btn btn-xs"
                :disabled="!selectedRows.length"
                :title="t('player.copySelected')"
                :aria-label="t('player.copySelected')"
                @click="copySelected"
            >
                <i class="bi-copy" />
            </button>
            <button
                class="btn btn-xs"
                :disabled="!selectedRows.length"
                :title="t('player.cutSelected')"
                :aria-label="t('player.cutSelected')"
                @click="cutSelected"
            >
                <i class="bi-scissors" />
            </button>
            <button
                class="btn btn-xs"
                :disabled="!playlistStore.clipboardItems.length"
                :title="t('player.pasteClipboard')"
                :aria-label="t('player.pasteClipboard')"
                @click="pasteClipboard"
            >
                <i class="bi-clipboard" />
            </button>
            <button
                class="btn btn-xs text-error"
                :disabled="!selectedRows.length"
                :title="t('player.deleteSelected')"
                :aria-label="t('player.deleteSelected')"
                @click="deleteSelected"
            >
                <i class="bi-trash" />
            </button>
            <button
                v-if="selectedRows.length"
                class="btn btn-xs"
                :title="t('player.clearSelection')"
                :aria-label="t('player.clearSelection')"
                @click="clearPlaylistSelection"
            >
                <i class="bi-x" />
            </button>
        </div>

        <div
            class="shrink-0 grid grid-cols-[85px_auto_85px_85px] 2md:grid-cols-[85px_auto_85px_85px_85px_85px_85px_85px_85px] bg-base-100 border-b border-base-content/30 rounded-tr-lg py-2 text-sm font-bold text-base-content/70"
        >
            <div v-if="!configStore.playout.playlist.infinit" class="px-3">
                {{ t('player.start') }}
            </div>
            <div>
                {{ t('player.file') }}
            </div>
            <div class="px-2 text-center">
                {{ t('player.play') }}
            </div>
            <div class="px-2 hidden 2md:block text-center">
                {{ t('player.duration') }}
            </div>
            <div class="px-2 hidden 2md:block text-center">
                {{ t('player.in') }}
            </div>
            <div class="px-2 hidden 2md:block text-center">
                {{ t('player.out') }}
            </div>
            <div class="px-2 hidden 2md:block text-center">
                {{ t('player.ad') }}
            </div>
            <div class="px-2 text-center">
                {{ t('player.edit') }}
            </div>
            <div class="px-2 hidden 2md:block text-center">
                {{ t('player.delete') }}
            </div>
        </div>

        <!-- Native drops and drag-over indicator for row and media drags;
             the sortable drop event of VirtualList is not used anymore. -->
        <div
            id="playlist-container"
            ref="playlistContainer"
            class="min-h-0 flex-1"
            @dragover="onNativeDragOver"
            @dragleave="onNativeDragLeave"
            @drop="onNativeDrop"
        >
            <VirtualList
                id="sort-container"
                ref="sortContainer"
                v-model="playlistStore.playlist"
                class="h-full"
                :data-key="'uid'"
                :disabled="true"
                wrap-tag="ul"
                wrap-class="relative list-none text-sm"
                :tableMode="false"
                :animation="100"
            >
                <template #header>
                    <div v-if="playlistStore.playlist.length === 0" class="is-empty relative">
                        <div class="absolute text-8xl text-base-content/40 text-center w-full mt-10">
                            <i class="bi-box-arrow-in-down" />
                        </div>
                    </div>
                </template>
                <template #item="{ item, index }">
                    <li
                        :id="item.uid"
                        :key="item.uid"
                        draggable="true"
                        class="grid grid-cols-[85px_auto_85px_85px] 2md:grid-cols-[85px_auto_85px_85px_85px_85px_85px_85px_85px] odd:bg-base-200 border-b border-base-content/20 duration-500 transition-colors py-2 select-none cursor-default"
                        :class="{
                            'bg-lime-500/30!': index === activeRowIndex,
                            'bg-amber-600/40!': item.overtime,
                            'bg-warning/40!': playlistStore.selectedUids.includes(item.uid),
                            'opacity-50':
                                playlistStore.clipboardMode === 'cut' &&
                                playlistStore.clipboardItems.some((clip) => clip.uid === item.uid),
                            'text-base-content/60': item.ad,
                        }"
                        @dragstart="handleRowDragStart($event, item)"
                        @dragend="handleRowDragEnd"
                        @mousedown="selectPlaylistRow(index, $event)"
                        @click="onRowClick(index)"
                    >
                        <div v-if="!configStore.playout.playlist.infinit" class="px-3 text-left">
                            {{ secondsToTime(item.begin) }}
                        </div>
                        <div
                            class="flex items-center gap-1 text-left truncate"
                            :class="{ 'handle cursor-grab': width > 768 }"
                        >
                            <span class="truncate">{{ item.title || filename(item.source) }}</span>
                            <i
                                v-if="item.audio"
                                class="bi-music-note-beamed shrink-0"
                                :title="`${t('player.audio')}: ${filename(item.audio)}`"
                            />
                        </div>
                        <div class="text-center hover:text-base-content/70">
                            <button class="cursor-pointer" @click="preview(item.source)">
                                <i class="bi-play-fill" />
                            </button>
                        </div>
                        <div class="text-center hidden 2md:block">{{ secToHMS(item.duration) }}</div>
                        <div class="text-center hidden 2md:block">
                            {{ secToHMS(item.in) }}
                        </div>
                        <div class="text-center hidden 2md:block">
                            {{ secToHMS(item.out) }}
                        </div>
                        <div class="text-center hidden 2md:block leading-3">
                            <input v-model="item.ad" class="checkbox checkbox-xs rounded" type="checkbox" />
                        </div>
                        <div class="text-center hover:text-base-content/70">
                            <button class="cursor-pointer" @click="editItem(index)">
                                <i class="bi-pencil-square" />
                            </button>
                        </div>
                        <div
                            class="text-center hidden 2md:block justify-center text-base-content/80 hover:text-base-content/60"
                        >
                            <button class="cursor-pointer" @click="deletePlaylistItem(index)">
                                <i class="bi-x-circle-fill" />
                            </button>
                        </div>
                    </li>
                </template>
            </VirtualList>
        </div>
    </div>
</template>
