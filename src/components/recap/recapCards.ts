import { hexToHSL } from '../../composables/useAccentTheme'
import {
  MIN_RECAP_PLAYS,
  type RecapAlbum,
  type RecapArtist,
  type RecapGenre,
  type RecapObsession,
  type RecapSession,
  type RecapTrack,
  type YearRecap,
} from '../../composables/useYearRecap'

/** plain: shader-driven surface; cover: shader plus soft ambient cover art aura. */
export type CardTone = 'plain' | 'cover'

/** Procedural WebGL2 backdrop style for a slide. */
export type BgScene = 'silk' | 'aurora' | 'scanner' | 'prism' | 'beams' | 'vortex'

export type CardKind =
  | 'empty' | 'intro' | 'minutes' | 'first' | 'variety' | 'topTrack' | 'topTracks'
  | 'topArtist' | 'topArtists' | 'topAlbum' | 'genres' | 'moments' | 'obsession'
  | 'marathon' | 'rhythm' | 'discovery' | 'collection' | 'habits' | 'character' | 'outro'

export type MomentMode = 'night' | 'morning' | 'day' | 'evening'

export type CharacterId =
  | 'nightOwl' | 'fan' | 'explorer' | 'marathoner' | 'energy' | 'calm'
  | 'dancer' | 'completist' | 'shuffler' | 'melomane'

export interface RecapCharacter {
  id: CharacterId
  /** Up to two secondary archetypes, shown as chips. */
  traits: CharacterId[]
}

/** Card accent from a cover or thematic mood; null falls back to the app's current accent. */
export interface CardAccent {
  hue: number
  sat: number
  /** Secondary hue offset (degrees) for multi-tone shader harmonies. */
  shift?: number
}

export interface RecapCard {
  id: string
  kind: CardKind
  tone: CardTone
  scene: BgScene
  accent: CardAccent | null
  /** Fragment that loops while the card is shown. */
  sound: string | null
  /** Auto-advance time; 0 keeps the card until the user moves on. */
  duration: number
  track?: RecapTrack
  tracks?: RecapTrack[]
  artist?: RecapArtist
  artists?: RecapArtist[]
  album?: RecapAlbum
  genres?: RecapGenre[]
  obsession?: RecapObsession
  session?: RecapSession
  moment?: { mode: MomentMode; peakHour: number; count: number }
  character?: RecapCharacter
}

const MOMENT_ACCENTS: Record<MomentMode, CardAccent> = {
  night: { hue: 236, sat: 64, shift: 44 },
  morning: { hue: 28, sat: 68, shift: 34 },
  day: { hue: 202, sat: 64, shift: 36 },
  evening: { hue: 282, sat: 62, shift: 46 },
}

const CHARACTER_ACCENTS: Record<CharacterId, CardAccent> = {
  nightOwl: { hue: 244, sat: 66, shift: 48 },
  fan: { hue: 344, sat: 68, shift: 32 },
  explorer: { hue: 168, sat: 62, shift: 42 },
  marathoner: { hue: 18, sat: 70, shift: 34 },
  energy: { hue: 356, sat: 70, shift: 38 },
  calm: { hue: 196, sat: 58, shift: 36 },
  dancer: { hue: 302, sat: 68, shift: 44 },
  completist: { hue: 218, sat: 62, shift: 38 },
  shuffler: { hue: 266, sat: 64, shift: 52 },
  melomane: { hue: 274, sat: 64, shift: 42 },
}

const SCENE_ROTATION: BgScene[] = ['silk', 'aurora', 'scanner', 'prism', 'beams', 'vortex']

/**
 * Cover accent, tamed like the app's dynamic accent: grey covers give none, loud ones are toned
 * down.
 */
function accentOf(track: RecapTrack | null | undefined, shift = 36): CardAccent | null {
  const hex = track?.accent_color
  if (!hex || !/^#[0-9a-f]{6}$/i.test(hex)) return null
  const { h, s } = hexToHSL(hex)
  if (s < 12) return null
  return { hue: h, sat: Math.min(72, Math.max(36, s)), shift }
}

function shiftAccent(base: CardAccent | null, deg: number, shift = 38): CardAccent | null {
  if (!base) return null
  return {
    hue: ((base.hue + deg) % 360 + 360) % 360,
    sat: base.sat,
    shift,
  }
}

function share(part: number, total: number): number {
  return total > 0 ? part / total : 0
}

export function peakIndex(values: number[]): number {
  let best = 0
  values.forEach((v, i) => {
    if (v > values[best]) best = i
  })
  return best
}

/** Distinct covers of the year's featured tracks, for mosaics. */
export function recapCovers(recap: YearRecap, limit: number): string[] {
  const urls = [
    ...recap.top_tracks,
    ...recap.top_artists.map(a => a.top_track),
    ...recap.top_albums.map(a => a.top_track),
    recap.discovery?.top_track,
    recap.obsession?.track,
    recap.first_track,
  ].map(t => t?.cover_url).filter((u): u is string => !!u)
  return [...new Set(urls)].slice(0, limit)
}

function momentOf(recap: YearRecap): NonNullable<RecapCard['moment']> {
  const peakHour = peakIndex(recap.hours)
  if (recap.night_plays >= 5 && share(recap.night_plays, recap.plays) >= 0.12) {
    return { mode: 'night', peakHour, count: recap.night_plays }
  }
  if (recap.morning_plays >= 5 && share(recap.morning_plays, recap.plays) >= 0.15) {
    return { mode: 'morning', peakHour, count: recap.morning_plays }
  }
  const sum = (from: number, to: number) => recap.hours.slice(from, to).reduce((a, b) => a + b, 0)
  const parts: [MomentMode, number][] = [
    ['night', sum(0, 5)],
    ['morning', sum(5, 12)],
    ['day', sum(12, 18)],
    ['evening', sum(18, 24)],
  ]
  parts.sort((a, b) => b[1] - a[1])
  return { mode: parts[0][0], peakHour, count: 0 }
}

/** Picks the listener archetype; every rule is a share or average, so it scales with the year. */
export function characterOf(recap: YearRecap): RecapCharacter {
  const plays = Math.max(1, recap.plays)
  const scores: [CharacterId, number][] = []
  const add = (id: CharacterId, score: number, threshold: number) => {
    if (score >= threshold) scores.push([id, score / threshold])
  }

  add('nightOwl', share(recap.night_plays, plays), 0.15)
  add('fan', share(recap.top_artists[0]?.plays ?? 0, plays), 0.25)
  if (recap.unique_artists >= 25) add('explorer', share(recap.unique_artists, plays), 0.3)
  if (recap.days_listened > 0) add('marathoner', recap.total_secs / recap.days_listened, 2.5 * 3600)
  if (recap.energy !== null) {
    add('energy', recap.energy, 0.6)
    add('calm', 1 - recap.energy, 0.62)
  }
  if (recap.danceability !== null) add('dancer', recap.danceability, 0.62)
  if (share(recap.skips, plays) < 0.15) add('completist', share(recap.full_listens, plays), 0.8)
  add('shuffler', recap.shuffle_share, 0.6)

  scores.sort((a, b) => b[1] - a[1])
  return {
    id: scores[0]?.[0] ?? 'melomane',
    traits: scores.slice(1, 3).map(s => s[0]),
  }
}

export function buildRecapCards(recap: YearRecap): RecapCard[] {
  if (recap.plays < MIN_RECAP_PLAYS) {
    return [{ id: 'empty', kind: 'empty', tone: 'plain', scene: 'aurora', accent: null, sound: null, duration: 0 }]
  }

  const top = recap.top_tracks
  const pick = (i: number) => top.length > 0 ? top[i % top.length] : undefined
  const sound = (track: RecapTrack | null | undefined) => track?.snippet_url ?? pick(0)?.snippet_url ?? null
  const yearAccent = top.map(t => accentOf(t, 42)).find(a => a !== null) ?? null
  const trackAccent = (track?: RecapTrack | null, shift = 36) => accentOf(track, shift) ?? yearAccent

  const cards: RecapCard[] = []
  const push = (
    card: Omit<RecapCard, 'accent'> & { color?: RecapTrack | null; customAccent?: CardAccent | null; hueOffset?: number; shift?: number },
  ) => {
    const { color, customAccent, hueOffset, shift, ...rest } = card
    const resolvedAccent = customAccent !== undefined
      ? customAccent
      : hueOffset !== undefined
        ? shiftAccent(yearAccent, hueOffset, shift ?? 40)
        : trackAccent(color, shift ?? 36)

    // Ensure two consecutive slides never reuse the exact same shader scene.
    let scene = rest.scene
    const prev = cards[cards.length - 1]
    if (prev && prev.scene === scene) {
      const idx = SCENE_ROTATION.indexOf(scene)
      scene = SCENE_ROTATION[(idx + 1) % SCENE_ROTATION.length]
    }
    cards.push({ ...rest, scene, accent: resolvedAccent })
  }

  push({ id: 'intro', kind: 'intro', tone: 'plain', scene: 'silk', sound: sound(pick(0)), duration: 6500, shift: 46 })
  push({ id: 'minutes', kind: 'minutes', tone: 'plain', scene: 'aurora', sound: sound(pick(0)), duration: 8000, hueOffset: 28, shift: 44 })

  if (recap.first_track) {
    push({
      id: 'first', kind: 'first', tone: 'cover', scene: 'beams', track: recap.first_track,
      sound: sound(recap.first_track), duration: 8000, color: recap.first_track, shift: 34,
    })
  }

  push({ id: 'variety', kind: 'variety', tone: 'plain', scene: 'prism', sound: sound(pick(1)), duration: 8000, color: pick(1), shift: 48 })

  if (top[0]) {
    push({ id: 'topTrack', kind: 'topTrack', tone: 'cover', scene: 'silk', track: top[0], sound: sound(top[0]), duration: 9000, color: top[0], shift: 38 })
  }
  if (top.length >= 3) {
    push({ id: 'topTracks', kind: 'topTracks', tone: 'plain', scene: 'scanner', tracks: top, sound: sound(top[0]), duration: 12000, color: top[0], shift: 34 })
  }

  const artist = recap.top_artists[0]
  if (artist) {
    push({
      id: 'topArtist', kind: 'topArtist', tone: 'cover', scene: 'prism', artist,
      sound: sound(artist.top_track), duration: 9000, color: artist.top_track, shift: 42,
    })
  }
  if (recap.top_artists.length >= 3) {
    push({
      id: 'topArtists', kind: 'topArtists', tone: 'plain', scene: 'vortex', artists: recap.top_artists,
      sound: sound(recap.top_artists[1]?.top_track), duration: 12000, color: recap.top_artists[1]?.top_track, shift: 40,
    })
  }

  const album = recap.top_albums[0]
  if (album && album.plays >= 5) {
    push({ id: 'topAlbum', kind: 'topAlbum', tone: 'cover', scene: 'beams', album, sound: sound(album.top_track), duration: 8000, color: album.top_track, shift: 34 })
  }

  if (recap.top_genres.length > 0) {
    push({ id: 'genres', kind: 'genres', tone: 'plain', scene: 'silk', genres: recap.top_genres, sound: sound(pick(2)), duration: 9000, color: pick(2), shift: 52 })
  }

  const moment = momentOf(recap)
  push({ id: 'moments', kind: 'moments', tone: 'plain', scene: 'aurora', moment, sound: sound(pick(3)), duration: 9000, customAccent: MOMENT_ACCENTS[moment.mode] })

  if (recap.obsession) {
    push({
      id: 'obsession', kind: 'obsession', tone: 'cover', scene: 'vortex', obsession: recap.obsession,
      sound: sound(recap.obsession.track), duration: 8000, color: recap.obsession.track, shift: 44,
    })
  }

  if (recap.marathon && recap.marathon.secs >= 3600) {
    push({ id: 'marathon', kind: 'marathon', tone: 'plain', scene: 'scanner', session: recap.marathon, sound: sound(pick(4)), duration: 8000, hueOffset: -36, shift: 42 })
  }

  if (recap.days_listened >= 7) {
    push({ id: 'rhythm', kind: 'rhythm', tone: 'plain', scene: 'beams', sound: sound(pick(1)), duration: 9000, hueOffset: 148, shift: 38 })
  }

  if (recap.discovery) {
    push({
      id: 'discovery', kind: 'discovery', tone: 'cover', scene: 'aurora', artist: recap.discovery,
      sound: sound(recap.discovery.top_track), duration: 8000, color: recap.discovery.top_track, shift: 44,
    })
  }

  if (recap.likes > 0 || recap.playlists_created > 0 || recap.downloads > 0 || recap.top_playlist) {
    push({ id: 'collection', kind: 'collection', tone: 'plain', scene: 'prism', sound: sound(pick(2)), duration: 9000, hueOffset: -64, shift: 46 })
  }

  push({ id: 'habits', kind: 'habits', tone: 'plain', scene: 'scanner', sound: sound(pick(3)), duration: 9000, hueOffset: 204, shift: 36 })

  const character = characterOf(recap)
  push({ id: 'character', kind: 'character', tone: 'plain', scene: 'vortex', character, sound: sound(pick(0)), duration: 10000, customAccent: CHARACTER_ACCENTS[character.id] })
  push({ id: 'outro', kind: 'outro', tone: 'plain', scene: 'silk', character, sound: sound(pick(0)), duration: 0, shift: 46 })

  return cards
}
