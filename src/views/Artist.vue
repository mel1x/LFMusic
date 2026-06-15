<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import MoreIcon from '../components/Icons/MoreIcon.vue'
import PlayIcon from '../components/Icons/PlayIcon.vue'
import PauseIcon from '../components/Icons/PauseIcon.vue'
import PinIcon from '../components/Icons/PinIcon.vue'
import EmptyIcon from '../components/Icons/EmptyIcon.vue'
import defaultTrackCover from '../assets/trackCover.webp'

import { usePlayer, type TrackMetadata } from '../composables/usePlayer'
import { usePlaylists } from '../composables/usePlaylists'
import { createRipple } from '../composables/useRipple'
import { isPhoneViewport } from '../composables/usePlatform'
import { tracksCountText, listenCountText } from '../utils/format'
import TrackContextMenu from '../components/TrackContextMenu.vue'
import PlaylistContextMenu from '../components/PlaylistContextMenu.vue'
import TrackArtists from '../components/TrackArtists.vue'
import { usePins } from '../composables/usePins'
import CoverEditor from '../components/CoverEditor.vue'
import AnimatedCover from '../components/AnimatedCover.vue'
import { useCustomCovers } from '../composables/useCustomCovers'
import {
    TRACK_STATISTICS_UPDATED_EVENT,
    sumListenCount,
    type TrackStatisticsUpdateEventDetail,
} from '../composables/useStatistics'

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
    getTrackByPath,
    cacheTracks,
} = usePlayer()

const { albums: albumPlaylists, artists: artistPlaylists, loadPlaylists, getPlaylistTracks } = usePlaylists()

const rawArtistName = computed(() => {
    const raw = Array.isArray(route.params.name) ? route.params.name[0] : route.params.name
    try { return decodeURIComponent(raw as string) } catch { return (raw as string) || '' }
})

// Плейлист исполнителя ищем регистронезависимо: бэкенд группирует артистов по
// нижнему регистру имени, поэтому «Pyrokinesis» и «pyrokinesis» — один артист.
const artistPlaylist = computed(() => {
    const lower = rawArtistName.value.toLowerCase()
    return artistPlaylists.value.find(p => p.name.toLowerCase() === lower)
})

// Каноничное имя артиста (как в плейлисте) — гарантирует один ник и одну
// аватарку независимо от регистра в ссылке. До загрузки плейлистов — исходное.
const artistName = computed(() => artistPlaylist.value?.name ?? rawArtistName.value)

// Треки исполнителя — из плейлиста исполнителя (бэкенд группирует по артисту).
const artistTracks = ref<TrackMetadata[]>([])
async function loadArtistTracks() {
    const pl = artistPlaylist.value
    if (!pl) { artistTracks.value = []; return }
    try {
        const tracks = await getPlaylistTracks(pl.id)
        cacheTracks(tracks)
        artistTracks.value = tracks
    } catch {
        artistTracks.value = []
    }
}
watch([artistPlaylist, artistName], loadArtistTracks, { immediate: true })

const sourceId = computed(() => `artist:${artistName.value}`)

const { getCustomCover: getCustomCoverFn } = useCustomCovers()

const artistCover = computed(() => {
    const custom = getCustomCoverFn('artist', artistName.value)
    if (custom) return custom
    const path = artistPlaylist.value?.first_file_path
    if (!path) return defaultTrackCover
    const t = getTrackByPath(path)
    return t?.cover_art || defaultTrackCover
})

function albumCoverFor(name: string, fallback: string | null): string {
    const custom = getCustomCoverFn('album', name)
    if (custom) return custom
    return fallback || defaultTrackCover
}

const previewTracks = computed(() => artistTracks.value.slice(0, 6))

// ── Albums of this artist (album playlists whose first track has this artist) ──

interface AlbumEntry {
    id: number
    name: string
    trackCount: number
    firstPath: string | null
    cover: string | null
}

const artistAlbums = computed<AlbumEntry[]>(() => {
    const albumNames = new Set<string>()
    for (const t of artistTracks.value) {
        if (t.album) albumNames.add(t.album.trim())
    }
    return albumPlaylists.value
        .filter(pl => albumNames.has(pl.name))
        .map(pl => {
            const t = pl.first_file_path
                ? getTrackByPath(pl.first_file_path)
                : undefined
            return {
                id: pl.id,
                name: pl.name,
                trackCount: pl.track_count,
                firstPath: pl.first_file_path,
                cover: t?.cover_art ?? null,
            }
        })
        .sort((a, b) => a.name.localeCompare(b.name))
})

const listenCount = ref(0)
const artistTrackPathsSignature = computed(() => artistTracks.value.map(t => t.file_path).join('\n'))
let listenCountRequestId = 0

async function refreshListenCount() {
    const requestId = ++listenCountRequestId
    const filePaths = artistTracks.value.map(t => t.file_path)
    if (filePaths.length === 0) {
        listenCount.value = 0
        return
    }

    try {
        const count = await sumListenCount(filePaths)
        if (requestId === listenCountRequestId) listenCount.value = count
    } catch (e) {
        console.error('Ошибка чтения статистики прослушиваний артиста:', e)
    }
}

function onTrackStatisticsUpdated(event: Event) {
    const detail = (event as CustomEvent<TrackStatisticsUpdateEventDetail>).detail
    if (!detail?.listenCountDelta) return
    if (!artistTracks.value.some(t => t.file_path === detail.filePath)) return
    void refreshListenCount()
}

watch(artistTrackPathsSignature, () => {
    void refreshListenCount()
}, { immediate: true })

// ── Cover prefetch for visible items ───────────────────────────────────────

function requestCovers() {
    const pending: TrackMetadata[] = []
    const headPath = artistPlaylist.value?.first_file_path
    if (headPath) {
        const t = getTrackByPath(headPath)
        if (t && !t.cover_art) pending.push(t)
    }
    for (const t of previewTracks.value) {
        if (!t.cover_art) pending.push(t)
    }
    for (const a of artistAlbums.value) {
        if (!a.firstPath) continue
        const t = getTrackByPath(a.firstPath)
        if (t && !t.cover_art) pending.push(t)
    }
    if (pending.length) loadCoversForTracks(pending)
}

onMounted(async () => {
    window.addEventListener(TRACK_STATISTICS_UPDATED_EVENT, onTrackStatisticsUpdated)
    if (albumPlaylists.value.length === 0) await loadPlaylists()
    await nextTick()
    requestCovers()
})

watch(() => artistTracks.value.length, () => requestCovers())
watch(artistName, () => requestCovers())

// ── Albums line scroll ─────────────────────────────────────────────────────

const albumsLineEl = ref<HTMLElement | null>(null)
const scrollLeft = ref(0)
const scrollMax = ref(0)
const itemWidth = ref(210)
const measured = ref(false)
let albumsRO: ResizeObserver | null = null
const ALBUMS_GAP = 16
const ALBUMS_BASE = 210

function measureAlbumsLine() {
    const node = albumsLineEl.value
    if (!node) return
    const cw = node.clientWidth
    if (cw > 0) {
        const n = Math.max(1, Math.floor((cw + ALBUMS_GAP) / (ALBUMS_BASE + ALBUMS_GAP)))
        itemWidth.value = (cw - (n - 1) * ALBUMS_GAP) / n
        measured.value = true
    }
    scrollLeft.value = node.scrollLeft
    scrollMax.value = Math.max(0, node.scrollWidth - node.clientWidth)
}

function onAlbumsScroll() {
    const node = albumsLineEl.value
    if (!node) return
    scrollLeft.value = node.scrollLeft
    scrollMax.value = Math.max(0, node.scrollWidth - node.clientWidth)
}

function scrollAlbumsBy(dir: 1 | -1) {
    const node = albumsLineEl.value
    if (!node) return
    node.scrollBy({ left: dir * node.clientWidth * 0.9, behavior: 'smooth' })
}

const canPrev = computed(() => scrollLeft.value > 1)
const canNext = computed(() => scrollLeft.value < scrollMax.value - 1)

watch(albumsLineEl, (el) => {
    if (!el) return
    measureAlbumsLine()
    if (!albumsRO) {
        albumsRO = new ResizeObserver(() => measureAlbumsLine())
        albumsRO.observe(el)
    }
})

watch(() => artistAlbums.value.length, async () => {
    await nextTick()
    measureAlbumsLine()
})

onUnmounted(() => {
    window.removeEventListener(TRACK_STATISTICS_UPDATED_EVENT, onTrackStatisticsUpdated)
    albumsRO?.disconnect()
    albumsRO = null
})

// ── Play artist ────────────────────────────────────────────────────────────

const isArtistActive = computed(() =>
    activePlaylistSource.value === sourceId.value
)

function onPlayArtist() {
    if (isArtistActive.value) {
        togglePlayPause()
        return
    }
    if (artistTracks.value.length === 0) return
    setPlaylistSource(sourceId.value, artistTracks.value)
    playTrack(artistTracks.value[0])
}

// ── Track interactions (preview list) ──────────────────────────────────────

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
    setPlaylistSource(sourceId.value, artistTracks.value)
    playTrack(track, { manualListStart: true })
}

// ── Track context menu ────────────────────────────────────────────────────

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

// ── Album interactions ────────────────────────────────────────────────────

const { isPinned: isPinnedFn, togglePin } = usePins()

function isAlbumPinned(name: string) {
    return isPinnedFn('album', name)
}

function toggleAlbumPin(name: string) {
    togglePin({ kind: 'album', key: name, name })
}

function onToggleAlbumPinClick(e: MouseEvent, album: AlbumEntry) {
    e.stopPropagation()
    toggleAlbumPin(album.name)
}

function isAlbumActive(album: AlbumEntry) {
    return activePlaylistSource.value === `album:${album.name}`
}

async function onPlayAlbum(e: MouseEvent, album: AlbumEntry) {
    e.stopPropagation()
    if (isAlbumActive(album)) {
        togglePlayPause()
        return
    }
    if (album.trackCount === 0) return
    const tracks = await getPlaylistTracks(album.id)
    if (tracks.length === 0) return
    const merged = tracks.map(t => getTrackByPath(t.file_path) ?? t)
    setPlaylistSource(`album:${album.name}`, merged)
    await playTrack(merged[0])
}

function openAlbum(album: AlbumEntry) {
    router.push(`/album/${encodeURIComponent(album.name)}`)
}

const albumMenu = ref<{ x: number; y: number; album: AlbumEntry; centerX: boolean } | null>(null)

function openAlbumMenu(e: MouseEvent, album: AlbumEntry) {
    e.stopPropagation()
    if (albumMenu.value?.album.name === album.name) {
        albumMenu.value = null
        return
    }
    const btn = e.currentTarget as HTMLElement
    const rect = btn.getBoundingClientRect()
    albumMenu.value = { x: rect.left + rect.width / 2, y: rect.bottom + 6, album, centerX: true }
}

function openAlbumMenuFromCover(e: MouseEvent, album: AlbumEntry) {
    e.preventDefault()
    e.stopPropagation()
    albumMenu.value = { x: e.clientX, y: e.clientY, album, centerX: false }
}

// ── Navigation ────────────────────────────────────────────────────────────

function goBack() {
    router.back()
}

function openArtistTracks() {
    router.push(`/artist/${encodeURIComponent(artistName.value)}/tracks`)
}

function openArtistAlbums() {
    router.push({ path: '/collections/albums', query: { q: artistName.value } })
}

// ── Artist page context menu (more button) ────────────────────────────────

const artistMenu = ref<{ x: number; y: number } | null>(null)
const isPinned = computed(() => isPinnedFn('artist', artistName.value))

function openArtistMenu(e: MouseEvent) {
    e.stopPropagation()
    if (artistMenu.value) {
        artistMenu.value = null
        return
    }
    const btn = e.currentTarget as HTMLElement
    const rect = btn.getBoundingClientRect()
    artistMenu.value = { x: rect.left + rect.width / 2, y: rect.bottom + 6 }
}

function onTogglePin() {
    togglePin({ kind: 'artist', key: artistName.value, name: artistName.value })
}

function onCoverImgClick() {
    if (isPhoneViewport()) return
    window.dispatchEvent(new CustomEvent('lfmusic:show-fullscreen-cover', { detail: artistCover.value }))
}
</script>

<template>
    <div class="pageRoot">
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
                    <div class="listType">Исполнитель</div>
                    <div class="listNameRow">
                        <h2 v-truncate class="listName">{{ artistName }}</h2>
                    </div>
                    <div class="badges">
                        <div class="badge" v-tooltip="'Количество треков исполнителя'">{{ tracksCountText(artistTracks.length) }}</div>
                        <div class="badge" v-tooltip="'Сколько раз была прослушана музыка этого исполнителя'">{{ listenCountText(listenCount) }}</div>
                    </div>
                </div>
                <div class="controls">
                    <button class="playListBtn" :disabled="artistTracks.length === 0" @click="onPlayArtist">
                        <PauseIcon v-if="isArtistActive && isPlaying" width="14" height="14" />
                        <PlayIcon v-else width="14" height="14" style="margin-left:0;" />
                        Слушать
                    </button>
                    <button class="moreBtn" :class="{ menuOpen: !!artistMenu }" @mousedown.stop="openArtistMenu($event)">
                        <MoreIcon width="22" height="22" />
                    </button>
                </div>
            </div>
        </div>

        <div class="pageLine">
            <div class="lineHat">
                <div class="lineTitle" @click="openArtistTracks">
                    Треки артиста
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                </div>
            </div>
            <div class="tracksList" v-if="previewTracks.length > 0">
                <div
                    v-for="track in previewTracks"
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
            <div v-else class="emptyRow">У этого исполнителя пока нет треков</div>
        </div>

        <div class="pageLine">
            <div class="lineHat">
                <div class="lineTitle" @click="openArtistAlbums">
                    Альбомы артиста
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                </div>
                <div class="lineControls" v-if="artistAlbums.length > 0">
                    <button class="prevBtn" :disabled="!canPrev" @click="scrollAlbumsBy(-1)"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                    <button class="nextBtn" :disabled="!canNext" @click="scrollAlbumsBy(1)"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                </div>
            </div>
            <div
                v-if="artistAlbums.length > 0"
                class="lineList"
                ref="albumsLineEl"
                :style="{ '--card-w': itemWidth + 'px', visibility: measured ? 'visible' : 'hidden' }"
                @scroll="onAlbumsScroll"
            >
                <div
                    v-for="album in artistAlbums"
                    :key="album.id"
                    class="playlistItem albumItem"
                    @click="openAlbum(album)"
                >
                    <div
                        class="coverWrap"
                        :class="{ menuOpen: albumMenu?.album.name === album.name }"
                        @contextmenu="openAlbumMenuFromCover($event, album)"
                    >
                        <AnimatedCover class="cover" :src="albumCoverFor(album.name, album.cover)" />
                        <button
                            class="overlayBtn pinBtn"
                            :class="{ active: isAlbumPinned(album.name) }"
                            @click="onToggleAlbumPinClick($event, album)"
                            @contextmenu.stop
                            v-tooltip="isAlbumPinned(album.name) ? 'Открепить' : 'Закрепить'"
                        >
                            <PinIcon width="16" height="16" />
                        </button>
                        <button
                            class="overlayBtn playBtn"
                            :disabled="album.trackCount === 0"
                            @click="onPlayAlbum($event, album)"
                            @contextmenu.stop
                            v-tooltip="isAlbumActive(album) && isPlaying ? 'Пауза' : 'Слушать'"
                        >
                            <PauseIcon v-if="isAlbumActive(album) && isPlaying" width="14" height="14" />
                            <PlayIcon v-else width="14" height="14" style="margin-left:1px;" />
                        </button>
                        <button
                            class="overlayBtn moreBtn cardMore"
                            :class="{ active: albumMenu?.album.name === album.name }"
                            @mousedown.stop="openAlbumMenu($event, album)"
                            @click.stop
                            @contextmenu.stop
                            v-tooltip="'Меню'"
                        >
                            <MoreIcon width="20" height="20" />
                        </button>
                    </div>
                    <div v-truncate class="title">{{ album.name }}</div>
                    <div v-truncate class="subtitle">{{ artistName }}</div>
                </div>
            </div>
            <div v-else class="emptyAlbums">
                <div class="emptyIcon"><EmptyIcon width="16" height="16" /></div>
                <div class="emptyText">Альбомов нет</div>
                <div class="emptyDesc">У этого исполнителя пока не сформированы альбомы.</div>
            </div>
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
        <PlaylistContextMenu
            v-if="albumMenu"
            :x="albumMenu.x"
            :y="albumMenu.y"
            :isPinned="isAlbumPinned(albumMenu.album.name)"
            :showDelete="false"
            :centerX="albumMenu.centerX"
            customCoverKind="album"
            :customCoverKey="albumMenu.album.name"
            @close="albumMenu = null"
            @togglePin="toggleAlbumPin(albumMenu!.album.name)"
        />
        <PlaylistContextMenu
            v-if="artistMenu"
            :x="artistMenu.x"
            :y="artistMenu.y"
            :isPinned="isPinned"
            :showDelete="false"
            customCoverKind="artist"
            :customCoverKey="artistName"
            centerX
            @close="artistMenu = null"
            @togglePin="onTogglePin"
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

    overflow-y: scroll;
}
.pageRoot::-webkit-scrollbar { width: 6px; }
.pageRoot::-webkit-scrollbar-track { background: transparent; }
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
    width: fit-content;
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
    background-color: #202020;
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
    gap: 8px;
    margin-top: 14px;
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
.moreBtn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 8px;
    border-radius: 999px;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.6);
    cursor: pointer;
    transition: .24s ease;
}
.moreBtn:hover,
.moreBtn.menuOpen {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: hsla(0, 0%, 100%, 1);
}

/* ── Lines (Tracks / Albums) ───────────────────────────────────────────── */

.pageLine {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 13px;
    margin-top: 36px;
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
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.85);
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 24px;

    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 1);
    }
}
.lineTitle svg {
    margin-top: 4px;
    display: block;
    color: hsla(0, 0%, 100%, 0.4);

    transition: .24s ease;
}
.lineTitle:hover svg {
    margin-left: 12px;
    color: hsla(0, 0%, 100%, 1);
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

    &:hover {
        border: 2px solid hsla(0, 0%, 100%, 0.4);
        color: hsla(0, 0%, 100%, 0.95);
    }
}
.lineControls button:disabled {
    &:hover {
        border: 2px solid hsla(0, 0%, 100%, 0.2);
        color: hsla(0, 0%, 100%, 0.6);
    }
}
.pageLine:hover .lineControls button {
    opacity: 1;
}
.pageLine:hover .lineControls button:disabled {
    opacity: 0.65;
}
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

/* ── Track list (preview) ────────────────────────────────────────────── */

.tracksList {
    margin-top: 0;
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

.emptyRow {
    user-select: none;
    padding: 30px 4px;
    color: hsla(0, 0%, 100%, 0.35);
    font-size: 14px;
    font-weight: 500;
}

/* ── Albums row card styles ──────────────────────────────────────────── */

.playlistItem {
    flex-shrink: 0;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
}
.playlistItem .coverWrap {
    position: relative;
    width: var(--card-w, 210px);
    height: var(--card-w, 210px);
    border-radius: 18px;
    overflow: hidden;
}
.playlistItem .cover {
    display: block;
    width: var(--card-w, 210px);
    height: var(--card-w, 210px);
    border-radius: 18px;
    object-fit: cover;
    transition: filter .24s ease, transform .24s ease;
    user-select: none;
}
.playlistItem:hover .cover,
.playlistItem .coverWrap.menuOpen .cover {
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
.playlistItem .overlayBtn:hover {
    background-color: hsla(0, 0%, 0%, 0.65);
    color: hsla(0, 0%, 100%, 1);
}
.playlistItem:hover .overlayBtn,
.playlistItem .coverWrap.menuOpen .overlayBtn,
.playlistItem .overlayBtn.active {
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
.playlistItem .pinBtn {
    top: 12px;
    right: 12px;
    padding: 8px;
}
.playlistItem .pinBtn.active {
    color: hsla(0, 0%, 100%, 1);
}
.playlistItem .pinBtn.active :deep(svg path) {
    fill: currentColor;
}
.playlistItem .cardMore {
    right: 12px;
    bottom: 12px;
    padding: 8px;
}
.playlistItem .cardMore.active {
    color: hsla(0, 0%, 100%, 1);
    background-color: hsla(0, 0%, 0%, 0.65);
}
.playlistItem .title {
    color: hsla(0, 0%, 100%, 0.8);
    font-weight: 500;
    font-size: 14px;
    margin-left: 4px;
    max-width: var(--card-w, 210px);
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
    max-width: var(--card-w, 210px);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.emptyAlbums {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 36px 0;
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
