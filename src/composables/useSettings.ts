import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export const settingItems = [
    { key: 'crossfade_on_seek',        label: 'Плавная перемотка трека',                    default: true },
    { key: 'crossfade_between_tracks', label: 'Плавный переход между треками',              default: true },
    { key: 'show_player_extra_block',  label: 'Отображение дополнительного блока у плеера', default: true },
    { key: 'show_github_badge',        label: 'Бейдж github в шапке',                       default: true },
] as const

export type SettingKey = typeof settingItems[number]['key']
export type AppSettings = Record<SettingKey, boolean>
export type SettingItem = typeof settingItems[number]

const defaults = Object.fromEntries(
    settingItems.map(s => [s.key, s.default])
) as AppSettings

const settings = ref<AppSettings>({ ...defaults })

let initialized = false

export function useSettings() {
    async function load() {
        if (initialized) return

        const saved = await invoke<Record<string, boolean>>('get_settings')
        const knownKeys = settingItems.map(s => s.key) as string[]

        const hasNew     = knownKeys.some(k => !(k in saved))
        const hasRemoved = Object.keys(saved).some(k => !knownKeys.includes(k))

        settings.value = Object.fromEntries(
            settingItems.map(s => [s.key, s.key in saved ? saved[s.key] : s.default])
        ) as AppSettings

        initialized = true

        if (hasNew || hasRemoved) await save()
    }

    async function save() {
        await invoke('save_settings', { settings: settings.value })
    }

    return { settings, settingItems, load, save }
}
