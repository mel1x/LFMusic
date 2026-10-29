<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'

import MoreIcon from '../components/Icons/MoreIcon.vue';
import HeartIcon from '../components/Icons/HeartIcon.vue';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import defaultCover from '../assets/trackCover.webp';
import cover from '../assets/cover.webp';
import SearchIcon from '../components/Icons/SearchIcon.vue';

import { invoke } from '@tauri-apps/api/core';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { createRipple } from '../composables/useRipple';
import { isPhoneViewport } from '../composables/usePlatform';
import { useVirtualScroll } from '../composables/useVirtualScroll';
import { tracksCountText } from '../utils/format';
import EmptyIcon from '../components/Icons/EmptyIcon.vue';
import TrackContextMenu from '../components/TrackContextMenu.vue';
import CatalogContextMenu from '../components/CatalogContextMenu.vue';
import TrackArtists from '../components/TrackArtists.vue';
import CoverEditor from '../components/CoverEditor.vue';
import AnimatedCover from '../components/AnimatedCover.vue';
import MobileScrollHeader from '../components/MobileScrollHeader.vue';
import LoadingSpinner from '../components/LoadingSpinner.vue';
import { useCustomCovers } from '../composables/useCustomCovers';
import { useCoverGate } from '../composables/useCoverGate';
import { usePlaylists } from '../composables/usePlaylists';

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
    formatTime,
    loadCoversForTracks,
    clearCoverQueue,
} = usePlayer();

const { getCustomCover } = useCustomCovers();
const catalogCustomCover = computed(() => getCustomCover('catalog', 'catalog'));

const { isFavorite, toggleFavorite, ensureFavoritesLoaded } = usePlaylists();
void ensureFavoritesLoaded();
function onToggleFav(track: TrackMetadata) {
    void toggleFavorite(track.file_path);
}

// Genre filter

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

import { useTypeToSearch } from '../composables/useTypeToSearch'
import { menuAnchorBelow } from '../utils/menu';
const searchInput = ref<HTMLInputElement | null>(null)
useTypeToSearch(searchInput)
const rawSearch = ref('')
const debouncedSearch = ref('')
let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null

function onSearchInput(e: Event) {
    const value = (e.target as HTMLInputElement).value
    rawSearch.value = value
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    searchDebounceTimer = setTimeout(() => {
        debouncedSearch.value = value
    }, 250)
}

const focusSearch = () => {
    searchInput.value?.focus()
}

function clearSearch() {
    rawSearch.value = ''
    debouncedSearch.value = ''
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    searchInput.value?.focus()
}

// Filtered paths (genre + search) come from the backend; virtual scroll iterates
// paths, metadata for the visible window is materialized on demand via cache.
const filteredPaths = ref<string[]>([])
let filterReqId = 0
async function refreshFilteredPaths() {
    const reqId = ++filterReqId
    const genre = selectedGenre.value || undefined
    const search = debouncedSearch.value.trim() || undefined
    try {
        const paths = await invoke<string[]>('library_get_catalog_paths', { genre, search })
        if (reqId === filterReqId) filteredPaths.value = paths
    } catch {
        if (reqId === filterReqId) filteredPaths.value = []
    }
}
watch([debouncedSearch, selectedGenre], refreshFilteredPaths)
// Refresh when the catalog composition changes (folders added/removed).
watch(catalogCount, () => { void loadGenres(); void refreshFilteredPaths() })

// Header height (for virtual scroll offset)

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

    // Restore selected genre from the active playback source
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

// Virtual scroll

const {
    containerEl: trackListEl,
    onScroll,
    visibleItems: visiblePaths,
    paddingTop: vsPaddingTop,
    paddingBottom: vsPaddingBottom,
    resetScroll,
} = useVirtualScroll(filteredPaths, { itemHeight: 58, buffer: 5, scrollOffset: scrollHeaderHeight })

// Placeholder for a not-yet-loaded path — keeps 1 row per path so the virtual
// list height doesn't shift during fast scrolling; replaced once cached.
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

watch([debouncedSearch, selectedGenre], () => {
    resetScroll()
})

watch(visiblePaths, (paths) => {
    if (paths.length) void fetchTracksByPaths(paths)
}, { immediate: true })

// Hold back first paint until the first 20 covers have resolved.
const gateTracks = computed<TrackMetadata[]>(() =>
    filteredPaths.value.slice(0, 20).map(p => getTrackByPath(p)).filter((t): t is TrackMetadata => !!t)
)
watch(() => filteredPaths.value.slice(0, 20), (paths) => {
    if (paths.length) void fetchTracksByPaths(paths)
}, { immediate: true })
const { ready: coversReady } = useCoverGate(() => gateTracks.value)

// Load covers for visible tracks (skip during scroll-to-top)
const isScrollingToTop = ref(false)

watch(visibleTracks, (tracks) => {
    if (!isScrollingToTop.value) {
        loadCoversForTracks(tracks)
    }
}, { immediate: true })

// Catalog play button

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

// Sticky header

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

// Track click

function onTrackClick(e: MouseEvent, track: TrackMetadata) {
    if (isPhoneViewport()) {
        onCoverClick(track)
        return
    }
    createRipple(e)
}

// Context menus

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
    // Refresh page order (sort_order changed on the backend).
    await refreshFilteredPaths()
}

const catalogSourceId = computed(() => {
    let id = 'catalog'
    if (selectedGenre.value) id += `:genre:${selectedGenre.value}`
    if (debouncedSearch.value.trim()) id += `:search`
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
    <div class="pageRoot catalogPage" ref="trackListEl" @scroll="handleScroll">
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader title="Мой каталог" :scrolled="scrollY > 16" @top="scrollToTop" />
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div>Мой каталог</div>
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
                        <h2 v-truncate class="listName">Мой каталог</h2>
                        <div class="badges" v-if="catalogCount > 0">
                            <div class="badge" v-tooltip="'Количество треков в каталоге'">{{ tracksCountText(catalogCount) }}</div>
                        </div>
                    </div>
                    <div class="controls">
                        <button class="playListBtn" type="button" aria-label="Слушать каталог" @click="onPlayCatalog">
                            <PauseIcon v-if="isCatalogActive && isPlaying" width="14" height="14" />
                            <PlayIcon v-else width="14" height="14" style="margin-left:0;" />
                            <span class="playListText">Слушать</span>
                        </button>
                        <button class="moreOptionsBtn" type="button" aria-label="Действия каталога" @mousedown="openCatalogMenu($event)"><MoreIcon width="22" height="22" /></button>
                    </div>
                </div>
            </div>
            <div class="searchZone" v-if="!isPhoneViewport()">
                <SearchIcon class="searchIcon" @click="focusSearch" />
                <input
                    ref="searchInput"
                    type="text"
                    class="search"
                    name="search"
                    id="search"
                    placeholder="Поиск трека"
                    autocomplete="off"
                    :value="rawSearch"
                    @input="onSearchInput"
                />
                <div class="searchClear" v-if="rawSearch" @click="clearSearch">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                    </svg>
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
        </div>
        <Transition name="gate-fade" mode="out-in">
        <LoadingSpinner v-if="filteredPaths.length > 0 && !coversReady" key="gate-spinner" />
        <div
            class="tracksList"
            v-else-if="filteredPaths.length > 0"
            key="gate-list"
        >
            <div class="vsPad" :style="{ height: vsPaddingTop + 'px' }"></div>
            <div
                v-for="track in visibleTracks"
                :key="track.file_path"
                class="trackElem"
                :class="{
                    active: currentTrack?.file_path === track.file_path,
                    activePaused: currentTrack?.file_path === track.file_path && !isPlaying,
                }"


                @click="onTrackClick($event, track)"
            >
                <div class="hoverIcon" @click.stop="onCoverClick(track)">
                    <PauseIcon
                        v-if="currentTrack?.file_path === track.file_path && isPlaying"
                        width="10" height="10"
                        style="margin-left:0.5px;"
                    />
                    <PlayIcon
                        v-else
                        width="10" height="10"
                        style="margin-left:1px;padding-top:0;"
                    />
                </div>
                <div
                    v-if="currentTrack?.file_path === track.file_path"
                    class="playingAnim"
                ></div>
                <div class="trackCover" @click.stop="onCoverClick(track)">
                    <img
                        :src="track.cover_art || defaultCover"
                        alt=""
                        draggable="false"
                        class="trackCoverImg"
                    />
                </div>
                <div class="trackInfo">
                    <div v-truncate class="trackName">{{ track.title || track.file_path.split(/[\\/]/).pop()?.replace(/\.[^/.]+$/, '') }}</div>
                    <div class="trackArtist"><TrackArtists :artist="track.artist" /></div>
                </div>
                <button
                    class="trackFavBtn"
                    :class="{ liked: isFavorite(track.file_path) }"
                    @click.stop="onToggleFav(track)"
                    @mousedown.stop
                >
                    <HeartIcon width="15" height="15" />
                </button>
                <div class="trackTime" :class="{ hidden: trackMenu?.track.file_path === track.file_path }">{{ track.duration_secs ? formatTime(track.duration_secs) : '' }}</div>
                <div class="hoverOptionsBtn" :class="{ menuOpen: trackMenu?.track.file_path === track.file_path }" @mousedown.stop="openTrackMenu($event, track)" @click.stop><MoreIcon width="26" height="26" /></div>
            </div>
            <div class="vsPad" :style="{ height: vsPaddingBottom + 'px' }"></div>
        </div>
        <div class="emptyState" v-else-if="debouncedSearch && filteredPaths.length === 0" key="gate-empty-search">
            <div class="emptyIcon"><SearchIcon width="16" height="16" /></div>
            <div class="emptyText">Ничего не найдено</div>
            <div class="emptyDesc">Попробуйте изменить критерии поиска или добавить папки с треками в настройках.</div>
        </div>
        <div class="emptyState" v-else-if="catalogCount === 0" key="gate-empty-lib">
            <div class="emptyIcon"><EmptyIcon width="16" height="16" /></div>
            <div class="emptyText">Нет треков в каталоге</div>
            <div class="emptyDesc">Добавьте папки с треками в настройках, чтобы они появились здесь.</div>
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
    padding: 20px 28px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: var(--light-bg);

    background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
    background-repeat: no-repeat;
    background-attachment: local;

    transition: .24s ease;
    
    overflow-y: scroll;
}
.pageRoot::-webkit-scrollbar { width: 6px; }
.pageRoot::-webkit-scrollbar-track { background-color: var(--light-bg); }
.pageRoot::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
}
.pageHat {
    display: flex;
    flex-direction: row;
    gap: 30px;
    width: 100%;
    flex-shrink: 0;
}
.pageHat .coverWrap {
    position: relative;
    width: 200px;
    height: 200px;
    flex-shrink: 0;
    border-radius: 18px;
    cursor: pointer;
}
.pageHat .cover {
    user-select: none;
    width: 200px;
    height: 200px;
    background-color: transparent;
    border-radius: 18px;
    transition: width .24s ease, height .24s ease, margin .24s ease, filter .28s ease;
    overflow: hidden;
    flex-shrink: 0;
    object-fit: cover;
}
.pageHat .coverWrap:hover .cover {
    width: 204px;
    height: 204px;
    margin: -2px;
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
    border-radius: 18px;
    background-color: hsla(var(--accent-h), min(var(--accent-s), 45%), 50%, 1);
    mix-blend-mode: color;
    pointer-events: none;
    transition: background-color .28s ease, inset .24s ease;
}
.pageHat .coverWrap.tinted:hover::after {
    inset: -2px;
}
.pageHat .dataZone {
    display: flex;
    flex-direction: column;
    gap: 35px;
    width: calc(100% - 230px);
}
.pageHat .dataZone .info {
    margin-top: 15px;
    width: 100%;
    display: flex;
    flex-direction: column;
}
.pageHat .dataZone .info .listType {
    user-select: none;
    margin-left: 1px;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.4);
    width: fit-content;
    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 0.6);
    }
}
.pageHat .dataZone .info .listName {
    cursor: default;
    margin-top: 2px;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 42px;
    font-weight: 900;
    color: hsla(0, 0%, 100%, 0.9);
}
.pageHat .dataZone .info .badges {
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 14px;
    max-width: 100%;
}
.badge {
    user-select: none;
    padding: 4px 10px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    font-size: 12px;
    font-weight: 500;
    transition: .24s ease;
    max-width: 100%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.badge:hover {
    background-color: hsla(0, 0%, 100%, 0.1);
    color: hsla(0, 0%, 100%, 0.9);
}
.pageHat .dataZone .controls {
    margin-top: 0px;
    display: flex;
    flex-direction: row;
    gap: 12px;
}
.pageHat .dataZone .controls .playListBtn {
    user-select: none;
    cursor: pointer;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 6px;
    padding: 11px 16px 11px 14px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 1);
    color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
    font-size: 12px;
    font-weight: 600;
    transition: .24s ease;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.8);
    }
}
.playListBtn svg {
    width: 16px;
}
.pageHat .dataZone .controls .moreOptionsBtn {
    cursor: pointer;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 8px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.07);
    color: hsla(0, 0%, 100%, 1);
    transition: .24s ease;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.12);
    }
}
.searchZone {
    position: relative;
    margin-top: 24px;
    width: 100%;
    flex-shrink: 0;
}
.searchIcon {
    cursor: text;
    position: absolute;
    top: 50%;
    left: 12px;
    transform: translateY(-50%);
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.4);
    transition: .24s ease;

    &:hover {
        color: hsla(var(--accent-h), var(--accent-s), 80%, 0.8);
    }
}
.searchZone:hover .searchIcon {
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.8);
}
.searchClear {
    position: absolute;
    height: max-content;
    width: max-content;
    top: 50%;
    right: 9px;
    transform: translateY(-50%);
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.3);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 6px;
    border-radius: 50%;
    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 0.6);
    }
}
.search {
    width: 100%;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.04);
    padding: 10px 36px 11px 40px;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.9);
}
.search::placeholder {
    user-select: none;
    color: hsla(0, 0%, 100%, 0.3);
    font-size: 13px;
    font-weight: 600;
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
    transition: .2s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 1);
    }
}
.genreArrow.left {
    left: 0;
    background-color: hsla(var(--accent-h), var(--accent-s), 17%, 1);
    border-radius: 9999px;
    padding: 8px;
    transition: .24s ease;

    &:hover {
        background-color: hsla(var(--accent-h), var(--accent-s), 21%, 1);
    }
}
.genreArrow.right {
    right: 0;
    background-color: hsla(var(--accent-h), var(--accent-s), 17%, 1);
    border-radius: 9999px;
    padding: 8px 7px 8px 9px;
    transition: .24s ease;

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
    transition: .24s ease;
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
    transition: .2s ease;

    &:hover {
        color: hsla(0, 0%, 0%, 0.85);
    }
}
.tracksList {
    margin-top: 4px;
}
.vsPad {
    width: 100%;
}
.trackElem {
    position: relative;
    cursor: pointer;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    padding: 8px 16px 8px 8px;
    border-radius: 9px;
    background-color: hsla(0, 0%, 100%, 0);
    transition: .31s ease-in-out;
    height: 58px;
    box-sizing: border-box;
    overflow: hidden;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.03);
    }
}
.trackElem :deep(.ripple) {
    position: absolute;
    border-radius: 50%;
    background: hsla(0, 0%, 100%, 0.08);
    transform: scale(0);
    animation: ripple-expand 0.5s ease-out forwards;
    pointer-events: none;
}
@keyframes ripple-expand {
    to {
        transform: scale(1);
        opacity: 0;
    }
}
.trackCover {
    width: 42px;
    height: 42px;
    background-color: #202020;
    border-radius: 5px;
    transition: .31s ease;
    overflow: hidden;
    flex-shrink: 0;
}
.trackCoverImg {
    user-select: none;
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
}
.trackElem:hover .trackCover,
.trackElem.activePaused .trackCover {
    opacity: 0.5;
}
.trackElem.active .trackCover {
    opacity: 0.45;
}
.hoverIcon {
    position: absolute;
    top: 50%;
    left: 18.5px;
    transform: translateY(-50%);
    opacity: 0;
    padding: 5px;
    border-radius: 9999px;
    color: #242424;
    background-color: hsla(0, 0%, 100%, 1.0);
    transition: .31s ease;
    z-index: 3;
}
.trackElem:hover .hoverIcon,
.trackElem.activePaused .hoverIcon {
    opacity: 1;
}
.hoverOptionsBtn {
    position: absolute;
    top: 50%;
    right: 12px;
    padding: 6px;
    transform: translateY(-50%);
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.45);
    transition: .31s ease;

    opacity: 0;

    &:hover {
        color: hsla(0, 0%, 100%, 0.7);
    }
}
.trackElem:hover .hoverOptionsBtn,
.hoverOptionsBtn.menuOpen {
    opacity: 1;
}
.trackFavBtn {
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
.trackFavBtn:hover {
    color: hsla(0, 0%, 100%, 0.85);
}
.trackFavBtn.liked {
    color: hsla(0, 0%, 100%, 0.95);
}
.trackFavBtn.liked :deep(svg path) {
    fill: currentColor;
}
.trackTime.hidden {
    opacity: 0 !important;
}
.trackInfo {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
}
.trackName {
    user-select: none;
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.trackElem.active .trackName {
    color: hsla(0, 0%, 100%, 1);
}
.trackArtist {
    user-select: none;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.4);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 0.6);
    }
}
.trackTime {
    margin-left: auto;
    user-select: none;
    font-size: 14px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.45);
    transition: .24s ease;
    flex-shrink: 0;

    opacity: 1;
}
.trackElem:hover .trackTime {
    opacity: 0;
}
.playingAnim {
    position: absolute;
    left: 21px;
    top: 21px;
    width: 16px;
    height: 16px;
    background: #fff;
    border-radius: 50%;
    transform-origin: center;
    animation: pulse 0.8s infinite ease;
    z-index: 2;
}

@keyframes pulse {
    0%, 100% { transform: scale(0.6); }
    50% { transform: scale(1); }
}


.stickyHeaderAnchor {
    position: sticky;
    top: -20px;
    height: 0;
    z-index: 10;
    margin: 0 -28px;
}
.stickyHeader {
    user-select: none;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    padding: 18px 28px;
    background-color: hsla(var(--accent-h), var(--accent-s), 14%, 0.3);
    backdrop-filter: blur(12px);
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 22px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.9);
}
.sticky-header-enter-active,
.sticky-header-leave-active {
    transition: opacity 0.2s ease, transform 0.2s ease;
}
.sticky-header-enter-from,
.sticky-header-leave-to {
    opacity: 0;
    transform: translateY(-8px);
}
.toTopBtn {
    cursor: pointer;
    padding: 6px 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.07);
    color: hsla(0, 0%, 100%, 1);
    font-size: 14px;
    font-weight: 500;
    transition: .24s ease;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.12);
    }
}
.emptyState {
    flex: 1;
    display: flex;
    gap: 10px;
    flex-direction: column;
    align-items: center;
    justify-content: center;
}
.emptyIcon {
    padding: 6px;
    border-radius: 5px;
    background-color: hsla(0, 0%, 100%, 0.09);
    color: #ffffff;
}
.emptyText {
    user-select: none;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.7);

    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 0.8);
    }
}
.emptyDesc {
    user-select: none;
    text-align: center;
    max-width: 270px;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.3);
}
.emptyBtn {
    user-select: none;
    margin-top: 4px;
    cursor: pointer;
    padding: 8px 20px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 1);
    font-size: 12px;
    font-weight: 500;
    transition: .24s ease;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.07);
    }
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    body.lfmusic-phone .catalogPage {
        padding: 0 14px !important;
        overflow-y: auto !important;
        overflow-x: hidden !important;
    }

    body.lfmusic-phone .catalogPage .scrollHeader {
        --catalog-mobile-cover-height: clamp(290px, 86vw, 390px);
        --catalog-mobile-header-overlap: 56px;
        position: relative;
        isolation: isolate;
        display: flex;
        flex-direction: column;
        align-items: center;
        width: 100%;
    }

    body.lfmusic-phone .catalogPage .scrollHeader::after {
        content: "";
        position: absolute;
        left: -14px;
        right: -14px;
        top: calc(var(--catalog-mobile-cover-height) - 18px);
        bottom: 0;
        z-index: 0;
        pointer-events: none;
        background:
            linear-gradient(
                to bottom,
                var(--light-bg) 0%,
                hsla(var(--accent-h), min(var(--accent-s), 52%), 15%, 0.42) 42%,
                hsla(var(--accent-h), min(var(--accent-s), 48%), 14%, 0.22) 70%,
                var(--light-bg) 100%
            );
    }

    body.lfmusic-phone .catalogPage .pageHat {
        position: relative;
        z-index: 1;
        align-items: center !important;
        justify-content: flex-start !important;
        width: calc(100% + 28px) !important;
        min-height: calc(var(--catalog-mobile-cover-height) + var(--catalog-mobile-header-overlap));
        margin: calc(var(--catalog-mobile-header-overlap) * -1) -14px 18px !important;
        gap: 0 !important;
        overflow: hidden;
        border-radius: 0 !important;
    }

    body.lfmusic-phone .catalogPage .pageHat .coverWrap {
        width: 100% !important;
        height: calc(var(--catalog-mobile-cover-height) + var(--catalog-mobile-header-overlap)) !important;
        overflow: hidden;
        border-radius: 0 !important;
        -webkit-mask-image: linear-gradient(to bottom, #000 0%, #000 72%, transparent 100%);
        mask-image: linear-gradient(to bottom, #000 0%, #000 72%, transparent 100%);
    }

    body.lfmusic-phone .catalogPage .pageHat .cover,
    body.lfmusic-phone .catalogPage .pageHat .cover img {
        width: 100% !important;
        height: 100% !important;
        border-radius: 0 !important;
        object-fit: cover;
        filter: saturate(1.05) brightness(0.86);
        transform: none !important;
    }

    /* Tint through filters only: the mix-blend-mode overlay breaks on the large masked layer
       in WebView (only the top part gets colored). */
    body.lfmusic-phone .catalogPage .pageHat .coverWrap.tinted .cover {
        filter: grayscale(1) sepia(1) hue-rotate(calc((var(--accent-h) - 45) * 1deg)) saturate(2.8) brightness(0.74);
    }

    body.lfmusic-phone .catalogPage .pageHat .coverWrap.tinted::after {
        display: none;
    }

    body.lfmusic-phone .catalogPage .pageHat .coverWrap::before {
        content: "";
        position: absolute;
        left: 0;
        right: 0;
        top: 0;
        bottom: 0;
        z-index: 2;
        pointer-events: none;
        background:
            radial-gradient(circle 180px at 50% 84%, hsla(var(--accent-h), min(var(--accent-s), 70%), 58%, 0.24), transparent),
            linear-gradient(
                to bottom,
                hsla(var(--accent-h), min(var(--accent-s), 50%), 10%, 0) 0%,
                hsla(var(--accent-h), min(var(--accent-s), 50%), 10%, 0) 26%,
                hsla(var(--accent-h), min(var(--accent-s), 58%), 14%, 0.12) 54%,
                hsla(var(--accent-h), min(var(--accent-s), 52%), 15%, 0.38) 81%,
                hsla(var(--accent-h), min(var(--accent-s), 52%), 15%, 0.5) 100%
            );
    }

    body.lfmusic-phone .catalogPage .coverEditButtons {
        display: none !important;
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone {
        position: absolute;
        top: auto !important;
        bottom: 20px !important;
        left: 50% !important;
        z-index: 3;
        width: min(270px, calc(100% - 36px)) !important;
        gap: 12px !important;
        align-items: center !important;
        transform: translateX(-50%) !important;
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone,
    body.lfmusic-phone .catalogPage .pageHat .dataZone .info {
        min-width: 0 !important;
        text-align: center !important;
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone .info {
        align-items: center !important;
        margin-top: 0 !important;
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone .info .listType {
        margin-left: 0;
        font-size: 11px;
        font-weight: 700;
        color: hsla(0, 0%, 100%, 0.56);
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone .info .listName {
        max-width: 100%;
        margin-top: 1px;
        font-size: 28px;
        line-height: 1.04;
        white-space: normal;
        overflow-wrap: anywhere;
        color: hsla(0, 0%, 100%, 0.96);
        text-shadow: 0 2px 18px rgba(0, 0, 0, 0.28);
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone .info .badges {
        justify-content: center;
        margin-top: 9px;
        gap: 6px !important;
    }

    body.lfmusic-phone .catalogPage .badge {
        padding: 3px 9px;
        font-size: 11px;
        background-color: hsla(0, 0%, 100%, 0.1);
        color: hsla(0, 0%, 100%, 0.72);
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone .controls {
        justify-content: center;
        gap: 10px !important;
        margin-top: 0;
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone .controls .playListBtn,
    body.lfmusic-phone .catalogPage .pageHat .dataZone .controls .moreOptionsBtn {
        width: 46px !important;
        height: 46px !important;
        min-width: 46px !important;
        min-height: 46px !important;
        padding: 0 !important;
        align-items: center;
        justify-content: center;
        border-radius: 999px;
        box-shadow: 0 10px 28px rgba(0, 0, 0, 0.22);
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone .controls .playListBtn {
        background-color: hsla(0, 0%, 100%, 0.94);
        color: hsla(var(--accent-h), min(var(--accent-s), 62%), 15%, 1);
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone .controls .moreOptionsBtn {
        flex-direction: row;
        background-color: hsla(0, 0%, 100%, 0.12);
        color: hsla(0, 0%, 100%, 0.94);
        backdrop-filter: blur(12px);
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone .controls .playListBtn:hover {
        background-color: hsla(0, 0%, 100%, 1);
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone .controls .moreOptionsBtn:hover {
        background-color: hsla(0, 0%, 100%, 0.18);
    }

    body.lfmusic-phone .catalogPage .playListText {
        position: absolute;
        width: 1px;
        height: 1px;
        overflow: hidden;
        clip: rect(0 0 0 0);
        white-space: nowrap;
    }

    body.lfmusic-phone .catalogPage .pageHat .dataZone .controls svg {
        width: 18px !important;
        height: 18px !important;
    }

    body.lfmusic-phone .catalogPage .genreBarWrap {
        position: relative;
        z-index: 2;
        width: 100% !important;
    }
}
</style>
