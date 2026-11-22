<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { goBackOrHome } from '../router'

import MoreIcon from '../components/Icons/MoreIcon.vue'
import PlayIcon from '../components/Icons/PlayIcon.vue'
import PauseIcon from '../components/Icons/PauseIcon.vue'
import PinIcon from '../components/Icons/PinIcon.vue'
import ShuffleIcon from '../components/Icons/ShuffleIcon.vue'
import defaultTrackCover from '../assets/trackCover.webp'

import { usePlayer, type TrackMetadata } from '../composables/usePlayer'
import { usePlaySource } from '../composables/usePlaySource'
import { usePlaylists } from '../composables/usePlaylists'
import { createRipple } from '../composables/useRipple'
import { isPhoneViewport } from '../composables/usePlatform'
import { tracksCountText, pluralizeRu } from '../utils/format'
import { useI18n } from '../composables/useI18n'
import TrackContextMenu from '../components/TrackContextMenu.vue'
import PlaylistContextMenu from '../components/PlaylistContextMenu.vue'
import TrackRow from '../components/TrackRow.vue'
import { usePins } from '../composables/usePins'
import CoverEditor from '../components/CoverEditor.vue'
import AnimatedCover from '../components/AnimatedCover.vue'
import MobileScrollHeader from '../components/MobileScrollHeader.vue'
import { useCustomCovers } from '../composables/useCustomCovers'
import { useExternalSources } from '../composables/useExternalSources'
import { menuAnchorBelow } from '../utils/menu'
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
    loadCoversForTracks,
    getTrackByPath,
    cacheTracks,
    shuffled,
    toggleShuffle,
} = usePlayer()
const { playSource } = usePlaySource()

const { albums: albumPlaylists, artists: artistPlaylists, loadPlaylists, getPlaylistTracks } = usePlaylists()

const rawArtistName = computed(() => {
    const raw = Array.isArray(route.params.name) ? route.params.name[0] : route.params.name
    try { return decodeURIComponent(raw as string) } catch { return (raw as string) || '' }
})

const artistPlaylist = computed(() => {
    const lower = rawArtistName.value.toLowerCase()
    return artistPlaylists.value.find(p => p.name.toLowerCase() === lower)
})

const artistName = computed(() => artistPlaylist.value?.name ?? rawArtistName.value)

const artistTracks = ref<TrackMetadata[]>([])
async function loadArtistTracks() {
    const pl = artistPlaylist.value
    if (!pl) { artistTracks.value = []; return }
    try {
        const tracks = await getPlaylistTracks(pl.id)
        cacheTracks(tracks)
        artistTracks.value = tracks.map(t => getTrackByPath(t.file_path) ?? t)
    } catch {
        artistTracks.value = []
    }
}
watch([artistPlaylist, artistName], loadArtistTracks, { immediate: true })

const sourceId = computed(() => `artist:${artistName.value}`)

const { getCustomCover: getCustomCoverFn } = useCustomCovers()
const { avatarFor } = useExternalSources()

const artistCover = computed(() => {
    const custom = getCustomCoverFn('artist', artistName.value)
    if (custom) return custom
    const external = avatarFor(artistName.value)
    if (external) return external
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

const PREVIEW_COUNT = 6
const MORE_STEP = 20
const shownCount = ref(PREVIEW_COUNT)
const previewTracks = computed(() => artistTracks.value.slice(0, shownCount.value))
const hiddenCount = computed(() => Math.max(0, artistTracks.value.length - shownCount.value))

function showMoreTracks() {
    shownCount.value += MORE_STEP
}

watch(rawArtistName, () => { shownCount.value = PREVIEW_COUNT })

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
watch(shownCount, () => requestCovers())
watch(artistName, () => requestCovers())

onUnmounted(() => {
    window.removeEventListener(TRACK_STATISTICS_UPDATED_EVENT, onTrackStatisticsUpdated)
})

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

const isShuffleActive = computed(() => isArtistActive.value && shuffled.value)

function onShuffleArtist() {
    const list = artistTracks.value
    if (list.length === 0) return
    if (isArtistActive.value) {
        toggleShuffle()
        return
    }
    if (!shuffled.value) toggleShuffle()
    setPlaylistSource(sourceId.value, list)
    playTrack(list[Math.floor(Math.random() * list.length)], { manualListStart: true })
}

const { language } = useI18n()
const listensLabel = computed(() => language.value === 'en'
    ? (listenCount.value === 1 ? 'listen' : 'listens')
    : pluralizeRu(listenCount.value, ['прослушивание', 'прослушивания', 'прослушиваний']))
const tracksLabel = computed(() => tracksCountText(artistTracks.value.length).replace(/^\d+\s*/, ''))

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
    await playSource(`album:${album.name}`, album.id)
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
    albumMenu.value = { ...menuAnchorBelow(e), album, centerX: true }
}

function openAlbumMenuFromCover(e: MouseEvent, album: AlbumEntry) {
    e.preventDefault()
    e.stopPropagation()
    albumMenu.value = { x: e.clientX, y: e.clientY, album, centerX: false }
}

const pageRootEl = ref<HTMLElement | null>(null)
const scrollY = ref(0)

function onPageScroll(e: Event) {
    scrollY.value = (e.target as HTMLElement).scrollTop
}

function scrollToTop() {
    pageRootEl.value?.scrollTo({ top: 0, behavior: 'smooth' })
}

function goBack() {
    goBackOrHome(router)
}

function openArtistAlbums() {
    router.push({ path: '/collections/albums', query: { q: artistName.value } })
}

const artistMenu = ref<{ x: number; y: number } | null>(null)
const isPinned = computed(() => isPinnedFn('artist', artistName.value))

function openArtistMenu(e: MouseEvent) {
    e.stopPropagation()
    if (artistMenu.value) {
        artistMenu.value = null
        return
    }
    artistMenu.value = { ...menuAnchorBelow(e) }
}

function onTogglePin() {
    togglePin({ kind: 'artist', key: artistName.value, name: artistName.value })
}

function onCoverImgClick() {
    if (isPhoneViewport()) return
    window.dispatchEvent(new CustomEvent('lfmusic:show-fullscreen-cover', { detail: { src: artistCover.value } }))
}
</script>

<template>
    <div class="pageRoot mobileHeroPage" ref="pageRootEl" @scroll="onPageScroll">
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader :title="artistName || 'Исполнитель'" :scrolled="scrollY > 16" @top="scrollToTop" />
        </div>
        <div class="artistLayout">
            <aside class="artistAside scrollHeader">
                <button class="backBtn" @click="goBack">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>
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
                                <span class="badge">
                                    <b>{{ listenCount }}</b>
                                    <span>{{ listensLabel }}</span>
                                </span>
                                <span class="badge">
                                    <b>{{ artistTracks.length }}</b>
                                    <span>{{ tracksLabel }}</span>
                                </span>
                            </div>
                        </div>
                        <div class="controls">
                            <button
                                class="playListBtn"
                                :disabled="artistTracks.length === 0"
                                :aria-label="isArtistActive && isPlaying ? 'Пауза' : 'Слушать'"
                                @click="onPlayArtist"
                            >
                                <PauseIcon v-if="isArtistActive && isPlaying" width="22" height="22" />
                                <PlayIcon v-else width="22" height="22" />
                            </button>
                            <button
                                class="iconBtn shuffleBtn"
                                :class="{ on: isShuffleActive }"
                                :disabled="artistTracks.length === 0"
                                v-tooltip="'Перемешать'"
                                @click="onShuffleArtist"
                            >
                                <ShuffleIcon width="20" height="20" />
                            </button>
                            <button class="iconBtn moreBtn" :class="{ menuOpen: !!artistMenu }" v-tooltip="'Меню'" @mousedown.stop="openArtistMenu($event)">
                                <MoreIcon width="22" height="22" />
                            </button>
                        </div>
                    </div>
                </div>
            </aside>

            <div class="artistMain">
                <section class="pageLine">
                    <div class="tracksList" v-if="previewTracks.length > 0">
                        <TrackRow
                            v-for="(track, i) in previewTracks"
                            :key="track.file_path"
                            large
                            :rank="i + 1"
                            :track="track"
                            :active="currentTrack?.file_path === track.file_path"
                            :activePaused="currentTrack?.file_path === track.file_path && !isPlaying"
                            :menuOpen="trackMenu?.track.file_path === track.file_path"
                            @rowClick="onTrackClick($event, track)"
                            @coverClick="onCoverClick(track)"
                            @openMenu="openTrackMenu($event, track)"
                        />
                    </div>
                    <div v-else class="emptyRow">У этого исполнителя пока нет треков</div>
                    <button v-if="hiddenCount > 0" class="showMoreBtn" @click="showMoreTracks">
                        <span class="showMoreIcon">
                            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>
                        </span>
                        <span class="showMoreText">Показать больше</span>
                        <span class="showMoreCount">{{ hiddenCount }}</span>
                    </button>
                </section>

                <section v-if="artistAlbums.length > 0" class="pageLine">
                    <div class="lineHat">
                        <h3 class="lineTitle">Альбомы</h3>
                        <button class="seeAllBtn" @click="openArtistAlbums">
                            Все
                            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                        </button>
                    </div>
                    <div class="albumGrid">
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
                            <div class="subtitle">{{ tracksCountText(album.trackCount) }}</div>
                        </div>
                    </div>
                </section>
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
    padding: 12px 32px 48px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: var(--light-bg);
    overflow-y: scroll;
}
.pageRoot::-webkit-scrollbar { width: 6px; }
.pageRoot::-webkit-scrollbar-track { background: transparent; }
.pageRoot::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
}

.artistLayout {
    display: flex;
    align-items: flex-start;
    gap: 40px;
    width: 100%;
}
.artistAside {
    position: sticky;
    top: 0;
    flex-shrink: 0;
    width: clamp(240px, 28%, 320px);
    display: flex;
    flex-direction: column;
}
.artistMain {
    flex: 1;
    min-width: 0;
    padding-top: 44px;
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
    flex-direction: column;
    gap: 16px;
    width: 100%;
}
.pageHat .coverWrap {
    position: relative;
    width: 100%;
    aspect-ratio: 1;
    border-radius: 16px;
    box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.35);
    cursor: pointer;
}
.pageHat .cover {
    user-select: none;
    width: 100%;
    height: 100%;
    background-color: #202020;
    border-radius: 16px;
    overflow: hidden;
    object-fit: cover;
    transition: filter .28s var(--ease-smooth);
}
.pageHat .dataZone {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
}
.pageHat .dataZone .info {
    display: flex;
    flex-direction: column;
    min-width: 0;
}
.pageHat .dataZone .info .listType {
    display: none;
}
:global(body.lfmusic-phone .pageHat .dataZone .info .listType) {
    display: block;
}
.pageHat .dataZone .info .listName {
    cursor: default;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 36px;
    font-weight: 900;
    letter-spacing: -0.02em;
    line-height: 1.1;
    color: hsla(0, 0%, 100%, 0.96);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    overflow: hidden;
    overflow-wrap: anywhere;
}
.pageHat .dataZone .info .badges {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 12px;
}
.badge {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    padding: 8px 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.05);
    user-select: none;
    white-space: nowrap;
}
.badge b {
    font-size: 16px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.96);
}
.badge span {
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.6);
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

.pageLine {
    width: 100%;
    display: flex;
    flex-direction: column;
}
.pageLine + .pageLine {
    margin-top: 40px;
}
.lineHat {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 12px;
}
.lineTitle {
    margin: 0;
    user-select: none;
    font-size: 18px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.96);
}
.seeAllBtn {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 8px 4px 10px;
    border-radius: 999px;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.6);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: color var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth);
}
.seeAllBtn:hover {
    color: hsla(0, 0%, 100%, 1);
    background-color: hsla(0, 0%, 100%, 0.06);
}
.seeAllBtn svg {
    transition: transform var(--duration-hover) var(--ease-smooth);
}
.seeAllBtn:hover svg {
    transform: translateX(2px);
}

.tracksList {
    display: flex;
    flex-direction: column;
}
.emptyRow {
    user-select: none;
    padding: 32px 16px;
    color: hsla(0, 0%, 100%, 0.45);
    font-size: 14px;
}
.showMoreBtn {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    margin-top: 12px;
    padding: 12px 16px 12px 12px;
    border-radius: 16px;
    background-color: hsla(0, 0%, 100%, 0.04);
    color: hsla(0, 0%, 100%, 0.94);
    cursor: pointer;
    user-select: none;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.showMoreBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.07);
}
.showMoreIcon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 10px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
}
.showMoreText {
    flex: 1;
    text-align: left;
    font-size: 14px;
    font-weight: 600;
}
.showMoreCount {
    padding: 2px 10px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.08);
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.7);
}

.albumGrid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 24px 16px;
}
.playlistItem {
    min-width: 0;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 4px;
}
.playlistItem .coverWrap {
    position: relative;
    width: 100%;
    aspect-ratio: 1;
    margin-bottom: 8px;
    border-radius: 12px;
    overflow: hidden;
}
.playlistItem .cover {
    display: block;
    width: 100%;
    height: 100%;
    border-radius: 12px;
    object-fit: cover;
    transition: filter var(--duration-hover) var(--ease-smooth);
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
    transition: opacity var(--duration-hover) var(--ease-smooth), transform var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
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
    left: 10px;
    bottom: 10px;
    padding: 12px;
    color: var(--light-bg);
    background-color: hsla(0, 0%, 100%, 0.96);
}
.playlistItem .playBtn:hover {
    color: var(--light-bg);
    background-color: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
}
.playlistItem .playBtn:disabled {
    cursor: default;
    color: hsla(0, 0%, 100%, 0.5);
    background-color: hsla(0, 0%, 100%, 0.18);
}
.playlistItem .pinBtn {
    top: 10px;
    right: 10px;
    padding: 8px;
}
.playlistItem .pinBtn.active {
    color: hsla(0, 0%, 100%, 1);
}
.playlistItem .pinBtn.active :deep(svg path) {
    fill: currentColor;
}
.playlistItem .cardMore {
    right: 10px;
    bottom: 10px;
    padding: 8px;
}
.playlistItem .cardMore.active {
    color: hsla(0, 0%, 100%, 1);
    background-color: hsla(0, 0%, 0%, 0.65);
}
.playlistItem .title {
    color: hsla(0, 0%, 100%, 0.94);
    font-weight: 500;
    font-size: 14px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.playlistItem .subtitle {
    color: hsla(0, 0%, 100%, 0.45);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

:global(body.lfmusic-phone .artistLayout) {
    display: block;
}
:global(body.lfmusic-phone .artistAside) {
    position: static;
    width: 100%;
}
:global(body.lfmusic-phone .artistMain) {
    padding-top: 0;
}
:global(body.lfmusic-phone .albumGrid) {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 18px 12px;
}
</style>
