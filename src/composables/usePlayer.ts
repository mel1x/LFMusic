import { ref, shallowRef, reactive, computed, watch, effectScope } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { useSettings } from "./useSettings";
import { useFolders } from "./useFolders";
import { saveMixQueuePaths, loadMixQueuePaths } from "./useMix";
import { usePlaylists } from "./usePlaylists";
import { useCoverLoader } from "./useCoverLoader";
import { emitTrackStatisticsUpdated } from "./useStatistics";
import router from "../router";

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
  /** Frontend-only: cover was set from an external source (not embedded/override).
   *  Lets us revert it when external covers are disabled without touching local covers. */
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

// Singleton state (shared between all consumers)

const currentTrack = ref<TrackMetadata | null>(null);
const playbackState = ref<"Stopped" | "Playing" | "Paused">("Stopped");
const volume = ref(0.5);
const positionSecs = ref(0);
const isLoading = ref(false);
const errorMessage = ref("");
// Lightweight ordered list of catalog paths (DB order) instead of a full reactive
// array of objects — avoids holding 10k heavy reactive objects just for queue/count.
const catalogPaths = shallowRef<string[]>([]);
const isSeeking = ref(false);
const seekValue = ref(0);
let statusInterval: ReturnType<typeof setInterval> | null = null;
let stopStatusWatcher: (() => void) | null = null;
let stopTrackEndedListener: UnlistenFn | null = null;
let isHandlingTrackEnd = false;
// The backend also pushes a `track_ended` event near-instantly on natural EOF (see
// spawn_track_end_watcher in lib.rs), so this poll is no longer the primary path for
// that — just position/volume drift correction (rAF handles visual smoothness) plus
// a safety net if an event is ever missed. Widened from 500ms now that it's not
// gating how quickly the next track starts.
const STATUS_POLL_MS = 1000;

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
// Flat playback queue (file_path list). In shuffle mode the tail is topped up with
// freshly generated reshuffle passes so at least QUEUE_LOOKAHEAD tracks are always
// visible ahead. Stores paths only — objects resolve through the cache.
const playbackOrder = shallowRef<string[]>([]);
// Explicit pointer to the current track's position in playbackOrder: needed because
// in shuffle mode the same track can appear in multiple passes, so findIndex won't work.
const currentOrderIndex = ref<number>(-1);

const showSettings = ref(false);

// Playback source: 'catalog' | 'playlist:ID' | null
const activePlaylistSource = ref<string | null>(null);
// Track objects for the current non-catalog source (playlist/album/artist/mix).
// Bounded in size so full objects are fine; unused for catalog (paths only).
const sourceTrackList = ref<TrackMetadata[]>([]);
// Ordered paths of the active source (queue driver for ANY source): catalog ->
// filtered/full paths, otherwise -> paths from sourceTrackList.
const sourcePaths = shallowRef<string[]>([]);

// Reactive LRU cache of track metadata (materialized on demand).
// A reactive Map: adding an entry retriggers computeds that resolved a path via
// getTrackByPath and got undefined (Vue 3 tracks key-level access, including misses).
// Holds metadata plus lazily loaded covers; LRU eviction bounds memory (mainly base64 covers).
const trackCache = reactive(new Map<string, TrackMetadata>());
const TRACK_CACHE_LIMIT = 3000;
// Non-reactive last-access tick, for LRU without extra reactivity triggers.
const cacheAccessTick = new Map<string, number>();
let cacheTick = 0;

const inFlightTrackFetches = new Set<string>();
// Set (not array) so scheduleTrackFetch's membership check is O(1) — it runs on
// every cache miss, e.g. once per row while scrolling an unmaterialized catalog window.
let pendingFetchPaths = new Set<string>();
let fetchTimer: ReturnType<typeof setTimeout> | null = null;

/** Paths that must not be evicted: current track + a small window of queue around it. */
function pinnedCachePaths(): Set<string> {
  const pinned = new Set<string>();
  if (currentTrack.value) pinned.add(currentTrack.value.file_path);
  const order = playbackOrder.value;
  const idx = currentOrderIndex.value;
  if (idx >= 0) {
    const from = Math.max(0, idx - 2);
    const to = Math.min(order.length, idx + 12);
    for (let i = from; i < to; i++) pinned.add(order[i]);
  }
  return pinned;
}

function maybeEvictCache() {
  if (trackCache.size <= TRACK_CACHE_LIMIT) return;
  const pinned = pinnedCachePaths();
  const candidates = [...trackCache.keys()].filter(p => !pinned.has(p));
  candidates.sort((a, b) => (cacheAccessTick.get(a) ?? 0) - (cacheAccessTick.get(b) ?? 0));
  // Evict down to ~90% of the limit in one batch to avoid evicting too often.
  const target = Math.floor(TRACK_CACHE_LIMIT * 0.9);
  let toRemove = trackCache.size - target;
  for (const path of candidates) {
    if (toRemove <= 0) break;
    trackCache.delete(path);
    cacheAccessTick.delete(path);
    toRemove--;
  }
}

/** Puts metadata into the cache without clobbering an already-loaded cover/accent. */
function cacheTrack(track: TrackMetadata) {
  const existing = trackCache.get(track.file_path);
  if (existing) {
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
  pendingFetchPaths = new Set();
  const paths = [...batch].filter(p => !trackCache.has(p));
  if (paths.length === 0) return;
  for (const p of paths) inFlightTrackFetches.add(p);
  invoke<TrackMetadata[]>("library_get_tracks_by_paths", { paths })
    .then(tracks => { cacheTracks(tracks); })
    .catch(() => {})
    .finally(() => { for (const p of paths) inFlightTrackFetches.delete(p); });
}

function scheduleTrackFetch(filePath: string) {
  if (trackCache.has(filePath) || inFlightTrackFetches.has(filePath)) return;
  pendingFetchPaths.add(filePath);
  if (!fetchTimer) fetchTimer = setTimeout(flushTrackFetches, 16);
}

/** Synchronous cache read; on a miss, schedules a background fetch (computed will re-run). */
function getTrackByPath(filePath: string): TrackMetadata | undefined {
  const hit = trackCache.get(filePath);
  if (hit) {
    cacheAccessTick.set(filePath, ++cacheTick);
    return hit;
  }
  scheduleTrackFetch(filePath);
  return undefined;
}

/** Explicit batch materialization of paths (e.g. a visible catalog window). */
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

let initialized = false;
let statsSession: PlaybackStatsSession | null = null;
let collectionStatsSession: CollectionStatsSession | null = null;

// usePlayer() is called from ~20 components. All refs are module singletons, but
// computed/functions used to be recreated per call (a separate Map copy per component).
// A single instance is created in its own effectScope so it isn't destroyed when the
// first calling component unmounts.
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

  // Shuffle/Repeat helpers

  /** Paths of the active playback source (queue driver). */
  function getActiveSourcePaths(): string[] {
    return sourcePaths.value;
  }

  /** Sets the source from track objects (playlist/album/artist/mix). */
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
    // Bail before deduping: for catalog sources `paths` can be the whole library.
    if (!ref) {
      collectionStatsSession = null;
      return;
    }
    const trackPaths = Array.from(new Set(paths.filter(Boolean)));
    if (trackPaths.length === 0) {
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

  /** Tracks after the current one that must always be pre-available in playbackOrder
   *  (queue shows 1 current + 7 upcoming = 8). */
  const QUEUE_LOOKAHEAD = 7;

  function upcomingCount(): number {
    const idx = currentOrderIndex.value;
    if (idx < 0) return playbackOrder.value.length;
    return Math.max(0, playbackOrder.value.length - idx - 1);
  }

  /** Fisher-Yates shuffle; does not mutate the source. */
  function shufflePermutation(source: string[]): string[] {
    const arr = source.slice();
    for (let i = arr.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [arr[i], arr[j]] = [arr[j], arr[i]];
    }
    return arr;
  }

  /** Tops up the playbackOrder tail with fresh random permutations of the source so
   *  at least QUEUE_LOOKAHEAD tracks follow the current one. Each pass is regenerated,
   *  so order never repeats; the first track of a new pass is never the same as the
   *  last track of the previous one (when source has >1 track). */
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

  /** Trims playbackOrder history so it doesn't grow unbounded in shuffle mode. */
  const HISTORY_KEEP = 20;
  function trimHistory() {
    if (!shuffled.value) return;
    const idx = currentOrderIndex.value;
    if (idx <= HISTORY_KEEP) return;
    const cut = idx - HISTORY_KEEP;
    playbackOrder.value = playbackOrder.value.slice(cut);
    currentOrderIndex.value -= cut;
  }

  /** Finds the closest position of a path in playbackOrder, searching forward first. */
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

  /** Fully rebuilds playbackOrder from the active source's paths. */
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

  /** Sets playlist/album/artist/mix tracks as the playback source. */
  function setPlaylistSource(sourceId: string, tracks: TrackMetadata[]) {
    activePlaylistSource.value = sourceId;
    setSourceTracks(tracks);
    // The "mix" queue only exists on the frontend, so persist it for restore after restart.
    if (sourceId === "mix") {
      saveMixQueuePaths(tracks.map(t => t.file_path));
    }
    syncPlaybackOrder();
    scheduleSaveState();
  }

  /** Sets the (possibly filtered) catalog as source: paths only. */
  function setCatalogSource(sourceId: string, paths: string[]) {
    activePlaylistSource.value = sourceId;
    sourceTrackList.value = [];
    sourcePaths.value = paths;
    syncPlaybackOrder();
    scheduleSaveState();
  }

  /** Sets the mix queue from paths without materializing every object up front
   *  (they resolve lazily through the cache). */
  function setMixSourceFromPaths(paths: string[]) {
    activePlaylistSource.value = "mix";
    sourceTrackList.value = [];
    sourcePaths.value = paths;
    saveMixQueuePaths(paths);
    syncPlaybackOrder();
    scheduleSaveState();
  }

  /** Moves a track to the front of the catalog (backend sort_order + local order). */
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
    if (shuffled.value) return true; // can always generate another pass
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

  const { loadCoversForTracks, clearQueue: clearCoverQueue } = useCoverLoader({
    getTrackByPath,
  });

  function getTrackDisplayName(track: TrackMetadata): string {
    if (track.title) {
      return track.artist ? `${track.artist} - ${track.title}` : track.title;
    }
    return extractFileName(track.file_path);
  }

  /** True if the active source is the catalog (or there is none). */
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
      // Default playback source is the whole catalog.
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

      // Catalog is every track in the library, so a missing path means the
      // track was removed from the library.
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

  /** One-time background rescan of active folders on startup: picks up files
   *  added/removed outside the app; new tracks land at the front of the catalog
   *  via backend sort_order. Silent (no progress modal/isScanning flag); refreshes
   *  the catalog through refreshLibrary() when done. */
  async function rescanOnStartup() {
    try {
      await invoke("library_rescan");
      await refreshLibrary();
    } catch {
      // Startup must not fail because of a background rescan.
    }
  }

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
      cacheTrack(metadata);
      if (options.orderIndex !== undefined && options.orderIndex >= 0 && options.orderIndex < playbackOrder.value.length) {
        currentOrderIndex.value = options.orderIndex;
      } else {
        currentOrderIndex.value = locateInOrder(metadata.file_path);
      }
      // Keep the shuffle tail filled so the UI can always show 8 tracks.
      if (shuffled.value) ensureLookahead();
      playbackState.value = "Playing";
      positionSecs.value = 0;
      beginStatsSession(metadata, 0, "Playing");
      recordTrackStartStatistics(metadata.file_path, options);
      showSettings.value = false;
      scheduleSaveState();
    } catch (e: any) {
      // Drop the unavailable track from the queue and try the next one.
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
      // Unshuffled: loop back to the start. Shuffled shouldn't reach here since
      // ensureLookahead guarantees a tail, but handle it just in case.
      await playTrack(playbackOrder.value[0], { orderIndex: 0, finishPrevious: false });
      if (shuffled.value) ensureLookahead();
    }
  }

  /** `skipRestart`: always go to the previous track (mini player swipe) instead of
   *  rewinding the current one after 3 seconds. */
  async function playPrev(opts?: { skipRestart?: boolean }) {
    if (playbackOrder.value.length === 0) return;
    if (!opts?.skipRestart && positionSecs.value > 3) {
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
        if (currentTrack.value) {
          await handleTrackNaturallyEnded(currentTrack.value.file_path);
        } else {
          positionSecs.value = 0;
        }
      }
    } catch {
      // ignore polling errors
    }
  }

  /** Shared by the `track_ended` event and the status-poll fallback: advances the
   *  queue once per natural end-of-track. `finishedPath` guards against a stale
   *  signal racing a manual track change that already moved playback elsewhere. */
  async function handleTrackNaturallyEnded(finishedPath: string) {
    if (isHandlingTrackEnd || isLoading.value) return;
    if (!currentTrack.value || currentTrack.value.file_path !== finishedPath) return;
    isHandlingTrackEnd = true;
    try {
      await finishCurrentStatsSession({ fullListen: true });
      await playNext(true);
    } finally {
      isHandlingTrackEnd = false;
    }
  }

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

  // Debounced state save: under event spam (rapid track switching, volume dragging,
  // page navigation) only the last call actually persists, not every intermediate one.
  let saveStateTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleSaveState() {
    if (saveStateTimer) clearTimeout(saveStateTimer);
    saveStateTimer = setTimeout(() => {
      saveStateTimer = null;
      void saveState();
    }, 650);
  }

  async function saveState() {
    // An immediate save supersedes any pending debounced one.
    if (saveStateTimer) {
      clearTimeout(saveStateTimer);
      saveStateTimer = null;
    }
    await flushCurrentStatisticsProgress();

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
      const v = Number.isFinite(saved.volume) ? Math.min(1, Math.max(0, saved.volume)) : 0.5;
      volume.value = v;
      await invoke("player_set_volume", { volume: v });
    } catch (e) {
      console.error("Ошибка восстановления громкости:", e);
    }

    try {
      shuffled.value = !!saved.shuffled;
      repeatMode.value = (saved.repeat_mode === "track" ? "track" : "queue");
      activePlaylistSource.value = saved.active_playlist_source ?? null;

      // Restore the playback source through the backend (without a full trackList).
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
        // The computed mix queue is stored on the frontend (localStorage).
        const mixPaths = loadMixQueuePaths();
        const tracks = mixPaths.length ? await fetchTracksByPaths(mixPaths) : [];
        if (tracks.length) setSourceTracks(tracks);
        else fallbackToCatalog();
      } else {
        // Catalog / search / no source: play the full catalog.
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
      // In shuffle mode playbackOrder may already exist but currentOrderIndex is -1:
      // put current first and top up the tail so the queue is valid.
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
      // Reset the broken state so the next startup is clean.
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

    // Silent one-time rescan of active folders (fire-and-forget, doesn't delay UI readiness).
    void rescanOnStartup();

    // Pushed by spawn_track_end_watcher (lib.rs) near-instantly on natural EOF —
    // primary path for advancing the queue; the status poll above is the fallback.
    stopTrackEndedListener = await listen<string>("track_ended", (event) => {
      void handleTrackNaturallyEnded(event.payload);
    });

    // Persist the current page on navigation (debounced). Set up after restoreState
    // so its own router.replace doesn't trigger a save.
    watch(() => router.currentRoute.value.path, () => {
      scheduleSaveState();
    });

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
    if (stopTrackEndedListener) {
      stopTrackEndedListener();
      stopTrackEndedListener = null;
    }
    initialized = false;
  }

  // Start init immediately on first import
  init();

  const catalogCount = computed(() => catalogPaths.value.length);

  return {
    // Player state
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

    // Library / settings state
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

    // Utilities
    formatTime,
    getTrackDisplayName,
    getTrackByPath,
    cacheTracks,
    fetchTracksByPaths,
    folderDisplayName,

    // Actions
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
