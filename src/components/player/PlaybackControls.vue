<script setup lang="ts">
import { ref, computed } from 'vue';
import trackCover from '../../assets/trackCover.webp';
import { toast } from 'vue-sonner';

import MoreIcon from '../Icons/MoreIcon.vue';
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

import { usePlayer, type TrackMetadata } from '../../composables/usePlayer';
import { usePlayerUi } from '../../composables/usePlayerUi';
import { isWebKitRuntime } from '../../composables/usePlatform';

// WebKitGTK (Linux) can't render the vertical volume popup, so we render a
// separate horizontal one there instead (see template / .volumeSliderPopupLinux).
const isWebKit = isWebKitRuntime();

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
    currentCoverArt,
    codecDisplay,
    codecDetails,
    volumePercent,
    togglePlayPause,
    playNext,
    playPrev,
    toggleShuffle,
    toggleRepeat,
    onSeekStart,
    onSeekInput,
    onSeekEnd,
} = usePlayer();

const {
    openFullscreen,
    openQueueFullscreen,
    canMoveToTop,
    moveTrackToTop,
    toggleMute,
    onVolumeScroll,
    onVolumeInput,
} = usePlayerUi();

const progressSliderValue = computed(() => {
    return isSeeking.value ? seekValue.value : positionSecs.value;
});

const progressFillPercent = computed(() => {
    if (!durationSecs.value) return 0;
    return (progressSliderValue.value / durationSecs.value) * 100;
});

const mobilePlayerQuery = '(max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px)';

function isMobilePlayerLayout() {
    return typeof window !== 'undefined' && window.matchMedia(mobilePlayerQuery).matches;
}

const playerControlsEl = ref<HTMLElement | null>(null);

function onPlayerRipple(e: MouseEvent) {
    const el = playerControlsEl.value;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const ripple = document.createElement('span');
    ripple.className = 'ripple';
    const size = Math.max(rect.width, rect.height) * 2;
    ripple.style.width = ripple.style.height = size + 'px';
    ripple.style.left = (e.clientX - rect.left - size / 2) + 'px';
    ripple.style.top = (e.clientY - rect.top - size / 2) + 'px';
    el.appendChild(ripple);
    ripple.addEventListener('animationend', () => ripple.remove());
}

function onPlayerSurfaceClick(e: MouseEvent) {
    if (isMobilePlayerLayout()) {
        openFullscreen();
        return;
    }

    if (e.target === e.currentTarget) onPlayerRipple(e);
}

const playerTrackMenu = ref<{ x: number; y: number } | null>(null);

function openPlayerTrackMenu(e: MouseEvent) {
    if (!currentTrack.value) return;
    e.stopPropagation();
    if (playerTrackMenu.value) {
        playerTrackMenu.value = null;
        return;
    }
    const btn = (e.currentTarget as HTMLElement);
    const rect = btn.getBoundingClientRect();
    playerTrackMenu.value = { x: rect.left + rect.width / 2, y: rect.top - 6 };
}

function onMoveToTop(track: TrackMetadata) {
    moveTrackToTop(track);
}
</script>

<template>
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
        <div class="botPart" @click="onPlayerSurfaceClick">
            <div class="trackInfo">
                <div class="coverWrapper">
                    <button class="fullscreenBtn" @click.stop="openFullscreen"><FullscreenIcon /></button>
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
                        <div v-truncate:top class="trackName">{{ trackTitle }}</div>
                        <button class="moreBtn" @mousedown.stop="openPlayerTrackMenu($event)" @click.stop><MoreIcon /></button>
                    </div>
                    <div class="artistName"><TrackArtists :artist="currentTrack.artist" tooltipPlacement="top" /></div>
                </div>
                <div class="trackTitle" v-else>
                    <div class="topLine">
                        <div class="trackName noTrack">Нет трека</div>
                    </div>
                </div>
            </div>
            <div class="controls" @click.stop>
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
                    <PauseIcon v-if="isPlaying" />
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
            <div class="additionalControls" @click.stop>
                <div v-if="currentTrack" class="queueList" @click.stop="openQueueFullscreen"><ListIcon /></div>
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
                    <div class="volumeSliderPopup" v-if="!isWebKit" :style="{ '--val': volumePercent + '%' }">
                        <div class="volumeTrack" aria-hidden="true"></div>
                        <input
                            type="range"
                            class="volume-slider"
                            min="0"
                            max="1"
                            step="0.01"
                            :value="volume"
                            @input.stop="onVolumeInput"
                            orient="vertical"
                        />
                    </div>
                    <div class="volumeSliderPopupLinux" v-else>
                        <input
                            type="range"
                            class="volume-slider-linux"
                            min="0"
                            max="1"
                            step="0.01"
                            :value="volume"
                            :style="{ '--val': volumePercent + '%' }"
                            @input.stop="onVolumeInput"
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
        :canMoveToTop="canMoveToTop"
        @close="playerTrackMenu = null"
        @moveToTop="onMoveToTop"
    />
</template>

<style scoped>
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
    background-color: hsla(var(--player-accent-h), min(var(--player-accent-s), 60%), var(--player-accent-l), 1);
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
    transition: opacity 0.3s ease;
}
.progress-bg {
    position: absolute;
    top: 0;
    left: 0;
    height: 104px;
    width: 0%;
    z-index: 2;
    background-color: hsla(0, 0%, 100%, 0.11);
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
    flex: 1;
    min-width: 0;
    max-width: 300px;
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
    background: hsla(var(--player-accent-h), var(--player-accent-s), 20%, 1);
    border-radius: 50%;
    cursor: pointer;
    transition: background-color .24s, opacity .3s ease-in-out;
    z-index: 4;
    opacity: 0;

    &:hover {
        background-color: hsla(var(--player-accent-h), var(--player-accent-s), 35%, 0.9);
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
    transition: transform .24s ease-in-out;
    transform-origin: center;
    color: hsla(var(--player-accent-h), min(var(--player-accent-s), 60%), var(--player-accent-l), 1);
    transform: translateZ(0);
    will-change: transform;
    backface-visibility: hidden;
}
.playBtn:hover:not(:disabled) {
    transform: scale(1.055) translateZ(0);
}
.playBtn:active:not(:disabled) {
    transform: scale(1.0) translateZ(0);
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
    flex: 1;
    min-width: 0;
    max-width: 300px;
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
    background-color: hsla(var(--player-accent-h), var(--player-accent-s), var(--player-accent-l), 0.15);
    border-radius: 16px;
    padding: 8px 12px;
    white-space: nowrap;
    z-index: 100;
    backdrop-filter: blur(17px);
    box-shadow: 0 0px 16px rgba(0, 0, 0, 0.05);
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
    color: hsla(var(--player-accent-h), calc(var(--player-accent-s) * 0.4), 80%, 0.5);
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
    align-items: center;
    justify-content: center;
    position: absolute;
    bottom: calc(100% - 4px);
    left: 50%;
    transform: translateX(-50%);
    background-color: hsla(var(--player-accent-h), var(--player-accent-s), var(--player-accent-l), 0.15);
    border-radius: 18px;
    padding: 12px 8px;
    backdrop-filter: blur(17px);
    box-shadow: 0 0px 16px rgba(0, 0, 0, 0.05);

    z-index: 11;
    transition: opacity 0.2s ease, visibility 0.2s ease;
}
/* Invisible ~24px zone around the popup keeps it open while the mouse is in it
   (bridges to the volume icon below, buffers the sides/top). Popup is a
   descendant of .volumeControl, so hovering this zone keeps :hover active. */
.volumeSliderPopup::after {
    content: '';
    position: absolute;
    inset: -24px;
}
.volumeControl:hover .volumeSliderPopup {
    visibility: visible;
    opacity: 1;
    pointer-events: auto;
}
/* Visible track is a separate div behind the transparent input: needs a 5px
   rounded width the input's own background can't do. Height is shortened by
   thumb size (11px) so thumb center reaches exactly the track ends, and the
   fill boundary lines up with the thumb center. Fill driven by --val (set on
   the popup). */
.volumeTrack {
    pointer-events: none;
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: 5px;
    height: calc(140px - 11px);
    border-radius: 999px;
    background: linear-gradient(
        to top,
        #fff var(--val, 50%),
        hsla(0, 0%, 100%, 0.25) var(--val, 50%)
    );
    z-index: 0;
}
.volume-slider {
    position: relative;
    z-index: 1;
    appearance: none;
    -webkit-appearance: none;
    writing-mode: vertical-lr;
    direction: rtl;
    /* 24px input width = drag hit area; thumb centers within it and lines up
       with the centered .volumeTrack. Background is transparent. */
    width: 24px;
    height: 140px;
    cursor: pointer;
    background: transparent;
}
.volume-slider::-webkit-slider-runnable-track {
    -webkit-appearance: none;
    appearance: none;
    background: transparent;
    border: 0;
}
.volume-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: #fff !important;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.35);
}
/* Separate HORIZONTAL volume popup for Linux/WebKitGTK only (v-if="isWebKit").
   Own classes, no inheritance from the vertical .volume-slider base — copy of
   the working qf-volume-slider. No transform. */
.volumeSliderPopupLinux {
    visibility: hidden;
    opacity: 0;
    pointer-events: none;

    position: absolute;
    bottom: calc(100% + 6px);
    left: 50%;
    width: 148px;
    margin-left: -74px;

    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsla(var(--player-accent-h), var(--player-accent-s), var(--player-accent-l), 0.15);
    border-radius: 16px;
    padding: 10px 0;
    backdrop-filter: blur(17px);
    box-shadow: 0 0px 16px rgba(0, 0, 0, 0.05);

    z-index: 11;
    transition: opacity 0.2s ease, visibility 0.2s ease;
}
.volumeControl:hover .volumeSliderPopupLinux {
    visibility: visible;
    opacity: 1;
    pointer-events: auto;
}
.volume-slider-linux {
    -webkit-appearance: none;
    appearance: none;
    width: 120px;
    height: 6px;
    border-radius: 3px;
    cursor: pointer;
    background: linear-gradient(to right, #fff var(--val, 50%), #ffffff25 var(--val, 50%));
}
.volume-slider-linux::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: #fff;
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

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .playerControlsWrapper {
        width: 100%;
        height: var(--mobile-player-height);
        padding: 8px 14px;
    }

    .progress-slider {
        display: none;
    }

    .playerControlsWrapper .progress-bar-bg,
    .playerControlsWrapper .progress-bar-fill,
    .progress-bg {
        display: none;
    }

    .playerControls {
        height: 100%;
        border-radius: 16px;
        background-color: hsla(var(--player-accent-h), min(var(--player-accent-s), 50%), 14%, 0.98);
    }

    .progress-bg {
        height: 100%;
    }

    .botPart {
        top: 0;
        width: 100%;
        height: 100%;
        padding: 0 24px 0 20px;
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto auto;
        gap: 4px;
        align-items: center;
        cursor: pointer;
    }

    .trackInfo {
        flex: initial;
        width: 100%;
        min-width: 0;
        margin-left: 0;
        gap: 8px;
    }

    .coverWrapper,
    .trackCover {
        width: 56px;
        height: 56px;
        border-radius: 8px;
    }

    .coverWrapper {
        margin-top: 0;
    }

    .fullscreenBtn {
        display: none;
    }

    .trackTitle {
        min-width: 0;
        flex: 1 1 auto;
        margin-top: 0;
        overflow: hidden;
    }

    .topLine {
        width: 100%;
        max-width: none;
    }

    .topLine .trackName,
    .artistName {
        width: 100%;
        max-width: 100%;
        font-size: 13px;
        line-height: 1.2;
    }

    .topLine .moreBtn {
        display: none;
    }

    .controls {
        flex: 0 0 auto;
        margin-top: 0;
        gap: 0;
        order: 3;
    }

    .shuffleBtn,
    .prevBtn,
    .nextBtn,
    .repeatBtn {
        display: none;
    }

    .shuffleBtn,
    .prevBtn,
    .nextBtn,
    .repeatBtn,
    .queueList {
        width: 38px;
        height: 38px;
        padding: 8px;
        margin: 0;
    }

    .playBtn {
        width: 38px;
        height: 38px;
        margin: 0;
        background: transparent;
        color: #fff;
    }

    .additionalControls {
        flex: 0 0 auto;
        width: auto;
        max-width: none;
        margin: 0;
        gap: 0;
        order: 2;
    }

    .codecInfo,
    .volumeControl {
        display: none;
    }
}

@media (max-width: 390px), (pointer: coarse) and (max-width: 390px) {
    .topLine {
        max-width: none;
    }
}
</style>
