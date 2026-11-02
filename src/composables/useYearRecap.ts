import { ref } from 'vue'
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

export const recapOpen = ref(false)
export const recapYear = ref(new Date().getFullYear())

export function openYearRecap(year: number) {
  recapYear.value = year
  recapOpen.value = true
}

export function loadYearRecap(year: number): Promise<YearRecap> {
  return invoke<YearRecap>('statistics_get_year_recap', { year })
}
