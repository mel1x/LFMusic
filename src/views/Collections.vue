<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import PlusIcon from '../components/Icons/PlusIcon.vue';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import PinIcon from '../components/Icons/PinIcon.vue';
import MoreIcon from '../components/Icons/MoreIcon.vue';
import PlaylistContextMenu from '../components/PlaylistContextMenu.vue';
import defaultCover from '../assets/playlistCover.webp';
import { usePlaylists, type PlaylistInfo } from '../composables/usePlaylists';
import { usePlayer } from '../composables/usePlayer';

const router = useRouter();
const { userPlaylists, loadPlaylists, createPlaylist, getPlaylistTracks, deletePlaylist, movePlaylistToTop } = usePlaylists();
const {
    trackList,
    loadCoversForTracks,
    isPlaying,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    setPlaylistSource,
} = usePlayer();

const playlistCovers = ref<Record<number, string | null>>({});

const lineListEl = ref<HTMLElement | null>(null);
const scrollLeft = ref(0);
const scrollMax = ref(0);
let lineResizeObserver: ResizeObserver | null = null;

function updateScrollBounds() {
    const el = lineListEl.value;
    if (!el) return;
    scrollLeft.value = el.scrollLeft;
    scrollMax.value = Math.max(0, el.scrollWidth - el.clientWidth);
}

function onLineScroll() {
    updateScrollBounds();
}

function scrollByPage(direction: 1 | -1) {
    const el = lineListEl.value;
    if (!el) return;
    el.scrollBy({ left: direction * el.clientWidth * 0.9, behavior: 'smooth' });
}

const canScrollPrev = computed(() => scrollLeft.value > 1);
const canScrollNext = computed(() => scrollLeft.value < scrollMax.value - 1);

async function refreshCoverFor(pl: PlaylistInfo) {
    if (pl.track_count === 0) {
        playlistCovers.value[pl.id] = null;
        return;
    }
    const tracks = await getPlaylistTracks(pl.id);
    const first = tracks[0];
    if (!first) {
        playlistCovers.value[pl.id] = null;
        return;
    }
    const inList = trackList.value.find(t => t.file_path === first.file_path);
    if (inList) {
        if (!inList.cover_art) loadCoversForTracks([inList]);
        playlistCovers.value[pl.id] = inList.cover_art;
    } else {
        playlistCovers.value[pl.id] = first.cover_art;
    }
}

async function refreshAllCovers() {
    for (const pl of userPlaylists.value) {
        await refreshCoverFor(pl);
    }
}

onMounted(async () => {
    await loadPlaylists();
    await refreshAllCovers();
    if (lineListEl.value) {
        updateScrollBounds();
        lineResizeObserver = new ResizeObserver(updateScrollBounds);
        lineResizeObserver.observe(lineListEl.value);
    }
});

onUnmounted(() => {
    lineResizeObserver?.disconnect();
});

watch(
    () => userPlaylists.value.map(p => `${p.id}:${p.track_count}`).join(','),
    async () => {
        await refreshAllCovers();
        updateScrollBounds();
    },
);

watch(
    () => trackList.value.map(t => t.cover_art ? '1' : '0').join(''),
    () => {
        for (const pl of userPlaylists.value) {
            if (pl.track_count === 0) continue;
            const current = playlistCovers.value[pl.id];
            if (current) continue;
            refreshCoverFor(pl);
        }
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

async function onCreatePlaylist() {
    const name = nextPlaylistName();
    const pl = await createPlaylist(name);
    if (pl) {
        router.push(`/playlist/${pl.id}`);
    }
}

function openPlaylist(pl: PlaylistInfo) {
    router.push(`/playlist/${pl.id}`);
}

// ── Hover overlay actions ───────────────────────────────────────────────────

const pinnedIds = ref<number[]>([]);

function isPinned(id: number) {
    return pinnedIds.value.includes(id);
}

function togglePinId(id: number) {
    const i = pinnedIds.value.indexOf(id);
    if (i >= 0) pinnedIds.value.splice(i, 1);
    else pinnedIds.value.push(id);
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

async function onDeleteFromMenu(pl: PlaylistInfo) {
    const i = pinnedIds.value.indexOf(pl.id);
    if (i >= 0) pinnedIds.value.splice(i, 1);
    await deletePlaylist(pl.id);
}

function onMoveToTopFromMenu(pl: PlaylistInfo) {
    movePlaylistToTop(pl.id);
    lineListEl.value?.scrollTo({ left: 0, behavior: 'smooth' });
}
</script>

<template>
    <div class="pageRoot">
        <h1 class="pageTitle">Коллекция</h1>
        <div class="pageLine">
            <div class="lineHat">
                <div class="lineTitle">
                    Мои плейлисты
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                </div>
                <div class="lineControls">
                    <button class="prevBtn" :disabled="!canScrollPrev" @click="scrollByPage(-1)"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                    <button class="nextBtn" :disabled="!canScrollNext" @click="scrollByPage(1)"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                </div>
            </div>
            <div class="lineList" ref="lineListEl" @scroll="onLineScroll">
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
                            <img class="cover" :src="playlistCovers[pl.id] || defaultCover" />
                            <button
                                class="overlayBtn pinBtn"
                                :class="{ active: isPinned(pl.id) }"
                                @click="onTogglePinClick($event, pl)"
                                @contextmenu.stop
                                :title="isPinned(pl.id) ? 'Открепить' : 'Закрепить'"
                            >
                                <PinIcon width="16" height="16" />
                            </button>
                            <button
                                class="overlayBtn playBtn"
                                :disabled="pl.track_count === 0"
                                @click="onPlayPlaylist($event, pl)"
                                @contextmenu.stop
                                :title="pl.track_count === 0 ? 'Плейлист пуст' : (isPlaylistActive(pl) && isPlaying ? 'Пауза' : 'Слушать')"
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
                                title="Меню"
                            >
                                <MoreIcon width="20" height="20" />
                            </button>
                        </div>
                        <div class="title">{{ pl.name }}</div>
                    </div>
                </TransitionGroup>
            </div>
        </div>
        <div class="pageLine">
            <div class="lineHat">
                <div class="lineTitle">
                    Альбомы
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                </div>
                <div class="lineControls">
                    <button class="prevBtn"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                    <button class="nextBtn"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                </div>
            </div>
            <div class="lineList">
                <h1 style="color:#fff;">Soon...</h1>
            </div>
        </div>
        <div class="pageLine">
            <div class="lineHat">
                <div class="lineTitle">
                    Исполнители
                    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                </div>
                <div class="lineControls">
                    <button class="prevBtn"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                    <button class="nextBtn"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                </div>
            </div>
            <div class="lineList">
                <h1 style="color:#fff;">Soon...</h1>
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
}
.pageRoot::-webkit-scrollbar { width: 6px; }
.pageRoot::-webkit-scrollbar-track { background: transparent; }
.pageRoot::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
}
.pageTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 900;
    font-size: 36px;
    color: #ffffff;
    margin-bottom: 48px;
}
.pageLine {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 13px;
    margin-bottom: 36px;
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
    width: 240px;
    height: 240px;
    border-radius: 12px;
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
    width: 240px;
    height: 240px;
    border-radius: 12px;
    overflow: hidden;
}
.playlistItem .cover {
    display: block;
    width: 240px;
    height: 240px;
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
    cursor: not-allowed;
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
    max-width: 240px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
</style>
