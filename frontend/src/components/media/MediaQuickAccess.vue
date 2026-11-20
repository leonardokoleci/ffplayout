<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { onMounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'

import { useConfig } from '@/stores/config'
import { useMedia } from '@/stores/media'

const { t } = useI18n()

const mediaStore = useMedia()
const { i } = storeToRefs(useConfig())

const manualPath = ref('')

async function pinManualPath() {
    const path = manualPath.value.trim().replace(/\\/g, '/')
    manualPath.value = ''

    if (path && !mediaStore.preferredFolders.includes(path)) {
        await mediaStore.togglePreferredFolder(path)
    }
}

const props = defineProps<{
    compact?: boolean
    dropFiles?: (files: File[], targetPath: string) => Promise<void> | void
}>()

function openPath(path: string) {
    mediaStore.getTree(path.replace(/\\/g, '/'))
}

onMounted(async () => {
    mediaStore.loadPreferredFolders()

    const start = await mediaStore.initialBrowsePath()

    if (start) {
        openPath(start)
    }
})

watch([i], async () => {
    const start = await mediaStore.initialBrowsePath()

    if (start) {
        openPath(start)
    }
})

const dropActivePath = ref('')

function folderName(path: string) {
    return path.split('/').filter(Boolean).slice(-1)[0] || path
}

function onDragOverQuick(event: DragEvent, path: string) {
    if (event.dataTransfer?.types.includes('Files') || event.dataTransfer?.types.includes('application/json')) {
        event.preventDefault()
        dropActivePath.value = path
    }
}

function onDragLeaveQuick(path: string) {
    if (dropActivePath.value === path) {
        dropActivePath.value = ''
    }
}

async function onDropQuick(event: DragEvent, path: string) {
    dropActivePath.value = ''

    const files = event.dataTransfer?.files

    if (files && files.length && props.dropFiles) {
        await props.dropFiles(Array.from(files), `${path}/`.replace(/\/[/]+/g, '/'))

        return
    }

    const itemData = event.dataTransfer?.getData('application/json')

    if (itemData) {
        const item = JSON.parse(itemData)
        const source = `${mediaStore.folderTree.source}/${item.name}`.replace(/\/[/]+/g, '/')

        await mediaStore.moveItem(source, path || '/', item.name)
    }
}
</script>
<template>
    <div class="relative bg-base-300" :class="compact ? 'flex min-h-0 flex-col' : 'h-full overflow-y-auto'">
        <div
            class="shrink-0 px-2 font-bold uppercase text-base-content/60"
            :class="compact ? 'pt-1 pb-0.5 text-[10px]' : 'pt-2 pb-1 text-xs'"
        >
            {{ t('media.drives') }}
        </div>

        <ul class="list-none px-1 border-b border-base-content/20" :class="[compact ? 'pb-0.5' : 'pb-1', compact ? 'shrink-0' : '']">
            <li
                v-for="drive in mediaStore.drives"
                :key="drive"
                class="group flex items-center gap-1 rounded px-1 cursor-pointer hover:bg-base-100/60"
                :class="[compact ? 'py-0 text-xs' : 'py-0.5 text-sm']"
                :title="drive"
                @click="openPath(drive)"
            >
                <i class="bi-device-hdd shrink-0" />
                <span class="truncate flex-1">{{ drive.replace(/\\/g, '/') }}</span>
            </li>
        </ul>

        <div
            class="shrink-0 px-2 font-bold uppercase text-base-content/60"
            :class="compact ? 'pt-1 pb-0.5 text-[10px]' : 'pt-2 pb-1 text-xs'"
        >
            {{ t('media.quickAccess') }}
        </div>

        <form
            class="shrink-0 flex gap-1 px-1 border-b border-base-content/20 items-center"
            :class="compact ? 'py-0.5' : 'py-1'"
            @submit.prevent="pinManualPath"
        >
            <input
                v-model="manualPath"
                type="text"
                class="input input-xs w-full"
                :placeholder="t('media.addFolder')"
            />
            <button type="submit" class="btn btn-xs btn-ghost px-1" :title="t('media.addFolder')">
                <i class="bi-plus-lg" />
            </button>
        </form>

        <ul
            v-if="mediaStore.preferredFolders.length"
            class="list-none px-1 border-b border-base-content/20"
            :class="[compact ? 'pb-0.5' : 'pb-1', compact ? 'shrink-0' : '']"
        >
            <li
                v-for="path in mediaStore.preferredFolders"
                :key="path"
                class="group flex items-center gap-1 rounded px-1 cursor-pointer hover:bg-base-100/60"
                :class="[compact ? 'py-0 text-xs' : 'py-0.5 text-sm', { 'bg-base-100/80!': dropActivePath === path }]"
                :title="path"
                @click="openPath(path)"
                @drop="onDropQuick($event, path)"
                @dragover="onDragOverQuick($event, path)"
                @dragleave="onDragLeaveQuick(path)"
            >
                <i class="bi-star-fill text-warning shrink-0" />
                <span
                    class="truncate flex-1"
                    :class="{ 'font-bold': mediaStore.currentPath === path }"
                >
                    {{ folderName(path) }}
                </span>
                <button
                    class="opacity-0 group-hover:opacity-60 hover:opacity-100! cursor-pointer shrink-0"
                    :title="t('media.unpinFolder')"
                    @click.stop="mediaStore.togglePreferredFolder(path)"
                >
                    <i class="bi-x" />
                </button>
            </li>
        </ul>
        <div
            v-else
            class="px-2 text-base-content/50 border-b border-base-content/20"
            :class="[compact ? 'pb-1 text-[10px] shrink-0' : 'pb-2 text-xs']"
        >
            {{ t('media.quickAccessHint') }}
        </div>
    </div>
</template>
