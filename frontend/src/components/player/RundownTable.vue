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

import { ref, onBeforeUnmount, onMounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useWindowSize } from '@vueuse/core'
import { storeToRefs } from 'pinia'

dayjs.extend(customParseFormat)
dayjs.extend(LocalizedFormat)
dayjs.extend(timezone)
dayjs.extend(utc)

import { stringFormatter } from '@/composables/helper'
import { flashRows } from '@/composables/rowFlash'
import {
    MEDIA_DRAG_TYPE,
    RUNDOWN_ROW_DRAG_TYPE,
    multiDragGhost,
    nativeDropIndex,
    stopDragScroll,
    updateDragScroll,
} from '@/composables/multiDragGhost'
import { useConfig } from '@/stores/config'
import { useIndex } from '@/stores/index'
import { useMedia } from '@/stores/media'
import { useRundown, type RundownItemView } from '@/stores/rundown'

const { t } = useI18n()
const { width } = useWindowSize({ initialWidth: 800 })
const configStore = useConfig()
const indexStore = useIndex()
const mediaStore = useMedia()
const rundownStore = useRundown()
const { secToHMS, filename } = stringFormatter()
const { i } = storeToRefs(useConfig())

const sortContainer = ref()

function beginTime(item: RundownItem) {
    return dayjs(item.begin_epoch * 1000)
        .tz(configStore.timezone)
        .format('HH:mm:ss')
}

function playTime(item: RundownItem) {
    return secToHMS(item.out - item.in)
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
    window.addEventListener('keydown', onRundownKeydown)
    setTimeout(() => {
        rundownStore.fetchQueue()
    }, 150)
})

onBeforeUnmount(() => {
    window.removeEventListener('keydown', onRundownKeydown)
})

watch(i, () => {
    clipboardItems.value = []
    clipboardMode.value = ''
    setTimeout(() => {
        rundownStore.fetchQueue()
    }, 800)
})

function scrollTo(index: number) {
    sortContainer.value?.scrollToIndex(index)
}

defineExpose({
    scrollTo,
})

// Anchors the shift-range selection to the last row pressed without modifiers.
const selectedUids = ref<string[]>([])
let selectionAnchorUid = ''
let pendingCollapse = false

// Selection happens on mousedown, not click: on draggable rows the browser
// suppresses click when the pointer moves slightly, which would otherwise
// swallow the selection.
function selectRundownRow(index: number, event: MouseEvent) {
    if (event.button !== 0) return

    if ((event.target as HTMLElement).closest('button, input')) return

    const item = rundownStore.items[index]

    if (!item) return

    pendingCollapse = false

    if (event.shiftKey && selectionAnchorUid) {
        const from = rundownStore.items.findIndex((entry) => entry.uid === selectionAnchorUid)
        const to = index

        if (from >= 0) {
            const [start, end] = from <= to ? [from, to] : [to, from]
            selectedUids.value = rundownStore.items.slice(start, end + 1).map((entry) => entry.uid)
        }

        return
    }

    selectionAnchorUid = item.uid

    if (event.ctrlKey || event.metaKey) {
        selectedUids.value = selectedUids.value.includes(item.uid)
            ? selectedUids.value.filter((uid) => uid !== item.uid)
            : [...selectedUids.value, item.uid]

        return
    }

    if (!selectedUids.value.includes(item.uid)) {
        selectedUids.value = [item.uid]

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

    const item = rundownStore.items[index]

    if (item) selectedUids.value = [item.uid]
}

async function deleteSelectedRows() {
    const uids = new Set(selectedUids.value)

    for (const row of rundownStore.items.filter((item) => uids.has(item.uid))) {
        try {
            await rundownStore.deleteItem(row.id)
        } catch (error) {
            indexStore.msgAlert(
                'error',
                error instanceof Error ? error.message : String(error),
                4,
            )
            await rundownStore.fetchQueue()

            return
        }
    }

    selectedUids.value = []
    selectionAnchorUid = ''
}

function onRundownKeydown(event: KeyboardEvent) {
    const target = event.target as HTMLElement | null

    if (target?.closest('input, textarea, select, [contenteditable]')) return

    if (event.ctrlKey || event.metaKey) {
        const key = event.key.toLowerCase()

        if (key === 'c' && copySelected()) {
            event.preventDefault()
        } else if (key === 'x' && cutSelected()) {
            event.preventDefault()
        } else if (key === 'v' && clipboardItems.value.length) {
            event.preventDefault()
            void pasteClipboard()
        }
    } else if (event.key === 'Delete' && selectedUids.value.length) {
        deleteSelectedRows()
        event.preventDefault()
    }
}

// Clipboard for copy/cut/paste of selected rows. Rows are recreated on the
// server on paste; a cut is a copy that deletes the originals right away.
const clipboardItems = ref<RundownItemView[]>([])
const clipboardMode = ref<'' | 'copy' | 'cut'>('')

function copySelected(): boolean {
    const rows = rundownStore.items.filter((item) => selectedUids.value.includes(item.uid))

    if (!rows.length) return false

    clipboardItems.value = rows.map((row) => ({ ...row }))
    clipboardMode.value = 'copy'

    return true
}

function cutSelected(): boolean {
    if (!copySelected()) return false

    clipboardMode.value = 'cut'
    void deleteSelectedRows()

    return true
}

async function pasteClipboard() {
    const items = clipboardItems.value

    if (!items.length) return

    // Paste after the last selected row, or append at the end. The anchor
    // is a row uid resolved against the refreshed queue, so each pasted
    // item lands right behind the previous one.
    let anchorUid: string | null = null
    let anchorAtEnd = true
    const freshUids: string[] = []

    const lastSelected = selectedUids.value.at(-1)

    if (lastSelected) {
        const index = rundownStore.items.findIndex((entry) => entry.uid === lastSelected)

        if (index >= 0) {
            anchorUid = rundownStore.items[index].uid
            anchorAtEnd = false
        }
    }

    try {
        for (const item of items) {
            const after = anchorAtEnd
                ? null
                : (rundownStore.items.find((entry) => entry.uid === anchorUid)?.position ?? -1)

            const knownIds = new Set(rundownStore.items.map((entry) => entry.id))

            await rundownStore.addItem({
                source: item.source,
                in: item.in,
                out: item.out,
                duration: item.duration,
                title: item.title,
                ad: item.ad,
                custom_filter: item.custom_filter,
                audio: item.audio,
                after_position: after,
            })

            const fresh = rundownStore.items.find((entry) => !knownIds.has(entry.id))

            if (fresh) {
                anchorUid = fresh.uid
                anchorAtEnd = false
                freshUids.push(fresh.uid)
            } else {
                anchorAtEnd = true
            }
        }

        flashRows(freshUids)
    } catch (error) {
        indexStore.msgAlert(
            'error',
            error instanceof Error ? error.message : String(error),
            4,
        )
        await rundownStore.fetchQueue()
    }

    if (clipboardMode.value === 'cut') {
        clipboardItems.value = []
        clipboardMode.value = ''
    }
}

// Native HTML5 drag state for the insertion indicator. The hint class is
// set directly on the row element: routing it through a reactive ref would
// re-render every visible row on each dragover event and stutter the drag.
let hintRow: HTMLElement | null = null
let hintAbove = false

function clearDropHint() {
    hintRow?.classList.remove('playlist-drop-above', 'playlist-drop-below')
    hintRow = null
}

// A drag that starts on a selected row carries the whole selection.
function draggedRows(item: RundownItemView) {
    return selectedUids.value.length > 1 && selectedUids.value.includes(item.uid)
        ? rundownStore.items.filter((entry) => selectedUids.value.includes(entry.uid))
        : [item]
}

// Same native HTML5 drag flow as the media browsers: the badge must be in
// the DOM before the browser snapshots the drag image.
function handleRowDragStart(event: DragEvent, item: RundownItemView) {
    const selection = draggedRows(item)

    // Only a drag that carries the whole selection gets the count badge.
    multiDragGhost(event, selection.length, t('player.selectedCount', { n: selection.length }))
    event.dataTransfer?.setData(
        RUNDOWN_ROW_DRAG_TYPE,
        JSON.stringify(selection.map((entry) => entry.id)),
    )
}

function handleRowDragEnd() {
    stopDragScroll()
    clearDropHint()
}

// Only rundown rows and media browser drags are accepted here; OS file
// drops keep the browser default behavior.
function onNativeDragOver(event: DragEvent) {
    const types = event.dataTransfer?.types

    if (!types?.includes(MEDIA_DRAG_TYPE) && !types?.includes(RUNDOWN_ROW_DRAG_TYPE)) {
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

async function insertSources(sources: any[], index: number) {
    // Anchor row for the first insert; later inserts chain onto the row
    // inserted before them. Every mutation can renumber positions, so the
    // anchor is stored as a row uid and resolved against the refreshed
    // queue before each insert instead of keeping a stale position. A drop
    // below the last row has no anchor row, which appends at the end.
    let anchorUid: string | null = index > 0 ? (rundownStore.items[index - 1]?.uid ?? null) : null
    let anchorAtEnd = index >= rundownStore.items.length
    const freshUids: string[] = []

    for (const file of sources) {
        const after = anchorAtEnd ? null : (rundownStore.items.find((entry) => entry.uid === anchorUid)?.position ?? -1)

        const sourcePath =
            file.source ?? `${mediaStore.folderTree.source}/${file.name}`.replace(/\\/g, '/')

        const knownIds = new Set(rundownStore.items.map((entry) => entry.id))

        await rundownStore.addItem({
            source: sourcePath,
            after_position: after,
            out: file.duration || undefined,
            duration: file.duration || undefined,
        })

        // The next file lands right after the one just inserted; identify
        // it in the refreshed queue as the item the queue did not have yet.
        const fresh = rundownStore.items.find((entry) => !knownIds.has(entry.id))

        if (fresh) {
            anchorUid = fresh.uid
            anchorAtEnd = false
            freshUids.push(fresh.uid)
        } else {
            anchorAtEnd = true
        }
    }

    flashRows(freshUids)
}

// Move the dragged rows to the pointer position on the server, keeping
// their order: each row is anchored right after the previously moved row.
async function reorderRows(event: DragEvent) {
    const ids: number[] = JSON.parse(event.dataTransfer?.getData(RUNDOWN_ROW_DRAG_TYPE) ?? '[]')

    if (!ids.length) return

    const draggedSet = new Set(ids.map((id) => String(id)))
    const index = nativeDropIndex(event, rundownStore.items)

    // Anchor at the nearest row before the drop point that is not part of
    // the drag itself; the anchor is a row uid, resolved against the
    // refreshed queue before every move. For a drop at or below the end the
    // walk-back ends at the last row outside the drag, so the block lands
    // there. A move with no anchor at all (whole queue dragged) goes on top.
    let anchorUid: string | null = null

    for (let i = index - 1; i >= 0; i--) {
        const row = rundownStore.items[i]

        if (row && !draggedSet.has(row.uid)) {
            anchorUid = row.uid

            break
        }
    }

    try {
        for (const id of ids) {
            const after = anchorUid
                ? (rundownStore.items.find((entry) => entry.uid === anchorUid)?.position ?? -1)
                : -1

            await rundownStore.moveItem(id, after)

            anchorUid = String(id)
        }

        flashRows(ids.map((id) => String(id)))
    } catch (error) {
        indexStore.msgAlert(
            'error',
            error instanceof Error ? error.message : String(error),
            4,
        )
        await rundownStore.fetchQueue()
    }
}

// Native HTML5 drops: rundown row drags move the row on the server, media
// browser drags insert the dragged files at the pointer position.
async function onNativeDrop(event: DragEvent) {
    const types = event.dataTransfer?.types
    stopDragScroll()
    clearDropHint()

    if (types?.includes(RUNDOWN_ROW_DRAG_TYPE)) {
        await reorderRows(event)

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

    try {
        await insertSources(sources, nativeDropIndex(event, rundownStore.items))
    } catch (error) {
        indexStore.msgAlert(
            'error',
            error instanceof Error ? error.message : String(error),
            4,
        )
        await rundownStore.fetchQueue()
    }
}

async function deleteRundownItem(item: RundownItem) {
    try {
        await rundownStore.deleteItem(item.id)
    } catch (error) {
        indexStore.msgAlert('error', error instanceof Error ? error.message : String(error), 4)
        await rundownStore.fetchQueue()
    }
}

async function jumpTo(item: RundownItem) {
    try {
        await rundownStore.jumpTo(item.id)
    } catch (error) {
        indexStore.msgAlert('error', error instanceof Error ? error.message : String(error), 4)
        await rundownStore.fetchQueue()
    }
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
        <div v-if="rundownStore.isLoading" class="w-full h-full absolute z-2 flex justify-center bg-base-100/70">
            <span class="loading loading-spinner loading-lg" />
        </div>

        <div
            class="shrink-0 grid grid-cols-[85px_auto_85px_85px] 2md:grid-cols-[85px_auto_85px_85px_85px_85px_85px_85px_85px] bg-base-100 border-b border-base-content/30 rounded-tr-lg py-2 text-sm font-bold text-base-content/70"
        >
            <div class="px-3">
                {{ t('player.start') }}
            </div>
            <div>
                {{ t('player.file') }}
            </div>
            <div class="px-2 text-center">
                {{ t('player.play') }}
            </div>
            <div class="px-2 hidden 2md:block text-center">
                {{ t('player.play') }}
            </div>
            <div class="px-2 hidden 2md:block text-center">
                {{ t('player.in') }}
            </div>
            <div class="px-2 hidden 2md:block text-center">
                {{ t('player.out') }}
            </div>
            <div class="px-2 hidden 2md:block text-center">
                {{ t('player.jump') }}
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
            id="rundown-container"
            class="min-h-0 flex-1"
            @dragover="onNativeDragOver"
            @dragleave="onNativeDragLeave"
            @drop="onNativeDrop"
        >
            <VirtualList
                id="sort-container"
                ref="sortContainer"
                v-model="rundownStore.items"
                class="h-full"
                :data-key="'uid'"
                :disabled="true"
                wrap-tag="ul"
                wrap-class="relative list-none text-sm"
                :tableMode="false"
                :animation="100"
            >
                <template #header>
                    <div v-if="rundownStore.items.length === 0" class="is-empty relative">
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
                            'bg-lime-500/30!': item.id === rundownStore.headItemId,
                            'bg-warning/40!': selectedUids.includes(item.uid),
                            'text-base-content/60': item.played,
                            'text-base-content/40': item.ad,
                        }"
                        @dragstart="handleRowDragStart($event, item)"
                        @dragend="handleRowDragEnd"
                        @mousedown="selectRundownRow(index, $event)"
                        @click="onRowClick(index)"
                    >
                        <div class="px-3 text-left">
                            {{ beginTime(item) }}
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
                        <div class="text-center hidden 2md:block">{{ playTime(item) }}</div>
                        <div class="text-center hidden 2md:block">
                            {{ secToHMS(item.in) }}
                        </div>
                        <div class="text-center hidden 2md:block">
                            {{ secToHMS(item.out) }}
                        </div>
                        <div class="text-center hidden 2md:block hover:text-base-content/70">
                            <button
                                v-if="item.id !== rundownStore.headItemId"
                                class="cursor-pointer"
                                :title="t('player.jump')"
                                @click="jumpTo(item)"
                            >
                                <i class="bi-box-arrow-in-right" />
                            </button>
                            <i v-else class="bi-broadcast text-lime-600" :title="t('player.onAir')" />
                        </div>
                        <div class="text-center hover:text-base-content/70">
                            <button class="cursor-pointer" @click="editItem(item.id)">
                                <i class="bi-pencil-square" />
                            </button>
                        </div>
                        <div
                            class="text-center hidden 2md:block justify-center text-base-content/80 hover:text-base-content/60"
                        >
                            <button class="cursor-pointer" @click="deleteRundownItem(item)">
                                <i class="bi-x-circle-fill" />
                            </button>
                        </div>
                    </li>
                </template>
            </VirtualList>
        </div>
    </div>
</template>
