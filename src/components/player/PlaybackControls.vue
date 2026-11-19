<script setup lang="ts">
import { ref, computed, watch, onUnmounted } from 'vue';
import trackCover from '../../assets/trackCover.webp';
import { toast } from 'vue-sonner';

import MoreIcon from '../Icons/MoreIcon.vue';
import TrackContextMenu from '../TrackContextMenu.vue';
import SoundMenu from './SoundMenu.vue';
import TrackArtists from '../TrackArtists.vue';
import ShuffleIcon from '../Icons/ShuffleIcon.vue';
import PrevIcon from '../Icons/PrevIcon.vue';
import PlayIcon from '../Icons/PlayIcon.vue';
import PauseIcon from '../Icons/PauseIcon.vue';
import NextIcon from '../Icons/NextIcon.vue';
import RepeatIcon from '../Icons/RepeatIcon.vue';
import RepeatOneIcon from '../Icons/RepeatOneIcon.vue';
import ListIcon from '../Icons/ListIcon.vue';
import LyricsIcon from '../Icons/LyricsIcon.vue';
import VolumeMaxIcon from '../Icons/VolumeMaxIcon.vue';
import VolumeMinIcon from '../Icons/VolumeMinIcon.vue';
import VolumeZeroIcon from '../Icons/VolumeZeroIcon.vue';
import FullscreenIcon from '../Icons/FullscreenIcon.vue';

import { usePlayer, type TrackMetadata } from '../../composables/usePlayer';
import { usePlayerUi } from '../../composables/usePlayerUi';
import { useLyrics } from '../../composables/useLyrics';
import { accentPreview, hexToHSL } from '../../composables/useAccentTheme';
import { vLiveWidth, vLiveValue } from '../../directives/live';
import { currentUiScale } from '../../composables/useUiScale';
import { useSoundFx } from '../../composables/useSoundFx';
import { useSleepTimer } from '../../composables/useSleepTimer';
import { useEqualizer } from '../../composables/useEqualizer';

const soundMenuAnchor = ref<DOMRect | null>(null);
const { fx: soundFx, load: loadSoundFx } = useSoundFx();
const { mode: sleepMode } = useSleepTimer();
const { eqEnabled: soundEqEnabled, load: loadSoundEq } = useEqualizer();
void loadSoundFx();
void loadSoundEq();
/** Any sound effect, speed change or sleep timer is on: the button lights up. */
const soundActive = computed(() => {
    const f = soundFx.value;
    return soundEqEnabled.value || f.space.enabled || f.crusher.enabled || f.vinyl.enabled
        || f.speed !== 1 || sleepMode.value !== 'off';
});
function toggleSoundMenu(e: MouseEvent) {
    soundMenuAnchor.value = soundMenuAnchor.value
        ? null
        : (e.currentTarget as HTMLElement).getBoundingClientRect();
}

const codecTipOpen = ref(false);
const codecTipStyle = ref<Record<string, string>>({});
let codecTipTimer: number | null = null;

function onCodecEnter(e: MouseEvent) {
    const el = e.currentTarget as HTMLElement;
    if (codecTipTimer !== null) window.clearTimeout(codecTipTimer);
    codecTipTimer = window.setTimeout(() => {
        codecTipTimer = null;
        const r = el.getBoundingClientRect();
        const scale = currentUiScale();
        codecTipStyle.value = {
            left: `${(r.left + r.width / 2) / scale}px`,
            top: `${(r.top - 16) / scale}px`,
        };
        codecTipOpen.value = true;
    }, 450);
}

function onCodecLeave() {
    if (codecTipTimer !== null) {
        window.clearTimeout(codecTipTimer);
        codecTipTimer = null;
    }
    codecTipOpen.value = false;
}

onUnmounted(onCodecLeave);

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
    setVolume,
    togglePlayPause,
    playNext,
    playPrev,
    toggleShuffle,
    toggleRepeat,
    onSeekStart,
    onSeekInput,
    onSeekEnd,
    playbackOrder,
    currentTrackIndex,
    getTrackByPath,
    loadCoversForTracks,
} = usePlayer();

const {
    openFullscreen,
    openQueueFullscreen,
    openLyricsFullscreen,
    canMoveToTop,
    moveTrackToTop,
    toggleMute,
    onVolumeScroll,
} = usePlayerUi();

// Plain-element volume slider: a vertical range input renders differently in WebView2 and
// WebKitGTK.
const VOLUME_SLIDER_H = 140;
const VOLUME_THUMB = 11;
const volumeDragging = ref(false);

function setVolumeFromPointer(e: PointerEvent) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const inset = (rect.height * VOLUME_THUMB) / VOLUME_SLIDER_H / 2;
    const ratio = (rect.bottom - inset - e.clientY) / (rect.height - inset * 2);
    setVolume(Math.round(Math.min(1, Math.max(0, ratio)) * 100) / 100);
}

function onVolumePointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    volumeDragging.value = true;
    setVolumeFromPointer(e);
}

function onVolumePointerMove(e: PointerEvent) {
    if (volumeDragging.value) setVolumeFromPointer(e);
}

function onVolumePointerUp() {
    volumeDragging.value = false;
}

function onVolumeKeydown(e: KeyboardEvent) {
    const step = e.key === 'ArrowUp' || e.key === 'ArrowRight' ? 0.05
        : e.key === 'ArrowDown' || e.key === 'ArrowLeft' ? -0.05
        : 0;
    if (!step) return;
    e.preventDefault();
    setVolume(Math.round(Math.min(1, Math.max(0, volume.value + step)) * 100) / 100);
}

const { hasLyrics } = useLyrics();

const progressSliderValue = computed(() => {
    return isSeeking.value ? seekValue.value : positionSecs.value;
});

const progressFillPercent = computed(() => {
    if (!durationSecs.value) return 0;
    return (progressSliderValue.value / durationSecs.value) * 100;
});
// Getters for v-live-*: the template must not read the per-frame position.
const liveSliderValue = () => progressSliderValue.value;
const liveFillPercent = () => progressFillPercent.value;

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

const wrapperEl = ref<HTMLElement | null>(null);
// Offset goes to a CSS variable: a reactive value would re-render the player on every pointermove.
let swipeX = 0;
function setSwipeX(x: number) {
    swipeX = x;
    wrapperEl.value?.style.setProperty('--swipe-x', `${x}px`);
}
const swipeAnimating = ref(false);
const PEEK_GAP = 6;
const BAR_INSET = 28;

function peekTrack(offset: 1 | -1): TrackMetadata | null {
    const order = playbackOrder.value;
    const idx = currentTrackIndex.value;
    if (!currentTrack.value || order.length < 2 || idx < 0) return null;
    const path = order[(idx + offset + order.length) % order.length];
    return getTrackByPath(path) ?? null;
}
const prevPeek = computed(() => peekTrack(-1));
const nextPeek = computed(() => peekTrack(1));

/** Card background from the track's own accent color, so each card keeps its color while sliding. */
function cardBackground(track: TrackMetadata | null): string {
    if (!track?.accent_color) return 'hsl(0, 0%, 7%)';
    const { h, s } = hexToHSL(track.accent_color);
    return `hsl(${h}, ${Math.min(s, 40)}%, 8%)`;
}
const currentCardBg = computed(() => cardBackground(currentTrack.value));

watch([prevPeek, nextPeek], (tracks) => {
    const missing = tracks.filter((t): t is TrackMetadata => !!t && !t.cover_art);
    if (missing.length && isMobilePlayerLayout()) loadCoversForTracks(missing);
});

let swipeStart: { x: number; y: number; id: number } | null = null;
let swipeDragging = false;
let suppressClickUntil = 0;

function onSwipeDown(e: PointerEvent) {
    if (!isMobilePlayerLayout() || swipeAnimating.value || !currentTrack.value) return;
    swipeStart = { x: e.clientX, y: e.clientY, id: e.pointerId };
    swipeDragging = false;
}

function onSwipeMove(e: PointerEvent) {
    if (!swipeStart || e.pointerId !== swipeStart.id) return;
    const dx = e.clientX - swipeStart.x;
    const dy = e.clientY - swipeStart.y;
    if (!swipeDragging) {
        if (Math.abs(dx) < 8 && Math.abs(dy) < 8) return;
        if (Math.abs(dy) > Math.abs(dx)) {
            swipeStart = null;
            return;
        }
        swipeDragging = true;
        wrapperEl.value?.setPointerCapture(e.pointerId);
    }
    const hasTarget = dx < 0 ? !!nextPeek.value : !!prevPeek.value;
    setSwipeX(hasTarget ? dx : dx * 0.25);
}

function wait(ms: number) {
    return new Promise<void>(resolve => setTimeout(resolve, ms));
}

async function onSwipeUp(e: PointerEvent) {
    if (!swipeStart || e.pointerId !== swipeStart.id) return;
    swipeStart = null;
    if (!swipeDragging) return;
    swipeDragging = false;
    suppressClickUntil = performance.now() + 400;

    const width = wrapperEl.value?.clientWidth ?? 0;
    const dx = swipeX;
    const dir = dx < 0 ? 1 : -1;
    const target = dir > 0 ? nextPeek.value : prevPeek.value;
    swipeAnimating.value = true;
    if (!target || Math.abs(dx) < width * 0.22) {
        setSwipeX(0);
        await wait(220);
        swipeAnimating.value = false;
        return;
    }
    setSwipeX(-dir * (width - BAR_INSET * 2 + PEEK_GAP));
    accentPreview.value = target.accent_color ?? '#1a1a1a';
    await wait(220);
    const action = (dir > 0 ? playNext() : playPrev({ skipRestart: true }))
        .finally(() => { accentPreview.value = null; });
    await Promise.race([action, wait(800)]);
    swipeAnimating.value = false;
    setSwipeX(0);
}

function onSwipeCancel() {
    if (!swipeStart) return;
    swipeStart = null;
    swipeDragging = false;
    swipeAnimating.value = true;
    setSwipeX(0);
    setTimeout(() => { swipeAnimating.value = false; }, 220);
}

function onWrapperClickCapture(e: MouseEvent) {
    if (performance.now() > suppressClickUntil) return;
    e.stopPropagation();
    e.preventDefault();
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
    <div
        ref="wrapperEl"
        class="playerControlsWrapper"
        :class="{ swipeAnimating }"
        @pointerdown="onSwipeDown"
        @pointermove="onSwipeMove"
        @pointerup="onSwipeUp"
        @pointercancel="onSwipeCancel"
        @click.capture="onWrapperClickCapture"
    >
        <div v-if="prevPeek" class="peekCard prev" :style="{ backgroundColor: cardBackground(prevPeek) }" aria-hidden="true">
            <img :src="prevPeek.cover_art || trackCover" class="peekCover" draggable="false" />
            <div class="peekText">
                <div class="peekName">{{ prevPeek.title || prevPeek.file_path.split(/[\\/]/).pop() }}</div>
                <div class="peekArtist">{{ prevPeek.artist }}</div>
            </div>
        </div>
        <div v-if="nextPeek" class="peekCard next" :style="{ backgroundColor: cardBackground(nextPeek) }" aria-hidden="true">
            <img :src="nextPeek.cover_art || trackCover" class="peekCover" draggable="false" />
            <div class="peekText">
                <div class="peekName">{{ nextPeek.title || nextPeek.file_path.split(/[\\/]/).pop() }}</div>
                <div class="peekArtist">{{ nextPeek.artist }}</div>
            </div>
        </div>
        <input
            type="range"
            class="progress-slider"
            min="0"
            :max="durationSecs || 0"
            step="0.1"
            v-live-value="liveSliderValue"
            @mousedown="onSeekStart"
            @input="onSeekInput"
            @mouseup="onSeekEnd"
            @touchstart="onSeekStart"
            @touchend="onSeekEnd"
            :disabled="!currentTrack"
        />
        <div ref="playerControlsEl" class="playerControls" :class="{ seeking: isSeeking }" :style="{ '--mobile-card-bg': currentCardBg }">
            <div class="progress-bar-bg"></div>
            <div class="progress-bar-fill" v-live-width="liveFillPercent"></div>
            <div class="progress-bg" v-live-width="liveFillPercent"></div>
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
                    @click="playPrev()"
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
                <button
                    v-if="currentTrack"
                    class="lyricsBtn"
                    :disabled="!hasLyrics"
                    @click.stop="openLyricsFullscreen"
                >
                    <LyricsIcon />
                </button>
                <div v-if="currentTrack" class="queueList lfi-host" @click.stop="openQueueFullscreen"><ListIcon /></div>
                <button
                    class="soundBtn"
                    :class="{ active: soundActive, open: !!soundMenuAnchor }"
                    aria-label="Звук"
                    @mousedown.stop
                    @click.stop="toggleSoundMenu"
                >
                    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M2 6c.6.5 1.2 1 2.5 1C7 7 7 5 9.5 5c2.6 0 2.4 2 5 2 2.5 0 2.5-2 5-2 1.3 0 1.9.5 2.5 1" />
                        <path d="M2 12c.6.5 1.2 1 2.5 1 2.5 0 2.5-2 5-2 2.6 0 2.4 2 5 2 2.5 0 2.5-2 5-2 1.3 0 1.9.5 2.5 1" />
                        <path d="M2 18c.6.5 1.2 1 2.5 1 2.5 0 2.5-2 5-2 2.6 0 2.4 2 5 2 2.5 0 2.5-2 5-2 1.3 0 1.9.5 2.5 1" />
                    </svg>
                </button>
                <div
                    class="codecInfo"
                    v-if="currentTrack && codecDisplay"
                    @mouseenter="onCodecEnter"
                    @mouseleave="onCodecLeave"
                >
                    {{ codecDisplay }}
                    <Teleport to="body">
                        <div class="codecTooltip" :class="{ visible: codecTipOpen }" :style="codecTipStyle">
                            <span class="codecTooltip-title">Качество трека</span>
                            <span class="codecTooltip-details">{{ codecDetails }}</span>
                        </div>
                    </Teleport>
                </div>
                <div
                    class="volumeControl"
                    :class="{ dragging: volumeDragging }"
                    @wheel.prevent="onVolumeScroll"
                >
                    <div class="volumeSliderPopup" :style="{ '--vol': volume }">
                        <div
                            class="volumeSlider"
                            role="slider"
                            tabindex="0"
                            aria-orientation="vertical"
                            aria-valuemin="0"
                            aria-valuemax="100"
                            :aria-valuenow="volumePercent"
                            @pointerdown="onVolumePointerDown"
                            @pointermove="onVolumePointerMove"
                            @pointerup="onVolumePointerUp"
                            @pointercancel="onVolumePointerUp"
                            @keydown="onVolumeKeydown"
                        >
                            <div class="volumeTrack" aria-hidden="true"></div>
                            <div class="volumeThumb" aria-hidden="true"></div>
                        </div>
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
    <SoundMenu v-if="soundMenuAnchor" :anchor="soundMenuAnchor" @close="soundMenuAnchor = null" />
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
    transition: opacity 0.3s var(--ease-smooth);
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
    transition: var(--duration-hover) var(--ease-smooth);
}
.progress-bar-bg {
    height: 4px;
    width: 100%;
    background-color: hsla(0, 0%, 100%, 0.12);
    opacity: 0;
    transition: opacity 0.3s var(--ease-smooth);
}
.progress-bar-fill {
    position: absolute;
    top: 0;
    left: 0;
    height: 4px;
    width: 0px;
    background-color: hsla(0, 0%, 100%, 0.65);
    opacity: 0;
    transition: opacity 0.3s var(--ease-smooth);
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
    transition: var(--duration-hover);
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
    transition: background-color var(--duration-hover), opacity .3s var(--ease-smooth-in-out);
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
    transition: var(--duration-hover);
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
    transition: var(--duration-hover);

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
    transition: var(--duration-hover);

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
    transition: var(--duration-hover);

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
    transition: var(--duration-hover);

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
    transition: transform .2s var(--icon-ease);
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
    transition: transform 80ms var(--ease-smooth);
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
    transition: var(--duration-hover);

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
    transition: var(--duration-hover);

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
.lyricsBtn {
    display: flex;
    margin-top: -1px;
    margin-right: -8px;
    padding: 8px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.5);
    cursor: pointer;
    transition: var(--duration-hover);

    &:hover:not(:disabled) {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.lyricsBtn:disabled {
    opacity: 0.25;
    cursor: default;
}
.soundBtn {
    display: flex;
    margin: -1px -8px 0;
    padding: 8px;
    border: none;
    border-radius: 10px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.5);
    cursor: pointer;
    transition: color var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth);

    &:hover,
    &.open {
        color: hsla(0, 0%, 100%, 0.8);
    }
    &.open {
        background-color: hsla(0, 0%, 100%, 0.08);
    }
    &.active {
        color: hsl(var(--player-accent-h), max(var(--player-accent-s), 55%), 75%);
    }
}
.queueList {
    margin-top: -1px;
    cursor: pointer;
    padding: 8px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.5);
    transition: var(--duration-hover);

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
    transition: var(--duration-hover);
    position: relative;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
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
    position: fixed;
    transform: translate(-50%, -100%);
    background-color: hsla(var(--player-accent-h), var(--player-accent-s), var(--player-accent-l), 0.15);
    border-radius: 16px;
    padding: 8px 12px;
    white-space: nowrap;
    z-index: 1000010;
    backdrop-filter: blur(17px);
    box-shadow: 0 0px 16px rgba(0, 0, 0, 0.05);
    transition: opacity var(--duration-hover) var(--ease-smooth), visibility var(--duration-hover) var(--ease-smooth);
}
.codecTooltip.visible {
    visibility: visible;
    opacity: 1;
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
    transition: var(--duration-hover);

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
    transition: opacity var(--duration-hover) var(--ease-smooth), visibility var(--duration-hover) var(--ease-smooth);
}
.volumeSliderPopup::after {
    content: '';
    position: absolute;
    inset: -24px;
}
.volumeControl:hover .volumeSliderPopup,
.volumeControl.dragging .volumeSliderPopup {
    visibility: visible;
    opacity: 1;
    pointer-events: auto;
}
.volumeSlider {
    position: relative;
    z-index: 1;
    width: 24px;
    height: 140px;
    cursor: pointer;
    touch-action: none;
    outline: none;
}
.volumeTrack {
    pointer-events: none;
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: 5px;
    height: calc(100% - 11px);
    border-radius: 999px;
    background: linear-gradient(
        to top,
        #fff calc(var(--vol, 0.5) * 100%),
        hsla(0, 0%, 100%, 0.25) calc(var(--vol, 0.5) * 100%)
    );
}
.volumeThumb {
    pointer-events: none;
    position: absolute;
    left: 50%;
    bottom: calc(var(--vol, 0.5) * (100% - 11px));
    width: 11px;
    height: 11px;
    margin-left: -5.5px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.35);
}
:global(.player:has(.volumeControl:hover)),
:global(.player:has(.volumeControl.dragging)) {
    z-index: 60;
}
.botPart:has(.volumeControl.dragging),
.botPart:has(.volumeControl:hover) {
    z-index: 100001;
}
.playerControls.seeking {
    transition: none;
}
.playerControls.seeking .progress-bar-fill {
    transition: opacity 0.3s var(--ease-smooth);
}
.playerControls.seeking .progress-bg {
    transition: none;
}
.playerControls :deep(.ripple) {
    position: absolute;
    border-radius: 50%;
    background: hsla(0, 0%, 100%, 0.15);
    transform: scale(0);
    animation: ripple-expand 0.6s var(--ease-smooth) forwards;
    pointer-events: none;
    z-index: 4;
}
@keyframes ripple-expand {
    to {
        transform: scale(1);
        opacity: 0;
    }
}

.peekCard {
    display: none;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .playerControlsWrapper {
        width: 100%;
        height: var(--mobile-player-height);
        padding: 8px 28px;
        overflow: hidden;
        touch-action: pan-y;
    }

    .playerControls,
    .botPart,
    .peekCard {
        transform: translateX(var(--swipe-x, 0px));
        transition: none;
    }

    .swipeAnimating .playerControls,
    .swipeAnimating .botPart,
    .swipeAnimating .peekCard {
        transition: transform var(--duration-hover) var(--ease-smooth);
    }

    .peekCard {
        position: absolute;
        top: 8px;
        bottom: 8px;
        width: calc(100% - 56px);
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 0 9px;
        border-radius: 16px;
        opacity: 0.7;
        pointer-events: none;
    }

    .peekCard.prev {
        left: calc(78px - 100%);
    }

    .peekCard.next {
        left: calc(100% - 22px);
    }

    .peekCover {
        width: 52px;
        height: 52px;
        flex-shrink: 0;
        border-radius: 8px;
        object-fit: cover;
        user-select: none;
    }

    .peekText {
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 1px;
    }

    .peekName,
    .peekArtist {
        font-size: 13px;
        line-height: 1.2;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .peekName {
        color: #fff;
        font-weight: 600;
    }

    .peekArtist {
        color: hsla(0, 0%, 100%, 0.5);
    }

    .progress-slider {
        display: none;
    }

    .playerControlsWrapper .progress-bar-bg,
    .playerControlsWrapper .progress-bar-fill {
        display: none;
    }

    .playerControls {
        height: 100%;
        border-radius: 16px;
        background-color: var(--mobile-card-bg, hsl(0, 0%, 7%));
    }

    .progress-bg {
        height: 100%;
        pointer-events: none;
    }

    .botPart {
        top: 0;
        width: 100%;
        height: 100%;
        padding: 0 38px 0 37px;
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
        width: 52px;
        height: 52px;
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
    .lyricsBtn,
    .soundBtn,
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
