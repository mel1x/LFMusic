<script setup lang="ts">
import { ref, computed } from 'vue';
import trackCover from '../assets/trackCover.png';

import MoreIcon from './Icons/MoreIcon.vue';
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

import { useSettings } from '../composables/useSettings';
import { usePlayer } from '../composables/usePlayer';

const { settings } = useSettings();

const {
    currentTrack,
    volume,
    positionSecs,
    isSeeking,
    seekValue,
    durationSecs,
    isPlaying,
    isStopped,
    isLoading,
    shuffled,
    repeatMode,
    trackTitle,
    trackArtist,
    currentCoverArt,
    currentAccentColor,
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
} = usePlayer();

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

const codecDisplay = computed(() => {
    if (!currentTrack.value) return '';
    return currentTrack.value.format?.toUpperCase() || '';
});

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
</script>

<template>
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
            <div ref="playerControlsEl" class="playerControls" :class="{ seeking: isSeeking }" :style="currentAccentColor ? { '--accent-color': currentAccentColor } as any : { '--accent-color': '#1a1a1a' }">
                <div class="progress-bar-bg"></div>
                <div class="progress-bar-fill" :style="{ width: progressFillPercent + '%' }"></div>
                <div class="progress-bg" :style="{ width: progressFillPercent + '%' }"></div>
            </div>
            <div class="botPart" @click.self="onPlayerRipple">
                <div class="trackInfo">
                    <div class="coverWrapper">
                        <button class="fullscreenBtn"><FullscreenIcon /></button>
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
                            <button class="moreBtn"><MoreIcon /></button>
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
                        @click="toggleShuffle"
                    >
                        <ShuffleIcon />
                    </button>
                    <button
                        class="prevBtn"
                        @click="playPrev"
                    >
                        <PrevIcon />
                    </button>
                    <button
                        class="playBtn"
                        @click="togglePlayPause"
                        :disabled="isStopped || isLoading"
                        :style="currentAccentColor ? { '--accent-color': currentAccentColor } as any : {}"
                    >
                        <PauseIcon v-if="isPlaying" />
                        <PlayIcon v-else />
                    </button>
                    <button
                        class="nextBtn"
                        @click="playNext()"
                    >
                        <NextIcon />
                    </button>
                    <button
                        class="repeatBtn"
                        :class="{ active: repeatMode === 'track' }"
                        @click="toggleRepeat"
                    >
                        <RepeatOneIcon v-if="repeatMode === 'track'" />
                        <RepeatIcon v-else />
                    </button>
                </div>
                <div class="additionalControls">
                    <div class="queueList"><ListIcon /></div>
                    <div class="codecInfo" v-if="codecDisplay">{{ codecDisplay }}</div>
                    <div
                        class="volumeControl"
                        @click="toggleMute"
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
                                @click.stop
                                @input.stop="onVolumeInput"
                                orient="vertical"
                            />
                        </div>
                        <VolumeZeroIcon v-if="volume === 0" />
                        <VolumeMinIcon v-else-if="volume <= 0.4" />
                        <VolumeMaxIcon v-else />
                        <span>{{ volumePercent }}%</span>
                    </div>
                </div>
            </div>
        </div>
    </div>
</template>

<style scoped>
.player {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    height: 104px;
}
.asideWrapper {
    width: 260px;
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
    width: calc(100% - 260px);
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
    z-index: 99999;
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
    z-index: 1;
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
    z-index: 2;
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
    background: hsla(0, 0%, 0%, 1);
    border-radius: 50%;
    cursor: pointer;
    transition: background-color .24s, opacity .3s ease-in-out;
    z-index: 3;
    opacity: 0;

    &:hover {
        background-color: hsla(0, 0%, 15%, 0.9);
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

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.volumeControl {
    user-select: none;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;

    cursor: pointer;
    padding: 8px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.5);
    transition: .24s;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.volumeControl span {
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
    background: #0d0d0d;
    border: 2px solid #ffffff20;
    border-radius: 24px;
    padding: 16px 2px;

    z-index: 10;
    transition: opacity 0.2s ease, visibility 0.2s ease;
}
.volumeSliderPopup::after {
    content: '';
    position: absolute;
    top: -15px;
    bottom: -30px;
    left: -15px;
    right: -15px;
    z-index: -1;
}
.volumeControl:hover .volumeSliderPopup {
    visibility: visible;
    opacity: 1;
    pointer-events: auto;
}
.volume-slider {
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
  #0d0d0d;
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
    z-index: 100000;
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
    z-index: 3;
}
@keyframes ripple-expand {
    to {
        transform: scale(1);
        opacity: 0;
    }
}
</style>
