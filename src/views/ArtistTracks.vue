<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { goBackOrHome } from '../router'

import PlayIcon from '../components/Icons/PlayIcon.vue'
import PauseIcon from '../components/Icons/PauseIcon.vue'
import SearchIcon from '../components/Icons/SearchIcon.vue'
import EmptyIcon from '../components/Icons/EmptyIcon.vue'
import defaultTrackCover from '../assets/trackCover.webp'

import { usePlayer, type TrackMetadata } from '../composables/usePlayer'
import { usePlaylists } from '../composables/usePlaylists'
import { createRipple } from '../composables/useRipple'
import { isPhoneViewport } from '../composables/usePlatform'
import { useVirtualScroll } from '../composables/useVirtualScroll'
import { searchRanked } from '../composables/useSearch'
import { tracksCountText } from '../utils/format'
import TrackContextMenu from '../components/TrackContextMenu.vue'
import TrackRow from '../components/TrackRow.vue'
import CoverEditor from '../components/CoverEditor.vue'
import AnimatedCover from '../components/AnimatedCover.vue'
import MobileScrollHeader from '../components/MobileScrollHeader.vue'
import { useCustomCovers } from '../composables/useCustomCovers'
import { useExternalSources } from '../composables/useExternalSources'

const route = useRoute()
const router = useRouter()

const {
    currentTrack,
    isPlaying,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    setPlaylistSource,
    loadCoversForTracks,
    clearCoverQueue,
    getTrackByPath,
    cacheTracks,
} = usePlayer()

const { artists: artistPlaylists, loadPlaylists, getPlaylistTracks } = usePlaylists()

const rawArtistName = computed(() => {
    const raw = Array.isArray(route.params.name) ? route.params.name[0] : route.params.name
    try { return decodeURIComponent(raw as string) } catch { return (raw as string) || '' }
})

// Case-insensitive lookup: backend groups artists by lowercased name, so
// "Pyrokinesis" and "pyrokinesis" are the same artist.
const artistPlaylist = computed(() => {
    const lower = rawArtistName.value.toLowerCase()
    return artistPlaylists.value.find(p => p.name.toLowerCase() === lower)
})

// Canonical artist name (from the playlist) ensures one nickname/avatar
// regardless of URL casing. Falls back to the raw name before playlists load.
const artistName = computed(() => artistPlaylist.value?.name ?? rawArtistName.value)

const artistTracks = ref<TrackMetadata[]>([])
async function loadArtistTracks() {
    const pl = artistPlaylist.value
    if (!pl) { artistTracks.value = []; return }
    try {
        const tracks = await getPlaylistTracks(pl.id)
        cacheTracks(tracks)
        // Use live cache objects: cacheTracks merges into an existing cached
        // object, and covers are written there too, so raw (uncached) track
        // references would never show a cover for already-cached tracks.
        artistTracks.value = tracks.map(t => getTrackByPath(t.file_path) ?? t)
    } catch {
        artistTracks.value = []
    }
}
watch([artistPlaylist, artistName], loadArtistTracks, { immediate: true })

const sourceId = computed(() => `artist:${artistName.value}`)

const { getCustomCover } = useCustomCovers()
const { avatarFor } = useExternalSources()

const artistCover = computed(() => {
    const custom = getCustomCover('artist', artistName.value)
    if (custom) return custom
    const external = avatarFor(artistName.value)
    if (external) return external
    const path = artistPlaylist.value?.first_file_path
    if (!path) return defaultTrackCover
    const t = getTrackByPath(path)
    return t?.cover_art || defaultTrackCover
})

// ── Search ──

import { useTypeToSearch } from '../composables/useTypeToSearch'
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

const filteredTracks = computed(() => {
    const q = debouncedSearch.value.trim()
    if (!q) return artistTracks.value
    return searchRanked(
        artistTracks.value,
        q,
        (t) => {
            const fileName = t.file_path.replace(/\\/g, '/').split('/').pop()?.replace(/\.[^/.]+$/, '') || ''
            return [t.title || '', t.album || '', fileName]
        },
        { formatPath: (t) => t.file_path },
    )
})

// ── Header height ──

const scrollHeaderRef = ref<HTMLElement | null>(null)
const scrollHeaderHeight = ref(0)
let headerResizeObserver: ResizeObserver | null = null

onMounted(async () => {
    if (artistPlaylists.value.length === 0) await loadPlaylists()
    const headPath = artistPlaylist.value?.first_file_path
    if (headPath) {
        const t = getTrackByPath(headPath)
        if (t && !t.cover_art) loadCoversForTracks([t])
    }
    if (scrollHeaderRef.value) {
        scrollHeaderHeight.value = scrollHeaderRef.value.offsetHeight
        headerResizeObserver = new ResizeObserver(() => {
            if (scrollHeaderRef.value) {
                scrollHeaderHeight.value = scrollHeaderRef.value.offsetHeight
            }
        })
        headerResizeObserver.observe(scrollHeaderRef.value)
    }
})

onUnmounted(() => {
    headerResizeObserver?.disconnect()
})

// ── Virtual scroll ──

const {
    containerEl: trackListEl,
    onScroll,
    visibleItems: visibleTracks,
    paddingTop: vsPaddingTop,
    paddingBottom: vsPaddingBottom,
    resetScroll,
} = useVirtualScroll(filteredTracks, { itemHeight: 58, buffer: 5, scrollOffset: scrollHeaderHeight })

watch(debouncedSearch, () => resetScroll())
watch(artistName, () => resetScroll())

const isScrollingToTop = ref(false)

watch(visibleTracks, (tracks) => {
    if (!isScrollingToTop.value) {
        loadCoversForTracks(tracks)
    }
}, { immediate: true })

// ── Play button ──

const isArtistActive = computed(() =>
    activePlaylistSource.value === sourceId.value
)

function onPlayArtist() {
    if (isArtistActive.value) {
        togglePlayPause()
    } else {
        if (filteredTracks.value.length === 0) return
        setPlaylistSource(sourceId.value, filteredTracks.value)
        playTrack(filteredTracks.value[0])
    }
}

// ── Sticky header ──

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

// ── Track click ──

function onTrackClick(e: MouseEvent, track: TrackMetadata) {
    if (isPhoneViewport()) {
        onCoverClick(track)
        return
    }
    createRipple(e)
}

function onCoverClick(track: TrackMetadata) {
    if (currentTrack.value?.file_path === track.file_path) {
        togglePlayPause()
        return
    }
    setPlaylistSource(sourceId.value, filteredTracks.value)
    playTrack(track, { manualListStart: true })
}

// ── Context menu ──

const trackMenu = ref<{ x: number; y: number; track: TrackMetadata } | null>(null)

function openTrackMenu(e: MouseEvent, track: TrackMetadata) {
    e.stopPropagation()
    if (trackMenu.value?.track.file_path === track.file_path) {
        trackMenu.value = null
        return
    }
    const btn = e.currentTarget as HTMLElement
    const rect = btn.getBoundingClientRect()
    trackMenu.value = { x: rect.left - 6, y: rect.top + rect.height / 2, track }
}

// ── Back ──

function goBack() {
    goBackOrHome(router)
}

function onCoverImgClick() {
    if (isPhoneViewport()) return
    window.dispatchEvent(new CustomEvent('lfmusic:show-fullscreen-cover', { detail: { src: artistCover.value } }))
}
</script>

<template>
    <div class="pageRoot mobileHeroPage" ref="trackListEl" @scroll="handleScroll">
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader :title="artistName || 'Исполнитель'" :scrolled="scrollY > 16" @top="scrollToTop" />
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div>{{ artistName || 'Исполнитель' }}</div>
                    <button class="toTopBtn" @click="scrollToTop">Вверх</button>
                </div>
            </Transition>
        </div>
        <div ref="scrollHeaderRef" class="scrollHeader">
            <button class="backBtn" @click="goBack">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
                <span>Назад</span>
            </button>
            <div class="pageHat">
                <div class="coverWrap cover-edit-host" @click="onCoverImgClick">
                    <AnimatedCover :src="artistCover" class="cover" alt="cover" />
                    <CoverEditor kind="artist" :coverKey="artistName" size="large" />
                </div>
                <div class="dataZone">
                    <div class="info">
                        <div class="listType">Подборка артиста</div>
                        <div class="listNameRow">
                            <h2 v-truncate class="listName">{{ artistName }}</h2>
                        </div>
                        <div class="badges">
                            <div class="badge" v-tooltip="artistTracks.length > 0 ? 'Количество треков исполнителя' : 'У исполнителя нет треков'">{{ artistTracks.length > 0 ? tracksCountText(artistTracks.length) : 'Пусто' }}</div>
                        </div>
                    </div>
                    <div class="controls">
                        <button class="playListBtn" :disabled="artistTracks.length === 0" @click="onPlayArtist">
                            <PauseIcon v-if="isArtistActive && isPlaying" width="14" height="14" />
                            <PlayIcon v-else width="14" height="14" style="margin-left:0;" />
                            <span class="playListText">Слушать</span>
                        </button>
                    </div>
                </div>
            </div>
            <div class="searchZone">
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
        </div>
        <div
            class="tracksList"
            v-if="filteredTracks.length > 0"
        >
            <div class="vsPad" :style="{ height: vsPaddingTop + 'px' }"></div>
            <TrackRow
                v-for="track in visibleTracks"
                :key="track.file_path"
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
        <div class="emptyState" v-else-if="debouncedSearch && filteredTracks.length === 0">
            <div class="emptyIcon"><SearchIcon width="16" height="16" /></div>
            <div class="emptyText">Ничего не найдено</div>
            <div class="emptyDesc">Попробуйте изменить критерии поиска.</div>
        </div>
        <div class="emptyState" v-else>
            <div class="emptyIcon"><EmptyIcon width="16" height="16" /></div>
            <div class="emptyText">У этого исполнителя нет треков</div>
            <div class="emptyDesc">Подборка формируется автоматически из вашей библиотеки.</div>
        </div>
        <TrackContextMenu
            v-if="trackMenu"
            :key="trackMenu.track.file_path"
            :x="trackMenu.x"
            :y="trackMenu.y"
            :track="trackMenu.track"
            alignRight
            centerY
            @close="trackMenu = null"
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
.backBtn {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 4px;
    padding: 6px 12px 6px 8px;
    margin-bottom: 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.8);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    user-select: none;
    transition: .24s ease;
}
.backBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.1);
    color: hsla(0, 0%, 100%, 1);
}
.backBtn svg {
    color: hsla(0, 0%, 100%, 0.6);
    transition: .24s ease;
}
.backBtn:hover svg {
    color: hsla(0, 0%, 100%, 1);
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
    border-radius: 9999px;
    cursor: pointer;
}
.pageHat .cover {
    user-select: none;
    width: 200px;
    height: 200px;
    background-color: transparent;
    border-radius: 9999px;
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
.pageHat .dataZone .info .listNameRow {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    margin-top: 2px;
    width: fit-content;
    max-width: 100%;
}
.pageHat .dataZone .info .listName {
    cursor: default;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 42px;
    font-weight: 900;
    color: hsla(0, 0%, 100%, 0.9);
    line-height: 1.1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
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
.pageHat .dataZone .controls .playListBtn:disabled {
    cursor: not-allowed;
    background-color: hsla(0, 0%, 100%, 0.3);
    &:hover {
        background-color: hsla(0, 0%, 100%, 0.3);
    }
}
.playListBtn svg {
    width: 16px;
}
.searchZone {
    position: relative;
    margin-top: 24px;
    margin-bottom: 16px;
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
.tracksList {
    margin-top: 4px;
}
.vsPad {
    width: 100%;
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
</style>
