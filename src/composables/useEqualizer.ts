import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export interface EqSettings {
    enabled: boolean
    bands: number[]
    gain: number
}

export interface EqPreset {
    name: string
    bands: number[]
}

export const EQ_BAND_LABELS = ['60', '170', '310', '600', '1K', '3K', '6K', '12K', '14K', '16K']

export const EQ_PRESETS: EqPreset[] = [
    {
        name: 'Flat',
        bands: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    },

    {
        name: 'Bass Boost',
        bands: [4.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    },
    {
        name: 'Deep Bass',
        bands: [5.0, 3.5, 1.0, -0.5, -0.5, 0.0, 1.0, 1.5, 2.0, 2.0]
    },
    {
        name: 'Sub Bass',
        bands: [4.5, 2.0, 0.0, -0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    },

    {
        name: 'Treble Boost',
        bands: [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.5, 3.0]
    },
    {
        name: 'Air & Sparkle',
        bands: [-1.0, 0.0, 0.5, 1.0, 1.0, 1.5, 2.0, 3.0, 4.0, 4.5]
    },

    {
        name: 'Modern V-Shape',
        bands: [4.0, 2.5, 0.0, -1.5, -1.5, 0.0, 2.0, 3.5, 4.0, 4.0]
    },
    {
        name: 'Electronic',
        bands: [4.5, 3.0, 0.5, -1.0, -1.0, 0.5, 2.0, 3.5, 4.0, 4.5]
    },
    {
        name: 'Bass & Treble',
        bands: [3.5, 2.5, 0.5, 0.0, 0.0, 0.0, 1.0, 2.5, 3.0, 3.0]
    },

    {
        name: 'Rock',
        bands: [3.5, 2.0, -1.0, -1.5, 1.0, 2.5, 3.5, 4.0, 3.5, 2.5]
    },
    {
        name: 'Metal',
        bands: [3.0, 1.5, -0.5, 1.0, 2.5, 4.0, 4.5, 4.0, 3.5, 2.5]
    },
    {
        name: 'Jazz',
        bands: [2.5, 2.0, 1.5, 1.0, 0.0, 1.0, 2.0, 2.5, 2.0, 1.5]
    },
    {
        name: 'Classical',
        bands: [2.0, 1.0, 0.5, 0.0, 0.0, 1.0, 1.5, 2.5, 3.0, 3.5]
    },
    {
        name: 'Acoustic',
        bands: [1.5, 1.5, 1.0, 0.5, 0.0, 1.5, 2.5, 3.5, 3.5, 3.0]
    },
    {
        name: 'Hip-Hop',
        bands: [5.0, 3.5, 1.0, -0.5, 0.5, 2.0, 2.0, 2.0, 2.5, 2.5]
    },

    {
        name: 'Vocal',
        bands: [-2.0, -1.0, 0.5, 2.0, 3.5, 2.5, 1.0, 0.0, 0.0, -1.0]
    },
    {
        name: 'Podcast',
        bands: [-3.0, -1.5, 1.0, 2.5, 4.0, 3.0, 1.5, 0.0, -1.0, -2.0]
    },

    {
        name: 'Loudness',
        bands: [3.5, 2.5, 0.5, -0.5, -0.5, 0.0, 1.5, 2.5, 3.0, 3.5]
    },
    {
        name: 'Night Mode',
        bands: [-2.5, -1.5, -0.5, 0.5, 1.0, 0.5, 0.0, -1.0, -1.5, -2.0]
    },
    {
        name: 'Chill',
        bands: [2.5, 3.0, 2.0, 0.5, -0.5, -1.0, -2.0, -3.0, -3.5, -3.5]
    },
    {
        name: 'Cinematic',
        bands: [4.5, 2.5, 0.0, -1.0, 0.0, 1.5, 2.5, 3.5, 4.0, 4.0]
    },

    {
        name: 'Bass Reduction',
        bands: [-2.0, -1.0, 0.5, 1.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.0]
    },
    {
        name: 'Harshness Fix',
        bands: [1.0, 1.0, 1.5, 1.5, 1.0, -1.5, -2.5, -1.0, 0.0, 0.5]
    },
    {
        name: 'Muddy Fix',
        bands: [0.0, -1.5, -2.0, -1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 0.0]
    },
];

const eqEnabled = ref(false)
const eqBands = ref<number[]>([0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
const eqGain = ref(0)
const activePreset = ref<string | null>('Flat')
let initialized = false

function findMatchingPreset(bands: number[]): string | null {
    for (const preset of EQ_PRESETS) {
        if (preset.bands.every((v, i) => Math.abs(v - (bands[i] ?? 0)) < 0.1)) {
            return preset.name
        }
    }
    return null
}

export function useEqualizer() {
    async function load() {
        if (initialized) return
        try {
            const settings = await invoke<EqSettings>('eq_get_settings')
            eqEnabled.value = settings.enabled
            eqBands.value = settings.bands.length === 10 ? settings.bands : [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
            eqGain.value = settings.gain ?? 0
            activePreset.value = findMatchingPreset(eqBands.value)
            initialized = true
        } catch (e) {
            console.error('Ошибка загрузки эквалайзера:', e)
        }
    }

    async function toggleEnabled() {
        eqEnabled.value = !eqEnabled.value
        try {
            await invoke('eq_set_enabled', { enabled: eqEnabled.value })
        } catch (e) {
            console.error('Ошибка переключения эквалайзера:', e)
        }
    }

    async function setBand(index: number, value: number) {
        eqBands.value[index] = value
        activePreset.value = findMatchingPreset(eqBands.value)
        try {
            await invoke('eq_set_bands', { bands: [...eqBands.value] })
        } catch (e) {
            console.error('Ошибка установки полосы эквалайзера:', e)
        }
    }

    async function applyPreset(preset: EqPreset) {
        eqBands.value = [...preset.bands]
        activePreset.value = preset.name
        try {
            await invoke('eq_set_bands', { bands: [...preset.bands] })
        } catch (e) {
            console.error('Ошибка применения пресета:', e)
        }
    }

    async function setGain(value: number) {
        eqGain.value = value
        try {
            await invoke('eq_set_gain', { gain: value })
        } catch (e) {
            console.error('Ошибка установки уровня эквалайзера:', e)
        }
    }

    async function resetBands() {
        eqBands.value = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        eqGain.value = 0
        activePreset.value = 'Flat'
        try {
            await invoke('eq_set_bands', { bands: eqBands.value })
            await invoke('eq_set_gain', { gain: 0 })
        } catch (e) {
            console.error('Ошибка сброса эквалайзера:', e)
        }
    }

    return {
        eqEnabled,
        eqBands,
        eqGain,
        activePreset,
        EQ_BAND_LABELS,
        EQ_PRESETS,
        load,
        toggleEnabled,
        setBand,
        setGain,
        applyPreset,
        resetBands,
    }
}
