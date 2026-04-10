<script setup lang="ts">
import { ref, computed, watch } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import trackCover from '../assets/trackCover.png';
import { Toaster, toast } from 'vue-sonner'
import 'vue-sonner/style.css'

import MoreIcon from './Icons/MoreIcon.vue';
import TrackContextMenu from './TrackContextMenu.vue';
import ShuffleIcon from './Icons/ShuffleIcon.vue';
import PrevIcon from './Icons/PrevIcon.vue';
import PlayIcon from './Icons/PlayIcon.vue';
import PauseIcon from './Icons/PauseIcon.vue';
import NextIcon from './Icons/NextIcon.vue';
import RepeatIcon from './Icons/RepeatIcon.vue';
import RepeatOneIcon from './Icons/RepeatOneIcon.vue';
import ListIcon from './Icons/ListIcon.vue';
import VolumeMaxIcon from './Icons/VolumeMaxIcon.vue';
import VolumeMinIcon from './Icons/VolumeMinIcon.vue';
import VolumeZeroIcon from './Icons/VolumeZeroIcon.vue';
import FullscreenIcon from './Icons/FullscreenIcon.vue';
import MinimizeScreenIcon from './Icons/MinimizeScreenIcon.vue';
import MaximizeIcon from './Icons/MaximizeIcon.vue';
import MinimizeIcon from './Icons/MinimizeIcon.vue';
import CloseIcon from './Icons/CloseIcon.vue';

import { useSettings } from '../composables/useSettings';
import { usePlayer } from '../composables/usePlayer';

const win = getCurrentWindow();

const { settings } = useSettings();

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
    trackArtist,
    trackAlbum,
    currentCoverArt,
    codecDisplay,
    codecDetails,
    volumePercent,
    togglePlayPause,
    playNext,
    playPrev,
    toggleShuffle,
    toggleRepeat,
    setVolume,
    onSeekStart,
    onSeekInput,
    onSeekEnd,
    saveState,
} = usePlayer();

// ── Fullscreen mode ────────────────────────────────────────────────────────

const showFullscreen = ref(false);

function openFullscreen() {
    if (currentTrack.value) showFullscreen.value = true;
}

function closeFullscreen() {
    showFullscreen.value = false;
}

// ── Fullscreen cover crossfade ─────────────────────────────────────────────

const fsCoverA = ref<string | null>(null);
const fsCoverB = ref<string | null>(null);
const fsActiveBuffer = ref<'a' | 'b'>('a');

watch(currentCoverArt, (newCover) => {
    if (!showFullscreen.value) {
        fsCoverA.value = newCover;
        fsCoverB.value = newCover;
        fsActiveBuffer.value = 'a';
        return;
    }
    if (fsActiveBuffer.value === 'a') {
        fsCoverB.value = newCover;
        fsActiveBuffer.value = 'b';
    } else {
        fsCoverA.value = newCover;
        fsActiveBuffer.value = 'a';
    }
}, { immediate: true });

watch(showFullscreen, (val) => {
    if (val) {
        fsCoverA.value = currentCoverArt.value;
        fsCoverB.value = currentCoverArt.value;
        fsActiveBuffer.value = 'a';
    }
});

// ── Window controls (for fullscreen titlebar) ──────────────────────────────

const minimizeWindow = () => win.minimize();
const maximizeWindow = () => win.toggleMaximize();
const closeWindow = async () => {
    await saveState();
    win.close();
};

// ── Fullscreen progress fill ───────────────────────────────────────────────

const fsProgressPercent = computed(() => {
    if (!durationSecs.value) return '0%';
    return ((progressSliderValue.value / durationSecs.value) * 100) + '%';
});

// ── Volume ──────────────────────────────────────────────────────────────────

const prevVolume = ref(volume.value || 0.66);

function toggleMute() {
    if (volume.value === 0) {
        setVolume(prevVolume.value || 0.66);
    } else {
        prevVolume.value = volume.value;
        setVolume(0);
    }
}

function onVolumeScroll(event: WheelEvent) {
    event.preventDefault();
    const delta = event.deltaY < 0 ? 0.01 : -0.01;
    const newVol = Math.min(1, Math.max(0, volume.value + delta));
    setVolume(newVol);
}

function onVolumeInput(event: Event) {
    const val = parseFloat((event.target as HTMLInputElement).value);
    setVolume(val);
}

// ── Progress slider ─────────────────────────────────────────────────────────

const progressSliderValue = computed(() => {
    return isSeeking.value ? seekValue.value : positionSecs.value;
});

const progressFillPercent = computed(() => {
    if (!durationSecs.value) return 0;
    return (progressSliderValue.value / durationSecs.value) * 100;
});

// ── Codec info ──────────────────────────────────────────────────────────────


const playerControlsEl = ref<HTMLElement | null>(null)

function onPlayerRipple(e: MouseEvent) {
    const el = playerControlsEl.value
    if (!el) return
    const rect = el.getBoundingClientRect()
    const ripple = document.createElement('span')
    ripple.className = 'ripple'
    const size = Math.max(rect.width, rect.height) * 2
    ripple.style.width = ripple.style.height = size + 'px'
    ripple.style.left = (e.clientX - rect.left - size / 2) + 'px'
    ripple.style.top = (e.clientY - rect.top - size / 2) + 'px'
    el.appendChild(ripple)
    ripple.addEventListener('animationend', () => ripple.remove())
}

// ── Track context menu ────────────────────────────────────────────────────

const playerTrackMenu = ref<{ x: number; y: number } | null>(null)

function openPlayerTrackMenu(e: MouseEvent) {
    if (!currentTrack.value) return
    e.stopPropagation()
    if (playerTrackMenu.value) {
        playerTrackMenu.value = null
        return
    }
    const btn = (e.currentTarget as HTMLElement)
    const rect = btn.getBoundingClientRect()
    playerTrackMenu.value = { x: rect.left + rect.width / 2, y: rect.top - 6 }
}

async function moveTrackToTop(track: import('../composables/usePlayer').TrackMetadata) {
    const { trackList } = usePlayer()
    const idx = trackList.value.findIndex(t => t.file_path === track.file_path)
    if (idx > 0) {
        const [item] = trackList.value.splice(idx, 1)
        trackList.value.unshift(item)
    }
    await invoke('library_move_track_to_top', { filePath: track.file_path })
}
</script>

<template>
    <Toaster 
        style="z-index: 0;"
        :toastOptions="{
          unstyled: true,
          style: { 
            bottom: '86px',
            right: '440px',
          },
          class: 'toast',
        }"
        :visibleToasts="1"
        theme="dark"
    >
        <template #info-icon>
            <ShuffleIcon width="30" height="30" style="background-color:#ffffff20;padding:7px;border-radius:999px;color:#ffffffa0" />
        </template>
        <template #warning-icon>
            <RepeatIcon width="30" height="30" style="background-color:#ffffff20;padding:7px;border-radius:999px;color:#ffffffa0" />
        </template>
    </Toaster>
    <div class="player">
        <div class="asideWrapper">
            <div v-if="settings.show_player_extra_block" class="aside">
            </div>
        </div>
        <div class="playerControlsWrapper">
            <input
                type="range"
                class="progress-slider"
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
            <div ref="playerControlsEl" class="playerControls" :class="{ seeking: isSeeking }" >
                <div class="progress-bar-bg"></div>
                <div class="progress-bar-fill" :style="{ width: progressFillPercent + '%' }"></div>
                <div class="progress-bg" :style="{ width: progressFillPercent + '%' }"></div>
            </div>
            <div class="botPart" @click.self="onPlayerRipple">
                <div class="trackInfo">
                    <div class="coverWrapper">
                        <button class="fullscreenBtn" @click="openFullscreen"><FullscreenIcon /></button>
                        <img
                            v-if="currentCoverArt"
                            :src="currentCoverArt"
                            alt="cover"
                            class="trackCover"
                            draggable="false"
                        />
                        <img v-else :src="trackCover" alt="music" class="trackCover"></img>
                    </div>
                    <div class="trackTitle" v-if="currentTrack">
                        <div class="topLine">
                            <div class="trackName">{{ trackTitle }}</div>
                            <button class="moreBtn" @mousedown="openPlayerTrackMenu($event)"><MoreIcon /></button>
                        </div>
                        <div class="artistName">{{ trackArtist }}</div>
                    </div>
                    <div class="trackTitle" v-else>
                        <div class="topLine">
                            <div class="trackName noTrack">Нет трека</div>
                        </div>
                    </div>
                </div>
                <div class="controls">
                    <button
                        class="shuffleBtn"
                        :class="{ active: shuffled }"
                        :disabled="!currentTrack"
                        @click="() => { 
                            toggleShuffle()
                            toast.info(shuffled ? 'Случайный порядок' : 'Воспроизведение подряд')
                        }"
                    >
                        <ShuffleIcon />
                    </button>
                    <button
                        class="prevBtn"
                        :disabled="!currentTrack"
                        @click="playPrev"
                    >
                        <PrevIcon />
                    </button>
                    <button
                        class="playBtn"
                        @click="togglePlayPause"
                        :disabled="isStopped && !isLoading"
                                            >
                        <PauseIcon style="margin-left:1px;" v-if="isPlaying" />
                        <PlayIcon style="margin-left:2px;" v-else />
                    </button>
                    <button
                        class="nextBtn"
                        :disabled="!currentTrack"
                        @click="playNext()"
                    >
                        <NextIcon />
                    </button>
                    <button
                        class="repeatBtn"
                        :class="{ active: repeatMode === 'track' }"
                        :disabled="!currentTrack"
                        @click="() => { 
                            toggleRepeat()
                            toast.warning(repeatMode === 'track' ? 'Повтор трека' : 'Повтор плейлиста')
                        }"
                    >
                        <RepeatOneIcon v-if="repeatMode === 'track'" />
                        <RepeatIcon v-else />
                    </button>
                </div>
                <div class="additionalControls">
                    <div v-if="currentTrack" class="queueList"><ListIcon /></div>
                    <div class="codecInfo" v-if="currentTrack && codecDisplay">
                        {{ codecDisplay }}
                        <div class="codecTooltip">
                            <span class="codecTooltip-title">Качество трека</span>
                            <span class="codecTooltip-details">{{ codecDetails }}</span>
                        </div>
                    </div>
                    <div
                        class="volumeControl"
                        @wheel.prevent="onVolumeScroll"
                    >
                        <div class="volumeSliderPopup">
                            <input
                                type="range"
                                class="volume-slider"
                                min="0"
                                max="1"
                                step="0.01"
                                :value="volume"
                                :style="{ '--val': volumePercent + '%' }"
                                @input.stop="onVolumeInput"
                                orient="vertical"
                            />
                        </div>
                        <div class="volumeIconWrap" @click="toggleMute">
                            <VolumeZeroIcon v-if="volume === 0" />
                            <VolumeMinIcon v-else-if="volume <= 0.4" />
                            <VolumeMaxIcon v-else />
                            <span>{{ volumePercent }}%</span>
                        </div>
                    </div>
                </div>
            </div>
        </div>
        <TrackContextMenu
            v-if="playerTrackMenu && currentTrack"
            :x="playerTrackMenu.x"
            :y="playerTrackMenu.y"
            :track="currentTrack"
            anchorBottom
            centerX
            submenuRight
            @close="playerTrackMenu = null"
            @moveToTop="moveTrackToTop"
        />
    </div>

    <!-- Fullscreen player overlay -->
    <Teleport to="body">
        <Transition name="fs">
            <div v-if="showFullscreen" class="fullscreen-overlay" @keydown.escape="closeFullscreen">
                <div class="fs-bg">
                    <img
                        v-if="fsCoverA"
                        :src="fsCoverA"
                        class="fs-bg-img"
                        :class="{ active: fsActiveBuffer === 'a' }"
                        draggable="false"
                    />
                    <img
                        v-if="fsCoverB"
                        :src="fsCoverB"
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
                <div class="fs-content">
                    <div class="fs-main">
                        <div class="fs-cover-wrapper">
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
                        <div class="fs-info">
                            <div class="fs-title">{{ trackTitle }}</div>
                            <div class="fs-artist">{{ trackArtist }}</div>
                            <div v-if="trackAlbum" class="fs-album">{{ trackAlbum }}</div>
                        </div>
                        <div class="fs-progress">
                            <input
                                type="range"
                                class="fs-progress-slider"
                                min="0"
                                :max="durationSecs || 0"
                                step="0.1"
                                :value="progressSliderValue"
                                :style="{ '--fs-val': fsProgressPercent }"
                                @mousedown="onSeekStart"
                                @input="onSeekInput"
                                @mouseup="onSeekEnd"
                                @touchstart="onSeekStart"
                                @touchend="onSeekEnd"
                                :disabled="!currentTrack"
                            />
                            <div class="fs-times">
                                <span>{{ positionFormatted }}</span>
                                <span>{{ durationFormatted }}</span>
                            </div>
                        </div>
                        <div class="fs-controls">
                            <button
                                class="fs-btn fs-shuffle"
                                :class="{ active: shuffled }"
                                :disabled="!currentTrack"
                                @click="() => { toggleShuffle(); toast.info(shuffled ? 'Случайный порядок' : 'Воспроизведение подряд') }"
                            >
                                <ShuffleIcon />
                            </button>
                            <button class="fs-btn" :disabled="!currentTrack" @click="playPrev">
                                <PrevIcon />
                            </button>
                            <button class="fs-play-btn" @click="togglePlayPause" :disabled="isStopped && !isLoading">
                                <PauseIcon v-if="isPlaying" style="margin-left:1px;" />
                                <PlayIcon v-else style="margin-left:3px;" />
                            </button>
                            <button class="fs-btn" :disabled="!currentTrack" @click="playNext()">
                                <NextIcon />
                            </button>
                            <button
                                class="fs-btn fs-repeat"
                                :class="{ active: repeatMode === 'track' }"
                                :disabled="!currentTrack"
                                @click="() => { toggleRepeat(); toast.warning(repeatMode === 'track' ? 'Повтор трека' : 'Повтор плейлиста') }"
                            >
                                <RepeatOneIcon v-if="repeatMode === 'track'" />
                                <RepeatIcon v-else />
                            </button>
                        </div>
                        <div class="fs-extra">
                            <div v-if="codecDisplay" class="fs-codec">
                                {{ codecDisplay }}
                                <div class="codecTooltip">
                                    <span class="codecTooltip-title">Качество трека</span>
                                    <span class="codecTooltip-details">{{ codecDetails }}</span>
                                </div>
                            </div>
                            <div class="fs-volume-group" @wheel.prevent="onVolumeScroll">
                                <div
                                    class="fs-volume-icon"
                                    @click="toggleMute"
                                >
                                    <VolumeZeroIcon v-if="volume === 0" />
                                    <VolumeMinIcon v-else-if="volume <= 0.4" />
                                    <VolumeMaxIcon v-else />
                                </div>
                                <input
                                    type="range"
                                    class="fs-volume-slider"
                                    min="0"
                                    max="1"
                                    step="0.01"
                                    :value="volume"
                                    :style="{ '--fs-vol': volumePercent + '%' }"
                                    @input.stop="onVolumeInput"
                                />
                                <span class="fs-volume-label">{{ volumePercent }}%</span>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        </Transition>
    </Teleport>
</template>

<style scoped>
.player {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    height: 104px;
    z-index: 1;
}
.asideWrapper {
    width: 240px;
    height: 100%;
    padding: 12px;
}
.aside {
    width: 100%;
    height: 100%;
    border-radius: 14px;
    background-color: #1a1a1a;
    border: 1px solid #ffffff04;
}
.playerControlsWrapper {
    display: flex;
    flex-direction: column;
    position: relative;
    width: calc(100% - 240px);
    height: 100%;
    padding: 12px 12px 12px 0px;
}
.progress-slider {
    overflow: visible;
    margin-bottom: -6px;
    margin-left: -4px;
    width: calc(100% + 4px * 2);
    height: 7px;
    background-color: transparent;
    appearance: none;
    -webkit-appearance: none;
    cursor: pointer;
    z-index: 100000;
}
.progress-slider:disabled {
    cursor: default;
    opacity: 0.3;
}
.progress-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;

    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #fff;
    opacity: 0;
    transition: opacity 0.3s ease;
}
.progress-slider:hover::-webkit-slider-thumb {
    opacity: 1;
}
.playerControls {
    position: relative;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 104px;
    border-radius: 14px;
    background-color: var(--accent-color);
    transition: .24s ease;
}
.progress-bar-bg {
    height: 4px;
    width: 100%;
    background-color: hsla(0, 0%, 100%, 0.12);
    opacity: 0;
    transition: opacity 0.3s ease;
}
.progress-bar-fill {
    position: absolute;
    top: 0;
    left: 0;
    height: 4px;
    width: 0px;
    background-color: hsla(0, 0%, 100%, 0.65);
    opacity: 0;
    transition: width 0.25s linear, opacity 0.3s ease;
}
.progress-bg {
    position: absolute;
    top: 0;
    left: 0;
    height: 104px;
    width: 0%;
    z-index: 2;
    background-color: hsla(0, 0%, 100%, 0.11);
    transition: width 0.25s linear;
}
.playerControlsWrapper:hover .progress-bar-bg {
    opacity: 1;
}
.playerControlsWrapper:hover .progress-bar-fill {
    opacity: 1;
}
.botPart {
    left: 0;
    top: 6px;
    position: absolute;
    height: calc(104px - 6px);
    width: calc(100% - 12px);
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    z-index: 3;
}

.trackInfo {
    margin-left: 8px;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 0.5rem;
}
.coverWrapper {
    position: relative;
    margin-top: -3px;
    width: 64px;
    height: 64px;
    border-radius: 8px;
    overflow: hidden;
    transition: .24s;
    flex-shrink: 0;
}
.fullscreenBtn {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    position: absolute;
    top: 11px;
    left: 10.5px;
    width: 42px;
    height: 42px;
    color: #fff;
    background: hsla(var(--accent-h), var(--accent-s), 20%, 1);
    border-radius: 50%;
    cursor: pointer;
    transition: background-color .24s, opacity .3s ease-in-out;
    z-index: 4;
    opacity: 0;

    &:hover {
        background-color: hsla(var(--accent-h), var(--accent-s), 35%, 0.9);
    }
}
.trackCover {
    user-select: none;
    width: 64px;
    height: 64px;
    border-radius: 8px;
    transition: .24s;
    object-fit: cover;
}
.coverWrapper:hover .trackCover {
    filter: brightness(0.7);
}
.coverWrapper:hover .fullscreenBtn {
    opacity: 1;
}
.trackTitle {
    margin-top: -6px;
    display: flex;
    flex-direction: column;
    gap: 1px;
}
.topLine {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    max-width: 200px;
    width: 200px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.topLine .trackName {
    user-select: none;
    cursor: default;
    font-size: 15px;
    font-weight: 500;
    color: #fff;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 200px;
}
.topLine .trackName.noTrack {
    color: hsla(0, 0%, 100%, 0.3);
}
.topLine .moreBtn {
    cursor: pointer;
    border-radius: 999px;
    color: hsla(0, 0%, 100%, 0.5);
    width: 20px;
    height: 20px;
    background: transparent;
    transition: .24s;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.artistName {
    max-width: 200px;
    width: max-content;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
    cursor: default;
    font-size: 15px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.5);
    transition: .24s;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.controls {
    margin-top: -3px;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: center;
}
.shuffleBtn {
    padding: 8px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.5);
    cursor: pointer;
    margin-right: 8px;
    transition: .24s;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.shuffleBtn:disabled {
    opacity: 0.25;
    cursor: not-allowed;
}
.shuffleBtn.active {
    color: hsla(0, 0%, 100%, 0.95);
}
.prevBtn {
    cursor: pointer;
    padding: 8px;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.5);
    transition: .24s;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.prevBtn:disabled {
    opacity: 0.25;
    cursor: not-allowed;
}
.playBtn {
    margin: 0px 4px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background-color: #ffffff;
    transition: .24s ease-in-out;
    transform-origin: center;
    color: var(--accent-color);
}
.playBtn:disabled {
    opacity: 0.25;
    cursor: not-allowed;
}
.nextBtn {
    cursor: pointer;
    padding: 8px;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.5);
    transition: .24s;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.nextBtn:disabled {
    opacity: 0.25;
    cursor: not-allowed;
}
.repeatBtn {
    padding: 8px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.5);
    cursor: pointer;
    margin-left: 8px;
    transition: .24s;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.repeatBtn:disabled {
    opacity: 0.25;
    cursor: not-allowed;
}
.repeatBtn.active {
    color: hsla(0, 0%, 100%, 0.95);
}
.additionalControls {
    margin-top: -3px;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: end;
    gap: 24px;
    margin-right: 16px;
    max-width: 260px;
    width: 260px;
}
.queueList {
    margin-top: -1px;
    cursor: pointer;
    padding: 8px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.5);
    transition: .24s;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.codecInfo {
    user-select: none;
    cursor: default;
    font-size: 15px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.5);
    transition: .24s;
    position: relative;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }

    &:hover .codecTooltip {
        visibility: visible;
        opacity: 1;
    }
}
.volumeControl {
    user-select: none;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;

    padding: 8px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.5);
    transition: .24s;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.volumeIconWrap {
    position: relative;
    z-index: 12;
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
}
.volumeIconWrap span {
    font-size: 14px;
    font-weight: 600;
    min-width: 38.5px;
}
.volumeControl {
    position: relative;
}
.volumeSliderPopup {
    visibility: hidden;
    opacity: 0;
    pointer-events: none;

    display: flex;
    position: absolute;
    bottom: calc(100% - 4px);
    left: 50%;
    transform: translateX(-50%);
    background: var(--dark-bg);
    border: 2px solid #ffffff20;
    border-radius: 24px;
    padding: 16px 2px;

    z-index: 11;
    transition: opacity 0.2s ease, visibility 0.2s ease;
}
.volumeSliderPopup::after {
    content: '';
    position: absolute;
    top: -15px;
    bottom: -30px;
    left: -15px;
    right: -15px;
    z-index: 0;
}
.volumeControl:hover .volumeSliderPopup {
    visibility: visible;
    opacity: 1;
    pointer-events: auto;
}
.volume-slider {
    position: relative;
    z-index: 1;
    writing-mode: vertical-lr;
    direction: rtl;
    width: 32px;
    height: 140px;
    cursor: pointer;
    appearance: none;
    -webkit-appearance: none;
    border-radius: 6px;
    background:
  linear-gradient(to top, #fff var(--val, 50%), #ffffff20 var(--val, 50%)) center / 4px 100% no-repeat,
  var(--dark-bg);
}
.volume-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #fff !important;
}
.botPart:has(.volumeControl:hover) {
    z-index: 100001;
}
.playerControls.seeking {
    transition: none;
}
.playerControls.seeking .progress-bar-fill {
    transition: opacity 0.3s ease;
}
.playerControls.seeking .progress-bg {
    transition: none;
}
.playerControls :deep(.ripple) {
    position: absolute;
    border-radius: 50%;
    background: hsla(0, 0%, 100%, 0.15);
    transform: scale(0);
    animation: ripple-expand 0.6s ease-out forwards;
    pointer-events: none;
    z-index: 4;
}
@keyframes ripple-expand {
    to {
        transform: scale(1);
        opacity: 0;
    }
}

/* ── Fullscreen overlay ───────────────────────────────────────────────────── */

.fullscreen-overlay {
    position: fixed;
    inset: 0;
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
.fs-content {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    width: 100%;
    height: 100%;
    padding: 56px 40px 40px;
    contain: layout style;
}
.fs-main {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 24px;
    flex: 1;
    max-width: 420px;
    width: 100%;
}
.fs-cover-wrapper {
    position: relative;
    width: 320px;
    height: 320px;
    border-radius: 16px;
    overflow: hidden;
    box-shadow: 0 24px 80px rgba(0, 0, 0, 0.5);
    flex-shrink: 0;
    transition: .24s ease;

    &:hover {
        transform: scale(1.02);
    }
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
    max-width: 100%;
}
.fs-title {
    font-size: 22px;
    font-weight: 700;
    color: #fff;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    letter-spacing: -0.01em;
}
.fs-artist {
    font-size: 16px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.55);
}
.fs-album {
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.35);
    margin-top: 2px;
}
.fs-progress {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
}
.fs-progress-slider {
    width: 100%;
    height: 14px;
    appearance: none;
    -webkit-appearance: none;
    cursor: pointer;
    outline: none;
    background: transparent;
}
.fs-progress-slider::-webkit-slider-runnable-track {
    height: 6px;
    border-radius: 3px;
    background: linear-gradient(
        to right,
        hsla(0, 0%, 100%, 0.7) var(--fs-val, 0%),
        hsla(0, 0%, 100%, 0.12) var(--fs-val, 0%)
    );
}
.fs-progress-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    cursor: pointer;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
    margin-top: -4px;
}
.fs-progress-slider:disabled {
    opacity: 0.3;
    cursor: default;
}
.fs-times {
    display: flex;
    justify-content: space-between;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.4);
    user-select: none;
}
.fs-controls {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: center;
    gap: 8px;
}
.fs-btn {
    padding: 10px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.5);
    cursor: pointer;
    transition: color .24s;
}
.fs-btn:hover {
    color: hsla(0, 0%, 100%, 0.9);
}
.fs-btn:disabled {
    opacity: 0.25;
    cursor: not-allowed;
}
.fs-btn.active {
    color: #fff;
}
.fs-play-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
    border-radius: 50%;
    background: #fff;
    color: #0a0a0a;
    cursor: pointer;
    margin: 0 8px;
}
.fs-play-btn:disabled {
    opacity: 0.25;
    cursor: not-allowed;
}
.fs-extra {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 24px;
}
.fs-codec {
    font-size: 14px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.4);
    user-select: none;
    position: relative;

    &:hover .codecTooltip {
        visibility: visible;
        opacity: 1;
    }
}
.codecTooltip {
    visibility: hidden;
    opacity: 0;
    pointer-events: none;

    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    position: absolute;
    bottom: calc(100% + 16px);
    left: 50%;
    transform: translateX(-50%);
    background: var(--dark-bg);
    border: 1px solid hsla(0, 0%, 100%, 0.1);
    border-radius: 8px;
    padding: 6px 8px;
    white-space: nowrap;
    z-index: 100;
    backdrop-filter: blur(12px);
    transition: opacity 0.2s ease, visibility 0.2s ease;
}
.codecTooltip-title {
    width: 100%;
    font-size: 12px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.85);
}
.codecTooltip-details {
    font-size: 11px;
    font-weight: 500;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
}
.fs-volume-group {
    display: flex;
    align-items: center;
    gap: 8px;
    color: hsla(0, 0%, 100%, 0.5);
    user-select: none;
}
.fs-volume-icon {
    cursor: pointer;
    padding: 4px;
    transition: color .24s;
}
.fs-volume-icon:hover {
    color: hsla(0, 0%, 100%, 0.9);
}
.fs-volume-slider {
    width: 100px;
    height: 4px;
    appearance: none;
    -webkit-appearance: none;
    border-radius: 2px;
    cursor: pointer;
    outline: none;
    background: linear-gradient(
        to right,
        hsla(0, 0%, 100%, 0.7) var(--fs-vol, 50%),
        hsla(0, 0%, 100%, 0.12) var(--fs-vol, 50%)
    );
}
.fs-volume-slider::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: transparent;
}
.fs-volume-slider::-webkit-slider-thumb {
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
.fs-volume-label {
    font-size: 14px;
    font-weight: 600;
    min-width: 38px;
    color: hsla(0, 0%, 100%, 0.5);
}

/* ── Transition ──────────────────────────────────────────────────────────── */

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
