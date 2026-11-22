<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { goBackOrHome } from '../router'

import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import MoreIcon from '../components/Icons/MoreIcon.vue';
import ShuffleIcon from '../components/Icons/ShuffleIcon.vue';
import TimeIcon from '../components/Icons/TimeIcon.vue';
import defaultTrackCover from '../assets/trackCover.webp';

import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { usePlaylists } from '../composables/usePlaylists';
import { createRipple } from '../composables/useRipple';
import { isPhoneViewport } from '../composables/usePlatform';
import { useVirtualScroll } from '../composables/useVirtualScroll';
import { tracksCountText, formatTotalDuration } from '../utils/format';
import { useCoverTint } from '../composables/useCoverTint';
import TrackContextMenu from '../components/TrackContextMenu.vue';
import PlaylistContextMenu from '../components/PlaylistContextMenu.vue';
import { usePins } from '../composables/usePins';
import TrackRow from '../components/TrackRow.vue';
import TrackArtists from '../components/TrackArtists.vue';
import CoverEditor from '../components/CoverEditor.vue';
import AnimatedCover from '../components/AnimatedCover.vue';
import MobileScrollHeader from '../components/MobileScrollHeader.vue';
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
    shuffled,
    toggleShuffle,
} = usePlayer();

const { albums: albumPlaylists, loadPlaylists, getPlaylistTracks } = usePlaylists();

const albumName = computed(() => {
    const raw = Array.isArray(route.params.name) ? route.params.name[0] : route.params.name
    try { return decodeURIComponent(raw as string) } catch { return (raw as string) || '' }
})

const { loadOrder: loadAlbumOrder, applyOrder: applyAlbumOrder, setOrder: setAlbumOrder } = useAlbumTrackOrder()

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

const { settings } = useSettings()
const { getAlbumLayout } = useExternalSources()

const albumLayout = ref<AlbumLayout | null>(null)
const layoutSettledFor = ref<string | null>(null)

const layoutOk = computed(() =>
    albumLayout.value?.status === 'ok' && (albumLayout.value?.entries.length ?? 0) > 0
)
const externalOrderActive = computed(() => settings.value.external_album_order && layoutOk.value)
const supplementActive = computed(() => settings.value.external_supplement_missing && layoutOk.value)

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

// Port of TITLE_NOISE_TOKENS from src-tauri/src/external.rs.
const TITLE_NOISE_TOKENS = new Set([
    'feat', 'ft', 'featuring', 'prod', 'produced', 'official', 'lyric', 'lyrics',
    'visualizer', 'remaster', 'remastered', 'explicit', 'mv',
])

function groupIsNoise(buf: string): boolean {
    return buf
        .split(/[^\p{L}\p{N}]+/u)
        .some(w => w.length > 0 && TITLE_NOISE_TOKENS.has(w))
}

// Mirrors external::title_match_key; must stay identical, or downloaded tracks stay phantoms.
function albumNormalizeTitle(s: string): string {
    const lower = (s || '').toLowerCase()
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
    if (buf.length > 0 && !groupIsNoise(buf)) stripped += buf
    for (const marker of [' feat.', ' feat ', ' ft.', ' ft ', ' featuring ', ' prod.', ' prod ']) {
        const i = stripped.indexOf(marker)
        if (i !== -1) stripped = stripped.slice(0, i)
    }
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

const orderedRows = computed<AlbumRow[]>(() => {
    const layout = albumLayout.value
    const order = externalOrderActive.value
    const supplement = supplementActive.value

    if ((!order && !supplement) || !layout) {
        return albumTracks.value.map(realRow)
    }

    const localByTitle = new Map<string, TrackMetadata[]>()
    for (const t of albumTracks.value) {
        const k = albumNormalizeTitle(t.title || filenameOf(t))
        if (!k) continue
        const arr = localByTitle.get(k)
        if (arr) arr.push(t); else localByTitle.set(k, [t])
    }
    const used = new Set<string>()
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
        for (const t of albumTracks.value) {
            if (!used.has(t.file_path)) rows.push(realRow(t))
        }
        return rows
    }

    const rows = albumTracks.value.map(realRow)
    let idx = rows.length
    for (const e of layout.entries) {
        if (!matchLocal(e)) {
            rows.push(phantomRow(e, idx++))
        }
    }
    return rows
})

const { getCustomCover } = useCustomCovers()

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
    return albumFirstTrack.value?.cover_art
        || albumTracks.value.find(t => t.cover_art)?.cover_art
        || defaultTrackCover
})

const coverTintRgb = useCoverTint(() => albumCover.value)
const heroStyle = computed(() => coverTintRgb.value ? { '--hero-tint': coverTintRgb.value } : {})

const totalDuration = computed(() =>
    albumTracks.value.reduce((sum, t) => sum + (t.duration_secs ?? 0), 0)
)

watch(albumFirstTrack, (t) => {
    if (t && !t.cover_art) loadCoversForTracks([t])
}, { immediate: true })

onMounted(() => {
    if (!albumPlaylists.value.length) loadPlaylists()
})

import { menuAnchorBelow } from '../utils/menu'
const displayRows = orderedRows

const playableTracks = computed<TrackMetadata[]>(() =>
    displayRows.value.filter(r => !r.phantom).map(r => r.track!)
)

const rankOf = computed(() => new Map(orderedRows.value.map((r, i) => [r.key, i + 1])))

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

const {
    containerEl: trackListEl,
    onScroll,
    visibleItems: visibleTracks,
    paddingTop: vsPaddingTop,
    paddingBottom: vsPaddingBottom,
    resetScroll,
} = useVirtualScroll(displayRows, { itemHeight: 64, buffer: 5, scrollOffset: scrollHeaderHeight })

watch(albumName, () => resetScroll())

const isScrollingToTop = ref(false)

const { ready: coversReady } = useCoverGate(
    () => displayRows.value.filter(r => !r.phantom).slice(0, 20).map(r => r.track!),
    { armed: () => tracksReady.value },
)

watch(visibleTracks, (rows) => {
    if (!isScrollingToTop.value) {
        loadCoversForTracks(rows.filter(r => !r.phantom).map(r => r.track!))
    }
}, { immediate: true })

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

const isShuffleActive = computed(() => isAlbumActive.value && shuffled.value)

function onShuffleAlbum() {
    const list = playableTracks.value
    if (list.length === 0) return
    if (isAlbumActive.value) {
        toggleShuffle()
        return
    }
    if (!shuffled.value) toggleShuffle()
    setPlaylistSource(sourceId.value, list)
    playTrack(list[Math.floor(Math.random() * list.length)], { manualListStart: true })
}

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

function onRowClick(e: MouseEvent, row: AlbumRow) {
    if (row.phantom || !row.track) return
    if (isPhoneViewport()) {
        onCoverClick(row.track)
        return
    }
    createRipple(e)
}

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

const { isPinned: isPinnedFn, togglePin } = usePins()
const isPinned = computed(() => isPinnedFn('album', albumName.value))
const albumMenu = ref<{ x: number; y: number } | null>(null)

function openAlbumMenu(e: MouseEvent) {
    e.stopPropagation()
    if (albumMenu.value) {
        albumMenu.value = null
        return
    }
    albumMenu.value = { ...menuAnchorBelow(e) }
}

function onTogglePin() {
    togglePin({ kind: 'album', key: albumName.value, name: albumName.value })
}

async function moveTrackToTopOfAlbum(track: TrackMetadata) {
    const paths = albumTracks.value.map(t => t.file_path)
    const idx = paths.indexOf(track.file_path)
    if (idx <= 0) return
    paths.splice(idx, 1)
    paths.unshift(track.file_path)
    await setAlbumOrder(albumName.value, paths)
    if (isAlbumActive.value) {
        setPlaylistSource(sourceId.value, albumTracks.value.slice())
    }
}

function goBack() {
    goBackOrHome(router)
}

function onCoverImgClick() {
    if (isPhoneViewport()) return
    window.dispatchEvent(new CustomEvent('lfmusic:show-fullscreen-cover', { detail: { src: albumCover.value } }))
}
</script>

<template>
    <div class="pageRoot mobileHeroPage" ref="trackListEl" :style="heroStyle" @scroll="handleScroll">
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader :title="albumName || 'Альбом'" :scrolled="scrollY > 16" @top="scrollToTop" />
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div v-truncate class="stickyTitle">{{ albumName || 'Альбом' }}</div>
                    <button class="toTopBtn" @click="scrollToTop">Вверх</button>
                </div>
            </Transition>
        </div>
        <div ref="scrollHeaderRef" class="scrollHeader">
            <button class="backBtn" @click="goBack">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>
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
                        <div v-if="albumTracks.length > 0" class="heroArtist">
                            <TrackArtists :artist="albumArtist" />
                        </div>
                        <div class="badges">
                            <template v-if="albumTracks.length > 0">
                                <span class="badge">{{ tracksCountText(albumTracks.length) }}</span>
                                <span v-if="totalDuration > 0" class="badge">{{ formatTotalDuration(totalDuration) }}</span>
                            </template>
                            <span v-else class="badge">Пусто</span>
                        </div>
                    </div>
                    <div class="controls">
                        <button
                            class="playListBtn"
                            :disabled="albumTracks.length === 0"
                            :aria-label="isAlbumActive && isPlaying ? 'Пауза' : 'Слушать'"
                            @click="onPlayAlbum"
                        >
                            <PauseIcon v-if="isAlbumActive && isPlaying" width="22" height="22" />
                            <PlayIcon v-else width="22" height="22" />
                        </button>
                        <button
                            class="iconBtn shuffleBtn"
                            :class="{ on: isShuffleActive }"
                            :disabled="albumTracks.length === 0"
                            v-tooltip="'Перемешать'"
                            @click="onShuffleAlbum"
                        >
                            <ShuffleIcon width="20" height="20" />
                        </button>
                        <button class="iconBtn moreBtn" :class="{ menuOpen: !!albumMenu }" v-tooltip="'Меню'" @mousedown.stop="openAlbumMenu($event)">
                            <MoreIcon width="22" height="22" />
                        </button>
                    </div>
                </div>
            </div>
            <div class="listHead">
                <span class="colRank">#</span>
                <span class="colTitle">Название</span>
                <span class="colTime"><TimeIcon width="13" height="13" /></span>
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
                <div
                    v-if="row.phantom"
                    class="trackElem phantom"
                    :class="{ downloadable: phantomClickable }"
                    @click="onPhantomClick(row)"
                    v-tooltip="phantomClickable ? 'Найти и скачать этот трек' : ''"
                >
                    <span class="phantomRank">{{ rankOf.get(row.key) }}</span>
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
                    large
                    :rank="rankOf.get(row.key)"
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
        <div class="emptyState" v-else key="gate-empty">
            <div class="emptyText">В этом альбоме нет треков</div>
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
        <PlaylistContextMenu
            v-if="albumMenu"
            :x="albumMenu.x"
            :y="albumMenu.y"
            :isPinned="isPinned"
            :showDelete="false"
            customCoverKind="album"
            :customCoverKey="albumName"
            centerX
            @close="albumMenu = null"
            @togglePin="onTogglePin"
        />
    </div>
</template>

<style scoped>
.pageRoot {
    --hero-tint: 120, 120, 120;
    padding: 12px 32px 48px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: var(--light-bg);
    background-image: linear-gradient(
        to bottom,
        rgba(var(--hero-tint), 0.42) 0px,
        rgba(var(--hero-tint), 0.14) 240px,
        rgba(var(--hero-tint), 0) 460px
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
.backBtn {
    display: flex;
    align-items: center;
    gap: 8px;
    width: fit-content;
    padding: 6px 14px 6px 10px;
    margin: 0 0 8px -10px;
    border-radius: 999px;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.8);
    font-size: 14px;
    font-weight: 400;
    cursor: pointer;
    user-select: none;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.backBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 1);
}
.backBtn svg {
    transition: transform var(--duration-hover) var(--ease-smooth);
}
.backBtn:hover svg {
    transform: translateX(-2px);
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
.heroArtist {
    margin-top: 10px;
    max-width: 100%;
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.94);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
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
.listHead {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 36px;
    margin-top: 36px;
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
.trackElem {
    position: relative;
    cursor: pointer;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 12px;
    padding: 0 16px;
    border-radius: 12px;
    transition: background-color var(--duration-hover) var(--ease-smooth);
    height: 64px;
    box-sizing: border-box;
    overflow: hidden;
}
.phantomRank {
    width: 22px;
    flex-shrink: 0;
    text-align: center;
    font-size: 14px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.42);
    user-select: none;
}
:global(body.lfmusic-phone .phantomRank) {
    display: none;
}
.trackCover {
    width: 48px;
    height: 48px;
    background-color: #202020;
    border-radius: 6px;
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
    color: hsla(0, 0%, 100%, 0.94);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.trackArtist {
    user-select: none;
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.4);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.trackTime {
    width: 44px;
    text-align: right;
    user-select: none;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.45);
    flex-shrink: 0;
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

.trackElem.phantom {
    cursor: default;
}
.trackElem.phantom .trackCover {
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
}
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
    transition: opacity var(--duration-hover) var(--ease-smooth);
}
.trackElem.phantom.downloadable:hover .phantomDlIcon {
    opacity: 1;
}
.trackElem.phantom.downloadable:hover .trackCover {
    opacity: 0.5;
}
</style>
