<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'

import MoreIcon from '../components/Icons/MoreIcon.vue';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import ShuffleIcon from '../components/Icons/ShuffleIcon.vue';
import TimeIcon from '../components/Icons/TimeIcon.vue';
import cover from '../assets/cover.webp';

import { invoke } from '@tauri-apps/api/core';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { createRipple } from '../composables/useRipple';
import { isPhoneViewport } from '../composables/usePlatform';
import { useVirtualScroll } from '../composables/useVirtualScroll';
import { tracksCountText } from '../utils/format';
import TrackContextMenu from '../components/TrackContextMenu.vue';
import CatalogContextMenu from '../components/CatalogContextMenu.vue';
import TrackRow from '../components/TrackRow.vue';
import CoverEditor from '../components/CoverEditor.vue';
import AnimatedCover from '../components/AnimatedCover.vue';
import MobileScrollHeader from '../components/MobileScrollHeader.vue';
import LoadingSpinner from '../components/LoadingSpinner.vue';
import { useCustomCovers } from '../composables/useCustomCovers';
import { useCoverGate } from '../composables/useCoverGate';
import { useCoverTint } from '../composables/useCoverTint';

const {
    catalogCount,
    currentTrack,
    isPlaying,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    setCatalogSource,
    moveCatalogTrackToTop,
    getTrackByPath,
    fetchTracksByPaths,
    loadCoversForTracks,
    clearCoverQueue,
    shuffled,
    toggleShuffle,
} = usePlayer();

const { getCustomCover } = useCustomCovers();
const catalogCustomCover = computed(() => getCustomCover('catalog', 'catalog'));

// The default cover is tinted with the accent, so only a custom cover gives the header its own tint.
const customTint = useCoverTint(() => catalogCustomCover.value)
const heroStyle = computed(() => customTint.value && catalogCustomCover.value
    ? { '--hero-color': `rgb(${customTint.value})` }
    : {})

const selectedGenre = ref<string | null>(null)

const genreBarEl = ref<HTMLElement | null>(null)
const canScrollLeft = ref(false)
const canScrollRight = ref(false)

const genreList = ref<{ name: string; count: number }[]>([])
async function loadGenres() {
    try {
        genreList.value = await invoke<{ name: string; count: number }[]>('library_list_genres')
    } catch {
        genreList.value = []
    }
}

function toggleGenre(genre: string) {
    selectedGenre.value = selectedGenre.value === genre ? null : genre
}

function updateGenreScrollState() {
    const el = genreBarEl.value
    if (!el) return
    canScrollLeft.value = el.scrollLeft > 2
    canScrollRight.value = el.scrollLeft + el.clientWidth < el.scrollWidth - 2
}

function onGenreWheel(e: WheelEvent) {
    const el = genreBarEl.value
    if (!el) return
    el.scrollBy({ left: (e.deltaY !== 0 ? e.deltaY : e.deltaX) * 2, behavior: 'smooth' })
}

function scrollGenre(dir: -1 | 1) {
    genreBarEl.value?.scrollBy({ left: dir * 400, behavior: 'smooth' })
}

function scrollToActiveGenre() {
    if (!selectedGenre.value || !genreBarEl.value) return
    const chip = genreBarEl.value.querySelector('.genreChip.active') as HTMLElement | null
    if (!chip) return
    const bar = genreBarEl.value
    const chipLeft = chip.offsetLeft
    const chipWidth = chip.offsetWidth
    const barWidth = bar.clientWidth
    bar.scrollTo({ left: chipLeft - barWidth / 2 + chipWidth / 2, behavior: 'smooth' })
}

import { menuAnchorBelow } from '../utils/menu';
const filteredPaths = ref<string[]>([])
let filterReqId = 0
async function refreshFilteredPaths() {
    const reqId = ++filterReqId
    const genre = selectedGenre.value || undefined
    try {
        const paths = await invoke<string[]>('library_get_catalog_paths', { genre })
        if (reqId === filterReqId) filteredPaths.value = paths
    } catch {
        if (reqId === filterReqId) filteredPaths.value = []
    }
}
watch(selectedGenre, refreshFilteredPaths)
watch(catalogCount, () => { void loadGenres(); void refreshFilteredPaths() })

const scrollHeaderRef = ref<HTMLElement | null>(null)
const scrollHeaderHeight = ref(0)
let headerResizeObserver: ResizeObserver | null = null

onMounted(() => {
    void loadGenres()

    if (scrollHeaderRef.value) {
        scrollHeaderHeight.value = scrollHeaderRef.value.offsetHeight
        headerResizeObserver = new ResizeObserver(() => {
            if (scrollHeaderRef.value) {
                scrollHeaderHeight.value = scrollHeaderRef.value.offsetHeight
            }
            updateGenreScrollState()
        })
        headerResizeObserver.observe(scrollHeaderRef.value)
    }
    updateGenreScrollState()

    const source = activePlaylistSource.value
    if (source && source.startsWith('catalog:genre:')) {
        selectedGenre.value = source.slice('catalog:genre:'.length)
        setTimeout(() => scrollToActiveGenre(), 100)
    }
    void refreshFilteredPaths()
})

onUnmounted(() => {
    headerResizeObserver?.disconnect()
})

const {
    containerEl: trackListEl,
    onScroll,
    visibleItems: visiblePaths,
    paddingTop: vsPaddingTop,
    paddingBottom: vsPaddingBottom,
    resetScroll,
} = useVirtualScroll(filteredPaths, { itemHeight: 64, buffer: 5, scrollOffset: scrollHeaderHeight })

function stubTrack(path: string): TrackMetadata {
    return {
        file_path: path, title: null, artist: null, album: null, genre: null,
        duration_secs: null, format: '', sample_rate: null, bit_depth: null,
        channels: null, bitrate: null, is_lossless: false, cover_art: null,
        accent_color: null, bpm: null, loudness: null, energy: null,
        danceability: null, spectral_centroid: null, key: null, audio_fingerprint: null,
    }
}

const visibleTracks = computed<TrackMetadata[]>(() =>
    visiblePaths.value.map(p => getTrackByPath(p) ?? stubTrack(p))
)

watch(selectedGenre, () => {
    resetScroll()
})

const rankOf = computed(() => new Map(filteredPaths.value.map((p, i) => [p, i + 1])))

watch(visiblePaths, (paths) => {
    if (paths.length) void fetchTracksByPaths(paths)
}, { immediate: true })

const gateTracks = computed<TrackMetadata[]>(() =>
    filteredPaths.value.slice(0, 20).map(p => getTrackByPath(p)).filter((t): t is TrackMetadata => !!t)
)
watch(() => filteredPaths.value.slice(0, 20), (paths) => {
    if (paths.length) void fetchTracksByPaths(paths)
}, { immediate: true })
const { ready: coversReady } = useCoverGate(() => gateTracks.value)

const isScrollingToTop = ref(false)

watch(visibleTracks, (tracks) => {
    if (!isScrollingToTop.value) {
        loadCoversForTracks(tracks)
    }
}, { immediate: true })

const isCatalogActive = computed(() => {
    const src = activePlaylistSource.value
    if (!src || !src.startsWith('catalog')) return false
    return src === catalogSourceId.value
})

function onPlayCatalog() {
    if (isCatalogActive.value && currentTrack.value) {
        togglePlayPause()
    } else {
        setCatalogSource(catalogSourceId.value, filteredPaths.value)
        if (filteredPaths.value.length > 0) {
            playTrack(filteredPaths.value[0])
        }
    }
}

const isShuffleActive = computed(() => isCatalogActive.value && shuffled.value)

function onShuffleCatalog() {
    const paths = filteredPaths.value
    if (paths.length === 0) return
    if (isCatalogActive.value) {
        toggleShuffle()
        return
    }
    if (!shuffled.value) toggleShuffle()
    setCatalogSource(catalogSourceId.value, paths)
    playTrack(paths[Math.floor(Math.random() * paths.length)], { manualListStart: true })
}

const scrollY = ref(0)
const showStickyHeader = computed(() => scrollY.value > 300)

function handleScroll(e: Event) {
    scrollY.value = (e.target as HTMLElement).scrollTop
    if (isScrollingToTop.value && scrollY.value <= 10) {
        isScrollingToTop.value = false
        loadCoversForTracks(visibleTracks.value)
    }
    onScroll(e)
}

function scrollToTop() {
    isScrollingToTop.value = true
    clearCoverQueue()
    trackListEl.value?.scrollTo({ top: 0, behavior: 'smooth' })
}

function onTrackClick(e: MouseEvent, track: TrackMetadata) {
    if (isPhoneViewport()) {
        onCoverClick(track)
        return
    }
    createRipple(e)
}

const catalogMenu = ref<{ x: number; y: number } | null>(null)
const trackMenu = ref<{ x: number; y: number; track: TrackMetadata } | null>(null)

function openCatalogMenu(e: MouseEvent) {
    e.stopPropagation()
    if (catalogMenu.value) {
        catalogMenu.value = null
        return
    }
    catalogMenu.value = { ...menuAnchorBelow(e) }
}

function openTrackMenu(e: MouseEvent, track: TrackMetadata) {
    e.stopPropagation()
    if (trackMenu.value?.track.file_path === track.file_path) {
        trackMenu.value = null
        return
    }
    const btn = (e.currentTarget as HTMLElement)
    const rect = btn.getBoundingClientRect()
    trackMenu.value = { x: rect.left - 6, y: rect.top + rect.height / 2, track }
}

async function moveTrackToTop(track: TrackMetadata) {
    await moveCatalogTrackToTop(track.file_path)
    await refreshFilteredPaths()
}

const catalogSourceId = computed(() => {
    let id = 'catalog'
    if (selectedGenre.value) id += `:genre:${selectedGenre.value}`
    return id
})

function onCoverClick(track: TrackMetadata) {
    if (currentTrack.value?.file_path === track.file_path) {
        togglePlayPause()
        return
    }
    setCatalogSource(catalogSourceId.value, filteredPaths.value)
    playTrack(track, { manualListStart: true })
}

function onCoverImgClick() {
    if (isPhoneViewport()) return
    window.dispatchEvent(new CustomEvent('lfmusic:show-fullscreen-cover', {
        detail: { src: catalogCustomCover.value || cover, tinted: !catalogCustomCover.value },
    }))
}


</script>

<template>
    <div class="pageRoot catalogPage mobileHeroPage" ref="trackListEl" :style="heroStyle" @scroll="handleScroll">
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader title="Мой каталог" :scrolled="scrollY > 16" @top="scrollToTop" />
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div class="stickyTitle">Мой каталог</div>
                    <button class="toTopBtn" @click="scrollToTop">Вверх</button>
                </div>
            </Transition>
        </div>
        <div ref="scrollHeaderRef" class="scrollHeader">
            <div class="pageHat">
                <div class="coverWrap cover-edit-host" :class="{ tinted: !catalogCustomCover }" @click="onCoverImgClick">
                    <AnimatedCover :src="catalogCustomCover || cover" class="cover" alt="cover" />
                    <CoverEditor kind="catalog" coverKey="catalog" size="large" />
                </div>
                <div class="dataZone">
                    <div class="info">
                        <div class="listType">Библиотека</div>
                        <div class="listNameRow">
                            <h2 v-truncate class="listName">Мой каталог</h2>
                        </div>
                        <div class="badges" v-if="catalogCount > 0">
                            <span class="badge">{{ tracksCountText(catalogCount) }}</span>
                        </div>
                    </div>
                    <div class="controls">
                        <button
                            class="playListBtn"
                            type="button"
                            :disabled="filteredPaths.length === 0"
                            :aria-label="isCatalogActive && isPlaying ? 'Пауза' : 'Слушать каталог'"
                            @click="onPlayCatalog"
                        >
                            <PauseIcon v-if="isCatalogActive && isPlaying" width="22" height="22" />
                            <PlayIcon v-else width="22" height="22" />
                        </button>
                        <button
                            class="iconBtn shuffleBtn"
                            type="button"
                            :class="{ on: isShuffleActive }"
                            :disabled="filteredPaths.length === 0"
                            v-tooltip="'Перемешать'"
                            @click="onShuffleCatalog"
                        >
                            <ShuffleIcon width="20" height="20" />
                        </button>
                        <button class="iconBtn moreOptionsBtn" type="button" :class="{ menuOpen: !!catalogMenu }" aria-label="Действия каталога" @mousedown="openCatalogMenu($event)"><MoreIcon width="22" height="22" /></button>
                    </div>
                </div>
            </div>
            <div class="genreBarWrap" v-if="genreList.length > 0" @wheel.prevent="onGenreWheel">
                <button v-if="canScrollLeft" class="genreArrow left" @click="scrollGenre(-1)">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"/></svg>
                </button>
                <div class="genreBar" ref="genreBarEl" @scroll="updateGenreScrollState">
                    <button
                        v-for="g in genreList"
                        :key="g.name"
                        class="genreChip"
                        :class="{ active: selectedGenre === g.name }"
                        @click="toggleGenre(g.name)"
                    >
                        {{ g.name }}
                        <div
                            v-if="selectedGenre === g.name"
                            class="genreChipClose"
                            @click.stop="selectedGenre = null"
                        >
                            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="4" stroke-linecap="round" stroke-linejoin="round">
                                <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                            </svg>
                        </div>
                    </button>
                </div>
                <button v-if="canScrollRight" class="genreArrow right" @click="scrollGenre(1)">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"/></svg>
                </button>
            </div>
            <div class="listHead">
                <span class="colRank">#</span>
                <span class="colTitle">Название</span>
                <span class="colTime"><TimeIcon width="13" height="13" /></span>
            </div>
        </div>
        <Transition name="gate-fade" mode="out-in">
        <LoadingSpinner v-if="filteredPaths.length > 0 && !coversReady" key="gate-spinner" />
        <div
            class="tracksList"
            v-else-if="filteredPaths.length > 0"
            key="gate-list"
        >
            <div class="vsPad" :style="{ height: vsPaddingTop + 'px' }"></div>
            <TrackRow
                v-for="track in visibleTracks"
                :key="track.file_path"
                large
                :rank="rankOf.get(track.file_path)"
                :track="track"
                :active="currentTrack?.file_path === track.file_path"
                :activePaused="currentTrack?.file_path === track.file_path && !isPlaying"
                :menuOpen="trackMenu?.track.file_path === track.file_path"
                @rowClick="onTrackClick($event, track)"
                @coverClick="onCoverClick(track)"
                @openMenu="openTrackMenu($event, track)"
            />
            <div class="vsPad" :style="{ height: vsPaddingBottom + 'px' }"></div>
        </div>
        <div class="emptyState" v-else-if="catalogCount === 0" key="gate-empty-lib">
            <div class="emptyText">Нет треков в каталоге</div>
            <div class="emptyDesc">Добавьте папки с треками в настройках.</div>
            <RouterLink to="/settings/library" class="emptyBtn">Библиотека</RouterLink>
        </div>
        </Transition>
        <CatalogContextMenu
            v-if="catalogMenu"
            :x="catalogMenu.x"
            :y="catalogMenu.y"
            centerX
            @close="catalogMenu = null"
        />
        <TrackContextMenu
            v-if="trackMenu"
            :key="trackMenu.track.file_path"
            :x="trackMenu.x"
            :y="trackMenu.y"
            :track="trackMenu.track"
            alignRight
            centerY
            canMoveToTop
            @close="trackMenu = null"
            @moveToTop="moveTrackToTop"
        />
    </div>
</template>

<style scoped>
.pageRoot {
    --hero-color: hsl(var(--accent-h), min(var(--accent-s), 55%), 52%);
    padding: 12px 32px 48px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: var(--light-bg);
    background-image: linear-gradient(
        to bottom,
        color-mix(in srgb, var(--hero-color) 42%, transparent) 0px,
        color-mix(in srgb, var(--hero-color) 14%, transparent) 240px,
        transparent 460px
    );
    background-repeat: no-repeat;
    background-attachment: local;
    overflow-y: scroll;
}
.pageRoot::-webkit-scrollbar { width: 6px; }
.pageRoot::-webkit-scrollbar-track { background-color: transparent; }
.pageRoot::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
}
.scrollHeader {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
}
.pageHat {
    display: flex;
    flex-direction: row;
    align-items: flex-end;
    gap: 32px;
    width: 100%;
    flex-shrink: 0;
    padding: 8px 0 0;
}
.pageHat .coverWrap {
    position: relative;
    width: 232px;
    height: 232px;
    flex-shrink: 0;
    border-radius: 16px;
    box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.35);
    cursor: pointer;
}
.pageHat .cover {
    user-select: none;
    width: 100%;
    height: 100%;
    border-radius: 16px;
    overflow: hidden;
    object-fit: cover;
    transition: filter .28s var(--ease-smooth);
}
.pageHat .coverWrap.tinted .cover {
    filter: grayscale(1);
}
.pageHat .coverWrap.tinted:has(.coverEditBtn:hover) .cover {
    filter: grayscale(1) brightness(0.55);
}
.pageHat .coverWrap.tinted::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: 16px;
    background-color: hsla(var(--accent-h), min(var(--accent-s), 45%), 50%, 1);
    mix-blend-mode: color;
    pointer-events: none;
    transition: background-color .28s var(--ease-smooth), inset var(--duration-hover) var(--ease-smooth);
}
.pageHat .dataZone {
    display: flex;
    flex-direction: column;
    gap: 20px;
    flex: 1;
    min-width: 0;
}
.pageHat .dataZone .info {
    display: flex;
    flex-direction: column;
    min-width: 0;
}
.pageHat .dataZone .info .listType {
    user-select: none;
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.7);
}
.pageHat .dataZone .info .listNameRow {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin-top: 2px;
    max-width: 100%;
}
.pageHat .dataZone .info .listName {
    cursor: default;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 48px;
    font-weight: 900;
    letter-spacing: -0.02em;
    line-height: 1.1;
    color: hsla(0, 0%, 100%, 0.96);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    overflow: hidden;
    overflow-wrap: anywhere;
}
.pageHat .dataZone .info .badges {
    display: flex;
    flex-wrap: wrap;
    margin-top: 4px;
    max-width: 100%;
}
.badge {
    user-select: none;
    font-size: 14px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.7);
    white-space: nowrap;
}
.badge + .badge::before {
    content: "\00B7";
    margin: 0 6px;
}
:global(body.lfmusic-phone .badge + .badge::before) {
    content: none;
}
.pageHat .dataZone .controls {
    display: flex;
    align-items: center;
    gap: 8px;
}
.pageHat .dataZone .controls .playListBtn {
    width: 56px;
    height: 56px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.96);
    color: var(--light-bg);
    cursor: pointer;
    user-select: none;
    transition: transform .2s cubic-bezier(0.34, 1.56, 0.64, 1), background-color var(--duration-hover) var(--ease-smooth);
}
.pageHat .dataZone .controls .playListBtn:hover:not(:disabled) {
    transform: scale(1.06);
    background-color: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
}
.pageHat .dataZone .controls .playListBtn:active:not(:disabled) {
    transform: scale(0.94);
    transition-duration: 80ms;
}
.pageHat .dataZone .controls .playListBtn:disabled {
    cursor: not-allowed;
    opacity: 0.3;
}
.iconBtn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: 999px;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.6);
    cursor: pointer;
    transition: color var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth), transform 80ms var(--ease-smooth);
}
.iconBtn:hover:not(:disabled),
.iconBtn.menuOpen {
    color: hsla(0, 0%, 100%, 1);
    background-color: hsla(0, 0%, 100%, 0.08);
}
.iconBtn:active:not(:disabled) {
    transform: scale(0.92);
}
.iconBtn:disabled {
    cursor: not-allowed;
    opacity: 0.3;
}
.shuffleBtn.on {
    color: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
}
.genreBarWrap {
    position: relative;
    margin-top: -6px;
    padding: 20px 0;
    flex-shrink: 0;
}
.genreArrow {
    max-height: 30px;
    height: 30px;
    max-width: 30px;
    min-width: 30px;
    position: absolute;
    top: 18px;
    bottom: 0;
    width: 28px;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.7);
    transition: var(--duration-hover) var(--ease-smooth);

    &:hover {
        color: hsla(0, 0%, 100%, 1);
    }
}
.genreArrow.left {
    left: 0;
    background-color: hsla(var(--accent-h), var(--accent-s), 17%, 1);
    border-radius: 9999px;
    padding: 8px;
    transition: var(--duration-hover) var(--ease-smooth);

    &:hover {
        background-color: hsla(var(--accent-h), var(--accent-s), 21%, 1);
    }
}
.genreArrow.right {
    right: 0;
    background-color: hsla(var(--accent-h), var(--accent-s), 17%, 1);
    border-radius: 9999px;
    padding: 8px 7px 8px 9px;
    transition: var(--duration-hover) var(--ease-smooth);

    &:hover {
        background-color: hsla(var(--accent-h), var(--accent-s), 21%, 1);
    }
}
.genreBar {
    display: flex;
    flex-direction: row;
    gap: 6px;
    overflow-x: auto;
    overflow-y: hidden;
    flex-shrink: 0;
    padding-bottom: 2px;
    scrollbar-width: none;
    scroll-behavior: smooth;
}
.genreBar::-webkit-scrollbar {
    display: none;
}
.genreChip {
    display: flex;
    flex-direction: row;
    align-items: center;
    cursor: pointer;
    user-select: none;
    white-space: nowrap;
    line-height: 1;
    padding: 6px 14px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.6);
    font-size: 12px;
    font-weight: 500;
    transition: var(--duration-hover) var(--ease-smooth);
    flex-shrink: 0;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.1);
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.genreChip.active {
    background-color: hsla(0, 0%, 100%, 0.9);
    color: hsla(0, 0%, 0%, 0.85);
    display: flex;
    align-items: center;
    gap: 4px;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.75);
        color: hsla(0, 0%, 0%, 0.85);
    }
}
.genreChipClose {
    margin-top: 1px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: hsla(0, 0%, 0%, 0.5);
    transition: var(--duration-hover) var(--ease-smooth);

    &:hover {
        color: hsla(0, 0%, 0%, 0.85);
    }
}
.listHead {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 36px;
    margin-top: 4px;
    margin-bottom: 4px;
    padding: 0 16px;
    font-size: 11px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.42);
    user-select: none;
}
.listHead .colRank {
    width: 22px;
    text-align: center;
    flex-shrink: 0;
}
.listHead .colTitle {
    flex: 1;
    min-width: 0;
    padding-left: 60px;
}
.listHead .colTime {
    display: flex;
    justify-content: flex-end;
    width: 44px;
    flex-shrink: 0;
}

:global(body.lfmusic-phone .listHead) {
    display: none;
}
.tracksList {
    margin-top: 0;
}
.vsPad {
    width: 100%;
}
.stickyHeaderAnchor {
    position: sticky;
    top: -12px;
    height: 0;
    z-index: 10;
    margin: 0 -32px;
}
.stickyHeader {
    user-select: none;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 32px;
    background-color: var(--light-bg);
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 20px;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.94);
}
.stickyTitle {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.sticky-header-enter-active,
.sticky-header-leave-active {
    transition: opacity var(--duration-hover) var(--ease-smooth), transform var(--duration-hover) var(--ease-smooth);
}
.sticky-header-enter-from,
.sticky-header-leave-to {
    opacity: 0;
    transform: translateY(-8px);
}
.toTopBtn {
    flex-shrink: 0;
    cursor: pointer;
    padding: 6px 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.07);
    color: hsla(0, 0%, 100%, 1);
    font-family: "Inter", sans-serif;
    font-size: 13px;
    font-weight: 500;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.toTopBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
}
.emptyState {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 48px 0;
}
.emptyText {
    user-select: none;
    font-size: 14px;
    color: hsla(0, 0%, 100%, 0.45);
}
.emptyState {
    flex-direction: column;
    gap: 12px;
}
.emptyDesc {
    user-select: none;
    text-align: center;
    max-width: 270px;
    font-size: 13px;
    color: hsla(0, 0%, 100%, 0.35);
}
.emptyBtn {
    user-select: none;
    cursor: pointer;
    padding: 8px 20px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.08);
    color: hsla(0, 0%, 100%, 1);
    font-size: 13px;
    font-weight: 500;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.emptyBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    /* Tint through filters only: mix-blend-mode breaks on the large masked layer in WebView. */
    body.lfmusic-phone .catalogPage .pageHat .coverWrap.tinted .cover {
        filter: grayscale(1) sepia(1) hue-rotate(calc((var(--accent-h) - 45) * 1deg)) saturate(2.8) brightness(0.74);
    }

    body.lfmusic-phone .catalogPage .pageHat .coverWrap.tinted::after {
        display: none;
    }

    body.lfmusic-phone .catalogPage .genreBarWrap {
        position: relative;
        z-index: 2;
        width: 100% !important;
    }
}
</style>
