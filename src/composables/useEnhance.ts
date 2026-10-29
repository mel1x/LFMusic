import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export type DeviceProfile = 'headphones' | 'laptop' | 'speakers' | 'tv' | 'universal'

export interface EnhanceStatus {
    enabled: boolean
    /** Forced profile, null = auto detection */
    device: DeviceProfile | null
    detected: DeviceProfile
    device_name: string | null
}

export const DEVICE_PROFILES: DeviceProfile[] = ['headphones', 'laptop', 'speakers', 'tv', 'universal']

const status = ref<EnhanceStatus>({ enabled: false, device: null, detected: 'universal', device_name: null })
let initialized = false

export function useEnhance() {
    async function load() {
        if (initialized) return
        try {
            status.value = await invoke<EnhanceStatus>('enhance_get_status')
            initialized = true
        } catch (e) {
            console.error('Failed to load sound enhancement:', e)
        }
    }

    async function toggleEnabled() {
        const enabled = !status.value.enabled
        status.value = { ...status.value, enabled }
        try {
            status.value = await invoke<EnhanceStatus>('enhance_set_enabled', { enabled })
        } catch (e) {
            console.error('Failed to toggle sound enhancement:', e)
        }
    }

    async function setDevice(device: DeviceProfile | null) {
        status.value = { ...status.value, device }
        try {
            status.value = await invoke<EnhanceStatus>('enhance_set_device', { device })
        } catch (e) {
            console.error('Failed to set enhancement device:', e)
        }
    }

    return { status, load, toggleEnabled, setDevice }
}
