import { defineStore } from 'pinia'

import { authFetch } from '@/composables/authFetch'
import { useAuth } from '@/stores/auth'
import { useIndex } from '@/stores/index'
import { useConfig } from '@/stores/config'

export type RundownItemView = RundownItem & { uid: string }

/**
 * Continuous rundown queue ("rundown" processing mode): ordered media items
 * played back-to-back without calendar dates. All mutations go through the
 * API immediately; the engine picks changes up at the next clip boundary.
 */
export const useRundown = defineStore('rundown', {
    state: () => ({
        items: [] as RundownItemView[],
        revision: 0,
        headItemId: null as number | null,
        isLoading: false,
    }),

    getters: {
        headIndex(state): number {
            return state.items.findIndex((item) => item.id === state.headItemId)
        },
    },

    actions: {
        async fetchQueue() {
            const authStore = useAuth()
            const channel = useConfig().channels[useConfig().i]?.id
            if (!channel) return

            try {
                const data = await authFetch<RundownQueue>(`/api/rundown/${channel}`, {
                    method: 'GET',
                    headers: authStore.authHeader,
                })

                this.revision = data.revision
                this.headItemId = data.head_item_id
                this.items = data.items.map((item) => ({ ...item, uid: String(item.id) }))
            } catch (error) {
                useIndex().msgAlert(
                    'error',
                    error instanceof Error ? error.message : String(error),
                    4,
                )
            }
        },

        /** Called from the SSE status stream; refetches when the engine or an
         * editor changed the queue. */
        async onStatus(revision?: number, head?: number | null) {
            if (revision === undefined) return

            if (head !== undefined && head !== null) this.headItemId = head

            if (revision !== this.revision) {
                await this.fetchQueue()
            }
        },

        /** Probe a source against the requested trim values and get the
         * validated seek/out/duration the queue would store. */
        async probeSource(payload: { source: string; seek?: number; out?: number; duration?: number }) {
            return await authFetch<RundownTrim>(
                `/api/rundown/${useConfig().channels[useConfig().i]?.id}/probe`,
                {
                    method: 'POST',
                    headers: {
                        'content-type': 'application/json;charset=UTF-8',
                        ...useAuth().authHeader,
                    },
                    body: JSON.stringify(payload),
                },
            )
        },

        async addItem(payload: Partial<RundownItem> & { source: string; after_position?: number | null }) {
            await authFetch(`/api/rundown/${useConfig().channels[useConfig().i]?.id}/item`, {
                method: 'POST',
                headers: { 'content-type': 'application/json;charset=UTF-8', ...useAuth().authHeader },
                body: JSON.stringify(payload),
            })

            await this.fetchQueue()
        },

        async editItem(itemId: number, patch: Partial<Pick<RundownItem, 'in' | 'out' | 'title' | 'ad' | 'custom_filter' | 'audio'>>) {
            await authFetch(`/api/rundown/${useConfig().channels[useConfig().i]?.id}/item/${itemId}`, {
                method: 'PATCH',
                headers: { 'content-type': 'application/json;charset=UTF-8', ...useAuth().authHeader },
                body: JSON.stringify(patch),
            })

            await this.fetchQueue()
        },

        async deleteItem(itemId: number) {
            await authFetch(`/api/rundown/${useConfig().channels[useConfig().i]?.id}/item/${itemId}`, {
                method: 'DELETE',
                headers: useAuth().authHeader,
            })

            await this.fetchQueue()
        },

        async moveItem(itemId: number, afterPosition: number | null) {
            await authFetch(`/api/rundown/${useConfig().channels[useConfig().i]?.id}/move`, {
                method: 'POST',
                headers: { 'content-type': 'application/json;charset=UTF-8', ...useAuth().authHeader },
                body: JSON.stringify({ item_id: itemId, after_position: afterPosition }),
            })

            await this.fetchQueue()
        },

        async jumpTo(itemId: number) {
            await authFetch(`/api/rundown/${useConfig().channels[useConfig().i]?.id}/jump`, {
                method: 'POST',
                headers: { 'content-type': 'application/json;charset=UTF-8', ...useAuth().authHeader },
                body: JSON.stringify({ item_id: itemId }),
            })

            await this.fetchQueue()
        },
    },
})
