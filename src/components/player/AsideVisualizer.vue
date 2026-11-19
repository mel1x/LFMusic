<script setup lang="ts">
import { computed, watch, onBeforeUnmount } from 'vue';
import trackCover from '../../assets/trackCover.webp';

import TrackArtists from '../TrackArtists.vue';
import NextIcon from '../Icons/NextIcon.vue';
import { useSettings } from '../../composables/useSettings';
import { usePlayer } from '../../composables/usePlayer';
import { usePlayerUi } from '../../composables/usePlayerUi';
import { useI18n } from '../../composables/useI18n';
import { useSpectrum, SPECTRUM_BARS } from '../../composables/useSpectrum';

const { settings } = useSettings();
const { t } = useI18n();
const { playNext, loadCoversForTracks } = usePlayer();
const { nextTrack, nextTrackTitle } = usePlayerUi();

const COLUMNS = Array.from({ length: SPECTRUM_BARS }, (_, i) => i);
const SEGMENTS = 7;
const SEGMENT_INDICES = Array.from({ length: SEGMENTS }, (_, i) => i);
const litCounts = new Int8Array(SPECTRUM_BARS);

let barsBox: HTMLElement | null = null;
function setBarsBox(el: unknown) {
    barsBox = el as HTMLElement | null;
    litCounts.fill(0);
}

const spectrum = useSpectrum();

function drawBars(levels: Float32Array) {
    const cols = barsBox?.children;
    if (!cols) return;
    for (let i = 0; i < cols.length; i++) {
        const v = Math.min(1, levels[i] ?? 0);
        const lit = v < 0.04 ? 0 : Math.max(1, Math.round(v * SEGMENTS));
        if (lit === litCounts[i]) continue;
        litCounts[i] = lit;
        const segs = cols[i].children;
        for (let j = 0; j < segs.length; j++) {
            segs[j].classList.toggle('on', j < lit);
            segs[j].classList.toggle('peak', j === lit - 1);
        }
    }
}

const mode = computed(() => settings.value.show_player_extra_block ? settings.value.player_extra_block_mode : null);

watch(
    () => mode.value === 'visualizer',
    (active) => {
        if (active) spectrum.start(drawBars);
        else spectrum.stop();
    },
    { immediate: true },
);

onBeforeUnmount(() => {
    spectrum.stop();
});

watch([() => mode.value === 'next_track', nextTrack], ([show, track]) => {
    if (show && track && !track.cover_art) {
        loadCoversForTracks([track]);
    }
}, { immediate: true });

const nextKey = computed(() => nextTrack.value?.file_path ?? '');
</script>

<template>
    <div class="asideWrapper">
        <div v-if="mode" class="aside">
            <div
                v-if="mode === 'visualizer'"
                :ref="setBarsBox"
                class="asideMeter"
                aria-hidden="true"
            >
                <span v-for="c in COLUMNS" :key="c" class="asideColumn">
                    <span v-for="s in SEGMENT_INDICES" :key="s" class="asideSegment"></span>
                </span>
            </div>

            <button
                v-else
                type="button"
                class="asideNext"
                :disabled="!nextTrack"
                :aria-label="`${t('player.nextTrack')}: ${nextTrackTitle}`"
                @click="playNext()"
            >
                <Transition name="asideSwap" mode="out-in">
                    <div :key="nextKey" class="asideNextBody">
                        <span class="asideNextCover">
                            <img
                                v-if="nextTrack"
                                :src="nextTrack.cover_art || trackCover"
                                alt=""
                                draggable="false"
                            />
                            <span v-if="nextTrack" class="asideNextHover"><NextIcon width="20" height="20" /></span>
                        </span>
                        <span class="asideNextInfo">
                            <span class="asideNextLabel">{{ t('player.nextTrack') }}</span>
                            <span v-truncate:top class="asideNextTitle">{{ nextTrackTitle }}</span>
                            <span v-if="nextTrack" class="asideNextArtist">
                                <TrackArtists :artist="nextTrack.artist" tooltipPlacement="top" />
                            </span>
                        </span>
                    </div>
                </Transition>
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
    overflow: hidden;
}

.asideMeter {
    height: 100%;
    padding: 12px;
    display: grid;
    grid-template-columns: repeat(16, minmax(0, 1fr));
    gap: 4px;
}
.asideColumn {
    display: flex;
    flex-direction: column-reverse;
    gap: 3px;
    min-width: 0;
}
.asideSegment {
    flex: 1;
    border-radius: 2px;
    background-color: hsla(0, 0%, 100%, 0.06);
    transition: background-color .08s linear;
}
.asideSegment.on {
    background-color: hsla(0, 0%, 100%, 0.42);
}
.asideSegment.peak {
    background-color: hsla(0, 0%, 100%, 0.75);
}

@media (prefers-reduced-motion: reduce) {
    .asideSegment {
        transition: none;
    }
}

.asideNext {
    width: 100%;
    height: 100%;
    padding: 10px;
    border-radius: inherit;
    background-color: transparent;
    text-align: left;
    color: inherit;
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.asideNext:hover:not(:disabled) {
    background-color: hsla(0, 0%, 100%, 0.04);
}
.asideNext:disabled {
    cursor: default;
}
.asideNextBody {
    height: 100%;
    display: grid;
    grid-template-columns: 56px minmax(0, 1fr);
    align-items: center;
    gap: 12px;
    min-width: 0;
}
.asideNextCover {
    position: relative;
    width: 56px;
    height: 56px;
    border-radius: 8px;
    overflow: hidden;
    background-color: hsla(0, 0%, 100%, 0.06);
}
.asideNextCover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
}
.asideNextHover {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    background-color: hsla(0, 0%, 0%, 0.45);
    opacity: 0;
    transition: opacity var(--duration-hover) var(--ease-smooth);
}
.asideNext:hover:not(:disabled) .asideNextHover {
    opacity: 1;
}
.asideNextInfo {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    user-select: none;
}
.asideNextLabel {
    font-size: 11px;
    font-weight: 600;
    line-height: 1.1;
    color: hsla(var(--player-accent-h), min(var(--player-accent-s), 60%), 78%, 0.7);
}
.asideNextTitle {
    min-width: 0;
    color: hsla(0, 0%, 100%, 0.9);
    font-size: 13px;
    font-weight: 600;
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.asideNext:disabled .asideNextTitle {
    color: hsla(0, 0%, 100%, 0.4);
}
.asideNextArtist {
    min-width: 0;
    color: hsla(0, 0%, 100%, 0.5);
    font-size: 12px;
    font-weight: 500;
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.asideSwap-enter-active,
.asideSwap-leave-active {
    transition: opacity var(--duration-hover) var(--ease-smooth), transform var(--duration-hover) var(--ease-smooth);
}
.asideSwap-enter-from {
    opacity: 0;
    transform: translateY(6px);
}
.asideSwap-leave-to {
    opacity: 0;
    transform: translateY(-6px);
}

@media (prefers-reduced-motion: reduce) {
    .asideSwap-enter-active,
    .asideSwap-leave-active {
        transition: none;
    }
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .asideWrapper {
        display: none;
    }
}
</style>
