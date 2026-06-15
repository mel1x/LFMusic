<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import trackCover from '../../assets/trackCover.webp';

import TrackArtists from '../TrackArtists.vue';
import { useSettings } from '../../composables/useSettings';
import { usePlayer } from '../../composables/usePlayer';
import { usePlayerUi } from '../../composables/usePlayerUi';
import { useI18n } from '../../composables/useI18n';

const { settings } = useSettings();
const { t } = useI18n();
const {
    currentTrack,
    currentCoverArt,
    isPlaying,
    trackTitle,
    playNext,
    loadCoversForTracks,
} = usePlayer();
const { nextTrack, nextTrackTitle } = usePlayerUi();

// ── Реальный визуализатор спектра (FFT из Rust-движка) ─────────────────
// Бэкенд считает БПФ по реально воспроизводимому звуку и отдаёт N полос (0..1).
// Опрашиваем его на кадрах rAF (~30 fps по данным) и сглаживаем переход к
// целевым значениям на каждом кадре (~60 fps по рендеру): быстрый «удар»,
// плавный спад — так визуализатор выглядит живым, без рывков.
const SPECTRUM_BARS = 16;
const SPECTRUM_INDICES = Array.from({ length: SPECTRUM_BARS }, (_, i) => i);
const spectrumLevels = ref<number[]>(new Array(SPECTRUM_BARS).fill(0));

const visualizerActive = computed(
    () => settings.value.show_player_extra_block
        && settings.value.player_extra_block_mode === 'visualizer',
);

let spectrumRaf: number | null = null;
let spectrumTargets = new Array(SPECTRUM_BARS).fill(0);
let spectrumLastFetch = 0;
let spectrumFetching = false;

async function fetchSpectrum() {
    if (spectrumFetching) return;
    spectrumFetching = true;
    try {
        const bars = await invoke<number[]>('player_get_spectrum', { bars: SPECTRUM_BARS });
        if (Array.isArray(bars) && bars.length === SPECTRUM_BARS) {
            spectrumTargets = bars;
        }
    } catch {
        // Бэкенд недоступен (например, чистый Vite-режим) — оставляем нули.
    } finally {
        spectrumFetching = false;
    }
}

function spectrumFrame(now: number) {
    spectrumRaf = null;
    const playing = isPlaying.value;

    if (playing && now - spectrumLastFetch >= 33) {
        spectrumLastFetch = now;
        void fetchSpectrum();
    }
    if (!playing) {
        spectrumTargets = spectrumTargets.map(() => 0);
    }

    const cur = spectrumLevels.value.slice();
    let moving = false;
    for (let i = 0; i < SPECTRUM_BARS; i++) {
        const target = spectrumTargets[i] ?? 0;
        const c = cur[i];
        // Атака быстрее спада — пики «выстреливают», но не дёргаются вниз.
        const k = target > c ? 0.5 : 0.16;
        let next = c + (target - c) * k;
        if (target === 0 && next < 0.001) next = 0;
        cur[i] = next;
        if (next > 0.004 || target > 0.004) moving = true;
    }
    spectrumLevels.value = cur;

    if (playing || moving) {
        spectrumRaf = requestAnimationFrame(spectrumFrame);
    }
}

function startSpectrum() {
    if (spectrumRaf == null) {
        spectrumLastFetch = 0;
        spectrumRaf = requestAnimationFrame(spectrumFrame);
    }
}

function stopSpectrum() {
    if (spectrumRaf != null) {
        cancelAnimationFrame(spectrumRaf);
        spectrumRaf = null;
    }
}

// Включаем/выключаем снятие спектра в Rust-движке: при выключенном
// визуализаторе бэкенд не тратит ничего на отвод звука и БПФ.
watch(
    visualizerActive,
    (active) => {
        void invoke('player_set_spectrum_enabled', { enabled: active }).catch(() => {});
    },
    { immediate: true },
);

watch(
    [isPlaying, visualizerActive],
    ([playing, active]) => {
        if (active && playing) {
            startSpectrum();
        } else if (!active) {
            stopSpectrum();
            spectrumLevels.value = new Array(SPECTRUM_BARS).fill(0);
        } else {
            // Пауза/стоп при активном визуализаторе: даём полосам плавно осесть.
            startSpectrum();
        }
    },
    { immediate: true },
);

onMounted(() => {
    if (visualizerActive.value && isPlaying.value) startSpectrum();
});

onBeforeUnmount(() => {
    stopSpectrum();
    void invoke('player_set_spectrum_enabled', { enabled: false }).catch(() => {});
});

// Прогружаем обложку следующего трека для режима «следующий трек».
const shouldLoadAsideNextCover = computed(() =>
    settings.value.show_player_extra_block && settings.value.player_extra_block_mode === 'next_track'
);

watch([shouldLoadAsideNextCover, nextTrack], ([show, track]) => {
    if (show && track && !track.cover_art) {
        loadCoversForTracks([track]);
    }
}, { immediate: true });
</script>

<template>
    <div class="asideWrapper">
        <div v-if="settings.show_player_extra_block" class="aside">
            <div
                v-if="settings.player_extra_block_mode === 'visualizer'"
                class="asideVisualizer"
                :class="{ playing: isPlaying, idle: !currentTrack }"
                :style="{
                    backgroundImage: currentCoverArt
                        ? `linear-gradient(90deg, hsla(var(--player-accent-h), min(var(--player-accent-s), 45%), 14%, 0.96) 0%, hsla(var(--player-accent-h), min(var(--player-accent-s), 45%), 16%, 0.86) 54%, hsla(var(--player-accent-h), min(var(--player-accent-s), 45%), 18%, 0.72) 100%), url(${currentCoverArt})`
                        : undefined,
                }"
            >
                <div class="asideMiniTrack">
                    <img
                        :src="currentCoverArt || trackCover"
                        alt=""
                        class="asideMiniCover"
                        draggable="false"
                    />
                    <div class="asideMiniInfo">
                        <span class="asideVisualLabel">{{ t('player.now') }}</span>
                        <span v-truncate:top class="asideVisualTitle">{{ trackTitle }}</span>
                        <span class="asideVisualArtist">
                            <TrackArtists :artist="currentTrack?.artist" tooltipPlacement="top" />
                        </span>
                    </div>
                </div>
                <div class="asideVisualBottom">
                    <div class="asideBars" aria-hidden="true">
                        <span
                            v-for="i in SPECTRUM_INDICES"
                            :key="i"
                            class="asideBar"
                            :style="{ '--bar-scale': Math.max(0.04, spectrumLevels[i]) }"
                        ></span>
                    </div>
                    <!-- <div class="asideProgressTrack">
                        <div class="asideProgressFill" :style="{ width: progressFillPercent + '%' }"></div>
                    </div> -->
                </div>
            </div>
            <button
                v-else
                type="button"
                class="asideNextTrack"
                :disabled="!nextTrack"
                :style="{
                    backgroundImage: nextTrack?.cover_art
                        ? `linear-gradient(90deg, hsla(var(--player-accent-h), min(var(--player-accent-s), 45%), 14%, 0.96) 0%, hsla(var(--player-accent-h), min(var(--player-accent-s), 45%), 15%, 0.84) 55%, hsla(var(--player-accent-h), min(var(--player-accent-s), 45%), 18%, 0.7) 100%), url(${nextTrack.cover_art})`
                        : undefined,
                }"
                @click="playNext()"
            >
                <img
                    :src="nextTrack?.cover_art || trackCover"
                    alt=""
                    class="asideNextCover"
                    draggable="false"
                />
                <div class="asideNextInfo">
                    <span class="asideNextLabel">{{ t('player.nextTrack') }}</span>
                    <span v-truncate:top class="asideNextTitle">{{ nextTrackTitle }}</span>
                    <span class="asideNextArtist">
                        <TrackArtists :artist="nextTrack?.artist" tooltipPlacement="top" />
                    </span>
                </div>
            </button>
        </div>
    </div>
</template>

<style scoped>
.asideWrapper {
    width: 240px;
    height: 100%;
    padding: 12px;
}
.aside {
    width: 100%;
    height: 100%;
    border-radius: 14px;
    background-color: var(--player-light-bg);
    border: 1px solid #ffffff04;
    overflow: hidden;
}
.asideVisualizer,
.asideNextTrack {
    width: 100%;
    height: 100%;
    border-radius: 13px;
    background-color: var(--player-light-bg);
    background-size: cover;
    background-position: center;
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.06);
}
.asideVisualizer {
    display: grid;
    grid-template-rows: 44px 1fr;
    gap: 6px;
    padding: 9px;
}
.asideMiniTrack {
    display: grid;
    grid-template-columns: 44px minmax(0, 1fr);
    align-items: center;
    gap: 8px;
    min-width: 0;
}
.asideMiniCover {
    width: 44px;
    height: 44px;
    border-radius: 7px;
    object-fit: cover;
    background-color: #202020;
}
.asideMiniInfo,
.asideNextInfo {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    min-width: 0;
}
.asideVisualLabel,
.asideNextLabel {
    flex-shrink: 0;
    user-select: none;
    font-size: 11px;
    font-weight: 600;
    line-height: 1.1;
    color: hsla(var(--player-accent-h), calc(var(--player-accent-s) * 0.35), 82%, 0.52);
}
.asideVisualTitle,
.asideNextTitle {
    user-select: none;
    min-width: 0;
    color: hsla(0, 0%, 100%, 0.88);
    font-size: 13px;
    font-weight: 600;
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.asideVisualArtist,
.asideNextArtist {
    min-width: 0;
    color: hsla(0, 0%, 100%, 0.42);
    font-size: 12px;
    font-weight: 500;
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.asideVisualBottom {
    display: grid;
    grid-template-rows: 1fr;
    gap: 5px;
    min-height: 0;
}
.asideBars {
    display: grid;
    grid-template-columns: repeat(16, minmax(2px, 1fr));
    align-items: end;
    gap: 3px;
    min-height: 0;
}
.asideBar {
    height: 100%;
    min-height: 4px;
    border-radius: 3px;
    background-color: hsla(0, 0%, 100%, 0.62);
    /* Высоту (--bar-scale) задаёт JS на каждом кадре rAF по реальному БПФ. */
    transform: scaleY(var(--bar-scale, 0.04));
    transform-origin: bottom;
    opacity: 0.56;
    will-change: transform;
}
.asideVisualizer.idle .asideBar {
    opacity: 0.18;
    transform: scaleY(0.18);
}
.asideProgressTrack {
    height: 3px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.08);
    overflow: hidden;
}
.asideProgressFill {
    height: 100%;
    width: 0%;
    border-radius: inherit;
    background-color: hsla(0, 0%, 100%, 0.56);
}
.asideNextTrack {
    display: grid;
    grid-template-columns: 60px minmax(0, 1fr);
    align-items: center;
    gap: 10px;
    padding: 10px;
    text-align: left;
    color: hsla(0, 0%, 100%, 0.82);
    cursor: pointer;
    transition: background-color .18s ease, color .18s ease, opacity .18s ease, box-shadow .18s ease;
}
.asideNextTrack:hover:not(:disabled) {
    color: hsla(0, 0%, 100%, 0.95);
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.09), inset 0 0 0 999px hsla(0, 0%, 100%, 0.035);
}
.asideNextTrack:disabled {
    cursor: default;
    opacity: 0.5;
}
.asideNextCover {
    width: 60px;
    height: 60px;
    border-radius: 7px;
    object-fit: cover;
    background-color: #202020;
}
.asideNextIcon {
    color: hsla(0, 0%, 100%, 0.42);
    transition: color .18s ease, transform .18s ease;
}
.asideNextTrack:hover:not(:disabled) .asideNextIcon {
    color: hsla(0, 0%, 100%, 0.72);
    transform: translateX(1px);
}
@media (prefers-reduced-motion: reduce) {
    .asideProgressFill {
        transition: none;
    }
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .asideWrapper {
        display: none;
    }
}
</style>
