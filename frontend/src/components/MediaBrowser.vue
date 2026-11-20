<script setup lang="ts">
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { useMedia } from '@/stores/media'

import MediaQuickAccess from '@/components/media/MediaQuickAccess.vue'
import MediaBrowseList from '@/components/media/MediaBrowseList.vue'

const { t } = useI18n()

defineProps<{
    preview?: (path: string) => Promise<void> | void
}>()

const mediaStore = useMedia()

// List pane visibility is a user choice and survives reloads.
const showList = ref(localStorage.getItem('ffplayout_media_browser_list') !== '0')

function toggleList() {
    showList.value = !showList.value
    localStorage.setItem('ffplayout_media_browser_list', showList.value ? '1' : '0')
}

// Loading the storage root listing; the quick access pane drives the
// initial and per-channel navigation.
watch(
    showList,
    (list) => {
        if (list && !mediaStore.currentPath) {
            mediaStore.getTree('')
        }
    },
    { immediate: true },
)
</script>

<template>
    <div class="h-full flex flex-col relative bg-base-300">
        <div v-if="mediaStore.isLoading" class="h-full w-full absolute z-10 flex justify-center bg-base-100/70">
            <span class="loading loading-spinner loading-lg" />
        </div>

        <div class="shrink-0 flex items-center justify-end gap-1 px-1 pt-1">
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
            <!-- Drives and quick access: content-sized, capped at half the
                 column and scrolling internally when many folders are pinned -->
            <MediaQuickAccess compact class="min-h-0 max-h-1/2 overflow-y-auto" />
            <MediaBrowseList v-if="showList" :preview="preview" class="flex-1 min-h-0" />
        </div>
    </div>
</template>
