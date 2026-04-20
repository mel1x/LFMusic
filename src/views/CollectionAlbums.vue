<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import PinIcon from '../components/Icons/PinIcon.vue';
import MoreIcon from '../components/Icons/MoreIcon.vue';
import SearchIcon from '../components/Icons/SearchIcon.vue';
import EmptyIcon from '../components/Icons/EmptyIcon.vue';
import PlaylistContextMenu from '../components/PlaylistContextMenu.vue';
import defaultTrackCover from '../assets/trackCover.webp';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';

const router = useRouter();
const {
    trackList,
    loadCoversForTracks,
    isPlaying,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    setPlaylistSource,
} = usePlayer();

interface AlbumEntry {
    name: string;
    tracks: TrackMetadata[];
    cover: string | null;
    artist: string;
}

const albums = computed<AlbumEntry[]>(() => {
    const map = new Map<string, TrackMetadata[]>();
    for (const t of trackList.value) {
        const name = (t.album || '').trim();
        if (!name) continue;
        const arr = map.get(name);
        if (arr) arr.push(t);
        else map.set(name, [t]);
    }
    const out: AlbumEntry[] = [];
    for (const [name, tracks] of map) {
        if (tracks.length < 2) continue;
        out.push({
            name,
            tracks,
            cover: tracks.find(t => t.cover_art)?.cover_art ?? null,
            artist: tracks[0].artist || 'Неизвестный исполнитель',
        });
    }
    return out.sort((a, b) => a.name.localeCompare(b.name));
});

function requestCoversForAlbums() {
    const pending: TrackMetadata[] = [];
    for (const a of albums.value) {
        if (!a.cover && a.tracks[0]) pending.push(a.tracks[0]);
    }
    if (pending.length) loadCoversForTracks(pending);
}

watch(albums, () => requestCoversForAlbums(), { immediate: true });

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

// ── Pinned / top order ─────────────────────────────────────────────────────

const pinnedAlbums = ref<string[]>([]);
const albumTopOrder = ref<string[]>([]);

function isAlbumPinned(name: string) {
    return pinnedAlbums.value.includes(name);
}

function toggleAlbumPin(name: string) {
    const i = pinnedAlbums.value.indexOf(name);
    if (i >= 0) pinnedAlbums.value.splice(i, 1);
    else pinnedAlbums.value.push(name);
}

function onToggleAlbumPinClick(e: MouseEvent, album: AlbumEntry) {
    e.stopPropagation();
    toggleAlbumPin(album.name);
}

function moveAlbumToTop(name: string) {
    const i = albumTopOrder.value.indexOf(name);
    if (i >= 0) albumTopOrder.value.splice(i, 1);
    albumTopOrder.value.unshift(name);
}

const sortedAlbums = computed(() => {
    const all = albums.value;
    const topSet = new Set(albumTopOrder.value);
    const top: AlbumEntry[] = [];
    for (const name of albumTopOrder.value) {
        const a = all.find(al => al.name === name);
        if (a) top.push(a);
    }
    const rest = all.filter(a => !topSet.has(a.name));
    return [...top, ...rest];
});

const filteredAlbums = computed(() => {
    const q = debouncedSearch.value.toLowerCase().trim();
    if (!q) return sortedAlbums.value;
    return sortedAlbums.value.filter(a =>
        a.name.toLowerCase().includes(q) || a.artist.toLowerCase().includes(q),
    );
});

// ── Actions ─────────────────────────────────────────────────────────────────

function isAlbumActive(album: AlbumEntry) {
    return activePlaylistSource.value === `album:${album.name}`;
}

async function onPlayAlbum(e: MouseEvent, album: AlbumEntry) {
    e.stopPropagation();
    if (isAlbumActive(album)) {
        togglePlayPause();
        return;
    }
    if (album.tracks.length === 0) return;
    setPlaylistSource(`album:${album.name}`, album.tracks);
    await playTrack(album.tracks[0]);
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

function goBack() {
    router.push('/collections');
}
</script>

<template>
    <div class="pageRoot">
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
        <div class="grid" v-if="filteredAlbums.length > 0">
            <div
                v-for="album in filteredAlbums"
                :key="album.name"
                class="playlistItem albumItem"
                @click="openAlbum(album)"
            >
                <div
                    class="coverWrap"
                    :class="{ menuOpen: albumMenu?.album.name === album.name }"
                    @contextmenu="openAlbumMenuFromCover($event, album)"
                >
                    <img class="cover" :src="album.cover || defaultTrackCover" draggable="false" />
                    <button
                        class="overlayBtn pinBtn"
                        :class="{ active: isAlbumPinned(album.name) }"
                        @click="onToggleAlbumPinClick($event, album)"
                        @contextmenu.stop
                        :title="isAlbumPinned(album.name) ? 'Открепить' : 'Закрепить'"
                    >
                        <PinIcon width="16" height="16" />
                    </button>
                    <button
                        class="overlayBtn playBtn"
                        :disabled="album.tracks.length === 0"
                        @click="onPlayAlbum($event, album)"
                        @contextmenu.stop
                        :title="isAlbumActive(album) && isPlaying ? 'Пауза' : 'Слушать'"
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
                        title="Меню"
                    >
                        <MoreIcon width="20" height="20" />
                    </button>
                </div>
                <div class="title">{{ album.name }}</div>
                <div class="subtitle">{{ album.artist }}</div>
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

.grid {
    --card-w: 200px;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--card-w), 1fr));
    gap: 24px 16px;
    padding-bottom: 24px;
}

.playlistItem {
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
}
.playlistItem .coverWrap {
    position: relative;
    width: 100%;
    aspect-ratio: 1 / 1;
    border-radius: 12px;
    overflow: hidden;
}
.playlistItem .cover {
    display: block;
    width: 100%;
    height: 100%;
    border-radius: 12px;
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
