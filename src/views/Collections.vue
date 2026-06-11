<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import PlusIcon from '../components/Icons/PlusIcon.vue';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import PinIcon from '../components/Icons/PinIcon.vue';
import MoreIcon from '../components/Icons/MoreIcon.vue';
import PlaylistContextMenu from '../components/PlaylistContextMenu.vue';
import ConfirmModal from '../components/ConfirmModal.vue';
import defaultCover from '../assets/playlistCover.webp';
import defaultTrackCover from '../assets/trackCover.webp';
import { usePlaylists, type PlaylistInfo } from '../composables/usePlaylists';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { usePins } from '../composables/usePins';
import AnimatedCover from '../components/AnimatedCover.vue';
import MobileScrollHeader from '../components/MobileScrollHeader.vue';
import LoadingSpinner from '../components/LoadingSpinner.vue';
import { useCustomCovers } from '../composables/useCustomCovers';
import { useCoverGate } from '../composables/useCoverGate';

const router = useRouter();
const {
    userPlaylists,
    albums: albumPlaylists,
    artists: artistPlaylists,
    albumTopOrder,
    artistTopOrder,
    loadPlaylists,
    createPlaylist,
    getPlaylistTracks,
    deletePlaylist,
    movePlaylistToTop,
    moveAlbumToTop,
    moveArtistToTop,
} = usePlaylists();
const {
    trackList,
    getTrackByPath,
    loadCoversForTracks,
    isPlaying,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    setPlaylistSource,
} = usePlayer();

const playlistCovers = ref<Record<number, string>>({});

const { getCustomCover } = useCustomCovers();

function playlistCoverFor(pl: PlaylistInfo): string {
    const custom = getCustomCover('playlist', pl.id);
    if (custom) return custom;
    const first = pl.first_file_path ? getTrackByPath(pl.first_file_path) : undefined;
    if (first?.cover_art) return first.cover_art;
    return playlistCovers.value[pl.id] || defaultCover;
}
function albumCoverFor(album: AlbumEntry): string {
    const custom = getCustomCover('album', album.name);
    if (custom) return custom;
    return album.cover || defaultTrackCover;
}
function artistCoverFor(artist: ArtistEntry): string {
    const custom = getCustomCover('artist', artist.name);
    if (custom) return custom;
    return artist.cover || defaultTrackCover;
}

// ── Reusable horizontal-scroll helper ─────────────────────────────────────────

function useScrollLine(baseItemWidth = 240, gap = 16) {
    const el = ref<HTMLElement | null>(null);
    const scrollLeft = ref(0);
    const scrollMax = ref(0);
    const itemWidth = ref(baseItemWidth);
    const measured = ref(false);
    let ro: ResizeObserver | null = null;

    async function update() {
        const node = el.value;
        if (!node) return;
        const cw = node.clientWidth;
        if (cw > 0) {
            const n = Math.max(1, Math.floor((cw + gap) / (baseItemWidth + gap)));
            itemWidth.value = (cw - (n - 1) * gap) / n;
            measured.value = true;
        }
        await nextTick();
        scrollLeft.value = node.scrollLeft;
        scrollMax.value = Math.max(0, node.scrollWidth - node.clientWidth);
    }
    function onScroll() {
        const node = el.value;
        if (!node) return;
        scrollLeft.value = node.scrollLeft;
        scrollMax.value = Math.max(0, node.scrollWidth - node.clientWidth);
    }
    function scrollByPage(dir: 1 | -1) {
        const node = el.value;
        if (!node) return;
        node.scrollBy({ left: dir * node.clientWidth * 0.9, behavior: 'smooth' });
    }
    function observe() {
        const node = el.value;
        if (!node) return;
        update();
        if (!ro) {
            ro = new ResizeObserver(() => update());
            ro.observe(node);
        }
    }
    function unobserve() { ro?.disconnect(); ro = null; }

    const canPrev = computed(() => scrollLeft.value > 1);
    const canNext = computed(() => scrollLeft.value < scrollMax.value - 1);

    return { el, canPrev, canNext, itemWidth, measured, onScroll, scrollByPage, observe, unobserve, update };
}

const playlistsLine = useScrollLine(210, 16);
const albumsLine = useScrollLine(210, 16);
const artistsLine = useScrollLine(190, 16);

function refreshCoverFor(pl: PlaylistInfo) {
    if (pl.track_count === 0 || !pl.first_file_path) {
        delete playlistCovers.value[pl.id];
        return;
    }
    const inList = getTrackByPath(pl.first_file_path);
    if (inList) {
        if (inList.cover_art) {
            playlistCovers.value[pl.id] = inList.cover_art;
        } else {
            loadCoversForTracks([inList]);
        }
    }
}

function refreshAllCovers() {
    for (const pl of userPlaylists.value) {
        refreshCoverFor(pl);
    }
}

// ── Albums / Artists (data from backend playlists) ───────────────────────────

interface AlbumEntry {
    id: number;
    name: string;
    cover: string | null;
    artist: string;
    trackCount: number;
    firstPath: string | null;
}

interface ArtistEntry {
    id: number;
    name: string;
    cover: string | null;
    firstPath: string | null;
    trackCount: number;
}

const albums = computed<AlbumEntry[]>(() => {
    return albumPlaylists.value
        .map(pl => {
            const t = pl.first_file_path ? getTrackByPath(pl.first_file_path) : undefined;
            return {
                id: pl.id,
                name: pl.name,
                cover: t?.cover_art ?? null,
                artist: t?.artist || 'Неизвестный исполнитель',
                trackCount: pl.track_count,
                firstPath: pl.first_file_path,
            };
        })
        .sort((a, b) => a.name.localeCompare(b.name));
});

const artists = computed<ArtistEntry[]>(() => {
    return artistPlaylists.value
        .map(pl => {
            const t = pl.first_file_path ? getTrackByPath(pl.first_file_path) : undefined;
            return {
                id: pl.id,
                name: pl.name,
                cover: t?.cover_art ?? null,
                firstPath: pl.first_file_path,
                trackCount: pl.track_count,
            };
        })
        .sort((a, b) => b.trackCount - a.trackCount || a.name.localeCompare(b.name));
});

// ── Incremental rendering for the horizontal scrollers ─────────────────────
const VISIBLE_INITIAL = 18;
const VISIBLE_STEP = 12;
const visibleAlbumsCount = ref(VISIBLE_INITIAL);
const visibleArtistsCount = ref(VISIBLE_INITIAL);

function onAlbumsScroll() {
    albumsLine.onScroll();
    const node = albumsLine.el.value;
    if (!node) return;
    if (node.scrollLeft >= node.scrollWidth - node.clientWidth - 600) {
        if (visibleAlbumsCount.value < albums.value.length) {
            visibleAlbumsCount.value = Math.min(albums.value.length, visibleAlbumsCount.value + VISIBLE_STEP);
        }
    }
}
function onArtistsScroll() {
    artistsLine.onScroll();
    const node = artistsLine.el.value;
    if (!node) return;
    if (node.scrollLeft >= node.scrollWidth - node.clientWidth - 600) {
        if (visibleArtistsCount.value < artists.value.length) {
            visibleArtistsCount.value = Math.min(artists.value.length, visibleArtistsCount.value + VISIBLE_STEP);
        }
    }
}

// Cover requests only for currently rendered cards.
function requestCoversForCards() {
    const pending: TrackMetadata[] = [];
    for (const a of visibleSortedAlbums.value) {
        if (!a.firstPath) continue;
        const t = getTrackByPath(a.firstPath);
        if (t && !t.cover_art) pending.push(t);
    }
    for (const a of visibleArtists.value) {
        if (!a.firstPath) continue;
        const t = getTrackByPath(a.firstPath);
        if (t && !t.cover_art) pending.push(t);
    }
    if (pending.length) loadCoversForTracks(pending);
}

onMounted(async () => {
    playlistsLine.observe();
    albumsLine.observe();
    artistsLine.observe();
    await loadPlaylists();
    refreshAllCovers();
    requestCoversForCards();
});

onUnmounted(() => {
    playlistsLine.unobserve();
    albumsLine.unobserve();
    artistsLine.unobserve();
});

watch(
    () => userPlaylists.value.map(p => `${p.id}:${p.track_count}`).join(','),
    () => {
        refreshAllCovers();
        playlistsLine.update();
    },
);

watch(
    () => trackList.value.length,
    () => {
        for (const pl of userPlaylists.value) {
            if (pl.track_count === 0) continue;
            const current = playlistCovers.value[pl.id];
            if (current) continue;
            refreshCoverFor(pl);
        }
        requestCoversForCards();
    },
);

watch(
    () => albums.value.length + artists.value.length,
    async () => {
        await nextTick();
        albumsLine.observe();
        artistsLine.observe();
        requestCoversForCards();
    },
);

function nextPlaylistName(): string {
    const re = /^Новый плейлист\s+(\d+)$/;
    let maxN = 0;
    for (const pl of userPlaylists.value) {
        const m = pl.name.match(re);
        if (m) {
            const n = parseInt(m[1], 10);
            if (n > maxN) maxN = n;
        }
    }
    return `Новый плейлист ${maxN + 1}`;
}

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

function openAlbum(album: AlbumEntry) {
    router.push(`/album/${encodeURIComponent(album.name)}`);
}

// ── Hover overlay actions (playlists) ───────────────────────────────────────

const { isPinned: isPinnedFn, togglePin, unpin } = usePins();

function isPinned(id: number) {
    return isPinnedFn('playlist', id);
}

function togglePinId(id: number) {
    const pl = userPlaylists.value.find(p => p.id === id);
    if (!pl) return;
    togglePin({ kind: 'playlist', key: id, name: pl.name });
}

function isPlaylistActive(pl: PlaylistInfo) {
    return activePlaylistSource.value === `playlist:${pl.id}`;
}

async function onPlayPlaylist(e: MouseEvent, pl: PlaylistInfo) {
    e.stopPropagation();
    if (isPlaylistActive(pl)) {
        togglePlayPause();
        return;
    }
    const tracks = await getPlaylistTracks(pl.id);
    if (tracks.length === 0) return;
    const map = new Map(trackList.value.map(t => [t.file_path, t]));
    const merged = tracks.map(t => map.get(t.file_path) ?? t);
    setPlaylistSource(`playlist:${pl.id}`, merged);
    await playTrack(merged[0]);
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
    const btn = e.currentTarget as HTMLElement;
    const rect = btn.getBoundingClientRect();
    playlistMenu.value = { x: rect.left + rect.width / 2, y: rect.bottom + 6, pl, centerX: true };
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

// ── Albums: hover actions ───────────────────────────────────────────────────

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

function isAlbumActive(album: AlbumEntry) {
    return activePlaylistSource.value === `album:${album.name}`;
}

async function onPlayAlbum(e: MouseEvent, album: AlbumEntry) {
    e.stopPropagation();
    if (isAlbumActive(album)) {
        togglePlayPause();
        return;
    }
    if (album.trackCount === 0) return;
    const tracks = await getPlaylistTracks(album.id);
    if (tracks.length === 0) return;
    const map = new Map(trackList.value.map(t => [t.file_path, t]));
    const merged = tracks.map(t => map.get(t.file_path) ?? t);
    setPlaylistSource(`album:${album.name}`, merged);
    await playTrack(merged[0]);
}

const sortedAlbums = computed(() => {
    const all = albums.value;
    const byName = new Map(all.map(a => [a.name, a]));
    const topSet = new Set(albumTopOrder.value);
    const top: AlbumEntry[] = [];
    for (const name of albumTopOrder.value) {
        const a = byName.get(name);
        if (a) top.push(a);
    }
    const rest = all.filter(a => !topSet.has(a.name));
    return [...top, ...rest];
});

const sortedArtists = computed(() => {
    const all = artists.value;
    const byName = new Map(all.map(a => [a.name, a]));
    const topSet = new Set(artistTopOrder.value);
    const top: ArtistEntry[] = [];
    for (const name of artistTopOrder.value) {
        const a = byName.get(name);
        if (a) top.push(a);
    }
    const rest = all.filter(a => !topSet.has(a.name));
    return [...top, ...rest];
});

const visibleSortedAlbums = computed(() => sortedAlbums.value.slice(0, visibleAlbumsCount.value));
const visibleArtists = computed(() => sortedArtists.value.slice(0, visibleArtistsCount.value));

watch([visibleSortedAlbums, visibleArtists], () => requestCoversForCards());

// Hold back first paint until the first covers of each line have resolved, so
// the cards don't visibly resize/flash while their covers stream in. The lines
// stay mounted (just hidden) so they measure & request covers during the hold.
const { ready: coversReady } = useCoverGate(() => {
    const out: TrackMetadata[] = [];
    const pushFirst = (path: string | null) => {
        if (!path) return;
        const t = getTrackByPath(path);
        if (t) out.push(t);
    };
    // Cover enough of each row to fill a wide viewport so no cards visibly
    // stream their covers in after the reveal.
    for (const a of sortedAlbums.value.slice(0, 16)) pushFirst(a.firstPath);
    for (const a of sortedArtists.value.slice(0, 16)) pushFirst(a.firstPath);
    for (const pl of userPlaylists.value.slice(0, 16)) pushFirst(pl.first_file_path);
    return out;
}, { timeout: 10000 });

const albumMenu = ref<{ x: number; y: number; album: AlbumEntry; centerX: boolean } | null>(null);

function openAlbumMenu(e: MouseEvent, album: AlbumEntry) {
    e.stopPropagation();
    if (albumMenu.value?.album.name === album.name) {
        albumMenu.value = null;
        return;
    }
    const btn = e.currentTarget as HTMLElement;
    const rect = btn.getBoundingClientRect();
    albumMenu.value = { x: rect.left + rect.width / 2, y: rect.bottom + 6, album, centerX: true };
}

function openAlbumMenuFromCover(e: MouseEvent, album: AlbumEntry) {
    e.preventDefault();
    e.stopPropagation();
    albumMenu.value = { x: e.clientX, y: e.clientY, album, centerX: false };
}

function onMoveAlbumToTopFromMenu(album: AlbumEntry) {
    moveAlbumToTop(album.name);
}

// ── Artists: hover actions ──────────────────────────────────────────────────

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
    const map = new Map(trackList.value.map(t => [t.file_path, t]));
    const merged = tracks.map(t => map.get(t.file_path) ?? t);
    setPlaylistSource(`artist:${artist.name}`, merged);
    await playTrack(merged[0]);
}

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

function openArtistPage(artist: ArtistEntry) {
    router.push(`/artist/${encodeURIComponent(artist.name)}`);
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
            <MobileScrollHeader title="Коллекция" :scrolled="scrollY > 16" @top="scrollToTop" />
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div>Коллекция</div>
                    <button class="toTopBtn" @click="scrollToTop">Вверх</button>
                </div>
            </Transition>
        </div>
        <Transition name="gate-fade">
            <LoadingSpinner v-if="!coversReady" class="pageGateOverlay" />
        </Transition>
        <h1 class="pageTitle">Коллекция</h1>
        <div class="linesGate">
        <div class="linesContent" :class="{ gateHidden: !coversReady }">
        <div class="pageLine">
            <div class="lineHat">
                <div class="lineTitle" @click="router.push('/collections/playlists')">
                    Мои плейлисты
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                </div>
                <div class="lineControls">
                    <button class="prevBtn" :disabled="!playlistsLine.canPrev.value" @click="playlistsLine.scrollByPage(-1)"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                    <button class="nextBtn" :disabled="!playlistsLine.canNext.value" @click="playlistsLine.scrollByPage(1)"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                </div>
            </div>
            <div class="lineList" :ref="(el: any) => { playlistsLine.el.value = el as HTMLElement | null }" :style="{ '--card-w': playlistsLine.itemWidth.value + 'px', visibility: playlistsLine.measured.value ? 'visible' : 'hidden' }" @scroll="playlistsLine.onScroll">
                <div class="createPlaylistItem" @click="onCreatePlaylist">
                    <div class="cover"><PlusIcon width="64px" height="64px" stroke-width="1.1" /></div>
                    <div class="title">Новый плейлист</div>
                </div>
                <TransitionGroup name="playlist">
                    <div
                        v-for="pl in userPlaylists"
                        :key="pl.id"
                        class="playlistItem"
                        @click="openPlaylist(pl)"
                    >
                        <div
                            class="coverWrap"
                            :class="{ menuOpen: playlistMenu?.pl.id === pl.id }"
                            @contextmenu="openPlaylistMenuFromCover($event, pl)"
                        >
                            <AnimatedCover class="cover" :src="playlistCoverFor(pl)" />
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
        <div class="pageLine">
            <div class="lineHat">
                <div class="lineTitle" @click="router.push('/collections/albums')">
                    Альбомы
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                </div>
                <div class="lineControls">
                    <button class="prevBtn" :disabled="!albumsLine.canPrev.value" @click="albumsLine.scrollByPage(-1)"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                    <button class="nextBtn" :disabled="!albumsLine.canNext.value" @click="albumsLine.scrollByPage(1)"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                </div>
            </div>
            <div v-if="sortedAlbums.length === 0" class="emptyRow">Пока нет альбомов</div>
            <div v-else class="lineList" :ref="(el: any) => { albumsLine.el.value = el as HTMLElement | null }" :style="{ '--card-w': albumsLine.itemWidth.value + 'px', visibility: albumsLine.measured.value ? 'visible' : 'hidden' }" @scroll="onAlbumsScroll">
                <div
                    v-for="album in visibleSortedAlbums"
                    :key="album.id"
                    class="playlistItem albumItem"
                    @click="openAlbum(album)"
                >
                    <div
                        class="coverWrap"
                        :class="{ menuOpen: albumMenu?.album.name === album.name }"
                        @contextmenu="openAlbumMenuFromCover($event, album)"
                    >
                        <AnimatedCover class="cover" :src="albumCoverFor(album)" />
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
                    <div v-truncate class="subtitle">{{ album.artist }}</div>
                </div>
            </div>
        </div>
        <div class="pageLine">
            <div class="lineHat">
                <div class="lineTitle" @click="router.push('/collections/artists')">
                    Исполнители
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                </div>
                <div class="lineControls">
                    <button class="prevBtn" :disabled="!artistsLine.canPrev.value" @click="artistsLine.scrollByPage(-1)"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                    <button class="nextBtn" :disabled="!artistsLine.canNext.value" @click="artistsLine.scrollByPage(1)"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                </div>
            </div>
            <div v-if="artists.length === 0" class="emptyRow">Пока нет исполнителей</div>
            <div v-else class="lineList artistLineList" :ref="(el: any) => { artistsLine.el.value = el as HTMLElement | null }" :style="{ '--card-w': artistsLine.itemWidth.value + 'px', visibility: artistsLine.measured.value ? 'visible' : 'hidden' }" @scroll="onArtistsScroll">
                <div
                    v-for="artist in visibleArtists"
                    :key="artist.id"
                    class="artistItem"
                    @click="openArtistPage(artist)"
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
        </div>
        </div>
        <PlaylistContextMenu
            v-if="playlistMenu"
            :x="playlistMenu.x"
            :y="playlistMenu.y"
            :isPinned="isPinned(playlistMenu.pl.id)"
            showMoveToTop
            :centerX="playlistMenu.centerX"
            @close="playlistMenu = null"
            @togglePin="togglePinId(playlistMenu!.pl.id)"
            @delete="onDeleteFromMenu(playlistMenu!.pl)"
            @moveToTop="onMoveToTopFromMenu(playlistMenu!.pl)"
        />
        <PlaylistContextMenu
            v-if="albumMenu"
            :x="albumMenu.x"
            :y="albumMenu.y"
            :isPinned="isAlbumPinned(albumMenu.album.name)"
            showMoveToTop
            :showDelete="false"
            :centerX="albumMenu.centerX"
            @close="albumMenu = null"
            @togglePin="toggleAlbumPin(albumMenu!.album.name)"
            @moveToTop="onMoveAlbumToTopFromMenu(albumMenu!.album)"
        />
        <PlaylistContextMenu
            v-if="artistMenu"
            :x="artistMenu.x"
            :y="artistMenu.y"
            :isPinned="isArtistPinned(artistMenu.artist.name)"
            showMoveToTop
            :showDelete="false"
            :centerX="artistMenu.centerX"
            @close="artistMenu = null"
            @togglePin="toggleArtistPin(artistMenu!.artist.name)"
            @moveToTop="onMoveArtistToTopFromMenu(artistMenu!.artist)"
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

    overflow-y: scroll;
}
.pageGateOverlay {
    position: absolute;
    inset: 0;
    z-index: 5;
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
.pageTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 900;
    font-size: 36px;
    color: #ffffff;
    margin-bottom: 48px;
}
.linesGate {
    position: relative;
    display: flex;
    flex-direction: column;
    flex: 1;
}
.linesContent {
    display: flex;
    flex-direction: column;
    transition: opacity 0.24s ease;
}
.linesContent.gateHidden {
    opacity: 0;
    pointer-events: none;
    /* Take the content out of flow while hidden so .linesGate collapses to the
       visible viewport height and the spinner centers in the viewframe. */
    position: absolute;
    inset: 0;
}
.pageLine {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 13px;
    margin-bottom: 36px;
}
.pageLine:last-child {
    margin-bottom: 0px;
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
.emptyRow {
    user-select: none;
    padding: 40px 4px;
    color: hsla(0, 0%, 100%, 0.35);
    font-size: 14px;
    font-weight: 500;
}
.createPlaylistItem {
    flex-shrink: 0;
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
    width: var(--card-w, 240px);
    height: var(--card-w, 240px);
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
    flex-shrink: 0;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
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
    width: var(--card-w, 240px);
    height: var(--card-w, 240px);
    border-radius: 18px;
    overflow: hidden;
}
.playlistItem .cover {
    display: block;
    width: var(--card-w, 240px);
    height: var(--card-w, 240px);
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
    max-width: var(--card-w, 240px);
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
    max-width: var(--card-w, 240px);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

/* Artists ── circular covers with hover overlay (mirrors album cards) */
.artistLineList {
    padding-bottom: 16px;
}
.artistItem {
    flex-shrink: 0;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
    user-select: none;
}
.artistCoverWrap {
    position: relative;
    width: var(--card-w, 200px);
    height: var(--card-w, 200px);
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
    max-width: var(--card-w, 200px);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
</style>
