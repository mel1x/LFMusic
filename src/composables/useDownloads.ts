import { ref, computed, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { usePlayer } from './usePlayer'
import { useFolders } from './useFolders'
import { useI18n } from './useI18n'
import { isAndroidRuntime } from './usePlatform'

// Types mirroring the serde structs in src-tauri/src/{external.rs, downloader/}

/** Catalog search hit (Deezer/iTunes) or a pasted-link entry. */
export interface ExternalSearchTrack {
  title: string
  artist: string
  album: string | null
  duration_secs: number | null
  cover_url: string | null
  source: string
  in_library: boolean
  /** Deezer ids (null for iTunes / links). */
  artist_id: number | null
  album_id: number | null
  genre: string | null
  /** Set for pasted links: download exactly this URL instead of searching. */
  source_url: string | null
  /** Video thumbnail for link entries (used as cover fallback). */
  thumbnail_url?: string | null
}

export interface ExternalSearchAlbum {
  id: number
  title: string
  artist: string
  artist_id: number | null
  cover_url: string | null
  nb_tracks: number | null
  source: string
}

export interface ExternalSearchArtist {
  id: number
  name: string
  picture_url: string | null
  nb_album: number | null
  source: string
}

export interface ExternalAlbumDetail {
  id: number
  title: string
  artist: string
  artist_id: number | null
  cover_url: string | null
  tracks: ExternalSearchTrack[]
}

export interface ExternalArtistDetail {
  id: number
  name: string
  picture_url: string | null
  albums: ExternalSearchAlbum[]
  top_tracks: ExternalSearchTrack[]
}

export interface ExternalSearchPage {
  tracks: ExternalSearchTrack[]
  /** Only on the first page. */
  albums: ExternalSearchAlbum[]
  artists: ExternalSearchArtist[]
  next_index: number
  has_more: boolean
}

interface LinkTrack {
  title: string
  artist: string
  album: string | null
  duration_secs: number | null
  source_url: string
  thumbnail_url: string | null
  in_library: boolean
}

export interface LinkResult {
  /** Playlist/album title when the link had several entries. */
  collection: string | null
  tracks: ExternalSearchTrack[]
}

export type DownloadStage = 'queued' | 'searching' | 'downloading' | 'processing' | 'done' | 'error'

/** Mirrors `downloader::ErrorKind`. */
export type DownloadErrorKind =
  | 'tool_missing'
  | 'not_found'
  | 'bot_check'
  | 'cookies'
  | 'auth_required'
  | 'unavailable'
  | 'rate_limited'
  | 'network'
  | 'format'
  | 'outdated'
  | 'disk'
  | 'import'
  | 'cancelled'
  | 'unknown'

export interface DownloadItem {
  id: number
  title: string
  artist: string
  album: string | null
  duration_secs: number | null
  cover_url: string | null
  album_id: number | null
  genre: string | null
  source_url: string | null
  thumbnail_url: string | null
  stage: DownloadStage
  /** 0..1 within the download step. */
  progress: number
  speed_bps: number | null
  eta_secs: number | null
  /** "YouTube" / "SoundCloud" / link host. */
  source: string | null
  media_url: string | null
  error: { kind: DownloadErrorKind; message: string } | null
  file_path: string | null
}

export type ToolKey = 'ytdlp' | 'deno' | 'ffmpeg'

export interface ToolInfo {
  installed: boolean
  /** App-managed copy (can be updated/removed). */
  managed: boolean
  version: string | null
  latest: string | null
  update_available: boolean
}

export interface ToolsState {
  info: { ytdlp: ToolInfo; deno: ToolInfo; ffmpeg: ToolInfo; unsupported: boolean }
  busy: boolean
  progress: { tool: ToolKey; message: string; fraction: number | null } | null
  error: string | null
}

/** Mirrors `settings::DownloaderSettings`. */
export type ProxyMode = 'system' | 'none' | 'custom'
export type CookiesMode = 'none' | 'browser' | 'file'
export interface DownloaderSettings {
  proxy_mode: ProxyMode
  proxy_url: string
  cookies_mode: CookiesMode
  cookies_browser: string
  cookies_file: string
  auto_update: boolean
}

// ── Singleton state ─────────────────────────────────────────────────────────

const emptyTool = (): ToolInfo => ({ installed: false, managed: false, version: null, latest: null, update_available: false })

const queue = ref<DownloadItem[]>([])
const tools = ref<ToolsState>({
  info: { ytdlp: emptyTool(), deno: emptyTool(), ffmpeg: emptyTool(), unsupported: false },
  busy: false,
  progress: null,
  error: null,
})
// Optimistic until the first local check, so the page doesn't flash a setup card.
const toolsLoaded = ref(false)
const dlSettings = ref<DownloaderSettings>({
  proxy_mode: 'system',
  proxy_url: '',
  cookies_mode: 'none',
  cookies_browser: 'firefox',
  cookies_file: '',
  auto_update: true,
})

// Seeded from the last known value so settings render without a shimmer.
const DOWNLOADS_DIR_CACHE_KEY = 'lfmusic:downloads-dir'
const downloadsDir = ref<string>(localStorage.getItem(DOWNLOADS_DIR_CACHE_KEY) ?? '')
watch(downloadsDir, (dir) => { if (dir) localStorage.setItem(DOWNLOADS_DIR_CACHE_KEY, dir) })

// The downloader needs yt-dlp as a subprocess — never available on Android.
const ytdlpMissing = computed(() => isAndroidRuntime() || (toolsLoaded.value && !tools.value.info.ytdlp.installed))
/** Without Deno, YouTube's signature challenge can't be solved. */
const denoMissing = computed(() => toolsLoaded.value && tools.value.info.ytdlp.installed && !tools.value.info.deno.installed)
const ffmpegMissing = computed(() => toolsLoaded.value && !tools.value.info.ffmpeg.installed)
const toolsUpdateAvailable = computed(() =>
  tools.value.info.ytdlp.update_available || tools.value.info.deno.update_available,
)
const activeCount = computed(() =>
  queue.value.filter(i => i.stage !== 'done' && i.stage !== 'error').length,
)

/**
 * Port of Rust `external::normalize` / `track_identity_key` — must stay
 * identical so queue lookups agree with the backend's dedup.
 */
const ALNUM_RE = /[\p{L}\p{N}]/u
function normalizeKey(s: string): string {
  let out = ''
  let prevSpace = true
  for (const ch of s.toLowerCase()) {
    if (ALNUM_RE.test(ch)) {
      out += ch
      prevSpace = false
    } else if (!prevSpace) {
      out += ' '
      prevSpace = true
    }
  }
  return out.replace(/ +$/, '')
}

export function trackKey(artist: string, title: string, album?: string | null): string {
  return normalizeKey(`${artist || ''} ${title || ''} ${album || ''}`)
}

function keyOf(t: { artist: string; title: string; album: string | null; source_url?: string | null }): string {
  return t.source_url ? `url:${t.source_url.trim()}` : trackKey(t.artist, t.title, t.album)
}

/** Queue item per track key (latest wins), for row status lookups. */
const queueIndex = computed(() => {
  const map = new Map<string, DownloadItem>()
  for (const item of queue.value) map.set(keyOf(item), item)
  return map
})

// ── Init ────────────────────────────────────────────────────────────────────

let initialized = false
let libraryRefreshTimer: ReturnType<typeof setTimeout> | null = null

// `downloads_library_updated` fires per finished track; one catalog reload per burst.
function scheduleLibraryRefresh(): void {
  if (libraryRefreshTimer) clearTimeout(libraryRefreshTimer)
  libraryRefreshTimer = setTimeout(() => {
    libraryRefreshTimer = null
    void usePlayer().refreshLibrary()
  }, 400)
}

async function refreshTools(): Promise<ToolsState> {
  const state = await invoke<ToolsState>('tools_status')
  tools.value = state
  toolsLoaded.value = true
  return state
}

// Update checks hit GitHub — at most once per TTL from page visits.
const UPDATE_CHECK_TTL_MS = 30 * 60 * 1000
let lastUpdateCheck = 0
async function checkUpdates(force = false): Promise<void> {
  if (!force && Date.now() - lastUpdateCheck < UPDATE_CHECK_TTL_MS) return
  if (!tools.value.info.ytdlp.installed && !force) return
  lastUpdateCheck = Date.now()
  try {
    tools.value = await invoke<ToolsState>('tools_check_updates')
  } catch {
    lastUpdateCheck = 0
  }
}

async function init(): Promise<void> {
  if (initialized || isAndroidRuntime()) return
  initialized = true
  try {
    // App-lifetime singleton — never unsubscribed.
    await listen<{ items: DownloadItem[] }>('downloads_queue', (e) => { queue.value = e.payload.items })
    await listen<ToolsState>('downloads_tools', (e) => {
      tools.value = e.payload
      toolsLoaded.value = true
    })
    await listen('downloads_library_updated', () => scheduleLibraryRefresh())
  } catch (e) {
    console.error('Ошибка подписки на события загрузок:', e)
  }
  try {
    queue.value = (await invoke<{ items: DownloadItem[] }>('downloads_snapshot')).items
  } catch (e) {
    console.error('Ошибка загрузки очереди:', e)
  }
  try { await refreshTools() } catch (e) { console.error('Ошибка проверки компонентов:', e) }
  try { downloadsDir.value = await invoke<string>('downloads_dir_path') } catch { /* keep cached */ }
  try { dlSettings.value = await invoke<DownloaderSettings>('downloads_get_settings') } catch { /* defaults */ }
}

// ── Operations ──────────────────────────────────────────────────────────────

function toRequest(t: ExternalSearchTrack, force: boolean) {
  return {
    title: t.title,
    artist: t.artist,
    album: t.album,
    duration_secs: t.duration_secs,
    cover_url: t.cover_url,
    album_id: t.album_id,
    genre: t.genre,
    source_url: t.source_url,
    thumbnail_url: t.thumbnail_url ?? null,
    force,
  }
}

async function enqueue(tracks: ExternalSearchTrack | ExternalSearchTrack[], force = false): Promise<number[]> {
  const list = Array.isArray(tracks) ? tracks : [tracks]
  if (list.length === 0) return []
  return await invoke<number[]>('downloads_enqueue', { requests: list.map(t => toRequest(t, force)) })
}

async function search(
  query: string,
  index = 0,
  limit = 30,
  exact?: { artist: string; title: string },
): Promise<ExternalSearchPage> {
  const q = query.trim()
  if (!q) return { tracks: [], albums: [], artists: [], next_index: index, has_more: false }
  return await invoke<ExternalSearchPage>('downloads_search', {
    query: q,
    index,
    limit,
    artist: exact?.artist || undefined,
    title: exact?.title || undefined,
  })
}

async function resolveLink(url: string): Promise<LinkResult> {
  const r = await invoke<{ collection: string | null; tracks: LinkTrack[] }>('downloads_resolve_link', { url })
  return {
    collection: r.collection,
    tracks: r.tracks.map(t => ({
      title: t.title,
      artist: t.artist,
      album: t.album,
      duration_secs: t.duration_secs,
      cover_url: t.thumbnail_url,
      source: 'link',
      in_library: t.in_library,
      artist_id: null,
      album_id: null,
      genre: null,
      source_url: t.source_url,
      thumbnail_url: t.thumbnail_url,
    })),
  }
}

async function installTools(which?: ToolKey[]): Promise<void> {
  tools.value = await invoke<ToolsState>('tools_install', { tools: which })
  lastUpdateCheck = Date.now()
}

async function removeTools(): Promise<void> {
  tools.value = await invoke<ToolsState>('tools_remove')
}

async function saveSettings(patch: Partial<DownloaderSettings>): Promise<void> {
  const next = { ...dlSettings.value, ...patch }
  await invoke('downloads_set_settings', { settings: next })
  dlSettings.value = next
}

function isDownloadedTrack(filePath: string): boolean {
  const dir = downloadsDir.value
  if (!dir || !filePath) return false
  const norm = (s: string) => s.replace(/\\/g, '/').toLowerCase()
  let d = norm(dir)
  if (!d.endsWith('/')) d += '/'
  return norm(filePath).startsWith(d)
}

async function deleteDownloadedTrack(filePath: string): Promise<void> {
  await invoke('downloads_delete_track', { filePath })
}

/** `folderPath` null resets to the default "LFMusic Downloads" folder. */
async function setDownloadsDir(folderPath?: string | null): Promise<string> {
  const resolved = await invoke<string>('downloads_set_dir', { folderPath: folderPath ?? null })
  downloadsDir.value = resolved
  try { await useFolders().refreshFolders() } catch { /* not initialized yet */ }
  try { await usePlayer().refreshLibrary() } catch { /* refreshes via event */ }
  return resolved
}

/** Queue entry for a catalog/link track, if any. */
function itemFor(t: ExternalSearchTrack): DownloadItem | undefined {
  return queueIndex.value.get(keyOf(t))
}

// ── Page helpers ────────────────────────────────────────────────────────────

type KeyedTrack = { artist: string; title: string; album: string | null }

function trackKeyOf(t: KeyedTrack): string {
  return trackKey(t.artist, t.title, t.album)
}

/** Tracks that are neither in the library nor queued (failed items don't count as queued). */
function notQueued(tracks: ExternalSearchTrack[]): ExternalSearchTrack[] {
  const queued = new Set(queue.value.filter(i => i.stage !== 'error').map(trackKeyOf))
  return tracks.filter(tr => !tr.in_library && !queued.has(trackKeyOf(tr)))
}

/**
 * Marks tracks as "in library" once their download finishes, so a row stays
 * "Downloaded" after the queue's "done" entry is cleared (`in_library` is a
 * one-shot snapshot from the initial fetch, never refreshed on its own).
 */
export function useKeepDownloadedInLibrary(lists: () => ExternalSearchTrack[][]) {
  watch(queue, (items) => {
    const done = new Set(items.filter(i => i.stage === 'done').map(trackKeyOf))
    if (!done.size) return
    for (const list of lists()) {
      for (const tr of list) if (!tr.in_library && done.has(trackKeyOf(tr))) tr.in_library = true
    }
  })
}

/** "Already in library, download again?" confirmation for a single track. */
export function useRedownloadConfirm(beforeEnqueue?: () => void) {
  const { t } = useI18n()
  const confirmTrack = ref<ExternalSearchTrack | null>(null)
  const confirmMessage = computed(() =>
    confirmTrack.value ? t('downloads.redownloadText', { title: confirmTrack.value.title }) : '',
  )
  async function confirmRedownload() {
    const track = confirmTrack.value
    confirmTrack.value = null
    if (!track) return
    beforeEnqueue?.()
    try { await enqueue(track, true) } catch (e) { console.error('Ошибка повторного скачивания:', e) }
  }
  return { confirmTrack, confirmMessage, confirmRedownload }
}

// ── Display helpers ─────────────────────────────────────────────────────────

function formatBytes(bps: number): string {
  if (bps >= 1024 * 1024) return `${(bps / 1024 / 1024).toFixed(1)} МБ/с`
  return `${Math.max(1, Math.round(bps / 1024))} КБ/с`
}

export function useDownloads() {
  void init()
  const { t } = useI18n()

  function errorText(item: Pick<DownloadItem, 'error'>): string {
    const kind = item.error?.kind ?? 'unknown'
    const key = `downloads.errors.${kind}`
    const text = t(key)
    return text && text !== key ? text : (item.error?.message || t('downloads.errors.unknown'))
  }

  /** One-line status for an active/finished queue item. */
  function stageText(item: DownloadItem): string {
    switch (item.stage) {
      case 'queued': return t('downloads.stage.queued')
      case 'searching': return item.source
        ? t('downloads.stage.searchingOn', { source: item.source })
        : t('downloads.stage.searching')
      case 'downloading': {
        const pct = `${Math.round(item.progress * 100)}%`
        return item.speed_bps ? `${pct} · ${formatBytes(item.speed_bps)}` : pct
      }
      case 'processing': return t('downloads.stage.processing')
      case 'done': return item.source ? t('downloads.stage.doneFrom', { source: item.source }) : t('downloads.stage.done')
      case 'error': return errorText(item)
    }
  }

  return {
    queue,
    tools,
    toolsLoaded,
    dlSettings,
    downloadsDir,
    ytdlpMissing,
    denoMissing,
    ffmpegMissing,
    toolsUpdateAvailable,
    activeCount,
    search,
    resolveLink,
    albumDetail: (id: number) => invoke<ExternalAlbumDetail | null>('downloads_album', { albumId: id }),
    artistDetail: (id: number) => invoke<ExternalArtistDetail | null>('downloads_artist', { artistId: id }),
    artistTracks: (id: number, index: number, limit: number, name?: string) =>
      invoke<ExternalSearchPage>('downloads_artist_tracks', { artistId: id, artistName: name || undefined, index, limit }),
    enqueue,
    notQueued,
    cancel: (id: number) => invoke('downloads_cancel', { id }),
    cancelAll: () => invoke('downloads_cancel_all'),
    retry: (ids?: number[]) => invoke('downloads_retry', { ids }),
    clearFinished: () => invoke('downloads_clear_finished'),
    refreshTools,
    checkUpdates,
    installTools,
    removeTools,
    saveSettings,
    isDownloadedTrack,
    deleteDownloadedTrack,
    setDownloadsDir,
    itemFor,
    trackKey,
    errorText,
    stageText,
  }
}
