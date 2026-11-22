import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

export interface RecapTrack {
  file_path: string
  title: string | null
  artist: string | null
  album: string | null
  accent_color: string | null
  duration_secs: number | null
  in_library: boolean
  cover_url: string | null
  snippet_url: string | null
  plays: number
  secs: number
}

export interface RecapArtist {
  name: string
  plays: number
  secs: number
  track_count: number
  top_track: RecapTrack
}

export interface RecapAlbum {
  name: string
  artist: string | null
  plays: number
  secs: number
  top_track: RecapTrack
}

export interface RecapGenre {
  name: string
  plays: number
  share: number
}

export interface RecapDay {
  day: string
  secs: number
  plays: number
}

export interface RecapObsession {
  track: RecapTrack
  day: string
  plays: number
}

export interface RecapSession {
  day: string
  start_hour: number
  secs: number
  tracks: number
}

export interface RecapPlaylist {
  id: number
  name: string
  kind: string
  plays: number
  first_file_path: string | null
  cover_url: string | null
}

export interface YearRecap {
  year: number
  total_secs: number
  plays: number
  unique_tracks: number
  unique_artists: number
  unique_albums: number
  unique_genres: number
  days_listened: number
  longest_streak: number
  busiest_day: RecapDay | null
  first_track: RecapTrack | null
  first_listen_at: number | null
  top_tracks: RecapTrack[]
  top_artists: RecapArtist[]
  top_albums: RecapAlbum[]
  top_genres: RecapGenre[]
  hours: number[]
  weekdays: number[]
  months: number[]
  night_plays: number
  morning_plays: number
  obsession: RecapObsession | null
  marathon: RecapSession | null
  discovery: RecapArtist | null
  likes: number
  playlists_created: number
  downloads: number
  top_playlist: RecapPlaylist | null
  skips: number
  early_skips: number
  full_listens: number
  search_starts: number
  shuffle_share: number
  mobile_share: number
  energy: number | null
  danceability: number | null
  bpm: number | null
}

/** Fewer counted listens than this and the recap shows its "not yet" card. */
export const MIN_RECAP_PLAYS = 10

/** Recap is offered in December (this year) and January (the year that just ended). */
export function seasonalRecapYear(now = new Date()): number | null {
  const month = now.getMonth()
  if (month === 11) return now.getFullYear()
  if (month === 0) return now.getFullYear() - 1
  return null
}

export type RecapBadgeOverride = 'on' | 'off' | null

const RECAP_BADGE_KEY = 'lfmusic_recap_badge'
const RECAP_DEMO_KEY = 'lfmusic_recap_demo'

function readBadgeOverride(): RecapBadgeOverride {
  try {
    const v = localStorage.getItem(RECAP_BADGE_KEY)
    return v === 'on' || v === 'off' ? v : null
  } catch {
    return null
  }
}

function readDemoFlag(): boolean {
  try {
    return localStorage.getItem(RECAP_DEMO_KEY) === '1'
  } catch {
    return false
  }
}

export const recapBadgeOverride = ref<RecapBadgeOverride>(readBadgeOverride())
export const recapDemoMode = ref<boolean>(readDemoFlag())

export function setRecapBadgeOverride(next: RecapBadgeOverride) {
  recapBadgeOverride.value = next
  try {
    if (next === null) localStorage.removeItem(RECAP_BADGE_KEY)
    else localStorage.setItem(RECAP_BADGE_KEY, next)
  } catch {
  }
}

export function setRecapDemoMode(enabled: boolean) {
  recapDemoMode.value = enabled
  try {
    if (enabled) localStorage.setItem(RECAP_DEMO_KEY, '1')
    else localStorage.removeItem(RECAP_DEMO_KEY)
  } catch {
  }
}

export const activeRecapYear = computed<number | null>(() => {
  if (recapBadgeOverride.value === 'off') return null
  if (recapBadgeOverride.value === 'on') return seasonalRecapYear() ?? new Date().getFullYear()
  return seasonalRecapYear()
})

export const recapOpen = ref(false)
export const recapYear = ref(new Date().getFullYear())
export const recapOpenDemo = ref(false)

export function openYearRecap(year: number, opts?: { demo?: boolean }) {
  recapYear.value = year
  recapOpenDemo.value = opts?.demo ?? recapDemoMode.value
  recapOpen.value = true
}

export function loadYearRecap(year: number): Promise<YearRecap> {
  return invoke<YearRecap>('statistics_get_year_recap', { year })
}

function demoCover(c1: string, c2: string, accent: string, tag: string): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 320"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0%" stop-color="${c1}"/><stop offset="100%" stop-color="${c2}"/></linearGradient></defs><rect width="320" height="320" fill="url(#g)"/><circle cx="240" cy="84" r="96" fill="${accent}" fill-opacity="0.24"/><circle cx="72" cy="252" r="78" fill="#000" fill-opacity="0.22"/><text x="28" y="286" fill="#fff" fill-opacity="0.88" font-family="Inter,sans-serif" font-size="38" font-weight="800">${tag}</text></svg>`
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
}

export function buildDemoYearRecap(year: number): YearRecap {
  const mkTrack = (
    idx: number,
    title: string,
    artist: string,
    album: string,
    accent: string,
    c1: string,
    c2: string,
    tag: string,
    plays: number,
    duration: number,
  ): RecapTrack => ({
    file_path: `/demo/${idx}-${title}.flac`,
    title,
    artist,
    album,
    accent_color: accent,
    duration_secs: duration,
    in_library: true,
    cover_url: demoCover(c1, c2, accent, tag),
    snippet_url: null,
    plays,
    secs: plays * duration,
  })

  const t1 = mkTrack(1, 'Midnight Signal', 'Kino Wave', 'Night Lines', '#e05a47', '#b83227', '#261015', 'MW', 118, 214)
  const t2 = mkTrack(2, 'Glass Horizon', 'Nora Vale', 'Static Bloom', '#3b9be8', '#1f6fb8', '#0d1b2e', 'GH', 94, 198)
  const t3 = mkTrack(3, 'Velvet Pulse', 'Sora & Kael', 'Afterhours', '#a855f7', '#7c2bd6', '#1c0f2e', 'VP', 82, 232)
  const t4 = mkTrack(4, 'Neon Rain', 'Kino Wave', 'Night Lines', '#f59e0b', '#c97608', '#291a08', 'NR', 71, 186)
  const t5 = mkTrack(5, 'Echoes of May', 'Mireille', 'Paper Cities', '#10b981', '#0c8a60', '#0a241c', 'EM', 65, 241)
  const t6 = mkTrack(6, 'First Snow', 'Nordic Tape', 'Winter Loop', '#60a5fa', '#2d68b8', '#101a2c', 'FS', 52, 205)
  const t7 = mkTrack(7, 'Subway Lights', 'Vektor 9', 'Concrete Sky', '#f43f5e', '#be123c', '#2b0d17', 'SL', 47, 192)
  const t8 = mkTrack(8, 'Solar Drift', 'Lumen', 'Orbit 04', '#eab308', '#b47d09', '#261e09', 'SD', 41, 224)
  const t9 = mkTrack(9, 'Quiet Room', 'Asta', 'Low Tide', '#14b8a6', '#0f766e', '#0a2221', 'QR', 36, 178)

  const topTracks = [t1, t2, t3, t4, t5]
  const topArtists: RecapArtist[] = [
    { name: 'Kino Wave', plays: 248, secs: 51200, track_count: 14, top_track: t1 },
    { name: 'Nora Vale', plays: 164, secs: 32900, track_count: 11, top_track: t2 },
    { name: 'Sora & Kael', plays: 138, secs: 31400, track_count: 9, top_track: t3 },
    { name: 'Mireille', plays: 109, secs: 25100, track_count: 8, top_track: t5 },
    { name: 'Vektor 9', plays: 84, secs: 16800, track_count: 6, top_track: t7 },
  ]
  const topAlbums: RecapAlbum[] = [
    { name: 'Night Lines', artist: 'Kino Wave', plays: 189, secs: 38400, top_track: t1 },
    { name: 'Static Bloom', artist: 'Nora Vale', plays: 121, secs: 24100, top_track: t2 },
    { name: 'Orbit 04', artist: 'Lumen', plays: 76, secs: 16900, top_track: t8 },
  ]

  return {
    year,
    total_secs: 186420,
    plays: 942,
    unique_tracks: 214,
    unique_artists: 48,
    unique_albums: 36,
    unique_genres: 9,
    days_listened: 187,
    longest_streak: 29,
    busiest_day: { day: `${year}-07-19`, secs: 16200, plays: 68 },
    first_track: t6,
    first_listen_at: Math.floor(new Date(year, 0, 2, 11, 24).getTime() / 1000),
    top_tracks: topTracks,
    top_artists: topArtists,
    top_albums: topAlbums,
    top_genres: [
      { name: 'Electronic', plays: 312, share: 0.33 },
      { name: 'Indie', plays: 226, share: 0.24 },
      { name: 'Synthwave', plays: 170, share: 0.18 },
      { name: 'Hip-Hop', plays: 132, share: 0.14 },
      { name: 'Ambient', plays: 102, share: 0.11 },
    ],
    hours: [28, 19, 12, 6, 4, 5, 11, 24, 38, 46, 51, 44, 49, 53, 58, 62, 67, 74, 82, 91, 98, 114, 108, 64],
    weekdays: [124, 118, 132, 129, 158, 152, 129],
    months: [54, 62, 71, 68, 84, 92, 118, 96, 82, 89, 66, 60],
    night_plays: 178,
    morning_plays: 64,
    obsession: { track: t1, day: `${year}-08-14`, plays: 24 },
    marathon: { day: `${year}-10-05`, start_hour: 20, secs: 14820, tracks: 58 },
    discovery: { name: 'Asta', plays: 62, secs: 11200, track_count: 5, top_track: t9 },
    likes: 86,
    playlists_created: 7,
    downloads: 43,
    top_playlist: {
      id: 1,
      name: 'Night Drive',
      kind: 'User',
      plays: 194,
      first_file_path: null,
      cover_url: t4.cover_url,
    },
    skips: 72,
    early_skips: 31,
    full_listens: 785,
    search_starts: 112,
    shuffle_share: 0.42,
    mobile_share: 0.18,
    energy: 0.72,
    danceability: 0.68,
    bpm: 124,
  }
}
