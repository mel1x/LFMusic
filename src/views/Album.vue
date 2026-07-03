<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { goBackOrHome } from '../router'

import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import SearchIcon from '../components/Icons/SearchIcon.vue';
import EmptyIcon from '../components/Icons/EmptyIcon.vue';
import defaultTrackCover from '../assets/trackCover.webp';

import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { usePlaylists } from '../composables/usePlaylists';
import { createRipple } from '../composables/useRipple';
import { isPhoneViewport } from '../composables/usePlatform';
import { useVirtualScroll } from '../composables/useVirtualScroll';
import { searchRanked } from '../composables/useSearch';
import { tracksCountText } from '../utils/format';
import TrackContextMenu from '../components/TrackContextMenu.vue';
import TrackRow from '../components/TrackRow.vue';
import TrackArtists from '../components/TrackArtists.vue';
import CoverEditor from '../components/CoverEditor.vue';
import AnimatedCover from '../components/AnimatedCover.vue';
import LoadingSpinner from '../components/LoadingSpinner.vue';
import { useCustomCovers } from '../composables/useCustomCovers';
import { useCoverGate } from '../composables/useCoverGate';
import { useAlbumTrackOrder } from '../composables/useAlbumTrackOrder';
import { useExternalSources, type AlbumLayout, type AlbumLayoutEntry } from '../composables/useExternalSources';
import { useSettings } from '../composables/useSettings';
import { useDownloads } from '../composables/useDownloads';

const route = useRoute()
const router = useRouter()

const {
    currentTrack,
    isPlaying,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    setPlaylistSource,
    formatTime,
    loadCoversForTracks,
    clearCoverQueue,
    getTrackByPath,
    cacheTracks,
} = usePlayer();

const { albums: albumPlaylists, loadPlaylists, getPlaylistTracks } = usePlaylists();

const albumName = computed(() => {
    const raw = Array.isArray(route.params.name) ? route.params.name[0] : route.params.name
    try { return decodeURIComponent(raw as string) } catch { return (raw as string) || '' }
})

const { loadOrder: loadAlbumOrder, applyOrder: applyAlbumOrder, setOrder: setAlbumOrder } = useAlbumTrackOrder()

// Load saved order from DB on album open / route change.
watch(albumName, (name) => { if (name) loadAlbumOrder(name) }, { immediate: true })

const albumPlaylistForTracks = computed(() =>
    albumPlaylists.value.find(p => p.name === albumName.value) || null
)
const albumTracksRaw = ref<TrackMetadata[]>([])
const tracksLoadedFor = ref<string | null>(null)
async function loadAlbumTracks() {
    const name = albumName.value
    const pl = albumPlaylistForTracks.value
    if (!pl) { albumTracksRaw.value = []; return }
    try {
        const tracks = await getPlaylistTracks(pl.id)
        cacheTracks(tracks)
        if (albumName.value === name) {
            // Use live cache objects, not raw ones — otherwise covers loaded
            // earlier into the cache object won't show on the raw copy. See cacheTrack.
            albumTracksRaw.value = tracks.map(t => getTrackByPath(t.file_path) ?? t)
            tracksLoadedFor.value = name
        }
    } catch {
        if (albumName.value === name) {
            albumTracksRaw.value = []
            tracksLoadedFor.value = name
        }
    }
}
watch([albumPlaylistForTracks, albumName], loadAlbumTracks, { immediate: true })

const albumTracks = computed<TrackMetadata[]>(() =>
    applyAlbumOrder(albumName.value, albumTracksRaw.value)
)

const albumArtist = computed(() =>
    albumTracks.value[0]?.artist || 'Неизвестный исполнитель'
)

const sourceId = computed(() => `album:${albumName.value}`)

// External track order + phantom (not downloaded) tracks
const { settings } = useSettings()
const { getAlbumLayout } = useExternalSources()

const albumLayout = ref<AlbumLayout | null>(null)
const layoutSettledFor = ref<string | null>(null)

const layoutOk = computed(() =>
    albumLayout.value?.status === 'ok' && (albumLayout.value?.entries.length ?? 0) > 0
)
const externalOrderActive = computed(() => settings.value.external_album_order && layoutOk.value)
const supplementActive = computed(() => settings.value.external_supplement_missing && layoutOk.value)

// Phantom is clickable only if yt-dlp is installed and downloads page is enabled;
// otherwise it's purely informational.
const { ytdlpMissing } = useDownloads()
const phantomClickable = computed(() => !ytdlpMissing.value && settings.value.show_downloads_page)
const layoutNeeded = computed(() =>
    settings.value.external_album_order || settings.value.external_supplement_missing
)

async function refreshAlbumLayout() {
    const name = albumName.value
    albumLayout.value = null
    layoutSettledFor.value = null
    if (!layoutNeeded.value || !name) return
    const layout = await getAlbumLayout(name)
    // Guard against race: album may have changed while the request was in flight.
    if (albumName.value === name) {
        albumLayout.value = layout
        layoutSettledFor.value = name
    }
}
watch(
    [albumName, () => settings.value.external_album_order, () => settings.value.external_supplement_missing],
    refreshAlbumLayout,
    { immediate: true },
)

// First-paint gate: wait for tracks (and external layout, if enabled) to load
// before showing the album, so order/phantoms appear without a jump. Capped at 2.5s.
const PAGE_LOAD_TIMEOUT_MS = 2500

const tracksReady = computed(() => tracksLoadedFor.value === albumName.value)
const layoutReady = computed(() =>
    !layoutNeeded.value || layoutSettledFor.value === albumName.value
)

const pageReady = ref(false)
let pageReadyTimer: ReturnType<typeof setTimeout> | null = null

function clearPageReadyTimer() {
    if (pageReadyTimer) { clearTimeout(pageReadyTimer); pageReadyTimer = null }
}

watch(albumName, () => {
    pageReady.value = false
    clearPageReadyTimer()
    pageReadyTimer = setTimeout(() => { pageReady.value = true; pageReadyTimer = null }, PAGE_LOAD_TIMEOUT_MS)
}, { immediate: true })

watch([tracksReady, layoutReady], () => {
    if (!pageReady.value && tracksReady.value && layoutReady.value) {
        pageReady.value = true
        clearPageReadyTimer()
    }
}, { immediate: true })

// A row is either a real local track or a "phantom" (present in the canonical
// album but missing from the library — rendered semi-transparent).
interface AlbumRow {
    phantom: boolean
    key: string
    track: TrackMetadata | null
    title: string
    artist: string
    duration_secs: number | null
}

function filenameOf(t: TrackMetadata): string {
    return t.file_path.split(/[\\/]/).pop()?.replace(/\.[^/.]+$/, '') || ''
}

// Port of TITLE_NOISE_TOKENS from src-tauri/src/external.rs. A bracketed group
// containing any of these is stripped entirely; version tags (remix/live/…) stay.
const TITLE_NOISE_TOKENS = new Set([
    'feat', 'ft', 'featuring', 'prod', 'produced', 'official', 'lyric', 'lyrics',
    'visualizer', 'remaster', 'remastered', 'explicit', 'mv',
])

function groupIsNoise(buf: string): boolean {
    return buf
        .split(/[^\p{L}\p{N}]+/u)
        .some(w => w.length > 0 && TITLE_NOISE_TOKENS.has(w))
}

// Mirrors `external::title_match_key` = normalize(strip_title_noise(title)) on the
// backend. Must stay identical: a prior mismatch (frontend stripped only feat.
// markers, backend stripped ALL noise groups) left "Believer (Official Video)"
// unmatched against local "Believer", so a downloaded track stayed a phantom.
function albumNormalizeTitle(s: string): string {
    const lower = (s || '').toLowerCase()
    // 1) strip_title_noise: strip noise bracket groups char by char, keeping
    //    meaningful ones (remix/live/…).
    let stripped = ''
    let buf = ''
    let depth = 0
    for (const ch of lower) {
        if (ch === '(' || ch === '[' || ch === '{') {
            depth += 1
            buf += ch
        } else if ((ch === ')' || ch === ']' || ch === '}') && depth > 0) {
            buf += ch
            depth -= 1
            if (depth === 0) {
                if (!groupIsNoise(buf)) stripped += buf
                buf = ''
            }
        } else if (depth > 0) {
            buf += ch
        } else {
            stripped += ch
        }
    }
    // Unclosed bracket — keep the remainder if it's not noise.
    if (buf.length > 0 && !groupIsNoise(buf)) stripped += buf
    // Trailing credits outside brackets.
    for (const marker of [' feat.', ' feat ', ' ft.', ' ft ', ' featuring ', ' prod.', ' prod ']) {
        const i = stripped.indexOf(marker)
        if (i !== -1) stripped = stripped.slice(0, i)
    }
    // 2) normalize: letters/digits only, everything else collapses to single spaces.
    let out = ''
    let prevSpace = true
    for (const ch of stripped) {
        if (/[\p{L}\p{N}]/u.test(ch)) { out += ch; prevSpace = false }
        else if (!prevSpace) { out += ' '; prevSpace = true }
    }
    return out.replace(/\s+$/u, '')
}

function realRow(t: TrackMetadata): AlbumRow {
    return {
        phantom: false,
        key: t.file_path,
        track: t,
        title: t.title || filenameOf(t),
        artist: t.artist || '',
        duration_secs: t.duration_secs ?? null,
    }
}

function phantomRow(e: AlbumLayoutEntry, idx: number): AlbumRow {
    return {
        phantom: true,
        key: `phantom:${idx}:${e.disc}:${e.position}:${e.title}`,
        track: null,
        title: e.title,
        artist: e.artist,
        duration_secs: e.duration_secs,
    }
}

const rawByPath = computed(() => {
    const m = new Map<string, TrackMetadata>()
    for (const t of albumTracksRaw.value) m.set(t.file_path, t)
    return m
})

// Full ordered row list (before search filtering). Depends on two independent
// settings: external order (reorders local tracks) and supplement (shows phantoms).
const orderedRows = computed<AlbumRow[]>(() => {
    const layout = albumLayout.value
    const order = externalOrderActive.value
    const supplement = supplementActive.value

    if ((!order && !supplement) || !layout) {
        return albumTracks.value.map(realRow)
    }

    // Pool of local tracks by normalized title, so a just-downloaded track (not
    // yet in the cached layout by file_path) can take its canonical slot and
    // replace a phantom.
    const localByTitle = new Map<string, TrackMetadata[]>()
    for (const t of albumTracks.value) {
        const k = albumNormalizeTitle(t.title || filenameOf(t))
        if (!k) continue
        const arr = localByTitle.get(k)
        if (arr) arr.push(t); else localByTitle.set(k, [t])
    }
    const used = new Set<string>()
    // Match canon entry to a local track: exact file_path from layout first,
    // then normalized title (for freshly downloaded tracks).
    const matchLocal = (e: AlbumLayoutEntry): TrackMetadata | undefined => {
        if (e.file_path) {
            const t = rawByPath.value.get(e.file_path)
            if (t && !used.has(t.file_path)) return t
        }
        const cands = localByTitle.get(albumNormalizeTitle(e.title))
        if (cands) {
            for (const t of cands) if (!used.has(t.file_path)) return t
        }
        return undefined
    }

    if (order) {
        // Canonical order; phantoms only if supplement is enabled.
        const rows: AlbumRow[] = []
        for (const e of layout.entries) {
            const local = matchLocal(e)
            if (local) {
                used.add(local.file_path)
                rows.push(realRow(local))
            } else if (supplement) {
                rows.push(phantomRow(e, rows.length))
            }
        }
        // Local tracks not in canon (bonus tracks etc.) go at the end.
        for (const t of albumTracks.value) {
            if (!used.has(t.file_path)) rows.push(realRow(t))
        }
        return rows
    }

    // Supplement without reordering: local order + missing tracks appended.
    const rows = albumTracks.value.map(realRow)
    let idx = rows.length
    for (const e of layout.entries) {
        // Phantom only if no local track matches by path or title (otherwise a
        // freshly downloaded track would show twice).
        if (!matchLocal(e)) {
            rows.push(phantomRow(e, idx++))
        }
    }
    return rows
})

const { getCustomCover } = useCustomCovers()

// The canonical "first" track comes from the playlist, same as the grid card
// (PlaylistInfo.first_file_path) — keeps the header cover in sync with the grid.
const albumPlaylist = computed(() =>
    albumPlaylists.value.find(p => p.name === albumName.value) || null
)
const albumFirstPath = computed(() => albumPlaylist.value?.first_file_path ?? null)
const albumFirstTrack = computed<TrackMetadata | null>(() => {
    const path = albumFirstPath.value
    if (!path) return null
    return getTrackByPath(path) ?? null
})

const albumCover = computed(() => {
    const custom = getCustomCover('album', albumName.value)
    if (custom) return custom
    // Prefer the "first" track's cover; fall back to any loaded track cover,
    // then the default — keeps this in sync with the grid.
    return albumFirstTrack.value?.cover_art
        || albumTracks.value.find(t => t.cover_art)?.cover_art
        || defaultTrackCover
})

// Force-load the "first" track's cover, otherwise the header shows the default
// until the user scrolls that track into view.
watch(albumFirstTrack, (t) => {
    if (t && !t.cover_art) loadCoversForTracks([t])
}, { immediate: true })

// Playlists may not be loaded yet on direct URL navigation.
onMounted(() => {
    if (!albumPlaylists.value.length) loadPlaylists()
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

const displayRows = computed<AlbumRow[]>(() => {
    const q = debouncedSearch.value.trim()
    if (!q) return orderedRows.value
    return searchRanked(
        orderedRows.value,
        q,
        (r) => {
            const fileName = r.track ? filenameOf(r.track) : ''
            return [r.title || '', r.artist || '', fileName]
        },
        { formatPath: (r) => r.key },
    )
})

// Playable (real) tracks only — for the queue and the "Listen" button.
const playableTracks = computed<TrackMetadata[]>(() =>
    displayRows.value.filter(r => !r.phantom).map(r => r.track!)
)

// ── Header height ──

const scrollHeaderRef = ref<HTMLElement | null>(null)
const scrollHeaderHeight = ref(0)
let headerResizeObserver: ResizeObserver | null = null

onMounted(() => {
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
    clearPageReadyTimer()
})

// ── Virtual scroll ──

const {
    containerEl: trackListEl,
    onScroll,
    visibleItems: visibleTracks,
    paddingTop: vsPaddingTop,
    paddingBottom: vsPaddingBottom,
    resetScroll,
} = useVirtualScroll(displayRows, { itemHeight: 58, buffer: 5, scrollOffset: scrollHeaderHeight })

watch(debouncedSearch, () => resetScroll())
watch(albumName, () => resetScroll())

const isScrollingToTop = ref(false)

// Hold back first paint until the first 20 covers have resolved. Only armed once
// tracks are loaded — otherwise it'd open on an empty list (content would flash).
const { ready: coversReady } = useCoverGate(
    () => displayRows.value.filter(r => !r.phantom).slice(0, 20).map(r => r.track!),
    { armed: () => tracksReady.value },
)

watch(visibleTracks, (rows) => {
    if (!isScrollingToTop.value) {
        loadCoversForTracks(rows.filter(r => !r.phantom).map(r => r.track!))
    }
}, { immediate: true })

// ── Play button ──

const isAlbumActive = computed(() =>
    activePlaylistSource.value === sourceId.value
)

function onPlayAlbum() {
    if (isAlbumActive.value) {
        togglePlayPause()
    } else {
        if (playableTracks.value.length === 0) return
        setPlaylistSource(sourceId.value, playableTracks.value)
        playTrack(playableTracks.value[0])
    }
}

// ── Sticky header ──

const scrollY = ref(0)
const showStickyHeader = computed(() => scrollY.value > 300)

function handleScroll(e: Event) {
    scrollY.value = (e.target as HTMLElement).scrollTop
    if (isScrollingToTop.value && scrollY.value <= 10) {
        isScrollingToTop.value = false
        loadCoversForTracks(visibleTracks.value.filter(r => !r.phantom).map(r => r.track!))
    }
    onScroll(e)
}

function scrollToTop() {
    isScrollingToTop.value = true
    clearCoverQueue()
    trackListEl.value?.scrollTo({ top: 0, behavior: 'smooth' })
}

// ── Track click ──

function onRowClick(e: MouseEvent, row: AlbumRow) {
    if (row.phantom || !row.track) return
    if (isPhoneViewport()) {
        onCoverClick(row.track)
        return
    }
    createRipple(e)
}

// Phantom click: seed the downloads page search with "artist title" and navigate
// there. dlArtist/dlTitle are passed separately for an exact Deezer field search —
// a generic title (e.g. "Intro") would otherwise get lost in free-text search.
function onPhantomClick(row: AlbumRow) {
    if (!phantomClickable.value) return
    const q = [row.title, row.artist].map(s => (s || '').trim()).filter(Boolean).join(' ')
    router.push({ path: '/downloads', query: { q, dlArtist: row.artist || '', dlTitle: row.title || '' } })
}

function onCoverClick(track: TrackMetadata) {
    if (isAlbumActive.value && currentTrack.value?.file_path === track.file_path) {
        togglePlayPause()
        return
    }
    setPlaylistSource(sourceId.value, playableTracks.value)
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
    const btn = (e.currentTarget as HTMLElement)
    const rect = btn.getBoundingClientRect()
    trackMenu.value = { x: rect.left - 6, y: rect.top + rect.height / 2, track }
}

// ── Move track to top of album ──

async function moveTrackToTopOfAlbum(track: TrackMetadata) {
    const paths = albumTracks.value.map(t => t.file_path)
    const idx = paths.indexOf(track.file_path)
    if (idx <= 0) return
    paths.splice(idx, 1)
    paths.unshift(track.file_path)
    await setAlbumOrder(albumName.value, paths)
    if (isAlbumActive.value) {
        // Rebuild the queue source from the reordered album tracks.
        setPlaylistSource(sourceId.value, albumTracks.value.slice())
    }
}

// ── Back ──

function goBack() {
    goBackOrHome(router)
}

function onCoverImgClick() {
    if (isPhoneViewport()) return
    window.dispatchEvent(new CustomEvent('lfmusic:show-fullscreen-cover', { detail: { src: albumCover.value } }))
}
</script>

<template>
    <div class="pageRoot" ref="trackListEl" @scroll="handleScroll">
        <div class="stickyHeaderAnchor">
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div>{{ albumName || 'Альбом' }}</div>
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
                    <AnimatedCover :src="albumCover" class="cover" alt="cover" />
                    <CoverEditor kind="album" :coverKey="albumName" size="large" />
                </div>
                <div class="dataZone">
                    <div class="info">
                        <div class="listType">Альбом</div>
                        <div class="listNameRow">
                            <h2 v-truncate class="listName">{{ albumName }}</h2>
                        </div>
                        <div class="badges" v-if="albumTracks.length > 0">
                            <div v-truncate class="badge" v-tooltip="albumArtist">{{ albumArtist }}</div>
                            <div class="badge" v-tooltip="'Количество треков в альбоме'">{{ tracksCountText(albumTracks.length) }}</div>
                        </div>
                        <div class="badges" v-else>
                            <div class="badge" v-tooltip="'Альбом пуст — добавте треки'">Пусто</div>
                        </div>
                    </div>
                    <div class="controls">
                        <button class="playListBtn" :disabled="albumTracks.length === 0" @click="onPlayAlbum">
                            <PauseIcon v-if="isAlbumActive && isPlaying" width="14" height="14" />
                            <PlayIcon v-else width="14" height="14" style="margin-left:0;" />
                            Слушать
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
        <Transition name="gate-fade" mode="out-in">
        <LoadingSpinner v-if="!pageReady || (displayRows.length > 0 && !coversReady)" key="gate-spinner" />
        <div
            class="tracksList"
            v-else-if="displayRows.length > 0"
            key="gate-list"
        >
            <div class="vsPad" :style="{ height: vsPaddingTop + 'px' }"></div>
            <template v-for="row in visibleTracks" :key="row.key">
                <!-- Phantom (not downloaded) track: semi-transparent, clickable
                     to search the downloads page if yt-dlp is installed. -->
                <div
                    v-if="row.phantom"
                    class="trackElem phantom"
                    :class="{ downloadable: phantomClickable }"
                    @click="onPhantomClick(row)"
                    v-tooltip="phantomClickable ? 'Найти и скачать этот трек' : ''"
                >
                    <div class="trackCover phantomCover">
                        <img :src="albumCover" alt="" draggable="false" class="trackCoverImg" />
                        <div v-if="phantomClickable" class="phantomDlIcon">
                            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
                        </div>
                    </div>
                    <div class="trackInfo">
                        <div v-truncate class="trackName">{{ row.title }}</div>
                        <div class="trackArtist"><TrackArtists :artist="row.artist" /></div>
                    </div>
                    <div class="phantomBadge" v-tooltip="'Трека нет в вашей библиотеке'">
                        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="2" y1="2" x2="22" y2="22"/><path d="M5.782 5.782A7 7 0 0 0 9 19h8.5a4.5 4.5 0 0 0 1.307-.193"/><path d="M21.532 16.5A4.5 4.5 0 0 0 17.5 10h-1.79A7.008 7.008 0 0 0 10 5.07"/></svg>
                    </div>
                    <div class="trackTime">{{ row.duration_secs ? formatTime(row.duration_secs) : '' }}</div>
                </div>
                <TrackRow
                    v-else
                    :track="row.track!"
                    :active="isAlbumActive && currentTrack?.file_path === row.track!.file_path"
                    :activePaused="isAlbumActive && currentTrack?.file_path === row.track!.file_path && !isPlaying"
                    :menuOpen="trackMenu?.track.file_path === row.track!.file_path"
                    @rowClick="onRowClick($event, row)"
                    @coverClick="onCoverClick(row.track!)"
                    @openMenu="openTrackMenu($event, row.track!)"
                />
            </template>
            <div class="vsPad" :style="{ height: vsPaddingBottom + 'px' }"></div>
        </div>
        <div class="emptyState" v-else-if="debouncedSearch && displayRows.length === 0" key="gate-empty-search">
            <div class="emptyIcon"><SearchIcon width="16" height="16" /></div>
            <div class="emptyText">Ничего не найдено</div>
            <div class="emptyDesc">Попробуйте изменить критерии поиска.</div>
        </div>
        <div class="emptyState" v-else key="gate-empty">
            <div class="emptyIcon"><EmptyIcon width="16" height="16" /></div>
            <div class="emptyText">В этом альбоме нет треков</div>
            <div class="emptyDesc">Альбом формируется автоматически из вашей библиотеки.</div>
        </div>
        </Transition>
        <TrackContextMenu
            v-if="trackMenu"
            :key="trackMenu.track.file_path"
            :x="trackMenu.x"
            :y="trackMenu.y"
            :track="trackMenu.track"
            :canMoveToTop="!externalOrderActive"
            alignRight
            centerY
            @close="trackMenu = null"
            @moveToTop="moveTrackToTopOfAlbum"
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

/* ── Phantom (not downloaded) tracks ── */
/* Don't fade the whole row (would be confused with the active-track look, which
   also dims the cover) — fade the cover more, text less, so text stays readable. */
.trackElem.phantom {
    cursor: default;
}
.trackElem.phantom:hover {
    background-color: hsla(0, 0%, 100%, 0.02);
}
.trackElem.phantom .trackCover {
    background-color: #1a1a1a;
    opacity: 0.25;
}
.trackElem.phantom .trackName,
.trackElem.phantom .trackArtist,
.trackElem.phantom .trackTime,
.trackElem.phantom .phantomBadge {
    opacity: 0.55;
}
.phantomBadge {
    display: flex;
    align-items: center;
    justify-content: center;
    color: hsla(0, 0%, 100%, 0.4);
    flex-shrink: 0;
    margin-left: auto;
    margin-right: 4px;
}
.trackElem.phantom .trackTime {
    margin-left: 0;
}
/* Clickable phantom (yt-dlp available): hint that the track can be downloaded. */
.trackElem.phantom.downloadable {
    cursor: pointer;
}
.trackElem.phantom.downloadable:hover {
    background-color: hsla(0, 0%, 100%, 0.04);
}
.trackElem.phantom.downloadable:hover .trackName,
.trackElem.phantom.downloadable:hover .trackArtist {
    opacity: 0.85;
}
.phantomCover {
    position: relative;
}
.phantomDlIcon {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    background-color: hsla(0, 0%, 0%, 0.45);
    opacity: 0;
    transition: opacity .2s ease;
}
.trackElem.phantom.downloadable:hover .phantomDlIcon {
    opacity: 1;
}
.trackElem.phantom.downloadable:hover .trackCover {
    opacity: 0.5;
}
</style>
