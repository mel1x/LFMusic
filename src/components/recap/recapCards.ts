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

// Turns a YearRecap into the ordered list of story cards. A card only makes it
// into the list when its numbers are worth showing, so a light year gets a
// shorter story instead of cards full of zeros.

/** plain: dark surface with one accent light; cover: the featured cover, blurred, behind it. */
export type CardTone = 'plain' | 'cover'

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

/** Card accent from a cover; null falls back to the app's current accent. */
export interface CardAccent {
  hue: number
  sat: number
}

export interface RecapCard {
  id: string
  kind: CardKind
  tone: CardTone
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

/** Cover accent, tamed like the app's dynamic accent: grey covers give none, loud ones are toned down. */
function accentOf(track: RecapTrack | null | undefined): CardAccent | null {
  const hex = track?.accent_color
  if (!hex || !/^#[0-9a-f]{6}$/i.test(hex)) return null
  const { h, s } = hexToHSL(hex)
  if (s < 12) return null
  return { hue: h, sat: Math.min(70, Math.max(30, s)) }
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
    return [{ id: 'empty', kind: 'empty', tone: 'plain', accent: null, sound: null, duration: 0 }]
  }

  const top = recap.top_tracks
  const pick = (i: number) => top.length > 0 ? top[i % top.length] : undefined
  const sound = (track: RecapTrack | null | undefined) => track?.snippet_url ?? pick(0)?.snippet_url ?? null
  // Cards without a featured cover share the year's color: the first top track with an accent.
  const yearAccent = top.map(accentOf).find(a => a !== null) ?? null
  const accent = (track?: RecapTrack | null) => accentOf(track) ?? yearAccent

  const cards: RecapCard[] = []
  const push = (card: Omit<RecapCard, 'accent'> & { color?: RecapTrack | null }) => {
    const { color, ...rest } = card
    cards.push({ ...rest, accent: accent(color) })
  }

  push({ id: 'intro', kind: 'intro', tone: 'plain', sound: sound(pick(0)), duration: 6500 })
  push({ id: 'minutes', kind: 'minutes', tone: 'plain', sound: sound(pick(0)), duration: 8000 })

  if (recap.first_track) {
    push({
      id: 'first', kind: 'first', tone: 'cover', track: recap.first_track,
      sound: sound(recap.first_track), duration: 8000, color: recap.first_track,
    })
  }

  push({ id: 'variety', kind: 'variety', tone: 'plain', sound: sound(pick(1)), duration: 8000 })

  if (top[0]) {
    push({ id: 'topTrack', kind: 'topTrack', tone: 'cover', track: top[0], sound: sound(top[0]), duration: 9000, color: top[0] })
  }
  if (top.length >= 3) {
    push({ id: 'topTracks', kind: 'topTracks', tone: 'plain', tracks: top, sound: sound(top[0]), duration: 12000 })
  }

  const artist = recap.top_artists[0]
  if (artist) {
    push({
      id: 'topArtist', kind: 'topArtist', tone: 'cover', artist,
      sound: sound(artist.top_track), duration: 9000, color: artist.top_track,
    })
  }
  if (recap.top_artists.length >= 3) {
    push({
      id: 'topArtists', kind: 'topArtists', tone: 'plain', artists: recap.top_artists,
      sound: sound(recap.top_artists[1]?.top_track), duration: 12000,
    })
  }

  const album = recap.top_albums[0]
  if (album && album.plays >= 5) {
    push({ id: 'topAlbum', kind: 'topAlbum', tone: 'cover', album, sound: sound(album.top_track), duration: 8000, color: album.top_track })
  }

  if (recap.top_genres.length > 0) {
    push({ id: 'genres', kind: 'genres', tone: 'plain', genres: recap.top_genres, sound: sound(pick(2)), duration: 9000 })
  }

  push({ id: 'moments', kind: 'moments', tone: 'plain', moment: momentOf(recap), sound: sound(pick(3)), duration: 9000 })

  if (recap.obsession) {
    push({
      id: 'obsession', kind: 'obsession', tone: 'cover', obsession: recap.obsession,
      sound: sound(recap.obsession.track), duration: 8000, color: recap.obsession.track,
    })
  }

  if (recap.marathon && recap.marathon.secs >= 3600) {
    push({ id: 'marathon', kind: 'marathon', tone: 'plain', session: recap.marathon, sound: sound(pick(4)), duration: 8000 })
  }

  if (recap.days_listened >= 7) {
    push({ id: 'rhythm', kind: 'rhythm', tone: 'plain', sound: sound(pick(1)), duration: 9000 })
  }

  if (recap.discovery) {
    push({
      id: 'discovery', kind: 'discovery', tone: 'cover', artist: recap.discovery,
      sound: sound(recap.discovery.top_track), duration: 8000, color: recap.discovery.top_track,
    })
  }

  if (recap.likes > 0 || recap.playlists_created > 0 || recap.downloads > 0 || recap.top_playlist) {
    push({ id: 'collection', kind: 'collection', tone: 'plain', sound: sound(pick(2)), duration: 9000 })
  }

  push({ id: 'habits', kind: 'habits', tone: 'plain', sound: sound(pick(3)), duration: 9000 })

  const character = characterOf(recap)
  push({ id: 'character', kind: 'character', tone: 'plain', character, sound: sound(pick(0)), duration: 10000 })
  push({ id: 'outro', kind: 'outro', tone: 'plain', character, sound: sound(pick(0)), duration: 0 })

  return cards
}
