import { useSettings } from '../composables/useSettings'

export function pluralizeRu(n: number, forms: [string, string, string]): string {
    const abs = Math.abs(n) % 100
    const rem = abs % 10
    if (abs > 10 && abs < 20) return forms[2]
    if (rem > 1 && rem < 5) return forms[1]
    if (rem === 1) return forms[0]
    return forms[2]
}

function isEnglish(): boolean {
    return useSettings().settings.value.language === 'en'
}

function pluralizeEn(n: number, one: string, many: string): string {
    return Math.abs(n) === 1 ? one : many
}

export function tracksCountText(n: number): string {
    if (isEnglish()) return `${n} ${pluralizeEn(n, 'track', 'tracks')}`
    return `${n} ${pluralizeRu(n, ['трек', 'трека', 'треков'])}`
}

export function albumsCountText(n: number): string {
    if (isEnglish()) return `${n} ${pluralizeEn(n, 'album', 'albums')}`
    return `${n} ${pluralizeRu(n, ['альбом', 'альбома', 'альбомов'])}`
}

export function listenCountText(n: number): string {
    if (isEnglish()) return `${n} ${pluralizeEn(n, 'listen', 'listens')}`
    return `${n} ${pluralizeRu(n, ['прослушивание', 'прослушивания', 'прослушиваний'])}`
}

export function timesCountText(n: number): string {
    if (isEnglish()) return `${n} ${pluralizeEn(n, 'time', 'times')}`
    return `${n} ${pluralizeRu(n, ['раз', 'раза', 'раз'])}`
}

export type ArtistPart = { type: 'artist' | 'sep'; text: string }

function escapeRegExp(s: string): string {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

export function splitArtistsWithSeparators(input: string, separators: string[]): ArtistPart[] {
    const raw = (input || '').trim()
    if (!raw) return []
    const valid = (separators || []).filter(s => typeof s === 'string' && s.length > 0)
    if (valid.length === 0) return [{ type: 'artist', text: raw }]
    const sorted = [...valid].sort((a, b) => b.length - a.length)
    const re = new RegExp(`(${sorted.map(escapeRegExp).join('|')})`)
    const parts = raw.split(re)
    const result: ArtistPart[] = []
    for (let i = 0; i < parts.length; i++) {
        const text = parts[i]
        if (i % 2 === 0) {
            const trimmed = text.trim()
            if (trimmed) result.push({ type: 'artist', text: trimmed })
        } else {
            result.push({ type: 'sep', text })
        }
    }
    return result
}

export function splitArtistsList(input: string, separators: string[]): string[] {
    return splitArtistsWithSeparators(input, separators)
        .filter(p => p.type === 'artist')
        .map(p => p.text)
}

export function formatTotalDuration(secs: number): string {
    const total = Math.max(0, Math.floor(secs))
    const h = Math.floor(total / 3600)
    const m = Math.floor((total % 3600) / 60)
    if (isEnglish()) {
        if (h > 0) return `${h} ${pluralizeEn(h, 'hour', 'hours')} ${m} min`
        return `${m} min`
    }
    if (h > 0) {
        return `${h} ${pluralizeRu(h, ['час', 'часа', 'часов'])} ${m} мин`
    }
    return `${m} мин`
}
