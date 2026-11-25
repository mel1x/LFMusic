import { reactive } from 'vue'
import type { TrackMetadata } from './usePlayer'

export const mixPresets = ['auto', 'fade', 'rise', 'blend', 'wave', 'melt', 'slam', 'none'] as const
export type MixPreset = typeof mixPresets[number]
const PRESET_BARS = 8

export type VolumeCurve = 'smooth' | 'linear' | 'overlap' | 'rise' | 'cut'

export type MixEffect = 'volume' | 'eq' | 'filter'
export type MixPoint = [number, number]
export type LaneCurves = Partial<Record<MixEffect, MixPoint[]>>
export interface MixTransition {
    curves?: [LaneCurves, LaneCurves]
    preset?: MixPreset | 'custom'
    volumeCurve?: VolumeCurve
    seconds: number
    skipIntro: number
    skipOutro: number
    equalPower: boolean
    eqBand: 'off' | 'bass' | 'centerBass' | 'mids' | 'highs'
    filter: 'off' | 'lowPass' | 'highPass'
}

interface PlaylistMix {
    enabled: boolean
    pairs: Record<string, MixTransition>
}

const mixes = reactive(new Map<string, PlaylistMix>())

function storageKey(source: string) {
    return 'lfmusic.playlistMix.v1.' + source
}

export function playlistMix(source: string): PlaylistMix {
    if (!mixes.has(source)) {
        let saved
        try { saved = JSON.parse(localStorage.getItem(storageKey(source)) || 'null') } catch {}
        mixes.set(source, { enabled: saved?.enabled === true, pairs: saved?.pairs ?? {} })
    }
    return mixes.get(source)!
}

export function togglePlaylistMix(source: string) {
    const mix = playlistMix(source)
    mix.enabled = !mix.enabled
    localStorage.setItem(storageKey(source), JSON.stringify(mix))
}

function pairKey(from: string, to: string) {
    return JSON.stringify([from, to])
}

export function savedTransition(source: string, from: string, to: string) {
    return playlistMix(source).pairs[pairKey(from, to)]
}

export function mixKey(key: string | null) {
    if (!key) return ''
    const note = /^([A-G][#b]?)\s*(major|minor|m)?$/.exec(key)
    if (!note) return key
    const minor = note[2] === 'm' || note[2] === 'minor'
    const circle = minor ? ['G#','D#','A#','F','C','G','D','A','E','B','F#','C#']
        : ['B','F#','C#','G#','D#','A#','F','C','G','D','A','E']
    const flat: Record<string, string> = { Db: 'C#', Eb: 'D#', Gb: 'F#', Ab: 'G#', Bb: 'A#' }
    const index = circle.indexOf(flat[note[1]] ?? note[1])
    return index < 0 ? key : String(index + 1) + (minor ? 'A' : 'B')
}

export function mixBarSeconds(from: TrackMetadata, to?: TrackMetadata) {
    return 240 / (from.bpm || to?.bpm || 120)
}

function autoTransition(from: TrackMetadata, to?: TrackMetadata): MixTransition {
    const bpm = from.bpm
    const tempoGap = bpm && to?.bpm ? Math.abs(bpm - to.bpm) / bpm : 0
    const rising = to?.energy != null && from.energy != null && to.energy - from.energy > 0.24
    return {
        seconds: PRESET_BARS * mixBarSeconds(from, to),
        skipIntro: 0, skipOutro: 0,
        preset: 'auto', volumeCurve: rising ? 'rise' : 'smooth', equalPower: true,
        eqBand: 'centerBass', filter: tempoGap > 0.12 ? 'lowPass' : rising ? 'highPass' : 'off',
    }
}

export function applyMixPreset(preset: MixPreset, from: TrackMetadata, draft: MixTransition, to?: TrackMetadata): MixTransition {
    const auto = autoTransition(from, to)
    if (preset === 'auto') return auto
    const profiles: Record<Exclude<MixPreset, 'auto'>, [VolumeCurve, MixTransition['eqBand'], MixTransition['filter']]> = {
        fade: ['smooth', 'centerBass', 'off'],
        rise: ['rise', 'off', 'highPass'],
        blend: ['overlap', 'centerBass', 'off'],
        wave: ['smooth', 'mids', 'highPass'],
        melt: ['overlap', 'bass', 'lowPass'],
        slam: ['cut', 'centerBass', 'off'],
        none: ['linear', 'off', 'off'],
    }
    const [volumeCurve, eqBand, filter] = profiles[preset]
    return { ...draft, curves: undefined, seconds: preset === 'none' ? 0 : auto.seconds,
        preset, volumeCurve, eqBand, filter,
        skipIntro: preset === 'none' ? 0 : draft.skipIntro, skipOutro: preset === 'none' ? 0 : draft.skipOutro }
}

export function resolveMixTransition(source: string, from: TrackMetadata, toPath: string, to?: TrackMetadata): MixTransition {
    const saved = savedTransition(source, from.file_path, toPath)
    if (!saved) return autoTransition(from, to)
    if (!saved.preset || saved.preset === 'custom' || saved.preset === 'none') return saved
    return { ...saved, seconds: PRESET_BARS * mixBarSeconds(from, to) }
}

export function mixOverlap(seconds: number, outgoing: number, incoming: number, speed = 1) {
    return Math.max(0, Math.min(32, seconds, outgoing / speed / 2, incoming / speed / 2))
}

export function saveTransition(source: string, from: string, to: string, transition: MixTransition | null) {
    const mix = playlistMix(source)
    const key = pairKey(from, to)
    if (transition) mix.pairs[key] = { ...transition }
    else delete mix.pairs[key]
    localStorage.setItem(storageKey(source), JSON.stringify(mix))
}
