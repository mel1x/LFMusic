import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { listen } from '@tauri-apps/api/event'
import { isAndroidRuntime } from './usePlatform'

export interface FolderInfo {
    path: string
    track_count: number
    exists: boolean
}

const folders = ref<FolderInfo[]>([])
const isScanning = ref(false)
const removingFolder = ref<string | null>(null)
const scanProgress = ref({ current: 0, total: 0, percent: 0 })
const showTracksReadyModal = ref(false)
const folderAddError = ref('')

let initialized = false

async function init() {
    if (initialized) return
    initialized = true

    folders.value = await invoke<FolderInfo[]>('library_get_folders')
    await listen<{ current: number; total: number; percent: number }>(
        'scan_progress',
        (event) => { scanProgress.value = event.payload }
    )
}

async function addFolder() {
    folderAddError.value = ''

    if (isAndroidRuntime()) {
        isScanning.value = true
        scanProgress.value = { current: 0, total: 0, percent: 0 }

        try {
            await invoke('library_add_android_music_folders')
            folders.value = await invoke<FolderInfo[]>('library_get_folders')
            return true
        } catch (e) {
            folderAddError.value = String(e)
            return false
        } finally {
            isScanning.value = false
        }
    }

    const dirPath = await open({ directory: true, multiple: false })
    if (!dirPath) return false

    isScanning.value = true
    scanProgress.value = { current: 0, total: 0, percent: 0 }

    try {
        await invoke('library_add_folder', { folderPath: dirPath as string })
        folders.value = await invoke<FolderInfo[]>('library_get_folders')
        return true
    } catch (e) {
        folderAddError.value = String(e)
        return false
    } finally {
        isScanning.value = false
    }
}

/** Перечитать список папок библиотеки (после изменений вне useFolders, напр.
 *  смены папки скачиваний на странице «Внешние источники»). */
async function refreshFolders() {
    folders.value = await invoke<FolderInfo[]>('library_get_folders')
}

async function removeFolder(folderPath: string) {
    removingFolder.value = folderPath
    try {
        await invoke('library_remove_folder', { folderPath })
        folders.value = folders.value.filter(f => f.path !== folderPath)
    } finally {
        removingFolder.value = null
    }
}

async function rescanLibrary() {
    isScanning.value = true
    scanProgress.value = { current: 0, total: 0, percent: 0 }
    folderAddError.value = ''

    try {
        await invoke('library_rescan')
        folders.value = await invoke<FolderInfo[]>('library_get_folders')
        return true
    } catch (e) {
        folderAddError.value = String(e)
        return false
    } finally {
        isScanning.value = false
    }
}

function folderDisplayName(path: string): string {
    return path.replace(/\\/g, '/')
}

function dismissTracksReadyModal() {
    showTracksReadyModal.value = false
}

function openTracksReadyModal() {
    showTracksReadyModal.value = true
}

export function useFolders() {
    return {
        folders,
        isScanning,
        removingFolder,
        scanProgress,
        showTracksReadyModal,
        folderAddError,
        init,
        addFolder,
        removeFolder,
        refreshFolders,
        rescanLibrary,
        folderDisplayName,
        dismissTracksReadyModal,
        openTracksReadyModal,
    }
}
