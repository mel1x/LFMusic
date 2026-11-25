<script setup lang="ts">
import { ref, shallowRef, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { goBackOrHome } from '../router'

import MoreIcon from '../components/Icons/MoreIcon.vue';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import ShuffleIcon from '../components/Icons/ShuffleIcon.vue';
import TimeIcon from '../components/Icons/TimeIcon.vue';
import EditIcon from '../components/Icons/EditIcon.vue';
import defaultPlaylistCover from '../assets/playlistCover.webp';
import favoritesCoverAsset from '../assets/favorites.webp';

import { invoke } from '@tauri-apps/api/core';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { usePlaylists, type PlaylistInfo } from '../composables/usePlaylists';
import { createRipple } from '../composables/useRipple';
import { isPhoneViewport } from '../composables/usePlatform';
import { useVirtualScroll } from '../composables/useVirtualScroll';
import { tracksCountText, formatTotalDuration } from '../utils/format';
import { useCoverTint } from '../composables/useCoverTint';
import TrackContextMenu from '../components/TrackContextMenu.vue';
import PlaylistContextMenu from '../components/PlaylistContextMenu.vue';
import FavoritesCover from '../components/FavoritesCover.vue';
import ConfirmModal from '../components/ConfirmModal.vue';
import TrackRow, { TRACK_ROW_HEIGHT } from '../components/TrackRow.vue';
import { usePins } from '../composables/usePins';
import CoverEditor from '../components/CoverEditor.vue';
import AnimatedCover from '../components/AnimatedCover.vue';
import MobileScrollHeader from '../components/MobileScrollHeader.vue';
import LoadingSpinner from '../components/LoadingSpinner.vue';
import { useCustomCovers } from '../composables/useCustomCovers';
import { useCoverGate } from '../composables/useCoverGate';

import TuneIcon from '../components/Icons/TuneIcon.vue'
import PlaylistTransition from '../components/PlaylistTransition.vue'
import { playlistMix, togglePlaylistMix, savedTransition } from '../composables/usePlaylistMix'
import { useI18n } from '../composables/useI18n'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()

const {
    getTrackByPath,
    peekTrackByPath,
    cacheTracks,
    currentTrack,
    isPlaying,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    setPlaylistSource,
    loadCoversForTracks,
    clearCoverQueue,
    shuffled,
    toggleShuffle,
} = usePlayer();

const { playlists, loadPlaylists, getPlaylistTracks, removeTrackFromPlaylist, deletePlaylist, renamePlaylist, isFavorite } = usePlaylists();

const playlistId = computed(() => {
    const raw = Array.isArray(route.params.id) ? route.params.id[0] : route.params.id
    return parseInt(raw as string, 10)
})

const playlist = computed<PlaylistInfo | undefined>(() =>
    playlists.value.find(p => p.id === playlistId.value)
)

const isFavorites = computed(() => playlist.value?.kind === 'Favorites')

const playlistTrackPaths = ref<string[]>([])
// Own copies: a playlist bigger than the shared cache would refetch and evict forever.
const ownTracks = shallowRef(new Map<string, TrackMetadata>())

async function reloadPlaylistTracks() {
    if (!Number.isFinite(playlistId.value)) {
        playlistTrackPaths.value = []
        return
    }
    try {
        const tracks = await getPlaylistTracks(playlistId.value)
        cacheTracks(tracks)
        ownTracks.value = new Map(tracks.map(t => [t.file_path, t]))
        playlistTrackPaths.value = tracks.map(t => t.file_path)
    } catch (e) {
        console.error('Ошибка загрузки треков плейлиста:', e)
        ownTracks.value = new Map()
        playlistTrackPaths.value = []
    }
}

const playlistTracks = computed<TrackMetadata[]>(() => {
    const out: TrackMetadata[] = []
    const own = ownTracks.value
    for (const p of playlistTrackPaths.value) {
        const t = peekTrackByPath(p) ?? own.get(p)
        if (t) out.push(t)
    }
    return out
})

const sourceId = computed(() => `playlist:${playlistId.value}`)

const { getCustomCover } = useCustomCovers()

const firstCoverTrack = computed(() => {
    const firstPath = playlistTrackPaths.value[0] ?? playlist.value?.first_file_path
    return firstPath ? getTrackByPath(firstPath) : undefined
})

const playlistCover = computed(() => {
    const custom = getCustomCover('playlist', playlistId.value)
    if (custom) return custom
    return firstCoverTrack.value?.cover_art || defaultPlaylistCover
})

const coverTintRgb = useCoverTint(() => isFavorites.value ? favoritesCoverAsset : playlistCover.value)
const heroStyle = computed(() => coverTintRgb.value ? { '--hero-tint': coverTintRgb.value } : {})

import { menuAnchorBelow } from '../utils/menu'
const baseTracks = computed(() =>
    isFavorites.value ? playlistTracks.value.filter(t => isFavorite(t.file_path)) : playlistTracks.value
)

const totalDuration = computed(() =>
    baseTracks.value.reduce((sum, t) => sum + (t.duration_secs ?? 0), 0)
)

const rankOf = computed(() => new Map(baseTracks.value.map((t, i) => [t.file_path, i + 1])))

const filteredTracks = baseTracks
const mixing = computed(() => playlist.value?.kind === 'User' && playlistMix(sourceId.value).enabled)
const editingPair = ref<{ from: TrackMetadata, to: TrackMetadata } | null>(null)
watch(playlistId, () => { editingPair.value = null })
function nextAfter(track: TrackMetadata) {
    return filteredTracks.value[rankOf.value.get(track.file_path) ?? filteredTracks.value.length]
}
function editTransition(track: TrackMetadata) {
    const next = nextAfter(track)
    if (next) editingPair.value = { from: track, to: next }
}
function transitionLabel(track: TrackMetadata) {
    const next = nextAfter(track)
    const settings = next && savedTransition(sourceId.value, track.file_path, next.file_path)
    return t('playlistMix.' + (settings ? settings.preset ?? 'custom' : 'auto'))
}
const scrollHeaderRef = ref<HTMLElement | null>(null)
const scrollHeaderHeight = ref(0)
let headerResizeObserver: ResizeObserver | null = null

const dataReady = ref(false)

onMounted(async () => {
    if (scrollHeaderRef.value) {
        scrollHeaderHeight.value = scrollHeaderRef.value.offsetHeight
        headerResizeObserver = new ResizeObserver(() => {
            if (scrollHeaderRef.value) {
                scrollHeaderHeight.value = scrollHeaderRef.value.offsetHeight
            }
        })
        headerResizeObserver.observe(scrollHeaderRef.value)
    }
    if (!playlists.value.length) await loadPlaylists()
    await reloadPlaylistTracks()
    dataReady.value = true
})

function onPlaylistTrackMovedToTop(e: Event) {
    const { playlistId: evtId } = (e as CustomEvent).detail as { playlistId: number }
    if (evtId === playlistId.value) reloadPlaylistTracks()
}

onMounted(() => {
    window.addEventListener('lfmusic:playlist-track-moved-to-top', onPlaylistTrackMovedToTop)
})

onUnmounted(() => {
    headerResizeObserver?.disconnect()
    window.removeEventListener('lfmusic:playlist-track-moved-to-top', onPlaylistTrackMovedToTop)
})

watch(playlistId, async () => {
    dataReady.value = false
    resetScroll()
    await reloadPlaylistTracks()
    dataReady.value = true
})

watch(firstCoverTrack, (track) => {
    if (track && !track.cover_art) {
        loadCoversForTracks([track])
    }
}, { immediate: true })

const {
    containerEl: trackListEl,
    onScroll,
    visibleItems: visibleTracks,
    paddingTop: vsPaddingTop,
    paddingBottom: vsPaddingBottom,
    resetScroll,
} = useVirtualScroll(filteredTracks, { itemHeight: computed(() => TRACK_ROW_HEIGHT + (mixing.value ? 32 : 0)), buffer: 5, scrollOffset: scrollHeaderHeight })


const isScrollingToTop = ref(false)

const { ready: coversReady } = useCoverGate(() => filteredTracks.value.slice(0, 20), {
    armed: () => dataReady.value,
})

watch(visibleTracks, (tracks) => {
    if (!isScrollingToTop.value) {
        loadCoversForTracks(tracks)
    }
}, { immediate: true })

const isPlaylistActive = computed(() =>
    activePlaylistSource.value === sourceId.value
)

function onPlayPlaylist() {
    if (isPlaylistActive.value) {
        togglePlayPause()
    } else {
        if (filteredTracks.value.length === 0) return
        setPlaylistSource(sourceId.value, filteredTracks.value)
        playTrack(filteredTracks.value[0])
    }
}

const isShuffleActive = computed(() => isPlaylistActive.value && shuffled.value)

function onShufflePlaylist() {
    const list = filteredTracks.value
    if (list.length === 0) return
    if (isPlaylistActive.value) {
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

function onCoverClick(track: TrackMetadata) {
    if (isPlaylistActive.value && currentTrack.value?.file_path === track.file_path) {
        togglePlayPause()
        return
    }
    setPlaylistSource(sourceId.value, filteredTracks.value)
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

async function onRemoveFromPlaylist(track: TrackMetadata) {
    const idx = playlistTrackPaths.value.indexOf(track.file_path)
    if (idx !== -1) playlistTrackPaths.value.splice(idx, 1)
    const ok = await removeTrackFromPlaylist(playlistId.value, track.file_path)
    if (!ok && idx !== -1 && !playlistTrackPaths.value.includes(track.file_path)) {
        playlistTrackPaths.value.splice(Math.min(idx, playlistTrackPaths.value.length), 0, track.file_path)
    }
}

const canMoveToTop = computed(() => true)

async function moveTrackToTopOfPlaylist(track: TrackMetadata) {
    const idx = playlistTrackPaths.value.indexOf(track.file_path)
    if (idx <= 0) return
    playlistTrackPaths.value.splice(idx, 1)
    playlistTrackPaths.value.unshift(track.file_path)
    await invoke('playlist_move_track_to_top', { playlistId: playlistId.value, filePath: track.file_path })
    if (isPlaylistActive.value) {
        setPlaylistSource(sourceId.value, playlistTracks.value.slice())
    }
    window.dispatchEvent(new CustomEvent('lfmusic:playlist-track-moved-to-top', { detail: { playlistId: playlistId.value } }))
}

function goBack() {
    goBackOrHome(router)
}

const playlistMenu = ref<{ x: number; y: number } | null>(null)
const { isPinned: isPinnedFn, togglePin, updatePinName, unpin } = usePins()
const isPinned = computed(() => isPinnedFn('playlist', playlistId.value))

function openPlaylistMenu(e: MouseEvent) {
    e.stopPropagation()
    if (playlistMenu.value) {
        playlistMenu.value = null
        return
    }
    playlistMenu.value = { ...menuAnchorBelow(e) }
}

function onTogglePin() {
    if (!playlist.value) return
    togglePin({ kind: 'playlist', key: playlistId.value, name: playlist.value.name })
}

watch(
    () => playlist.value?.name,
    (name) => {
        if (name) updatePinName('playlist', playlistId.value, name)
    },
)

const deleteModalOpen = ref(false)

function onDeletePlaylist() {
    deleteModalOpen.value = true
}

async function onConfirmDeletePlaylist() {
    deleteModalOpen.value = false
    const id = playlistId.value
    unpin('playlist', id)
    router.push('/collections')
    await deletePlaylist(id)
}

const isEditingName = ref(false)
const editingName = ref('')
const nameInputEl = ref<HTMLInputElement | null>(null)

async function startEditName() {
    if (!playlist.value) return
    editingName.value = playlist.value.name
    isEditingName.value = true
    await nextTick()
    nameInputEl.value?.focus()
    nameInputEl.value?.select()
}

async function commitEditName() {
    if (!isEditingName.value) return
    const name = editingName.value.trim()
    isEditingName.value = false
    if (!name || !playlist.value || name === playlist.value.name) return
    await renamePlaylist(playlistId.value, name)
}

function cancelEditName() {
    isEditingName.value = false
}

function onNameKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
        e.preventDefault()
        ;(e.target as HTMLInputElement).blur()
    } else if (e.key === 'Escape') {
        e.preventDefault()
        cancelEditName()
    }
}

function onCoverImgClick() {
    if (isPhoneViewport()) return
    if (isFavorites.value) {
        window.dispatchEvent(new CustomEvent('lfmusic:show-fullscreen-cover', { detail: { src: favoritesCoverAsset, tinted: true } }))
        return
    }
    window.dispatchEvent(new CustomEvent('lfmusic:show-fullscreen-cover', { detail: { src: playlistCover.value } }))
}
</script>

<template>
    <div class="pageRoot mobileHeroPage" ref="trackListEl" :style="heroStyle" @scroll="handleScroll">
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader :title="playlist?.name || 'Плейлист'" :scrolled="scrollY > 16" @top="scrollToTop" />
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div v-truncate class="stickyTitle">{{ playlist?.name || 'Плейлист' }}</div>
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
                <div class="coverWrap" :class="{ 'cover-edit-host': !isFavorites }" @click="onCoverImgClick">
                    <FavoritesCover v-if="isFavorites" class="cover" />
                    <template v-else>
                        <AnimatedCover :src="playlistCover" class="cover" alt="cover" />
                        <CoverEditor kind="playlist" :coverKey="playlistId" size="large" />
                    </template>
                </div>
                <div class="dataZone">
                    <div class="info">
                        <div class="listType">Плейлист</div>
                        <div class="listNameRow">
                            <h2 v-if="!isEditingName" v-truncate class="listName" @dblclick="!isFavorites && startEditName()">{{ playlist?.name || '' }}</h2>
                            <input
                                v-else
                                ref="nameInputEl"
                                class="listName listNameInput"
                                type="text"
                                v-model="editingName"
                                maxlength="120"
                                spellcheck="false"
                                @blur="commitEditName"
                                @keydown="onNameKeydown"
                            />
                            <button v-if="!isEditingName && !isFavorites" class="iconBtn editNameBtn" @click="startEditName" v-tooltip="'Переименовать'">
                                <EditIcon width="16" height="16" />
                            </button>
                        </div>
                        <div class="badges">
                            <template v-if="baseTracks.length > 0">
                                <span class="badge">{{ tracksCountText(baseTracks.length) }}</span>
                                <span v-if="totalDuration > 0" class="badge">{{ formatTotalDuration(totalDuration) }}</span>
                            </template>
                            <span v-else class="badge">Пусто</span>
                        </div>
                    </div>
                    <div class="controls">
                        <button
                            class="playListBtn staticPlaybackBtn"
                            :disabled="baseTracks.length === 0"
                            :aria-label="isPlaylistActive && isPlaying ? 'Пауза' : 'Слушать'"
                            @click="onPlayPlaylist"
                        >
                            <PauseIcon v-if="isPlaylistActive && isPlaying" width="22" height="22" />
                            <PlayIcon v-else width="22" height="22" />
                        </button>
                        <button
                            class="iconBtn shuffleBtn"
                            :class="{ on: isShuffleActive }"
                            :disabled="baseTracks.length === 0"
                            v-tooltip="'Перемешать'"
                            @click="onShufflePlaylist"
                        >
                            <ShuffleIcon width="20" height="20" />
                        </button>
                        <button v-if="playlist?.kind === 'User' && baseTracks.length > 1" class="iconBtn shuffleBtn" :class="{ on: mixing }" :aria-pressed="mixing" :aria-label="t('playlistMix.mode')" v-tooltip="t('playlistMix.mode')" @click="togglePlaylistMix(sourceId)"><TuneIcon /></button>
                        <button class="iconBtn moreBtn" :class="{ menuOpen: !!playlistMenu }" v-tooltip="'Меню'" @mousedown.stop="openPlaylistMenu($event)">
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
        <LoadingSpinner v-if="filteredTracks.length > 0 && !coversReady" key="gate-spinner" />
        <div
            class="tracksList"
            v-else-if="filteredTracks.length > 0"
            key="gate-list"
        >
            <div class="vsPad" :style="{ height: vsPaddingTop + 'px' }"></div>
            <template v-for="track in visibleTracks" :key="track.file_path">
            <TrackRow
                large
                :rank="rankOf.get(track.file_path)"
                :track="track"
                :active="isPlaylistActive && currentTrack?.file_path === track.file_path"
                :activePaused="isPlaylistActive && currentTrack?.file_path === track.file_path && !isPlaying"
                :menuOpen="trackMenu?.track.file_path === track.file_path"
                @rowClick="onTrackClick($event, track)"
                @coverClick="onCoverClick(track)"
                @openMenu="openTrackMenu($event, track)"
            />
            <div v-if="mixing" class="mixRow">
                <button v-if="nextAfter(track)" :aria-label="t('playlistMix.title') + ': ' + transitionLabel(track)" @click="editTransition(track)"><TuneIcon width="12" height="12" />{{ transitionLabel(track) }}</button>
            </div>
            </template>
            <div class="vsPad" :style="{ height: vsPaddingBottom + 'px' }"></div>
        </div>
        <div class="emptyState" v-else key="gate-empty">
            <div class="emptyText">В плейлисте пока нет треков</div>
            <button class="emptyAction" @click="router.push('/catalog')">Добавить треки</button>
        </div>
        </Transition>
        <PlaylistTransition v-if="editingPair" :source="sourceId" v-bind="editingPair" @close="editingPair = null" />
        <TrackContextMenu
            v-if="trackMenu"
            :key="trackMenu.track.file_path"
            :x="trackMenu.x"
            :y="trackMenu.y"
            :track="trackMenu.track"
            :canMoveToTop="canMoveToTop"
            :hideRemoveFromPlaylist="isFavorites"
            alignRight
            centerY
            inPlaylist
            @close="trackMenu = null"
            @removeFromPlaylist="onRemoveFromPlaylist"
            @moveToTop="moveTrackToTopOfPlaylist"
        />
        <PlaylistContextMenu
            v-if="playlistMenu"
            :x="playlistMenu.x"
            :y="playlistMenu.y"
            :isPinned="isPinned"
            :showDelete="!isFavorites"
            :customCoverKind="isFavorites ? undefined : 'playlist'"
            :customCoverKey="isFavorites ? undefined : playlistId"
            centerX
            @close="playlistMenu = null"
            @togglePin="onTogglePin"
            @delete="onDeletePlaylist"
        />
        <ConfirmModal
            :open="deleteModalOpen"
            title="Удалить плейлист?"
            :message="playlist ? `Плейлист «${playlist.name}» будет удалён без возможности восстановления.` : ''"
            confirmText="Удалить"
            cancelText="Отмена"
            @confirm="onConfirmDeletePlaylist"
            @cancel="deleteModalOpen = false"
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
    align-items: flex-start;
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
    align-self: center;
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    gap: 24px;
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
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.55);
}
.pageHat .dataZone .info .listNameRow {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin-top: 6px;
    max-width: 100%;
}
.pageHat .dataZone .info .listName {
    cursor: default;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: clamp(32px, 7cqi, 40px);
    font-weight: 800;
    letter-spacing: -0.02em;
    line-height: 1.1;
    color: hsla(0, 0%, 100%, 0.96);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    overflow: hidden;
    overflow-wrap: anywhere;
}
.pageHat .dataZone .info .listNameInput {
    cursor: text;
    background-color: hsla(0, 0%, 100%, 0.06);
    border: 1px solid hsla(0, 0%, 100%, 0.12);
    border-radius: 8px;
    padding: 2px 10px;
    outline: none;
    min-width: 0;
    width: 100%;
    max-width: 100%;
    transition: border-color var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth);
}
.pageHat .dataZone .info .listNameInput:focus {
    background-color: hsla(0, 0%, 100%, 0.09);
    border-color: hsla(0, 0%, 100%, 0.25);
}
.editNameBtn {
    flex-shrink: 0;
    margin-top: 2px;
    opacity: 0;
}
.listNameRow:hover .editNameBtn {
    opacity: 1;
}
.pageHat .dataZone .info .badges {
    display: flex;
    flex-wrap: wrap;
    margin-top: 8px;
    max-width: 100%;
}
.badge {
    user-select: none;
    font-size: 13px;
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
    gap: 12px;
}
.pageHat .dataZone .controls .playListBtn {
    width: 46px;
    height: 46px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.96);
    color: var(--light-bg);
    cursor: pointer;
    user-select: none;
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
.mixRow { height:32px; display:flex; align-items:center; position:relative; padding-left:calc(16px + 22px + 12px); }
.mixRow:has(button)::before { content:''; position:absolute; left:27px; top:1px; bottom:1px; width:2px; border-radius:2px; background:#ffffff26; }
:global(body.lfmusic-phone .mixRow) { padding-left:5px; }
:global(body.lfmusic-phone .mixRow:has(button)::before) { left:0; }
.mixRow button { display: inline-flex; align-items: center; gap: 6px; height: 26px; background: var(--dark-bg); border: 0; color: #fffb; cursor: pointer; border-radius: 14px; padding: 0 10px; font-size: 12px; }
.mixRow button:hover { background: #ffffff0e; color: white; }
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
.emptyAction {
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
.emptyAction:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
}
</style>
