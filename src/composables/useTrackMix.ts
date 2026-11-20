import { invoke } from '@tauri-apps/api/core'
import { usePlayer, type TrackMetadata } from './usePlayer'
import { useMixSettings } from './useMix'
import router from '../router'

const MIX_LIMIT = 80

let building = false

/** "Mix from track": plays the track followed by similar ones from the library as the mix queue
 * and opens the Recommendations page, where the mix is shown. */
export async function startTrackMix(filePath: string): Promise<void> {
  if (building) return
  building = true
  if (router.currentRoute.value.path !== '/') void router.push('/')
  try {
    const player = usePlayer()
    const { losslessOnly } = useMixSettings()
    const tracks = await invoke<TrackMetadata[]>('statistics_get_similar_tracks', {
      filePath,
      limit: MIX_LIMIT,
      losslessOnly: losslessOnly.value || undefined,
    })
    if (!tracks.length) return
    player.cacheTracks(tracks)
    const paths = tracks.map(t => t.file_path)
    player.shuffled.value = false
    player.setMixSourceFromPaths(paths)
    if (player.currentTrack.value?.file_path === filePath) {
      if (!player.isPlaying.value) await player.togglePlayPause()
    } else {
      await player.playTrack(paths[0], { manualListStart: true })
    }
    player.loadCoversForTracks(paths.slice(0, 24).map(p => player.getTrackByPath(p)).filter((t): t is TrackMetadata => !!t))
  } catch (e) {
    console.error('Не удалось запустить микс от трека:', e)
  } finally {
    building = false
  }
}
