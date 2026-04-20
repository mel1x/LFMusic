<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import SearchIcon from '../components/Icons/SearchIcon.vue';
import EmptyIcon from '../components/Icons/EmptyIcon.vue';
import defaultTrackCover from '../assets/trackCover.webp';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';

const router = useRouter();
const { trackList, loadCoversForTracks } = usePlayer();

interface ArtistEntry {
    name: string;
    tracks: TrackMetadata[];
    cover: string | null;
}

const artists = computed<ArtistEntry[]>(() => {
    const map = new Map<string, TrackMetadata[]>();
    for (const t of trackList.value) {
        const name = (t.artist || '').trim();
        if (!name) continue;
        const arr = map.get(name);
        if (arr) arr.push(t);
        else map.set(name, [t]);
    }
    const out: ArtistEntry[] = [];
    for (const [name, tracks] of map) {
        out.push({
            name,
            tracks,
            cover: tracks.find(t => t.cover_art)?.cover_art ?? null,
        });
    }
    return out.sort((a, b) => a.name.localeCompare(b.name));
});

function requestCoversForArtists() {
    const pending: TrackMetadata[] = [];
    for (const a of artists.value) {
        if (!a.cover && a.tracks[0]) pending.push(a.tracks[0]);
    }
    if (pending.length) loadCoversForTracks(pending);
}

watch(artists, () => requestCoversForArtists(), { immediate: true });

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

const filteredArtists = computed(() => {
    const q = debouncedSearch.value.toLowerCase().trim();
    if (!q) return artists.value;
    return artists.value.filter(a => a.name.toLowerCase().includes(q));
});

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
        <div class="grid" v-if="filteredArtists.length > 0">
            <div
                v-for="artist in filteredArtists"
                :key="artist.name"
                class="artistItem"
            >
                <div class="artistCoverWrap">
                    <img class="artistCover" :src="artist.cover || defaultTrackCover" draggable="false" />
                </div>
                <div class="artistTitle">{{ artist.name }}</div>
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
    --card-w: 170px;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--card-w), 1fr));
    gap: 24px 16px;
    padding-bottom: 24px;
}

.artistItem {
    display: flex;
    flex-direction: column;
    gap: 8px;
    user-select: none;
    min-width: 0;
}
.artistCoverWrap {
    width: 100%;
    aspect-ratio: 1 / 1;
    border-radius: 9999px;
    overflow: hidden;
    background-color: #202020;
    transition: transform .24s ease;
}
.artistItem:hover .artistCoverWrap {
    transform: scale(1.015);
}
.artistCover {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    user-select: none;
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
