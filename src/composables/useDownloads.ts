import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { usePlayer } from './usePlayer'
import { useFolders } from './useFolders'

// ──────────────────────────────────────────────────────────────────────────────
// Типы (зеркало serde-структур из src-tauri/src/{external,downloads}.rs)
// ──────────────────────────────────────────────────────────────────────────────

/** Найденный во внешнем источнике трек — кандидат на скачивание. */
export interface ExternalSearchTrack {
  title: string
  artist: string
  album: string | null
  duration_secs: number | null
  /** Прямой URL обложки (Deezer/iTunes CDN) для <img>. */
  cover_url: string | null
  source: string
  /** Трек уже есть в локальной библиотеке (по исполнителю+названию). */
  in_library: boolean
  /** Deezer-id исполнителя — артист трека кликабелен (→ страница скачивания). null у iTunes. */
  artist_id: number | null
  /** Deezer-id альбома — альбом трека кликабелен (→ страница скачивания). null у iTunes. */
  album_id: number | null
}

/** Альбом из внешнего источника (Deezer) — результат поиска. */
export interface ExternalSearchAlbum {
  id: number
  title: string
  artist: string
  artist_id: number | null
  cover_url: string | null
  nb_tracks: number | null
  source: string
}

/** Исполнитель из внешнего источника (Deezer) — результат поиска. */
export interface ExternalSearchArtist {
  id: number
  name: string
  picture_url: string | null
  nb_album: number | null
  source: string
}

/** Детальная карточка альбома (метаданные + треки) для страницы скачивания. */
export interface ExternalAlbumDetail {
  id: number
  title: string
  artist: string
  artist_id: number | null
  cover_url: string | null
  tracks: ExternalSearchTrack[]
}

/** Детальная карточка исполнителя (альбомы + популярные треки). */
export interface ExternalArtistDetail {
  id: number
  name: string
  picture_url: string | null
  albums: ExternalSearchAlbum[]
  top_tracks: ExternalSearchTrack[]
}

/** Страница результатов поиска с явным курсором пагинации (считается по Deezer). */
export interface ExternalSearchPage {
  tracks: ExternalSearchTrack[]
  /** Альбомы (только на первой странице, index === 0). */
  albums: ExternalSearchAlbum[]
  /** Исполнители (только на первой странице, index === 0). */
  artists: ExternalSearchArtist[]
  /** Смещение для следующего запроса (передаётся как index). */
  next_index: number
  /** Есть ли ещё страницы. */
  has_more: boolean
}

export type DownloadStatus = 'queued' | 'downloading' | 'done' | 'error'

/** Элемент очереди скачивания. */
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
  file_path: string | null
}

/** Доступность внешних утилит + статус авторизации YouTube (зеркало downloads.rs::ToolsStatus). */
export interface ToolsStatus {
  ytdlp: boolean
  ffmpeg: boolean
  /** JS-движок Deno (ставится/удаляется вместе с yt-dlp). */
  deno: boolean
  preparing: boolean
  message: string | null
  /** Есть сохранённые cookies (пользователь авторизовался в YouTube). */
  cookies_authorized: boolean
  /** Cookies прошли проверку — скачивание разблокировано. */
  cookies_valid: boolean
  /** Идёт проверка cookies (сетевой тест). */
  cookies_checking: boolean
  /** Открыто окно входа в YouTube. */
  cookies_login_open: boolean
  /** Сообщение об ошибке проверки/авторизации. */
  cookies_message: string | null
}

interface DownloadSnapshot {
  items: DownloadItem[]
  tools: ToolsStatus
}

/** Информация об одной утилите (для страницы настроек и уведомлений). */
export interface ToolInfo {
  /** Доступна (в bin-папке приложения или в системном PATH). */
  installed: boolean
  /** Установлена приложением (в bin-папке) — можно обновлять/удалять. */
  managed: boolean
  /** Версия установленной утилиты, если удалось получить. */
  version: string | null
  /** Доступна более новая версия. Для yt-dlp — и для системного (PATH): при
   *  обновлении скачается управляемая приложением копия; для ffmpeg — только managed. */
  update_available: boolean
}

export interface ToolsInfo {
  ytdlp: ToolInfo
  ffmpeg: ToolInfo
  /** JS-движок Deno (ставится/удаляется с yt-dlp; нужен для YouTube). Полный
   *  ToolInfo — UI смотрит на managed, чтобы разрешить удаление app-managed Deno. */
  deno: ToolInfo
}

export type ToolKey = 'ytdlp' | 'ffmpeg'

interface ToolsProgressEvent {
  tool: ToolKey
  message: string | null
  active: boolean
}

// ──────────────────────────────────────────────────────────────────────────────
// Singleton state
// ──────────────────────────────────────────────────────────────────────────────

const queue = ref<DownloadItem[]>([])
const tools = ref<ToolsStatus>({
  ytdlp: false, ffmpeg: false, deno: false, preparing: false, message: null,
  cookies_authorized: false, cookies_valid: false, cookies_checking: false, cookies_login_open: false, cookies_message: null,
})

const emptyTool = (): ToolInfo => ({ installed: false, managed: false, version: null, update_available: false })
const toolsInfo = ref<ToolsInfo>({ ytdlp: emptyTool(), ffmpeg: emptyTool(), deno: emptyTool() })
// Папка скачиваний — чтобы фронт определял «скачанные приложением» треки (для
// пункта «Удалить трек» в контекстном меню). Грузится один раз в init().
const downloadsDir = ref<string>('')
/** Текст прогресса установки/обновления по утилите (null — не идёт). */
const toolsProgress = ref<Record<ToolKey, string | null>>({ ytdlp: null, ffmpeg: null })
// Статус утилит ещё не загружен — отображаем оптимистично (как будто всё на
// месте), чтобы не было ложной вспышки баннера/блокировки поиска до первой
// (быстрой, локальной) проверки.
const toolsLoaded = ref(false)

// yt-dlp — обязательная зависимость: без него поиск/скачивание недоступны.
const ytdlpMissing = computed(() => toolsLoaded.value && !toolsInfo.value.ytdlp.installed)
// ffmpeg — рекомендованный компонент (лучшее качество), не обязательный.
const ffmpegMissing = computed(() => toolsLoaded.value && !toolsInfo.value.ffmpeg.installed)
// yt-dlp есть, а JS-движок Deno — нет (обязателен для YouTube). Так бывает у тех,
// кто поставил yt-dlp до появления связки с Deno: нужно доустановить Deno.
const denoMissing = computed(() => toolsLoaded.value && toolsInfo.value.ytdlp.installed && !toolsInfo.value.deno.installed)
// Не хватает обязательной зависимости (yt-dlp).
const depsMissing = ytdlpMissing
// Для какой-то из установленных утилит доступно обновление.
const depsUpdateAvailable = computed(() => toolsLoaded.value && (toolsInfo.value.ytdlp.update_available || toolsInfo.value.ffmpeg.update_available))

// ── Авторизация YouTube (обязательная фаза разблокировки скачивания) ─────────
// Вход выполняется в отдельном webview-окне; после него cookies сохраняются и
// проверяются. Статус прилетает снапшотом `download_queue` в `tools`.
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
  // `tools_info` — локальная проверка (без сети): она не знает про обновления и
  // всегда отдаёт update_available=false. Переносим флаг из последнего результата
  // сетевой проверки (checkToolUpdates), пока версия утилиты та же. Иначе
  // refreshToolsInfo (вызывается при входе на «Скачивание»/настройки и по событию
  // tools_changed) затирал бы «доступно обновление» в false — баннер мигал и
  // пропадал, а TTL-кэш checkToolUpdates потом отдавал то же false. Если версия
  // изменилась (утилиту поставили/обновили) — прежний флаг неактуален, оставляем
  // false (после обновления checkToolUpdates всё равно сходит в сеть заново).
  const prev = toolsInfo.value
  for (const key of ['ytdlp', 'ffmpeg', 'deno'] as const) {
    if (info[key].version === prev[key].version && prev[key].update_available) {
      info[key].update_available = true
    }
  }
  toolsInfo.value = info
  toolsLoaded.value = true
  return info
}

// Проверка обновлений утилит ходит в сеть (GitHub). Страница «Скачивание» и
// настройки «Внешние источники» дёргают её при каждом mount — кэшируем результат
// на TTL, чтобы переходы между страницами не порождали лишних запросов. Метку
// ставим только при успехе (без сети повторим при следующем заходе).
const TOOL_UPDATE_TTL_MS = 15 * 60 * 1000
let lastToolUpdateCheck = 0

/**
 * Проверить наличие обновлений утилит (обращается к сети). Результат кэшируется
 * на TTL — повторные вызовы в его пределах возвращают текущий `toolsInfo` без
 * запроса. `force` обходит кэш (после установки/обновления).
 */
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

// Дебаунс обновления библиотеки: при пакетном скачивании событие
// `downloads_library_updated` прилетает на КАЖДЫЙ трек. Без дебаунса очередь из N
// треков вызвала бы N полных перезагрузок каталога. Коалесцируем всплески в одну
// перезагрузку (треки появляются почти сразу — задержка незаметна).
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
    // Синглтон-композабл живёт всё время работы приложения — не отписываемся.
    await listen<DownloadSnapshot>('download_queue', (e) => {
      queue.value = e.payload.items
      tools.value = e.payload.tools
    })
    // После скачивания библиотека пересканирована — обновляем каталог, чтобы
    // новые треки появились без перехода между страницами.
    await listen('downloads_library_updated', () => {
      scheduleLibraryRefresh()
    })
    // Утилиты установлены/обновлены/удалены где-то ещё — обновляем статус.
    await listen('tools_changed', () => {
      void refreshToolsInfo()
    })
    // Прогресс установки/обновления утилиты (страница настроек).
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
  // Локальный статус утилит (быстро, без сети) — для блокировки поиска и баннера.
  try {
    await refreshToolsInfo()
  } catch (e) {
    console.error('Ошибка загрузки статуса утилит:', e)
  }
  // Проверка обновлений (сеть) один раз при запуске — чтобы баннер «доступно
  // обновление» появлялся сразу, без захода на «Скачивание»/в настройки.
  // Fire-and-forget (не задерживаем запуск); TTL-кэш не даст дублировать запрос
  // при последующих заходах на эти страницы.
  void checkToolUpdates().catch(() => { /* без сети — просто без обновлений */ })
  // Путь папки скачиваний — для определения «скачанных приложением» треков.
  try {
    downloadsDir.value = await invoke<string>('downloads_dir_path')
  } catch (e) {
    console.error('Ошибка получения папки скачиваний:', e)
  }
}

/** Трек скачан этим приложением (его файл лежит в папке скачиваний). */
function isDownloadedTrack(filePath: string): boolean {
  const dir = downloadsDir.value
  if (!dir || !filePath) return false
  const norm = (s: string) => s.replace(/\\/g, '/').toLowerCase()
  // Требуем границу-разделитель, иначе соседняя папка с тем же префиксом
  // («…/LFMusic Downloads-old/…») ошибочно считалась бы «скачанной».
  let d = norm(dir)
  if (!d.endsWith('/')) d += '/'
  return norm(filePath).startsWith(d)
}

/** Удалить скачанный приложением трек (из библиотеки и файл с диска). */
async function deleteDownloadedTrack(filePath: string): Promise<void> {
  await invoke('downloads_delete_track', { filePath })
}

/**
 * Сменить папку скачиваний. `folderPath` — абсолютный путь выбранной папки, либо
 * `null`/`undefined` для сброса на стандартную папку «LFMusic Downloads» в
 * системной музыкальной директории. Бэкенд при любой ошибке делает fallback на
 * стандартную папку и возвращает фактически применённый путь — обновляем им
 * локальное состояние. Старая папка убирается из сканируемых, новая добавляется,
 * поэтому перечитываем список папок и каталог.
 */
async function setDownloadsDir(folderPath?: string | null): Promise<string> {
  const resolved = await invoke<string>('downloads_set_dir', { folderPath: folderPath ?? null })
  downloadsDir.value = resolved
  // Набор сканируемых папок и каталог изменились — обновляем оба представления.
  try { await useFolders().refreshFolders() } catch { /* список папок ещё не инициализирован */ }
  try { await usePlayer().refreshLibrary() } catch { /* каталог обновится по событию */ }
  return resolved
}

/**
 * Нормализация строки — порт Rust-функции `external::normalize`: приводит к
 * нижнему регистру, оставляет только буквы/цифры (любых алфавитов), а любые
 * прочие символы схлопывает в одиночные пробелы (без ведущих/хвостовых).
 * Держим идентичной бэкенду, чтобы дедупликация очереди совпадала с ним.
 */
// Кэшируем регулярку: литерал в теле цикла пересоздавался бы на каждом символе.
const ALNUM_RE = /[\p{L}\p{N}]/u

function normalizeKey(s: string): string {
  let out = ''
  let prevSpace = true // схлопываем ведущие пробелы
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
 * Ключ идентичности трека (исполнитель + название + альбом). Совпадает с Rust
 * `external::track_identity_key` = `normalize("{artist} {title} {album}")`.
 * Включает альбом, чтобы одноимённые треки одного исполнителя из РАЗНЫХ альбомов
 * не схлопывались в один — они показываются и качаются по отдельности. Пустой
 * альбом → ключ сводится к «исполнитель + название».
 */
export function trackKey(artist: string, title: string, album?: string | null): string {
  return normalizeKey(`${artist || ''} ${title || ''} ${album || ''}`)
}

export function useDownloads() {
  void init()

  /**
   * Поиск треков во внешних источниках (постранично: index — смещение). При
   * клике по «фантому» альбома известны точные исполнитель и название — передаём
   * их в `exact`, тогда бэкенд добавляет точный поиск по полям Deezer (иначе
   * нишевый трек с «общим» названием, напр. «Интро», теряется в свободной выдаче).
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

  /** Поставить трек в очередь скачивания (force — повторное скачивание). */
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

  /** Поставить пачку треков в очередь одним вызовом (кнопка «Скачать все треки»). */
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

  /** Детальная карточка альбома по Deezer-id (метаданные + треки). null — не найден. */
  async function albumDetail(id: number): Promise<ExternalAlbumDetail | null> {
    return await invoke<ExternalAlbumDetail | null>('downloads_album', { albumId: id })
  }

  /** Детальная карточка исполнителя по Deezer-id (альбомы + популярные треки). */
  async function artistDetail(id: number): Promise<ExternalArtistDetail | null> {
    return await invoke<ExternalArtistDetail | null>('downloads_artist', { artistId: id })
  }

  /** Убрать/отменить элемент очереди. */
  async function remove(id: number): Promise<void> {
    await invoke('downloads_remove', { id })
  }

  /** Повторить скачивание ошибочного элемента очереди (error → queued). */
  async function retry(id: number): Promise<void> {
    await invoke('downloads_retry', { id })
  }

  /** Очистить завершённые/ошибочные элементы. */
  async function clearFinished(): Promise<void> {
    await invoke('downloads_clear_finished')
  }

  /** Проверить доступность утилит (без скачивания). */
  async function checkTools(): Promise<ToolsStatus> {
    const s = await invoke<ToolsStatus>('downloads_check_tools')
    tools.value = s
    return s
  }

  /** Установить/обновить утилиту (скачивает в папку приложения). */
  async function installTool(tool: ToolKey): Promise<void> {
    const info = await invoke<ToolsInfo>('tools_install', { tool })
    toolsInfo.value = info
    // Версия изменилась — следующая проверка обновлений должна сходить в сеть.
    lastToolUpdateCheck = 0
  }

  /** Доустановить Deno (для тех, кто поставил yt-dlp до связки с Deno). */
  async function installDeno(): Promise<void> {
    const info = await invoke<ToolsInfo>('tools_install_deno')
    toolsInfo.value = info
  }

  /** Установить сразу все утилиты (yt-dlp + Deno + ffmpeg) одним действием. */
  async function installAll(): Promise<void> {
    const info = await invoke<ToolsInfo>('tools_install_all')
    toolsInfo.value = info
    // Версии изменились — следующая проверка обновлений должна сходить в сеть.
    lastToolUpdateCheck = 0
  }

  /** Удалить скачанную приложением утилиту. */
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
    trackKey,
  }
}
