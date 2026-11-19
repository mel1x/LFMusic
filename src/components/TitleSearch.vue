<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from 'vue';
import { useRouter } from 'vue-router';
import { invoke } from '@tauri-apps/api/core';

import SearchIcon from './Icons/SearchIcon.vue';
import PlayIcon from './Icons/PlayIcon.vue';
import PauseIcon from './Icons/PauseIcon.vue';
import AnimatedCover from './AnimatedCover.vue';
import TrackContextMenu from './TrackContextMenu.vue';

import defaultTrackCover from '../assets/trackCover.webp';
import defaultPlaylistCover from '../assets/playlistCover.webp';

import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { usePlaySource } from '../composables/usePlaySource';
import { usePlaylists, type PlaylistInfo } from '../composables/usePlaylists';
import { useCustomCovers } from '../composables/useCustomCovers';
import { useExternalSources } from '../composables/useExternalSources';
import { useSettings } from '../composables/useSettings';
import { useI18n } from '../composables/useI18n';
import { isPhoneViewport } from '../composables/usePlatform';
import { searchRanked } from '../composables/useSearch';
import { tracksCountText, joinArtists } from '../utils/format';

type Filter = 'all' | 'tracks' | 'artists' | 'albums' | 'playlists';

const router = useRouter();
const { t } = useI18n();
const { settings } = useSettings();
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
} = usePlayer();
const { playSource } = usePlaySource();
const { userPlaylists, albums, artists, loadPlaylists } = usePlaylists();
const { getCustomCover } = useCustomCovers();
const { avatarFor } = useExternalSources();

const open = ref(false);
const inputEl = ref<HTMLInputElement | null>(null);
const resultsEl = ref<HTMLElement | null>(null);

function openSearch() {
    if (open.value) {
        inputEl.value?.focus();
        return;
    }
    open.value = true;
    void loadPlaylists();
    nextTick(() => {
        inputEl.value?.focus();
        inputEl.value?.select();
    });
}

function closeSearch() {
    open.value = false;
    trackMenu.value = null;
}

watch(() => router.currentRoute.value.fullPath, () => closeSearch());

function onGlobalKeydown(e: KeyboardEvent) {
    if (open.value && e.key === 'Escape' && e.target !== inputEl.value && !trackMenu.value) {
        e.preventDefault();
        closeSearch();
        return;
    }
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.code === 'KeyK') {
        if (isPhoneViewport()) return;
        e.preventDefault();
        openSearch();
    }
}
onMounted(() => window.addEventListener('keydown', onGlobalKeydown));
onUnmounted(() => window.removeEventListener('keydown', onGlobalKeydown));

const rawQuery = ref('');
const query = ref('');
const filter = ref<Filter>('all');
let debounce: ReturnType<typeof setTimeout> | null = null;

watch(rawQuery, (v) => {
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(() => { query.value = v.trim(); }, 160);
});

function clearQuery() {
    rawQuery.value = '';
    query.value = '';
    filter.value = 'all';
    inputEl.value?.focus();
}

watch(query, () => {
    trackLimit.value = PAGE;
    activeIndex.value = -1;
    resultsEl.value?.scrollTo({ top: 0 });
});
watch(filter, () => {
    trackLimit.value = PAGE;
    activeIndex.value = -1;
    resultsEl.value?.scrollTo({ top: 0 });
});

const PAST_KEY = 'lfmusic_past_searches';
const pastSearches = ref<string[]>([]);
function loadPast() {
    try {
        const raw = localStorage.getItem(PAST_KEY);
        pastSearches.value = raw ? JSON.parse(raw) : [];
    } catch {
        pastSearches.value = [];
    }
}
function rememberQuery() {
    const q = rawQuery.value.trim();
    if (!q) return;
    const list = [q, ...pastSearches.value.filter(p => p !== q)].slice(0, 15);
    pastSearches.value = list;
    try { localStorage.setItem(PAST_KEY, JSON.stringify(list)); } catch {}
}

const hintIndex = ref(0);
let hintTimer: ReturnType<typeof setInterval> | null = null;
const hint = computed(() => pastSearches.value.length
    ? pastSearches.value[hintIndex.value % pastSearches.value.length]
    : '');

function stopHints() {
    if (hintTimer) clearInterval(hintTimer);
    hintTimer = null;
}
watch(open, (v) => {
    stopHints();
    if (!v) return;
    loadPast();
    hintIndex.value = 0;
    hintTimer = setInterval(() => {
        if (!rawQuery.value && pastSearches.value.length > 1) hintIndex.value++;
    }, 2600);
});
onUnmounted(stopHints);

function useHint() {
    rawQuery.value = hint.value;
    query.value = hint.value;
}

const trackResults = ref<TrackMetadata[]>([]);
const searching = ref(false);
let reqId = 0;

watch(query, async (q) => {
    const id = ++reqId;
    if (!q) {
        trackResults.value = [];
        searching.value = false;
        return;
    }
    searching.value = true;
    try {
        const res = await invoke<TrackMetadata[]>('library_search', { query: q });
        if (id !== reqId) return;
        cacheTracks(res);
        trackResults.value = res.map(tr => getTrackByPath(tr.file_path) ?? tr);
    } catch {
        if (id === reqId) trackResults.value = [];
    } finally {
        if (id === reqId) searching.value = false;
    }
});

interface CollectionResult {
    kind: 'artist' | 'album' | 'playlist';
    id: number;
    name: string;
    sub: string;
    count: number;
    firstPath: string | null;
}

function firstTrack(path: string | null) {
    return path ? getTrackByPath(path) : undefined;
}

const artistResults = computed<CollectionResult[]>(() => {
    if (!query.value) return [];
    const list = artists.value.map(pl => ({
        kind: 'artist' as const, id: pl.id, name: pl.name, sub: t('globalSearch.artist'),
        count: pl.track_count, firstPath: pl.first_file_path,
    }));
    return searchRanked(list, query.value, a => [a.name]);
});

const albumResults = computed<CollectionResult[]>(() => {
    if (!query.value) return [];
    const list = albums.value.map(pl => {
        const artist = firstTrack(pl.first_file_path)?.artist;
        return {
            kind: 'album' as const, id: pl.id, name: pl.name,
            sub: artist ? joinArtists(artist, settings.value.artist_separators) : t('globalSearch.album'),
            count: pl.track_count, firstPath: pl.first_file_path,
        };
    });
    return searchRanked(list, query.value, a => [a.name, a.sub]);
});

const playlistResults = computed<CollectionResult[]>(() => {
    if (!query.value) return [];
    const list = userPlaylists.value.map((pl: PlaylistInfo) => ({
        kind: 'playlist' as const, id: pl.id, name: pl.name, sub: t('globalSearch.playlist'),
        count: pl.track_count, firstPath: pl.first_file_path,
    }));
    return searchRanked(list, query.value, p => [p.name]);
});

const PAGE = 40;
const ALL_TRACKS = 5;
const ALL_OTHERS = 3;
const trackLimit = ref(PAGE);

const shownTracks = computed(() => filter.value === 'all'
    ? trackResults.value.slice(0, ALL_TRACKS)
    : filter.value === 'tracks' ? trackResults.value.slice(0, trackLimit.value) : []);

function shownOf(kind: Filter, list: CollectionResult[]) {
    if (filter.value === 'all') return list.slice(0, ALL_OTHERS);
    return filter.value === kind ? list : [];
}
const shownArtists = computed(() => shownOf('artists', artistResults.value));
const shownAlbums = computed(() => shownOf('albums', albumResults.value));
const shownPlaylists = computed(() => shownOf('playlists', playlistResults.value));

const sections = computed(() => [
    { kind: 'artists', label: t('globalSearch.filter.artists'), list: shownArtists.value },
    { kind: 'albums', label: t('globalSearch.filter.albums'), list: shownAlbums.value },
    { kind: 'playlists', label: t('globalSearch.filter.playlists'), list: shownPlaylists.value },
].filter(s => s.list.length > 0));

const moreTracks = computed(() => filter.value === 'all'
    ? trackResults.value.length > ALL_TRACKS
    : filter.value === 'tracks' && trackResults.value.length > trackLimit.value);

function showMoreTracks() {
    if (filter.value === 'all') filter.value = 'tracks';
    else trackLimit.value += PAGE;
}

const hasAny = computed(() => trackResults.value.length + artistResults.value.length
    + albumResults.value.length + playlistResults.value.length > 0);
const showSkeleton = computed(() => searching.value && !hasAny.value);
const nothingFound = computed(() => !!query.value && !searching.value && !hasAny.value);

const filters = computed(() => {
    const all: { id: Filter; n: number }[] = [
        { id: 'tracks', n: trackResults.value.length },
        { id: 'artists', n: artistResults.value.length },
        { id: 'albums', n: albumResults.value.length },
        { id: 'playlists', n: playlistResults.value.length },
    ];
    return [{ id: 'all' as Filter, n: 1 }, ...all.filter(f => f.n > 0)];
});

watch([shownTracks, shownArtists, shownAlbums, shownPlaylists], () => {
    const pending: TrackMetadata[] = shownTracks.value.filter(tr => !tr.cover_art);
    for (const c of [...shownArtists.value, ...shownAlbums.value, ...shownPlaylists.value]) {
        const tr = firstTrack(c.firstPath);
        if (tr && !tr.cover_art) pending.push(tr);
    }
    if (pending.length) loadCoversForTracks(pending);
});

function coverOf(c: CollectionResult): string {
    const tr = firstTrack(c.firstPath);
    if (c.kind === 'artist') return getCustomCover('artist', c.name) || avatarFor(c.name) || tr?.cover_art || defaultTrackCover;
    if (c.kind === 'album') return getCustomCover('album', c.name) || tr?.cover_art || defaultTrackCover;
    return getCustomCover('playlist', c.id) || tr?.cover_art || defaultPlaylistCover;
}

function trackLine(tr: TrackMetadata): string {
    const artist = tr.artist ? joinArtists(tr.artist, settings.value.artist_separators) : t('recap.unknownArtist');
    return tr.album ? `${artist} - ${tr.album}` : artist;
}

function trackTitle(tr: TrackMetadata): string {
    return tr.title || tr.file_path.split(/[\\/]/).pop()?.replace(/\.[^/.]+$/, '') || t('player.untitled');
}

function duration(secs: number | null | undefined): string {
    if (!secs || secs <= 0) return '';
    const m = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    return `${m}:${String(s).padStart(2, '0')}`;
}

function isCurrent(tr: TrackMetadata) {
    return currentTrack.value?.file_path === tr.file_path;
}

function playFromResults(tr: TrackMetadata) {
    if (isCurrent(tr)) {
        togglePlayPause();
        return;
    }
    rememberQuery();
    setPlaylistSource(`search:tracks:${query.value}`, trackResults.value);
    playTrack(tr, { searchStart: true, manualListStart: true });
}

function sourceId(c: CollectionResult) {
    return c.kind === 'playlist' ? `playlist:${c.id}` : `${c.kind}:${c.name}`;
}

function isSourcePlaying(c: CollectionResult) {
    return activePlaylistSource.value === sourceId(c) && isPlaying.value;
}

async function playCollection(e: MouseEvent, c: CollectionResult) {
    e.stopPropagation();
    if (c.count === 0) return;
    await playSource(sourceId(c), c.id, { searchStart: true }, rememberQuery);
}

function openCollection(c: CollectionResult) {
    rememberQuery();
    const path = c.kind === 'playlist' ? `/playlist/${c.id}` : `/${c.kind}/${encodeURIComponent(c.name)}`;
    if (router.currentRoute.value.fullPath === path) closeSearch();
    else void router.push(path);
}

const trackMenu = ref<{ x: number; y: number; track: TrackMetadata } | null>(null);
function openTrackMenu(e: MouseEvent, tr: TrackMetadata) {
    rememberQuery();
    trackMenu.value = { x: e.clientX, y: e.clientY, track: tr };
}

type Item = { type: 'track'; track: TrackMetadata } | { type: 'collection'; c: CollectionResult };

const items = computed<Item[]>(() => [
    ...shownTracks.value.map(track => ({ type: 'track' as const, track })),
    ...sections.value.flatMap(s => s.list).map(c => ({ type: 'collection' as const, c })),
]);
const activeIndex = ref(-1);

function indexOfTrack(tr: TrackMetadata) {
    return items.value.findIndex(it => it.type === 'track' && it.track.file_path === tr.file_path);
}
function indexOfCollection(c: CollectionResult) {
    return items.value.findIndex(it => it.type === 'collection' && it.c.kind === c.kind && it.c.id === c.id);
}

function activate(it: Item) {
    if (it.type === 'track') playFromResults(it.track);
    else openCollection(it.c);
}

function onInputKeydown(e: KeyboardEvent) {
    const n = items.value.length;
    if (e.key === 'Tab' && !rawQuery.value && hint.value) {
        e.preventDefault();
        useHint();
    } else if (e.key === 'Escape') {
        e.preventDefault();
        if (rawQuery.value) clearQuery();
        else closeSearch();
    } else if (e.key === 'ArrowDown' && n) {
        e.preventDefault();
        activeIndex.value = (activeIndex.value + 1) % n;
        scrollActiveIntoView();
    } else if (e.key === 'ArrowUp' && n) {
        e.preventDefault();
        activeIndex.value = activeIndex.value <= 0 ? n - 1 : activeIndex.value - 1;
        scrollActiveIntoView();
    } else if (e.key === 'Enter') {
        const it = items.value[activeIndex.value >= 0 ? activeIndex.value : 0];
        if (it) {
            e.preventDefault();
            activate(it);
        } else {
            rememberQuery();
        }
    }
}

function scrollActiveIntoView() {
    nextTick(() => {
        resultsEl.value?.querySelector('.gsRow.active')?.scrollIntoView({ block: 'nearest' });
    });
}
</script>

<template>
    <button type="button" class="gsPill" @click="openSearch">
        <SearchIcon width="16" height="16" />
        <span class="gsPillText">{{ t('globalSearch.pill') }}</span>
        <span class="gsPillKey">Ctrl K</span>
    </button>

    <Teleport to="body">
        <Transition name="gsFade">
            <div v-if="open" class="gsBackdrop" @mousedown.self="closeSearch">
                <div class="gsPanel" role="dialog" aria-modal="true" :aria-label="t('globalSearch.pill')">
                    <div class="gsField">
                        <span v-if="searching" class="gsSpinner" aria-hidden="true"></span>
                        <SearchIcon v-else class="gsFieldIcon" width="18" height="18" />
                        <div class="gsInputWrap">
                            <input
                                ref="inputEl"
                                v-model="rawQuery"
                                type="text"
                                class="gsInput"
                                :aria-label="t('globalSearch.placeholder')"
                                autocomplete="off"
                                spellcheck="false"
                                @keydown="onInputKeydown"
                            />
                            <span v-if="!rawQuery" class="gsHint" aria-hidden="true">
                                <Transition name="gsHintSwap" mode="out-in">
                                    <span :key="hint || '-'" class="gsHintText">{{ hint || t('globalSearch.placeholder') }}</span>
                                </Transition>
                            </span>
                        </div>
                        <span v-if="!rawQuery && hint" class="gsKey">Tab</span>
                        <button v-if="rawQuery" type="button" class="gsClear" :aria-label="t('globalSearch.clear')" @click="clearQuery">
                            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg>
                        </button>
                    </div>

                    <div v-if="query" class="gsBody">
                        <div v-if="hasAny" class="gsChips">
                            <button
                                v-for="f in filters"
                                :key="f.id"
                                type="button"
                                class="gsChip"
                                :class="{ active: filter === f.id }"
                                @click="filter = f.id; inputEl?.focus()"
                            >{{ t(`globalSearch.filter.${f.id}`) }}</button>
                        </div>

                        <div ref="resultsEl" class="gsResults">
                            <template v-if="showSkeleton">
                                <div v-for="i in 4" :key="i" class="gsRow gsSkeleton">
                                    <span class="gsCover"></span>
                                    <span class="gsText"><span class="gsBar wide"></span><span class="gsBar"></span></span>
                                </div>
                            </template>

                            <div v-else-if="nothingFound" class="gsEmpty">{{ t('globalSearch.nothing') }}</div>

                            <template v-else>
                                <section v-if="shownTracks.length" class="gsGroup">
                                    <div v-if="filter === 'all'" class="gsSection">{{ t('globalSearch.filter.tracks') }}</div>
                                    <div
                                        v-for="tr in shownTracks"
                                        :key="tr.file_path"
                                        class="gsRow"
                                        :class="{ active: activeIndex === indexOfTrack(tr), current: isCurrent(tr) }"
                                        @click="playFromResults(tr)"
                                        @contextmenu.prevent="openTrackMenu($event, tr)"
                                    >
                                        <span class="gsCover">
                                            <img :src="tr.cover_art || defaultTrackCover" alt="" draggable="false" />
                                            <span class="gsCoverPlay">
                                                <PauseIcon v-if="isCurrent(tr) && isPlaying" width="16" height="16" />
                                                <PlayIcon v-else width="16" height="16" />
                                            </span>
                                        </span>
                                        <span class="gsText">
                                            <span class="gsTitle">{{ trackTitle(tr) }}</span>
                                            <span class="gsSub">{{ trackLine(tr) }}</span>
                                        </span>
                                        <span class="gsMeta">{{ duration(tr.duration_secs) }}</span>
                                    </div>
                                    <button v-if="moreTracks" type="button" class="gsMore" @click="showMoreTracks">
                                        {{ t('globalSearch.showMore') }}
                                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 18 6-6-6-6" /></svg>
                                    </button>
                                </section>

                                <section v-for="group in sections" :key="group.kind" class="gsGroup">
                                    <div v-if="filter === 'all'" class="gsSection">{{ group.label }}</div>
                                    <div
                                        v-for="c in group.list"
                                        :key="c.kind + c.id"
                                        class="gsRow"
                                        :class="{ active: activeIndex === indexOfCollection(c) }"
                                        @click="openCollection(c)"
                                    >
                                        <span class="gsCover" :class="{ round: c.kind === 'artist' }">
                                            <AnimatedCover :src="coverOf(c)" />
                                            <span v-if="c.kind !== 'artist' && c.count > 0" class="gsCoverPlay" @click="playCollection($event, c)">
                                                <PauseIcon v-if="isSourcePlaying(c)" width="16" height="16" />
                                                <PlayIcon v-else width="16" height="16" />
                                            </span>
                                        </span>
                                        <span class="gsText">
                                            <span class="gsTitle">{{ c.name }}</span>
                                            <span class="gsSub">{{ c.sub }}</span>
                                        </span>
                                        <span class="gsMeta">{{ tracksCountText(c.count) }}</span>
                                        <svg class="gsArrow" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 18 6-6-6-6" /></svg>
                                    </div>
                                </section>
                            </template>
                        </div>
                    </div>
                </div>
            </div>
        </Transition>

        <TrackContextMenu
            v-if="trackMenu"
            :key="trackMenu.track.file_path"
            :x="trackMenu.x"
            :y="trackMenu.y"
            :track="trackMenu.track"
            @close="trackMenu = null"
        />
    </Teleport>
</template>

<style scoped>
.gsPill {
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: min(340px, 34vw);
    height: 34px;
    padding: 0 10px 0 14px;
    display: flex;
    align-items: center;
    gap: 10px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.45);
    cursor: text;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.gsPill:hover {
    background-color: hsla(0, 0%, 100%, 0.09);
    color: hsla(0, 0%, 100%, 0.65);
}
.gsPillText {
    flex: 1;
    min-width: 0;
    text-align: left;
    font-size: 13px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}
.gsPillKey {
    flex-shrink: 0;
    padding: 2px 7px;
    border-radius: 6px;
    background-color: hsla(0, 0%, 100%, 0.06);
    font-size: 11px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.35);
    user-select: none;
}

.gsBackdrop {
    position: fixed;
    inset: 0;
    z-index: 1000000;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 64px 24px 40px;
    background-color: hsla(0, 0%, 0%, 0.7);
}
.gsPanel {
    width: min(620px, 100%);
    max-height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-radius: 22px;
    background-color: var(--light-bg);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.45);
    overflow: hidden;
}
.gsField {
    flex-shrink: 0;
    height: 56px;
    padding: 0 10px 0 20px;
    display: flex;
    align-items: center;
    gap: 12px;
}
.gsFieldIcon {
    flex-shrink: 0;
    color: hsla(0, 0%, 100%, 0.5);
}
.gsSpinner {
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    margin: 1px;
    border-radius: 50%;
    border: 2px solid hsla(0, 0%, 100%, 0.15);
    border-top-color: hsla(0, 0%, 100%, 0.7);
    animation: gsSpin .7s linear infinite;
}
@keyframes gsSpin {
    to { transform: rotate(360deg); }
}
.gsInputWrap {
    position: relative;
    flex: 1;
    min-width: 0;
    height: 100%;
}
.gsInput {
    width: 100%;
    height: 100%;
    background: transparent;
    border: 0;
    outline: 0;
    color: #fff;
    font-family: inherit;
    font-size: 16px;
    font-weight: 500;
}
.gsHint {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    overflow: hidden;
    pointer-events: none;
}
.gsHintText {
    display: block;
    color: hsla(0, 0%, 100%, 0.35);
    font-size: 16px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.gsHintSwap-enter-active,
.gsHintSwap-leave-active {
    transition: opacity var(--duration-hover) var(--ease-smooth), transform var(--duration-hover) var(--ease-smooth);
}
.gsHintSwap-enter-from {
    opacity: 0;
    transform: translateY(10px);
}
.gsHintSwap-leave-to {
    opacity: 0;
    transform: translateY(-10px);
}
.gsKey {
    flex-shrink: 0;
    margin-right: 6px;
    padding: 3px 8px;
    border-radius: 7px;
    background-color: hsla(0, 0%, 100%, 0.07);
    color: hsla(0, 0%, 100%, 0.45);
    font-size: 11px;
    font-weight: 600;
    user-select: none;
}
.gsClear {
    flex-shrink: 0;
    width: 34px;
    height: 34px;
    border-radius: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.45);
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.gsClear:hover {
    background-color: hsla(0, 0%, 100%, 0.07);
    color: #fff;
}

.gsBody {
    flex: 0 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
}
.gsChips {
    flex-shrink: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 0 16px 10px;
}
.gsChip {
    height: 30px;
    padding: 0 13px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    font-family: inherit;
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    user-select: none;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.gsChip:hover {
    color: #fff;
    background-color: hsla(0, 0%, 100%, 0.1);
}
.gsChip.active {
    background-color: hsla(0, 0%, 100%, 0.92);
    color: #111;
}

.gsResults {
    flex: 0 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 0 8px 10px;
    scrollbar-width: none;
}
.gsResults::-webkit-scrollbar {
    display: none;
}
.gsGroup + .gsGroup {
    margin-top: 10px;
}
.gsSection {
    padding: 6px 12px;
    color: hsla(0, 0%, 100%, 0.42);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    user-select: none;
}
.gsRow {
    height: 54px;
    padding: 0 12px 0 8px;
    display: flex;
    align-items: center;
    gap: 12px;
    border-radius: 12px;
    cursor: pointer;
    user-select: none;
    transition: background-color .12s var(--ease-smooth);
}
.gsRow:hover,
.gsRow.active {
    background-color: hsla(0, 0%, 100%, 0.06);
}
.gsCover {
    position: relative;
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    border-radius: 8px;
    overflow: hidden;
    background-color: hsla(0, 0%, 100%, 0.06);
}
.gsCover.round {
    border-radius: 50%;
}
.gsCover img,
.gsCover :deep(img) {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
}
.gsCoverPlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    background-color: hsla(0, 0%, 0%, 0.5);
    opacity: 0;
    transition: opacity .12s var(--ease-smooth);
}
.gsRow:hover .gsCoverPlay,
.gsRow.current .gsCoverPlay {
    opacity: 1;
}
.gsText {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
}
.gsTitle {
    min-width: 0;
    color: hsla(0, 0%, 100%, 0.92);
    font-size: 14px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.gsRow.current .gsTitle {
    color: hsl(var(--accent-h), min(var(--accent-s), 80%), 78%);
}
.gsSub {
    min-width: 0;
    color: hsla(0, 0%, 100%, 0.45);
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.gsMeta {
    flex-shrink: 0;
    color: hsla(0, 0%, 100%, 0.38);
    font-size: 12px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
}
.gsArrow {
    flex-shrink: 0;
    width: 0;
    color: hsla(0, 0%, 100%, 0.5);
    opacity: 0;
    transition: opacity .12s var(--ease-smooth), width .12s var(--ease-smooth);
}
.gsRow:hover .gsArrow,
.gsRow.active .gsArrow {
    width: 16px;
    opacity: 1;
}
.gsMore {
    margin: 2px 0 0 8px;
    height: 30px;
    padding: 0 8px 0 12px;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    border-radius: 999px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.55);
    font-family: inherit;
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color .12s var(--ease-smooth), color .12s var(--ease-smooth);
}
.gsMore:hover {
    background-color: hsla(0, 0%, 100%, 0.06);
    color: #fff;
}
.gsEmpty {
    padding: 4px 20px 18px;
    color: hsla(0, 0%, 100%, 0.45);
    font-size: 13px;
    font-weight: 500;
}
.gsSkeleton {
    cursor: default;
}
.gsSkeleton:hover {
    background-color: transparent;
}
.gsBar {
    display: block;
    width: 30%;
    height: 9px;
    border-radius: 5px;
    background-color: hsla(0, 0%, 100%, 0.06);
}
.gsBar.wide {
    width: 50%;
    background-color: hsla(0, 0%, 100%, 0.09);
}

.gsFade-enter-active,
.gsFade-leave-active {
    transition: opacity var(--duration-hover) var(--ease-smooth);
}
.gsFade-enter-active .gsPanel,
.gsFade-leave-active .gsPanel {
    transition: transform var(--duration-hover) var(--ease-smooth);
}
.gsFade-enter-from,
.gsFade-leave-to {
    opacity: 0;
}
.gsFade-enter-from .gsPanel,
.gsFade-leave-to .gsPanel {
    transform: translateY(-6px) scale(0.98);
}

@media (prefers-reduced-motion: reduce) {
    .gsFade-enter-active,
    .gsFade-leave-active,
    .gsFade-enter-active .gsPanel,
    .gsFade-leave-active .gsPanel,
    .gsHintSwap-enter-active,
    .gsHintSwap-leave-active {
        transition: none;
    }
}
</style>
