import { ref, computed, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";

import { useSettings } from "./useSettings";
import { useFolders } from "./useFolders";
import { usePlaylists } from "./usePlaylists";
import { useCoverLoader } from "./useCoverLoader";
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
  accent_color: string | null;
}

export interface PlayerStatus {
  state: "Stopped" | "Playing" | "Paused";
  volume: number;
  position_secs: number;
  current_file_path: string | null;
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

// ──────────────────────────────────────────────────────────────────────────────
// Singleton state (shared between all consumers)
// ──────────────────────────────────────────────────────────────────────────────

const currentTrack = ref<TrackMetadata | null>(null);
const playbackState = ref<"Stopped" | "Playing" | "Paused">("Stopped");
const volume = ref(1.0);
const positionSecs = ref(0);
const isLoading = ref(false);
const errorMessage = ref("");
const trackList = ref<TrackMetadata[]>([]);
const isSeeking = ref(false);
const seekValue = ref(0);
let statusInterval: ReturnType<typeof setInterval> | null = null;
let isHandlingTrackEnd = false;

// Shuffle / Repeat
const shuffled = ref(false);
const repeatMode = ref<"queue" | "track">("queue");
// playbackOrder — плоская очередь воспроизведения.
// В режиме shuffle хвост очереди дозаполняется заранее сгенерированными
// решафл-проходами, чтобы всегда было видно не менее QUEUE_LOOKAHEAD
// треков после текущего.
const playbackOrder = ref<TrackMetadata[]>([]);
// Явный указатель на позицию текущего трека в playbackOrder.
// Нужен, потому что в shuffle-режиме тот же трек может встречаться
// в нескольких проходах — findIndex не годится.
const currentOrderIndex = ref<number>(-1);

// Состояние библиотеки / настроек
const showSettings = ref(false);

// Источник воспроизведения: 'catalog' | 'playlist:ID' | null
const activePlaylistSource = ref<string | null>(null);
// Треки текущего активного источника (для плейлистов — отдельный список)
const sourceTrackList = ref<TrackMetadata[]>([]);

// Initialization flag
let initialized = false;

// ──────────────────────────────────────────────────────────────────────────────
// Composable
// ──────────────────────────────────────────────────────────────────────────────

export function usePlayer() {
  const { settings, load: loadSettings } = useSettings();
  const {
    folders, isScanning, removingFolder, scanProgress,
    init: initFolders, addFolder: addFolderRaw, removeFolder: removeFolderRaw,
    rescanLibrary: rescanLibraryRaw, folderDisplayName,
  } = useFolders();

  const { loadPlaylists, getPlaylistTracks } = usePlaylists();

  // ── Track map для O(1) поиска по file_path ──────────────────────────────

  const trackMap = computed(() => new Map(trackList.value.map(t => [t.file_path, t])));

  function getTrackByPath(filePath: string): TrackMetadata | undefined {
    return trackMap.value.get(filePath);
  }

  function getTrackIndex(filePath: string): number {
    return trackList.value.findIndex(t => t.file_path === filePath);
  }

  // ── Shuffle/Repeat helpers ────────────────────────────────────────────────

  /** Список треков текущего источника воспроизведения */
  function getActiveSourceTracks(): TrackMetadata[] {
    if (activePlaylistSource.value && activePlaylistSource.value !== 'catalog') {
      return sourceTrackList.value;
    }
    return trackList.value;
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

  /** Случайная перестановка массива (Fisher–Yates), без мутации исходного. */
  function shufflePermutation(source: TrackMetadata[]): TrackMetadata[] {
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
    const source = getActiveSourceTracks();
    if (source.length === 0) return;

    while (upcomingCount() < QUEUE_LOOKAHEAD) {
      const full = playbackOrder.value;
      const lastPath = full.length > 0
        ? full[full.length - 1].file_path
        : currentTrack.value?.file_path ?? null;

      const perm = shufflePermutation(source);
      if (source.length > 1 && lastPath && perm[0].file_path === lastPath) {
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

  /** Находит позицию трека в playbackOrder ближайшую к текущей (forward-first). */
  function locateInOrder(filePath: string): number {
    const order = playbackOrder.value;
    const from = Math.max(0, currentOrderIndex.value);
    for (let i = from; i < order.length; i++) {
      if (order[i].file_path === filePath) return i;
    }
    for (let i = 0; i < from; i++) {
      if (order[i].file_path === filePath) return i;
    }
    return -1;
  }

  /** Полностью пересобирает playbackOrder из активного источника. */
  function syncPlaybackOrder() {
    if (!shuffled.value) {
      playbackOrder.value = getActiveSourceTracks();
      currentOrderIndex.value = currentTrack.value
        ? playbackOrder.value.findIndex(t => t.file_path === currentTrack.value!.file_path)
        : -1;
      return;
    }

    const current = currentTrack.value;
    const source = getActiveSourceTracks();
    const currentInSource = !!current && source.some(t => t.file_path === current.file_path);
    const head = currentInSource && current ? [current] : [];
    playbackOrder.value = head;
    currentOrderIndex.value = currentInSource ? 0 : -1;
    ensureLookahead();
  }

  /** Установить треки плейлиста как источник воспроизведения */
  function setPlaylistSource(sourceId: string, tracks: TrackMetadata[]) {
    activePlaylistSource.value = sourceId;
    sourceTrackList.value = tracks;
    syncPlaybackOrder();
  }

  function toggleShuffle() {
    shuffled.value = !shuffled.value;
    syncPlaybackOrder();
  }

  function toggleRepeat() {
    repeatMode.value = repeatMode.value === "queue" ? "track" : "queue";
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
    trackList,
    getTrackByPath,
    getTrackIndex,
  });

  // ── Утилиты (публичные) ───────────────────────────────────────────────────

  function getTrackDisplayName(track: TrackMetadata): string {
    if (track.title) {
      return track.artist ? `${track.artist} — ${track.title}` : track.title;
    }
    return extractFileName(track.file_path);
  }

  // ── Загрузка библиотеки ───────────────────────────────────────────────────

  async function loadLibrary() {
    try {
      isLoading.value = true;
      errorMessage.value = "";
      await initFolders();
      await invoke("library_cleanup");
      trackList.value = await invoke<TrackMetadata[]>("library_get_tracks");
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
      const tracks = await invoke<TrackMetadata[]>("library_get_tracks");
      const prevTrack = currentTrack.value;
      trackList.value = tracks;
      syncPlaybackOrder();
      await loadPlaylists();

      if (prevTrack && !tracks.some(t => t.file_path === prevTrack.file_path)) {
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
    await addFolderRaw();
    await refreshLibrary();
  }

  async function removeFolder(folder: string) {
    errorMessage.value = "";
    await removeFolderRaw(folder);
    await refreshLibrary();
  }

  async function rescanLibrary() {
    errorMessage.value = "";
    await rescanLibraryRaw();
    await refreshLibrary();
  }

  // ── Управление плеером ────────────────────────────────────────────────────

  async function playTrack(track: TrackMetadata, _skipAttempt = false, orderIndex?: number) {
    try {
      isLoading.value = true;
      errorMessage.value = "";
      const useCrossfade = settings.value.crossfade_between_tracks && playbackState.value === "Playing";
      const metadata = useCrossfade
        ? await invoke<TrackMetadata>("player_play_crossfade", { filePath: track.file_path, crossfadeSecs: 0.25 })
        : await invoke<TrackMetadata>("player_play", { filePath: track.file_path });

      if (!metadata.cover_art) {
        const inList = getTrackByPath(track.file_path);
        if (inList?.cover_art) metadata.cover_art = inList.cover_art;
        else {
          invoke<string | null>("get_cover_art", { filePath: track.file_path })
            .then(art => {
              if (art) {
                const idx = getTrackIndex(track.file_path);
                if (idx !== -1) trackList.value[idx].cover_art = art;
                if (currentTrack.value?.file_path === track.file_path)
                  currentTrack.value.cover_art = art;
              }
            })
            .catch(() => {});
        }
      }

      currentTrack.value = metadata;
      // Обновляем указатель на позицию в playbackOrder
      if (orderIndex !== undefined && orderIndex >= 0 && orderIndex < playbackOrder.value.length) {
        currentOrderIndex.value = orderIndex;
      } else {
        currentOrderIndex.value = locateInOrder(metadata.file_path);
      }
      // В shuffle-режиме гарантируем, что хвост очереди достаточно заполнен,
      // чтобы всегда корректно показать 8 треков в UI.
      if (shuffled.value) ensureLookahead();
      // Сохраняем accent_color в trackList, чтобы pollStatus не затирал его
      if (metadata.accent_color) {
        const idx = getTrackIndex(metadata.file_path);
        if (idx !== -1) trackList.value[idx].accent_color = metadata.accent_color;
      }
      playbackState.value = "Playing";
      positionSecs.value = 0;
      showSettings.value = false;
    } catch (e: any) {
      // Удаляем недоступный трек из списка и пробуем следующий
      const failedIdx = trackList.value.findIndex(t => t.file_path === track.file_path);
      if (failedIdx !== -1) {
        trackList.value.splice(failedIdx, 1);
        syncPlaybackOrder();
      }
      if (!_skipAttempt && playbackOrder.value.length > 0) {
        const nextIdx = Math.min(failedIdx, playbackOrder.value.length - 1);
        if (nextIdx >= 0) {
          await playTrack(playbackOrder.value[nextIdx], true);
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
        await invoke("player_pause");
        playbackState.value = "Paused";
      } else if (playbackState.value === "Paused") {
        await invoke("player_resume");
        playbackState.value = "Playing";
      }
    } catch (e: any) {
      errorMessage.value = `Ошибка: ${e}`;
    }
  }

  async function stop() {
    try {
      errorMessage.value = "";
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
      await playTrack(currentTrack.value, false, currentOrderIndex.value);
      return;
    }
    if (shuffled.value) ensureLookahead();
    if (playbackOrder.value.length === 0) return;
    const idx = currentOrderIndex.value;
    if (idx >= 0 && idx < playbackOrder.value.length - 1) {
      const nextIdx = idx + 1;
      await playTrack(playbackOrder.value[nextIdx], false, nextIdx);
      if (shuffled.value) {
        ensureLookahead();
        trimHistory();
      }
    } else {
      // В unshuffled-режиме зацикливаемся в начало; в shuffled сюда попасть
      // не должны (ensureLookahead гарантирует хвост), но на всякий случай.
      await playTrack(playbackOrder.value[0], false, 0);
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
    if (idx > 0) {
      const prevIdx = idx - 1;
      await playTrack(playbackOrder.value[prevIdx], false, prevIdx);
    } else {
      const lastIdx = playbackOrder.value.length - 1;
      await playTrack(playbackOrder.value[lastIdx], false, lastIdx);
    }
  }

  async function changeVolume(event: Event) {
    const target = event.target as HTMLInputElement;
    const newVolume = parseFloat(target.value);
    volume.value = newVolume;
    try {
      await invoke("player_set_volume", { volume: newVolume });
    } catch (e: any) {
      errorMessage.value = `Ошибка громкости: ${e}`;
    }
  }

  async function setVolume(newVolume: number) {
    volume.value = newVolume;
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
    } catch (e: any) {
      errorMessage.value = `Ошибка перемотки: ${e}`;
    }
  }

  async function pollStatus() {
    try {
      const status = await invoke<PlayerStatus>("player_get_status");
      playbackState.value = status.state;
      volume.value = status.volume;

      if (!isSeeking.value) positionSecs.value = status.position_secs;

      if (status.current_file_path) {
        if (currentTrack.value?.file_path !== status.current_file_path) {
          const inList = getTrackByPath(status.current_file_path!);
          if (inList) {
            currentTrack.value = inList;
          } else {
            const meta = await invoke<TrackMetadata>("get_track_metadata", { filePath: status.current_file_path });
            currentTrack.value = meta;
          }
        }
      } else if (status.state === "Stopped") {
        if (currentTrack.value && !isHandlingTrackEnd && !isLoading.value) {
          isHandlingTrackEnd = true;
          try {
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

  async function saveState() {
    // Вычисляем индекс текущего трека в источнике
    let trackIndex: number | null = null;
    if (currentTrack.value) {
      const source = getActiveSourceTracks();
      const idx = source.findIndex(t => t.file_path === currentTrack.value!.file_path);
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

      // Для плейлиста — дозагружаем его треки, чтобы playbackOrder не был пустым
      const src = activePlaylistSource.value;
      if (src && src.startsWith("playlist:")) {
        const pid = Number(src.slice("playlist:".length));
        if (Number.isFinite(pid)) {
          try {
            const pathOnly = await getPlaylistTracks(pid);
            const map = new Map(trackList.value.map(t => [t.file_path, t]));
            sourceTrackList.value = pathOnly
              .map(t => map.get(t.file_path) ?? t)
              .filter(Boolean) as TrackMetadata[];
          } catch (err) {
            console.error("Ошибка восстановления треков плейлиста:", err);
            activePlaylistSource.value = null;
            sourceTrackList.value = [];
          }
        }
      } else if (src && src.startsWith("album:")) {
        const name = src.slice("album:".length);
        sourceTrackList.value = trackList.value.filter(t => (t.album || '').trim() === name);
        if (sourceTrackList.value.length === 0) {
          activePlaylistSource.value = null;
        }
      } else if (src && src.startsWith("artist:")) {
        const name = src.slice("artist:".length);
        sourceTrackList.value = trackList.value.filter(t => (t.artist || '').trim() === name);
        if (sourceTrackList.value.length === 0) {
          activePlaylistSource.value = null;
        }
      } else {
        sourceTrackList.value = [];
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
    const track = getTrackByPath(saved.current_track_path);
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
      if (metadata.accent_color) {
        const idx = getTrackIndex(metadata.file_path);
        if (idx !== -1) trackList.value[idx].accent_color = metadata.accent_color;
      }
      currentTrack.value = metadata;
      // В shuffle playbackOrder уже сгенерирован, но currentOrderIndex=-1.
      // Ставим current первым и дозаполняем хвост, чтобы очередь была корректной.
      if (shuffled.value) {
        const source = getActiveSourceTracks();
        const inSource = source.some(t => t.file_path === metadata.file_path);
        if (inSource) {
          playbackOrder.value = [metadata];
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
    statusInterval = setInterval(pollStatus, 500);

    // Останавливаем поллинг когда плеер остановлен и нет трека
    watch([playbackState, currentTrack] as const, ([state, track]: [string, TrackMetadata | null]) => {
      if (state === 'Stopped' && !track) {
        if (statusInterval) {
          clearInterval(statusInterval);
          statusInterval = null;
        }
      } else if (!statusInterval) {
        statusInterval = setInterval(pollStatus, 500);
      }
    });
  }

  function destroy() {
    if (statusInterval) {
      clearInterval(statusInterval);
      statusInterval = null;
    }
    initialized = false;
  }

  // Start init immediately on first import
  init();

  return {
    // Состояние плеера
    currentTrack,
    playbackState,
    volume,
    positionSecs,
    isLoading,
    errorMessage,
    trackList,
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
    trackMap,
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
    getActiveSourceTracks,
    loadCoversForTracks,
    clearCoverQueue,
    saveState,
    destroy,
  };
}
