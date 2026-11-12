<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import VirtualList from 'vue-virtual-sortable'

import { stringFormatter } from '@/composables/helper'
import { useMedia } from '@/stores/media'

const { t } = useI18n()
const { mediaType, secToHMS } = stringFormatter()

defineProps<{
    preview?: (path: string) => Promise<void> | void
}>()

const mediaStore = useMedia()

const dragGroup = { name: 'dragGroup', pull: 'clone', put: false }

function folderPath(name: string) {
    return `/${mediaStore.folderTree.source}/${name}`.replace(/\/[/]+/g, '/')
}

// Navigate through the tree path, so the tree and this list stay in sync.
function openFolder(name: string) {
    mediaStore.revealTreePath(folderPath(name))
}

function togglePin(name: string) {
    mediaStore.togglePreferredFolder(folderPath(name))
}
</script>
<template>
    <div class="flex flex-col min-h-0">
        <!-- Breadcrumb path of the current folder -->
        <div v-if="mediaStore.crumbs.length" class="shrink-0 bg-base-100 border-y border-base-content/20">
            <nav class="breadcrumbs px-2 py-1">
                <ul>
                    <li v-for="(crumb, index) in mediaStore.crumbs" :key="index">
                        <button
                            v-if="mediaStore.crumbs.length - 1 > index"
                            class="flex items-center"
                            @click="mediaStore.revealTreePath(crumb.path)"
                        >
                            <i class="bi-folder-fill me-1" />
                            <span class="text-xs font-bold text-base-content/60 leading-tight">
                                {{ crumb.text }}
                            </span>
                        </button>
                        <div v-else class="flex items-center">
                            <i class="bi-folder-fill me-1" />
                            <span class="text-xs font-bold text-base-content/60 leading-tight">
                                {{ crumb.text }}
                            </span>
                        </div>
                    </li>
                </ul>
            </nav>
        </div>

        <div class="flex-1 min-h-0">
            <VirtualList
                v-if="mediaStore.folderTree.folders.length > 0 || mediaStore.folderTree.files.length > 0"
                id="mediaList"
                v-model="mediaStore.folderTree.files"
                class="w-full h-full"
                :group="dragGroup"
                :data-key="'name'"
                :handle="'.handle'"
                :sortable="false"
                ghost-class="sortable-ghost"
                placeholder-class="media-placeholder"
                chosen-class="cursor-grabbing"
                wrap-tag="ul"
                wrap-class="relative list-none"
                :animation="50"
            >
                <template #header>
                    <ul class="border-b border-base-content/20 list-none">
                        <li
                            v-for="folder in mediaStore.folderTree.folders"
                            :key="folder.uid"
                            class="grid grid-cols-[30px_auto_24px] even:bg-base-200 py-1 items-center cursor-pointer hover:bg-base-100/60"
                            @click="openFolder(folder.name)"
                        >
                            <div class="px-2">
                                <i class="bi-folder-fill" />
                            </div>
                            <div class="truncate pe-1">
                                {{ folder.name }}
                            </div>
                            <button
                                class="opacity-30 hover:opacity-100 cursor-pointer"
                                :title="mediaStore.isPreferred(folderPath(folder.name)) ? t('media.unpinFolder') : t('media.pinFolder')"
                                @click.stop="togglePin(folder.name)"
                            >
                                <i
                                    :class="
                                        mediaStore.isPreferred(folderPath(folder.name))
                                            ? 'bi-star-fill text-warning'
                                            : 'bi-star'
                                    "
                                />
                            </button>
                        </li>
                    </ul>
                </template>
                <template #item="{ item }">
                    <li
                        :key="item.name"
                        class="grid grid-cols-[30px_auto_32px_62px] odd:bg-base-200 border-b border-base-content/20 py-1 items-center"
                    >
                        <div class="px-2">
                            <i v-if="mediaType(item.name) === 'audio'" class="bi-music-note-beamed" />
                            <i v-else-if="mediaType(item.name) === 'video'" class="bi-film" />
                            <i v-else-if="mediaType(item.name) === 'image'" class="bi-file-earmark-image" />
                            <i v-else class="bi-file-binary" />
                        </div>
                        <div class="truncate handle cursor-grab">
                            {{ item.name }}
                        </div>
                        <div class="text-center leading-3">
                            <button class="cursor-pointer" @click="preview?.(item.name)">
                                <i class="bi-play-fill" />
                            </button>
                        </div>
                        <div class="text-nowrap px-1">
                            {{ secToHMS(item.duration) }}
                        </div>
                    </li>
                </template>
            </VirtualList>
        </div>
    </div>
</template>
