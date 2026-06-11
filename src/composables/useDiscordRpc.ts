import type { ComputedRef, Ref } from 'vue'
import { watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { TrackMetadata } from './usePlayer'
import { useSettings } from './useSettings'
import { isAndroidRuntime } from './usePlatform'

type PlaybackState = 'Stopped' | 'Playing' | 'Paused'

interface DiscordRpcOptions {
  currentTrack: Ref<TrackMetadata | null>
  playbackState: Ref<PlaybackState>
  positionSecs: Ref<number>
  durationSecs: ComputedRef<number>
  /** Подпись источника: альбом/плейлист/артист/каталог/микс (см. Player.vue) */
  sourceLabel: ComputedRef<string>
}

/** Зеркало discord_rpc::PresencePayload (ключи — snake_case, как в Rust) */
interface PresencePayload {
  /** Первая строка — название трека */
  details: string
  /** Вторая строка — исполнитель */
  state: string
  cover_url: string | null
  /** Третья строка — источник: альбом/плейлист/артист/каталог/микс */
  cover_text: string | null
  duration_secs: number
  position_secs: number
  playing: boolean
}

const SEND_DEBOUNCE_MS = 250
/** Повтор, если клиент Discord был недоступен (вдруг его запустят) */
const RETRY_MS = 20_000
/** Расхождение позиции с экстраполяцией, считающееся перемоткой */
const SEEK_RESEND_THRESHOLD_SECS = 3

function presenceTitle(track: TrackMetadata): string {
  const fileName = track.file_path.split(/[/\\]/).pop()?.replace(/\.[^/.]+$/, '')
  return track.title || fileName || 'LFMusic'
}

// Кэш найденных обложек на сессию: file_path → https-URL (null — не нашлась).
const coverUrlCache = new Map<string, string | null>()

// Rich Presence принимает только https-URL картинок, а обложки библиотеки —
// локальные файлы. Ищем обложку по артисту+названию в iTunes Search API
// (без ключа, отдаёт CORS) и отдаём увеличенный артворк.
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
    // Сеть недоступна — не кэшируем, попробуем в следующий раз.
    return null
  }
}

// Discord Rich Presence: транслирует текущий трек в профиль Discord —
// «Слушает LFMusic», обложка, название, источник и прогресс-бар.
// Прогресс-бар Discord двигает сам по таймстемпам, поэтому шлём обновления
// только на смену трека/источника, паузу и перемотку. На Android — no-op.
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
    // Пока искали обложку, состояние могло смениться — обновление устарело.
    if (seq !== sendSeq) return

    const playing = options.playbackState.value === 'Playing'
    const title = presenceTitle(track)
    const payload: PresencePayload = {
      details: playing ? title : `⏸ ${title}`,
      state: track.artist || 'Неизвестный исполнитель',
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

  // Перемотка: позиция разошлась с экстраполяцией от последней отправки.
  watch(options.positionSecs, (pos) => {
    if (options.playbackState.value !== 'Playing') return
    if (!settings.value.discord_rpc_enabled || !options.currentTrack.value) return
    if (sendTimer) return
    const expected = lastSentPositionSecs + (Date.now() - lastSentAtMs) / 1000
    if (Math.abs(pos - expected) > SEEK_RESEND_THRESHOLD_SECS) scheduleSend()
  })
}
