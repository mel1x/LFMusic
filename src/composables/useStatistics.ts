import { invoke } from '@tauri-apps/api/core'

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
