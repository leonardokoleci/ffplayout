<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { computed, onMounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'

import { useConfig } from '@/stores/config'
import { useMedia } from '@/stores/media'

import MediaTreeItem from './MediaTreeItem.vue'

const { t } = useI18n()

const mediaStore = useMedia()
const configStore = useConfig()
const { i } = storeToRefs(useConfig())

const props = withDefaults(
    defineProps<{
        compact?: boolean
        showTree?: boolean
        dropFiles?: (files: File[], targetPath: string) => Promise<void> | void
        preview?: (path: string) => Promise<void> | void
    }>(),
    { showTree: true },
)

// Compact mode runs without the full browse flow: the tree root name comes
// from the channel storage config and the tree loads its content itself.
const rootName = computed(() => {
    if (props.compact) {
        const storage = configStore.channels[configStore.i]?.storage ?? ''

        return storage.split('/').filter(Boolean).slice(-1)[0] || storage
    }

    return mediaStore.folderTree.parent
})

onMounted(() => {
    if (props.compact) {
        mediaStore.setTreeExpanded('', true)
        mediaStore.loadPreferredFolders()
    } else if (!mediaStore.folderTree.parent) {
        mediaStore.getTree('')
    }
})

watch([i], () => {
    if (props.compact) {
        mediaStore.resetTree()
        mediaStore.setTreeExpanded('', true)
        mediaStore.loadPreferredFolders()
    } else {
        mediaStore.getTree('')
    }
})

const dropActivePath = ref('')

function folderName(path: string) {
    const name = path.split('/').filter(Boolean).slice(-1)[0]

    return name || mediaStore.folderTree.parent || path
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
        const source = `/${mediaStore.folderTree.source}/${item.name}`.replace(/\/[/]+/g, '/')

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
            {{ t('media.quickAccess') }}
        </div>

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
                @click="compact ? mediaStore.revealTreePath(path) : mediaStore.getTree(path)"
                @drop="onDropQuick($event, path)"
                @dragover="onDragOverQuick($event, path)"
                @dragleave="onDragLeaveQuick(path)"
            >
                <i class="bi-star-fill text-warning shrink-0" />
                <span
                    class="truncate flex-1"
                    :class="{ 'font-bold': compact ? mediaStore.treePath === path : mediaStore.currentPath === path }"
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

        <ul v-if="rootName && showTree" class="list-none pt-1" :class="compact ? 'flex-1 min-h-0 overflow-y-auto' : ''">
            <MediaTreeItem
                :key="rootName"
                :name="rootName"
                :path="''"
                :depth="0"
                :compact="compact"
                :drop-files="dropFiles"
                :preview="preview"
            />
        </ul>
    </div>
</template>
