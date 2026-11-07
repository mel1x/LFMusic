import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { openUrl } from '@tauri-apps/plugin-opener'
import { isWindowsDesktop } from './usePlatform'

// Updates come from the latest GitHub release (Windows only, see src-tauri/src/updater.rs).
export const RELEASES_URL = 'https://github.com/mel1x/LFMusic/releases/latest'

interface UpdateCheck {
    supported: boolean
    portable: boolean
    current: string
    version: string | null
}

export type UpdateStatus = 'idle' | 'checking' | 'latest' | 'downloading' | 'error'

const supported = ref(isWindowsDesktop())
const portable = ref(false)
const currentVersion = ref('')
const availableVersion = ref<string | null>(null)
const status = ref<UpdateStatus>('idle')
const progress = ref(0)

let progressListening = false

async function check() {
    if (!supported.value || status.value === 'checking' || status.value === 'downloading') return
    status.value = 'checking'
    try {
        const r = await invoke<UpdateCheck>('update_check')
        supported.value = r.supported
        portable.value = r.portable
        currentVersion.value = r.current
        availableVersion.value = r.version
        status.value = r.version ? 'idle' : 'latest'
    } catch {
        status.value = 'error'
    }
}

// Installed builds download and run the installer (it closes the app);
// the portable exe opens the release page instead.
async function install(beforeInstall?: () => Promise<void>) {
    if (!availableVersion.value || status.value === 'downloading') return
    if (portable.value) {
        await openUrl(RELEASES_URL).catch(() => window.open(RELEASES_URL, '_blank', 'noopener'))
        return
    }
    if (!progressListening) {
        progressListening = true
        await listen<number>('update-progress', e => { progress.value = e.payload })
    }
    progress.value = 0
    status.value = 'downloading'
    try {
        await beforeInstall?.()
        await invoke('update_install')
    } catch {
        status.value = 'error'
    }
}

export function useUpdater() {
    return { supported, portable, currentVersion, availableVersion, status, progress, check, install }
}
