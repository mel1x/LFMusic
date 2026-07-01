<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import SearchIcon from '../components/Icons/SearchIcon.vue'
import DownloadIcon from '../components/Icons/DownloadIcon.vue'
import LoadingSpinner from '../components/LoadingSpinner.vue'
import ConfirmModal from '../components/ConfirmModal.vue'
import CoverImage from '../components/CoverImage.vue'
import DownloadTrackRow from '../components/DownloadTrackRow.vue'
import DownloadAlbumCard from '../components/DownloadAlbumCard.vue'

import defaultCover from '../assets/cover.webp'

import {
    useDownloads,
    type ExternalSearchTrack,
    type ExternalSearchAlbum,
    type ExternalSearchArtist,
} from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'
import { useVirtualScroll } from '../composables/useVirtualScroll'
import { useScrollLine } from '../composables/useScrollLine'
import { switchLayout } from '../composables/useSearch'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const {
    queue,
    search,
    enqueue,
    refreshToolsInfo,
    checkToolUpdates,
    depsMissing,
    depsUpdateAvailable,
    ffmpegMissing,
    ytdlpMissing,
    denoMissing,
    cookiesAuthorized,
    trackKey,
} = useDownloads()

// Login is optional; only missing yt-dlp blocks search/download. Cookies are only
// needed for the bot-wall case (see botWallHit), which shows a soft hint, not a block.
const downloadsLocked = computed(() => ytdlpMissing.value)
const searchDisabledText = computed(() => t('downloads.searchDisabledPlaceholder'))

// Detected by error_kind === 'bot_wall_network' (not by message text): an IP/network
// wall, not an expired login. Logging in is one way to work around it.
const botWallHit = computed(() =>
    queue.value.some(i => i.status === 'error' && i.error_kind === 'bot_wall_network'),
)

// Search input (debounced, network)
import { useTypeToSearch } from '../composables/useTypeToSearch'
const searchInput = ref<HTMLInputElement | null>(null)
useTypeToSearch(searchInput)
const rawSearch = ref('')
const debouncedSearch = ref('')
// Exact artist/title from clicking an album "phantom" track: until the input is
// touched, search uses exact Deezer field match (otherwise a generic title like
// "Intro" gets lost in free-text search). Any manual typing resets this.
const exactMatch = ref<{ artist: string; title: string } | null>(null)
let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null

const focusSearch = () => { if (!downloadsLocked.value) searchInput.value?.focus() }

function onSearchInput(e: Event) {
    const value = (e.target as HTMLInputElement).value
    rawSearch.value = value
    exactMatch.value = null
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    searchDebounceTimer = setTimeout(() => {
        debouncedSearch.value = value.trim()
    }, 300)
}

function clearSearch() {
    rawSearch.value = ''
    debouncedSearch.value = ''
    exactMatch.value = null
    if (searchDebounceTimer) {
        clearTimeout(searchDebounceTimer)
        searchDebounceTimer = null
    }
    searchInput.value?.focus()
}

// Enter triggers search immediately, skipping the debounce, and saves the query.
function onSearchEnter() {
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    debouncedSearch.value = rawSearch.value.trim()
    rememberCurrentSearch()
}

// Past search suggestions (mirrors the regular search page)
const PAST_SEARCHES_KEY = 'lfmusic_past_downloads'
const MAX_PAST_SEARCHES = 15
const pastSearches = ref<string[]>([])

function loadPastSearches() {
    try {
        const raw = localStorage.getItem(PAST_SEARCHES_KEY)
        if (raw) pastSearches.value = JSON.parse(raw)
    } catch { /* ignore */ }
}
function savePastSearches() {
    localStorage.setItem(PAST_SEARCHES_KEY, JSON.stringify(pastSearches.value))
}
function addPastSearch(queryStr: string) {
    const trimmed = queryStr.trim()
    if (!trimmed) return
    pastSearches.value = pastSearches.value.filter(q => q !== trimmed)
    pastSearches.value.unshift(trimmed)
    if (pastSearches.value.length > MAX_PAST_SEARCHES) {
        pastSearches.value = pastSearches.value.slice(0, MAX_PAST_SEARCHES)
    }
    savePastSearches()
}
// Any meaningful action on results (download, opening an album/artist) means the
// query was useful, so save it.
function rememberCurrentSearch() {
    addPastSearch(rawSearch.value.trim())
}
const filteredPastSearches = computed(() => {
    const q = rawSearch.value.trim().toLowerCase()
    if (!q) return pastSearches.value.slice(0, 15)
    return pastSearches.value.filter(p => p.toLowerCase().includes(q)).slice(0, 15)
})
function onPastSearchClick(q: string) {
    rawSearch.value = q
    debouncedSearch.value = q
    if (searchDebounceTimer) { clearTimeout(searchDebounceTimer); searchDebounceTimer = null }
    searchInput.value?.focus()
}

const query = computed(() => debouncedSearch.value.trim())
const hasQuery = computed(() => !!query.value)
// Whether the response has anything at all (tracks/albums/artists), for the
// loading spinner and "nothing found" state.
const hasAnyResults = computed(
    () => results.value.length > 0 || resultAlbums.value.length > 0 || resultArtists.value.length > 0,
)

// Results (paged, loaded further on scroll)
const PAGE_SIZE = 30
const results = ref<ExternalSearchTrack[]>([])
// Albums and artists only arrive with the first page (index 0) and are shown
// as separate sections above the track list.
const resultAlbums = ref<ExternalSearchAlbum[]>([])
const resultArtists = ref<ExternalSearchArtist[]>([])
const searching = ref(false)     // initial load for a new query
const loadingMore = ref(false)   // loading the next page
const hasMore = ref(false)
let nextIndex = 0                 // offset (Deezer index) for the next page
// Query actually sent to Deezer: may differ from the typed one if layout-swap
// fixup kicked in (see runSearch). Paging uses this value.
let effectiveQuery = ''
let searchReqId = 0

// Virtualized results list: the scroll container is pageRoot itself, and the
// header above the list has variable height, so the list offset is measured
// dynamically and passed as scrollOffset (subtracted from scrollTop).
const resultsListEl = ref<HTMLElement | null>(null)
// Header container height is tracked with a ResizeObserver to recompute the
// virtual list offset on any DOM height change, instead of a fragile watch
// over many reactive fields.
const headerEl = ref<HTMLElement | null>(null)
const resultsOffset = ref(0)
const {
    containerEl: pageRootEl,
    onScroll: vsOnScroll,
    visibleItems: visibleResults,
    paddingTop: vsPadTop,
    paddingBottom: vsPadBottom,
    resetScroll: vsResetScroll,
} = useVirtualScroll(results, { itemHeight: 58, buffer: 6, scrollOffset: resultsOffset })

/** Recompute the results list offset from the top of the scroll container. */
function measureResultsOffset() {
    const list = resultsListEl.value
    if (!list || !pageRootEl.value) { resultsOffset.value = 0; return }
    // offsetTop relative to pageRoot (position: relative) = header height above the list.
    resultsOffset.value = list.offsetTop
}

/** Merge pages, dropping duplicates by "artist + title" both across pages and
 *  within the new batch (the seen set grows as it filters). */
function dedupeInto(base: ExternalSearchTrack[], extra: ExternalSearchTrack[]): ExternalSearchTrack[] {
    const seen = new Set(base.map(t => trackKey(t.artist, t.title, t.album)))
    const fresh = extra.filter(t => {
        const k = trackKey(t.artist, t.title, t.album)
        if (seen.has(k)) return false
        seen.add(k)
        return true
    })
    return [...base, ...fresh]
}

watch(query, (q) => { void runSearch(q) })

/** Whether the page has nothing at all (no tracks, albums, or artists). */
function isEmptyPage(p: { tracks: unknown[]; albums?: unknown[]; artists?: unknown[] }): boolean {
    return p.tracks.length === 0 && (p.albums?.length ?? 0) === 0 && (p.artists?.length ?? 0) === 0
}

async function runSearch(q: string) {
    const reqId = ++searchReqId
    results.value = []
    resultAlbums.value = []
    resultArtists.value = []
    nextIndex = 0
    hasMore.value = false
    loadingMore.value = false
    effectiveQuery = q
    // New query resets the virtual list to the top, otherwise it could open mid-scroll
    // or with an empty window if the previous results were scrolled down.
    vsResetScroll()
    if (!q) {
        searching.value = false
        return
    }
    searching.value = true
    try {
        let page = await search(q, 0, PAGE_SIZE, exactMatch.value ?? undefined)
        let usedQuery = q
        // Layout-swap fixup (mirrors regular search): empty results for the typed
        // query often mean the user typed in the wrong keyboard layout. Retry with
        // the swapped-layout string and use it if it finds something.
        if (isEmptyPage(page)) {
            const switched = switchLayout(q)
            if (switched && switched !== q) {
                const alt = await search(switched, 0, PAGE_SIZE, exactMatch.value ?? undefined)
                if (!isEmptyPage(alt)) { page = alt; usedQuery = switched }
            }
        }
        if (reqId !== searchReqId) return
        effectiveQuery = usedQuery
        results.value = dedupeInto([], page.tracks)
        resultAlbums.value = page.albums ?? []
        resultArtists.value = page.artists ?? []
        nextIndex = page.next_index
        hasMore.value = page.has_more
    } catch (e) {
        console.error('Ошибка поиска треков:', e)
        if (reqId === searchReqId) results.value = []
    } finally {
        if (reqId === searchReqId) searching.value = false
    }
}

async function loadMore() {
    if (loadingMore.value || searching.value || !hasMore.value || !query.value) return
    const reqId = searchReqId
    loadingMore.value = true
    try {
        const page = await search(effectiveQuery, nextIndex, PAGE_SIZE, exactMatch.value ?? undefined)
        if (reqId !== searchReqId) return
        results.value = dedupeInto(results.value, page.tracks)
        nextIndex = page.next_index
        hasMore.value = page.has_more
    } catch (e) {
        console.error('Ошибка подгрузки треков:', e)
    } finally {
        if (reqId === searchReqId) loadingMore.value = false
    }
}

function onScroll(e: Event) {
    vsOnScroll(e)
    const el = e.target as HTMLElement
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 360) void loadMore()
}

// Artists: wrapping/collapsible badges (mirrors regular search)
const artistsExpanded = ref(false)
const artistsContainer = ref<HTMLElement | null>(null)
const artistsOverflow = ref(false)
const artistsCollapsedMaxHeight = ref(0)
let artistsRO: ResizeObserver | null = null

function recomputeArtistsOverflow() {
    const el = artistsContainer.value
    if (!el || resultArtists.value.length === 0) { artistsOverflow.value = false; return }
    const item = el.querySelector('.artistBadge') as HTMLElement | null
    if (!item) { artistsOverflow.value = false; return }
    const cs = getComputedStyle(el)
    const rowGap = parseFloat(cs.rowGap || '0') || 0
    const rowH = item.offsetHeight
    const collapsedH = rowH * 2 + rowGap
    artistsCollapsedMaxHeight.value = collapsedH
    const wasCollapsed = el.style.maxHeight !== ''
    if (wasCollapsed) el.style.maxHeight = ''
    const fullH = el.scrollHeight
    if (wasCollapsed && !artistsExpanded.value) el.style.maxHeight = collapsedH + 'px'
    artistsOverflow.value = fullH > collapsedH + 1
}

watch(resultArtists, async () => {
    artistsExpanded.value = false
    await nextTick()
    recomputeArtistsOverflow()
})
watch(artistsContainer, (el) => {
    if (el) {
        if (!artistsRO) artistsRO = new ResizeObserver(() => recomputeArtistsOverflow())
        artistsRO.observe(el)
        nextTick(() => recomputeArtistsOverflow())
    }
})

function openArtist(a: ExternalSearchArtist) {
    rememberCurrentSearch()
    router.push(`/downloads/artist/${a.id}`)
}

// Albums: horizontal row with arrow navigation (mirrors regular search)
const albumsLine = useScrollLine(190, 16)
watch(resultAlbums, async () => {
    await nextTick()
    albumsLine.observe()
})

// Track row state (queued/in library) and the download button live in
// DownloadTrackRow; it delegates redownload confirmation here via the
// redownload event, using the shared confirm modal below.

// Redownload confirmation (already in library)
const confirmTrack = ref<ExternalSearchTrack | null>(null)
const confirmMessage = computed(() =>
    confirmTrack.value
        ? t('downloads.redownloadText', { title: confirmTrack.value.title })
        : '',
)

async function confirmRedownload() {
    const track = confirmTrack.value
    confirmTrack.value = null
    if (!track) return
    rememberCurrentSearch()
    try {
        await enqueue(track, true)
    } catch (e) {
        console.error('Ошибка повторного скачивания:', e)
    }
}

// Once a track finishes downloading, mark the matching result as "in library" so
// the button stays "Downloaded" after clearing the queue instead of reverting.
watch(queue, (items) => {
    const doneKeys = new Set(
        items.filter(i => i.status === 'done').map(i => trackKey(i.artist, i.title, i.album)),
    )
    if (doneKeys.size === 0) return
    for (const r of results.value) {
        if (!r.in_library && doneKeys.has(trackKey(r.artist, r.title, r.album))) r.in_library = true
    }
})

// The download queue itself moved to the app header (DownloadQueueIndicator),
// visible globally. `queue` here only marks results as queued/in-library.

// Tools status banner. Priority: install required yt-dlp -> install Deno (needed
// for stable YouTube downloads) -> soft-prompt login on bot-wall -> update
// available -> recommend optional ffmpeg. Button opens settings for all of this.
type DepsBannerMode = 'install' | 'deno' | 'botwall' | 'update' | 'recommend' | 'none'
const depsBannerMode = computed<DepsBannerMode>(() => {
    if (depsMissing.value) return 'install'
    if (denoMissing.value) return 'deno'
    if (botWallHit.value && !cookiesAuthorized.value) return 'botwall'
    if (depsUpdateAvailable.value) return 'update'
    if (ffmpegMissing.value) return 'recommend'
    return 'none'
})
const showDepsBanner = computed(() => depsBannerMode.value !== 'none')

function goToToolsSettings() {
    router.push('/settings/external')
}

// List only mounts once there are results (otherwise a spinner/empty state is
// shown), so its appearance triggers one offset recompute (the header's
// ResizeObserver won't fire since the header's own height hasn't changed).
watch(resultsListEl, (el) => { if (el) void nextTick(measureResultsOffset) })

// Any header height change (banner, queue, line wraps on window resize) triggers
// a precise offset recompute, more reliable than a manual watch over many fields.
let headerRO: ResizeObserver | null = null

onMounted(async () => {
    loadPastSearches()
    // Prefill search from query params (navigating from an album's phantom track).
    const pickParam = (v: typeof route.query[string]) => (Array.isArray(v) ? v[0] : v) || ''
    const q = pickParam(route.query.q)
    const dlArtist = pickParam(route.query.dlArtist)
    const dlTitle = pickParam(route.query.dlTitle)
    if (q) {
        rawSearch.value = q
        // Exact match fields must be set before debouncedSearch: watch(query) ->
        // runSearch reads them to enable exact Deezer field matching.
        if (dlArtist && dlTitle) exactMatch.value = { artist: dlArtist, title: dlTitle }
        debouncedSearch.value = q.trim()
    }
    // Local tools status (instant, no network) determines whether search is
    // available. Wait for it before focusing, otherwise focus could land on a
    // field that immediately becomes disabled (yt-dlp missing).
    try {
        await refreshToolsInfo()
    } catch { /* stays optimistic; focus as usual */ }
    if (!downloadsLocked.value) {
        await nextTick()
        searchInput.value?.focus()
    }
    // Update check (network) for the banner; doesn't block focus.
    void checkToolUpdates().catch(() => { /* no network, just no update info */ })
    await nextTick()
    measureResultsOffset()
    if (headerEl.value) {
        headerRO = new ResizeObserver(() => measureResultsOffset())
        headerRO.observe(headerEl.value)
    }
})

onUnmounted(() => {
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    headerRO?.disconnect()
    headerRO = null
    artistsRO?.disconnect()
    artistsRO = null
    albumsLine.unobserve()
})
</script>

<template>
    <div class="pageRoot downloadsPage" ref="pageRootEl" @scroll="onScroll">
        <!-- Header: everything above the results list; height tracked by ResizeObserver. -->
        <div class="downloadsHeader" ref="headerEl">
        <h1 class="pageTitle">{{ t('downloads.title') }}</h1>

        <div class="searchZone" :class="{ disabled: downloadsLocked }">
            <SearchIcon class="searchIcon" @click="focusSearch" />
            <input
                ref="searchInput"
                type="text"
                class="search"
                name="downloadSearch"
                id="downloadSearch"
                :placeholder="downloadsLocked ? searchDisabledText : t('downloads.searchPlaceholder')"
                autocomplete="off"
                :disabled="downloadsLocked"
                :value="rawSearch"
                @input="onSearchInput"
                @keydown.enter="onSearchEnter"
            />
            <div class="searchClear" v-if="rawSearch && !downloadsLocked" @click="clearSearch">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
            </div>
        </div>

        <!-- Past search suggestions (when input is empty) -->
        <div class="pastSearchesBar" v-if="!downloadsLocked && pastSearches.length > 0 && !rawSearch.trim()">
            <button
                v-for="(pq, idx) in filteredPastSearches"
                :key="idx"
                class="pastSearchChip"
                @click="onPastSearchClick(pq)"
            >
                {{ pq }}
            </button>
        </div>

        <!-- Dependencies banner -->
        <div v-if="showDepsBanner" class="toolsBanner" :class="{ soft: depsBannerMode === 'update' || depsBannerMode === 'recommend' }">
            <div class="toolsIcon">
                <DownloadIcon width="18" height="18" />
            </div>
            <div class="toolsInfo">
                <div class="toolsTitle">
                    <template v-if="depsBannerMode === 'install'">{{ t('downloads.depsInstallTitle') }}</template>
                    <template v-else-if="depsBannerMode === 'deno'">{{ t('downloads.depsDenoTitle') }}</template>
                    <template v-else-if="depsBannerMode === 'botwall'">{{ t('downloads.depsBotwallTitle') }}</template>
                    <template v-else-if="depsBannerMode === 'update'">{{ t('downloads.depsUpdateTitle') }}</template>
                    <template v-else>{{ t('downloads.depsRecommendTitle') }}</template>
                </div>
                <div class="toolsText">
                    <template v-if="depsBannerMode === 'install'">{{ t('downloads.depsInstallText') }}</template>
                    <template v-else-if="depsBannerMode === 'deno'">{{ t('downloads.depsDenoText') }}</template>
                    <template v-else-if="depsBannerMode === 'botwall'">{{ t('downloads.depsBotwallText') }}</template>
                    <template v-else-if="depsBannerMode === 'update'">{{ t('downloads.depsUpdateText') }}</template>
                    <template v-else>{{ t('downloads.depsRecommendText') }}</template>
                </div>
            </div>
            <button class="toolsBtn" @click="goToToolsSettings">{{ t('downloads.depsOpenSettings') }}</button>
        </div>

        <!-- Artists (first page of results only) -->
        <section v-if="hasQuery && resultArtists.length" class="pageLine">
            <div class="lineHat">
                <div class="lineTitle nonClickable">{{ t('downloads.artistsTitle') }}</div>
            </div>
            <div class="artistsBlockWrap">
                <div
                    class="artistsList"
                    ref="artistsContainer"
                    :class="{ collapsed: !artistsExpanded && artistsOverflow }"
                    :style="!artistsExpanded && artistsOverflow ? { maxHeight: artistsCollapsedMaxHeight + 'px' } : {}"
                >
                    <div
                        v-for="a in resultArtists"
                        :key="a.id"
                        class="artistBadge"
                        @click="openArtist(a)"
                        v-tooltip="t('downloads.openArtist')"
                    >
                        <div class="artistAvatar"><CoverImage :src="a.picture_url" :fallback="defaultCover" /></div>
                        <span v-truncate class="artistBadgeName">{{ a.name }}</span>
                    </div>
                </div>
                <div v-if="artistsOverflow" class="moreArtistsRow" :class="{ collapsed: !artistsExpanded }">
                    <button class="moreArtistsBtn" @click="artistsExpanded = !artistsExpanded">
                        <span class="moreArtistsLabel">{{ artistsExpanded ? t('downloads.collapse') : t('downloads.showAllArtists') }}</span>
                        <span v-if="!artistsExpanded" class="moreArtistsCount">{{ resultArtists.length }}</span>
                        <svg class="chev" :class="{ expanded: artistsExpanded }" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
                    </button>
                </div>
            </div>
        </section>

        <!-- Albums (first page of results only) -->
        <section v-if="hasQuery && resultAlbums.length" class="pageLine">
            <div class="lineHat">
                <div class="lineTitle nonClickable">{{ t('downloads.albumsTitle') }}</div>
                <div class="lineControls">
                    <button class="prevBtn" :disabled="!albumsLine.canPrev.value" @click="albumsLine.scrollByPage(-1)"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                    <button class="nextBtn" :disabled="!albumsLine.canNext.value" @click="albumsLine.scrollByPage(1)"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                </div>
            </div>
            <div
                class="lineList"
                :ref="(el: any) => { albumsLine.el.value = el as HTMLElement | null }"
                :style="{ '--card-w': albumsLine.itemWidth.value + 'px', visibility: albumsLine.measured.value ? 'visible' : 'hidden' }"
                @scroll="albumsLine.onScroll"
            >
                <DownloadAlbumCard
                    v-for="al in resultAlbums"
                    :key="al.id"
                    :album="al"
                    @open="rememberCurrentSearch"
                    @download="rememberCurrentSearch"
                />
            </div>
        </section>

        </div>
        <!-- /header -->

        <!-- Initial loading for a fresh query (no results of any kind yet) -->
        <LoadingSpinner v-if="hasQuery && searching && !hasAnyResults" />

        <!-- Tracks -->
        <section v-if="hasQuery && results.length" class="pageLine resultsLine">
            <div class="lineHat">
                <div class="lineTitle nonClickable">{{ t('downloads.tracksTitle') }}</div>
            </div>

            <div class="tracksList" ref="resultsListEl">
                <div class="vsPad" :style="{ height: vsPadTop + 'px' }"></div>
                <DownloadTrackRow
                    v-for="track in visibleResults"
                    :key="trackKey(track.artist, track.title, track.album)"
                    :track="track"
                    @download="rememberCurrentSearch"
                    @redownload="confirmTrack = $event"
                />
                <div class="vsPad" :style="{ height: vsPadBottom + 'px' }"></div>
            </div>

            <div v-if="loadingMore" class="loadMoreRow"><div class="miniSpinner"></div></div>
        </section>

        <!-- Nothing found (no tracks, albums or artists) -->
        <div v-if="hasQuery && !searching && !hasAnyResults" class="emptyState small">
            <div class="emptyIcon"><SearchIcon width="16" height="16" /></div>
            <div class="emptyText">{{ t('downloads.emptyNothingTitle') }}</div>
            <div class="emptyDesc">{{ t('downloads.emptyNothingDesc') }}</div>
        </div>

        <!-- Idle empty state -->
        <div v-if="!hasQuery" class="emptyState">
            <div class="emptyIcon"><DownloadIcon width="22" height="22" /></div>
            <div class="emptyText">{{ t('downloads.idleTitle') }}</div>
            <div class="emptyDesc">{{ t('downloads.idleDesc') }}</div>
        </div>

        <ConfirmModal
            :open="confirmTrack !== null"
            :title="t('downloads.redownloadTitle')"
            :message="confirmMessage"
            :cancel-text="t('downloads.redownloadCancel')"
            :confirm-text="t('downloads.redownloadConfirm')"
            @cancel="confirmTrack = null"
            @confirm="confirmRedownload"
        />
    </div>
</template>

<style scoped>
.pageRoot {
    position: relative;
    padding: 20px 28px 28px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: var(--light-bg);
    background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
    background-repeat: no-repeat;
    background-attachment: local;
    overflow-y: auto;
}
.pageRoot::-webkit-scrollbar { width: 6px; }
.pageRoot::-webkit-scrollbar-track { background-color: var(--light-bg); }
.pageRoot::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
}
/* Same flex-column as pageRoot so children's spacing (search/banner/queue) stays
   the same; doesn't shrink itself. */
.downloadsHeader {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    width: 100%;
}

/* Search input (mirrors Search.vue) */
.searchZone {
    position: relative;
    width: 100%;
    flex-shrink: 0;
    margin-bottom: 16px;
}
.searchIcon {
    cursor: text;
    position: absolute;
    top: 50%;
    left: 14px;
    transform: translateY(-50%);
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.4);
    transition: .24s ease;
}
.searchZone:hover .searchIcon {
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.8);
}
.searchClear {
    position: absolute;
    height: max-content;
    width: max-content;
    top: 50%;
    right: 11px;
    transform: translateY(-50%);
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.3);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 6px;
    border-radius: 50%;
    transition: .24s ease;
}
.searchClear:hover {
    color: hsla(0, 0%, 100%, 0.6);
}
.search {
    width: 100%;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.04);
    padding: 13px 38px 14px 42px;
    font-size: 14px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.9);
}
.search::placeholder {
    user-select: none;
    color: hsla(0, 0%, 100%, 0.3);
    font-size: 14px;
    font-weight: 600;
}
/* Search unavailable without yt-dlp: dim the zone and block input. */
.searchZone.disabled {
    opacity: 0.55;
}
.searchZone.disabled .searchIcon {
    cursor: default;
}
.search:disabled {
    cursor: not-allowed;
}

/* Past search suggestions (mirrors regular search) */
.pastSearchesBar {
    display: flex;
    flex-direction: row;
    gap: 6px;
    overflow-x: auto;
    flex-shrink: 0;
    padding-bottom: 2px;
    scrollbar-width: none;
    scroll-behavior: smooth;
    margin-bottom: 24px;
}
.pastSearchesBar::-webkit-scrollbar { display: none; }
.pastSearchChip {
    display: block;
    cursor: pointer;
    user-select: none;
    white-space: nowrap;
    line-height: 1;
    padding: 6px 14px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.6);
    font-size: 12px;
    font-weight: 500;
    transition: .24s ease;
    flex-shrink: 0;
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
}
.pastSearchChip:hover {
    background-color: hsla(0, 0%, 100%, 0.1);
    color: hsla(0, 0%, 100%, 0.8);
}

/* Tools banner */
.toolsBanner {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 14px;
    padding: 14px 16px;
    margin-bottom: 24px;
    border-radius: 999px;
    background: linear-gradient(172deg, hsla(var(--accent-h), var(--accent-s), 22%, 0.6), hsla(var(--accent-h), var(--accent-s), 14%, 0.6));
    flex-shrink: 0;
}
/* Only ffmpeg missing: non-critical, styled neutrally. */
.toolsBanner.soft {
    background: hsla(0, 0%, 100%, 0.04);
}
.toolsIcon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 38px;
    height: 38px;
    border-radius: 24px;
    background-color: hsla(0, 0%, 100%, 0.07);
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.95);
    flex-shrink: 0;
}
.miniSpinner {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid hsla(0, 0%, 100%, 0.15);
    border-top-color: hsla(var(--accent-h), var(--accent-s), 70%, 0.95);
    animation: spin 0.8s linear infinite;
}
@keyframes spin {
    to { transform: rotate(360deg); }
}
.toolsInfo {
    flex: 1;
    min-width: 0;
}
.toolsTitle {
    font-size: 13px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.92);
}
.toolsText {
    margin-top: 3px;
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.5);
    line-height: 1.35;
}
.toolsBtn {
    cursor: pointer;
    flex-shrink: 0;
    padding: 9px 18px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.95);
    color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
    font-size: 13px;
    font-weight: 700;
    transition: .2s ease;
}
.toolsBtn:hover {
    background-color: #fff;
    transform: translateY(-1px);
}

/* Sections */
.pageLine {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 13px;
    margin-bottom: 32px;
}
.resultsLine {
    margin-bottom: 0;
}
.lineHat {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
}
.lineTitle {
    user-select: none;
    color: hsla(0, 0%, 100%, 0.85);
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 22px;
}
.lineTitle.nonClickable { cursor: default; }

/* Arrow navigation for rows (mirrors regular search) */
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
}
.lineControls button:hover {
    border: 2px solid hsla(0, 0%, 100%, 0.4);
    color: hsla(0, 0%, 100%, 0.95);
}
.lineControls button:disabled:hover {
    border: 2px solid hsla(0, 0%, 100%, 0.2);
    color: hsla(0, 0%, 100%, 0.6);
}
.pageLine:hover .lineControls button {
    opacity: 1;
}
.pageLine:hover .lineControls button:disabled {
    opacity: 0.65;
}

/* Artists: badges (mirrors regular search) */
.artistsBlockWrap {
    position: relative;
}
.artistsList {
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
    gap: 8px;
    overflow: hidden;
    transition: max-height .28s ease;
}
.artistBadge {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 10px;
    padding: 4px 14px 4px 4px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.9);
    font-size: 13px;
    font-weight: 500;
    height: 38px;
    box-sizing: border-box;
    cursor: pointer;
    user-select: none;
    max-width: 100%;
    transition: background-color .2s ease, color .2s ease;
}
.artistBadge:hover {
    background-color: hsla(0, 0%, 100%, 0.09);
    color: hsla(0, 0%, 100%, 1);
}
.artistAvatar {
    width: 30px;
    height: 30px;
    border-radius: 9999px;
    background-color: #202020;
    flex-shrink: 0;
    overflow: hidden;
}
.artistBadgeName {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 220px;
}
.moreArtistsRow {
    display: flex;
    justify-content: center;
    margin-top: 14px;
}
.moreArtistsRow.collapsed {
    position: absolute;
    left: 0;
    right: 0;
    bottom: -6px;
    margin-top: 0;
    padding-top: 28px;
    pointer-events: none;
    background: linear-gradient(
        to bottom,
        hsla(var(--accent-h), min(var(--accent-s), 45%), 15%, 0) 0%,
        hsla(var(--accent-h), min(var(--accent-s), 45%), 15%, 0.85) 55%,
        hsla(var(--accent-h), min(var(--accent-s), 45%), 15%, 1) 100%
    );
}
.moreArtistsRow.collapsed .moreArtistsBtn {
    pointer-events: auto;
}
.moreArtistsBtn {
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 6px 0 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.85);
    font-size: 12px;
    font-weight: 600;
    transition: background-color .2s ease, color .2s ease, transform .2s ease;
}
.moreArtistsBtn:hover {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.18);
    color: #fff;
    transform: translateY(-1px);
}
.moreArtistsLabel { line-height: 1; }
.moreArtistsCount {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    height: 22px;
    padding: 0 7px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.08);
    color: hsla(0, 0%, 100%, 0.7);
    font-size: 11px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
}
.moreArtistsBtn:hover .moreArtistsCount {
    background-color: hsla(0, 0%, 100%, 0.18);
    color: #fff;
}
.moreArtistsBtn .chev {
    margin-right: 6px;
    color: hsla(0, 0%, 100%, 0.55);
    transition: transform .24s ease, color .2s ease;
}
.moreArtistsBtn:hover .chev { color: #fff; }
.moreArtistsBtn .chev.expanded { transform: rotate(180deg); }

/* Albums: horizontal row (mirrors regular search) */
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
.lineList::-webkit-scrollbar { display: none; }

/* Results track list */
.tracksList {
    display: flex;
    flex-direction: column;
}
/* Virtual scroll spacers (reserve height outside the visible window). */
.vsPad {
    width: 100%;
    flex-shrink: 0;
}

.loadMoreRow {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 18px 0 6px;
}

/* Empty states */
.emptyState {
    flex: 1;
    display: flex;
    gap: 10px;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding-bottom: 40px;
}
.emptyState.small {
    flex: none;
    padding: 40px 0;
}
.emptyIcon {
    padding: 10px;
    border-radius: 9px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: #ffffff;
}
.emptyText {
    user-select: none;
    font-size: 14px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.7);
}
.emptyDesc {
    user-select: none;
    text-align: center;
    max-width: 340px;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.3);
    line-height: 1.4;
}

/* ── Title (desktop hides; phone shows, mirrors Search.vue) ───────── */
.pageTitle { display: none; }

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .pageTitle {
        display: block !important;
        margin-top: 12px;
        margin-bottom: 22px !important;
        font-size: 30px !important;
        line-height: 1.08 !important;
        font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
        font-weight: 800;
        color: hsla(0, 0%, 100%, 0.9);
    }
    .downloadsPage.pageRoot {
        padding: 0 16px 80px !important;
    }
    .searchZone {
        height: 48px !important;
        border-radius: 14px !important;
        margin-bottom: 16px !important;
    }
    .search {
        height: 100% !important;
        padding: 0 44px 0 42px !important;
        border-radius: 14px !important;
    }
}
</style>
