import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export const STATIC_ACCENT_COLOR_DEFAULT = '#3D3D3D'
export const ARTIST_SEPARATORS_DEFAULT: string[] = ['; ']

export const settingItems = [
    { key: 'crossfade_on_seek',        label: 'Плавная перемотка трека',                    default: true },
    { key: 'crossfade_between_tracks', label: 'Плавный переход между треками',              default: true },
    { key: 'show_player_extra_block',  label: 'Отображение дополнительного блока у плеера', default: true },
    { key: 'show_github_badge',        label: 'Бейдж github в шапке',                       default: true },
    { key: 'dynamic_accent_color',     label: 'Динамический акцентный цвет',                default: true },
] as const

export type SettingKey = typeof settingItems[number]['key']
export type SettingItem = typeof settingItems[number]
export type AppSettings = { [K in SettingKey]: boolean } & {
    static_accent_color: string
    artist_separators: string[]
}

const boolDefaults = Object.fromEntries(
    settingItems.map(s => [s.key, s.default])
) as { [K in SettingKey]: boolean }

const defaults: AppSettings = {
    ...boolDefaults,
    static_accent_color: STATIC_ACCENT_COLOR_DEFAULT,
    artist_separators: [...ARTIST_SEPARATORS_DEFAULT],
}

const settings = ref<AppSettings>({ ...defaults, artist_separators: [...ARTIST_SEPARATORS_DEFAULT] })

let initialized = false

type RawSettings = Record<string, boolean | string | string[]>

export function useSettings() {
    async function load() {
        if (initialized) return

        const saved = await invoke<RawSettings>('get_settings')
        const knownKeys = settingItems.map(s => s.key as string)
        knownKeys.push('static_accent_color', 'artist_separators')

        const next: AppSettings = { ...defaults, artist_separators: [...ARTIST_SEPARATORS_DEFAULT] }
        for (const item of settingItems) {
            const v = saved[item.key]
            if (typeof v === 'boolean') next[item.key] = v
        }
        const sac = saved.static_accent_color
        next.static_accent_color = typeof sac === 'string' && /^#[0-9a-fA-F]{6}$/.test(sac)
            ? sac
            : STATIC_ACCENT_COLOR_DEFAULT

        const seps = saved.artist_separators
        if (Array.isArray(seps)) {
            const cleaned = seps.filter((s): s is string => typeof s === 'string' && s.length > 0)
            next.artist_separators = cleaned.length > 0 ? cleaned : [...ARTIST_SEPARATORS_DEFAULT]
        }

        const hasNew     = knownKeys.some(k => !(k in saved))
        const hasRemoved = Object.keys(saved).some(k => !knownKeys.includes(k))

        settings.value = next
        initialized = true

        if (hasNew || hasRemoved) await save()
    }

    async function save() {
        await invoke('save_settings', { settings: settings.value })
    }

    return { settings, settingItems, load, save }
}
