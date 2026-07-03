<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { goBackOrHome } from '../router'

import MoreIcon from '../components/Icons/MoreIcon.vue';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import SearchIcon from '../components/Icons/SearchIcon.vue';
import EmptyIcon from '../components/Icons/EmptyIcon.vue';
import EditIcon from '../components/Icons/EditIcon.vue';
import defaultPlaylistCover from '../assets/playlistCover.webp';

import { invoke } from '@tauri-apps/api/core';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { usePlaylists, type PlaylistInfo } from '../composables/usePlaylists';
import { createRipple } from '../composables/useRipple';
import { isPhoneViewport } from '../composables/usePlatform';
import { useVirtualScroll } from '../composables/useVirtualScroll';
import { searchRanked } from '../composables/useSearch';
import { tracksCountText } from '../utils/format';
import TrackContextMenu from '../components/TrackContextMenu.vue';
import PlaylistContextMenu from '../components/PlaylistContextMenu.vue';
import ConfirmModal from '../components/ConfirmModal.vue';
import TrackRow from '../components/TrackRow.vue';
import { usePins } from '../composables/usePins';
import CoverEditor from '../components/CoverEditor.vue';
import AnimatedCover from '../components/AnimatedCover.vue';
import MobileScrollHeader from '../components/MobileScrollHeader.vue';
import LoadingSpinner from '../components/LoadingSpinner.vue';
import { useCustomCovers } from '../composables/useCustomCovers';
import { useCoverGate } from '../composables/useCoverGate';

const route = useRoute()
const router = useRouter()

const {
    getTrackByPath,
    cacheTracks,
    currentTrack,
    isPlaying,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    setPlaylistSource,
    loadCoversForTracks,
    clearCoverQueue,
} = usePlayer();

const { playlists, loadPlaylists, getPlaylistTracks, removeTrackFromPlaylist, deletePlaylist, renamePlaylist } = usePlaylists();

const playlistId = computed(() => {
    const raw = Array.isArray(route.params.id) ? route.params.id[0] : route.params.id
    return parseInt(raw as string, 10)
})

const playlist = computed<PlaylistInfo | undefined>(() =>
    playlists.value.find(p => p.id === playlistId.value)
)

const playlistTrackPaths = ref<string[]>([])

async function reloadPlaylistTracks() {
    if (!Number.isFinite(playlistId.value)) return
    const tracks = await getPlaylistTracks(playlistId.value)
    cacheTracks(tracks)
    playlistTrackPaths.value = tracks.map(t => t.file_path)
}

const playlistTracks = computed<TrackMetadata[]>(() => {
    const out: TrackMetadata[] = []
    for (const p of playlistTrackPaths.value) {
        const t = getTrackByPath(p)
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
    if (!q) return playlistTracks.value
    return searchRanked(
        playlistTracks.value,
        q,
        (t) => {
            const fileName = t.file_path.replace(/\\/g, '/').split('/').pop()?.replace(/\.[^/.]+$/, '') || ''
            return [t.title || '', t.artist || '', t.album || '', fileName]
        },
        { formatPath: (t) => t.file_path },
    )
})

// ── Header height ──

const scrollHeaderRef = ref<HTMLElement | null>(null)
const scrollHeaderHeight = ref(0)
let headerResizeObserver: ResizeObserver | null = null

// Gate must stay closed until playlist tracks are loaded, or it opens on an
// empty list, content flashes, then the loader appears over the cover fetch.
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
    // Close the gate while switching playlists, or it'd stay open and flash
    // the previous playlist's content while the new one's tracks load.
    dataReady.value = false
    await reloadPlaylistTracks()
    dataReady.value = true
})

watch(firstCoverTrack, (track) => {
    if (track && !track.cover_art) {
        loadCoversForTracks([track])
    }
}, { immediate: true })

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

const isScrollingToTop = ref(false)

// Hold back first paint until the first 20 covers have resolved.
const { ready: coversReady } = useCoverGate(() => filteredTracks.value.slice(0, 20), {
    armed: () => dataReady.value,
})

watch(visibleTracks, (tracks) => {
    if (!isScrollingToTop.value) {
        loadCoversForTracks(tracks)
    }
}, { immediate: true })

// ── Play button ──

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
    if (isPlaylistActive.value && currentTrack.value?.file_path === track.file_path) {
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
    const btn = (e.currentTarget as HTMLElement)
    const rect = btn.getBoundingClientRect()
    trackMenu.value = { x: rect.left - 6, y: rect.top + rect.height / 2, track }
}

async function onRemoveFromPlaylist(track: TrackMetadata) {
    const idx = playlistTrackPaths.value.indexOf(track.file_path)
    if (idx !== -1) playlistTrackPaths.value.splice(idx, 1)
    await removeTrackFromPlaylist(playlistId.value, track.file_path)
}

// "Move to top" is always available here; playback order only resyncs if
// this playlist is the active source.
const canMoveToTop = computed(() => true)

async function moveTrackToTopOfPlaylist(track: TrackMetadata) {
    const idx = playlistTrackPaths.value.indexOf(track.file_path)
    if (idx <= 0) return
    playlistTrackPaths.value.splice(idx, 1)
    playlistTrackPaths.value.unshift(track.file_path)
    await invoke('playlist_move_track_to_top', { playlistId: playlistId.value, filePath: track.file_path })
    if (isPlaylistActive.value) {
        // Rebuild the source from the reordered playlist.
        setPlaylistSource(sourceId.value, playlistTracks.value.slice())
    }
    window.dispatchEvent(new CustomEvent('lfmusic:playlist-track-moved-to-top', { detail: { playlistId: playlistId.value } }))
}

// ── Back ──

function goBack() {
    goBackOrHome(router)
}

// ── Playlist context menu ──

const playlistMenu = ref<{ x: number; y: number } | null>(null)
const { isPinned: isPinnedFn, togglePin, updatePinName, unpin } = usePins()
const isPinned = computed(() => isPinnedFn('playlist', playlistId.value))

function openPlaylistMenu(e: MouseEvent) {
    e.stopPropagation()
    if (playlistMenu.value) {
        playlistMenu.value = null
        return
    }
    const btn = e.currentTarget as HTMLElement
    const rect = btn.getBoundingClientRect()
    playlistMenu.value = { x: rect.left + rect.width / 2, y: rect.bottom + 6 }
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

// ── Rename playlist ──

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
    window.dispatchEvent(new CustomEvent('lfmusic:show-fullscreen-cover', { detail: { src: playlistCover.value } }))
}
</script>

<template>
    <div class="pageRoot" ref="trackListEl" @scroll="handleScroll">
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader :title="playlist?.name || 'Плейлист'" :scrolled="scrollY > 16" @top="scrollToTop" />
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div>{{ playlist?.name || 'Плейлист' }}</div>
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
                    <AnimatedCover :src="playlistCover" class="cover" alt="cover" />
                    <CoverEditor kind="playlist" :coverKey="playlistId" size="large" />
                </div>
                <div class="dataZone">
                    <div class="info">
                        <div class="listType">Плейлист</div>
                        <div class="listNameRow">
                            <h2 v-if="!isEditingName" v-truncate class="listName" @dblclick="startEditName">{{ playlist?.name || '' }}</h2>
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
                            <button v-if="!isEditingName" class="editNameBtn" @click="startEditName" v-tooltip="'Переименовать'">
                                <EditIcon width="16" height="16" />
                            </button>
                        </div>
                        <div class="badges">
                            <div class="badge" v-tooltip="playlistTracks.length > 0 ? 'Количество треков в плейлисте' : 'Плейлист пуст — добавте треки'">{{ playlistTracks.length > 0 ? tracksCountText(playlistTracks.length) : 'Пусто' }}</div>
                        </div>
                    </div>
                    <div class="controls">
                        <button class="playListBtn" :disabled="playlistTracks.length === 0" @click="onPlayPlaylist">
                            <PauseIcon v-if="isPlaylistActive && isPlaying" width="14" height="14" />
                            <PlayIcon v-else width="14" height="14" style="margin-left:0;" />
                            Слушать
                        </button>
                        <button class="moreBtn" :class="{ menuOpen: !!playlistMenu }" @mousedown.stop="openPlaylistMenu($event)">
                            <MoreIcon width="22" height="22" />
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
        <LoadingSpinner v-if="filteredTracks.length > 0 && !coversReady" key="gate-spinner" />
        <div
            class="tracksList"
            v-else-if="filteredTracks.length > 0"
            key="gate-list"
        >
            <div class="vsPad" :style="{ height: vsPaddingTop + 'px' }"></div>
            <TrackRow
                v-for="track in visibleTracks"
                :key="track.file_path"
                :track="track"
                :active="isPlaylistActive && currentTrack?.file_path === track.file_path"
                :activePaused="isPlaylistActive && currentTrack?.file_path === track.file_path && !isPlaying"
                :menuOpen="trackMenu?.track.file_path === track.file_path"
                @rowClick="onTrackClick($event, track)"
                @coverClick="onCoverClick(track)"
                @openMenu="openTrackMenu($event, track)"
            />
            <div class="vsPad" :style="{ height: vsPaddingBottom + 'px' }"></div>
        </div>
        <div class="emptyState" v-else-if="debouncedSearch && filteredTracks.length === 0" key="gate-empty-search">
            <div class="emptyIcon"><SearchIcon width="16" height="16" /></div>
            <div class="emptyText">Ничего не найдено</div>
            <div class="emptyDesc">Попробуйте изменить критерии поиска.</div>
        </div>
        <div class="emptyState" v-else key="gate-empty">
            <div class="emptyIcon"><EmptyIcon width="16" height="16" /></div>
            <div class="emptyText">В плейлисте пока нет треков</div>
            <div class="emptyDesc">Добавьте треки через контекстное меню в каталоге.</div>
            <button class="emptyAction" @click="router.push('/catalog')">Добавить треки</button>
        </div>
        </Transition>
        <TrackContextMenu
            v-if="trackMenu"
            :key="trackMenu.track.file_path"
            :x="trackMenu.x"
            :y="trackMenu.y"
            :track="trackMenu.track"
            :canMoveToTop="canMoveToTop"
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
            customCoverKind="playlist"
            :customCoverKey="playlistId"
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
.pageHat .dataZone .info .listNameInput {
    cursor: text;
    background-color: hsla(0, 0%, 100%, 0.06);
    border: 1px solid hsla(0, 0%, 100%, 0.12);
    border-radius: 8px;
    padding: 2px 10px;
    outline: none;
    min-width: 240px;
    max-width: 100%;
    transition: border-color .24s ease, background-color .24s ease;
}
.pageHat .dataZone .info .listNameInput:focus {
    background-color: hsla(0, 0%, 100%, 0.09);
    border-color: hsla(0, 0%, 100%, 0.25);
}
.pageHat .dataZone .info .editNameBtn {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 12px;
    border-radius: 9999px;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.55);
    cursor: pointer;
    opacity: 0;
    transform: translateX(-4px);
    transition: opacity .24s ease, transform .24s ease, background-color .24s ease, color .24s ease;
}
.pageHat .dataZone .info .listNameRow:hover .editNameBtn {
    opacity: 1;
    transform: translateX(0);
}
.pageHat .dataZone .info .editNameBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: hsla(0, 0%, 100%, 1);
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
.emptyAction {
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
</style>
