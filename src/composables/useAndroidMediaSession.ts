import type { ComputedRef, Ref } from 'vue'
import { onMounted, onUnmounted, watch } from 'vue'
import type { TrackMetadata } from './usePlayer'
import { isAndroidRuntime } from './usePlatform'
import { useSettings } from './useSettings'
import { joinArtists } from '../utils/format'

declare global {
  interface Window {
    LFMusicAndroid?: {
      updateMediaSession(payload: string): void
      updateAudioFeatureProgress?(payload: string): void
      updateSystemBars?(color: string): void
      getPermissionStatus?(): string
      getSystemInsets?(): string
      requestRequiredPermissionsFromJs?(): void
      installApk?(path: string): string
    }
  }
}

type PlaybackState = 'Stopped' | 'Playing' | 'Paused'

interface AndroidMediaCommandEvent {
  command?: string
}

interface AndroidMediaSessionOptions {
  currentTrack: Ref<TrackMetadata | null>
  currentCoverArt: Ref<string | null>
  playbackState: Ref<PlaybackState>
  positionSecs: Ref<number>
  durationSecs: ComputedRef<number>
  playPrev: () => void | Promise<void>
  playNext: () => void | Promise<void>
  togglePlayPause: () => void | Promise<void>
}

function trackTitle(track: TrackMetadata): string {
  const fileName = track.file_path.split(/[/\\]/).pop()?.replace(/\.[^/.]+$/, '')
  return track.title || fileName || 'LFMusic'
}

// The native side decodes only base64 data URLs, while covers are protocol URLs
// (lfmusic-cover / lfmusic-external), so the image is fetched and inlined here.
let artworkSource = ''
let artworkData = ''

async function artworkDataUrl(source: string): Promise<string> {
  if (!source || source.startsWith('data:')) return source
  if (source === artworkSource) return artworkData
  try {
    const blob = await (await fetch(source)).blob()
    const data = await new Promise<string>(resolve => {
      const reader = new FileReader()
      reader.onload = () => resolve(typeof reader.result === 'string' ? reader.result : '')
      reader.onerror = () => resolve('')
      reader.readAsDataURL(blob)
    })
    artworkSource = source
    artworkData = data
    return data
  } catch {
    return ''
  }
}

function sendUpdate(options: AndroidMediaSessionOptions, artwork?: string) {
  const bridge = window.LFMusicAndroid
  if (!bridge) return false

  const { settings } = useSettings()
  const track = options.currentTrack.value
  const payload: Record<string, unknown> = track
    ? {
        hasTrack: true,
        title: trackTitle(track),
        artist: track.artist ? joinArtists(track.artist, settings.value.artist_separators) : '',
        album: track.album || '',
        state: options.playbackState.value,
        positionSecs: Math.max(0, options.positionSecs.value || 0),
        durationSecs: Math.max(0, options.durationSecs.value || 0),
      }
    : {
        hasTrack: false,
        state: options.playbackState.value,
        positionSecs: 0,
        durationSecs: 0,
      }

  if (typeof artwork === 'string') {
    payload.artwork = artwork
  }

  bridge.updateMediaSession(JSON.stringify(payload))
  return true
}

export function useAndroidMediaSession(options: AndroidMediaSessionOptions) {
  if (!isAndroidRuntime()) return

  let lastPositionSentAt = 0
  let lastArtworkSent = ''
  let retryTimer: ReturnType<typeof window.setTimeout> | null = null

  const sendSoon = (forceArtwork = false) => {
    if (retryTimer) window.clearTimeout(retryTimer)
    retryTimer = window.setTimeout(async () => {
      retryTimer = null
      const source = options.currentCoverArt.value || ''
      const includeArtwork = forceArtwork || source !== lastArtworkSent
      const artwork = includeArtwork ? await artworkDataUrl(source) : undefined
      // The cover changed while it was loading; the newer call sends it.
      if ((options.currentCoverArt.value || '') !== source) return
      const delivered = sendUpdate(options, artwork)
      if (delivered && includeArtwork) lastArtworkSent = source
    }, 120)
  }

  const onCommand = (event: Event) => {
    const command = (event as CustomEvent<AndroidMediaCommandEvent>).detail?.command
    if (command === 'previous') {
      void options.playPrev()
      return
    }
    if (command === 'next') {
      void options.playNext()
      return
    }
    if (command === 'play' && options.playbackState.value !== 'Playing') {
      void options.togglePlayPause()
      return
    }
    if (command === 'pause' && options.playbackState.value === 'Playing') {
      void options.togglePlayPause()
      return
    }
    if (command === 'play_pause') {
      void options.togglePlayPause()
    }
  }

  watch(
    [
      () => options.currentTrack.value?.file_path,
      () => options.playbackState.value,
      () => options.durationSecs.value,
    ],
    () => sendSoon(true),
    { immediate: true },
  )

  watch(
    () => options.currentCoverArt.value,
    () => sendSoon(true),
  )

  watch(options.positionSecs, () => {
    const now = Date.now()
    if (now - lastPositionSentAt < 2500) return
    lastPositionSentAt = now
    sendSoon()
  })

  onMounted(() => {
    window.addEventListener('lfmusic:android-media-command', onCommand as EventListener)
    sendSoon(true)
  })

  onUnmounted(() => {
    window.removeEventListener('lfmusic:android-media-command', onCommand as EventListener)
    if (retryTimer) {
      window.clearTimeout(retryTimer)
      retryTimer = null
    }
  })
}
