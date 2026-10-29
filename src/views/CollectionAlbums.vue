<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { goBackOrHome } from '../router';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import PinIcon from '../components/Icons/PinIcon.vue';
import MoreIcon from '../components/Icons/MoreIcon.vue';
import SearchIcon from '../components/Icons/SearchIcon.vue';
import EmptyIcon from '../components/Icons/EmptyIcon.vue';
import PlaylistContextMenu from '../components/PlaylistContextMenu.vue';
import defaultTrackCover from '../assets/trackCover.webp';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { sortByTopOrder, usePlaySource } from '../composables/usePlaySource';
import { usePlaylists } from '../composables/usePlaylists';
import { usePins } from '../composables/usePins';
import { searchRanked } from '../composables/useSearch';
import AnimatedCover from '../components/AnimatedCover.vue';
import MobileScrollHeader from '../components/MobileScrollHeader.vue';
import LoadingSpinner from '../components/LoadingSpinner.vue';
import { useCustomCovers } from '../composables/useCustomCovers';
import { useCoverGate } from '../composables/useCoverGate';

const router = useRouter();
const route = useRoute();
const {
    catalogCount,
    getTrackByPath,
    loadCoversForTracks,
    isPlaying,
    activePlaylistSource,
} = usePlayer();
const { playSource } = usePlaySource();

const { albums: albumPlaylists, albumTopOrder, loadPlaylists, moveAlbumToTop } = usePlaylists();

interface AlbumEntry {
    id: number;
    name: string;
    trackCount: number;
    firstPath: string | null;
}

// Deliberately not reading track fields here: looking them up at render
// time means a cover load only re-renders the matching card, not the whole list.
const albums = computed<AlbumEntry[]>(() => {
    return albumPlaylists.value
        .map(pl => ({
            id: pl.id,
            name: pl.name,
            trackCount: pl.track_count,
            firstPath: pl.first_file_path,
        }))
        .sort((a, b) => a.name.localeCompare(b.name));
});

const { getCustomCover } = useCustomCovers();

function coverFor(path: string | null): string | null {
    if (!path) return null;
    return getTrackByPath(path)?.cover_art ?? null;
}

function albumCoverFor(album: AlbumEntry): string | null {
    const custom = getCustomCover('album', album.name);
    if (custom) return custom;
    return coverFor(album.firstPath);
}
function artistFor(path: string | null): string {
    if (!path) return 'Неизвестный исполнитель';
    return getTrackByPath(path)?.artist || 'Неизвестный исполнитель';
}

// Gate must stay closed until albums load, or it opens on an empty list
// (content flashes in, then the loader reappears over cover loading).
const dataReady = ref(false);

onMounted(async () => {
    await loadPlaylists();
    dataReady.value = true;
    const q = route.query.q;
    const initial = Array.isArray(q) ? q[0] : q;
    if (typeof initial === 'string' && initial.trim()) {
        rawSearch.value = initial;
        debouncedSearch.value = initial;
    }
});

watch(() => route.query.q, (q) => {
    const next = Array.isArray(q) ? q[0] : q;
    if (typeof next !== 'string') return;
    rawSearch.value = next;
    debouncedSearch.value = next;
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

const { isPinned: isPinnedFn, togglePin } = usePins();

function isAlbumPinned(name: string) {
    return isPinnedFn('album', name);
}

function toggleAlbumPin(name: string) {
    togglePin({ kind: 'album', key: name, name });
}

function onToggleAlbumPinClick(e: MouseEvent, album: AlbumEntry) {
    e.stopPropagation();
    toggleAlbumPin(album.name);
}

const sortedAlbums = computed(() => sortByTopOrder(albums.value, albumTopOrder.value));

const filteredAlbums = computed(() => {
    const q = debouncedSearch.value.trim();
    if (!q) return sortedAlbums.value;
    return searchRanked(
        sortedAlbums.value,
        q,
        (a) => [a.name, artistFor(a.firstPath)],
    );
});

// Hold back first paint until the first 30 covers have resolved.
const { ready: coversReady } = useCoverGate(() =>
    filteredAlbums.value
        .slice(0, 30)
        .map(a => (a.firstPath ? getTrackByPath(a.firstPath) : undefined))
        .filter((t): t is TrackMetadata => !!t),
    { armed: () => dataReady.value },
);

// All cards render up-front to keep document height/scrollbar stable;
// off-screen ones skip paint via CSS `content-visibility: auto`. Covers load
// per-card via one IntersectionObserver rooted at .pageRoot, bottom margin = 2 rows.
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
function bindCard(el: any, album: AlbumEntry) {
    if (!el) return;
    if (!album.firstPath) return;
    ensureObserver();
    const prev = elPath.get(el);
    if (prev === album.firstPath) return;
    if (prev) {
        coverObs!.unobserve(el);
        observedEls.delete(el);
    }
    elPath.set(el, album.firstPath);
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
    // Recreate now that the real scroll root is known — earlier bindCard
    // calls may have built the observer with a null (window) root.
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

// Re-observe when the library populates after mount, so cards still in
// view fire again now that they have track data.
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

function isAlbumActive(album: AlbumEntry) {
    return activePlaylistSource.value === `album:${album.name}`;
}

async function onPlayAlbum(e: MouseEvent, album: AlbumEntry) {
    e.stopPropagation();
    await playSource(`album:${album.name}`, album.id);
}

function openAlbum(album: AlbumEntry) {
    router.push(`/album/${encodeURIComponent(album.name)}`);
}

const albumMenu = ref<{ x: number; y: number; album: AlbumEntry; centerX: boolean } | null>(null);

function openAlbumMenu(e: MouseEvent, album: AlbumEntry) {
    e.stopPropagation();
    if (albumMenu.value?.album.name === album.name) {
        albumMenu.value = null;
        return;
    }
    albumMenu.value = { ...menuAnchorBelow(e), album, centerX: true };
}

function openAlbumMenuFromCover(e: MouseEvent, album: AlbumEntry) {
    e.preventDefault();
    e.stopPropagation();
    albumMenu.value = { x: e.clientX, y: e.clientY, album, centerX: false };
}

function onMoveAlbumToTopFromMenu(album: AlbumEntry) {
    moveAlbumToTop(album.name);
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
    <div class="pageRoot" ref="pageRootEl" @scroll="handleScroll">
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader title="Альбомы" :scrolled="scrollY > 16" @top="scrollToTop" />
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div>Альбомы</div>
                    <button class="toTopBtn" @click="scrollToTop">Вверх</button>
                </div>
            </Transition>
        </div>
        <button class="backBtn" @click="goBack">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
            <span>Назад</span>
        </button>
        <h1 class="pageTitle">Альбомы</h1>
        <div class="searchZone">
            <SearchIcon class="searchIcon" @click="focusSearch" />
            <input
                ref="searchInput"
                type="text"
                class="search"
                placeholder="Поиск альбома"
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
        <div class="gridGate" v-if="filteredAlbums.length > 0">
            <Transition name="gate-fade">
                <LoadingSpinner v-if="!coversReady" class="gateOverlay" />
            </Transition>
            <div class="grid" :class="{ gateHidden: !coversReady }" ref="gridEl">
            <div
                v-for="album in filteredAlbums"
                :ref="(el: any) => bindCard(el, album)"
                :key="album.id"
                class="playlistItem albumItem"
                @click="openAlbum(album)"
            >
                <div
                    class="coverWrap"
                    :class="{ menuOpen: albumMenu?.album.name === album.name }"
                    @contextmenu="openAlbumMenuFromCover($event, album)"
                >
                    <AnimatedCover class="cover" :src="albumCoverFor(album) || defaultTrackCover" />
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
                        class="overlayBtn moreBtn"
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
                <div v-truncate class="subtitle">{{ artistFor(album.firstPath) }}</div>
            </div>
            </div>
        </div>
        <div class="emptyState" v-else-if="debouncedSearch">
            <div class="emptyIcon"><SearchIcon width="16" height="16" /></div>
            <div class="emptyText">Ничего не найдено</div>
            <div class="emptyDesc">Попробуйте изменить критерии поиска.</div>
        </div>
        <div class="emptyState" v-else>
            <div class="emptyIcon"><EmptyIcon width="16" height="16" /></div>
            <div class="emptyText">Пока нет альбомов</div>
            <div class="emptyDesc">Альбомы формируются автоматически из вашей библиотеки.</div>
        </div>
        <PlaylistContextMenu
            v-if="albumMenu"
            :x="albumMenu.x"
            :y="albumMenu.y"
            :isPinned="isAlbumPinned(albumMenu.album.name)"
            showMoveToTop
            :showDelete="false"
            :centerX="albumMenu.centerX"
            customCoverKind="album"
            :customCoverKey="albumMenu.album.name"
            @close="albumMenu = null"
            @togglePin="toggleAlbumPin(albumMenu!.album.name)"
            @moveToTop="onMoveAlbumToTopFromMenu(albumMenu!.album)"
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

.playlistItem {
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
    /* Skip paint/layout for off-screen cards while preserving full document height. */
    content-visibility: auto;
    contain-intrinsic-size: 200px 280px;
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
.playlistItem .subtitle {
    color: hsla(0, 0%, 100%, 0.4);
    font-weight: 500;
    font-size: 12px;
    margin-left: 4px;
    margin-top: -4px;
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
