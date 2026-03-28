<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

// --- Типы ---
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
}

interface PlayerStatus {
  state: "Stopped" | "Playing" | "Paused";
  current_track: TrackMetadata | null;
  volume: number;
  position_secs: number;
}

// --- Состояние ---
const currentTrack = ref<TrackMetadata | null>(null);
const playbackState = ref<"Stopped" | "Playing" | "Paused">("Stopped");
const volume = ref(1.0);
const positionSecs = ref(0);
const isLoading = ref(false);
const errorMessage = ref("");
const trackList = ref<TrackMetadata[]>([]);
const currentDir = ref("");
const isSeeking = ref(false);
const seekValue = ref(0);
let statusInterval: ReturnType<typeof setInterval> | null = null;

// --- Вычисляемые свойства ---
const trackTitle = computed(() => {
  if (!currentTrack.value) return "Нет трека";
  return currentTrack.value.title || extractFileName(currentTrack.value.file_path);
});

const trackArtist = computed(() => {
  return currentTrack.value?.artist || "Неизвестный исполнитель";
});

const trackAlbum = computed(() => {
  return currentTrack.value?.album || "";
});

const trackInfo = computed(() => {
  if (!currentTrack.value) return "";
  const parts: string[] = [];
  parts.push(currentTrack.value.format);
  if (currentTrack.value.sample_rate) {
    parts.push(`${(currentTrack.value.sample_rate / 1000).toFixed(1)} kHz`);
  }
  if (currentTrack.value.bit_depth) {
    parts.push(`${currentTrack.value.bit_depth} bit`);
  }
  if (currentTrack.value.channels) {
    parts.push(currentTrack.value.channels === 2 ? "Stereo" : `${currentTrack.value.channels}ch`);
  }
  if (currentTrack.value.bitrate) {
    parts.push(`${currentTrack.value.bitrate} kbps`);
  }
  return parts.join(" • ");
});

const durationSecs = computed(() => currentTrack.value?.duration_secs ?? 0);

const durationFormatted = computed(() => {
  if (!durationSecs.value) return "--:--";
  return formatTime(durationSecs.value);
});

const positionFormatted = computed(() => {
  return formatTime(isSeeking.value ? seekValue.value : positionSecs.value);
});

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

const hasNext = computed(() => {
  return currentTrackIndex.value >= 0 && currentTrackIndex.value < trackList.value.length - 1;
});

const hasPrev = computed(() => {
  return currentTrackIndex.value > 0;
});

// --- Утилиты ---
function extractFileName(filePath: string): string {
  const parts = filePath.replace(/\\/g, "/").split("/");
  const fileName = parts[parts.length - 1];
  return fileName.replace(/\.[^/.]+$/, "");
}

function formatTime(seconds: number): string {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

function getTrackDisplayName(track: TrackMetadata): string {
  if (track.title) {
    if (track.artist) return `${track.artist} — ${track.title}`;
    return track.title;
  }
  return extractFileName(track.file_path);
}

// --- Действия ---
async function openDirectory() {
  try {
    errorMessage.value = "";
    const dirPath = await open({
      directory: true,
      multiple: false,
    });

    if (dirPath) {
      await loadDirectory(dirPath as string);
    }
  } catch (e: any) {
    errorMessage.value = `Ошибка открытия директории: ${e}`;
  }
}

async function loadDirectory(dirPath: string) {
  try {
    isLoading.value = true;
    errorMessage.value = "";
    const tracks = await invoke<TrackMetadata[]>("scan_directory", { dirPath });
    trackList.value = tracks;
    currentDir.value = dirPath;
    if (tracks.length === 0) {
      errorMessage.value = "В директории не найдено аудиофайлов";
    }
  } catch (e: any) {
    errorMessage.value = `Ошибка сканирования: ${e}`;
  } finally {
    isLoading.value = false;
  }
}

async function playTrack(track: TrackMetadata) {
  try {
    isLoading.value = true;
    errorMessage.value = "";
    const metadata = await invoke<TrackMetadata>("player_play", { filePath: track.file_path });
    currentTrack.value = metadata;
    playbackState.value = "Playing";
    positionSecs.value = 0;
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
  if (hasNext.value) {
    await playTrack(trackList.value[currentTrackIndex.value + 1]);
  }
}

async function playPrev() {
  // Если прошло больше 3 секунд — перемотать в начало, иначе предыдущий трек
  if (positionSecs.value > 3) {
    await seekTo(0);
  } else if (hasPrev.value) {
    await playTrack(trackList.value[currentTrackIndex.value - 1]);
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

// --- Прогресс-бар ---
function onSeekStart(event: Event) {
  isSeeking.value = true;
  const target = event.target as HTMLInputElement;
  seekValue.value = parseFloat(target.value);
}

function onSeekInput(event: Event) {
  const target = event.target as HTMLInputElement;
  seekValue.value = parseFloat(target.value);
}

async function onSeekEnd(event: Event) {
  const target = event.target as HTMLInputElement;
  const newPos = parseFloat(target.value);
  isSeeking.value = false;
  await seekTo(newPos);
}

async function seekTo(position: number) {
  try {
    await invoke("player_seek_crossfade", { positionSecs: position, crossfadeSecs: 0.25 });
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

    if (!isSeeking.value) {
      positionSecs.value = status.position_secs;
    }

    if (status.current_track) {
      currentTrack.value = status.current_track;
    } else if (status.state === "Stopped") {
      // Трек закончился — автоматически следующий
      if (currentTrack.value && hasNext.value) {
        await playNext();
      } else {
        currentTrack.value = null;
        positionSecs.value = 0;
      }
    }
  } catch {
    // Игнорируем ошибки поллинга
  }
}

// --- Жизненный цикл ---
onMounted(() => {
  statusInterval = setInterval(pollStatus, 250);
});

onUnmounted(() => {
  if (statusInterval) {
    clearInterval(statusInterval);
  }
});
</script>

<template>
  <div class="audio-player">
    <!-- Заголовок и выбор директории -->
    <div class="header">
      <button class="btn btn-open" @click="openDirectory" :disabled="isLoading" title="Открыть папку">
        <span class="icon">📂</span>
        <span class="btn-text">Открыть папку</span>
      </button>
      <div class="current-dir" v-if="currentDir" :title="currentDir">
        {{ currentDir.replace(/\\/g, "/").split("/").pop() }}
      </div>
    </div>

    <!-- Список треков -->
    <div class="track-list" v-if="trackList.length > 0">
      <div
        v-for="(track, index) in trackList"
        :key="track.file_path"
        class="track-item"
        :class="{
          active: currentTrack?.file_path === track.file_path,
          playing: currentTrack?.file_path === track.file_path && isPlaying,
        }"
        @click="playTrack(track)"
        @dblclick="playTrack(track)"
      >
        <div class="track-item-index">
          <span v-if="currentTrack?.file_path === track.file_path && isPlaying" class="playing-indicator">▶</span>
          <span v-else-if="currentTrack?.file_path === track.file_path && playbackState === 'Paused'" class="playing-indicator paused">⏸</span>
          <span v-else>{{ index + 1 }}</span>
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
    </div>

    <!-- Пустое состояние -->
    <div class="empty-state" v-else-if="!isLoading">
      <p>Выберите папку с музыкой</p>
    </div>

    <!-- Нижняя панель плеера -->
    <div class="player-bar" v-if="currentTrack">
      <!-- Информация о треке -->
      <div class="now-playing">
        <div class="now-playing-title">{{ trackTitle }}</div>
        <div class="now-playing-artist">{{ trackArtist }}</div>
        <div class="now-playing-details">
          <span class="format-badge" :class="{ lossless: currentTrack.is_lossless }">
            {{ currentTrack.is_lossless ? "LOSSLESS" : "LOSSY" }}
          </span>
          <span class="now-playing-technical">{{ trackInfo }}</span>
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

        <!-- Громкость -->
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

/* Заголовок */
.header {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 16px 20px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.btn-open {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 16px;
  border: none;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.08);
  color: #e0e0e0;
  cursor: pointer;
  font-size: 0.85em;
  transition: all 0.2s;
}

.btn-open:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.14);
}

.btn-text {
  white-space: nowrap;
}

.current-dir {
  font-size: 0.8em;
  opacity: 0.5;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Список треков */
.track-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px 0;
}

.track-list::-webkit-scrollbar {
  width: 6px;
}

.track-list::-webkit-scrollbar-track {
  background: transparent;
}

.track-list::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.15);
  border-radius: 3px;
}

.track-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 20px;
  cursor: pointer;
  transition: background 0.15s;
  user-select: none;
}

.track-item:hover {
  background: rgba(255, 255, 255, 0.05);
}

.track-item.active {
  background: rgba(67, 97, 238, 0.15);
}

.track-item.playing {
  background: rgba(67, 97, 238, 0.2);
}

.track-item-index {
  width: 28px;
  text-align: center;
  font-size: 0.8em;
  opacity: 0.5;
  flex-shrink: 0;
}

.track-item.active .track-item-index {
  opacity: 1;
  color: #4361ee;
}

.playing-indicator {
  color: #4361ee;
  font-size: 0.9em;
}

.playing-indicator.paused {
  opacity: 0.6;
}

.track-item-info {
  flex: 1;
  min-width: 0;
}

.track-item-name {
  font-size: 0.9em;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.track-item.active .track-item-name {
  color: #4361ee;
  font-weight: 500;
}

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

/* Пустое состояние */
.empty-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  opacity: 0.4;
  font-size: 0.95em;
}

/* Нижняя панель плеера */
.player-bar {
  border-top: 1px solid rgba(255, 255, 255, 0.08);
  padding: 16px 20px;
  background: rgba(0, 0, 0, 0.2);
}

.now-playing {
  text-align: center;
  margin-bottom: 12px;
}

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
}

.now-playing-details {
  display: flex;
  align-items: center;
  justify-content: center;
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

.format-badge.lossless {
  background: #2d6a4f;
  color: #b7e4c7;
}

.now-playing-technical {
  letter-spacing: 0.3px;
}

/* Прогресс-бар */
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

.progress-bar-container:hover .progress-slider::-webkit-slider-thumb {
  opacity: 1;
}

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

.progress-bar-container:hover .progress-slider::-moz-range-thumb {
  opacity: 1;
}

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

.btn-ctrl {
  width: 36px;
  height: 36px;
  font-size: 1em;
}

.btn:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.16);
  transform: scale(1.05);
}

.btn:active:not(:disabled) {
  transform: scale(0.95);
}

.btn:disabled {
  opacity: 0.25;
  cursor: not-allowed;
}

.btn-play {
  width: 48px;
  height: 48px;
  font-size: 1.2em;
  background: #4361ee;
}

.btn-play:hover:not(:disabled) {
  background: #3a56d4;
}

.icon {
  line-height: 1;
}

/* Громкость */
.volume-control {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-left: auto;
}

.volume-icon {
  font-size: 0.95em;
  width: 22px;
  text-align: center;
}

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
  0%,
  100% {
    opacity: 0.7;
  }
  50% {
    opacity: 0.3;
  }
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
