import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { usePlayer } from './usePlayer'
import { useFolders } from './useFolders'

// Types mirroring serde structs from src-tauri/src/{external,downloads}.rs

export interface ExternalSearchTrack {
  title: string
  artist: string
  album: string | null
  duration_secs: number | null
  cover_url: string | null
  source: string
  in_library: boolean
  /** Deezer artist id; null for iTunes. */
  artist_id: number | null
  /** Deezer album id; null for iTunes. */
  album_id: number | null
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

/** Search results page with an explicit pagination cursor (Deezer-based). */
export interface ExternalSearchPage {
  tracks: ExternalSearchTrack[]
  /** Only populated on the first page (index === 0). */
  albums: ExternalSearchAlbum[]
  /** Only populated on the first page (index === 0). */
  artists: ExternalSearchArtist[]
  next_index: number
  has_more: boolean
}

export type DownloadStatus = 'queued' | 'downloading' | 'done' | 'error'

/**
 * Mirrors `downloads.rs::DownloadErrorKind`. UI distinguishes causes by type, not
 * text: `bot_wall_network` (IP/VPN block, not an expired login) vs `cookies_expired`
 * (actually needs re-login).
 */
export type DownloadErrorKind =
  | 'bot_wall_network'
  | 'cookies_expired'
  | 'network_timeout'
  | 'no_format'
  | 'tool_missing'
  | 'cancelled'
  | 'unknown'

export interface DownloadItem {
  id: number
  title: string
  artist: string
  album: string | null
  query: string
  cover_url: string | null
  status: DownloadStatus
  progress: number
  error: string | null
  /** null when not an error. */
  error_kind: DownloadErrorKind | null
  file_path: string | null
}

/** Mirrors `downloads.rs::ProxyConfig`. */
export type ProxyMode = 'system' | 'none' | 'custom'
export interface ProxyConfig {
  mode: ProxyMode
  url: string
}

/** Mirrors `downloads.rs::ToolsStatus`. */
export interface ToolsStatus {
  ytdlp: boolean
  ffmpeg: boolean
  /** Installed/removed alongside yt-dlp. */
  deno: boolean
  /** PO-token provider for YouTube's bot-wall — optional, installed separately. */
  pot: boolean
  preparing: boolean
  message: string | null
  cookies_authorized: boolean
  /** Cookies passed validation — downloading is unlocked. */
  cookies_valid: boolean
  cookies_checking: boolean
  cookies_login_open: boolean
  cookies_message: string | null
}

interface DownloadSnapshot {
  items: DownloadItem[]
  tools: ToolsStatus
}

export interface ToolInfo {
  /** In the app's bin folder or on system PATH. */
  installed: boolean
  /** Installed by the app (bin folder) — can be updated/removed. */
  managed: boolean
  version: string | null
  /** For yt-dlp this also covers the system (PATH) case: updating installs an
   *  app-managed copy. For ffmpeg only the managed case applies. */
  update_available: boolean
}

export interface ToolsInfo {
  ytdlp: ToolInfo
  ffmpeg: ToolInfo
  /** UI checks `managed` to allow removing an app-managed Deno install. */
  deno: ToolInfo
  /** PO-token provider — desktop-only, installed independently (not part of "install all"). */
  pot: ToolInfo
}

export type ToolKey = 'ytdlp' | 'ffmpeg' | 'pot'

interface ToolsProgressEvent {
  tool: ToolKey
  message: string | null
  active: boolean
}

// Singleton state

const queue = ref<DownloadItem[]>([])
const tools = ref<ToolsStatus>({
  ytdlp: false, ffmpeg: false, deno: false, pot: false, preparing: false, message: null,
  cookies_authorized: false, cookies_valid: false, cookies_checking: false, cookies_login_open: false, cookies_message: null,
})

const emptyTool = (): ToolInfo => ({ installed: false, managed: false, version: null, update_available: false })
const toolsInfo = ref<ToolsInfo>({ ytdlp: emptyTool(), ffmpeg: emptyTool(), deno: emptyTool(), pot: emptyTool() })
// Used to detect app-downloaded tracks (for "Delete track" in the context menu). Loaded once in init().
const downloadsDir = ref<string>('')
const proxyConfig = ref<ProxyConfig>({ mode: 'system', url: '' })
const toolsProgress = ref<Record<ToolKey, string | null>>({ ytdlp: null, ffmpeg: null, pot: null })
// Optimistic default (as if everything's installed) until the first local check
// completes, to avoid a false banner/search-block flash on startup.
const toolsLoaded = ref(false)

// yt-dlp is a hard dependency: search/download don't work without it.
const ytdlpMissing = computed(() => toolsLoaded.value && !toolsInfo.value.ytdlp.installed)
// ffmpeg is recommended (better quality) but not required.
const ffmpegMissing = computed(() => toolsLoaded.value && !toolsInfo.value.ffmpeg.installed)
// yt-dlp present but Deno missing — happens for installs predating the Deno pairing.
const denoMissing = computed(() => toolsLoaded.value && toolsInfo.value.ytdlp.installed && !toolsInfo.value.deno.installed)
const depsMissing = ytdlpMissing
const depsUpdateAvailable = computed(() => toolsLoaded.value && (toolsInfo.value.ytdlp.update_available || toolsInfo.value.ffmpeg.update_available))
// POT provider is fully optional (opt-in bot-wall mitigation) — not folded into depsMissing/depsUpdateAvailable.
const potMissing = computed(() => toolsLoaded.value && !toolsInfo.value.pot.installed)
const potUpdateAvailable = computed(() => toolsLoaded.value && toolsInfo.value.pot.update_available)

// YouTube login runs in a separate webview window; afterward cookies are saved and
// validated. Status arrives via the `download_queue` snapshot into `tools`.
async function openYoutubeLogin(): Promise<void> {
  await invoke('downloads_youtube_login')
}
async function validateCookies(): Promise<void> {
  await invoke('downloads_validate_cookies')
}
async function logoutYoutube(): Promise<void> {
  await invoke('downloads_youtube_logout')
}
const cookiesAuthorized = computed(() => tools.value.cookies_authorized)
const cookiesChecking = computed(() => tools.value.cookies_checking)
const cookiesValid = computed(() => tools.value.cookies_valid)
const cookiesLoginOpen = computed(() => tools.value.cookies_login_open)
const cookiesMessage = computed(() => tools.value.cookies_message)

async function refreshToolsInfo(): Promise<ToolsInfo> {
  const info = await invoke<ToolsInfo>('tools_info')
  // `tools_info` is a local (offline) check that always reports update_available=false.
  // Carry over the flag from the last network check (checkToolUpdates) as long as the
  // version hasn't changed, otherwise the banner would flicker off until the TTL cache
  // re-checks. If the version changed, the old flag is stale so we drop it.
  const prev = toolsInfo.value
  for (const key of ['ytdlp', 'ffmpeg', 'deno', 'pot'] as const) {
    if (info[key].version === prev[key].version && prev[key].update_available) {
      info[key].update_available = true
    }
  }
  toolsInfo.value = info
  toolsLoaded.value = true
  return info
}

// Update check hits the network (GitHub). Both the Downloads page and External
// Sources settings call this on every mount, so cache on a TTL to avoid duplicate
// requests. The timestamp is only set on success (retry next visit if offline).
const TOOL_UPDATE_TTL_MS = 15 * 60 * 1000
let lastToolUpdateCheck = 0

/** `force` bypasses the TTL cache (used right after install/update). */
async function checkToolUpdates(force = false): Promise<ToolsInfo> {
  if (!force && lastToolUpdateCheck && Date.now() - lastToolUpdateCheck < TOOL_UPDATE_TTL_MS) {
    return toolsInfo.value
  }
  const info = await invoke<ToolsInfo>('tools_check_updates')
  toolsInfo.value = info
  toolsLoaded.value = true
  lastToolUpdateCheck = Date.now()
  return info
}

let initialized = false

// Debounced: `downloads_library_updated` fires per track during a batch download,
// so without this an N-track queue would trigger N full catalog reloads.
let libraryRefreshTimer: ReturnType<typeof setTimeout> | null = null
function scheduleLibraryRefresh(): void {
  if (libraryRefreshTimer) clearTimeout(libraryRefreshTimer)
  libraryRefreshTimer = setTimeout(() => {
    libraryRefreshTimer = null
    void usePlayer().refreshLibrary()
  }, 400)
}

async function init(): Promise<void> {
  if (initialized) return
  initialized = true
  try {
    // Singleton composable lives for the app's lifetime — never unsubscribed.
    await listen<DownloadSnapshot>('download_queue', (e) => {
      queue.value = e.payload.items
      tools.value = e.payload.tools
    })
    await listen('downloads_library_updated', () => {
      scheduleLibraryRefresh()
    })
    await listen('tools_changed', () => {
      void refreshToolsInfo()
    })
    await listen<ToolsProgressEvent>('tools_progress', (e) => {
      toolsProgress.value = {
        ...toolsProgress.value,
        [e.payload.tool]: e.payload.active ? e.payload.message : null,
      }
    })
  } catch (e) {
    console.error('Ошибка подписки на очередь скачивания:', e)
  }
  try {
    const snap = await invoke<DownloadSnapshot>('downloads_snapshot')
    queue.value = snap.items
    tools.value = snap.tools
  } catch (e) {
    console.error('Ошибка загрузки очереди скачивания:', e)
  }
  try {
    await refreshToolsInfo()
  } catch (e) {
    console.error('Ошибка загрузки статуса утилит:', e)
  }
  // Fire-and-forget so startup isn't delayed; TTL cache avoids duplicate requests
  // from later page visits.
  void checkToolUpdates().catch(() => { /* offline — skip update banner */ })
  try {
    downloadsDir.value = await invoke<string>('downloads_dir_path')
  } catch (e) {
    console.error('Ошибка получения папки скачиваний:', e)
  }
  try {
    proxyConfig.value = await invoke<ProxyConfig>('downloads_get_proxy')
  } catch (e) {
    console.error('Ошибка получения настроек прокси:', e)
  }
}

function isDownloadedTrack(filePath: string): boolean {
  const dir = downloadsDir.value
  if (!dir || !filePath) return false
  const norm = (s: string) => s.replace(/\\/g, '/').toLowerCase()
  // Require a separator boundary so a sibling folder sharing the same prefix
  // (e.g. "…/LFMusic Downloads-old/…") isn't mistaken for the downloads folder.
  let d = norm(dir)
  if (!d.endsWith('/')) d += '/'
  return norm(filePath).startsWith(d)
}

async function deleteDownloadedTrack(filePath: string): Promise<void> {
  await invoke('downloads_delete_track', { filePath })
}

/**
 * `folderPath` null/undefined resets to the default "LFMusic Downloads" folder.
 * Backend falls back to the default on any error and returns the path actually
 * applied. Old folder is unscanned, new one added, so refresh folders + catalog.
 */
async function setDownloadsDir(folderPath?: string | null): Promise<string> {
  const resolved = await invoke<string>('downloads_set_dir', { folderPath: folderPath ?? null })
  downloadsDir.value = resolved
  try { await useFolders().refreshFolders() } catch { /* folder list not yet initialized */ }
  try { await usePlayer().refreshLibrary() } catch { /* catalog will refresh via event */ }
  return resolved
}

/** `url` only applies to `mode: 'custom'` (e.g. socks5://127.0.0.1:1080 for ByeDPI). */
async function setProxy(mode: ProxyMode, url = ''): Promise<void> {
  await invoke('downloads_set_proxy', { mode, url })
  proxyConfig.value = { mode, url: mode === 'custom' ? url.trim() : '' }
}

/**
 * Port of Rust `external::normalize`. Kept identical to the backend so queue
 * dedup matches it exactly.
 */
const ALNUM_RE = /[\p{L}\p{N}]/u

function normalizeKey(s: string): string {
  let out = ''
  let prevSpace = true // collapse leading spaces too
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

/**
 * Mirrors Rust `external::track_identity_key`. Includes album so same-titled
 * tracks from different albums by the same artist aren't collapsed together.
 */
export function trackKey(artist: string, title: string, album?: string | null): string {
  return normalizeKey(`${artist || ''} ${title || ''} ${album || ''}`)
}

export function useDownloads() {
  void init()

  /**
   * `exact` is passed when the artist/title are already known (e.g. clicking a
   * phantom album track) so the backend adds a precise Deezer field search —
   * otherwise a niche track with a generic title (e.g. "Intro") gets lost in
   * free-text results.
   */
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

  async function enqueue(track: ExternalSearchTrack, force = false): Promise<number> {
    return await invoke<number>('downloads_enqueue', {
      request: {
        title: track.title,
        artist: track.artist,
        album: track.album,
        cover_url: track.cover_url,
        force,
      },
    })
  }

  async function enqueueMany(tracks: ExternalSearchTrack[], force = false): Promise<number[]> {
    if (tracks.length === 0) return []
    return await invoke<number[]>('downloads_enqueue_many', {
      requests: tracks.map(t => ({
        title: t.title,
        artist: t.artist,
        album: t.album,
        cover_url: t.cover_url,
        force,
      })),
    })
  }

  async function albumDetail(id: number): Promise<ExternalAlbumDetail | null> {
    return await invoke<ExternalAlbumDetail | null>('downloads_album', { albumId: id })
  }

  async function artistDetail(id: number): Promise<ExternalArtistDetail | null> {
    return await invoke<ExternalArtistDetail | null>('downloads_artist', { artistId: id })
  }

  async function remove(id: number): Promise<void> {
    await invoke('downloads_remove', { id })
  }

  /** error -> queued */
  async function retry(id: number): Promise<void> {
    await invoke('downloads_retry', { id })
  }

  async function clearFinished(): Promise<void> {
    await invoke('downloads_clear_finished')
  }

  async function checkTools(): Promise<ToolsStatus> {
    const s = await invoke<ToolsStatus>('downloads_check_tools')
    tools.value = s
    return s
  }

  async function installTool(tool: ToolKey): Promise<void> {
    const info = await invoke<ToolsInfo>('tools_install', { tool })
    toolsInfo.value = info
    // Version changed — force the next update check to hit the network.
    lastToolUpdateCheck = 0
  }

  async function installDeno(): Promise<void> {
    const info = await invoke<ToolsInfo>('tools_install_deno')
    toolsInfo.value = info
  }

  async function installAll(): Promise<void> {
    const info = await invoke<ToolsInfo>('tools_install_all')
    toolsInfo.value = info
    // Versions changed — force the next update check to hit the network.
    lastToolUpdateCheck = 0
  }

  async function deleteTool(tool: ToolKey): Promise<void> {
    const info = await invoke<ToolsInfo>('tools_delete', { tool })
    toolsInfo.value = info
  }

  return {
    queue,
    tools,
    toolsInfo,
    toolsProgress,
    toolsLoaded,
    depsMissing,
    depsUpdateAvailable,
    ytdlpMissing,
    ffmpegMissing,
    denoMissing,
    potMissing,
    potUpdateAvailable,
    openYoutubeLogin,
    validateCookies,
    logoutYoutube,
    cookiesAuthorized,
    cookiesChecking,
    cookiesValid,
    cookiesLoginOpen,
    cookiesMessage,
    search,
    enqueue,
    enqueueMany,
    albumDetail,
    artistDetail,
    remove,
    retry,
    clearFinished,
    checkTools,
    refreshToolsInfo,
    checkToolUpdates,
    installTool,
    installDeno,
    installAll,
    deleteTool,
    isDownloadedTrack,
    deleteDownloadedTrack,
    downloadsDir,
    setDownloadsDir,
    proxyConfig,
    setProxy,
    trackKey,
  }
}
