import { ref, computed, watch, onMounted, onUnmounted, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ──────────────────────────────────────────────────────────────────────────────
// Типы
// ──────────────────────────────────────────────────────────────────────────────

export interface TrackMetadata {
  title: string | null;
  artist: string | null;
  album: string | null;
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

export interface FolderInfo {
  path: string;
  track_count: number;
  exists: boolean;
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
// Composable
// ──────────────────────────────────────────────────────────────────────────────

export function usePlayer() {
  // Состояние плеера
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
  let originalOrder: TrackMetadata[] = [];

  function toggleShuffle() {
    shuffled.value = !shuffled.value;
    if (shuffled.value) {
      originalOrder = [...trackList.value];
      const current = currentTrack.value;
      const rest = trackList.value.filter(t => t.file_path !== current?.file_path);
      for (let i = rest.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [rest[i], rest[j]] = [rest[j], rest[i]];
      }
      trackList.value = current ? [current, ...rest] : rest;
    } else {
      if (originalOrder.length) trackList.value = originalOrder;
      originalOrder = [];
    }
  }

  function toggleRepeat() {
    repeatMode.value = repeatMode.value === "queue" ? "track" : "queue";
  }

  // Состояние библиотеки / настроек
  const showSettings = ref(false);
  const folders = ref<FolderInfo[]>([]);
  const isScanning = ref(false);
  const scanMessage = ref("");
  const removingFolder = ref<string | null>(null);
  const scanProgress = ref({ current: 0, total: 0, percent: 0 });
  let unlistenScanProgress: UnlistenFn | null = null;

  // ── Computed ──────────────────────────────────────────────────────────────

  const trackTitle = computed(() => {
    if (!currentTrack.value) return "Нет трека";
    return currentTrack.value.title || extractFileName(currentTrack.value.file_path);
  });

  const trackArtist = computed(() => currentTrack.value?.artist || "Неизвестный исполнитель");
  const trackAlbum = computed(() => currentTrack.value?.album || "");

  const trackInfo = computed(() => {
    if (!currentTrack.value) return "";
    const parts: string[] = [];
    parts.push(currentTrack.value.format);
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

  const currentTrackIndex = computed(() => {
    if (!currentTrack.value) return -1;
    return trackList.value.findIndex(t => t.file_path === currentTrack.value!.file_path);
  });

  const hasNext = computed(() => currentTrackIndex.value >= 0 && currentTrackIndex.value < trackList.value.length - 1);
  const hasPrev = computed(() => currentTrackIndex.value > 0);

  const currentCoverArt = computed(() => {
    if (!currentTrack.value) return null;
    const inList = trackList.value.find(t => t.file_path === currentTrack.value!.file_path);
    return inList?.cover_art ?? currentTrack.value.cover_art ?? null;
  });

  const currentAccentColor = computed(() => {
    return currentTrack.value?.accent_color ?? null;
  });

  // ── Virtual scroll ────────────────────────────────────────────────────────

  const ITEM_HEIGHT = 58;
  const VSCROLL_BUFFER = 5;
  const COVER_PRELOAD_BUFFER = 16;

  const trackListEl = ref<HTMLElement | null>(null);
  const vsScrollTop = ref(0);
  const vsContainerHeight = ref(600);

  function onTrackListScroll(e: Event) {
    vsScrollTop.value = (e.target as HTMLElement).scrollTop;
  }

  const vsRange = computed(() => {
    const start = Math.max(0, Math.floor(vsScrollTop.value / ITEM_HEIGHT) - VSCROLL_BUFFER);
    const end = Math.min(
      trackList.value.length,
      Math.ceil((vsScrollTop.value + vsContainerHeight.value) / ITEM_HEIGHT) + VSCROLL_BUFFER
    );
    return { start, end };
  });

  const visibleTracks = computed(() => trackList.value.slice(vsRange.value.start, vsRange.value.end));
  const vsPaddingTop = computed(() => vsRange.value.start * ITEM_HEIGHT);
  const vsPaddingBottom = computed(() => (trackList.value.length - vsRange.value.end) * ITEM_HEIGHT);

  const coverPreloadRange = computed(() => {
    const start = Math.max(0, Math.floor(vsScrollTop.value / ITEM_HEIGHT) - COVER_PRELOAD_BUFFER);
    const end = Math.min(
      trackList.value.length,
      Math.ceil((vsScrollTop.value + vsContainerHeight.value) / ITEM_HEIGHT) + COVER_PRELOAD_BUFFER
    );
    return { start, end };
  });

  const coverPreloadTracks = computed(() =>
    trackList.value.slice(coverPreloadRange.value.start, coverPreloadRange.value.end)
  );

  const loadingCovers = new Set<string>();
  const MAX_CONCURRENT_COVERS = 3;
  let activeCoverRequests = 0;
  let coverQueue: string[] = [];
  let coverDebounceTimer: ReturnType<typeof setTimeout> | null = null;

  function processCoverQueue() {
    while (activeCoverRequests < MAX_CONCURRENT_COVERS && coverQueue.length > 0) {
      const filePath = coverQueue.shift()!;
      const track = trackList.value.find(t => t.file_path === filePath);
      if (!track || track.cover_art) {
        loadingCovers.delete(filePath);
        continue;
      }
      activeCoverRequests++;
      invoke<string | null>("get_cover_art", { filePath })
        .then(art => {
          if (art) {
            const idx = trackList.value.findIndex(t => t.file_path === filePath);
            if (idx !== -1) trackList.value[idx].cover_art = art;
          }
        })
        .catch(() => {})
        .finally(() => {
          loadingCovers.delete(filePath);
          activeCoverRequests--;
          processCoverQueue();
        });
    }
  }

  watch(coverPreloadTracks, (tracks) => {
    if (coverDebounceTimer) clearTimeout(coverDebounceTimer);
    coverDebounceTimer = setTimeout(() => {
      // Visible tracks first, then preload buffer
      const visible = visibleTracks.value.map(t => t.file_path);
      const all = tracks.map(t => t.file_path);
      const preloadOnly = all.filter(p => !visible.includes(p));
      const ordered = [...visible, ...preloadOnly];

      coverQueue = [];
      for (const filePath of ordered) {
        const track = trackList.value.find(t => t.file_path === filePath);
        if (!track || track.cover_art || loadingCovers.has(filePath)) continue;
        loadingCovers.add(filePath);
        coverQueue.push(filePath);
      }
      processCoverQueue();
    }, 50);
  }, { immediate: true });

  // ── Утилиты (публичные) ───────────────────────────────────────────────────

  function getTrackDisplayName(track: TrackMetadata): string {
    if (track.title) {
      return track.artist ? `${track.artist} — ${track.title}` : track.title;
    }
    return extractFileName(track.file_path);
  }

  function folderDisplayName(path: string): string {
    return path.replace(/\\/g, "/");
  }

  // ── Загрузка библиотеки ───────────────────────────────────────────────────

  async function loadLibrary() {
    try {
      isLoading.value = true;
      errorMessage.value = "";
      const [tracks, folderList] = await Promise.all([
        invoke<TrackMetadata[]>("library_get_tracks"),
        invoke<FolderInfo[]>("library_get_folders"),
      ]);
      trackList.value = tracks;
      folders.value = folderList;
      if (shuffled.value) { shuffled.value = false; originalOrder = []; }
    } catch (e: any) {
      errorMessage.value = `Ошибка загрузки: ${e}`;
    } finally {
      isLoading.value = false;
    }
  }

  // ── Управление папками ────────────────────────────────────────────────────

  async function addFolder() {
    try {
      const dirPath = await open({ directory: true, multiple: false });
      if (!dirPath) return;

      isScanning.value = true;
      scanMessage.value = "Сканирование...";
      scanProgress.value = { current: 0, total: 0, percent: 0 };
      errorMessage.value = "";

      const tracks = await invoke<TrackMetadata[]>("library_add_folder", {
        folderPath: dirPath as string,
      });
      trackList.value = tracks;
      folders.value = await invoke<FolderInfo[]>("library_get_folders");
      scanMessage.value = "";
    } catch (e: any) {
      errorMessage.value = `Ошибка добавления: ${e}`;
      scanMessage.value = "";
    } finally {
      isScanning.value = false;
    }
  }

  async function removeFolder(folder: string) {
    try {
      removingFolder.value = folder;
      await invoke("library_remove_folder", { folderPath: folder });
      folders.value = folders.value.filter(f => f.path !== folder);
      trackList.value = await invoke<TrackMetadata[]>("library_get_tracks");
    } catch (e: any) {
      errorMessage.value = `Ошибка удаления: ${e}`;
    } finally {
      removingFolder.value = null;
    }
  }

  async function rescanLibrary() {
    try {
      isScanning.value = true;
      scanMessage.value = "Пересканирование...";
      scanProgress.value = { current: 0, total: 0, percent: 0 };
      errorMessage.value = "";

      const tracks = await invoke<TrackMetadata[]>("library_rescan");
      trackList.value = tracks;
      folders.value = await invoke<FolderInfo[]>("library_get_folders");
      scanMessage.value = "";
    } catch (e: any) {
      errorMessage.value = `Ошибка пересканирования: ${e}`;
      scanMessage.value = "";
    } finally {
      isScanning.value = false;
    }
  }

  // ── Управление плеером ────────────────────────────────────────────────────

  async function playTrack(track: TrackMetadata) {
    try {
      isLoading.value = true;
      errorMessage.value = "";
      const metadata = await invoke<TrackMetadata>("player_play", { filePath: track.file_path });

      if (!metadata.cover_art) {
        const inList = trackList.value.find(t => t.file_path === track.file_path);
        if (inList?.cover_art) metadata.cover_art = inList.cover_art;
        else {
          invoke<string | null>("get_cover_art", { filePath: track.file_path })
            .then(art => {
              if (art) {
                const idx = trackList.value.findIndex(t => t.file_path === track.file_path);
                if (idx !== -1) trackList.value[idx].cover_art = art;
                if (currentTrack.value?.file_path === track.file_path)
                  currentTrack.value.cover_art = art;
              }
            })
            .catch(() => {});
        }
      }

      currentTrack.value = metadata;
      playbackState.value = "Playing";
      positionSecs.value = 0;
      showSettings.value = false;
    } catch (e: any) {
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

  async function playNext() {
    if (repeatMode.value === "track" && currentTrack.value) {
      await playTrack(currentTrack.value);
      return;
    }
    if (hasNext.value) {
      await playTrack(trackList.value[currentTrackIndex.value + 1]);
    } else if (trackList.value.length > 0) {
      await playTrack(trackList.value[0]);
    }
  }

  async function playPrev() {
    if (positionSecs.value > 3) await seekTo(0);
    else if (hasPrev.value) await playTrack(trackList.value[currentTrackIndex.value - 1]);
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

  function onSeekStart(event: Event) {
    isSeeking.value = true;
    seekValue.value = parseFloat((event.target as HTMLInputElement).value);
  }

  function onSeekInput(event: Event) {
    seekValue.value = parseFloat((event.target as HTMLInputElement).value);
  }

  async function onSeekEnd(event: Event) {
    const newPos = parseFloat((event.target as HTMLInputElement).value);
    isSeeking.value = false;
    await seekTo(newPos);
  }

  async function seekTo(position: number) {
    if (playbackState.value !== "Paused") {
      try {
        await invoke("player_seek_crossfade", { positionSecs: position, crossfadeSecs: 0.25 });
        positionSecs.value = position;
      } catch (e: any) {
        errorMessage.value = `Ошибка перемотки: ${e}`;
      }
    } else {
      try {
        await invoke("player_seek", { positionSecs: position, crossfadeSecs: 0.25 });
        positionSecs.value = position;
      } catch (e: any) {
        errorMessage.value = `Ошибка перемотки: ${e}`;
      }
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
          const inList = trackList.value.find(t => t.file_path === status.current_file_path);
          if (inList) {
            currentTrack.value = inList;
          } else {
            const meta = await invoke<TrackMetadata>("get_track_metadata", { filePath: status.current_file_path });
            currentTrack.value = meta;
          }
        }
      } else if (status.state === "Stopped") {
        if (currentTrack.value && !isHandlingTrackEnd) {
          isHandlingTrackEnd = true;
          try {
            if (repeatMode.value === "track") {
              await playTrack(currentTrack.value);
            } else if (hasNext.value) {
              await playNext();
            } else if (trackList.value.length > 0) {
              await playTrack(trackList.value[0]);
            }
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

  // ── Жизненный цикл ───────────────────────────────────────────────────────

  onMounted(async () => {
    await loadLibrary();
    statusInterval = setInterval(pollStatus, 250);
    await nextTick();
    if (trackListEl.value) vsContainerHeight.value = trackListEl.value.clientHeight;
    unlistenScanProgress = await listen<{ current: number; total: number; percent: number }>(
      "scan_progress",
      (event) => { scanProgress.value = event.payload; }
    );
  });

  onUnmounted(() => {
    if (statusInterval) clearInterval(statusInterval);
    if (unlistenScanProgress) unlistenScanProgress();
  });

  return {
    // Состояние плеера
    currentTrack,
    playbackState,
    volume,
    positionSecs,
    isLoading,
    errorMessage,
    trackList,
    isSeeking,
    seekValue,
    shuffled,
    repeatMode,

    // Состояние библиотеки / настроек
    showSettings,
    folders,
    isScanning,
    scanMessage,
    removingFolder,
    scanProgress,

    // Computed
    trackTitle,
    trackArtist,
    trackAlbum,
    trackInfo,
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

    // Virtual scroll
    trackListEl,
    vsScrollTop,
    vsContainerHeight,
    onTrackListScroll,
    visibleTracks,
    vsPaddingTop,
    vsPaddingBottom,

    // Утилиты
    formatTime,
    getTrackDisplayName,
    folderDisplayName,

    // Действия
    loadLibrary,
    addFolder,
    removeFolder,
    rescanLibrary,
    playTrack,
    togglePlayPause,
    stop,
    playNext,
    playPrev,
    changeVolume,
    onSeekStart,
    onSeekInput,
    onSeekEnd,
    toggleShuffle,
    toggleRepeat,
  };
}
