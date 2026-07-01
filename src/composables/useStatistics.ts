import { invoke } from '@tauri-apps/api/core'
import { type TrackMetadata } from './usePlayer'

export interface TrackStatistics {
  file_path: string
  early_skip_count: number
  skip_count: number
  listen_count: number
  full_listen_count: number
  playlist_add_count: number
  search_start_count: number
  manual_start_count: number
  repeat_enabled_count: number
  move_to_top_count: number
  morning_listen_count: number
  day_listen_count: number
  evening_listen_count: number
  night_listen_count: number
  total_listened_secs: number
}

export interface TopArtist {
  name: string
  listen_count: number
  total_listened_secs: number
  track_count: number
}

export interface TopAlbum {
  name: string
  artist: string
  listen_count: number
  total_listened_secs: number
  first_file_path: string | null
}

export interface SummaryStats {
  total_tracks: number
  total_folders: number
  total_listened_secs: number
  total_listens: number
  morning_listens: number
  day_listens: number
  evening_listens: number
  night_listens: number
}

export interface TrackStatisticsUpdateEventDetail {
  filePath: string
  listenCountDelta?: number
}

export const TRACK_STATISTICS_UPDATED_EVENT = 'lfmusic:track-statistics-updated'

export function emitTrackStatisticsUpdated(detail: TrackStatisticsUpdateEventDetail) {
  window.dispatchEvent(new CustomEvent<TrackStatisticsUpdateEventDetail>(
    TRACK_STATISTICS_UPDATED_EVENT,
    { detail },
  ))
}

export function getTrackStatistics(filePath: string): Promise<TrackStatistics> {
  return invoke<TrackStatistics>('statistics_get_track', { filePath })
}

export function sumListenCount(filePaths: string[]): Promise<number> {
  if (filePaths.length === 0) return Promise.resolve(0)
  return invoke<number>('statistics_sum_listen_count', { filePaths })
}

export function getRecentlyPlayed(limit: number): Promise<TrackMetadata[]> {
  return invoke<TrackMetadata[]>('statistics_get_recently_played', { limit })
}

export function getMostPlayed(limit: number): Promise<TrackMetadata[]> {
  return invoke<TrackMetadata[]>('statistics_get_most_played', { limit })
}

export function getTopArtists(limit: number): Promise<TopArtist[]> {
  return invoke<TopArtist[]>('statistics_get_top_artists', { limit })
}

export function getTopAlbums(limit: number): Promise<TopAlbum[]> {
  return invoke<TopAlbum[]>('statistics_get_top_albums', { limit })
}

export function getSummaryStats(): Promise<SummaryStats> {
  return invoke<SummaryStats>('statistics_get_summary_stats')
}

export interface RecommendationOptions {
  /** 'fresh' = rarer/unfamiliar tracks, 'favorite' = more familiar/liked, 'balanced' = no filter */
  familiarity?: 'fresh' | 'balanced' | 'favorite'
  losslessOnly?: boolean
}

export function getRecommendations(
  mood: string,
  limit: number,
  opts: RecommendationOptions = {},
): Promise<TrackMetadata[]> {
  return invoke<TrackMetadata[]>('statistics_get_recommendations', {
    mood,
    limit,
    familiarity: opts.familiarity,
    losslessOnly: opts.losslessOnly,
  })
}

export interface TasteProfile {
  analyzed_tracks: number
  energy: number | null
  danceability: number | null
  bpm: number | null
  spectral_centroid: number | null
  loudness: number | null
}

export function getTasteProfile(): Promise<TasteProfile> {
  return invoke<TasteProfile>('statistics_get_taste_profile')
}
