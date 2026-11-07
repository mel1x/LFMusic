import type { ComputedRef, Ref } from 'vue'
import { watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { TrackMetadata } from './usePlayer'
import { useSettings } from './useSettings'
import { isAndroidRuntime } from './usePlatform'
import { joinArtists } from '../utils/format'

type PlaybackState = 'Stopped' | 'Playing' | 'Paused'

interface DiscordRpcOptions {
  currentTrack: Ref<TrackMetadata | null>
  playbackState: Ref<PlaybackState>
  positionSecs: Ref<number>
  durationSecs: ComputedRef<number>
  /** Source label: album/playlist/artist/catalog/mix (see Player.vue) */
  sourceLabel: ComputedRef<string>
}

/** Mirrors discord_rpc::PresencePayload (snake_case keys, matching Rust) */
interface PresencePayload {
  /** Line 1: track title */
  details: string
  /** Line 2: artist */
  state: string
  cover_url: string | null
  /** Line 3: source (album/playlist/artist/catalog/mix) */
  cover_text: string | null
  duration_secs: number
  position_secs: number
  playing: boolean
}

const SEND_DEBOUNCE_MS = 250
/** Retry in case Discord client was unavailable (may get launched later) */
const RETRY_MS = 20_000
/** Position drift from extrapolation that counts as a seek */
const SEEK_RESEND_THRESHOLD_SECS = 3

function presenceTitle(track: TrackMetadata): string {
  const fileName = track.file_path.split(/[/\\]/).pop()?.replace(/\.[^/.]+$/, '')
  return track.title || fileName || 'LFMusic'
}

// Session cache of resolved covers: file_path -> https URL (null = not found).
const coverUrlCache = new Map<string, string | null>()

// Rich Presence only accepts https image URLs, but library covers are local
// files, so look up cover art via the iTunes Search API (no key, CORS-enabled).
async function findCoverUrl(track: TrackMetadata): Promise<string | null> {
  const cached = coverUrlCache.get(track.file_path)
  if (cached !== undefined) return cached

  const term = [track.artist, track.title].filter(Boolean).join(' ').trim()
  if (!term) {
    coverUrlCache.set(track.file_path, null)
    return null
  }

  try {
    const resp = await fetch(
      `https://itunes.apple.com/search?term=${encodeURIComponent(term)}&media=music&entity=song&limit=1`,
    )
    if (!resp.ok) return null
    const data = await resp.json()
    const art: unknown = data?.results?.[0]?.artworkUrl100
    const url = typeof art === 'string' ? art.replace('100x100bb', '512x512bb') : null
    coverUrlCache.set(track.file_path, url)
    return url
  } catch {
    // Network unavailable: don't cache, retry next time.
    return null
  }
}

// Discord Rich Presence: publishes the current track to the Discord profile.
// Discord animates the progress bar itself from timestamps, so we only send
// updates on track/source change, pause, and seeks. No-op on Android.
export function useDiscordRpc(options: DiscordRpcOptions) {
  if (isAndroidRuntime()) return

  const { settings } = useSettings()

  let sendTimer: ReturnType<typeof setTimeout> | null = null
  let retryTimer: ReturnType<typeof setTimeout> | null = null
  let lastSentPositionSecs = 0
  let lastSentAtMs = 0
  let sendSeq = 0

  function clearRetry() {
    if (retryTimer) {
      clearTimeout(retryTimer)
      retryTimer = null
    }
  }

  function scheduleSend() {
    clearRetry()
    if (sendTimer) clearTimeout(sendTimer)
    sendTimer = setTimeout(() => {
      sendTimer = null
      void send()
    }, SEND_DEBOUNCE_MS)
  }

  async function send() {
    const seq = ++sendSeq
    const track = options.currentTrack.value

    if (!settings.value.discord_rpc_enabled || !track || options.playbackState.value === 'Stopped') {
      invoke('discord_clear_presence').catch(() => {})
      return
    }

    const coverUrl = await findCoverUrl(track)
    // State may have changed while awaiting the cover lookup; this update is stale.
    if (seq !== sendSeq) return

    const playing = options.playbackState.value === 'Playing'
    const title = presenceTitle(track)
    const payload: PresencePayload = {
      details: playing ? title : `⏸ ${title}`,
      state: track.artist ? joinArtists(track.artist, settings.value.artist_separators) : 'Неизвестный исполнитель',
      cover_url: coverUrl,
      cover_text: options.sourceLabel.value,
      duration_secs: Math.max(0, options.durationSecs.value || 0),
      position_secs: Math.max(0, options.positionSecs.value || 0),
      playing,
    }

    lastSentPositionSecs = payload.position_secs
    lastSentAtMs = Date.now()

    const delivered = await invoke<boolean>('discord_update_presence', { payload }).catch(() => true)
    if (delivered === false && seq === sendSeq) {
      clearRetry()
      retryTimer = setTimeout(() => {
        retryTimer = null
        void send()
      }, RETRY_MS)
    }
  }

  watch(
    [
      () => options.currentTrack.value?.file_path,
      () => options.playbackState.value,
      () => options.sourceLabel.value,
      () => settings.value.discord_rpc_enabled,
    ],
    scheduleSend,
    { immediate: true },
  )

  // Seek: position diverged from the extrapolation of the last sent update.
  watch(options.positionSecs, (pos) => {
    if (options.playbackState.value !== 'Playing') return
    if (!settings.value.discord_rpc_enabled || !options.currentTrack.value) return
    if (sendTimer) return
    const expected = lastSentPositionSecs + (Date.now() - lastSentAtMs) / 1000
    if (Math.abs(pos - expected) > SEEK_RESEND_THRESHOLD_SECS) scheduleSend()
  })
}
