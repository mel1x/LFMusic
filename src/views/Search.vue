<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'

import SearchIcon from '../components/Icons/SearchIcon.vue'
import PlayIcon from '../components/Icons/PlayIcon.vue'
import PauseIcon from '../components/Icons/PauseIcon.vue'
import MoreIcon from '../components/Icons/MoreIcon.vue'

import defaultTrackCover from '../assets/trackCover.webp'
import defaultPlaylistCover from '../assets/playlistCover.webp'

import { invoke } from '@tauri-apps/api/core'
import { usePlayer, type TrackMetadata } from '../composables/usePlayer'
import { usePlaylists, type PlaylistInfo } from '../composables/usePlaylists'
import { searchRanked } from '../composables/useSearch'
import { createRipple } from '../composables/useRipple'
import { isPhoneViewport } from '../composables/usePlatform'

import TrackArtists from '../components/TrackArtists.vue'
import TrackContextMenu from '../components/TrackContextMenu.vue'
import AnimatedCover from '../components/AnimatedCover.vue'
import LoadingSpinner from '../components/LoadingSpinner.vue'
import { useCustomCovers } from '../composables/useCustomCovers'
import { useExternalSources } from '../composables/useExternalSources'
import { useCoverGate } from '../composables/useCoverGate'
import { useVirtualScroll } from '../composables/useVirtualScroll'

const router = useRouter()

// ── Past search queries ─────────────────────────────────────────────────
const PAST_SEARCHES_KEY = 'lfmusic_past_searches'
const MAX_PAST_SEARCHES = 15

const pastSearches = ref<string[]>([])

function loadPastSearches() {
    try {
        const raw = localStorage.getItem(PAST_SEARCHES_KEY)
        if (raw) pastSearches.value = JSON.parse(raw)
    } catch { /* ignore */ }
}

function savePastSearches() {
    localStorage.setItem(PAST_SEARCHES_KEY, JSON.stringify(pastSearches.value))
}

function addPastSearch(queryStr: string) {
    const trimmed = queryStr.trim()
    if (!trimmed) return
    pastSearches.value = pastSearches.value.filter(q => q !== trimmed)
    pastSearches.value.unshift(trimmed)
    if (pastSearches.value.length > MAX_PAST_SEARCHES) {
        pastSearches.value = pastSearches.value.slice(0, MAX_PAST_SEARCHES)
    }
    savePastSearches()
}

const filteredPastSearches = computed(() => {
    const q = rawSearch.value.trim().toLowerCase()
    if (!q) return pastSearches.value.slice(0, 15)
    return pastSearches.value.filter(p =>
        p.toLowerCase().includes(q)
    ).slice(0, 15)
})

function onPastSearchClick(query: string) {
    rawSearch.value = query
    debouncedSearch.value = query
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    searchInput.value?.focus()
}

function onSearchKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
        const trimmed = rawSearch.value.trim()
        if (trimmed) {
            addPastSearch(trimmed)
        }
    }
}

onMounted(() => {
    loadPastSearches()
})

// ── Search input ───────────────────────────────────────────────────────────

const searchInput = ref<HTMLInputElement | null>(null)
const rawSearch = ref('')
const debouncedSearch = ref('')
let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null

const focusSearch = () => searchInput.value?.focus()

function onSearchInput(e: Event) {
    const value = (e.target as HTMLInputElement).value
    rawSearch.value = value
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    searchDebounceTimer = setTimeout(() => {
        debouncedSearch.value = value
    }, 200)
}

function clearSearch() {
    rawSearch.value = ''
    debouncedSearch.value = ''
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    searchInput.value?.focus()
}

// Любое целевое действие над результатами (воспроизведение, открытие,
// разворачивание секции) означает, что текущий запрос оказался полезным —
// сохраняем его в подсказки. Сам ввод/выбор подсказки сюда не входит.
function rememberCurrentSearch() {
    const trimmed = rawSearch.value.trim()
    if (trimmed) addPastSearch(trimmed)
}

const query = computed(() => debouncedSearch.value.trim())
const hasQuery = computed(() => !!query.value)

onMounted(() => {
    nextTick(() => searchInput.value?.focus())
    loadPlaylists()
    window.addEventListener('resize', recomputeTracksOffset)
})

const {
    currentTrack,
    isPlaying,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    setPlaylistSource,
    formatTime,
    loadCoversForTracks,
    getTrackByPath,
    cacheTracks,
} = usePlayer()

const {
    userPlaylists,
    albums: albumPlaylists,
    artists: artistPlaylists,
    loadPlaylists,
    getPlaylistTracks,
} = usePlaylists()

const { getCustomCover } = useCustomCovers()
const { avatarFor } = useExternalSources()

function albumCoverFor(name: string, fallback: string | null): string {
    return getCustomCover('album', name) || fallback || defaultTrackCover
}
function playlistCoverFor(id: number, fallback: string | null): string {
    return getCustomCover('playlist', id) || fallback || defaultPlaylistCover
}
function artistCoverFor(name: string, fallback: string | null): string {
    return getCustomCover('artist', name) || avatarFor(name) || fallback || defaultTrackCover
}

// ── Artists ────────────────────────────────────────────────────────────────

interface ArtistResult {
    id: number
    name: string
    cover: string | null
    firstPath: string | null
    trackCount: number
}

const artistResults = computed<ArtistResult[]>(() => {
    if (!hasQuery.value) return []
    const list = artistPlaylists.value.map(pl => {
        const t = pl.first_file_path ? getTrackByPath(pl.first_file_path) : undefined
        return {
            id: pl.id,
            name: pl.name,
            cover: t?.cover_art ?? null,
            firstPath: pl.first_file_path,
            trackCount: pl.track_count,
        }
    })
    return searchRanked(list, query.value, (a) => [a.name])
})

const artistsExpanded = ref(false)
const artistsContainer = ref<HTMLElement | null>(null)
const artistsOverflow = ref(false)
const artistsCollapsedMaxHeight = ref(0)
let artistsRO: ResizeObserver | null = null

function recomputeArtistsOverflow() {
    const el = artistsContainer.value
    if (!el || artistResults.value.length === 0) {
        artistsOverflow.value = false
        return
    }
    const item = el.querySelector('.artistBadge') as HTMLElement | null
    if (!item) {
        artistsOverflow.value = false
        return
    }
    const cs = getComputedStyle(el)
    const rowGap = parseFloat(cs.rowGap || '0') || 0
    const rowH = item.offsetHeight
    const collapsedH = rowH * 3 + rowGap * 2
    artistsCollapsedMaxHeight.value = collapsedH
    const wasCollapsed = el.style.maxHeight !== ''
    if (wasCollapsed) {
        el.style.maxHeight = ''
    }
    const fullH = el.scrollHeight
    if (wasCollapsed && !artistsExpanded.value) {
        el.style.maxHeight = collapsedH + 'px'
    }
    artistsOverflow.value = fullH > collapsedH + 1
}

watch(artistResults, async () => {
    artistsExpanded.value = false
    await nextTick()
    recomputeArtistsOverflow()
})

watch(artistsContainer, (el) => {
    if (el) {
        if (!artistsRO) artistsRO = new ResizeObserver(() => recomputeArtistsOverflow())
        artistsRO.observe(el)
        nextTick(() => recomputeArtistsOverflow())
    }
})

onUnmounted(() => {
    artistsRO?.disconnect()
    artistsRO = null
})

function openArtist(a: ArtistResult) {
    rememberCurrentSearch()
    router.push(`/artist/${encodeURIComponent(a.name)}`)
}

// ── Albums ─────────────────────────────────────────────────────────────────

interface AlbumResult {
    id: number
    name: string
    cover: string | null
    artist: string
    firstPath: string | null
    trackCount: number
}

const albumResults = computed<AlbumResult[]>(() => {
    if (!hasQuery.value) return []
    const list = albumPlaylists.value.map(pl => {
        const t = pl.first_file_path ? getTrackByPath(pl.first_file_path) : undefined
        return {
            id: pl.id,
            name: pl.name,
            cover: t?.cover_art ?? null,
            artist: t?.artist || 'Неизвестный исполнитель',
            firstPath: pl.first_file_path,
            trackCount: pl.track_count,
        }
    })
    return searchRanked(list, query.value, (a) => [a.name, a.artist])
})

function openAlbum(a: AlbumResult) {
    rememberCurrentSearch()
    router.push(`/album/${encodeURIComponent(a.name)}`)
}

function isAlbumActive(a: AlbumResult) {
    return activePlaylistSource.value === `album:${a.name}`
}

async function onPlayAlbum(e: MouseEvent, a: AlbumResult) {
    e.stopPropagation()
    const sourceId = `album:${a.name}`
    if (activePlaylistSource.value === sourceId) {
        togglePlayPause()
        return
    }
    if (a.trackCount === 0) return
    const tracks = await getPlaylistTracks(a.id)
    if (tracks.length === 0) return
    const merged = tracks.map(t => getTrackByPath(t.file_path) ?? t)
    setPlaylistSource(sourceId, merged)
    rememberCurrentSearch()
    await playTrack(merged[0], { searchStart: true })
}

// ── Tracks ─────────────────────────────────────────────────────────────────

// Поиск треков — на бэкенде (то же ранжирование, перенесённое в Rust).
const trackResults = ref<TrackMetadata[]>([])
let trackSearchReqId = 0
watch(query, async (q) => {
    const reqId = ++trackSearchReqId
    if (!q) { trackResults.value = []; return }
    try {
        const res = await invoke<TrackMetadata[]>('library_search', { query: q })
        if (reqId === trackSearchReqId) {
            trackResults.value = res
            cacheTracks(res)
        }
    } catch {
        if (reqId === trackSearchReqId) trackResults.value = []
    }
}, { immediate: true })

const tracksExpanded = ref(false)

// ── Виртуализация раскрытого списка треков ───────────────────────────────────
// Раскрытый список («Больше») может содержать тысячи строк — рендерим окном
// видимых через useVirtualScroll (как в Catalog). Скроллер — вся страница
// (pageRootEl), смещение секции — её offsetTop внутри страницы (pageRoot имеет
// position: relative). Фиксированная высота строки 58px согласована со scrollTop.
const tracksListEl = ref<HTMLElement | null>(null)
const tracksOffset = ref(0)

const {
    containerEl: pageRootEl,
    onScroll: vsOnScroll,
    visibleItems: vsVisibleTracks,
    paddingTop: vsPaddingTop,
    paddingBottom: vsPaddingBottom,
} = useVirtualScroll(trackResults, { itemHeight: 58, buffer: 6, scrollOffset: tracksOffset })

// Свёрнуто — первые 8; раскрыто — окно виртуализации.
const renderedTracks = computed(() =>
    tracksExpanded.value ? vsVisibleTracks.value : trackResults.value.slice(0, 8),
)
const tracksListStyle = computed(() =>
    tracksExpanded.value
        ? { paddingTop: vsPaddingTop.value + 'px', paddingBottom: vsPaddingBottom.value + 'px' }
        : {},
)

function recomputeTracksOffset() {
    tracksOffset.value = tracksListEl.value?.offsetTop ?? 0
}

watch([renderedTracks, tracksExpanded], () => {
    nextTick(recomputeTracksOffset)
})

watch(trackResults, () => {
    tracksExpanded.value = false
})

const trackSourceId = computed(() => `search:tracks:${query.value}`)

function onTrackCoverClick(track: TrackMetadata) {
    if (currentTrack.value?.file_path === track.file_path) {
        togglePlayPause()
        return
    }
    rememberCurrentSearch()
    setPlaylistSource(trackSourceId.value, trackResults.value)
    playTrack(track, { searchStart: true, manualListStart: true })
}

function onTrackClick(e: MouseEvent, track: TrackMetadata) {
    if (isPhoneViewport()) {
        onTrackCoverClick(track)
        return
    }
    createRipple(e)
}

const trackMenu = ref<{ x: number; y: number; track: TrackMetadata } | null>(null)

function openTrackMenu(e: MouseEvent, track: TrackMetadata) {
    e.stopPropagation()
    if (trackMenu.value?.track.file_path === track.file_path) {
        trackMenu.value = null
        return
    }
    rememberCurrentSearch()
    const btn = e.currentTarget as HTMLElement
    const rect = btn.getBoundingClientRect()
    trackMenu.value = { x: rect.left - 6, y: rect.top + rect.height / 2, track }
}

watch(renderedTracks, (tracks) => {
    if (tracks.length) loadCoversForTracks(tracks)
}, { immediate: true })

watch([albumResults, artistResults], () => {
    const pending: TrackMetadata[] = []
    for (const a of albumResults.value) {
        if (a.firstPath) {
            const t = getTrackByPath(a.firstPath)
            if (t && !t.cover_art) pending.push(t)
        }
    }
    for (const a of artistResults.value) {
        if (a.firstPath) {
            const t = getTrackByPath(a.firstPath)
            if (t && !t.cover_art) pending.push(t)
        }
    }
    if (pending.length) loadCoversForTracks(pending)
})

// Hold back results until the first 30 artist avatars and 15 track covers
// have resolved, so they don't flash empty squares on appearance.
const { ready: coversReady } = useCoverGate(() => {
    const out: TrackMetadata[] = []
    for (const a of artistResults.value.slice(0, 30)) {
        if (!a.firstPath) continue
        const t = getTrackByPath(a.firstPath)
        if (t) out.push(t)
    }
    out.push(...trackResults.value.slice(0, 15))
    return out
})

// ── Playlists ──────────────────────────────────────────────────────────────

const playlistResults = computed<PlaylistInfo[]>(() => {
    if (!hasQuery.value) return []
    return searchRanked(userPlaylists.value, query.value, (p) => [p.name])
})

const playlistCovers = ref<Record<number, string | null>>({})

async function refreshPlaylistCover(pl: PlaylistInfo) {
    if (pl.track_count === 0) {
        playlistCovers.value[pl.id] = null
        return
    }
    const tracks = await getPlaylistTracks(pl.id)
    const first = tracks[0]
    if (!first) {
        playlistCovers.value[pl.id] = null
        return
    }
    cacheTracks([first])
    const live = getTrackByPath(first.file_path) ?? first
    if (!live.cover_art) loadCoversForTracks([live])
    playlistCovers.value[pl.id] = live.cover_art
}

watch(playlistResults, async (list) => {
    for (const pl of list) {
        if (playlistCovers.value[pl.id] === undefined) {
            await refreshPlaylistCover(pl)
        }
    }
}, { immediate: true })

function openPlaylist(pl: PlaylistInfo) {
    rememberCurrentSearch()
    router.push(`/playlist/${pl.id}`)
}

function isPlaylistActive(pl: PlaylistInfo) {
    return activePlaylistSource.value === `playlist:${pl.id}`
}

async function onPlayPlaylist(e: MouseEvent, pl: PlaylistInfo) {
    e.stopPropagation()
    const sourceId = `playlist:${pl.id}`
    if (activePlaylistSource.value === sourceId) {
        togglePlayPause()
        return
    }
    if (pl.track_count === 0) return
    const tracks = await getPlaylistTracks(pl.id)
    if (tracks.length === 0) return
    const merged = tracks.map(t => getTrackByPath(t.file_path) ?? t)
    setPlaylistSource(sourceId, merged)
    rememberCurrentSearch()
    await playTrack(merged[0], { searchStart: true })
}

// ── Horizontal scroll helper ───────────────────────────────────────────────

function useScrollLine(baseItemWidth = 210, gap = 16) {
    const el = ref<HTMLElement | null>(null)
    const scrollLeft = ref(0)
    const scrollMax = ref(0)
    const itemWidth = ref(baseItemWidth)
    const measured = ref(false)
    let ro: ResizeObserver | null = null

    async function update() {
        const node = el.value
        if (!node) return
        const cw = node.clientWidth
        if (cw > 0) {
            const n = Math.max(1, Math.floor((cw + gap) / (baseItemWidth + gap)))
            itemWidth.value = (cw - (n - 1) * gap) / n
            measured.value = true
        }
        await nextTick()
        scrollLeft.value = node.scrollLeft
        scrollMax.value = Math.max(0, node.scrollWidth - node.clientWidth)
    }
    function onScroll() {
        const node = el.value
        if (!node) return
        scrollLeft.value = node.scrollLeft
        scrollMax.value = Math.max(0, node.scrollWidth - node.clientWidth)
    }
    function scrollByPage(dir: 1 | -1) {
        const node = el.value
        if (!node) return
        node.scrollBy({ left: dir * node.clientWidth * 0.9, behavior: 'smooth' })
    }
    function observe() {
        const node = el.value
        if (!node) return
        update()
        if (!ro) {
            ro = new ResizeObserver(() => update())
            ro.observe(node)
        }
    }
    function unobserve() { ro?.disconnect(); ro = null }

    const canPrev = computed(() => scrollLeft.value > 1)
    const canNext = computed(() => scrollLeft.value < scrollMax.value - 1)

    return { el, canPrev, canNext, itemWidth, measured, onScroll, scrollByPage, observe, unobserve, update }
}

const albumsLine = useScrollLine(190, 16)
const playlistsLine = useScrollLine(190, 16)

watch([albumResults, playlistResults], async () => {
    await nextTick()
    albumsLine.observe()
    playlistsLine.observe()
})

// The results (and their horizontal line lists) mount only once the gate
// transition finishes entering — re-measure then so scroll state/visibility
// is correct. Fires harmlessly for the spinner/empty states too (no lists).
function onGateAfterEnter() {
    albumsLine.observe()
    playlistsLine.observe()
}

onUnmounted(() => {
    albumsLine.unobserve()
    playlistsLine.unobserve()
    window.removeEventListener('resize', recomputeTracksOffset)
})

// ── No results ─────────────────────────────────────────────────────────────

const noResults = computed(() => hasQuery.value &&
    artistResults.value.length === 0 &&
    albumResults.value.length === 0 &&
    trackResults.value.length === 0 &&
    playlistResults.value.length === 0)

// ── Sticky header ──────────────────────────────────────────────────────────

const scrollY = ref(0)
const showStickyHeader = computed(() => scrollY.value > 200)

function handleScroll(e: Event) {
    scrollY.value = (e.target as HTMLElement).scrollTop
    recomputeTracksOffset()
    vsOnScroll(e)
}

function scrollToTop() {
    pageRootEl.value?.scrollTo({ top: 0, behavior: 'smooth' })
}
</script>

<template>
    <div class="pageRoot searchPage" ref="pageRootEl" @scroll="handleScroll">
        <div class="stickyHeaderAnchor">
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div>Поиск</div>
                    <button class="toTopBtn" @click="scrollToTop">Вверх</button>
                </div>
            </Transition>
        </div>
        <h1 class="pageTitle">Поиск</h1>
        <div class="searchZone">
            <SearchIcon class="searchIcon" @click="focusSearch" />
            <input
                ref="searchInput"
                type="text"
                class="search"
                name="search"
                id="search"
                placeholder="Поиск треков, альбомов, исполнителей и плейлистов"
                autocomplete="off"
                :value="rawSearch"
                @input="onSearchInput"
                @keydown="onSearchKeydown"
            />
            <div class="searchClear" v-if="rawSearch" @click="clearSearch">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
            </div>
        </div>

        <!-- Past search queries -->
        <div class="pastSearchesBar" v-if="pastSearches.length > 0 && !rawSearch.trim()">
            <button
                v-for="(pq, idx) in filteredPastSearches"
                :key="idx"
                class="pastSearchChip"
                @click="onPastSearchClick(pq)"
            >
                {{ pq }}
            </button>
        </div>

        <Transition name="gate-fade" mode="out-in" @after-enter="onGateAfterEnter">
        <div v-if="!hasQuery" class="emptyState" key="gate-no-query">
            <div class="emptyIcon"><SearchIcon width="16" height="16" /></div>
            <div class="emptyText">Введите запрос</div>
            <div class="emptyDesc">Введите текст для поиска треков, альбомов, исполнителей и плейлистов в вашей библиотеке.</div>
        </div>

        <div v-else-if="noResults" class="emptyState" key="gate-no-results">
            <div class="emptyIcon"><SearchIcon width="16" height="16" /></div>
            <div class="emptyText">Ничего не найдено</div>
            <div class="emptyDesc">Попробуйте изменить запрос или добавить папки с треками в настройках.</div>
        </div>

        <LoadingSpinner v-else-if="!coversReady" key="gate-spinner" />

        <div v-else class="searchResults" key="gate-results">
            <!-- Artists -->
            <section v-if="artistResults.length > 0" class="pageLine">
                <div class="lineHat">
                    <div class="lineTitle nonClickable">Исполнители</div>
                </div>
                <div class="artistsBlockWrap">
                    <div
                        class="artistsList"
                        ref="artistsContainer"
                        :class="{ collapsed: !artistsExpanded && artistsOverflow }"
                        :style="!artistsExpanded && artistsOverflow ? { maxHeight: artistsCollapsedMaxHeight + 'px' } : {}"
                    >
                        <div
                            v-for="a in artistResults"
                            :key="a.id"
                            class="artistBadge"
                            @click="openArtist(a)"
                            v-tooltip="'Открыть подборку исполнителя'"
                        >
                            <AnimatedCover class="artistAvatar" :src="artistCoverFor(a.name, a.cover)" />
                            <span v-truncate class="artistBadgeName">{{ a.name }}</span>
                        </div>
                    </div>
                    <div
                        v-if="artistsOverflow"
                        class="moreArtistsRow"
                        :class="{ collapsed: !artistsExpanded }"
                    >
                        <button
                            class="moreArtistsBtn"
                            @click="artistsExpanded = !artistsExpanded; if (artistsExpanded) rememberCurrentSearch()"
                        >
                            <span class="moreArtistsLabel">{{ artistsExpanded ? 'Свернуть' : 'Показать всех' }}</span>
                            <span v-if="!artistsExpanded" class="moreArtistsCount">{{ artistResults.length }}</span>
                            <svg class="chev" :class="{ expanded: artistsExpanded }" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
                        </button>
                    </div>
                </div>
            </section>

            <!-- Albums -->
            <section v-if="albumResults.length > 0" class="pageLine">
                <div class="lineHat">
                    <div class="lineTitle nonClickable">Альбомы</div>
                    <div class="lineControls">
                        <button class="prevBtn" :disabled="!albumsLine.canPrev.value" @click="albumsLine.scrollByPage(-1)"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                        <button class="nextBtn" :disabled="!albumsLine.canNext.value" @click="albumsLine.scrollByPage(1)"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                    </div>
                </div>
                <div
                    class="lineList"
                    :ref="(el: any) => { albumsLine.el.value = el as HTMLElement | null }"
                    :style="{ '--card-w': albumsLine.itemWidth.value + 'px', visibility: albumsLine.measured.value ? 'visible' : 'hidden' }"
                    @scroll="albumsLine.onScroll"
                >
                    <div
                        v-for="album in albumResults"
                            :key="album.id"
                            class="playlistItem albumItem"
                            @click="openAlbum(album)"
                            v-tooltip="'Открыть альбом'"
                        >
                        <div class="coverWrap">
                            <AnimatedCover class="cover" :src="albumCoverFor(album.name, album.cover)" />
                            <button
                                class="overlayBtn playBtn"
                                :disabled="album.trackCount === 0"
                                @click="onPlayAlbum($event, album)"
                                v-tooltip="isAlbumActive(album) && isPlaying ? 'Пауза' : 'Слушать'"
                            >
                                <PauseIcon v-if="isAlbumActive(album) && isPlaying" width="14" height="14" />
                                <PlayIcon v-else width="14" height="14" style="margin-left:1px;" />
                            </button>
                        </div>
                        <div v-truncate class="title">{{ album.name }}</div>
                        <div v-truncate class="subtitle">{{ album.artist }}</div>
                    </div>
                </div>
            </section>

            <!-- Tracks -->
            <section v-if="trackResults.length > 0" class="pageLine">
                <div class="lineHat">
                    <div class="lineTitle nonClickable">Треки</div>
                </div>
                <div class="tracksList" ref="tracksListEl" :style="tracksListStyle">
                    <div
                        v-for="track in renderedTracks"
                        :key="track.file_path"
                        class="trackElem"
                        :class="{
                            active: currentTrack?.file_path === track.file_path,
                            activePaused: currentTrack?.file_path === track.file_path && !isPlaying,
                        }"
                        @click="onTrackClick($event, track)"
                    >
                        <div class="hoverIcon" @click.stop="onTrackCoverClick(track)">
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
                        <div class="trackCover" @click.stop="onTrackCoverClick(track)">
                            <img
                                :src="track.cover_art || defaultTrackCover"
                                alt=""
                                draggable="false"
                                class="trackCoverImg"
                            />
                        </div>
                        <div class="trackInfo">
                            <div v-truncate class="trackName">{{ track.title || track.file_path.split(/[\\/]/).pop()?.replace(/\.[^/.]+$/, '') }}</div>
                            <div class="trackArtist"><TrackArtists :artist="track.artist" /></div>
                        </div>
                        <div class="trackTime" :class="{ hidden: trackMenu?.track.file_path === track.file_path }">{{ track.duration_secs ? formatTime(track.duration_secs) : '' }}</div>
                        <div class="hoverOptionsBtn" :class="{ menuOpen: trackMenu?.track.file_path === track.file_path }" @mousedown.stop="openTrackMenu($event, track)" @click.stop><MoreIcon width="26" height="26" /></div>
                    </div>
                </div>
                <button
                    v-if="trackResults.length > 8"
                    class="moreTracksBtn"
                    @click="tracksExpanded = !tracksExpanded; if (tracksExpanded) rememberCurrentSearch()"
                >
                    {{ tracksExpanded ? 'Свернуть' : 'Больше' }}
                </button>
            </section>

            <!-- Playlists -->
            <section v-if="playlistResults.length > 0" class="pageLine">
                <div class="lineHat">
                    <div class="lineTitle nonClickable">Плейлисты</div>
                    <div class="lineControls">
                        <button class="prevBtn" :disabled="!playlistsLine.canPrev.value" @click="playlistsLine.scrollByPage(-1)"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                        <button class="nextBtn" :disabled="!playlistsLine.canNext.value" @click="playlistsLine.scrollByPage(1)"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                    </div>
                </div>
                <div
                    class="lineList"
                    :ref="(el: any) => { playlistsLine.el.value = el as HTMLElement | null }"
                    :style="{ '--card-w': playlistsLine.itemWidth.value + 'px', visibility: playlistsLine.measured.value ? 'visible' : 'hidden' }"
                    @scroll="playlistsLine.onScroll"
                >
                    <div
                        v-for="pl in playlistResults"
                        :key="pl.id"
                        class="playlistItem"
                        @click="openPlaylist(pl)"
                    >
                        <div class="coverWrap">
                            <AnimatedCover class="cover" :src="playlistCoverFor(pl.id, playlistCovers[pl.id])" />
                            <button
                                class="overlayBtn playBtn"
                                :disabled="pl.track_count === 0"
                                @click="onPlayPlaylist($event, pl)"
                                v-tooltip="pl.track_count === 0 ? 'Плейлист пуст' : (isPlaylistActive(pl) && isPlaying ? 'Пауза' : 'Слушать')"
                            >
                                <PauseIcon v-if="isPlaylistActive(pl) && isPlaying" width="14" height="14" />
                                <PlayIcon v-else width="14" height="14" style="margin-left:1px;" />
                            </button>
                        </div>
                        <div v-truncate class="title">{{ pl.name }}</div>
                    </div>
                </div>
            </section>
        </div>
        </Transition>

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
    position: relative;
    padding: 20px 28px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: var(--light-bg);
    background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
    background-repeat: no-repeat;
    background-attachment: local;
    overflow-y: auto;
}
.pageRoot::-webkit-scrollbar { width: 6px; }
.pageRoot::-webkit-scrollbar-track { background-color: var(--light-bg); }
.pageRoot::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
}

/* ── Sticky header ────────────────────────────────────────────── */
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
}
.toTopBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
}

/* ── Search input ─────────────────────────────────────────────── */
.searchZone {
    position: relative;
    width: 100%;
    flex-shrink: 0;
    margin-bottom: 12px;
}
.searchIcon {
    cursor: text;
    position: absolute;
    top: 50%;
    left: 14px;
    transform: translateY(-50%);
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.4);
    transition: .24s ease;
}
.searchZone:hover .searchIcon {
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.8);
}
.searchClear {
    position: absolute;
    height: max-content;
    width: max-content;
    top: 50%;
    right: 11px;
    transform: translateY(-50%);
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.3);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 6px;
    border-radius: 50%;
    transition: .24s ease;
}
.searchClear:hover {
    color: hsla(0, 0%, 100%, 0.6);
}
.search {
    width: 100%;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.04);
    padding: 13px 38px 14px 42px;
    font-size: 14px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.9);
}
.search::placeholder {
    user-select: none;
    color: hsla(0, 0%, 100%, 0.3);
    font-size: 14px;
    font-weight: 600;
}

/* ── Past search queries ─────────────────────────────────────── */
.pastSearchesBar {
    display: flex;
    flex-direction: row;
    gap: 6px;
    overflow-x: auto;
    flex-shrink: 0;
    padding-bottom: 2px;
    scrollbar-width: none;
    scroll-behavior: smooth;
    margin-bottom: 28px;
}
.pastSearchesBar::-webkit-scrollbar {
    display: none;
}
.pastSearchChip {
    display: block;
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
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
}
.pastSearchChip:hover {
    background-color: hsla(0, 0%, 100%, 0.1);
    color: hsla(0, 0%, 100%, 0.8);
}

/* ── Section ──────────────────────────────────────────────────── */
.pageLine {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 13px;
    margin-bottom: 36px;
}
.pageLine:last-of-type {
    margin-bottom: 0;
}
.lineHat {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
}
.lineTitle {
    display: flex;
    flex-direction: row;
    align-items: center;
    user-select: none;
    color: hsla(0, 0%, 100%, 0.85);
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 22px;
}
.lineTitle.nonClickable {
    cursor: default;
}
.lineControls {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 4px;
}
.lineControls button {
    cursor: pointer;
    border-radius: 9999px;
    padding: 6px;
    border: 2px solid hsla(0, 0%, 100%, 0.2);
    background-color: var(--light-bg);
    color: hsla(0, 0%, 100%, 0.6);
    opacity: 0;
    transition: .24s ease;
}
.lineControls button:hover {
    border: 2px solid hsla(0, 0%, 100%, 0.4);
    color: hsla(0, 0%, 100%, 0.95);
}
.lineControls button:disabled:hover {
    border: 2px solid hsla(0, 0%, 100%, 0.2);
    color: hsla(0, 0%, 100%, 0.6);
}
.pageLine:hover .lineControls button {
    opacity: 1;
}
.pageLine:hover .lineControls button:disabled {
    opacity: 0.65;
}

/* ── Artists row (badges) ─────────────────────────────────────── */
.artistsBlockWrap {
    position: relative;
}
.artistsList {
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
    gap: 8px;
    overflow: hidden;
    transition: max-height .28s ease;
}
.artistsList.collapsed {
    /* max-height set inline */
}
.artistBadge {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 10px;
    padding: 4px 14px 4px 4px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.9);
    font-size: 13px;
    font-weight: 500;
    height: 38px;
    box-sizing: border-box;
    cursor: pointer;
    user-select: none;
    max-width: 100%;
    transition: background-color .2s ease, color .2s ease;
}
.artistBadge:hover {
    background-color: hsla(0, 0%, 100%, 0.09);
    color: hsla(0, 0%, 100%, 1);
}
.artistAvatar {
    display: block;
    width: 30px;
    height: 30px;
    border-radius: 9999px;
    background-color: #202020;
    object-fit: cover;
    object-position: center;
    aspect-ratio: 1 / 1;
    flex-shrink: 0;
    user-select: none;
}
.artistBadgeName {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 220px;
}
.moreArtistsRow {
    display: flex;
    justify-content: center;
    margin-top: 14px;
}
.moreArtistsRow.collapsed {
    position: absolute;
    left: 0;
    right: 0;
    bottom: -6px;
    margin-top: 0;
    padding-top: 28px;
    pointer-events: none;
    background: linear-gradient(
        to bottom,
        hsla(var(--accent-h), min(var(--accent-s), 45%), 15%, 0) 0%,
        hsla(var(--accent-h), min(var(--accent-s), 45%), 15%, 0.85) 55%,
        hsla(var(--accent-h), min(var(--accent-s), 45%), 15%, 1) 100%
    );
}
.moreArtistsRow.collapsed .moreArtistsBtn {
    pointer-events: auto;
}
.moreArtistsBtn {
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 6px 0 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.85);
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.01em;
    transition: background-color .2s ease, color .2s ease, transform .2s ease;
}
.moreArtistsBtn:hover {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.18);
    color: #fff;
    transform: translateY(-1px);
}
.moreArtistsLabel {
    line-height: 1;
}
.moreArtistsCount {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    height: 22px;
    padding: 0 7px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.08);
    color: hsla(0, 0%, 100%, 0.7);
    font-size: 11px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    transition: background-color .2s ease, color .2s ease;
}
.moreArtistsBtn:hover .moreArtistsCount {
    background-color: hsla(0, 0%, 100%, 0.18);
    color: #fff;
}
.moreArtistsBtn .chev {
    margin-right: 6px;
    color: hsla(0, 0%, 100%, 0.55);
    transition: transform .24s ease, color .2s ease;
}
.moreArtistsBtn:hover .chev {
    color: #fff;
}
.moreArtistsBtn .chev.expanded {
    transform: rotate(180deg);
}

/* ── Horizontal lines (albums / playlists) ────────────────────── */
.lineList {
    position: relative;
    display: flex;
    flex-direction: row;
    gap: 16px;
    flex-wrap: nowrap;
    overflow-x: auto;
    overflow-y: hidden;
    scroll-behavior: smooth;
    scrollbar-width: none;
    -ms-overflow-style: none;
}
.lineList::-webkit-scrollbar {
    display: none;
}
.playlistItem {
    flex-shrink: 0;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
}
.playlistItem .coverWrap {
    position: relative;
    width: var(--card-w, 200px);
    height: var(--card-w, 200px);
    border-radius: 18px;
    overflow: hidden;
}
.playlistItem .cover {
    display: block;
    width: var(--card-w, 200px);
    height: var(--card-w, 200px);
    border-radius: 18px;
    object-fit: cover;
    transition: filter .24s ease, transform .24s ease;
    user-select: none;
}
.playlistItem:hover .cover {
    filter: brightness(0.6);
}
.playlistItem .overlayBtn {
    position: absolute;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    border-radius: 9999px;
    color: hsla(0, 0%, 100%, 0.95);
    background-color: hsla(0, 0%, 0%, 0.45);
    backdrop-filter: blur(8px);
    opacity: 0;
    transform: translateY(4px);
    transition: opacity .24s ease, transform .24s ease, background-color .24s ease, color .24s ease;
    z-index: 2;
}
.playlistItem:hover .overlayBtn {
    opacity: 1;
    transform: translateY(0);
}
.playlistItem .playBtn {
    left: 12px;
    bottom: 12px;
    padding: 12px;
    color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
    background-color: hsla(0, 0%, 100%, 0.95);
}
.playlistItem .playBtn:hover {
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
    background-color: hsla(0, 0%, 100%, 1);
}
.playlistItem .playBtn:disabled {
    cursor: default;
    color: hsla(0, 0%, 100%, 0.5);
    background-color: hsla(0, 0%, 100%, 0.18);
}
.playlistItem .playBtn:disabled:hover {
    color: hsla(0, 0%, 100%, 0.5);
    background-color: hsla(0, 0%, 100%, 0.18);
}
.playlistItem .title {
    color: hsla(0, 0%, 100%, 0.85);
    font-weight: 500;
    font-size: 14px;
    margin-left: 4px;
    max-width: var(--card-w, 200px);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.playlistItem .subtitle {
    color: hsla(0, 0%, 100%, 0.4);
    font-weight: 500;
    font-size: 12px;
    margin-left: 4px;
    margin-top: -4px;
    max-width: var(--card-w, 200px);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

/* ── Tracks list (mirrors Catalog) ────────────────────────────── */
.tracksList {
    display: flex;
    flex-direction: column;
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
}
.trackElem:hover {
    background-color: hsla(0, 0%, 100%, 0.03);
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
}
.hoverOptionsBtn:hover {
    color: hsla(0, 0%, 100%, 0.7);
}
.trackElem:hover .hoverOptionsBtn,
.hoverOptionsBtn.menuOpen {
    opacity: 1;
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
}
.trackArtist:hover {
    color: hsla(0, 0%, 100%, 0.6);
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
.moreTracksBtn {
    align-self: flex-start;
    margin-top: 8px;
    margin-left: 8px;
    cursor: pointer;
    padding: 7px 18px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.8);
    font-size: 12px;
    font-weight: 600;
    transition: .2s ease;
}
.moreTracksBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
    color: hsla(0, 0%, 100%, 1);
}

/* ── Empty state ──────────────────────────────────────────────── */
.emptyState {
    flex: 1;
    display: flex;
    gap: 10px;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding-bottom: 40px;
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
}
.emptyText:hover {
    color: hsla(0, 0%, 100%, 0.8);
}
.emptyDesc {
    user-select: none;
    text-align: center;
    max-width: 320px;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.3);
}

.pageTitle {
    display: none;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .pageTitle {
        display: block !important;
        margin-top: 12px;
        margin-bottom: 22px !important;
        font-size: 30px !important;
        line-height: 1.08 !important;
        font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
        font-weight: 800;
        color: hsla(0, 0%, 100%, 0.9);
    }

    .searchPage.pageRoot {
        padding: 0 16px 80px !important;
    }

    .searchZone {
        height: 48px !important;
        border-radius: 14px !important;
        margin-bottom: 16px !important;
    }

    .search {
        height: 100% !important;
        padding: 0 44px 0 42px !important;
        border-radius: 14px !important;
        font-size: 14px !important;
    }

    .searchClear {
        width: 44px !important;
        height: 44px !important;
        right: 4px !important;
        padding: 0 !important;
        display: flex !important;
        align-items: center !important;
        justify-content: center !important;
    }

    .searchClear svg {
        width: 16px !important;
        height: 16px !important;
    }

    .pastSearchesBar {
        gap: 8px !important;
        margin-bottom: 20px !important;
    }

    .pastSearchChip {
        padding: 8px 14px !important;
        border-radius: 12px !important;
        font-size: 13px !important;
    }

    .lineList {
        width: 100vw !important;
        margin-left: -16px !important;
        margin-right: -16px !important;
        padding: 0 16px 12px !important;
        --card-w: 132px !important;
    }

    .lineControls {
        display: none !important;
    }

    .trackElem {
        height: 54px !important;
        min-height: 54px !important;
        padding: 5px 46px 5px 5px !important;
        border-radius: 10px !important;
        gap: 10px !important;
    }

    .trackCover {
        border-radius: 6px !important;
        width: 40px !important;
        height: 40px !important;
    }

    .trackCoverImg {
        width: 100% !important;
        height: 100% !important;
    }

    .playingAnim {
        left: 17px !important;
    }

    .trackTime {
        display: block !important;
        opacity: 1 !important;
        margin-right: 32px !important;
    }

    .hoverOptionsBtn {
        opacity: 1 !important;
    }

    .moreTracksBtn {
        align-self: stretch !important;
        width: 100% !important;
        margin-left: 0 !important;
        text-align: center !important;
        justify-content: center !important;
        box-sizing: border-box !important;
        height: 40px !important;
        display: flex !important;
        align-items: center !important;
    }

    .moreArtistsRow {
        width: 100% !important;
        display: flex !important;
        box-sizing: border-box !important;
    }

    .moreArtistsBtn {
        display: inline-flex !important;
        width: 100% !important;
        justify-content: center !important;
        height: 40px !important;
        box-sizing: border-box !important;
    }
}
</style>
