import { defineStore } from 'pinia'

import { AuthFetchError, authFetch } from '@/composables/authFetch'
import { i18n } from '@/i18n'
import { useAuth } from './auth'
import { useConfig } from './config'
import { useIndex } from './index'

import { playlistOperations } from '../composables/helper'

const { genUID } = playlistOperations()

export const useMedia = defineStore('media', {
    state: () => ({
        currentPath: '',
        crumbs: [] as Crumb[],
        folderTree: {
            source: '',
            parent: '',
            parent_folders: [],
            folders: [],
            files: [],
        } as FileFolderObject,
        folderList: {
            source: '',
            parent: '',
            folders: [],
        } as FolderObject,
        folderCrumbs: [] as Crumb[],
        preferredFolders: [] as string[],
        isLoading: false,
        treeVersion: 0,
        treePath: '',
        treeNavVersion: 0,
        expandedFolders: [] as string[],
    }),

    getters: {
        isPreferred: (state) => {
            return (path: string) => state.preferredFolders.includes(path)
        },
        isTreeExpanded: (state) => {
            return (path: string) => state.expandedFolders.includes(path)
        },
    },
    actions: {
        resetTree() {
            this.treePath = ''
            this.expandedFolders = []
        },

        setTreeExpanded(path: string, value: boolean) {
            if (value) {
                if (!this.expandedFolders.includes(path)) {
                    this.expandedFolders.push(path)
                }
            } else {
                this.expandedFolders = this.expandedFolders.filter((folder) => folder !== path)
            }
        },

        // Expand the whole chain from the tree root down to the given folder,
        // so the compact tree can reveal it even when its ancestors were
        // collapsed and not loaded yet.
        revealTreePath(path: string) {
            this.treePath = path
            this.treeNavVersion += 1

            const chain: string[] = ['']
            let current = ''

            for (const segment of path.split('/').filter(Boolean)) {
                current += `/${segment}`
                chain.push(current)
            }

            this.expandedFolders = Array.from(new Set([...this.expandedFolders, ...chain]))
        },

        loadPreferredFolders() {
            const authStore = useAuth()
            const configStore = useConfig()
            const id = configStore.channels[configStore.i]?.id

            authFetch<string[]>(`/api/file/${id}/favorites`, {
                headers: { ...authStore.authHeader },
            })
                .then((paths) => {
                    this.preferredFolders = Array.isArray(paths) ? paths : []
                })
                .catch(() => {
                    // Older backend without favorites support: fall back to
                    // browser storage.
                    try {
                        const raw = localStorage.getItem(`ffplayout_preferred_folders_${id}`)

                        this.preferredFolders = raw ? JSON.parse(raw) : []
                    } catch {
                        this.preferredFolders = []
                    }
                })
        },

        savePreferredFolders() {
            const configStore = useConfig()
            const id = configStore.channels[configStore.i]?.id

            localStorage.setItem(
                `ffplayout_preferred_folders_${id}`,
                JSON.stringify(this.preferredFolders),
            )
        },

        async togglePreferredFolder(path: string) {
            const authStore = useAuth()
            const configStore = useConfig()
            const id = configStore.channels[configStore.i]?.id
            const wasPinned = this.preferredFolders.includes(path)

            if (wasPinned) {
                this.preferredFolders = this.preferredFolders.filter((p) => p !== path)
            } else {
                this.preferredFolders.push(path)
            }

            const request = wasPinned
                ? { method: 'DELETE' as const }
                : { method: 'POST' as const }

            await authFetch<void>(`/api/file/${id}/favorites`, {
                ...request,
                headers: { ...configStore.contentType, ...authStore.authHeader },
                body: JSON.stringify({ path }),
            }).catch(() => {
                // Keep pins working against older backends by persisting
                // them in browser storage.
                this.savePreferredFolders()
            })
        },

        async moveItem(source: string, targetFolder: string, name: string) {
            const authStore = useAuth()
            const configStore = useConfig()
            const indexStore = useIndex()
            const id = configStore.channels[configStore.i]?.id
            const target = `${targetFolder}/${name}`.replace(/\/[/]+/g, '/')

            if (source === target) {
                return
            }

            await authFetch<void>(`/api/file/${id}/rename`, {
                method: 'POST',
                headers: { ...configStore.contentType, ...authStore.authHeader },
                body: JSON.stringify({ source, target }),
            })
                .then(() => {
                    this.treeVersion += 1
                    this.getTree(this.currentPath)
                })
                .catch((e) => {
                    indexStore.msgAlert('error', `${i18n.t('media.moveError')}: ${e}`, 3)
                })
        },

        async getTree(path: string, foldersOnly: boolean = false) {
            if (!foldersOnly) {
                this.isLoading = true
                this.loadPreferredFolders()
            }

            const authStore = useAuth()
            const configStore = useConfig()
            const indexStore = useIndex()
            const id = configStore.channels[configStore.i]?.id
            const crumbs: Crumb[] = []
            let root = '/'

            await authFetch<any>(`/api/file/${id}/browse`, {
                method: 'POST',
                headers: { ...configStore.contentType, ...authStore.authHeader },
                body: JSON.stringify({ source: path, folders_only: foldersOnly }),
            })
                .then((data) => {
                    const pathStr = `${data.parent}/` + data.source
                    const pathArr = pathStr.split('/')

                    if (path && path !== '/') {
                        for (const crumb of pathArr) {
                            if (crumb === data.parent) {
                                crumbs.push({ text: crumb, path: root })
                            } else if (crumb) {
                                root += crumb + '/'
                                crumbs.push({ text: crumb, path: root })
                            }
                        }
                    } else {
                        crumbs.push({ text: data.parent, path: '' })
                    }

                    if (foldersOnly) {
                        this.folderCrumbs = crumbs
                        data.parent_folders = data.parent_folders?.map((i: any) => ({ uid: genUID(), name: i })) ?? []
                        data.folders = Array.isArray(data.folders)
                            ? data.folders.map((i: any) => ({ uid: genUID(), name: i }))
                            : []
                        data.files = Array.isArray(data.files) ? data.files : []
                        this.folderList = data
                    } else {
                        this.currentPath = path
                        this.crumbs = crumbs
                        data.parent_folders = data.parent_folders?.map((i: any) => ({ uid: genUID(), name: i })) ?? []
                        data.folders = Array.isArray(data.folders)
                            ? data.folders.map((i: any) => ({ uid: genUID(), name: i }))
                            : []
                        data.files = Array.isArray(data.files) ? data.files : []
                        this.folderTree = data
                    }
                })
                .catch((error) => {
                    const message =
                        error instanceof AuthFetchError && error.message
                            ? error.message
                            : i18n.t('media.notExists')

                    indexStore.msgAlert('error', message, 3)
                })

            this.isLoading = false
        },
    },
})
