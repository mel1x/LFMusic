<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useRoute } from 'vue-router';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import trackCover from '../../assets/trackCover.webp';
import { toast } from 'vue-sonner';

import MoreIcon from '../Icons/MoreIcon.vue';
import HeartIcon from '../Icons/HeartIcon.vue';
import TrackContextMenu from '../TrackContextMenu.vue';
import TrackArtists from '../TrackArtists.vue';
import ShuffleIcon from '../Icons/ShuffleIcon.vue';
import PrevIcon from '../Icons/PrevIcon.vue';
import PlayIcon from '../Icons/PlayIcon.vue';
import PauseIcon from '../Icons/PauseIcon.vue';
import NextIcon from '../Icons/NextIcon.vue';
import RepeatIcon from '../Icons/RepeatIcon.vue';
import RepeatOneIcon from '../Icons/RepeatOneIcon.vue';
import ListIcon from '../Icons/ListIcon.vue';
import VolumeMaxIcon from '../Icons/VolumeMaxIcon.vue';
import VolumeMinIcon from '../Icons/VolumeMinIcon.vue';
import VolumeZeroIcon from '../Icons/VolumeZeroIcon.vue';
import FullscreenIcon from '../Icons/FullscreenIcon.vue';
import MinimizeScreenIcon from '../Icons/MinimizeScreenIcon.vue';
import MaximizeIcon from '../Icons/MaximizeIcon.vue';
import MinimizeIcon from '../Icons/MinimizeIcon.vue';
import CloseIcon from '../Icons/CloseIcon.vue';

import { usePlayer, type TrackMetadata } from '../../composables/usePlayer';
import { usePlayerUi } from '../../composables/usePlayerUi';
import { usePlaylists } from '../../composables/usePlaylists';
import { createRipple } from '../../composables/useRipple';
import { isPhoneViewport } from '../../composables/usePlatform';

const win = getCurrentWindow();

const {
    currentTrack,
    volume,
    positionSecs,
    isSeeking,
    seekValue,
    durationSecs,
    durationFormatted,
    positionFormatted,
    isPlaying,
    isStopped,
    isLoading,
    shuffled,
    repeatMode,
    trackTitle,
    volumePercent,
    formatTime,
    togglePlayPause,
    playNext,
    playPrev,
    playTrack,
    toggleShuffle,
    toggleRepeat,
    onSeekStart,
    onSeekInput,
    onSeekEnd,
    saveState,
} = usePlayer();

const {
    fsMode,
    showOverlay,
    closeFullscreen,
    toggleFsMode,
    fsCoverA,
    fsCoverB,
    fsActiveBuffer,
    toBlurCover,
    queueSourcePrefix,
    queueSourceName,
    isMixSource,
    canMoveToTop,
    displayQueue,
    moveTrackToTop,
    toggleMute,
    onVolumeScroll,
    onVolumeInput,
} = usePlayerUi();

const { isFavorite, toggleFavorite, ensureFavoritesLoaded } = usePlaylists();
void ensureFavoritesLoaded();
function onQfToggleFav(track: TrackMetadata) {
    void toggleFavorite(track.file_path);
}

// Close overlay on page navigation.
const route = useRoute();
watch(() => route.fullPath, () => {
    if (showOverlay.value) closeFullscreen();
});

const progressSliderValue = computed(() => {
    return isSeeking.value ? seekValue.value : positionSecs.value;
});

const fsProgressPercent = computed(() => {
    if (!durationSecs.value) return '0%';
    return ((progressSliderValue.value / durationSecs.value) * 100) + '%';
});

const minimizeWindow = () => win.minimize();
// Backend handles this: if in OS fullscreen (F11), click exits it and maximizes;
// otherwise it's a normal maximize toggle.
const maximizeWindow = () => invoke('toggle_window_maximize');
const closeWindow = async () => {
    await saveState();
    win.close();
};

const qfTrackMenu = ref<{ x: number; y: number; track: TrackMetadata } | null>(null);

function onQfTrackClick(e: MouseEvent, track: TrackMetadata) {
    createRipple(e);
    if (isPhoneViewport()) {
        onQfCoverClick(track);
    }
}

function onQfCoverClick(track: TrackMetadata) {
    if (currentTrack.value?.file_path === track.file_path) {
        togglePlayPause();
        return;
    }
    playTrack(track, { manualListStart: true });
}

function openQfTrackMenu(e: MouseEvent, track: TrackMetadata) {
    e.stopPropagation();
    if (qfTrackMenu.value?.track.file_path === track.file_path) {
        qfTrackMenu.value = null;
        return;
    }
    const btn = (e.currentTarget as HTMLElement);
    const rect = btn.getBoundingClientRect();
    qfTrackMenu.value = { x: rect.left - 6, y: rect.top + rect.height / 2, track };
}

const qfCoverMenu = ref<{ x: number; y: number } | null>(null);

function openQfCoverMenu(e: MouseEvent) {
    if (!currentTrack.value) return;
    e.stopPropagation();
    if (qfCoverMenu.value) {
        qfCoverMenu.value = null;
        return;
    }
    const btn = (e.currentTarget as HTMLElement);
    const rect = btn.getBoundingClientRect();
    qfCoverMenu.value = { x: rect.left + rect.width / 2, y: rect.top - 6 };
}
</script>

<template>
    <Teleport to="body">
        <Transition name="fs">
            <div
                v-if="showOverlay"
                class="fullscreen-overlay"
                :class="[fsMode === 'queue' ? 'mode-queue' : 'mode-player']"
                @keydown.escape="closeFullscreen"
                tabindex="-1"
            >
                <div class="fs-bg">
                    <img
                        v-if="fsCoverA"
                        :src="toBlurCover(fsCoverA)"
                        class="fs-bg-img"
                        :class="{ active: fsActiveBuffer === 'a' }"
                        draggable="false"
                    />
                    <img
                        v-if="fsCoverB"
                        :src="toBlurCover(fsCoverB)"
                        class="fs-bg-img"
                        :class="{ active: fsActiveBuffer === 'b' }"
                        draggable="false"
                    />
                </div>
                <div class="fs-titlebar" data-tauri-drag-region>
                    <div></div>
                    <div class="fs-win-controls">
                        <button class="fs-win-btn" @click="minimizeWindow"><MinimizeIcon /></button>
                        <button class="fs-win-btn" @click="maximizeWindow"><MaximizeIcon /></button>
                        <button class="fs-win-btn fs-win-close" @click="closeWindow"><CloseIcon /></button>
                    </div>
                </div>
                <button class="fs-close-btn" @click="closeFullscreen"><MinimizeScreenIcon /></button>
                <div class="qf-content">
                    <div class="qf-left">
                        <div class="qf-cover-container" :class="{ 'menu-open': qfCoverMenu }">
                            <div class="qf-cover-wrapper">
                                <img
                                    :src="fsCoverA || trackCover"
                                    alt="cover"
                                    class="fs-cover"
                                    :class="{ active: fsActiveBuffer === 'a' }"
                                    draggable="false"
                                />
                                <img
                                    :src="fsCoverB || trackCover"
                                    alt="cover"
                                    class="fs-cover"
                                    :class="{ active: fsActiveBuffer === 'b' }"
                                    draggable="false"
                                />
                            </div>
                            <div class="qf-cover-dim"></div>

                            <div class="qf-player-controls desktop-only" :class="{ 'menu-open': qfCoverMenu }">
                                <button
                                    class="qf-corner-btn qf-corner-tr"
                                    :disabled="!currentTrack"
                                    @click="toggleFsMode"
                                >
                                    <FullscreenIcon v-if="fsMode === 'queue'" />
                                    <ListIcon v-else />
                                </button>
                                <button
                                    class="qf-corner-btn qf-corner-bl"
                                    :disabled="!currentTrack"
                                    @mousedown="openQfCoverMenu($event)"
                                >
                                    <MoreIcon />
                                </button>
                                <div class="qf-volume qf-corner-br" @wheel.prevent="onVolumeScroll">
                                    <div class="qf-volume-slider-popup">
                                        <input
                                            type="range"
                                            class="qf-volume-slider"
                                            min="0"
                                            max="1"
                                            step="0.01"
                                            :value="volume"
                                            :style="{ '--fs-vol': volumePercent + '%' }"
                                            @input.stop="onVolumeInput"
                                        />
                                        <span class="qf-volume-label">{{ volumePercent }}%</span>
                                    </div>
                                    <button class="qf-corner-btn qf-volume-btn" @click="toggleMute">
                                        <VolumeZeroIcon v-if="volume === 0" />
                                        <VolumeMinIcon v-else-if="volume <= 0.4" />
                                        <VolumeMaxIcon v-else />
                                    </button>
                                </div>
                                <button
                                    class="qf-side-btn qf-side-left"
                                    :class="{ active: shuffled }"
                                    :disabled="!currentTrack"
                                    @click="() => { toggleShuffle(); toast.info(shuffled ? 'Случайный порядок' : 'Воспроизведение подряд') }"
                                >
                                    <ShuffleIcon />
                                </button>
                                <button
                                    class="qf-side-btn qf-side-right"
                                    :class="{ active: repeatMode === 'track' }"
                                    :disabled="!currentTrack"
                                    @click="() => { toggleRepeat(); toast.warning(repeatMode === 'track' ? 'Повтор трека' : 'Повтор плейлиста') }"
                                >
                                    <RepeatOneIcon v-if="repeatMode === 'track'" />
                                    <RepeatIcon v-else />
                                </button>
                                <div class="qf-center-controls">
                                    <button class="qf-center-btn" :disabled="!currentTrack" @click="playPrev">
                                        <PrevIcon />
                                    </button>
                                    <button class="qf-center-play" @click="togglePlayPause" :disabled="isStopped && !isLoading">
                                        <PauseIcon v-if="isPlaying" style="margin-left:1px;" />
                                        <PlayIcon v-else style="margin-left:3px;" />
                                    </button>
                                    <button class="qf-center-btn" :disabled="!currentTrack" @click="playNext()">
                                        <NextIcon />
                                    </button>
                                </div>
                            </div>
                        </div>

                        <div class="fs-info desktop-only">
                            <div v-truncate class="fs-title">{{ trackTitle }}</div>
                            <div class="fs-artist"><TrackArtists :artist="currentTrack?.artist" /></div>
                        </div>
                        <div class="fs-progress qf-progress-hover desktop-only">
                            <div class="fs-progress-bar-wrapper">
                                <input
                                    type="range"
                                    class="fs-progress-slider"
                                    min="0"
                                    :max="durationSecs || 0"
                                    step="0.1"
                                    :value="progressSliderValue"
                                    @mousedown="onSeekStart"
                                    @input="onSeekInput"
                                    @mouseup="onSeekEnd"
                                    @touchstart="onSeekStart"
                                    @touchend="onSeekEnd"
                                    :disabled="!currentTrack"
                                />
                                <div class="fs-progress-track">
                                    <div class="fs-progress-fill" :style="{ width: fsProgressPercent }"></div>
                                </div>
                            </div>
                            <div class="fs-times">
                                <span>{{ positionFormatted }}</span>
                                <span>{{ durationFormatted }}</span>
                            </div>
                        </div>

                        <div class="qf-mobile-layout mobile-only">
                            <div class="qf-mobile-meta-row">
                                <div class="fs-info">
                                    <div v-truncate class="fs-title">{{ trackTitle }}</div>
                                    <div class="fs-artist"><TrackArtists :artist="currentTrack?.artist" /></div>
                                </div>
                                <button
                                    class="qf-mobile-more-btn"
                                    :disabled="!currentTrack"
                                    @mousedown="openQfCoverMenu($event)"
                                >
                                    <MoreIcon />
                                </button>
                            </div>

                            <div class="fs-progress qf-progress-hover">
                                <div class="fs-progress-bar-wrapper">
                                    <input
                                        type="range"
                                        class="fs-progress-slider"
                                        min="0"
                                        :max="durationSecs || 0"
                                        step="0.1"
                                        :value="progressSliderValue"
                                        @mousedown="onSeekStart"
                                        @input="onSeekInput"
                                        @mouseup="onSeekEnd"
                                        @touchstart="onSeekStart"
                                        @touchend="onSeekEnd"
                                        :disabled="!currentTrack"
                                    />
                                    <div class="fs-progress-track">
                                        <div class="fs-progress-fill" :style="{ width: fsProgressPercent }"></div>
                                    </div>
                                </div>
                                <div class="fs-times">
                                    <span>{{ positionFormatted }}</span>
                                    <span>{{ durationFormatted }}</span>
                                </div>
                            </div>

                            <div class="qf-mobile-playback-row">
                                <button
                                    class="qf-mobile-side-btn"
                                    :class="{ active: shuffled }"
                                    :disabled="!currentTrack"
                                    @click="() => { toggleShuffle(); toast.info(shuffled ? 'Случайный порядок' : 'Воспроизведение подряд') }"
                                >
                                    <ShuffleIcon />
                                </button>
                                <button class="qf-mobile-playback-btn" :disabled="!currentTrack" @click="playPrev">
                                    <PrevIcon />
                                </button>
                                <button class="qf-mobile-playback-play" @click="togglePlayPause" :disabled="isStopped && !isLoading">
                                    <PauseIcon v-if="isPlaying" style="margin-left:1px;" />
                                    <PlayIcon v-else style="margin-left:3px;" />
                                </button>
                                <button class="qf-mobile-playback-btn" :disabled="!currentTrack" @click="playNext()">
                                    <NextIcon />
                                </button>
                                <button
                                    class="qf-mobile-side-btn"
                                    :class="{ active: repeatMode === 'track' }"
                                    :disabled="!currentTrack"
                                    @click="() => { toggleRepeat(); toast.warning(repeatMode === 'track' ? 'Повтор трека' : 'Повтор плейлиста') }"
                                >
                                    <RepeatOneIcon v-if="repeatMode === 'track'" />
                                    <RepeatIcon v-else />
                                </button>
                            </div>

                            <button
                                class="qf-mobile-queue-toggle-btn"
                                :disabled="!currentTrack"
                                @click="toggleFsMode"
                            >
                                <ListIcon class="qf-mobile-queue-icon" />
                                <span>Очередь</span>
                            </button>
                        </div>
                    </div>
                    <div class="qf-right-wrap">
                        <div class="qf-right">
                            <div class="qf-queue-header" style="color:#fff;margin-bottom:2px;margin-top:2px;">{{ queueSourcePrefix }}</div>
                            <div v-truncate class="qf-queue-header styled-header" style="color:#fff;margin-top:2px;">{{ queueSourceName }}</div>
                            <div class="qf-queue-list">
                                <div
                                    v-if="currentTrack"
                                    class="qf-queue-item qf-queue-current"
                                    :class="{ activePaused: !isPlaying }"
                                    @click="onQfTrackClick($event, currentTrack)"
                                >
                                    <div class="qf-hover-icon" @click.stop="onQfCoverClick(currentTrack)">
                                        <PauseIcon v-if="isPlaying" width="10" height="10" style="margin-left:0.5px;" />
                                        <PlayIcon v-else width="10" height="10" style="margin-left:1px;" />
                                    </div>
                                    <div class="qf-queue-cover-wrap" @click.stop="onQfCoverClick(currentTrack)">
                                        <img
                                            :src="currentTrack.cover_art || trackCover"
                                            alt="cover"
                                            class="qf-queue-cover-img"
                                            draggable="false"
                                        />
                                    </div>
                                    <div class="qf-queue-info">
                                        <div v-truncate class="qf-queue-title">{{ trackTitle }}</div>
                                        <div class="qf-queue-artist"><TrackArtists :artist="currentTrack.artist" /></div>
                                    </div>
                                    <button
                                        class="qf-queue-fav"
                                        :class="{ liked: isFavorite(currentTrack.file_path) }"
                                        @click.stop="onQfToggleFav(currentTrack)"
                                        @mousedown.stop
                                    >
                                        <HeartIcon width="15" height="15" />
                                    </button>
                                    <div class="qf-queue-duration" :class="{ hidden: qfTrackMenu?.track.file_path === currentTrack.file_path }">{{ durationFormatted }}</div>
                                    <div class="qf-hover-options" :class="{ menuOpen: qfTrackMenu?.track.file_path === currentTrack.file_path }" @mousedown.stop="openQfTrackMenu($event, currentTrack)">
                                        <MoreIcon width="26" height="26" />
                                    </div>
                                </div>
                                <div class="qf-queue-header" style="margin-top:12px;margin-bottom:2px;">{{ isMixSource ? 'Следующий трек' : 'Далее в очереди' }}</div>
                                <div
                                    v-for="(track, i) in displayQueue"
                                    :key="i"
                                    class="qf-queue-item"
                                    @click="onQfTrackClick($event, track)"
                                >
                                    <div class="qf-hover-icon" @click.stop="onQfCoverClick(track)">
                                        <PlayIcon width="10" height="10" style="margin-left:1px;" />
                                    </div>
                                    <div class="qf-queue-cover-wrap" @click.stop="onQfCoverClick(track)">
                                        <img
                                            :src="track.cover_art || trackCover"
                                            alt="cover"
                                            class="qf-queue-cover-img"
                                            draggable="false"
                                        />
                                    </div>
                                    <div class="qf-queue-info">
                                        <div v-truncate class="qf-queue-title">{{ track.title || track.file_path.split(/[/\\]/).pop() }}</div>
                                        <div class="qf-queue-artist"><TrackArtists :artist="track.artist" /></div>
                                    </div>
                                    <button
                                        class="qf-queue-fav"
                                        :class="{ liked: isFavorite(track.file_path) }"
                                        @click.stop="onQfToggleFav(track)"
                                        @mousedown.stop
                                    >
                                        <HeartIcon width="15" height="15" />
                                    </button>
                                    <div class="qf-queue-duration" :class="{ hidden: qfTrackMenu?.track.file_path === track.file_path }">{{ track.duration_secs ? formatTime(track.duration_secs) : '--:--' }}</div>
                                    <div class="qf-hover-options" :class="{ menuOpen: qfTrackMenu?.track.file_path === track.file_path }" @mousedown.stop="openQfTrackMenu($event, track)">
                                        <MoreIcon width="26" height="26" />
                                    </div>
                                </div>
                                <div v-if="displayQueue.length === 0" class="qf-queue-empty">
                                    Нет треков в очереди
                                </div>
                            </div>
                        </div>
                    </div>
                </div>
                <TrackContextMenu
                    v-if="qfTrackMenu"
                    :x="qfTrackMenu.x"
                    :y="qfTrackMenu.y"
                    :track="qfTrackMenu.track"
                    alignRight
                    centerY
                    :canMoveToTop="canMoveToTop"
                    @close="qfTrackMenu = null"
                    @moveToTop="moveTrackToTop"
                />
                <TrackContextMenu
                    v-if="qfCoverMenu && currentTrack"
                    :x="qfCoverMenu.x"
                    :y="qfCoverMenu.y"
                    :track="currentTrack"
                    anchorBottom
                    centerX
                    submenuRight
                    :canMoveToTop="canMoveToTop"
                    @close="qfCoverMenu = null"
                    @moveToTop="moveTrackToTop"
                />
            </div>
        </Transition>
    </Teleport>
</template>

<style scoped>
.fullscreen-overlay {
    position: fixed;
    top: 0;
    left: 0;
    width: calc(100vw / var(--overlay-scale, 1));
    height: calc(100vh / var(--overlay-scale, 1));
    z-index: 999999;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #0a0a0a;
    overflow: hidden;
    contain: layout style;
}
.fs-bg {
    position: absolute;
    inset: -40px;
    z-index: 0;
    overflow: hidden;
}
.fs-bg-img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    filter: blur(80px) brightness(0.3) saturate(1.4);
    transform: scale(1.1);
    opacity: 0;
    transition: opacity 0.6s ease;
    will-change: opacity;
}
.fs-bg-img.active {
    opacity: 1;
}
.fs-titlebar {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 46px;
    z-index: 10;
    display: flex;
    justify-content: space-between;
}
.fs-close-btn {
    position: absolute;
    top: 16px;
    left: 16px;
    z-index: 10;
    width: 40px;
    height: 40px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: hsla(0, 0%, 100%, 0.08);
    color: hsla(0, 0%, 100%, 0.7);
    border-radius: 50%;
    cursor: pointer;
    transition: background .24s, color .24s;
}
.fs-close-btn:hover {
    background: hsla(0, 0%, 100%, 0.15);
    color: #fff;
}
.fs-win-controls {
    display: flex;
    align-items: center;
    height: 38px;
}
.fs-win-btn {
    cursor: pointer;
    width: 46px;
    height: 38px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    background-color: transparent;
    transition: background-color 0.14s ease-in-out;
}
.fs-win-btn:hover {
    background-color: hsla(0, 0%, 100%, 0.1);
}
.fs-win-close:hover {
    background-color: #f23f42;
}
.fs-cover {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    user-select: none;
    opacity: 0;
    transition: opacity 0.5s ease;
    will-change: opacity;
}
.fs-cover.active {
    opacity: 1;
}
.fs-info {
    text-align: center;
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    max-width: 100%;
    min-width: 0;
}
.fs-title {
    cursor: default;
    width: 100%;
    max-width: 100%;
    font-size: 22px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.85);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    letter-spacing: -0.01em;

    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 1);
    }
}
.fs-artist {
    cursor: default;
    width: 100%;
    max-width: 100%;
    font-size: 16px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.55);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;

    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 0.7);
    }
}
.fs-album {
    cursor: default;
    width: 100%;
    max-width: 100%;
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.35);
    margin-top: 2px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;

    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 0.5);
    }
}
.fs-progress {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 8px;
}
.fs-progress-bar-wrapper {
    position: relative;
    width: 100%;
    height: 14px;
    display: flex;
    align-items: center;
}
.fs-progress-slider {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    appearance: none;
    -webkit-appearance: none;
    cursor: pointer;
    outline: none;
    background: transparent;
    z-index: 2;
}
.fs-progress-slider::-webkit-slider-runnable-track {
    background: transparent;
    height: 14px;
}
.fs-progress-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #fff;
    cursor: pointer;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.4);
    margin-top: 1px;
    opacity: 0;
    transition: opacity 0.25s ease;
}
.fs-progress:hover .fs-progress-slider::-webkit-slider-thumb,
.fs-progress-slider:active::-webkit-slider-thumb {
    opacity: 1;
}
.fs-progress-slider:disabled {
    opacity: 0.3;
    cursor: default;
}
.fs-progress-track {
    position: absolute;
    left: 0;
    right: 0;
    height: 4px;
    border-radius: 2px;
    background-color: hsla(0, 0%, 100%, 0.12);
    z-index: 1;
    overflow: hidden;
    transition: background-color 0.25s ease;
}
.fs-progress-fill {
    height: 100%;
    width: 0%;
    border-radius: 2px;
    background-color: hsla(0, 0%, 100%, 0.7);
    transition: background-color 0.25s ease;
}
.fs-progress:hover .fs-progress-track,
.fs-progress-bar-wrapper:hover .fs-progress-track {
    background-color: hsla(0, 0%, 100%, 0.18);
}
.fs-progress:hover .fs-progress-fill,
.fs-progress-bar-wrapper:hover .fs-progress-fill {
    background-color: #fff;
}
.fs-times {
    display: flex;
    justify-content: space-between;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.4);
    user-select: none;
}

.qf-content {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: row;
    align-items: stretch;
    justify-content: center;
    width: 100%;
    height: 100%;
    padding: 56px 48px 48px;
    gap: 48px;
    contain: layout style;
}
.mobile-only {
    display: none !important;
}
.desktop-only {
}
.qf-left {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    align-self: center;
    gap: 24px;
    flex: 0 0 440px;
    width: 440px;
    min-width: 0;
    position: relative;
    z-index: 2;
    transition: transform .48s cubic-bezier(0.22, 0.61, 0.36, 1);
}
.mode-player .qf-left {
    transform: translateX(calc((720px + 48px) / 2));
}
.qf-cover-container {
    position: relative;
    width: 440px;
    height: 440px;
    flex-shrink: 0;
}
.qf-cover-wrapper {
    position: relative;
    width: 440px;
    height: 440px;
    border-radius: 20px;
    overflow: hidden;
    box-shadow: 0 28px 90px rgba(0, 0, 0, 0.55);
    flex-shrink: 0;
    transition: .24s ease;
}
.qf-cover-dim {
    position: absolute;
    inset: 0;
    background: hsla(0, 0%, 0%, 0.45);
    opacity: 0;
    transition: opacity .48s ease;
    pointer-events: none;
    z-index: 2;
    border-radius: 20px;
}
.qf-cover-container:hover .qf-cover-dim,
.qf-cover-container.menu-open .qf-cover-dim {
    opacity: 1;
}
.qf-player-controls {
    position: absolute;
    inset: 0;
    z-index: 3;
    opacity: 0;
    transition: opacity .48s ease;
    pointer-events: none;
}
.qf-cover-container:hover .qf-player-controls,
.qf-cover-container.menu-open .qf-player-controls {
    opacity: 1;
    pointer-events: auto;
}
.qf-corner-btn {
    position: absolute;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background: hsla(var(--accent-h), min(var(--accent-s), 60%), var(--accent-l), 0.9);
    color: #fff;
    cursor: pointer;
    transition: background .24s, color .24s, transform .24s;
}
.qf-corner-btn:hover {
    background: hsla(var(--accent-h), min(var(--accent-s), 70%), calc(var(--accent-l) * 1.15), 1);
    transform: scale(1.08);
}
.qf-corner-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
}
.qf-corner-tr {
    top: 16px;
    right: 16px;
}
.qf-corner-bl {
    bottom: 16px;
    left: 16px;
}
.qf-corner-br {
    position: absolute;
    bottom: 16px;
    right: 16px;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-direction: row-reverse;
}
.qf-volume-btn {
    position: static;
    flex-shrink: 0;
}
.qf-volume-slider-popup {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 40px;
    padding: 0 14px 0 16px;
    border-radius: 999px;
    background: hsla(var(--accent-h), min(var(--accent-s), 60%), var(--accent-l), 0.9);
    max-width: 0;
    padding-left: 0;
    padding-right: 0;
    opacity: 0;
    overflow: hidden;
    pointer-events: none;
    transition: max-width .32s cubic-bezier(0.22, 0.61, 0.36, 1),
                padding .32s cubic-bezier(0.22, 0.61, 0.36, 1),
                opacity .24s ease;
}
.qf-corner-br:hover .qf-volume-slider-popup {
    max-width: 220px;
    padding: 0 14px 0 16px;
    opacity: 1;
    pointer-events: auto;
}
.qf-volume-slider {
    width: 120px;
    height: 4px;
    appearance: none;
    -webkit-appearance: none;
    border-radius: 2px;
    cursor: pointer;
    outline: none;
    background: linear-gradient(
        to right,
        #fff var(--fs-vol, 50%),
        hsla(0, 0%, 100%, 0.25) var(--fs-vol, 50%)
    );
    flex-shrink: 0;
}
.qf-volume-slider::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: transparent;
}
.qf-volume-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #fff;
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.3);
    margin-top: -4px;
}
.qf-volume-label {
    font-size: 13px;
    font-weight: 600;
    min-width: 36px;
    color: #fff;
    user-select: none;
    white-space: nowrap;
}
.qf-side-btn {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    border-radius: 50%;
    background: hsla(var(--accent-h), min(var(--accent-s), 60%), var(--accent-l), 0.9);
    color: hsla(0, 0%, 100%, 0.85);
    cursor: pointer;
    transition: color .24s, background .24s, transform .24s;
}
.qf-side-btn:hover {
    color: #fff;
    background: hsla(var(--accent-h), min(var(--accent-s), 70%), calc(var(--accent-l) * 1.15), 1);
    transform: translateY(-50%) scale(1.08);
}
.qf-side-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
}
.qf-side-btn.active {
    color: #fff;
    background: hsla(var(--accent-h), min(var(--accent-s), 80%), calc(var(--accent-l) * 1.25), 1);
}
.qf-side-left {
    left: 24px;
}
.qf-side-right {
    right: 24px;
}
.qf-center-controls {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: center;
    gap: 14px;
}
.qf-center-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 52px;
    height: 52px;
    border-radius: 50%;
    background: hsla(var(--accent-h), min(var(--accent-s), 60%), var(--accent-l), 0.9);
    color: #fff;
    cursor: pointer;
    transition: color .24s, background .24s, transform .24s;
}
.qf-center-btn:hover {
    background: hsla(var(--accent-h), min(var(--accent-s), 70%), calc(var(--accent-l) * 1.15), 1);
    transform: scale(1.08);
}
.qf-center-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
}
.qf-center-play {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 72px;
    height: 72px;
    border-radius: 50%;
    background: #fff;
    color: hsla(var(--accent-h), min(var(--accent-s), 60%), var(--accent-l), 1);
    cursor: pointer;
    transition: transform .24s ease;
}
.qf-center-play:hover {
    transform: scale(1.06);
}
.qf-center-play:disabled {
    opacity: 0.35;
    cursor: not-allowed;
}
.qf-progress-hover .fs-times {
    opacity: 0.45;
    transition: opacity .24s ease;
}
.qf-progress-hover:hover .fs-times,
.qf-progress-hover .fs-progress-slider:hover + .fs-times,
.qf-progress-hover .fs-progress-slider:focus + .fs-times,
.qf-progress-hover .fs-progress-slider:active + .fs-times {
    opacity: 0.9;
}
.qf-right-wrap {
    position: relative;
    flex: 0 0 720px;
    width: 720px;
    max-width: 720px;
    z-index: 1;
    transition: transform .48s cubic-bezier(0.22, 0.61, 0.36, 1),
                opacity .18s ease;
}
.mode-player .qf-right-wrap {
    transform: translateX(calc(-100% - 48px));
    opacity: 0;
    pointer-events: none;
}
.qf-right {
    display: flex;
    flex-direction: column;
    justify-content: center;
    width: 100%;
    height: 100%;
    overflow-y: auto;
}
.qf-queue-header {
    font-size: 14px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.75);
    margin-top: 8px;
    margin-bottom: 12px;
    padding-left: 4px;
    user-select: none;

    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 0.95);
    }
}
.qf-queue-header.styled-header {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 19px;
    margin-bottom: 8px;
}
.qf-queue-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
}
.qf-queue-item {
    position: relative;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    padding: 6px 16px 6px 6px;
    border-radius: 9px;
    cursor: pointer;
    transition: .31s ease-in-out;
    user-select: none;
    overflow: hidden;
    background-color: hsla(0, 0%, 100%, 0);
}
.qf-queue-item:hover {
    background-color: hsla(0, 0%, 100%, 0.03);
}
.qf-queue-current {
    background-color: hsla(0, 0%, 100%, 0.05);
}
.qf-queue-current:hover {
    background-color: hsla(0, 0%, 100%, 0.06);
}
.qf-queue-item :deep(.ripple) {
    position: absolute;
    border-radius: 50%;
    background: hsla(0, 0%, 100%, 0.08);
    transform: scale(0);
    animation: qf-ripple-expand 0.5s ease-out forwards;
    pointer-events: none;
}
@keyframes qf-ripple-expand {
    to {
        transform: scale(1);
        opacity: 0;
    }
}
.qf-hover-icon {
    position: absolute;
    top: 50%;
    /* cover center: padding-left 6px + 44/2 = 28px */
    left: 28px;
    transform: translate(-50%, -50%);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    opacity: 0;
    border-radius: 9999px;
    color: #242424;
    background-color: hsla(0, 0%, 100%, 1.0);
    transition: .31s ease;
    z-index: 3;
}
.qf-queue-item:hover .qf-hover-icon,
.qf-queue-current.activePaused .qf-hover-icon {
    opacity: 1;
}
.qf-queue-cover-wrap {
    width: 44px;
    height: 44px;
    background-color: #202020;
    border-radius: 6px;
    transition: .31s ease;
    overflow: hidden;
    flex-shrink: 0;
}
.qf-queue-cover-img {
    user-select: none;
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
}
.qf-queue-item:hover .qf-queue-cover-wrap,
.qf-queue-current.activePaused .qf-queue-cover-wrap {
    opacity: 0.5;
}
.qf-queue-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow: hidden;
    flex: 1;
    min-width: 0;
}
.qf-queue-title {
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.qf-queue-current .qf-queue-title {
    color: hsla(0, 0%, 100%, 1);
}
.qf-queue-artist {
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.4);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: .24s ease;
}
.qf-queue-item:hover .qf-queue-artist {
    color: hsla(0, 0%, 100%, 0.6);
}
.qf-queue-duration {
    margin-left: auto;
    font-size: 14px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.45);
    transition: .24s ease;
    flex-shrink: 0;
    opacity: 1;
}
.qf-queue-item:hover .qf-queue-duration {
    opacity: 0;
}
.qf-queue-duration.hidden {
    opacity: 0 !important;
}
.qf-queue-fav {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4px;
    margin-right: 4px;
    background: none;
    border: none;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.35);
    transition: color .24s ease;
    z-index: 3;
}
.qf-queue-fav:hover {
    color: hsla(0, 0%, 100%, 0.85);
}
.qf-queue-fav.liked {
    color: hsla(0, 0%, 100%, 0.95);
}
.qf-queue-fav.liked :deep(svg path) {
    fill: currentColor;
}
.qf-hover-options {
    position: absolute;
    top: 50%;
    right: 12px;
    padding: 6px;
    transform: translateY(-50%);
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.45);
    transition: .31s ease;
    opacity: 0;
}
.qf-hover-options:hover {
    color: hsla(0, 0%, 100%, 0.7);
}
.qf-queue-item:hover .qf-hover-options,
.qf-hover-options.menuOpen {
    opacity: 1;
}
.qf-queue-empty {
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.3);
    padding: 24px 12px;
    text-align: center;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .mobile-only {
        display: unset !important;
    }

    .desktop-only {
        display: none !important;
    }

    .qf-left {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        width: 100%;
        max-width: 420px;
        align-self: center;
        gap: 20px;
        transform: none !important;
        margin: auto 0;
    }

    .qf-cover-container {
        position: relative;
        width: min(88vw, 360px);
        height: min(88vw, 360px);
        border-radius: 12px;
        box-shadow: 0 22px 60px rgba(0, 0, 0, 0.45);
        flex-shrink: 0;
    }

    .qf-cover-wrapper {
        width: 100%;
        height: 100%;
        border-radius: 12px;
        overflow: hidden;
    }

    .qf-cover-dim {
        display: none !important;
    }

    .qf-mobile-layout {
        display: flex !important;
        flex-direction: column !important;
        width: 100% !important;
        gap: 20px !important;
        align-items: stretch !important;
    }

    .qf-mobile-meta-row {
        display: flex !important;
        flex-direction: row !important;
        align-items: center !important;
        justify-content: space-between !important;
        width: 100% !important;
        gap: 12px !important;
    }

    .qf-mobile-meta-row .fs-info {
        flex: 1 !important;
        display: flex !important;
        flex-direction: column !important;
        align-items: flex-start !important;
        text-align: left !important;
        min-width: 0 !important;
        gap: 4px !important;
    }

    .qf-mobile-meta-row .fs-title {
        font-size: 20px !important;
        font-weight: 800 !important;
        color: #ffffff !important;
        width: 100% !important;
    }

    .qf-mobile-meta-row .fs-artist {
        font-size: 14px !important;
        color: hsla(0, 0%, 100%, 0.6) !important;
        width: 100% !important;
    }

    .qf-mobile-more-btn {
        width: 40px !important;
        height: 40px !important;
        min-width: 40px !important;
        border-radius: 50% !important;
        background: hsla(0, 0%, 100%, 0.08) !important;
        color: hsla(0, 0%, 100%, 0.8) !important;
        display: flex !important;
        align-items: center !important;
        justify-content: center !important;
        cursor: pointer !important;
        border: none !important;
        transition: background-color 0.2s !important;
    }

    .qf-mobile-more-btn:active {
        background: hsla(0, 0%, 100%, 0.16) !important;
    }

    .qf-mobile-more-btn svg {
        width: 20px !important;
        height: 20px !important;
    }

    .qf-mobile-layout .fs-progress {
        width: 100% !important;
        max-width: none !important;
        margin-top: 0 !important;
    }

    .qf-mobile-layout .fs-times {
        opacity: 1 !important;
        display: flex !important;
        justify-content: space-between !important;
        margin-top: 6px !important;
        font-size: 11px !important;
        color: hsla(0, 0%, 100%, 0.5) !important;
    }

    .qf-mobile-playback-row {
        display: flex !important;
        flex-direction: row !important;
        align-items: center !important;
        justify-content: space-between !important;
        width: 100% !important;
        padding: 0 4px !important;
    }

    .qf-mobile-playback-btn {
        width: 40px !important;
        height: 40px !important;
        border-radius: 50% !important;
        background: hsla(0, 0%, 100%, 0.08) !important;
        color: #ffffff !important;
        display: flex !important;
        align-items: center !important;
        justify-content: center !important;
        cursor: pointer !important;
        border: none !important;
        transition: background-color 0.2s !important;
    }

    .qf-mobile-playback-btn:active {
        background: hsla(0, 0%, 100%, 0.16) !important;
    }

    .qf-mobile-playback-btn svg {
        width: 16px !important;
        height: 16px !important;
    }

    .qf-mobile-playback-play {
        width: 58px !important;
        height: 58px !important;
        border-radius: 50% !important;
        background: #ffffff !important;
        color: #121212 !important;
        display: flex !important;
        align-items: center !important;
        justify-content: center !important;
        cursor: pointer !important;
        border: none !important;
        box-shadow: 0 4px 16px rgba(0, 0, 0, 0.25) !important;
        transition: background-color 0.2s, transform 0.1s !important;
    }

    .qf-mobile-playback-play:active {
        background: hsla(0, 0%, 100%, 0.9) !important;
        transform: scale(0.96) !important;
    }

    .qf-mobile-playback-play svg {
        width: 22px !important;
        height: 22px !important;
    }

    .qf-mobile-side-btn {
        width: 40px !important;
        height: 40px !important;
        border-radius: 50% !important;
        background: transparent !important;
        color: hsla(0, 0%, 100%, 0.5) !important;
        display: flex !important;
        align-items: center !important;
        justify-content: center !important;
        cursor: pointer !important;
        border: none !important;
        transition: color 0.2s !important;
    }

    .qf-mobile-side-btn.active {
        color: hsla(0, 0%, 100%, 0.95) !important;
    }

    .qf-mobile-side-btn svg {
        width: 20px !important;
        height: 20px !important;
    }

    .qf-mobile-queue-toggle-btn {
        width: 100% !important;
        height: 48px !important;
        border-radius: 9999px !important;
        border: none !important;
        background: hsla(0, 0%, 100%, 0.08) !important;
        color: hsla(0, 0%, 100%, 0.9) !important;
        font-size: 14px !important;
        font-weight: 600 !important;
        display: flex !important;
        align-items: center !important;
        justify-content: center !important;
        gap: 8px !important;
        cursor: pointer !important;
        transition: background-color 0.2s !important;
        margin-top: 6px !important;
    }

    .qf-mobile-queue-toggle-btn:active {
        background: hsla(0, 0%, 100%, 0.16) !important;
    }

    .qf-mobile-queue-icon {
        width: 18px !important;
        height: 18px !important;
    }

    .fullscreen-overlay {
        width: 100vw;
        height: 100dvh;
        align-items: stretch;
        justify-content: stretch;
    }

    .fs-bg {
        inset: -24px;
    }

    .fs-bg-img {
        filter: blur(56px) brightness(0.28) saturate(1.35);
    }

    .fs-titlebar,
    .fs-win-controls {
        display: none;
    }

    .fs-close-btn {
        top: calc(var(--safe-top) + 12px);
        left: calc(var(--safe-left) + 12px);
        width: 44px;
        height: 44px;
        background: hsla(0, 0%, 100%, 0.11);
        backdrop-filter: blur(12px);
    }

    .qf-content {
        flex-direction: column;
        align-items: center;
        justify-content: flex-start;
        min-height: 100%;
        height: 100%;
        padding: calc(var(--safe-top) + 68px) calc(var(--safe-right) + 16px) calc(var(--safe-bottom) + 18px) calc(var(--safe-left) + 16px);
        gap: 18px;
        overflow-y: auto;
        contain: none;
    }

    .qf-left {
        flex: 0 0 auto;
        width: 100%;
        max-width: 420px;
        align-self: center;
        justify-content: flex-start;
        gap: 14px;
        transform: none !important;
    }


    .qf-corner-btn,
    .qf-side-btn,
    .qf-center-btn {
        width: 44px;
        height: 44px;
    }

    .qf-corner-tr {
        top: 12px;
        right: 12px;
    }

    .qf-corner-bl {
        bottom: 12px;
        left: 12px;
    }

    .qf-corner-br {
        right: 12px;
        bottom: 12px;
    }

    .qf-volume-slider-popup {
        display: none;
    }

    .qf-side-left {
        left: 16px;
    }

    .qf-side-right {
        right: 16px;
    }

    .qf-center-controls {
        gap: 10px;
    }

    .qf-center-play {
        width: 64px;
        height: 64px;
    }

    .fs-info {
        gap: 5px;
        padding: 0 4px;
    }

    .fs-title {
        font-size: 21px;
    }

    .fs-artist {
        font-size: 15px;
    }

    .fs-album {
        font-size: 13px;
    }

    .fs-progress {
        width: 100%;
        max-width: 420px;
    }

    .fs-times {
        opacity: 1 !important;
    }

    .qf-right-wrap {
        flex: 1 1 auto;
        width: 100%;
        max-width: 520px;
        transform: none !important;
        opacity: 1 !important;
    }

    .mode-player .qf-right-wrap {
        display: none;
    }

    .mode-queue .qf-right-wrap {
        display: block;
    }

    .mode-queue .qf-left {
        display: none;
    }

    .qf-right {
        justify-content: flex-start;
        height: auto;
        overflow-y: visible;
        padding-bottom: 12px;
    }

    .qf-queue-header {
        padding-left: 2px;
        margin-top: 4px;
    }

    .qf-queue-header.styled-header {
        font-size: 21px;
        line-height: 1.15;
        white-space: normal;
    }

    .qf-queue-list {
        gap: 3px;
    }

    .qf-queue-item {
        min-height: 58px;
        padding: 7px 48px 7px 7px;
        border-radius: 12px;
    }

    .qf-queue-cover-wrap {
        width: 44px;
        height: 44px;
        opacity: 0.78;
    }

    .qf-hover-icon {
        left: 28px;
        opacity: 0;
    }
    .qf-queue-current .qf-hover-icon {
        opacity: 1;
    }

    .qf-queue-duration {
        display: none;
    }

    .qf-hover-options {
        right: 8px;
        width: 40px;
        height: 40px;
        padding: 7px;
        opacity: 1;
    }
}

.fs-enter-active {
    transition: opacity .35s ease, transform .35s ease;
}
.fs-leave-active {
    transition: opacity .25s ease, transform .25s ease;
}
.fs-enter-from {
    opacity: 0;
    transform: scale(0.96) translateY(20px);
}
.fs-leave-to {
    opacity: 0;
    transform: scale(0.96) translateY(20px);
}
</style>
