<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import VirtualList from 'vue-virtual-sortable'

import { authFetch } from '@/composables/authFetch'
import { playlistOperations, stringFormatter } from '@/composables/helper'
import { useAuth } from '@/stores/auth'
import { useConfig } from '@/stores/config'
import { useMedia } from '@/stores/media'

const { genUID } = playlistOperations()
const { mediaType, toMin } = stringFormatter()

const dragGroup = { name: 'dragGroup', pull: 'clone', put: false }

const props = defineProps<{
    name: string
    path: string
    depth: number
    compact?: boolean
    dropFiles?: (files: File[], targetPath: string) => Promise<void> | void
    preview?: (path: string) => Promise<void> | void
}>()

const authStore = useAuth()
const configStore = useConfig()
const mediaStore = useMedia()

const localExpanded = ref(false)
const isLoaded = ref(false)
const loadingChildren = ref(false)
const children = ref<Folder[]>([])
const files = ref<(FileObject & { uid: string; source: string })[]>([])
const dropActive = ref(false)
const rowEl = ref<HTMLElement | null>(null)

// Full mode keeps its local expansion state, compact mode shares it through
// the media store so every node knows its state at mount time.
const expanded = computed({
    get: () => (props.compact ? mediaStore.isTreeExpanded(props.path) : localExpanded.value),
    set: (value: boolean) => {
        if (props.compact) {
            mediaStore.setTreeExpanded(props.path, value)
        } else {
            localExpanded.value = value
        }
    },
})

const isActive = computed(() =>
    props.compact ? mediaStore.treePath === props.path : mediaStore.currentPath === props.path,
)
const isPinned = computed(() => mediaStore.isPreferred(props.path))

function isOnCurrentPath(current: string) {
    if (current === props.path) {
        return true
    }

    const prefix = props.path === '' ? '/' : `${props.path}/`

    return current.startsWith(prefix)
}

// Full mode: follow navigation from breadcrumbs, quick access or the file
// list by expanding every node on the way to the current folder and
// scrolling the active folder into view.
watch(
    () => mediaStore.currentPath,
    async (current) => {
        if (props.compact || !isOnCurrentPath(current)) {
            return
        }

        expanded.value = true
        await loadChildren()

        if (current === props.path) {
            await nextTick()
            rowEl.value?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
        }
    },
    { immediate: true },
)

// Load the folder content whenever the node gets expanded.
watch(
    expanded,
    (value) => {
        if (value) {
            loadChildren()
        }
    },
    { immediate: true },
)

// Compact mode: quick access reveals a folder by expanding its whole chain
// in the store, then the target node scrolls itself into view once mounted.
watch(
    () => [mediaStore.treePath, mediaStore.treeNavVersion] as const,
    async ([target]) => {
        if (!props.compact || target !== props.path) {
            return
        }

        await loadChildren()
        await nextTick()
        rowEl.value?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
    },
    { immediate: true },
)

watch(
    () => mediaStore.treeVersion,
    () => {
        if (expanded.value) {
            isLoaded.value = false
            loadChildren()
        }
    },
)

function joinPath(parentPath: string, name: string) {
    return `${parentPath}/${name}`.replace(/\/[/]+/g, '/')
}

async function loadChildren() {
    if (isLoaded.value || loadingChildren.value) {
        return
    }

    loadingChildren.value = true

    const id = configStore.channels[configStore.i]?.id
    const storage = configStore.channels[configStore.i]?.storage ?? ''

    try {
        const data = await authFetch<any>(`/api/file/${id}/browse`, {
            method: 'POST',
            headers: { ...configStore.contentType, ...authStore.authHeader },
            body: JSON.stringify({ source: props.path, folders_only: !props.compact, with_duration: !props.compact }),
        })

        children.value = Array.isArray(data.folders)
            ? data.folders.map((name: string) => ({ uid: genUID(), name }))
            : []
        files.value =
            props.compact && Array.isArray(data.files)
                ? data.files.map((file: FileObject) => ({
                      ...file,
                      uid: genUID(),
                      source: `${storage}${props.path}/${file.name}`.replace(/\/[/]+/g, '/'),
                  }))
                : []
        isLoaded.value = true
    } catch {
        isLoaded.value = true
    } finally {
        loadingChildren.value = false
    }
}

function toggleExpand() {
    expanded.value = !expanded.value
}

function handleClick() {
    if (props.compact) {
        mediaStore.treePath = props.path
        expanded.value = !expanded.value
    } else {
        mediaStore.getTree(props.path)
    }
}

async function previewFile(file: FileObject & { uid: string; source: string }) {
    await props.preview?.(file.source)
}

function fileIcon(name: string) {
    const type = mediaType(name)

    if (type === 'audio') {
        return 'bi-music-note-beamed'
    }

    if (type === 'video') {
        return 'bi-film'
    }

    if (type === 'image') {
        return 'bi-file-earmark-image'
    }

    return 'bi-file-binary'
}

function onDragOver(event: DragEvent) {
    if (event.dataTransfer?.types.includes('Files') || event.dataTransfer?.types.includes('application/json')) {
        event.preventDefault()
        dropActive.value = true
    }
}

function onDragLeave() {
    dropActive.value = false
}

async function onDrop(event: DragEvent) {
    dropActive.value = false

    const files = event.dataTransfer?.files

    if (files && files.length && props.dropFiles) {
        await props.dropFiles(Array.from(files), `${props.path}/`.replace(/\/[/]+/g, '/'))

        return
    }

    const itemData = event.dataTransfer?.getData('application/json')

    if (itemData) {
        const item = JSON.parse(itemData)
        const source = `/${mediaStore.folderTree.source}/${item.name}`.replace(/\/[/]+/g, '/')

        await mediaStore.moveItem(source, props.path || '/', item.name)
    }
}
</script>
<template>
    <li>
        <div
            ref="rowEl"
            class="group flex items-center gap-1 rounded cursor-pointer hover:bg-base-100/60"
            :class="[compact ? 'py-0 text-xs' : 'px-1 py-0.5 text-sm', { 'bg-base-100/80!': dropActive, 'bg-base-100/60!': isActive }]"
            :style="{ paddingLeft: `${depth * (compact ? 8 : 12) + 2}px` }"
            :title="path || name"
            @click="handleClick"
            @drop="onDrop"
            @dragover="onDragOver"
            @dragleave="onDragLeave"
        >
            <button
                class="w-3 h-3 flex items-center justify-center cursor-pointer"
                :title="expanded ? 'collapse' : 'expand'"
                @click.stop="toggleExpand"
            >
                <i v-if="expanded" class="bi-chevron-down text-[10px] text-base-content/60" />
                <i v-else class="bi-chevron-right text-[10px] text-base-content/60" />
            </button>
            <i class="text-base-content/70" :class="expanded ? 'bi-folder2-open' : 'bi-folder-fill'" />
            <span class="truncate flex-1" :class="{ 'font-bold': isActive }">{{ name }}</span>
            <button
                v-if="compact && path"
                class="cursor-pointer shrink-0"
                :class="isPinned ? 'text-warning' : 'opacity-0 group-hover:opacity-60 hover:opacity-100!'"
                :title="isPinned ? $t('media.unpinFolder') : $t('media.pinFolder')"
                @click.stop="mediaStore.togglePreferredFolder(path)"
            >
                <i :class="isPinned ? 'bi-star-fill' : 'bi-star'" />
            </button>
        </div>

        <ul v-if="expanded && isLoaded" class="list-none">
            <MediaTreeItem
                v-for="folder in children"
                :key="folder.uid"
                :name="folder.name"
                :path="joinPath(path, folder.name)"
                :depth="depth + 1"
                :compact="compact"
                :drop-files="dropFiles"
                :preview="preview"
            />

            <li v-if="compact && files.length">
                <VirtualList
                    id="mediaList"
                    v-model="files"
                    :keeps="files.length || 1"
                    :group="dragGroup"
                    :data-key="'uid'"
                    :handle="'.handle'"
                    :sortable="false"
                    ghost-class="sortable-ghost"
                    placeholder-class="media-placeholder"
                    chosen-class="cursor-grabbing"
                    wrap-tag="ul"
                    wrap-class="list-none"
                >
                    <template #item="{ item }">
                        <li
                            class="flex items-center gap-1 rounded py-0 text-xs hover:bg-base-100/60"
                            :style="{ paddingLeft: `${(depth + 1) * 8 + 16}px` }"
                            :title="item.source"
                            @click="previewFile(item)"
                        >
                            <i class="text-base-content/60" :class="fileIcon(item.name)" />
                            <span class="truncate handle cursor-grab">{{ item.name }}</span>
                            <span v-if="item.duration" class="ms-auto shrink-0 text-base-content/40">
                                {{ toMin(item.duration) }}
                            </span>
                        </li>
                    </template>
                </VirtualList>
            </li>
        </ul>
    </li>
</template>
