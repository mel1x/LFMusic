<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { goBackOrHome } from '../router';
import PlusIcon from '../components/Icons/PlusIcon.vue';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import PinIcon from '../components/Icons/PinIcon.vue';
import MoreIcon from '../components/Icons/MoreIcon.vue';
import SearchIcon from '../components/Icons/SearchIcon.vue';
import EmptyIcon from '../components/Icons/EmptyIcon.vue';
import PlaylistContextMenu from '../components/PlaylistContextMenu.vue';
import ConfirmModal from '../components/ConfirmModal.vue';
import defaultCover from '../assets/playlistCover.webp';
import { usePlaylists, type PlaylistInfo } from '../composables/usePlaylists';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { usePlaySource } from '../composables/usePlaySource';
import { usePins } from '../composables/usePins';
import { searchRanked } from '../composables/useSearch';
import AnimatedCover from '../components/AnimatedCover.vue';
import FavoritesCover from '../components/FavoritesCover.vue';
import MobileScrollHeader from '../components/MobileScrollHeader.vue';
import LoadingSpinner from '../components/LoadingSpinner.vue';
import { useCustomCovers } from '../composables/useCustomCovers';
import { useCoverGate } from '../composables/useCoverGate';

const router = useRouter();
const { userPlaylists, favoritesPlaylist, loadPlaylists, createPlaylist, nextPlaylistName, deletePlaylist, movePlaylistToTop } = usePlaylists();
const {
    catalogCount,
    getTrackByPath,
    loadCoversForTracks,
    isPlaying,
    activePlaylistSource,
} = usePlayer();
const { playSource } = usePlaySource();

const { getCustomCover } = useCustomCovers();

function coverFor(path: string | null): string | null {
    if (!path) return null;
    return getTrackByPath(path)?.cover_art ?? null;
}

function playlistCoverFor(pl: PlaylistInfo): string {
    const custom = getCustomCover('playlist', pl.id);
    if (custom) return custom;
    return coverFor(pl.first_file_path) || defaultCover;
}

// Gate must stay closed until playlists load, or it opens on an empty list
// (content flashes in, then the loader reappears over cover loading).
const dataReady = ref(false);

onMounted(async () => {
    await loadPlaylists();
    dataReady.value = true;
});

import { useTypeToSearch } from '../composables/useTypeToSearch';
import { menuAnchorBelow } from '../utils/menu';
const searchInput = ref<HTMLInputElement | null>(null);
useTypeToSearch(searchInput);
const rawSearch = ref('');
const debouncedSearch = ref('');
let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null;

function onSearchInput(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    rawSearch.value = value;
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
    searchDebounceTimer = setTimeout(() => {
        debouncedSearch.value = value;
    }, 200);
}

function focusSearch() {
    searchInput.value?.focus();
}

function clearSearch() {
    rawSearch.value = '';
    debouncedSearch.value = '';
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
    searchInput.value?.focus();
}

const filteredPlaylists = computed(() => {
    const q = debouncedSearch.value.trim();
    if (!q) return userPlaylists.value;
    return searchRanked(userPlaylists.value, q, (p) => [p.name]);
});

// The favorites playlist is pinned to the front and hidden while searching
// (it isn't part of the searchable user-playlist set).
const gridPlaylists = computed<PlaylistInfo[]>(() => {
    const fav = favoritesPlaylist.value;
    if (!fav || debouncedSearch.value.trim()) return filteredPlaylists.value;
    return [fav, ...filteredPlaylists.value];
});

function isFavoritesPl(pl: PlaylistInfo): boolean {
    return pl.kind === 'Favorites';
}

// Hold back first paint until the first 30 covers have resolved.
const { ready: coversReady } = useCoverGate(() =>
    filteredPlaylists.value
        .slice(0, 30)
        .map(p => (p.first_file_path ? getTrackByPath(p.first_file_path) : undefined))
        .filter((t): t is TrackMetadata => !!t),
    { armed: () => dataReady.value },
);

// Single IntersectionObserver rooted at .pageRoot; bottom rootMargin = 2 card rows.
const gridEl = ref<HTMLElement | null>(null);
let coverObs: IntersectionObserver | null = null;
let resizeObs: ResizeObserver | null = null;
let scrollRoot: HTMLElement | null = null;
let lastBottomMargin = 600;
const elPath = new WeakMap<HTMLElement, string>();
const observedEls = new Set<HTMLElement>();

function findScrollAncestor(el: HTMLElement): HTMLElement | null {
    let node: HTMLElement | null = el.parentElement;
    while (node) {
        const oy = getComputedStyle(node).overflowY;
        if (oy === 'auto' || oy === 'scroll') return node;
        node = node.parentElement;
    }
    return null;
}

function onIntersect(entries: IntersectionObserverEntry[]) {
    const pending: TrackMetadata[] = [];
    for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const path = elPath.get(entry.target as HTMLElement);
        if (!path) continue;
        const t = getTrackByPath(path);
        if (t && !t.cover_art) pending.push(t);
    }
    if (pending.length) loadCoversForTracks(pending);
}

function buildObserver() {
    if (coverObs) coverObs.disconnect();
    coverObs = new IntersectionObserver(onIntersect, {
        root: scrollRoot,
        rootMargin: `0px 0px ${lastBottomMargin}px 0px`,
    });
    for (const el of observedEls) coverObs.observe(el);
}

function ensureObserver() {
    if (!coverObs) buildObserver();
}

// Runs as a v-for function ref during render, before onMounted — observer
// must be created lazily here or most cards never get registered.
function bindCard(el: any, pl: PlaylistInfo) {
    if (!el) return;
    if (!pl.first_file_path) return;
    ensureObserver();
    const prev = elPath.get(el);
    if (prev === pl.first_file_path) return;
    if (prev) {
        coverObs!.unobserve(el);
        observedEls.delete(el);
    }
    elPath.set(el, pl.first_file_path);
    coverObs!.observe(el);
    observedEls.add(el);
}

function measureRowHeight(): number {
    if (!gridEl.value) return 0;
    const card = gridEl.value.querySelector<HTMLElement>('.playlistItem');
    if (!card) return 0;
    const cardH = card.getBoundingClientRect().height;
    if (cardH <= 0) return 0;
    const rowGap = parseFloat(getComputedStyle(gridEl.value).rowGap || '0') || 0;
    return cardH + rowGap;
}

function refreshMargin() {
    const rowH = measureRowHeight();
    if (rowH <= 0) return;
    const bottom = Math.max(1, Math.round(rowH * 2));
    if (bottom === lastBottomMargin && coverObs) return;
    lastBottomMargin = bottom;
    buildObserver();
}

onMounted(() => {
    if (gridEl.value) {
        scrollRoot = findScrollAncestor(gridEl.value);
    }
    buildObserver();
    requestAnimationFrame(refreshMargin);
    if (gridEl.value) {
        resizeObs = new ResizeObserver(() => refreshMargin());
        resizeObs.observe(gridEl.value);
    }
});

onUnmounted(() => {
    coverObs?.disconnect();
    coverObs = null;
    resizeObs?.disconnect();
    resizeObs = null;
    scrollRoot = null;
    observedEls.clear();
});

watch(catalogCount, () => {
    if (!gridEl.value) return;
    ensureObserver();
    refreshMargin();
    const cards = gridEl.value.querySelectorAll<HTMLElement>('.playlistItem');
    for (const el of cards) {
        if (!elPath.has(el)) continue;
        coverObs!.unobserve(el);
        coverObs!.observe(el);
        observedEls.add(el);
    }
});

const createModalOpen = ref(false);

function onCreatePlaylist() {
    createModalOpen.value = true;
}

async function onConfirmCreate() {
    createModalOpen.value = false;
    const name = nextPlaylistName();
    const pl = await createPlaylist(name);
    if (pl) {
        router.push(`/playlist/${pl.id}`);
    }
}

function openPlaylist(pl: PlaylistInfo) {
    router.push(`/playlist/${pl.id}`);
}

const { isPinned: isPinnedFn, togglePin, unpin } = usePins();

function isPinned(id: number) {
    return isPinnedFn('playlist', id);
}

function togglePinId(id: number) {
    const pl = userPlaylists.value.find(p => p.id === id)
        ?? (favoritesPlaylist.value?.id === id ? favoritesPlaylist.value : null);
    if (!pl) return;
    togglePin({ kind: 'playlist', key: id, name: pl.name });
}

function isPlaylistActive(pl: PlaylistInfo) {
    return activePlaylistSource.value === `playlist:${pl.id}`;
}

async function onPlayPlaylist(e: MouseEvent, pl: PlaylistInfo) {
    e.stopPropagation();
    await playSource(`playlist:${pl.id}`, pl.id);
}

function onTogglePinClick(e: MouseEvent, pl: PlaylistInfo) {
    e.stopPropagation();
    togglePinId(pl.id);
}

const playlistMenu = ref<{ x: number; y: number; pl: PlaylistInfo; centerX: boolean } | null>(null);

function openPlaylistMenu(e: MouseEvent, pl: PlaylistInfo) {
    e.stopPropagation();
    if (playlistMenu.value?.pl.id === pl.id) {
        playlistMenu.value = null;
        return;
    }
    playlistMenu.value = { ...menuAnchorBelow(e), pl, centerX: true };
}

function openPlaylistMenuFromCover(e: MouseEvent, pl: PlaylistInfo) {
    e.preventDefault();
    e.stopPropagation();
    playlistMenu.value = { x: e.clientX, y: e.clientY, pl, centerX: false };
}

const deleteModalTarget = ref<PlaylistInfo | null>(null);

function onDeleteFromMenu(pl: PlaylistInfo) {
    deleteModalTarget.value = pl;
}

async function onConfirmDelete() {
    const pl = deleteModalTarget.value;
    deleteModalTarget.value = null;
    if (!pl) return;
    unpin('playlist', pl.id);
    await deletePlaylist(pl.id);
}

function onMoveToTopFromMenu(pl: PlaylistInfo) {
    movePlaylistToTop(pl.id);
}

function goBack() {
    goBackOrHome(router);
}

const pageRootEl = ref<HTMLElement | null>(null);
const scrollY = ref(0);
const showStickyHeader = computed(() => scrollY.value > 200);

function handleScroll(e: Event) {
    scrollY.value = (e.target as HTMLElement).scrollTop;
}

function scrollToTop() {
    pageRootEl.value?.scrollTo({ top: 0, behavior: 'smooth' });
}
</script>

<template>
    <div class="pageRoot mobilePage" ref="pageRootEl" @scroll="handleScroll">
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader title="Мои плейлисты" :scrolled="scrollY > 16" @top="scrollToTop" />
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div>Мои плейлисты</div>
                    <button class="toTopBtn" @click="scrollToTop">Вверх</button>
                </div>
            </Transition>
        </div>
        <button class="backBtn" @click="goBack">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
            <span>Назад</span>
        </button>
        <h1 class="pageTitle">Мои плейлисты</h1>
        <div class="searchZone">
            <SearchIcon class="searchIcon" @click="focusSearch" />
            <input
                ref="searchInput"
                type="text"
                class="search"
                name="search"
                id="search"
                placeholder="Поиск плейлиста"
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
        <div class="gridGate" v-if="filteredPlaylists.length > 0 || !debouncedSearch">
            <Transition name="gate-fade">
                <LoadingSpinner v-if="!coversReady" class="gateOverlay" />
            </Transition>
            <div class="grid" :class="{ gateHidden: !coversReady }" ref="gridEl">
            <div v-if="!debouncedSearch" class="createPlaylistItem" @click="onCreatePlaylist">
                <div class="cover"><PlusIcon width="64px" height="64px" stroke-width="1.1" /></div>
                <div class="title">Новый плейлист</div>
            </div>
            <TransitionGroup name="playlist">
                <div
                    v-for="pl in gridPlaylists"
                    :ref="(el: any) => bindCard(el, pl)"
                    :key="pl.id"
                    class="playlistItem"
                    @click="openPlaylist(pl)"
                >
                    <div
                        class="coverWrap"
                        :class="{ menuOpen: playlistMenu?.pl.id === pl.id }"
                        @contextmenu="openPlaylistMenuFromCover($event, pl)"
                    >
                        <FavoritesCover v-if="isFavoritesPl(pl)" class="cover" />
                        <AnimatedCover v-else class="cover" :src="playlistCoverFor(pl)" />
                        <button
                            class="overlayBtn pinBtn"
                            :class="{ active: isPinned(pl.id) }"
                            @click="onTogglePinClick($event, pl)"
                            @contextmenu.stop
                            v-tooltip="isPinned(pl.id) ? 'Открепить' : 'Закрепить'"
                        >
                            <PinIcon width="16" height="16" />
                        </button>
                        <button
                            class="overlayBtn playBtn"
                            :disabled="pl.track_count === 0"
                            @click="onPlayPlaylist($event, pl)"
                            @contextmenu.stop
                            v-tooltip="pl.track_count === 0 ? 'Плейлист пуст' : (isPlaylistActive(pl) && isPlaying ? 'Пауза' : 'Слушать')"
                        >
                            <PauseIcon v-if="isPlaylistActive(pl) && isPlaying" width="14" height="14" />
                            <PlayIcon v-else width="14" height="14" style="margin-left:1px;" />
                        </button>
                        <button
                            class="overlayBtn moreBtn"
                            :class="{ active: playlistMenu?.pl.id === pl.id }"
                            @mousedown.stop="openPlaylistMenu($event, pl)"
                            @click.stop
                            @contextmenu.stop
                            v-tooltip="'Меню'"
                        >
                            <MoreIcon width="20" height="20" />
                        </button>
                    </div>
                    <div v-truncate class="title">{{ pl.name }}</div>
                </div>
            </TransitionGroup>
            </div>
        </div>
        <div class="emptyState" v-else>
            <div class="emptyIcon"><SearchIcon width="16" height="16" /></div>
            <div class="emptyText">Ничего не найдено</div>
            <div class="emptyDesc">Попробуйте изменить критерии поиска.</div>
        </div>
        <div class="emptyState" v-if="!debouncedSearch && userPlaylists.length === 0" style="display:none;">
            <div class="emptyIcon"><EmptyIcon width="16" height="16" /></div>
        </div>
        <PlaylistContextMenu
            v-if="playlistMenu"
            :x="playlistMenu.x"
            :y="playlistMenu.y"
            :isPinned="isPinned(playlistMenu.pl.id)"
            :showMoveToTop="!isFavoritesPl(playlistMenu.pl)"
            :showDelete="!isFavoritesPl(playlistMenu.pl)"
            :centerX="playlistMenu.centerX"
            :customCoverKind="isFavoritesPl(playlistMenu.pl) ? undefined : 'playlist'"
            :customCoverKey="isFavoritesPl(playlistMenu.pl) ? undefined : playlistMenu.pl.id"
            @close="playlistMenu = null"
            @togglePin="togglePinId(playlistMenu!.pl.id)"
            @delete="onDeleteFromMenu(playlistMenu!.pl)"
            @moveToTop="onMoveToTopFromMenu(playlistMenu!.pl)"
        />
        <ConfirmModal
            :open="createModalOpen"
            title="Вы уверены?"
            message="Новый плейлист будет добавлен в вашу коллекцию"
            confirmText="Создать"
            cancelText="Отмена"
            @confirm="onConfirmCreate"
            @cancel="createModalOpen = false"
        />
        <ConfirmModal
            :open="deleteModalTarget !== null"
            title="Вы уверены?"
            :message="deleteModalTarget ? `Плейлист ${deleteModalTarget.name} будет удалён без возможности восстановления` : ''"
            confirmText="Удалить"
            cancelText="Отмена"
            @confirm="onConfirmDelete"
            @cancel="deleteModalTarget = null"
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
.pageTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 900;
    font-size: 36px;
    color: #ffffff;
    margin-bottom: 20px;
}
.searchZone {
    position: relative;
    margin-bottom: 24px;
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

.gridGate {
    position: relative;
    display: flex;
    flex-direction: column;
    flex: 1;
}
.gateOverlay {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    z-index: 1;
}
.grid {
    --card-w: 200px;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--card-w), 1fr));
    gap: 24px 16px;
    padding-bottom: 24px;
    transition: opacity 0.24s ease;
}
.grid.gateHidden {
    opacity: 0;
    pointer-events: none;
}

.createPlaylistItem {
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
}
.createPlaylistItem .cover {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    background-color: hsl(var(--accent-h), min(var(--accent-s), 50%), 8%);
    width: 100%;
    aspect-ratio: 1 / 1;
    border-radius: 18px;
    transition: .24s ease;
}
.createPlaylistItem .cover svg {
    transition: .24s ease;
    color: hsla(0, 0%, 100%, 0.4);
}
.createPlaylistItem:hover .cover {
    background-color: hsl(var(--accent-h), min(var(--accent-s), 50%), 11%);
}
.createPlaylistItem:hover .cover svg {
    color: hsla(0, 0%, 100%, 0.6);
}
.createPlaylistItem .title {
    color: hsla(0, 0%, 100%, 0.8);
    font-weight: 500;
    font-size: 14px;
    margin-left: 4px;
}

.playlistItem {
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
}
.playlist-enter-active {
    transition: opacity .28s ease, transform .28s ease;
}
.playlist-enter-from {
    opacity: 0;
    transform: scale(0.85);
}
.playlist-leave-active {
    transition: opacity .2s ease;
    pointer-events: none;
}
.playlist-leave-to {
    opacity: 0;
}
.playlist-move {
    transition: transform .28s ease;
}
.playlistItem .coverWrap {
    position: relative;
    width: 100%;
    aspect-ratio: 1 / 1;
    border-radius: 18px;
    overflow: hidden;
}
.playlistItem .cover {
    display: block;
    width: 100%;
    height: 100%;
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
.playlistItem .playBtn:disabled:hover {
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
.playlistItem .moreBtn {
    right: 12px;
    bottom: 12px;
    padding: 8px;
}
.playlistItem .moreBtn.active {
    color: hsla(0, 0%, 100%, 1);
    background-color: hsla(0, 0%, 0%, 0.65);
}
.playlistItem .title {
    color: hsla(0, 0%, 100%, 0.8);
    font-weight: 500;
    font-size: 14px;
    margin-left: 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.emptyState {
    flex: 1;
    display: flex;
    gap: 10px;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 60px 0;
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
