<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import SearchIcon from '../components/Icons/SearchIcon.vue';
import EmptyIcon from '../components/Icons/EmptyIcon.vue';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import PinIcon from '../components/Icons/PinIcon.vue';
import MoreIcon from '../components/Icons/MoreIcon.vue';
import PlaylistContextMenu from '../components/PlaylistContextMenu.vue';
import defaultTrackCover from '../assets/trackCover.webp';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { usePlaylists } from '../composables/usePlaylists';
import { usePins } from '../composables/usePins';
import { searchRanked } from '../composables/useSearch';
import AnimatedCover from '../components/AnimatedCover.vue';
import MobileScrollHeader from '../components/MobileScrollHeader.vue';
import LoadingSpinner from '../components/LoadingSpinner.vue';
import { useCustomCovers } from '../composables/useCustomCovers';
import { useCoverGate } from '../composables/useCoverGate';
import { useExternalSources } from '../composables/useExternalSources';

const router = useRouter();
const {
    catalogCount,
    getTrackByPath,
    loadCoversForTracks,
    isPlaying,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    setPlaylistSource,
} = usePlayer();
const { artists: artistPlaylists, artistTopOrder, loadPlaylists, getPlaylistTracks, moveArtistToTop } = usePlaylists();

interface ArtistEntry {
    id: number;
    name: string;
    firstPath: string | null;
    trackCount: number;
}

// Don't read track fields in this computed — looking them up at render time
// keeps a single cover load from invalidating the whole list.
// Sort by track count desc, then name as tiebreaker.
const artists = computed<ArtistEntry[]>(() => {
    return artistPlaylists.value
        .map(pl => ({
            id: pl.id,
            name: pl.name,
            firstPath: pl.first_file_path,
            trackCount: pl.track_count,
        }))
        .sort((a, b) => b.trackCount - a.trackCount || a.name.localeCompare(b.name));
});

const { getCustomCover } = useCustomCovers();
const { avatarFor } = useExternalSources();

function coverFor(path: string | null): string | null {
    if (!path) return null;
    return getTrackByPath(path)?.cover_art ?? null;
}

function artistCoverFor(artist: ArtistEntry): string {
    const custom = getCustomCover('artist', artist.name);
    if (custom) return custom;
    const external = avatarFor(artist.name);
    if (external) return external;
    return coverFor(artist.firstPath) || defaultTrackCover;
}

onMounted(async () => {
    await loadPlaylists();
});

// ── Search ──────────────────────────────────────────────────────────────────

const searchInput = ref<HTMLInputElement | null>(null);
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

const sortedArtists = computed(() => {
    const all = artists.value;
    const topSet = new Set(artistTopOrder.value);
    const top: ArtistEntry[] = [];
    for (const name of artistTopOrder.value) {
        const a = all.find(ar => ar.name === name);
        if (a) top.push(a);
    }
    const rest = all.filter(a => !topSet.has(a.name));
    return [...top, ...rest];
});

const filteredArtists = computed(() => {
    const q = debouncedSearch.value.trim();
    if (!q) return sortedArtists.value;
    return searchRanked(sortedArtists.value, q, (a) => [a.name]);
});

// Hold back first paint until the first 30 covers have resolved.
const { ready: coversReady } = useCoverGate(() =>
    filteredArtists.value
        .slice(0, 30)
        .map(a => (a.firstPath ? getTrackByPath(a.firstPath) : undefined))
        .filter((t): t is TrackMetadata => !!t),
);

// ── Pinned ─────────────────────────────────────────────────────────────────

const { isPinned: isPinnedFn, togglePin } = usePins();

function isArtistPinned(name: string) {
    return isPinnedFn('artist', name);
}

function toggleArtistPin(name: string) {
    togglePin({ kind: 'artist', key: name, name });
}

function onToggleArtistPinClick(e: MouseEvent, artist: ArtistEntry) {
    e.stopPropagation();
    toggleArtistPin(artist.name);
}

// ── Play artist ────────────────────────────────────────────────────────────

function isArtistActive(artist: ArtistEntry) {
    return activePlaylistSource.value === `artist:${artist.name}`;
}

async function onPlayArtist(e: MouseEvent, artist: ArtistEntry) {
    e.stopPropagation();
    if (isArtistActive(artist)) {
        togglePlayPause();
        return;
    }
    if (artist.trackCount === 0) return;
    const tracks = await getPlaylistTracks(artist.id);
    if (tracks.length === 0) return;
    const merged = tracks.map(t => getTrackByPath(t.file_path) ?? t);
    setPlaylistSource(`artist:${artist.name}`, merged);
    await playTrack(merged[0]);
}

// ── Context menu ───────────────────────────────────────────────────────────

const artistMenu = ref<{ x: number; y: number; artist: ArtistEntry; centerX: boolean } | null>(null);

function openArtistMenu(e: MouseEvent, artist: ArtistEntry) {
    e.stopPropagation();
    if (artistMenu.value?.artist.name === artist.name) {
        artistMenu.value = null;
        return;
    }
    const btn = e.currentTarget as HTMLElement;
    const rect = btn.getBoundingClientRect();
    artistMenu.value = { x: rect.left + rect.width / 2, y: rect.bottom + 6, artist, centerX: true };
}

function openArtistMenuFromCover(e: MouseEvent, artist: ArtistEntry) {
    e.preventDefault();
    e.stopPropagation();
    artistMenu.value = { x: e.clientX, y: e.clientY, artist, centerX: false };
}

function onMoveArtistToTopFromMenu(artist: ArtistEntry) {
    moveArtistToTop(artist.name);
}

// ── Lazy cover loading ─────────────────────────────────────────────────────
// Render every card up-front so the document height stays constant; off-screen
// cards are skipped via CSS `content-visibility: auto`. A single
// IntersectionObserver whose `root` is the actual scroll container
// (.pageRoot) and whose bottom rootMargin equals exactly 2 card rows.
const gridEl = ref<HTMLElement | null>(null);
let coverObs: IntersectionObserver | null = null;
let resizeObs: ResizeObserver | null = null;
let scrollRoot: HTMLElement | null = null;
let lastBottomMargin = 600;
const elPath = new WeakMap<HTMLElement, string>();
const elArtist = new WeakMap<HTMLElement, string>();
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
        // Видимая карточка — поднимаем приоритет её внешней аватарки (если включено).
        const name = elArtist.get(entry.target as HTMLElement);
        if (name) avatarFor(name);
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

// bindCard runs as a v-for function ref *during render*, before onMounted —
// so the observer must be created lazily here, not in onMounted, otherwise
// most cards never get registered.
function bindCard(el: any, artist: ArtistEntry) {
    if (!el) return;
    if (!artist.firstPath) return;
    ensureObserver();
    const prev = elPath.get(el);
    if (prev === artist.firstPath) return;
    if (prev) {
        coverObs!.unobserve(el);
        observedEls.delete(el);
    }
    elPath.set(el, artist.firstPath);
    elArtist.set(el, artist.name);
    coverObs!.observe(el);
    observedEls.add(el);
}

function measureRowHeight(): number {
    if (!gridEl.value) return 0;
    const card = gridEl.value.querySelector<HTMLElement>('.artistItem');
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
    const cards = gridEl.value.querySelectorAll<HTMLElement>('.artistItem');
    for (const el of cards) {
        if (!elPath.has(el)) continue;
        coverObs!.unobserve(el);
        coverObs!.observe(el);
        observedEls.add(el);
    }
});

function goBack() {
    router.back();
}

// ── Sticky header ───────────────────────────────────────────────────────────

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
            <MobileScrollHeader title="Исполнители" :scrolled="scrollY > 16" @top="scrollToTop" />
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div>Исполнители</div>
                    <button class="toTopBtn" @click="scrollToTop">Вверх</button>
                </div>
            </Transition>
        </div>
        <button class="backBtn" @click="goBack">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
            <span>Назад</span>
        </button>
        <h1 class="pageTitle">Исполнители</h1>
        <div class="searchZone">
            <SearchIcon class="searchIcon" @click="focusSearch" />
            <input
                ref="searchInput"
                type="text"
                class="search"
                placeholder="Поиск исполнителя"
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
        <div class="gridGate" v-if="filteredArtists.length > 0">
            <Transition name="gate-fade">
                <LoadingSpinner v-if="!coversReady" class="gateOverlay" />
            </Transition>
            <div class="grid" :class="{ gateHidden: !coversReady }" ref="gridEl">
            <div
                v-for="artist in filteredArtists"
                :ref="(el: any) => bindCard(el, artist)"
                :key="artist.id"
                class="artistItem"
                @click="router.push(`/artist/${encodeURIComponent(artist.name)}`)"
            >
                <div
                    class="artistCoverWrap"
                    :class="{ menuOpen: artistMenu?.artist.name === artist.name }"
                    @contextmenu="openArtistMenuFromCover($event, artist)"
                >
                    <AnimatedCover class="artistCover" :src="artistCoverFor(artist)" />
                    <button
                        class="overlayBtn pinBtn"
                        :class="{ active: isArtistPinned(artist.name) }"
                        @click="onToggleArtistPinClick($event, artist)"
                        @contextmenu.stop
                        v-tooltip="isArtistPinned(artist.name) ? 'Открепить' : 'Закрепить'"
                    >
                        <PinIcon width="16" height="16" />
                    </button>
                    <button
                        class="overlayBtn playBtn"
                        :disabled="artist.trackCount === 0"
                        @click="onPlayArtist($event, artist)"
                        @contextmenu.stop
                        v-tooltip="isArtistActive(artist) && isPlaying ? 'Пауза' : 'Слушать'"
                    >
                        <PauseIcon v-if="isArtistActive(artist) && isPlaying" width="14" height="14" />
                        <PlayIcon v-else width="14" height="14" style="margin-left:1px;" />
                    </button>
                    <button
                        class="overlayBtn moreBtn"
                        :class="{ active: artistMenu?.artist.name === artist.name }"
                        @mousedown.stop="openArtistMenu($event, artist)"
                        @click.stop
                        @contextmenu.stop
                        v-tooltip="'Меню'"
                    >
                        <MoreIcon width="20" height="20" />
                    </button>
                </div>
                <div v-truncate class="artistTitle">{{ artist.name }}</div>
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
            <div class="emptyText">Пока нет исполнителей</div>
            <div class="emptyDesc">Исполнители формируются автоматически из вашей библиотеки.</div>
        </div>
        <PlaylistContextMenu
            v-if="artistMenu"
            :x="artistMenu.x"
            :y="artistMenu.y"
            :isPinned="isArtistPinned(artistMenu.artist.name)"
            showMoveToTop
            :showDelete="false"
            :centerX="artistMenu.centerX"
            customCoverKind="artist"
            :customCoverKey="artistMenu.artist.name"
            @close="artistMenu = null"
            @togglePin="toggleArtistPin(artistMenu!.artist.name)"
            @moveToTop="onMoveArtistToTopFromMenu(artistMenu!.artist)"
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
    --card-w: 170px;
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

.artistItem {
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
    user-select: none;
    min-width: 0;
    /* Skip paint/layout for off-screen cards while preserving full document height. */
    content-visibility: auto;
    contain-intrinsic-size: 170px 220px;
}
.artistCoverWrap {
    position: relative;
    width: 100%;
    aspect-ratio: 1 / 1;
}
.artistCover {
    display: block;
    width: 100%;
    height: 100%;
    border-radius: 9999px;
    object-fit: cover;
    user-select: none;
    background-color: #202020;
    transition: filter .24s ease;
}
.artistItem:hover .artistCover,
.artistCoverWrap.menuOpen .artistCover {
    filter: brightness(0.6);
}
.artistItem .overlayBtn {
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
.artistItem .overlayBtn:hover {
    background-color: hsla(0, 0%, 0%, 0.65);
    color: hsla(0, 0%, 100%, 1);
}
.artistItem:hover .overlayBtn,
.artistItem .artistCoverWrap.menuOpen .overlayBtn,
.artistItem .overlayBtn.active {
    opacity: 1;
    transform: translateY(0);
}
.artistItem .playBtn {
    left: 12px;
    bottom: 12px;
    padding: 12px;
    color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
    background-color: hsla(0, 0%, 100%, 0.95);
}
.artistItem .playBtn:hover {
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
    background-color: hsla(0, 0%, 100%, 1);
}
.artistItem .playBtn:disabled {
    cursor: default;
    color: hsla(0, 0%, 100%, 0.5);
    background-color: hsla(0, 0%, 100%, 0.18);
}
.artistItem .pinBtn {
    top: 12px;
    right: 12px;
    padding: 8px;
}
.artistItem .pinBtn.active {
    color: hsla(0, 0%, 100%, 1);
}
.artistItem .pinBtn.active :deep(svg path) {
    fill: currentColor;
}
.artistItem .moreBtn {
    right: 12px;
    bottom: 12px;
    padding: 8px;
}
.artistItem .moreBtn.active {
    color: hsla(0, 0%, 100%, 1);
    background-color: hsla(0, 0%, 0%, 0.65);
}
.artistTitle {
    text-align: center;
    color: hsla(0, 0%, 100%, 0.8);
    font-weight: 500;
    font-size: 14px;
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
