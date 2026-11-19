import { settingItems } from '../composables/useSettings'

export type SettingsCategoryId = 'appearance' | 'playback' | 'library' | 'external' | 'general'

export interface SettingsCategory {
    id: SettingsCategoryId
    title: string
    desc: string
    group: string
    /** Lucide-style inner SVG markup for the 24x24 icon. */
    icon: string
    /** Keys of the toggles from `settingItems` shown on this page. */
    settingKeys: string[]
    /** Other titles on the page (section headers, non-toggle settings) for search. */
    extra: string[]
}

export const settingsCategories: SettingsCategory[] = [
    {
        id: 'appearance',
        title: 'Оформление',
        desc: 'Цветовая тема, дополнительный блок плеера и бейдж GitHub',
        group: 'Интерфейс',
        icon: '<circle cx="13.5" cy="6.5" r=".5" fill="currentColor"/><circle cx="17.5" cy="10.5" r=".5" fill="currentColor"/><circle cx="8.5" cy="7.5" r=".5" fill="currentColor"/><circle cx="6.5" cy="12.5" r=".5" fill="currentColor"/><path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.926 0 1.648-.746 1.648-1.688 0-.437-.18-.835-.437-1.125-.29-.289-.438-.652-.438-1.125a1.64 1.64 0 0 1 1.668-1.668h1.996c3.051 0 5.555-2.503 5.555-5.554C21.965 6.012 17.461 2 12 2z"/>',
        settingKeys: ['show_donate_badge', 'show_github_badge', 'holiday_events', 'show_player_extra_block', 'dynamic_accent_color'],
        extra: ['Интерфейс', 'Цветовая тема'],
    },
    {
        id: 'playback',
        title: 'Воспроизведение',
        desc: 'Плавные переходы между треками и эквалайзер',
        group: 'Музыка',
        icon: '<path d="M2 10v3"/><path d="M6 6v11"/><path d="M10 3v18"/><path d="M14 8v7"/><path d="M18 5v13"/><path d="M22 10v3"/>',
        settingKeys: ['crossfade_on_seek', 'crossfade_between_tracks'],
        extra: ['Эквалайзер', 'Включить эквалайзер'],
    },
    {
        id: 'library',
        title: 'Библиотека',
        desc: 'Папки с музыкой и имена исполнителей',
        group: 'Музыка',
        icon: '<path d="m16 6 4 14"/><path d="M12 6v14"/><path d="M8 8v12"/><path d="M4 4v16"/>',
        settingKeys: [],
        extra: ['Разделители исполнителей', 'Локальные папки'],
    },
    {
        id: 'external',
        title: 'Внешние источники',
        desc: 'Порядок треков в альбомах и аватарки исполнителей из открытых сервисов',
        group: 'Музыка',
        icon: '<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>',
        settingKeys: ['external_album_order', 'external_supplement_missing', 'external_track_covers', 'external_artist_avatars'],
        extra: ['Папка для скачивания', 'Компоненты', 'Cookies', 'Прокси'],
    },
    {
        id: 'general',
        title: 'Общее',
        desc: 'Язык интерфейса и интеграции',
        group: 'Приложение',
        icon: '<rect width="20" height="14" x="2" y="3" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/>',
        settingKeys: ['discord_rpc_enabled', 'show_downloads_page', 'auto_update_check', 'dev_mode'],
        extra: ['Язык интерфейса', 'Версия'],
    },
]

export interface SettingsSearchEntry {
    category: SettingsCategoryId
    /** Visible title of the matched setting (Russian source text). */
    title: string
    /** Extra text the query may match (description). */
    detail: string
}

/** Flat list of every searchable setting, in page order. */
export const settingsSearchIndex: SettingsSearchEntry[] = settingsCategories.flatMap(c => [
    ...c.settingKeys.flatMap(k => {
        const item = settingItems.find(i => i.key === k)
        return item ? [{ category: c.id, title: item.label as string, detail: item.description as string }] : []
    }),
    ...c.extra.map(title => ({ category: c.id, title, detail: '' })),
])
