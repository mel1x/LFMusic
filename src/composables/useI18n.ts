import { computed, watch } from 'vue'
import ru from '../locales/ru.json'
import en from '../locales/en.json'
import { useSettings, type Language } from './useSettings'

type Params = Record<string, string | number>
type LocaleTree = Record<string, unknown>

const locales: Record<Language, LocaleTree> = { ru, en }
const sourceMap = (en as { source?: Record<string, string> }).source ?? {}
const normalizedSourceMap = Object.fromEntries(
    Object.entries(sourceMap).map(([key, value]) => [normalizeSpaces(key), value])
)
const enTextCache = new Map<string, string>()

function normalizeSpaces(value: string): string {
    return value.trim().replace(/\s+/g, ' ')
}

function readPath(source: LocaleTree, path: string): string | undefined {
    const value = path.split('.').reduce<unknown>((acc, part) => {
        if (acc && typeof acc === 'object' && part in acc) {
            return (acc as Record<string, unknown>)[part]
        }
        return undefined
    }, source)
    return typeof value === 'string' ? value : undefined
}

function interpolate(value: string, params?: Params): string {
    if (!params) return value
    return value.replace(/\{(\w+)\}/g, (_, key) => String(params[key] ?? ''))
}

function withOriginalSpacing(original: string, translated: string): string {
    const start = original.match(/^\s*/)?.[0] ?? ''
    const end = original.match(/\s*$/)?.[0] ?? ''
    return `${start}${translated}${end}`
}

function pluralEn(n: number, one: string, many: string): string {
    return Math.abs(n) === 1 ? one : many
}

function translatePlaylistName(name: string): string {
    const match = name.trim().match(/^Новый плейлист\s+(\d+)$/i)
    if (match) return `New Playlist ${match[1]}`
    return name
}

function translatePattern(trimmed: string): string | null {
    let match = trimmed.match(/^(\d+)\s+(?:трек|трека|треков|Треков)$/i)
    if (match) {
        const n = Number(match[1])
        return `${n} ${pluralEn(n, 'track', 'tracks')}`
    }

    match = trimmed.match(/^(\d+)\s+(?:альбом|альбома|альбомов)$/i)
    if (match) {
        const n = Number(match[1])
        return `${n} ${pluralEn(n, 'album', 'albums')}`
    }

    match = trimmed.match(/^(\d+)\s+раз прослушано$/i)
    if (match) {
        const n = Number(match[1])
        return `listened ${n} ${pluralEn(n, 'time', 'times')}`
    }

    match = trimmed.match(/^(\d+)\s*\/\s*(\d+)\s+файлов$/i)
    if (match) return `${match[1]} / ${match[2]} files`

    match = trimmed.match(/^(\d+)\s+файлов$/i)
    if (match) {
        const n = Number(match[1])
        return `${n} ${pluralEn(n, 'file', 'files')}`
    }

    match = trimmed.match(/^(\d+)\s+каналов$/i)
    if (match) {
        const n = Number(match[1])
        return `${n} ${pluralEn(n, 'channel', 'channels')}`
    }

    match = trimmed.match(/^(\d+)\s+бит$/i)
    if (match) return `${match[1]} bit`

    match = trimmed.match(/^(.+)\s+кГц$/i)
    if (match) return `${match[1]} kHz`

    match = trimmed.match(/^(.+)\s+кбит\/с$/i)
    if (match) return `${match[1]} kbps`

    match = trimmed.match(/^Новый плейлист\s+(\d+)$/i)
    if (match) return `New Playlist ${match[1]}`

    match = trimmed.match(/^Плейлист\s+(.+)\s+будет удалён без возможности восстановления$/i)
    if (match) return `Playlist ${translatePlaylistName(match[1])} will be permanently deleted`

    match = trimmed.match(/^Плейлист «(.+)» будет удалён без возможности восстановления\.$/i)
    if (match) return `Playlist "${translatePlaylistName(match[1])}" will be permanently deleted.`

    return null
}

export function translateText(text: string): string {
    const { settings } = useSettings()
    if (settings.value.language !== 'en') return text

    const cached = enTextCache.get(text)
    if (cached !== undefined) return cached
    const finish = (value: string) => {
        if (value !== text) enTextCache.set(text, value)
        return value
    }

    const trimmed = text.trim()
    if (!trimmed) return finish(text)

    const exact = sourceMap[trimmed]
    if (exact) return finish(withOriginalSpacing(text, exact))

    const normalized = normalizedSourceMap[normalizeSpaces(trimmed)]
    if (normalized) return finish(withOriginalSpacing(text, normalized))

    const pattern = translatePattern(trimmed)
    if (pattern) return finish(withOriginalSpacing(text, pattern))

    if (text.includes('\n')) {
        const translated = text.split('\n').map((line) => {
            if (!line.trim()) return line
            const lineTrimmed = line.trim()
            const lineExact = sourceMap[lineTrimmed] ?? normalizedSourceMap[normalizeSpaces(lineTrimmed)]
            if (lineExact) return withOriginalSpacing(line, lineExact)
            const linePattern = translatePattern(lineTrimmed)
            return linePattern ? withOriginalSpacing(line, linePattern) : line
        }).join('\n')
        if (translated !== text) return finish(translated)
    }

    return finish(text)
}

export function useI18n() {
    const { settings, save } = useSettings()
    const language = computed(() => settings.value.language)

    function t(key: string, params?: Params): string {
        const value = readPath(locales[language.value], key) ?? readPath(ru, key) ?? key
        return interpolate(value, params)
    }

    async function setLanguage(next: Language) {
        if (settings.value.language === next) return
        settings.value.language = next
        await save()
    }

    watch(language, (next) => {
        document.documentElement.lang = next
    }, { immediate: true })

    return { language, t, setLanguage, translateText }
}
