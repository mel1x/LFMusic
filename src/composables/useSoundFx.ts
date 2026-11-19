import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export interface FxSettings {
    space: { enabled: boolean; mix: number; size: number }
    crusher: { enabled: boolean; bits: number; downsample: number }
    vinyl: { enabled: boolean; crackle: number; wear: number }
    /** Playback rate 0.5..2; pitch follows it. */
    speed: number
}

export type FxEffect = 'space' | 'crusher' | 'vinyl'

export const SPEED_MIN = 0.5
export const SPEED_MAX = 2
export const SPEED_PRESETS = [0.75, 0.9, 1, 1.1, 1.25, 1.5]

function defaults(): FxSettings {
    return {
        space: { enabled: false, mix: 0.35, size: 0.6 },
        crusher: { enabled: false, bits: 8, downsample: 3 },
        vinyl: { enabled: false, crackle: 0.5, wear: 0.4 },
        speed: 1,
    }
}

const fx = ref<FxSettings>(defaults())
let loadPromise: Promise<void> | null = null

/** Current playback rate, read by the player's smooth progress. */
export const playbackSpeed = computed(() => fx.value.speed || 1)

export function formatSpeed(v: number): string {
    return `${v.toFixed(2)}×`
}

const PUSH_MS = 50
let pushTimer: ReturnType<typeof setTimeout> | null = null
let lastPush = 0

async function pushNow() {
    pushTimer = null
    lastPush = performance.now()
    try {
        const applied = await invoke<FxSettings>('fx_set_settings', { settings: fx.value })
        if (!pushTimer) fx.value = { ...fx.value, speed: applied.speed }
    } catch (e) {
        console.error('Ошибка применения звуковых эффектов:', e)
    }
}

function schedulePush() {
    if (pushTimer) return
    const wait = Math.max(0, PUSH_MS - (performance.now() - lastPush))
    pushTimer = setTimeout(pushNow, wait)
}

export function useSoundFx() {
    function load(): Promise<void> {
        loadPromise ??= invoke<FxSettings>('fx_get_settings')
            .then(s => {
                const d = defaults()
                fx.value = {
                    space: { ...d.space, ...s.space },
                    crusher: { ...d.crusher, ...s.crusher },
                    vinyl: { ...d.vinyl, ...s.vinyl },
                    speed: s.speed || 1,
                }
            })
            .catch(e => {
                loadPromise = null
                console.error('Ошибка загрузки звуковых эффектов:', e)
            })
        return loadPromise
    }

    function update(mutate: (s: FxSettings) => void) {
        const next: FxSettings = JSON.parse(JSON.stringify(fx.value))
        mutate(next)
        fx.value = next
        schedulePush()
    }

    function toggle(effect: FxEffect) {
        update(s => { s[effect].enabled = !s[effect].enabled })
    }

    function setSpeed(v: number) {
        const clamped = Math.min(SPEED_MAX, Math.max(SPEED_MIN, Math.round(v * 100) / 100))
        update(s => { s.speed = clamped })
    }

    return { fx, load, update, toggle, setSpeed }
}
