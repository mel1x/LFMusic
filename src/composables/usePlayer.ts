import { ref, shallowRef, reactive, computed, watch, effectScope } from "vue";
import { invoke } from "@tauri-apps/api/core";

import { useSettings } from "./useSettings";
import { useFolders } from "./useFolders";
import { saveMixQueuePaths, loadMixQueuePaths } from "./useMix";
import { usePlaylists } from "./usePlaylists";
import { useCoverLoader } from "./useCoverLoader";
import { emitTrackStatisticsUpdated } from "./useStatistics";
import router from "../router";

// ──────────────────────────────────────────────────────────────────────────────
// Типы
// ──────────────────────────────────────────────────────────────────────────────

export interface SavedPlayerState {
  current_track_path: string | null;
  position_secs: number;
  volume: number;
  shuffled: boolean;
  repeat_mode: string;
  active_playlist_source: string | null;
  current_track_index: number | null;
  current_route: string | null;
}

export interface TrackMetadata {
  title: string | null;
  artist: string | null;
  album: string | null;
  genre: string | null;
  duration_secs: number | null;
  format: string;
  sample_rate: number | null;
  bit_depth: number | null;
  channels: number | null;
  bitrate: number | null;
  is_lossless: boolean;
  file_path: string;
  cover_art: string | null;
  /** Frontend-only: обложка проставлена из внешнего источника (а не встроенная/
   *  override). Нужно, чтобы при выключении внешних обложек откатить её, не
   *  трогая локальные обложки. */
  cover_external?: boolean;
  accent_color: string | null;
  bpm: number | null;
  loudness: number | null;
  energy: number | null;
  danceability: number | null;
  spectral_centroid: number | null;
  key: string | null;
  audio_fingerprint: string | null;
}

export interface PlayerStatus {
  state: "Stopped" | "Playing" | "Paused";
  volume: number;
  position_secs: number;
  current_file_path: string | null;
}

export interface PlayTrackOptions {
  skipAttempt?: boolean;
  orderIndex?: number;
  finishPrevious?: boolean;
  manualListStart?: boolean;
  searchStart?: boolean;
}

interface TrackStatisticsDelta {
  filePath: string;
  earlySkipCountDelta?: number;
  skipCountDelta?: number;
  listenCountDelta?: number;
  fullListenCountDelta?: number;
  playlistAddCountDelta?: number;
  searchStartCountDelta?: number;
  manualStartCountDelta?: number;
  repeatEnabledCountDelta?: number;
  moveToTopCountDelta?: number;
  morningListenCountDelta?: number;
  dayListenCountDelta?: number;
  eveningListenCountDelta?: number;
  nightListenCountDelta?: number;
  totalListenedSecsDelta?: number;
}

interface PlaybackStatsSession {
  filePath: string;
  durationSecs: number;
  startedAtPositionSecs: number;
  lastPositionSecs: number;
  maxPositionSecs: number;
  listenedSecs: number;
  reportedListenedSecs: number;
  listenThresholdSecs: number;
  listenRecorded: boolean;
  fullRecorded: boolean;
  lastWallClockMs: number | null;
}

type CollectionStatsKind = "album" | "playlist";

interface CollectionStatsSession {
  kind: CollectionStatsKind;
  key: string;
  signature: string;
  trackPaths: Set<string>;
  completedPaths: Set<string>;
}

interface AudioFeatureProgressStatus {
  current: number;
  total: number;
  percent: number;
  active: boolean;
}

// ──────────────────────────────────────────────────────────────────────────────
// Утилиты
// ──────────────────────────────────────────────────────────────────────────────

function extractFileName(filePath: string): string {
  const parts = filePath.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1].replace(/\.[^/.]+$/, "");
}

function formatTime(seconds: number): string {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

function listenThresholdSecs(durationSecs: number | null | undefined): number {
  return typeof durationSecs === "number" && Number.isFinite(durationSecs) && durationSecs > 0
    ? Math.min(90, durationSecs / 3)
    : 90;
}

function dayPartListenDelta(): Partial<TrackStatisticsDelta> {
  const hour = new Date().getHours();
  if (hour >= 5 && hour < 12) return { morningListenCountDelta: 1 };
  if (hour >= 12 && hour < 18) return { dayListenCountDelta: 1 };
  if (hour >= 18 && hour < 23) return { eveningListenCountDelta: 1 };
  return { nightListenCountDelta: 1 };
}

function createTrackDelta(filePath: string): TrackStatisticsDelta {
  return { filePath };
}

function hasTrackDelta(delta: TrackStatisticsDelta): boolean {
  return Object.entries(delta).some(([key, value]) =>
    key !== "filePath" && typeof value === "number" && value !== 0
  );
}

function roundedSeconds(seconds: number): number {
  return Math.round(Math.max(0, seconds) * 1000) / 1000;
}

// ──────────────────────────────────────────────────────────────────────────────
// Singleton state (shared between all consumers)
// ──────────────────────────────────────────────────────────────────────────────

const currentTrack = ref<TrackMetadata | null>(null);
const playbackState = ref<"Stopped" | "Playing" | "Paused">("Stopped");
const volume = ref(1.0);
const positionSecs = ref(0);
const isLoading = ref(false);
const errorMessage = ref("");
// Лёгкий упорядоченный список путей всего каталога (канонический порядок БД).
// Заменяет прежний полный реактивный массив объектов: идентичность/счётчик/
// источник очереди каталога без хранения 10k тяжёлых реактивных объектов.
const catalogPaths = shallowRef<string[]>([]);
const isSeeking = ref(false);
const seekValue = ref(0);
let statusInterval: ReturnType<typeof setInterval> | null = null;
let stopStatusWatcher: (() => void) | null = null;
let isHandlingTrackEnd = false;
const STATUS_POLL_MS = 500;

let lastFrameTime = performance.now();
let animationFrameId: number | null = null;

function updateSmoothProgress() {
  if (playbackState.value !== "Playing") {
    animationFrameId = null;
    return;
  }
  const now = performance.now();
  const delta = (now - lastFrameTime) / 1000;
  lastFrameTime = now;

  if (!isSeeking.value) {
    if (delta > 0 && delta < 1) {
      const duration = currentTrack.value?.duration_secs ?? 0;
      if (duration > 0) {
        positionSecs.value = Math.min(duration, positionSecs.value + delta);
      } else {
        positionSecs.value += delta;
      }
    }
  }
  animationFrameId = requestAnimationFrame(updateSmoothProgress);
}

function startSmoothProgress() {
  if (animationFrameId === null) {
    lastFrameTime = performance.now();
    animationFrameId = requestAnimationFrame(updateSmoothProgress);
  }
}

function stopSmoothProgress() {
  if (animationFrameId !== null) {
    cancelAnimationFrame(animationFrameId);
    animationFrameId = null;
  }
}

// Shuffle / Repeat
const shuffled = ref(false);
const repeatMode = ref<"queue" | "track">("queue");
// playbackOrder — плоская очередь воспроизведения (пути file_path).
// В режиме shuffle хвост очереди дозаполняется заранее сгенерированными
// решафл-проходами, чтобы всегда было видно не менее QUEUE_LOOKAHEAD
// треков после текущего. Хранит только пути — объекты резолвятся через кэш.
const playbackOrder = shallowRef<string[]>([]);
// Явный указатель на позицию текущего трека в playbackOrder.
// Нужен, потому что в shuffle-режиме тот же трек может встречаться
// в нескольких проходах — findIndex не годится.
const currentOrderIndex = ref<number>(-1);

// Состояние библиотеки / настроек
const showSettings = ref(false);

// Источник воспроизведения: 'catalog' | 'playlist:ID' | null
const activePlaylistSource = ref<string | null>(null);
// Объекты треков текущего НЕ-каталожного источника (плейлист/альбом/артист/микс).
// Ограничены по размеру, поэтому держим объекты целиком; для каталога не используется
// (там источник — пути из catalogPaths/sourcePaths).
const sourceTrackList = ref<TrackMetadata[]>([]);
// Упорядоченные пути активного источника (драйвер очереди для ЛЮБОГО источника).
// Каталог → отфильтрованные/полные пути; иначе → пути из sourceTrackList.
const sourcePaths = shallowRef<string[]>([]);

// ──────────────────────────────────────────────────────────────────────────────
// Реактивный LRU-кэш метаданных треков (материализация по требованию)
// ──────────────────────────────────────────────────────────────────────────────

// Реактивная Map: добавление записи дотриггеривает computed, которые резолвили
// путь через getTrackByPath и получили undefined (Vue 3 отслеживает доступ по
// ключу, в т.ч. промахи). Хранит и метаданные, и лениво подгруженные обложки —
// вытеснение по LRU ограничивает суммарную память (главное — base64-обложки).
const trackCache = reactive(new Map<string, TrackMetadata>());
const TRACK_CACHE_LIMIT = 3000;
// Неактивная (нереактивная) метка последнего доступа — для LRU без лишних
// срабатываний реактивности.
const cacheAccessTick = new Map<string, number>();
let cacheTick = 0;

const inFlightTrackFetches = new Set<string>();
let pendingFetchPaths: string[] = [];
let fetchTimer: ReturnType<typeof setTimeout> | null = null;

/** Пути, которые нельзя вытеснять: текущий трек + небольшое окно очереди вокруг него. */
function pinnedCachePaths(): Set<string> {
  const pinned = new Set<string>();
  if (currentTrack.value) pinned.add(currentTrack.value.file_path);
  const order = playbackOrder.value;
  const idx = currentOrderIndex.value;
  if (idx >= 0) {
    // Окно вокруг текущего: пара назад + лукэхед очереди вперёд (с запасом).
    const from = Math.max(0, idx - 2);
    const to = Math.min(order.length, idx + 12);
    for (let i = from; i < to; i++) pinned.add(order[i]);
  }
  return pinned;
}

function maybeEvictCache() {
  if (trackCache.size <= TRACK_CACHE_LIMIT) return;
  const pinned = pinnedCachePaths();
  // Сортируем кандидатов (не закреплённые) по возрастанию метки доступа.
  const candidates = [...trackCache.keys()].filter(p => !pinned.has(p));
  candidates.sort((a, b) => (cacheAccessTick.get(a) ?? 0) - (cacheAccessTick.get(b) ?? 0));
  // Вытесняем пачкой до ~90% лимита, чтобы не дёргать вытеснение часто.
  const target = Math.floor(TRACK_CACHE_LIMIT * 0.9);
  let toRemove = trackCache.size - target;
  for (const path of candidates) {
    if (toRemove <= 0) break;
    trackCache.delete(path);
    cacheAccessTick.delete(path);
    toRemove--;
  }
}

/** Положить метаданные в кэш (не затирая уже подгруженную обложку/accent живого объекта). */
function cacheTrack(track: TrackMetadata) {
  const existing = trackCache.get(track.file_path);
  if (existing) {
    // Обновляем поля, но сохраняем уже загруженную обложку/accent, если новые пусты.
    if (!track.cover_art && existing.cover_art) track.cover_art = existing.cover_art;
    if (!track.accent_color && existing.accent_color) track.accent_color = existing.accent_color;
    Object.assign(existing, track);
  } else {
    trackCache.set(track.file_path, track);
  }
  cacheAccessTick.set(track.file_path, ++cacheTick);
  maybeEvictCache();
}

function cacheTracks(tracks: TrackMetadata[]) {
  for (const t of tracks) cacheTrack(t);
}

function flushTrackFetches() {
  fetchTimer = null;
  const batch = pendingFetchPaths;
  pendingFetchPaths = [];
  const paths = batch.filter(p => !trackCache.has(p));
  if (paths.length === 0) return;
  for (const p of paths) inFlightTrackFetches.add(p);
  invoke<TrackMetadata[]>("library_get_tracks_by_paths", { paths })
    .then(tracks => { cacheTracks(tracks); })
    .catch(() => {})
    .finally(() => { for (const p of paths) inFlightTrackFetches.delete(p); });
}

function scheduleTrackFetch(filePath: string) {
  if (trackCache.has(filePath) || inFlightTrackFetches.has(filePath)) return;
  if (!pendingFetchPaths.includes(filePath)) pendingFetchPaths.push(filePath);
  if (!fetchTimer) fetchTimer = setTimeout(flushTrackFetches, 16);
}

/** Синхронный доступ из кэша; при промахе — фоновая дозагрузка (computed пересчитается). */
function getTrackByPath(filePath: string): TrackMetadata | undefined {
  const hit = trackCache.get(filePath);
  if (hit) {
    cacheAccessTick.set(filePath, ++cacheTick);
    return hit;
  }
  scheduleTrackFetch(filePath);
  return undefined;
}

/** Явная пакетная материализация путей (видимое окно каталога). Возвращает промис. */
async function fetchTracksByPaths(paths: string[]): Promise<TrackMetadata[]> {
  const missing = paths.filter(p => !trackCache.has(p));
  if (missing.length > 0) {
    try {
      const tracks = await invoke<TrackMetadata[]>("library_get_tracks_by_paths", { paths: missing });
      cacheTracks(tracks);
    } catch { /* ignore */ }
  }
  return paths.map(p => trackCache.get(p)).filter((t): t is TrackMetadata => !!t);
}

// Initialization flag
let initialized = false;
let statsSession: PlaybackStatsSession | null = null;
let collectionStatsSession: CollectionStatsSession | null = null;

// ──────────────────────────────────────────────────────────────────────────────
// Composable
// ──────────────────────────────────────────────────────────────────────────────

// usePlayer() вызывается из ~20 компонентов. Все refs — модульные синглтоны,
// но computed/функции раньше пересоздавались на каждый вызов (по копии Map
// на компонент). Создаём единственный экземпляр в отдельном effectScope,
// чтобы он не уничтожался при размонтировании первого вызвавшего компонента.
let playerInstance: ReturnType<typeof createPlayer> | null = null;

export function usePlayer() {
  if (!playerInstance) {
    playerInstance = effectScope(true).run(createPlayer)!;
  }
  return playerInstance;
}

function createPlayer() {
  const { settings, load: loadSettings } = useSettings();
  const {
    folders, isScanning, removingFolder, scanProgress,
    init: initFolders, addFolder: addFolderRaw, removeFolder: removeFolderRaw,
    rescanLibrary: rescanLibraryRaw, folderDisplayName, openTracksReadyModal,
    folderAddError,
  } = useFolders();

  const { loadPlaylists, getPlaylistTracks, playlists: allPlaylists } = usePlaylists();

  // ── Доступ к трекам по пути (через LRU-кэш, см. модульный блок выше) ───────
  // getTrackByPath / cacheTracks / fetchTracksByPaths определены на уровне модуля.

  // ── Shuffle/Repeat helpers ────────────────────────────────────────────────

  /** Пути треков текущего источника воспроизведения (драйвер очереди). */
  function getActiveSourcePaths(): string[] {
    return sourcePaths.value;
  }

  /** Установить источник из объектов треков (плейлист/альбом/артист/микс). */
  function setSourceTracks(tracks: TrackMetadata[]) {
    sourceTrackList.value = tracks;
    sourcePaths.value = tracks.map(t => t.file_path);
    cacheTracks(tracks);
  }

  function sendTrackStatisticsDelta(delta: TrackStatisticsDelta): Promise<void> {
    if (!hasTrackDelta(delta)) return Promise.resolve();
    return invoke("statistics_add_track_delta", { delta })
      .then(() => {
        if ((delta.listenCountDelta ?? 0) !== 0) {
          emitTrackStatisticsUpdated({
            filePath: delta.filePath,
            listenCountDelta: delta.listenCountDelta,
          });
        }
      })
      .catch((e) => {
        console.error("Ошибка записи статистики трека:", e);
      });
  }

  function beginStatsSession(track: TrackMetadata, startPositionSecs = 0, state: PlayerStatus["state"] = "Playing") {
    const duration = track.duration_secs ?? 0;
    statsSession = {
      filePath: track.file_path,
      durationSecs: duration,
      startedAtPositionSecs: startPositionSecs,
      lastPositionSecs: startPositionSecs,
      maxPositionSecs: startPositionSecs,
      listenedSecs: 0,
      reportedListenedSecs: 0,
      listenThresholdSecs: listenThresholdSecs(duration),
      listenRecorded: false,
      fullRecorded: false,
      lastWallClockMs: state === "Playing" ? performance.now() : null,
    };
  }

  function updateStatsProgress(position: number, state: PlayerStatus["state"]) {
    const session = statsSession;
    if (!session) return;

    const safePosition = Number.isFinite(position) ? Math.max(0, position) : session.lastPositionSecs;
    session.lastPositionSecs = safePosition;
    session.maxPositionSecs = Math.max(session.maxPositionSecs, safePosition);

    const now = performance.now();
    if (state === "Playing") {
      if (session.lastWallClockMs !== null) {
        const elapsed = (now - session.lastWallClockMs) / 1000;
        if (elapsed > 0 && elapsed < 5) {
          session.listenedSecs += elapsed;
        }
      }
      session.lastWallClockMs = now;
    } else {
      session.lastWallClockMs = null;
    }

    if (!session.listenRecorded && session.listenedSecs >= session.listenThresholdSecs) {
      session.listenRecorded = true;
      void sendTrackStatisticsDelta({
        ...createTrackDelta(session.filePath),
        listenCountDelta: 1,
        ...dayPartListenDelta(),
      });
    }
  }

  function addUnreportedListenedTime(delta: TrackStatisticsDelta, fullListen: boolean) {
    const session = statsSession;
    if (!session) return;
    const target = fullListen && session.durationSecs > 0
      ? session.durationSecs
      : session.listenedSecs;
    const unreported = roundedSeconds(target - session.reportedListenedSecs);
    if (unreported > 0) {
      delta.totalListenedSecsDelta = (delta.totalListenedSecsDelta ?? 0) + unreported;
      session.reportedListenedSecs += unreported;
    }
  }

  async function flushCurrentStatisticsProgress() {
    const session = statsSession;
    if (!session) return;
    updateStatsProgress(positionSecs.value, playbackState.value);
    const delta = createTrackDelta(session.filePath);
    addUnreportedListenedTime(delta, false);
    await sendTrackStatisticsDelta(delta);
  }

  async function finishCurrentStatsSession(options: { fullListen?: boolean; forwardSkip?: boolean } = {}) {
    const session = statsSession;
    if (!session) return;

    updateStatsProgress(positionSecs.value, playbackState.value);
    const delta = createTrackDelta(session.filePath);

    if (options.forwardSkip) {
      delta.skipCountDelta = 1;
      if (session.lastPositionSecs <= 15) {
        delta.earlySkipCountDelta = 1;
      }
    }

    if (options.fullListen && !session.fullRecorded) {
      session.fullRecorded = true;
      delta.fullListenCountDelta = 1;
    }

    if (options.fullListen && !session.listenRecorded) {
      session.listenRecorded = true;
      delta.listenCountDelta = (delta.listenCountDelta ?? 0) + 1;
      Object.assign(delta, dayPartListenDelta());
    }

    addUnreportedListenedTime(delta, !!options.fullListen);
    await sendTrackStatisticsDelta(delta);

    if (options.fullListen) {
      await markCollectionTrackCompleted(session.filePath);
    }

    statsSession = null;
  }

  function recordTrackStartStatistics(filePath: string, options: PlayTrackOptions) {
    const delta = createTrackDelta(filePath);
    if (options.searchStart) delta.searchStartCountDelta = 1;
    if (options.manualListStart) delta.manualStartCountDelta = 1;
    void sendTrackStatisticsDelta(delta);
  }

  function collectionRefFromSource(sourceId: string | null): { kind: CollectionStatsKind; key: string } | null {
    if (!sourceId) return null;
    if (sourceId.startsWith("album:")) {
      return { kind: "album", key: sourceId.slice("album:".length) };
    }
    if (sourceId.startsWith("playlist:")) {
      return { kind: "playlist", key: sourceId.slice("playlist:".length) };
    }
    return null;
  }

  function syncCollectionStatsSource(sourceId: string | null, paths: string[]) {
    const ref = collectionRefFromSource(sourceId);
    const trackPaths = Array.from(new Set(paths.filter(Boolean)));
    if (!ref || trackPaths.length === 0) {
      collectionStatsSession = null;
      return;
    }

    const signature = `${ref.kind}:${ref.key}:${trackPaths.slice().sort().join("\n")}`;
    if (collectionStatsSession?.signature === signature) return;

    collectionStatsSession = {
      ...ref,
      signature,
      trackPaths: new Set(trackPaths),
      completedPaths: new Set(),
    };
  }

  async function markCollectionTrackCompleted(filePath: string) {
    const session = collectionStatsSession;
    if (!session || !session.trackPaths.has(filePath)) return;

    session.completedPaths.add(filePath);
    if (session.completedPaths.size < session.trackPaths.size) return;

    await invoke("statistics_add_collection_full_listen", {
      kind: session.kind,
      key: session.key,
    }).catch((e) => {
      console.error("Ошибка записи статистики подборки:", e);
    });
    session.completedPaths.clear();
  }

  /** Количество треков после текущего, которые всегда должны быть заранее
   *  доступны в playbackOrder (для очереди из 1 текущего + 7 предстоящих = 8). */
  const QUEUE_LOOKAHEAD = 7;

  /** Возвращает количество треков после текущего в playbackOrder. */
  function upcomingCount(): number {
    const idx = currentOrderIndex.value;
    if (idx < 0) return playbackOrder.value.length;
    return Math.max(0, playbackOrder.value.length - idx - 1);
  }

  /** Случайная перестановка путей (Fisher–Yates), без мутации исходного. */
  function shufflePermutation(source: string[]): string[] {
    const arr = source.slice();
    for (let i = arr.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [arr[i], arr[j]] = [arr[j], arr[i]];
    }
    return arr;
  }

  /** Дозаполняет хвост playbackOrder свежими случайными перестановками
   *  источника, чтобы после текущего было минимум QUEUE_LOOKAHEAD треков.
   *  Каждый новый «проход» генерируется заново, поэтому порядок не повторяется.
   *  На стыке проходов первый трек нового прохода не совпадает с последним
   *  треком предыдущего (если в источнике > 1 трека). */
  function ensureLookahead() {
    if (!shuffled.value) return;
    const source = getActiveSourcePaths();
    if (source.length === 0) return;

    while (upcomingCount() < QUEUE_LOOKAHEAD) {
      const full = playbackOrder.value;
      const lastPath = full.length > 0
        ? full[full.length - 1]
        : currentTrack.value?.file_path ?? null;

      const perm = shufflePermutation(source);
      if (source.length > 1 && lastPath && perm[0] === lastPath) {
        const j = 1 + Math.floor(Math.random() * (perm.length - 1));
        [perm[0], perm[j]] = [perm[j], perm[0]];
      }
      playbackOrder.value = [...full, ...perm];
    }
  }

  /** Подрезает историю playbackOrder, чтобы не росла бесконечно в shuffle-режиме. */
  const HISTORY_KEEP = 20;
  function trimHistory() {
    if (!shuffled.value) return;
    const idx = currentOrderIndex.value;
    if (idx <= HISTORY_KEEP) return;
    const cut = idx - HISTORY_KEEP;
    playbackOrder.value = playbackOrder.value.slice(cut);
    currentOrderIndex.value -= cut;
  }

  /** Находит позицию пути в playbackOrder ближайшую к текущей (forward-first). */
  function locateInOrder(filePath: string): number {
    const order = playbackOrder.value;
    const from = Math.max(0, currentOrderIndex.value);
    for (let i = from; i < order.length; i++) {
      if (order[i] === filePath) return i;
    }
    for (let i = 0; i < from; i++) {
      if (order[i] === filePath) return i;
    }
    return -1;
  }

  /** Полностью пересобирает playbackOrder из активного источника (пути). */
  function syncPlaybackOrder() {
    const source = getActiveSourcePaths();
    if (!shuffled.value) {
      playbackOrder.value = source;
      currentOrderIndex.value = currentTrack.value
        ? source.indexOf(currentTrack.value.file_path)
        : -1;
      syncCollectionStatsSource(activePlaylistSource.value, source);
      return;
    }

    const currentPath = currentTrack.value?.file_path ?? null;
    const currentInSource = !!currentPath && source.includes(currentPath);
    const head = currentInSource && currentPath ? [currentPath] : [];
    playbackOrder.value = head;
    currentOrderIndex.value = currentInSource ? 0 : -1;
    ensureLookahead();
    syncCollectionStatsSource(activePlaylistSource.value, source);
  }

  /** Установить треки плейлиста/альбома/артиста/микса как источник воспроизведения */
  function setPlaylistSource(sourceId: string, tracks: TrackMetadata[]) {
    activePlaylistSource.value = sourceId;
    setSourceTracks(tracks);
    // Очередь «Моего микса» существует только во фронтенде — сохраняем её,
    // чтобы восстановить после перезапуска приложения.
    if (sourceId === "mix") {
      saveMixQueuePaths(tracks.map(t => t.file_path));
    }
    syncPlaybackOrder();
    scheduleSaveState();
  }

  /** Установить каталог (возможно отфильтрованный) как источник: только пути. */
  function setCatalogSource(sourceId: string, paths: string[]) {
    activePlaylistSource.value = sourceId;
    sourceTrackList.value = [];
    sourcePaths.value = paths;
    syncPlaybackOrder();
    scheduleSaveState();
  }

  /** Установить очередь «Моего микса» из путей (вся библиотека учитывается без
   *  материализации всех объектов — они резолвятся лениво через кэш). */
  function setMixSourceFromPaths(paths: string[]) {
    activePlaylistSource.value = "mix";
    sourceTrackList.value = [];
    sourcePaths.value = paths;
    // Очередь микса существует только во фронтенде — сохраняем для восстановления.
    saveMixQueuePaths(paths);
    syncPlaybackOrder();
    scheduleSaveState();
  }

  /** Переместить трек в начало каталога (sort_order на бэкенде + локальный порядок). */
  async function moveCatalogTrackToTop(filePath: string) {
    await invoke("library_move_track_to_top", { filePath });
    const moveFront = (arr: string[]): string[] => {
      const i = arr.indexOf(filePath);
      if (i <= 0) return arr;
      const next = arr.slice();
      next.splice(i, 1);
      next.unshift(filePath);
      return next;
    };
    catalogPaths.value = moveFront(catalogPaths.value);
    if (isCatalogActiveSource()) sourcePaths.value = moveFront(sourcePaths.value);
    syncPlaybackOrder();
  }

  function toggleShuffle() {
    shuffled.value = !shuffled.value;
    syncPlaybackOrder();
    scheduleSaveState();
  }

  function toggleRepeat() {
    repeatMode.value = repeatMode.value === "queue" ? "track" : "queue";
    if (repeatMode.value === "track" && currentTrack.value) {
      void sendTrackStatisticsDelta({
        ...createTrackDelta(currentTrack.value.file_path),
        repeatEnabledCountDelta: 1,
      });
    }
    scheduleSaveState();
  }

  // ── Computed ──────────────────────────────────────────────────────────────

  const trackTitle = computed(() => {
    if (!currentTrack.value) return "Нет трека";
    return currentTrack.value.title || extractFileName(currentTrack.value.file_path);
  });

  const trackArtist = computed(() => currentTrack.value?.artist || "Неизвестный исполнитель");
  const trackAlbum = computed(() => currentTrack.value?.album || "");

  const codecDisplay = computed(() => {
    if (!currentTrack.value) return "";
    const fmt = currentTrack.value.format || "";
    const match = fmt.match(/\(([^)]+)\)/);
    return (match ? match[1] : fmt).toUpperCase();
  });

  const codecDetails = computed(() => {
    if (!currentTrack.value) return "";
    const parts: string[] = [];
    if (currentTrack.value.sample_rate)
      parts.push(`${(currentTrack.value.sample_rate / 1000).toFixed(1)} kHz`);
    if (currentTrack.value.bit_depth)
      parts.push(`${currentTrack.value.bit_depth} bit`);
    if (currentTrack.value.channels)
      parts.push(currentTrack.value.channels === 2 ? "Stereo" : `${currentTrack.value.channels}ch`);
    if (currentTrack.value.bitrate)
      parts.push(`${currentTrack.value.bitrate} kbps`);
    return parts.join(" • ");
  });

  const durationSecs = computed(() => currentTrack.value?.duration_secs ?? 0);
  const durationFormatted = computed(() => (durationSecs.value ? formatTime(durationSecs.value) : "--:--"));
  const positionFormatted = computed(() => formatTime(isSeeking.value ? seekValue.value : positionSecs.value));

  const progressPercent = computed(() => {
    if (!durationSecs.value) return 0;
    const pos = isSeeking.value ? seekValue.value : positionSecs.value;
    return (pos / durationSecs.value) * 100;
  });

  const isPlaying = computed(() => playbackState.value === "Playing");
  const isStopped = computed(() => playbackState.value === "Stopped");
  const volumePercent = computed(() => Math.round(volume.value * 100));

  const currentTrackIndex = computed(() => currentOrderIndex.value);

  const hasNext = computed(() => {
    if (playbackOrder.value.length === 0) return false;
    if (shuffled.value) return true; // всегда можем догенерировать следующий проход
    return currentOrderIndex.value >= 0 && currentOrderIndex.value < playbackOrder.value.length - 1;
  });
  const hasPrev = computed(() => currentOrderIndex.value > 0);

  const currentCoverArt = computed(() => {
    if (!currentTrack.value) return null;
    const inList = getTrackByPath(currentTrack.value!.file_path);
    return inList?.cover_art ?? currentTrack.value.cover_art ?? null;
  });

  const currentAccentColor = computed(() => {
    return currentTrack.value?.accent_color ?? null;
  });

  // ── Cover loader ────────────────────────────────────────────────────────

  const { loadCoversForTracks, clearQueue: clearCoverQueue } = useCoverLoader({
    getTrackByPath,
  });

  // ── Утилиты (публичные) ───────────────────────────────────────────────────

  function getTrackDisplayName(track: TrackMetadata): string {
    if (track.title) {
      return track.artist ? `${track.artist} — ${track.title}` : track.title;
    }
    return extractFileName(track.file_path);
  }

  // ── Загрузка библиотеки ───────────────────────────────────────────────────

  /** True, если активный источник — каталог (или его нет). */
  function isCatalogActiveSource(): boolean {
    const src = activePlaylistSource.value;
    return !src || src.startsWith("catalog");
  }

  async function loadLibrary() {
    try {
      isLoading.value = true;
      errorMessage.value = "";
      await initFolders();
      await invoke("library_cleanup");
      catalogPaths.value = await invoke<string[]>("library_get_catalog_paths", {});
      // По умолчанию источник воспроизведения — весь каталог.
      if (isCatalogActiveSource()) sourcePaths.value = catalogPaths.value;
      shuffled.value = false;
      syncPlaybackOrder();
      await loadPlaylists();
    } catch (e: any) {
      errorMessage.value = `Ошибка загрузки: ${e}`;
    } finally {
      isLoading.value = false;
    }
  }

  async function refreshLibrary() {
    try {
      const paths = await invoke<string[]>("library_get_catalog_paths", {});
      const prevTrack = currentTrack.value;
      catalogPaths.value = paths;
      if (isCatalogActiveSource()) sourcePaths.value = paths;
      syncPlaybackOrder();
      await loadPlaylists();

      // Каталог — это все треки библиотеки, поэтому отсутствие пути означает,
      // что трек удалён из библиотеки.
      if (prevTrack && !paths.includes(prevTrack.file_path)) {
        await invoke("player_stop");
        playbackState.value = "Stopped";
        currentTrack.value = null;
        positionSecs.value = 0;
      }
    } catch (e: any) {
      errorMessage.value = `Ошибка обновления библиотеки: ${e}`;
    }
  }

  // ── Управление папками (делегирование в useFolders) ───────────────────────

  async function addFolder() {
    errorMessage.value = "";
    const featureStatus = await invoke<AudioFeatureProgressStatus>("library_audio_features_status")
      .catch(() => null);
    const workerWasActive = featureStatus?.active === true;
    try {
      const added = await addFolderRaw();
      if (!added) return;
    } catch (e: any) {
      errorMessage.value = `Ошибка добавления папки: ${e}`;
    } finally {
      await refreshLibrary();
    }
    if (!workerWasActive) {
      openTracksReadyModal();
    }
  }

  async function removeFolder(folder: string) {
    errorMessage.value = "";
    try {
      await removeFolderRaw(folder);
    } catch (e: any) {
      errorMessage.value = `Ошибка удаления папки: ${e}`;
    } finally {
      await refreshLibrary();
    }
  }

  async function rescanLibrary() {
    errorMessage.value = "";
    try {
      await rescanLibraryRaw();
    } catch (e: any) {
      errorMessage.value = `Ошибка сканирования: ${e}`;
    } finally {
      await refreshLibrary();
    }
  }

  // ── Управление плеером ────────────────────────────────────────────────────

  async function playTrack(track: TrackMetadata | string, options: PlayTrackOptions = {}) {
    const filePath = typeof track === "string" ? track : track.file_path;
    try {
      isLoading.value = true;
      errorMessage.value = "";
      if (options.finishPrevious !== false) {
        await finishCurrentStatsSession();
      }
      const useCrossfade = settings.value.crossfade_between_tracks && playbackState.value === "Playing";
      const metadata = useCrossfade
        ? await invoke<TrackMetadata>("player_play_crossfade", { filePath, crossfadeSecs: 0.25 })
        : await invoke<TrackMetadata>("player_play", { filePath });

      if (!metadata.cover_art) {
        const cached = getTrackByPath(filePath);
        if (cached?.cover_art) metadata.cover_art = cached.cover_art;
        else {
          invoke<string | null>("get_cover_art", { filePath })
            .then(art => {
              if (art) {
                const cachedTrack = trackCache.get(filePath);
                if (cachedTrack) cachedTrack.cover_art = art;
                if (currentTrack.value?.file_path === filePath)
                  currentTrack.value.cover_art = art;
              }
            })
            .catch(() => {});
        }
      }

      currentTrack.value = metadata;
      // Кэшируем свежие метаданные (для резолва в очереди/списках).
      cacheTrack(metadata);
      // Обновляем указатель на позицию в playbackOrder
      if (options.orderIndex !== undefined && options.orderIndex >= 0 && options.orderIndex < playbackOrder.value.length) {
        currentOrderIndex.value = options.orderIndex;
      } else {
        currentOrderIndex.value = locateInOrder(metadata.file_path);
      }
      // В shuffle-режиме гарантируем, что хвост очереди достаточно заполнен,
      // чтобы всегда корректно показать 8 треков в UI.
      if (shuffled.value) ensureLookahead();
      playbackState.value = "Playing";
      positionSecs.value = 0;
      beginStatsSession(metadata, 0, "Playing");
      recordTrackStartStatistics(metadata.file_path, options);
      showSettings.value = false;
      // Источник/трек сменились — планируем сохранение (включение трека,
      // переход по очереди, ручное переключение — всё проходит здесь).
      scheduleSaveState();
    } catch (e: any) {
      // Убираем недоступный трек из очереди и пробуем следующий
      const failedIdx = playbackOrder.value.indexOf(filePath);
      if (failedIdx !== -1) {
        playbackOrder.value = playbackOrder.value.filter((_, i) => i !== failedIdx);
      }
      if (!options.skipAttempt && playbackOrder.value.length > 0) {
        const nextIdx = Math.min(failedIdx === -1 ? 0 : failedIdx, playbackOrder.value.length - 1);
        if (nextIdx >= 0) {
          await playTrack(playbackOrder.value[nextIdx], { skipAttempt: true, finishPrevious: false });
          return;
        }
      }
      errorMessage.value = `Ошибка воспроизведения: ${e}`;
    } finally {
      isLoading.value = false;
    }
  }

  async function togglePlayPause() {
    try {
      errorMessage.value = "";
      if (playbackState.value === "Playing") {
        updateStatsProgress(positionSecs.value, "Playing");
        if (statsSession) statsSession.lastWallClockMs = null;
        await invoke("player_pause");
        playbackState.value = "Paused";
      } else if (playbackState.value === "Paused") {
        await invoke("player_resume");
        playbackState.value = "Playing";
        if (statsSession) statsSession.lastWallClockMs = performance.now();
      }
    } catch (e: any) {
      errorMessage.value = `Ошибка: ${e}`;
    }
  }

  async function stop() {
    try {
      errorMessage.value = "";
      await finishCurrentStatsSession();
      await invoke("player_stop");
      playbackState.value = "Stopped";
      currentTrack.value = null;
      positionSecs.value = 0;
    } catch (e: any) {
      errorMessage.value = `Ошибка остановки: ${e}`;
    }
  }

  async function playNext(auto = false) {
    if (auto && repeatMode.value === "track" && currentTrack.value) {
      await playTrack(currentTrack.value, { orderIndex: currentOrderIndex.value, finishPrevious: false });
      return;
    }
    if (!auto) {
      await finishCurrentStatsSession({ forwardSkip: true });
    }
    if (shuffled.value) ensureLookahead();
    if (playbackOrder.value.length === 0) return;
    const idx = currentOrderIndex.value;
    if (idx >= 0 && idx < playbackOrder.value.length - 1) {
      const nextIdx = idx + 1;
      await playTrack(playbackOrder.value[nextIdx], { orderIndex: nextIdx, finishPrevious: false });
      if (shuffled.value) {
        ensureLookahead();
        trimHistory();
      }
    } else {
      // В unshuffled-режиме зацикливаемся в начало; в shuffled сюда попасть
      // не должны (ensureLookahead гарантирует хвост), но на всякий случай.
      await playTrack(playbackOrder.value[0], { orderIndex: 0, finishPrevious: false });
      if (shuffled.value) ensureLookahead();
    }
  }

  async function playPrev() {
    if (playbackOrder.value.length === 0) return;
    if (positionSecs.value > 3) {
      await seekTo(0);
      return;
    }
    const idx = currentOrderIndex.value;
    await finishCurrentStatsSession();
    if (idx > 0) {
      const prevIdx = idx - 1;
      await playTrack(playbackOrder.value[prevIdx], { orderIndex: prevIdx, finishPrevious: false });
    } else {
      const lastIdx = playbackOrder.value.length - 1;
      await playTrack(playbackOrder.value[lastIdx], { orderIndex: lastIdx, finishPrevious: false });
    }
  }

  async function changeVolume(event: Event) {
    const target = event.target as HTMLInputElement;
    const newVolume = parseFloat(target.value);
    volume.value = newVolume;
    scheduleSaveState();
    try {
      await invoke("player_set_volume", { volume: newVolume });
    } catch (e: any) {
      errorMessage.value = `Ошибка громкости: ${e}`;
    }
  }

  async function setVolume(newVolume: number) {
    volume.value = newVolume;
    scheduleSaveState();
    try {
      await invoke("player_set_volume", { volume: newVolume });
    } catch (e: any) {
      errorMessage.value = `Ошибка громкости: ${e}`;
    }
  }

  function onSeekStart(event: Event) {
    isSeeking.value = true;
    seekValue.value = parseFloat((event.target as HTMLInputElement).value);
  }

  function onSeekInput(event: Event) {
    seekValue.value = parseFloat((event.target as HTMLInputElement).value);
  }

  async function onSeekEnd(event: Event) {
    const newPos = parseFloat((event.target as HTMLInputElement).value);
    seekValue.value = newPos;
    positionSecs.value = newPos;
    lastFrameTime = performance.now();
    await seekTo(newPos);
    isSeeking.value = false;
  }

  async function seekTo(position: number) {
    const useCrossfade = settings.value.crossfade_on_seek && playbackState.value !== "Paused";
    try {
      if (useCrossfade) {
        await invoke("player_seek_crossfade", { positionSecs: position, crossfadeSecs: 0.25 });
      } else {
        await invoke("player_seek", { positionSecs: position });
      }
      positionSecs.value = position;
      lastFrameTime = performance.now();
      updateStatsProgress(position, playbackState.value);
    } catch (e: any) {
      errorMessage.value = `Ошибка перемотки: ${e}`;
    }
  }

  async function pollStatus() {
    try {
      const status = await invoke<PlayerStatus>("player_get_status");
      playbackState.value = status.state;
      volume.value = status.volume;

      if (!isSeeking.value) {
        positionSecs.value = status.position_secs;
        lastFrameTime = performance.now();
      }
      updateStatsProgress(status.position_secs, status.state);

      if (status.current_file_path) {
        if (currentTrack.value?.file_path !== status.current_file_path) {
          const cached = trackCache.get(status.current_file_path);
          if (cached) {
            currentTrack.value = cached;
            beginStatsSession(cached, status.position_secs, status.state);
          } else {
            const meta = await invoke<TrackMetadata>("get_track_metadata", { filePath: status.current_file_path });
            currentTrack.value = meta;
            cacheTrack(meta);
            beginStatsSession(meta, status.position_secs, status.state);
          }
        }
      } else if (status.state === "Stopped") {
        if (currentTrack.value && !isHandlingTrackEnd && !isLoading.value) {
          isHandlingTrackEnd = true;
          try {
            await finishCurrentStatsSession({ fullListen: true });
            await playNext(true);
          } finally {
            isHandlingTrackEnd = false;
          }
        } else if (!currentTrack.value) {
          positionSecs.value = 0;
        }
      }
    } catch {
      // игнорируем ошибки поллинга
    }
  }

  // ── Сохранение / восстановление состояния ─────────────────────────────────

  function startStatusPolling() {
    if (!statusInterval) {
      statusInterval = setInterval(pollStatus, STATUS_POLL_MS);
    }
  }

  function stopStatusPolling() {
    if (statusInterval) {
      clearInterval(statusInterval);
      statusInterval = null;
    }
  }

  // Отложенное (debounced) сохранение состояния. При спаме событий
  // (быстрое переключение треков, кручение громкости, навигация по страницам)
  // перезаписываем только последний результат, а не каждое промежуточное.
  let saveStateTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleSaveState() {
    if (saveStateTimer) clearTimeout(saveStateTimer);
    saveStateTimer = setTimeout(() => {
      saveStateTimer = null;
      void saveState();
    }, 650);
  }

  async function saveState() {
    // Немедленное сохранение отменяет отложенное — оно уже неактуально.
    if (saveStateTimer) {
      clearTimeout(saveStateTimer);
      saveStateTimer = null;
    }
    await flushCurrentStatisticsProgress();

    // Вычисляем индекс текущего трека в источнике (по путям)
    let trackIndex: number | null = null;
    if (currentTrack.value) {
      const idx = getActiveSourcePaths().indexOf(currentTrack.value.file_path);
      trackIndex = idx >= 0 ? idx : null;
    }

    const state: SavedPlayerState = {
      current_track_path: currentTrack.value?.file_path ?? null,
      position_secs: positionSecs.value,
      volume: volume.value,
      shuffled: shuffled.value,
      repeat_mode: repeatMode.value,
      active_playlist_source: activePlaylistSource.value,
      current_track_index: trackIndex,
      current_route: router.currentRoute.value.path,
    };
    try {
      await invoke("save_player_state", { playerState: state });
    } catch (e) {
      console.error("Ошибка сохранения состояния плеера:", e);
    }
  }

  async function restoreState() {
    let saved: SavedPlayerState;
    try {
      saved = await invoke<SavedPlayerState>("load_player_state");
    } catch (e) {
      console.error("Ошибка загрузки сохранённого состояния:", e);
      return;
    }

    try {
      const v = Number.isFinite(saved.volume) ? Math.min(1, Math.max(0, saved.volume)) : 1;
      volume.value = v;
      await invoke("player_set_volume", { volume: v });
    } catch (e) {
      console.error("Ошибка восстановления громкости:", e);
    }

    try {
      shuffled.value = !!saved.shuffled;
      repeatMode.value = (saved.repeat_mode === "track" ? "track" : "queue");
      activePlaylistSource.value = saved.active_playlist_source ?? null;

      // Восстанавливаем источник воспроизведения через бэкенд (без полного trackList).
      const src = activePlaylistSource.value;
      const fallbackToCatalog = () => {
        activePlaylistSource.value = null;
        sourceTrackList.value = [];
        sourcePaths.value = catalogPaths.value;
      };
      if (src && src.startsWith("playlist:")) {
        const pid = Number(src.slice("playlist:".length));
        if (Number.isFinite(pid)) {
          try {
            const tracks = await getPlaylistTracks(pid);
            if (tracks.length) setSourceTracks(tracks);
            else fallbackToCatalog();
          } catch (err) {
            console.error("Ошибка восстановления треков плейлиста:", err);
            fallbackToCatalog();
          }
        } else {
          fallbackToCatalog();
        }
      } else if (src && src.startsWith("album:")) {
        const name = src.slice("album:".length);
        const pl = allPlaylists.value.find(p => p.kind === "Album" && p.name === name);
        const tracks = pl ? await getPlaylistTracks(pl.id) : [];
        if (tracks.length) setSourceTracks(tracks);
        else fallbackToCatalog();
      } else if (src && src.startsWith("artist:")) {
        const name = src.slice("artist:".length);
        const pl = allPlaylists.value.find(p => p.kind === "Artist" && p.name === name);
        const tracks = pl ? await getPlaylistTracks(pl.id) : [];
        if (tracks.length) setSourceTracks(tracks);
        else fallbackToCatalog();
      } else if (src === "mix") {
        // Просчитанная очередь микса хранится во фронтенде (localStorage).
        const mixPaths = loadMixQueuePaths();
        const tracks = mixPaths.length ? await fetchTracksByPaths(mixPaths) : [];
        if (tracks.length) setSourceTracks(tracks);
        else fallbackToCatalog();
      } else {
        // Каталог / поиск / без источника — играем по полному каталогу.
        sourceTrackList.value = [];
        sourcePaths.value = catalogPaths.value;
      }
      syncPlaybackOrder();
    } catch (e) {
      console.error("Ошибка восстановления режимов:", e);
    }

    try {
      if (saved.current_route && saved.current_route !== router.currentRoute.value.path) {
        router.replace(saved.current_route);
      }
    } catch (e) {
      console.error("Ошибка восстановления маршрута:", e);
    }

    if (!saved.current_track_path) return;
    const restored = await fetchTracksByPaths([saved.current_track_path]);
    const track = restored[0];
    if (!track) return;

    const rawPos = saved.position_secs;
    let pos = Number.isFinite(rawPos) && rawPos >= 0 ? rawPos : 0;
    if (track.duration_secs && pos >= track.duration_secs - 1) pos = 0;

    try {
      const metadata = await invoke<TrackMetadata>("player_play_paused", {
        filePath: track.file_path,
        positionSecs: pos,
      });
      if (!metadata.cover_art && track.cover_art) {
        metadata.cover_art = track.cover_art;
      }
      currentTrack.value = metadata;
      cacheTrack(metadata);
      // В shuffle playbackOrder уже сгенерирован, но currentOrderIndex=-1.
      // Ставим current первым и дозаполняем хвост, чтобы очередь была корректной.
      if (shuffled.value) {
        const inSource = getActiveSourcePaths().includes(metadata.file_path);
        if (inSource) {
          playbackOrder.value = [metadata.file_path];
          currentOrderIndex.value = 0;
          ensureLookahead();
        } else {
          currentOrderIndex.value = locateInOrder(metadata.file_path);
        }
      } else {
        currentOrderIndex.value = locateInOrder(metadata.file_path);
      }
      playbackState.value = "Paused";
      positionSecs.value = pos;
      beginStatsSession(metadata, pos, "Paused");
    } catch (e) {
      console.error("Ошибка восстановления трека, сбрасываем сохранённое состояние:", e);
      // Сбрасываем сломанное состояние, чтобы следующий запуск был чистым
      try {
        await invoke("save_player_state", {
          playerState: {
            current_track_path: null,
            position_secs: 0,
            volume: volume.value,
            shuffled: shuffled.value,
            repeat_mode: repeatMode.value,
            active_playlist_source: null,
            current_track_index: null,
            current_route: router.currentRoute.value.path,
          },
        });
      } catch (e2) {
        console.error("Не удалось сбросить состояние плеера:", e2);
      }
    }
  }

  // ── Инициализация (один раз) ─────────────────────────────────────────────

  async function init() {
    if (initialized) return;
    initialized = true;

    await loadSettings();
    await loadLibrary();
    await restoreState();
    if (playbackState.value === "Playing") {
      startStatusPolling();
      startSmoothProgress();
    }

    // Сохраняем открытую страницу при каждой навигации (debounced).
    // Ставится после restoreState, чтобы восстановительный replace не сработал.
    watch(() => router.currentRoute.value.path, () => {
      scheduleSaveState();
    });

    // Останавливаем поллинг, когда плеер не воспроизводит звук.
    stopStatusWatcher = watch(playbackState, (state: string) => {
      if (state === 'Playing') {
        startStatusPolling();
        startSmoothProgress();
      } else {
        stopStatusPolling();
        stopSmoothProgress();
      }
    });
  }

  function destroy() {
    stopStatusPolling();
    stopSmoothProgress();
    if (stopStatusWatcher) {
      stopStatusWatcher();
      stopStatusWatcher = null;
    }
    initialized = false;
  }

  // Start init immediately on first import
  init();

  const catalogCount = computed(() => catalogPaths.value.length);

  return {
    // Состояние плеера
    currentTrack,
    playbackState,
    volume,
    positionSecs,
    isLoading,
    errorMessage,
    catalogPaths,
    catalogCount,
    sourcePaths,
    playbackOrder,
    isSeeking,
    seekValue,
    shuffled,
    repeatMode,
    activePlaylistSource,
    sourceTrackList,

    // Состояние библиотеки / настроек
    showSettings,
    folders,
    isScanning,
    removingFolder,
    scanProgress,
    folderAddError,

    // Computed
    trackTitle,
    trackArtist,
    trackAlbum,
    codecDisplay,
    codecDetails,
    durationSecs,
    durationFormatted,
    positionFormatted,
    progressPercent,
    isPlaying,
    isStopped,
    volumePercent,
    currentTrackIndex,
    hasNext,
    hasPrev,
    currentCoverArt,
    currentAccentColor,

    // Утилиты
    formatTime,
    getTrackDisplayName,
    getTrackByPath,
    cacheTracks,
    fetchTracksByPaths,
    folderDisplayName,

    // Действия
    loadLibrary,
    refreshLibrary,
    addFolder,
    removeFolder,
    rescanLibrary,
    playTrack,
    togglePlayPause,
    stop,
    playNext,
    playPrev,
    changeVolume,
    setVolume,
    onSeekStart,
    onSeekInput,
    onSeekEnd,
    seekTo,
    toggleShuffle,
    toggleRepeat,
    syncPlaybackOrder,
    setPlaylistSource,
    setCatalogSource,
    setMixSourceFromPaths,
    moveCatalogTrackToTop,
    getActiveSourcePaths,
    loadCoversForTracks,
    clearCoverQueue,
    saveState,
    destroy,
  };
}
