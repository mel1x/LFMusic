import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { stopDevCapture } from './useDevTools'

export const STATIC_ACCENT_COLOR_DEFAULT = '#3D3D3D'
export const ARTIST_SEPARATORS_DEFAULT: string[] = ['; ', ', ']
export const PLAYER_EXTRA_BLOCK_MODE_DEFAULT: PlayerExtraBlockMode = 'visualizer'

export type Language = 'ru' | 'en'
export const LANGUAGE_DEFAULT: Language = 'en'
export type PlayerExtraBlockMode = 'visualizer' | 'next_track'

function detectInitialLanguage(): Language {
    const browserLanguage = globalThis.navigator?.languages?.[0] ?? globalThis.navigator?.language ?? ''
    return browserLanguage.toLowerCase().startsWith('ru') ? 'ru' : LANGUAGE_DEFAULT
}

export const settingItems = [
    { key: 'crossfade_on_seek',        label: 'Плавная перемотка трека',                    default: true,  description: 'При включении звучание трека будет плавно переходить при перемотке',                          warning: '' },
    { key: 'crossfade_between_tracks', label: 'Плавный переход между треками',              default: true,  description: 'При включении звучание треков будет плавно переходить при переключении',                       warning: '' },
    { key: 'show_player_extra_block',  label: 'Отображение дополнительного блока у плеера', default: false, description: 'Переключает отображение дополнительного блока слева от плеера',                                warning: 'Увеличивает потребление CPU примерно в x1.5-x4 раза' },
    { key: 'show_github_badge',        label: 'Бейдж github в шапке',                       default: true,  description: 'Переключает отображение кнопки github в шапке приложения',                                     warning: '' },
    { key: 'discord_rpc_enabled',      label: 'Отображение в статусе discord',              default: true,  description: 'При включении в вашем discord профиле будет отображаться информация о прослушивании',          warning: '' },
    { key: 'dynamic_accent_color',     label: 'Динамическая цветовая тема',                 default: true,  description: 'Цвета интерфейса автоматически будут подстраиваться под обложку текущего трека',               warning: '' },
    { key: 'external_album_order',     label: 'Получать порядок треков в альбоме',          default: false, description: 'Расставляет треки локальных альбомов в правильном порядке из внешних источников',               warning: 'Требует подключения к интернету' },
    { key: 'external_supplement_missing', label: 'Дополнять плейлисты недостающими треками', default: false, description: 'Показывает в альбомах отсутствующие в библиотеке треки полупрозрачными (не воспроизводятся и не идут в очередь)', warning: 'Требует подключения к интернету' },
    { key: 'external_track_covers',    label: 'Искать обложки для треков без обложки',      default: false, description: 'Если у трека нет встроенной обложки - подставляет обложку альбома из внешних источников',         warning: 'Требует подключения к интернету' },
    { key: 'external_artist_avatars',  label: 'Получать аватарки исполнителей',             default: false, description: 'Подставляет аватарки исполнителей из внешних открытых источников',                              warning: 'Требует подключения к интернету' },
    { key: 'holiday_events',           label: 'Праздничное оформление',                     default: true,  description: 'В праздники добавляет тематический фон, плашку в шапке и снег на Новый год',               warning: '' },
    { key: 'show_downloads_page',      label: 'Страница «Скачивание» в меню',               default: false,  description: 'Показывать раздел «Скачивание» в боковом меню (только на ПК)',                                  warning: '' },
    { key: 'auto_update_check',        label: 'Проверять обновления при запуске',           default: true,  description: 'При запуске проверяет новую версию на GitHub и показывает кнопку обновления в шапке',            warning: '' },
    { key: 'dev_mode',                label: 'Режим разработчика',                         default: false, description: 'Добавляет справа снизу кнопку панели разработчика: вызовы бэкенда, ошибки, метрики',           warning: 'Применяется после перезапуска приложения' },
] as const

export type SettingKey = typeof settingItems[number]['key']
export type SettingItem = typeof settingItems[number]
export type AppSettings = { [K in SettingKey]: boolean } & {
    static_accent_color: string
    artist_separators: string[]
    language: Language
    player_extra_block_mode: PlayerExtraBlockMode
}

const boolDefaults = Object.fromEntries(
    settingItems.map(s => [s.key, s.default])
) as { [K in SettingKey]: boolean }

const defaults: AppSettings = {
    ...boolDefaults,
    static_accent_color: STATIC_ACCENT_COLOR_DEFAULT,
    artist_separators: [...ARTIST_SEPARATORS_DEFAULT],
    language: detectInitialLanguage(),
    player_extra_block_mode: PLAYER_EXTRA_BLOCK_MODE_DEFAULT,
}

const settings = ref<AppSettings>({ ...defaults, artist_separators: [...ARTIST_SEPARATORS_DEFAULT] })

let initialized = false

// Dev mode as read at startup: toggling it applies only after a restart.
const devModeActive = ref(false)

type RawSettings = Record<string, boolean | string | string[]>

export function useSettings() {
    async function load() {
        if (initialized) return

        const saved = await invoke<RawSettings>('get_settings')
        const knownKeys = settingItems.map(s => s.key as string)
        knownKeys.push('static_accent_color', 'artist_separators', 'language', 'player_extra_block_mode')

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

        const lang = saved.language
        next.language = lang === 'en' || lang === 'ru' ? lang : detectInitialLanguage()

        const extraBlockMode = saved.player_extra_block_mode
        next.player_extra_block_mode = extraBlockMode === 'visualizer' || extraBlockMode === 'next_track'
            ? extraBlockMode
            : PLAYER_EXTRA_BLOCK_MODE_DEFAULT

        const hasNew     = knownKeys.some(k => !(k in saved))
        const hasRemoved = Object.keys(saved).some(k => !knownKeys.includes(k))

        settings.value = next
        devModeActive.value = next.dev_mode
        if (!next.dev_mode) stopDevCapture()
        initialized = true

        if (hasNew || hasRemoved) await save()
    }

    async function save() {
        await invoke('save_settings', { settings: settings.value })
    }

    return { settings, settingItems, devModeActive, load, save }
}
