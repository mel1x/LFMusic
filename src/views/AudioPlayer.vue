<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, nextTick, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ──────────────────────────────────────────────────────────────────────────────
// Типы
// ──────────────────────────────────────────────────────────────────────────────

interface TrackMetadata {
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

interface PlayerStatus {
  state: "Stopped" | "Playing" | "Paused";
  volume: number;
  position_secs: number;
  current_file_path: string | null;
}

interface FolderInfo {
  path: string;
  track_count: number;
  exists: boolean;
}

// ──────────────────────────────────────────────────────────────────────────────
// Состояние плеера
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

// ──────────────────────────────────────────────────────────────────────────────
// Состояние библиотеки / настроек
// ──────────────────────────────────────────────────────────────────────────────

const showSettings = ref(false);
const folders = ref<FolderInfo[]>([]);
const isScanning = ref(false);
const scanMessage = ref("");
const removingFolder = ref<string | null>(null);
const scanProgress = ref({ current: 0, total: 0, percent: 0 });
let unlistenScanProgress: UnlistenFn | null = null;

// ──────────────────────────────────────────────────────────────────────────────
// Computed
// ──────────────────────────────────────────────────────────────────────────────

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

// ──────────────────────────────────────────────────────────────────────────────
// Virtual scroll
// ──────────────────────────────────────────────────────────────────────────────

const ITEM_HEIGHT = 58; // px (42px thumb + 8+8 padding)
const VSCROLL_BUFFER = 5;

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

const loadingCovers = new Set<string>();

watch(visibleTracks, (tracks) => {
  for (const track of tracks) {
    if (track.cover_art || loadingCovers.has(track.file_path)) continue;
    loadingCovers.add(track.file_path);
    invoke<string | null>("get_cover_art", { filePath: track.file_path })
      .then(art => {
        if (art) {
          const idx = trackList.value.findIndex(t => t.file_path === track.file_path);
          if (idx !== -1) trackList.value[idx].cover_art = art;
        }
      })
      .catch(() => {})
      .finally(() => loadingCovers.delete(track.file_path));
  }
}, { immediate: true });

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

function getTrackDisplayName(track: TrackMetadata): string {
  if (track.title) {
    return track.artist ? `${track.artist} — ${track.title}` : track.title;
  }
  return extractFileName(track.file_path);
}

function folderDisplayName(path: string): string {
  return path.replace(/\\/g, "/");
}

// ──────────────────────────────────────────────────────────────────────────────
// Загрузка библиотеки
// ──────────────────────────────────────────────────────────────────────────────

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
  } catch (e: any) {
    errorMessage.value = `Ошибка загрузки: ${e}`;
  } finally {
    isLoading.value = false;
  }
}

// ──────────────────────────────────────────────────────────────────────────────
// Управление папками
// ──────────────────────────────────────────────────────────────────────────────

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

// ──────────────────────────────────────────────────────────────────────────────
// Управление плеером
// ──────────────────────────────────────────────────────────────────────────────

async function playTrack(track: TrackMetadata) {
  try {
    isLoading.value = true;
    errorMessage.value = "";
    const metadata = await invoke<TrackMetadata>("player_play", { filePath: track.file_path });

    // Сохраняем cover_art из кэша если плеер не вернул
    if (!metadata.cover_art) {
      const inList = trackList.value.find(t => t.file_path === track.file_path);
      if (inList?.cover_art) metadata.cover_art = inList.cover_art;
      else {
        // Ленивая загрузка обложки
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
    showSettings.value = false; // закрываем настройки при воспроизведении
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
  if (hasNext.value) await playTrack(trackList.value[currentTrackIndex.value + 1]);
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
      // Трек сменился — подгружаем метаданные
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
      if (currentTrack.value && hasNext.value) await playNext();
      else {
        currentTrack.value = null;
        positionSecs.value = 0;
      }
    }
  } catch {
    // игнорируем ошибки поллинга
  }
}

// ──────────────────────────────────────────────────────────────────────────────
// Жизненный цикл
// ──────────────────────────────────────────────────────────────────────────────

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
</script>

<template>
  <div class="audio-player">
    <!-- ── Шапка ──────────────────────────────────────────────────────────── -->
    <div class="header">
      <span class="header-title">🎵 Музыка</span>
      <span class="header-count" v-if="trackList.length > 0">
        {{ trackList.length }} треков
      </span>
      <button
        class="btn-icon"
        :class="{ active: showSettings }"
        @click="showSettings = !showSettings"
        title="Настройки библиотеки"
      >⚙</button>
    </div>

    <!-- ── Панель настроек (поверх списка треков) ────────────────────────── -->
    <div class="settings-panel" v-if="showSettings">
      <div class="settings-header">
        <span class="settings-title">Папки с музыкой</span>
        <div class="settings-actions">
          <button
            class="btn-action"
            @click="rescanLibrary"
            :disabled="isScanning"
            title="Пересканировать все папки"
          >
            <span>↻</span>
            Пересканировать
          </button>
          <button
            class="btn-action btn-action-primary"
            @click="addFolder"
            :disabled="isScanning"
          >
            + Добавить папку
          </button>
        </div>
      </div>

      <!-- Прогресс сканирования -->
      <div class="scan-progress-block" v-if="isScanning">
        <div class="scan-progress-header">
          <span class="scan-progress-label">
            {{ scanProgress.total === 0 ? 'Подготовка...' : `${scanProgress.current} / ${scanProgress.total} файлов` }}
          </span>
          <span class="scan-progress-percent">{{ Math.round(scanProgress.percent) }}%</span>
        </div>
        <div class="scan-progress-track">
          <div class="scan-progress-fill" :style="{ width: scanProgress.percent + '%' }" />
        </div>
      </div>

      <!-- Список папок -->
      <div class="folder-list" v-if="folders.length > 0">
        <div
          v-for="folder in folders"
          :key="folder.path"
          class="folder-item"
          :class="{ 'folder-missing': !folder.exists }"
        >
          <div class="folder-info">
            <div class="folder-path" :title="folder.path">{{ folderDisplayName(folder.path) }}</div>
            <div class="folder-meta">
              <span v-if="!folder.exists" class="folder-missing-badge">Папка недоступна</span>
              <span v-else class="folder-track-count">{{ folder.track_count }} треков</span>
            </div>
          </div>
          <button
            class="btn-remove"
            @click="removeFolder(folder.path)"
            :disabled="removingFolder === folder.path"
            title="Удалить папку из библиотеки"
          >
            {{ removingFolder === folder.path ? "…" : "✕" }}
          </button>
        </div>
      </div>

      <!-- Пустое состояние настроек -->
      <div class="settings-empty" v-else>
        <p>Добавьте папки с музыкой, чтобы начать</p>
      </div>
    </div>

    <!-- ── Список треков ─────────────────────────────────────────────────── -->
    <div class="track-list" v-if="!showSettings && trackList.length > 0" ref="trackListEl" @scroll="onTrackListScroll">
      <div :style="{ height: vsPaddingTop + 'px' }"></div>
      <div
        v-for="track in visibleTracks"
        :key="track.file_path"
        class="track-item"
        :class="{
          active: currentTrack?.file_path === track.file_path,
          playing: currentTrack?.file_path === track.file_path && isPlaying,
        }"
        @click="playTrack(track)"
      >
        <!-- Миниатюра -->
        <div class="track-thumb">
          <img v-if="track.cover_art" :src="track.cover_art" class="track-thumb-img" alt="" draggable="false" />
          <div v-else class="track-thumb-placeholder"><span>♪</span></div>
          <div class="track-thumb-overlay" v-if="currentTrack?.file_path === track.file_path">
            <span v-if="isPlaying" class="overlay-icon playing-anim">▶</span>
            <span v-else class="overlay-icon">⏸</span>
          </div>
        </div>

        <div class="track-item-info">
          <div class="track-item-name">{{ getTrackDisplayName(track) }}</div>
          <div class="track-item-meta">
            <span class="format-badge-small" :class="{ lossless: track.is_lossless }">
              {{ track.format }}
            </span>
            <span v-if="track.duration_secs" class="track-item-duration">
              {{ formatTime(track.duration_secs) }}
            </span>
          </div>
        </div>
      </div>
      <div :style="{ height: vsPaddingBottom + 'px' }"></div>
    </div>

    <!-- Пустое состояние -->
    <div class="empty-state" v-else-if="!showSettings && !isLoading">
      <div class="empty-icon">🎵</div>
      <p>Нет треков в библиотеке</p>
      <button class="btn-action btn-action-primary" @click="showSettings = true">
        Добавить папку
      </button>
    </div>

    <!-- ── Плеер ─────────────────────────────────────────────────────────── -->
    <div class="player-bar" v-if="currentTrack" :style="currentAccentColor ? { '--accent-color': currentAccentColor } as any : {}">
      <div class="now-playing">
        <div class="cover-art-container">
          <img
            v-if="currentCoverArt"
            :src="currentCoverArt"
            class="cover-art-img"
            alt="Обложка"
            draggable="false"
          />
          <div v-else class="cover-art-placeholder">
            <span class="cover-art-note">♪</span>
          </div>
        </div>

        <div class="now-playing-text">
          <div class="now-playing-title">{{ trackTitle }}</div>
          <div class="now-playing-artist">{{ trackArtist }}</div>
          <div class="now-playing-details">
            <span class="format-badge" :class="{ lossless: currentTrack.is_lossless }">
              {{ currentTrack.is_lossless ? "LOSSLESS" : "LOSSY" }}
            </span>
            <span class="now-playing-technical">{{ trackInfo }}</span>
          </div>
        </div>
      </div>

      <!-- Прогресс-бар -->
      <div class="progress-section">
        <span class="time-label">{{ positionFormatted }}</span>
        <div class="progress-bar-container">
          <div class="progress-bar-fill" :style="{ width: progressPercent + '%' }"></div>
          <input
            type="range"
            class="progress-slider"
            min="0"
            :max="durationSecs"
            step="0.1"
            :value="isSeeking ? seekValue : positionSecs"
            @mousedown="onSeekStart"
            @input="onSeekInput"
            @mouseup="onSeekEnd"
            @touchstart="onSeekStart"
            @touchend="onSeekEnd"
          />
        </div>
        <span class="time-label">{{ durationFormatted }}</span>
      </div>

      <!-- Элементы управления -->
      <div class="controls">
        <button class="btn btn-ctrl" @click="playPrev" :disabled="!hasPrev && positionSecs < 3" title="Предыдущий">
          <span class="icon">⏮</span>
        </button>
        <button
          class="btn btn-play"
          @click="togglePlayPause"
          :disabled="isStopped || isLoading"
          :title="isPlaying ? 'Пауза' : 'Воспроизвести'"
        >
          <span class="icon">{{ isPlaying ? "⏸" : "▶" }}</span>
        </button>
        <button class="btn btn-ctrl" @click="stop" :disabled="isStopped" title="Стоп">
          <span class="icon">⏹</span>
        </button>
        <button class="btn btn-ctrl" @click="playNext" :disabled="!hasNext" title="Следующий">
          <span class="icon">⏭</span>
        </button>

        <div class="volume-control">
          <span class="volume-icon">{{ volume === 0 ? "🔇" : volume < 0.5 ? "🔉" : "🔊" }}</span>
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            :value="volume"
            @input="changeVolume"
            class="volume-slider"
            title="Громкость"
          />
          <span class="volume-value">{{ volumePercent }}%</span>
        </div>
      </div>
    </div>

    <!-- Загрузка -->
    <div class="loading" v-if="isLoading">Загрузка...</div>

    <!-- Ошибки -->
    <div class="error" v-if="errorMessage">{{ errorMessage }}</div>
  </div>
</template>

<style scoped>
.audio-player {
  width: 100%;
  max-width: 700px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  height: 100vh;
  max-height: 100vh;
  color: var(--player-text, #e0e0e0);
}

/* ── Шапка ──────────────────────────────────────────────────────────────── */
.header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px 20px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  flex-shrink: 0;
}

.header-title {
  font-size: 0.95em;
  font-weight: 600;
  letter-spacing: 0.2px;
}

.header-count {
  font-size: 0.75em;
  opacity: 0.4;
  margin-left: 2px;
  font-variant-numeric: tabular-nums;
}

.btn-icon {
  margin-left: auto;
  width: 32px;
  height: 32px;
  border: none;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.06);
  color: #e0e0e0;
  cursor: pointer;
  font-size: 1em;
  transition: all 0.15s;
  display: flex;
  align-items: center;
  justify-content: center;
}

.btn-icon:hover {
  background: rgba(255, 255, 255, 0.12);
}

.btn-icon.active {
  background: rgba(67, 97, 238, 0.3);
  color: #7b9cff;
}

/* ── Панель настроек ─────────────────────────────────────────────────────── */
.settings-panel {
  flex: 1;
  overflow-y: auto;
  padding: 20px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.settings-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.settings-title {
  font-size: 0.85em;
  font-weight: 600;
  opacity: 0.6;
  text-transform: uppercase;
  letter-spacing: 0.8px;
}

.settings-actions {
  display: flex;
  gap: 8px;
  align-items: center;
}

.btn-action {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
  border: none;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.08);
  color: #e0e0e0;
  cursor: pointer;
  font-size: 0.82em;
  transition: all 0.15s;
  white-space: nowrap;
}

.btn-action:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.14);
}

.btn-action:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.btn-action-primary {
  background: rgba(67, 97, 238, 0.3);
  color: #a0b4ff;
}

.btn-action-primary:hover:not(:disabled) {
  background: rgba(67, 97, 238, 0.45);
}

/* Прогресс сканирования */
.scan-progress-block {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 14px;
  background: rgba(255, 255, 255, 0.04);
  border-radius: 8px;
}

.scan-progress-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.scan-progress-label {
  font-size: 0.8em;
  opacity: 0.6;
}

.scan-progress-percent {
  font-size: 0.82em;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}

.scan-progress-track {
  height: 4px;
  background: rgba(255, 255, 255, 0.1);
  border-radius: 999px;
  overflow: hidden;
}

.scan-progress-fill {
  height: 100%;
  background: #4361ee;
  border-radius: 999px;
}

/* Список папок */
.folder-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.folder-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  background: rgba(255, 255, 255, 0.04);
  border-radius: 10px;
  border: 1px solid rgba(255, 255, 255, 0.06);
  transition: background 0.15s;
}

.folder-item:hover {
  background: rgba(255, 255, 255, 0.07);
}

.folder-item.folder-missing {
  border-color: rgba(255, 107, 107, 0.2);
  background: rgba(255, 107, 107, 0.05);
}

.folder-info {
  flex: 1;
  min-width: 0;
}

.folder-path {
  font-size: 0.82em;
  font-family: monospace;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  opacity: 0.85;
}

.folder-meta {
  margin-top: 3px;
  font-size: 0.75em;
}

.folder-track-count {
  opacity: 0.45;
}

.folder-missing-badge {
  color: #ff8080;
  opacity: 0.8;
}

.btn-remove {
  flex-shrink: 0;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.06);
  color: rgba(255, 255, 255, 0.4);
  cursor: pointer;
  font-size: 0.8em;
  transition: all 0.15s;
  display: flex;
  align-items: center;
  justify-content: center;
}

.btn-remove:hover:not(:disabled) {
  background: rgba(255, 80, 80, 0.2);
  color: #ff8080;
}

.btn-remove:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* Пустое состояние настроек */
.settings-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  opacity: 0.5;
  text-align: center;
  font-size: 0.9em;
}

/* ── Список треков ───────────────────────────────────────────────────────── */
.track-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px 0;
}

.track-list::-webkit-scrollbar { width: 6px; }
.track-list::-webkit-scrollbar-track { background: transparent; }
.track-list::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.15);
  border-radius: 3px;
}

.track-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 16px;
  cursor: pointer;
  transition: background 0.15s;
  user-select: none;
}

.track-item:hover { background: rgba(255, 255, 255, 0.05); }
.track-item.active { background: rgba(67, 97, 238, 0.15); }
.track-item.playing { background: rgba(67, 97, 238, 0.2); }

.track-thumb {
  position: relative;
  width: 42px;
  height: 42px;
  flex-shrink: 0;
  border-radius: 6px;
  overflow: hidden;
}

.track-thumb-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.track-thumb-placeholder {
  width: 100%;
  height: 100%;
  background: rgba(255, 255, 255, 0.07);
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 1.1em;
  opacity: 0.4;
}

.track-thumb-overlay {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
}

.overlay-icon { font-size: 0.9em; color: #fff; }

@keyframes playPulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.5; }
}

.overlay-icon.playing-anim { animation: playPulse 1.2s ease-in-out infinite; }

.track-item-info { flex: 1; min-width: 0; }

.track-item-name {
  font-size: 0.9em;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.track-item.active .track-item-name { color: #4361ee; font-weight: 500; }

.track-item-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 2px;
}

.format-badge-small {
  font-size: 0.65em;
  padding: 1px 5px;
  border-radius: 3px;
  background: rgba(255, 255, 255, 0.1);
  color: rgba(255, 255, 255, 0.5);
  font-weight: 600;
}

.format-badge-small.lossless {
  background: rgba(45, 106, 79, 0.4);
  color: #b7e4c7;
}

.track-item-duration {
  font-size: 0.75em;
  opacity: 0.4;
  font-variant-numeric: tabular-nums;
}

/* ── Пустое состояние ───────────────────────────────────────────────────── */
.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  opacity: 0.5;
  text-align: center;
  font-size: 0.9em;
}

.empty-icon { font-size: 2.5em; opacity: 0.4; }

/* ── Плеер ──────────────────────────────────────────────────────────────── */
.player-bar {
  border-top: 1px solid rgba(255, 255, 255, 0.08);
  padding: 16px 20px;
  background: rgba(0, 0, 0, 0.2);
  flex-shrink: 0;
}

.now-playing {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-bottom: 14px;
}

.cover-art-container {
  flex-shrink: 0;
  width: 72px;
  height: 72px;
  border-radius: 10px;
  overflow: hidden;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
}

.cover-art-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
  transition: transform 0.3s ease;
}

.cover-art-placeholder {
  width: 100%;
  height: 100%;
  background: rgba(255, 255, 255, 0.06);
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 10px;
}

.cover-art-note { font-size: 1.8em; opacity: 0.3; }

@keyframes coverBreath {
  0%, 100% { box-shadow: 0 4px 16px rgba(67, 97, 238, 0.25); }
  50% { box-shadow: 0 4px 24px rgba(67, 97, 238, 0.55); }
}

.cover-art-container:has(.spinning) {
  animation: coverBreath 2.4s ease-in-out infinite;
}

.now-playing-text { flex: 1; min-width: 0; text-align: left; }

.now-playing-title {
  font-size: 1.05em;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.now-playing-artist {
  font-size: 0.85em;
  opacity: 0.7;
  margin-top: 2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.now-playing-details {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 6px;
  font-size: 0.75em;
  opacity: 0.6;
}

.format-badge {
  padding: 2px 6px;
  border-radius: 3px;
  font-weight: 600;
  font-size: 0.85em;
  text-transform: uppercase;
  background: #555;
  color: #ccc;
}

.format-badge.lossless { background: #2d6a4f; color: #b7e4c7; }
.now-playing-technical { letter-spacing: 0.3px; }

/* Прогресс */
.progress-section {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
}

.time-label {
  font-size: 0.75em;
  opacity: 0.6;
  font-variant-numeric: tabular-nums;
  min-width: 36px;
  text-align: center;
}

.progress-bar-container {
  flex: 1;
  position: relative;
  height: 6px;
  background: rgba(255, 255, 255, 0.1);
  border-radius: 3px;
  cursor: pointer;
}

.progress-bar-fill {
  position: absolute;
  top: 0;
  left: 0;
  height: 100%;
  background: #4361ee;
  border-radius: 3px;
  pointer-events: none;
  transition: width 0.1s linear;
}

.progress-slider {
  position: absolute;
  top: -6px;
  left: 0;
  width: 100%;
  height: 18px;
  -webkit-appearance: none;
  appearance: none;
  background: transparent;
  cursor: pointer;
  margin: 0;
}

.progress-slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: #4361ee;
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.15s;
}

.progress-bar-container:hover .progress-slider::-webkit-slider-thumb { opacity: 1; }

.progress-slider::-moz-range-thumb {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: #4361ee;
  cursor: pointer;
  border: none;
  opacity: 0;
  transition: opacity 0.15s;
}

.progress-bar-container:hover .progress-slider::-moz-range-thumb { opacity: 1; }

/* Элементы управления */
.controls {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
}

.btn {
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.08);
  color: #fff;
  cursor: pointer;
  transition: all 0.2s ease;
  padding: 0;
}

.btn-ctrl { width: 36px; height: 36px; font-size: 1em; }

.btn:hover:not(:disabled) { background: rgba(255, 255, 255, 0.16); transform: scale(1.05); }
.btn:active:not(:disabled) { transform: scale(0.95); }
.btn:disabled { opacity: 0.25; cursor: not-allowed; }

.btn-play {
  width: 48px;
  height: 48px;
  font-size: 1.2em;
  background: #4361ee;
}

.btn-play:hover:not(:disabled) { background: #3a56d4; }
.icon { line-height: 1; }

/* Громкость */
.volume-control {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-left: auto;
}

.volume-icon { font-size: 0.95em; width: 22px; text-align: center; }

.volume-slider {
  width: 80px;
  height: 4px;
  -webkit-appearance: none;
  appearance: none;
  background: rgba(255, 255, 255, 0.15);
  border-radius: 2px;
  outline: none;
  cursor: pointer;
}

.volume-slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: #4361ee;
  cursor: pointer;
}

.volume-slider::-moz-range-thumb {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: #4361ee;
  cursor: pointer;
  border: none;
}

.volume-value {
  font-size: 0.7em;
  opacity: 0.5;
  min-width: 28px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

/* Загрузка и ошибки */
.loading {
  text-align: center;
  padding: 16px;
  opacity: 0.7;
  font-size: 0.9em;
  animation: pulse 1.5s infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 0.7; }
  50% { opacity: 0.3; }
}

.error {
  text-align: center;
  margin: 8px 20px;
  color: #ff6b6b;
  font-size: 0.8em;
  padding: 8px;
  background: rgba(255, 107, 107, 0.1);
  border-radius: 8px;
}
</style>