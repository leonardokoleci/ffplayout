<script setup lang="ts">
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { storeToRefs } from 'pinia'

import { useConfig } from '@/stores/config'
import { useMedia } from '@/stores/media'

import MediaTree from '@/components/media/MediaTree.vue'
import MediaBrowseList from '@/components/media/MediaBrowseList.vue'

const { t } = useI18n()

defineProps<{
    preview?: (path: string) => Promise<void> | void
}>()

const mediaStore = useMedia()
const { i } = storeToRefs(useConfig())

// Pane visibility is a user choice and survives reloads; both views can be
// hidden, leaving only the quick access list.
const showTree = ref(localStorage.getItem('ffplayout_media_browser_tree') !== '0')
const showList = ref(localStorage.getItem('ffplayout_media_browser_list') !== '0')

function toggleTree() {
    showTree.value = !showTree.value
    localStorage.setItem('ffplayout_media_browser_tree', showTree.value ? '1' : '0')
}

function toggleList() {
    showList.value = !showList.value
    localStorage.setItem('ffplayout_media_browser_list', showList.value ? '1' : '0')
}

// The browse list follows the folder selected in the tree: every tree path
// change (folder click, quick access, breadcrumb) loads its listing.
watch(
    [showList, () => mediaStore.treePath],
    ([list, path]) => {
        if (list) {
            mediaStore.getTree(path)
        }
    },
    { immediate: true },
)

watch([i], () => {
    mediaStore.getTree(mediaStore.treePath)
})
</script>

<template>
    <div class="h-full flex flex-col relative bg-base-300">
        <div v-if="mediaStore.isLoading" class="h-full w-full absolute z-10 flex justify-center bg-base-100/70">
            <span class="loading loading-spinner loading-lg" />
        </div>

        <div class="shrink-0 flex items-center justify-end gap-1 px-1 pt-1">
            <button
                class="btn btn-xs"
                :class="showTree ? 'btn-primary' : 'btn-ghost'"
                :title="t('media.folderTree')"
                @click="toggleTree"
            >
                <i class="bi-diagram-3" />
            </button>
            <button
                class="btn btn-xs"
                :class="showList ? 'btn-primary' : 'btn-ghost'"
                :title="t('media.folderList')"
                @click="toggleList"
            >
                <i class="bi-list-ul" />
            </button>
        </div>

        <div class="flex-1 min-h-0 flex flex-col">
            <!-- Quick access is always rendered by MediaTree, only the tree part is toggleable -->
            <MediaTree
                compact
                :show-tree="showTree"
                :preview="preview"
                :class="showTree ? 'flex-1 min-h-0' : 'shrink-0'"
            />
            <MediaBrowseList v-if="showList" :preview="preview" class="flex-1 min-h-0" />
        </div>
    </div>
</template>
