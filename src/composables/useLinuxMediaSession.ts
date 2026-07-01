import type { ComputedRef, Ref } from 'vue'
import { onMounted, onUnmounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { TrackMetadata } from './usePlayer'
import { isLinuxDesktop } from './usePlatform'

type PlaybackState = 'Stopped' | 'Playing' | 'Paused'

interface LinuxMediaSessionOptions {
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

// System media controls on desktop Linux via MPRIS (souvlaki backend).
// No-op elsewhere (Windows uses the taskbar, Android uses useAndroidMediaSession).
export function useLinuxMediaSession(options: LinuxMediaSessionOptions) {
  if (!isLinuxDesktop()) return

  let unlisten: UnlistenFn | null = null
  let lastPositionSentAt = 0

  const sendMetadata = () => {
    const track = options.currentTrack.value
    invoke('mpris_update_metadata', {
      title: track ? trackTitle(track) : null,
      artist: track?.artist || null,
      album: track?.album || null,
      // Backend builds a file:// URL to the cached WebP thumbnail from file_path.
      filePath: track?.file_path || null,
      durationSecs: Math.max(0, options.durationSecs.value || 0),
    }).catch(() => {})
  }

  const sendPlayback = () => {
    invoke('mpris_update_playback', {
      state: options.playbackState.value,
      positionSecs: Math.max(0, options.positionSecs.value || 0),
    }).catch(() => {})
  }

  const onCommand = (action: string) => {
    const playing = options.playbackState.value === 'Playing'
    switch (action) {
      case 'next':
        void options.playNext()
        break
      case 'prev':
        void options.playPrev()
        break
      case 'play':
        if (!playing) void options.togglePlayPause()
        break
      case 'pause':
      case 'stop':
        if (playing) void options.togglePlayPause()
        break
      case 'playpause':
        void options.togglePlayPause()
        break
    }
  }

  // Metadata: on track, cover, or duration change.
  watch(
    [
      () => options.currentTrack.value?.file_path,
      () => options.currentCoverArt.value,
      () => options.durationSecs.value,
    ],
    () => {
      sendMetadata()
      sendPlayback()
    },
    { immediate: true },
  )

  // Play/pause/stop state is sent immediately (with the current position).
  watch(
    () => options.playbackState.value,
    () => sendPlayback(),
  )

  // Position updates throttled to once per 3s; MPRIS clients extrapolate themselves.
  watch(options.positionSecs, () => {
    const now = Date.now()
    if (now - lastPositionSentAt < 3000) return
    lastPositionSentAt = now
    sendPlayback()
  })

  onMounted(() => {
    listen<string>('mpris_media', (event) => onCommand(event.payload))
      .then((un) => { unlisten = un })
      .catch(() => {})
    sendMetadata()
    sendPlayback()
  })

  onUnmounted(() => {
    if (unlisten) {
      unlisten()
      unlisten = null
    }
  })
}
