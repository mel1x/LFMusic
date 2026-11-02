<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import SearchIcon from '../components/Icons/SearchIcon.vue'
import DownloadIcon from '../components/Icons/DownloadIcon.vue'
import AlbumIcon from '../components/Icons/AlbumIcon.vue'
import CloseIcon from '../components/Icons/CloseIcon.vue'
import ConfirmModal from '../components/ConfirmModal.vue'
import CoverImage from '../components/CoverImage.vue'
import DownloadTrackRow from '../components/DownloadTrackRow.vue'
import DownloadAlbumCard from '../components/DownloadAlbumCard.vue'

import defaultCover from '../assets/cover.webp'
import defaultTrackCover from '../assets/trackCover.webp'

import {
    useDownloads,
    useKeepDownloadedInLibrary,
    useRedownloadConfirm,
    type ExternalSearchTrack,
    type ExternalSearchAlbum,
    type ExternalSearchArtist,
    type LinkResult,
} from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'
import { usePlayer } from '../composables/usePlayer'
import { useScrollLine } from '../composables/useScrollLine'
import { useTypeToSearch } from '../composables/useTypeToSearch'
import { switchLayout } from '../composables/useSearch'
import { albumsCountText } from '../utils/format'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const { formatTime } = usePlayer()
const {
    queue,
    tools,
    toolsLoaded,
    dlSettings,
    ytdlpMissing,
    denoMissing,
    search,
    resolveLink,
    enqueue,
    itemFor,
    installTools,
    checkUpdates,
    refreshTools,
    trackKey,
} = useDownloads()

// ── Search input ────────────────────────────────────────────────────────────

const searchInput = ref<HTMLInputElement | null>(null)
useTypeToSearch(searchInput)
const rawSearch = ref('')
const query = ref('')
// Exact artist/title when arriving from an album's missing track: Deezer
// field search finds generic titles ("Intro") that free text buries.
const exactMatch = ref<{ artist: string; title: string } | null>(null)
let debounceTimer: ReturnType<typeof setTimeout> | null = null

const locked = computed(() => ytdlpMissing.value)

function onInput(e: Event) {
    rawSearch.value = (e.target as HTMLInputElement).value
    exactMatch.value = null
    if (debounceTimer) clearTimeout(debounceTimer)
    debounceTimer = setTimeout(() => { query.value = rawSearch.value.trim() }, 320)
}
function submitNow() {
    if (debounceTimer) clearTimeout(debounceTimer)
    query.value = rawSearch.value.trim()
    remember()
}
function clearSearch() {
    if (debounceTimer) clearTimeout(debounceTimer)
    rawSearch.value = ''
    query.value = ''
    exactMatch.value = null
    searchInput.value?.focus()
}
function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') submitNow()
    else if (e.key === 'Escape' && rawSearch.value) { e.preventDefault(); clearSearch() }
}

const hasQuery = computed(() => query.value.length > 0)

// A URL in the box downloads exactly that page (any yt-dlp-supported site).
function isUrl(s: string): boolean {
    if (!/^https?:\/\/\S+$/i.test(s)) return false
    try { return !!new URL(s).hostname } catch { return false }
}
const linkMode = computed(() => isUrl(query.value))
const typingLink = computed(() => isUrl(rawSearch.value.trim()))

// ── Recent searches ─────────────────────────────────────────────────────────

const RECENT_KEY = 'lfmusic_past_downloads'
const recent = ref<string[]>([])
function loadRecent() {
    try { recent.value = JSON.parse(localStorage.getItem(RECENT_KEY) || '[]') } catch { recent.value = [] }
}
function saveRecent() {
    try { localStorage.setItem(RECENT_KEY, JSON.stringify(recent.value)) } catch { /* storage unavailable */ }
}
/** Called whenever a result is actually used — only useful queries are kept. */
function remember() {
    const q = rawSearch.value.trim()
    if (!q || isUrl(q)) return
    recent.value = [q, ...recent.value.filter(r => r.toLowerCase() !== q.toLowerCase())].slice(0, 12)
    saveRecent()
}
function forgetRecent(q: string) {
    recent.value = recent.value.filter(r => r !== q)
    saveRecent()
}
function clearRecent() {
    recent.value = []
    saveRecent()
}
function useRecent(q: string) {
    if (debounceTimer) clearTimeout(debounceTimer)
    rawSearch.value = q
    query.value = q
    searchInput.value?.focus()
}

// ── Catalog search ──────────────────────────────────────────────────────────

type Tab = 'all' | 'tracks' | 'albums' | 'artists'
const tab = ref<Tab>('all')
const PAGE = 30
const tracks = ref<ExternalSearchTrack[]>([])
const albums = ref<ExternalSearchAlbum[]>([])
const artists = ref<ExternalSearchArtist[]>([])
const searching = ref(false)
const loadingMore = ref(false)
const hasMore = ref(false)
const searchError = ref(false)
let nextIndex = 0
let effectiveQuery = ''
let searchSeq = 0

const anyResults = computed(() => tracks.value.length + albums.value.length + artists.value.length > 0)

function dedupe(base: ExternalSearchTrack[], extra: ExternalSearchTrack[]) {
    const seen = new Set(base.map(t => trackKey(t.artist, t.title, t.album)))
    return [...base, ...extra.filter(t => {
        const k = trackKey(t.artist, t.title, t.album)
        if (seen.has(k)) return false
        seen.add(k)
        return true
    })]
}
const isEmptyPage = (p: { tracks: unknown[]; albums: unknown[]; artists: unknown[] }) =>
    p.tracks.length === 0 && p.albums.length === 0 && p.artists.length === 0

async function runSearch(q: string) {
    const seq = ++searchSeq
    tracks.value = []
    albums.value = []
    artists.value = []
    hasMore.value = false
    // A page of the previous query may still be in flight; it won't reset this.
    loadingMore.value = false
    searchError.value = false
    nextIndex = 0
    tab.value = 'all'
    pageEl.value?.scrollTo({ top: 0 })
    if (!q) { searching.value = false; return }
    searching.value = true
    try {
        let page = await search(q, 0, PAGE, exactMatch.value ?? undefined)
        let used = q
        // Typed in the wrong keyboard layout? Retry with the swapped string.
        if (isEmptyPage(page)) {
            const swapped = switchLayout(q)
            if (swapped && swapped !== q) {
                const alt = await search(swapped, 0, PAGE)
                if (!isEmptyPage(alt)) { page = alt; used = swapped }
            }
        }
        if (seq !== searchSeq) return
        effectiveQuery = used
        tracks.value = dedupe([], page.tracks)
        albums.value = page.albums
        artists.value = page.artists
        nextIndex = page.next_index
        hasMore.value = page.has_more
    } catch (e) {
        console.error('Ошибка поиска:', e)
        if (seq === searchSeq) searchError.value = true
    } finally {
        if (seq === searchSeq) searching.value = false
    }
}

async function loadMore() {
    if (loadingMore.value || searching.value || !hasMore.value || !hasQuery.value || linkMode.value) return
    const seq = searchSeq
    loadingMore.value = true
    try {
        const page = await search(effectiveQuery, nextIndex, PAGE, exactMatch.value ?? undefined)
        if (seq !== searchSeq) return
        tracks.value = dedupe(tracks.value, page.tracks)
        nextIndex = page.next_index
        hasMore.value = page.has_more
    } catch (e) {
        console.error('Ошибка подгрузки:', e)
    } finally {
        if (seq === searchSeq) loadingMore.value = false
    }
}

const pageEl = ref<HTMLElement | null>(null)
// The search dock gets its own background only once it sticks, so it doesn't
// cut a band through the page's accent glow while at the top.
const stuck = ref(false)
function onScroll(e: Event) {
    const el = e.target as HTMLElement
    stuck.value = el.scrollTop > 64
    if (tab.value === 'tracks' && el.scrollHeight - el.scrollTop - el.clientHeight < 400) void loadMore()
}

// ── Links ───────────────────────────────────────────────────────────────────

const link = ref<LinkResult | null>(null)
const linkResolving = ref(false)
const linkError = ref<string | null>(null)
let linkSeq = 0

async function runLink(url: string) {
    const seq = ++linkSeq
    searchSeq++
    searching.value = false
    link.value = null
    linkError.value = null
    linkResolving.value = true
    try {
        const r = await resolveLink(url)
        if (seq === linkSeq) link.value = r
    } catch (e) {
        if (seq === linkSeq) linkError.value = e instanceof Error ? e.message : String(e)
    } finally {
        if (seq === linkSeq) linkResolving.value = false
    }
}

const linkPending = computed(() => (link.value?.tracks ?? []).filter(t => !t.in_library && !itemFor(t)))
async function downloadLinkAll() {
    if (linkPending.value.length) await enqueue(linkPending.value).catch(e => console.error(e))
}

watch(query, (q) => {
    if (isUrl(q)) {
        void runLink(q)
    } else {
        linkSeq++
        link.value = null
        linkError.value = null
        linkResolving.value = false
        void runSearch(q)
    }
})

// ── "All" tab composition ───────────────────────────────────────────────────

type Best =
    | { kind: 'artist'; artist: ExternalSearchArtist }
    | { kind: 'track'; track: ExternalSearchTrack }
const best = computed<Best | null>(() => {
    const a = artists.value[0]
    const tr = tracks.value[0]
    const q = trackKey(effectiveQuery || query.value, '')
    if (a && trackKey(a.name, '') === q) return { kind: 'artist', artist: a }
    if (tr) return { kind: 'track', track: tr }
    if (a) return { kind: 'artist', artist: a }
    return null
})
const topTracks = computed(() => tracks.value.slice(0, 5))
const counts = computed(() => ({
    tracks: tracks.value.length + (hasMore.value ? '+' : ''),
    albums: albums.value.length,
    artists: artists.value.length,
}))
const tabs = computed(() => [
    { key: 'all' as Tab, label: t('downloads.tabs.all'), count: null },
    { key: 'tracks' as Tab, label: t('downloads.tabs.tracks'), count: tracks.value.length ? counts.value.tracks : null },
    { key: 'albums' as Tab, label: t('downloads.tabs.albums'), count: counts.value.albums || null },
    { key: 'artists' as Tab, label: t('downloads.tabs.artists'), count: counts.value.artists || null },
])
function selectTab(k: Tab) {
    tab.value = k
    pageEl.value?.scrollTo({ top: 0, behavior: 'smooth' })
}

const albumsLine = useScrollLine(180, 18)
watch([albums, tab], async () => {
    await nextTick()
    albumsLine.observe()
})

function openArtist(a: ExternalSearchArtist) {
    remember()
    router.push(`/downloads/artist/${a.id}`)
}

const bestQueued = computed(() => (best.value?.kind === 'track' ? itemFor(best.value.track) : undefined))
async function downloadBest() {
    if (best.value?.kind !== 'track' || locked.value) return
    const tr = best.value.track
    if (tr.in_library) { confirmTrack.value = tr; return }
    remember()
    await enqueue(tr).catch(e => console.error(e))
}

// ── Redownload confirm ──────────────────────────────────────────────────────

const { confirmTrack, confirmMessage, confirmRedownload } = useRedownloadConfirm(remember)

useKeepDownloadedInLibrary(() => [tracks.value, link.value?.tracks ?? []])

// ── Idle page: recent downloads ─────────────────────────────────────────────

const recentDownloads = computed<ExternalSearchTrack[]>(() =>
    queue.value
        .filter(i => i.stage === 'done')
        .slice(-6)
        .reverse()
        .map(i => ({
            title: i.title,
            artist: i.artist,
            album: i.album,
            duration_secs: i.duration_secs,
            cover_url: i.cover_url || i.thumbnail_url,
            source: i.source || '',
            in_library: true,
            artist_id: null,
            album_id: i.album_id,
            genre: i.genre,
            source_url: i.source_url,
            thumbnail_url: i.thumbnail_url,
        })),
)

// ── Tools setup ─────────────────────────────────────────────────────────────

const setupNeeded = computed(() => toolsLoaded.value && !tools.value.info.unsupported && (ytdlpMissing.value || denoMissing.value))
const installing = computed(() => tools.value.busy)
const installConfirm = ref(false)
const installError = ref<string | null>(null)
const installProgress = computed(() => tools.value.progress)
const setupItems = computed(() => [
    { key: 'ytdlp', name: 'yt-dlp', on: tools.value.info.ytdlp.installed, optional: false },
    { key: 'deno', name: 'Deno', on: tools.value.info.deno.installed, optional: false },
    { key: 'ffmpeg', name: 'ffmpeg', on: tools.value.info.ffmpeg.installed, optional: true },
])
async function runInstall() {
    installConfirm.value = false
    installError.value = null
    try {
        await installTools()
        await nextTick()
        searchInput.value?.focus()
    } catch (e) {
        installError.value = String(e)
    }
}
// Auto-update covers this when enabled; otherwise offer it here.
const showUpdate = computed(() =>
    !setupNeeded.value && !dlSettings.value.auto_update && tools.value.info.ytdlp.managed && tools.value.info.ytdlp.update_available,
)
async function runUpdate() {
    try { await installTools(['ytdlp']) } catch (e) { installError.value = String(e) }
}

// ── Lifecycle ───────────────────────────────────────────────────────────────

onMounted(async () => {
    loadRecent()
    const pick = (v: unknown) => (Array.isArray(v) ? v[0] : v) as string | undefined
    const q = pick(route.query.q) || ''
    const a = pick(route.query.dlArtist) || ''
    const ti = pick(route.query.dlTitle) || ''
    if (q) {
        rawSearch.value = q
        if (a && ti) exactMatch.value = { artist: a, title: ti }
        query.value = q.trim()
    }
    try { await refreshTools() } catch { /* stays optimistic */ }
    if (!locked.value) {
        await nextTick()
        searchInput.value?.focus()
    }
    void checkUpdates()
})

onUnmounted(() => {
    if (debounceTimer) clearTimeout(debounceTimer)
    albumsLine.unobserve()
})
</script>

<template>
    <div class="dlPage" ref="pageEl" @scroll="onScroll">
        <!-- ── Hero: title + search ─────────────────────────────────────── -->
        <header class="hero">
            <h1 class="heroTitle">{{ t('downloads.title') }}</h1>
            <p class="heroSub">{{ t('downloads.subtitle') }}</p>
        </header>

        <div class="searchDock" :class="{ stuck }">
            <div class="searchBox" :class="{ link: typingLink, disabled: locked, filled: !!rawSearch }">
                <span class="searchLead" @click="searchInput?.focus()">
                    <svg v-if="typingLink" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>
                    <SearchIcon v-else />
                </span>
                <input
                    ref="searchInput"
                    class="searchInput"
                    type="text"
                    name="downloadSearch"
                    autocomplete="off"
                    spellcheck="false"
                    :placeholder="locked ? t('downloads.searchDisabledPlaceholder') : t('downloads.searchPlaceholder')"
                    :disabled="locked"
                    :value="rawSearch"
                    @input="onInput"
                    @keydown="onKeydown"
                />
                <Transition name="fade">
                    <span v-if="typingLink" class="modeTag">{{ t('downloads.linkTag') }}</span>
                </Transition>
                <span v-if="searching || linkResolving" class="searchSpinner"></span>
                <button v-if="rawSearch && !locked" class="searchClear" @click="clearSearch">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                </button>
            </div>

            <nav v-if="hasQuery && !linkMode && anyResults" class="tabs" role="tablist">
                <button
                    v-for="tb in tabs"
                    :key="tb.key"
                    class="tab"
                    :class="{ active: tab === tb.key }"
                    :disabled="tb.key !== 'all' && tb.count == null"
                    role="tab"
                    @click="selectTab(tb.key)"
                >
                    {{ tb.label }}<span v-if="tb.count != null" class="tabCount">{{ tb.count }}</span>
                </button>
            </nav>
        </div>

        <!-- ── First-run setup ──────────────────────────────────────────── -->
        <section v-if="setupNeeded" class="setupCard">
            <div class="setupIcon"><DownloadIcon width="26" height="26" /></div>
            <div class="setupBody">
                <h2 class="setupTitle">{{ ytdlpMissing ? t('downloads.setup.title') : t('downloads.setup.denoTitle') }}</h2>
                <p class="setupText">{{ ytdlpMissing ? t('downloads.setup.text') : t('downloads.setup.denoText') }}</p>
                <div class="setupChips">
                    <span v-for="c in setupItems" :key="c.key" class="setupChip" :class="{ on: c.on, busy: installProgress?.tool === c.key }">
                        <span class="chipDot"></span>{{ c.name }}<em v-if="c.optional && !c.on">{{ t('downloads.setup.optional') }}</em>
                    </span>
                </div>
                <div v-if="installing" class="setupProgress">
                    <div class="setupBar">
                        <span
                            class="setupBarFill"
                            :class="{ indeterminate: installProgress?.fraction == null }"
                            :style="installProgress?.fraction != null ? { width: Math.round(installProgress.fraction * 100) + '%' } : undefined"
                        ></span>
                    </div>
                    <span class="setupProgressText">{{ installProgress?.message || t('downloads.setup.installing') }}</span>
                </div>
                <p v-else-if="installError || tools.error" class="setupError">{{ installError || tools.error }}</p>
            </div>
            <button class="primaryBtn" :disabled="installing" @click="installConfirm = true">
                <span v-if="installing" class="btnSpinner"></span>
                {{ installing ? t('downloads.setup.installing') : t('downloads.setup.install') }}
            </button>
        </section>

        <section v-else-if="showUpdate" class="updateBar">
            <div class="updateText">
                <strong>{{ t('downloads.setup.updateTitle') }}</strong>
                <span>{{ t('downloads.setup.updateText') }}</span>
            </div>
            <button class="ghostBtn" :disabled="installing" @click="runUpdate">
                <span v-if="installing" class="btnSpinner light"></span>{{ t('downloads.setup.update') }}
            </button>
        </section>

        <!-- ── Idle ─────────────────────────────────────────────────────── -->
        <template v-if="!hasQuery">
            <section v-if="recent.length && !locked" class="block">
                <div class="blockHead">
                    <h3 class="blockTitle">{{ t('downloads.recentTitle') }}</h3>
                    <button class="textBtn" @click="clearRecent">{{ t('downloads.clearRecent') }}</button>
                </div>
                <div class="recentChips">
                    <span v-for="r in recent" :key="r" class="recentChip" @click="useRecent(r)">
                        <SearchIcon class="recentIcon" width="13" height="13" />
                        <span class="recentText">{{ r }}</span>
                        <button class="recentRemove" @click.stop="forgetRecent(r)"><CloseIcon width="10" height="10" /></button>
                    </span>
                </div>
            </section>

            <section v-if="recentDownloads.length" class="block">
                <div class="blockHead"><h3 class="blockTitle">{{ t('downloads.recentDownloads') }}</h3></div>
                <div class="trackList">
                    <DownloadTrackRow v-for="tr in recentDownloads" :key="trackKey(tr.artist, tr.title, tr.album)" :track="tr" @redownload="confirmTrack = $event" />
                </div>
            </section>

            <section class="tips">
                <div class="tip">
                    <span class="tipIcon"><SearchIcon width="18" height="18" /></span>
                    <strong>{{ t('downloads.tips.searchTitle') }}</strong>
                    <span>{{ t('downloads.tips.searchText') }}</span>
                </div>
                <div class="tip">
                    <span class="tipIcon"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg></span>
                    <strong>{{ t('downloads.tips.linkTitle') }}</strong>
                    <span>{{ t('downloads.tips.linkText') }}</span>
                </div>
                <div class="tip">
                    <span class="tipIcon"><AlbumIcon width="18" height="18" /></span>
                    <strong>{{ t('downloads.tips.albumTitle') }}</strong>
                    <span>{{ t('downloads.tips.albumText') }}</span>
                </div>
            </section>
        </template>

        <!-- ── Link ─────────────────────────────────────────────────────── -->
        <template v-else-if="linkMode">
            <div v-if="linkResolving" class="stateBox">
                <span class="bigSpinner"></span>
                <span class="stateText">{{ t('downloads.link.resolving') }}</span>
            </div>
            <div v-else-if="linkError" class="stateBox">
                <span class="stateIcon error">!</span>
                <span class="stateText">{{ t('downloads.link.errorTitle') }}</span>
                <span class="stateDesc">{{ linkError }}</span>
            </div>
            <section v-else-if="link" class="block">
                <div class="blockHead">
                    <div class="linkHead">
                        <span class="eyebrow">{{ link.tracks.length > 1 ? t('downloads.link.collection') : t('downloads.link.single') }}</span>
                        <h3 v-if="link.collection" class="blockTitle big">{{ link.collection }}</h3>
                    </div>
                    <button v-if="link.tracks.length > 1" class="primaryBtn small" :disabled="!linkPending.length || locked" @click="downloadLinkAll">
                        <DownloadIcon width="15" height="15" />{{ t('downloads.link.downloadAll', { count: linkPending.length }) }}
                    </button>
                </div>
                <div class="trackList">
                    <DownloadTrackRow v-for="tr in link.tracks" :key="tr.source_url!" :track="tr" @redownload="confirmTrack = $event" />
                </div>
            </section>
        </template>

        <!-- ── Catalog results ──────────────────────────────────────────── -->
        <template v-else>
            <div v-if="searching && !anyResults" class="skeleton">
                <div class="skelHero"><span class="skelCover"></span><span class="skelLines"><i></i><i></i><i></i></span></div>
                <div v-for="n in 5" :key="n" class="skelRow"><span class="skelThumb"></span><span class="skelLines"><i></i><i></i></span></div>
            </div>

            <div v-else-if="searchError" class="stateBox">
                <span class="stateIcon error">!</span>
                <span class="stateText">{{ t('downloads.searchError') }}</span>
            </div>

            <div v-else-if="!searching && !anyResults" class="stateBox">
                <span class="stateIcon"><SearchIcon width="18" height="18" /></span>
                <span class="stateText">{{ t('downloads.emptyNothingTitle') }}</span>
                <span class="stateDesc">{{ t('downloads.emptyNothingDesc') }}</span>
            </div>

            <!-- All -->
            <template v-else-if="tab === 'all'">
                <div class="topGrid">
                    <section v-if="best" class="bestWrap">
                        <h3 class="blockTitle">{{ t('downloads.topResult') }}</h3>
                        <div v-if="best.kind === 'artist'" class="bestCard" @click="openArtist(best.artist)">
                            <div class="bestArt round"><CoverImage :src="best.artist.picture_url" :fallback="defaultCover" /></div>
                            <div class="bestName" v-truncate>{{ best.artist.name }}</div>
                            <div class="bestMeta">
                                <span class="typePill">{{ t('downloads.typeArtist') }}</span>
                                <span v-if="best.artist.nb_album">{{ albumsCountText(best.artist.nb_album) }}</span>
                            </div>
                            <span class="bestGo"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></span>
                        </div>
                        <div v-else class="bestCard" @click="downloadBest">
                            <div class="bestArt"><CoverImage :src="best.track.cover_url" :fallback="defaultTrackCover" /></div>
                            <div class="bestName" v-truncate>{{ best.track.title }}</div>
                            <div class="bestMeta">
                                <span class="typePill">{{ t('downloads.typeTrack') }}</span>
                                <span v-truncate>{{ best.track.artist }}</span>
                                <span v-if="best.track.duration_secs" class="dim">{{ formatTime(best.track.duration_secs) }}</span>
                            </div>
                            <span class="bestGo download" :class="{ done: best.track.in_library || bestQueued?.stage === 'done', busy: bestQueued && bestQueued.stage !== 'done' && bestQueued.stage !== 'error' }">
                                <span v-if="bestQueued && bestQueued.stage !== 'done' && bestQueued.stage !== 'error'" class="btnSpinner"></span>
                                <svg v-else-if="best.track.in_library || bestQueued?.stage === 'done'" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                                <DownloadIcon v-else width="20" height="20" />
                            </span>
                        </div>
                    </section>

                    <section v-if="topTracks.length" class="topTracks">
                        <div class="blockHead">
                            <h3 class="blockTitle">{{ t('downloads.tracksTitle') }}</h3>
                            <button v-if="tracks.length > topTracks.length" class="textBtn" @click="selectTab('tracks')">{{ t('downloads.showAll') }}</button>
                        </div>
                        <div class="trackList">
                            <DownloadTrackRow v-for="tr in topTracks" :key="trackKey(tr.artist, tr.title, tr.album)" :track="tr" @download="remember" @redownload="confirmTrack = $event" />
                        </div>
                    </section>
                </div>

                <section v-if="artists.length" class="block">
                    <div class="blockHead">
                        <h3 class="blockTitle">{{ t('downloads.artistsTitle') }}</h3>
                        <button v-if="artists.length > 6" class="textBtn" @click="selectTab('artists')">{{ t('downloads.showAll') }}</button>
                    </div>
                    <div class="artistRow">
                        <div v-for="a in artists.slice(0, 6)" :key="a.id" class="artistCard" @click="openArtist(a)">
                            <div class="artistAvatar"><CoverImage :src="a.picture_url" :fallback="defaultCover" /></div>
                            <div class="artistName" v-truncate>{{ a.name }}</div>
                            <div class="artistSub">{{ t('downloads.typeArtist') }}</div>
                        </div>
                    </div>
                </section>

                <section v-if="albums.length" class="block lineBlock">
                    <div class="blockHead">
                        <h3 class="blockTitle">{{ t('downloads.albumsTitle') }}</h3>
                        <div class="lineControls">
                            <button :disabled="!albumsLine.canPrev.value" @click="albumsLine.scrollByPage(-1)"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg></button>
                            <button :disabled="!albumsLine.canNext.value" @click="albumsLine.scrollByPage(1)"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                        </div>
                    </div>
                    <div
                        class="albumLine"
                        :ref="(el: any) => { albumsLine.el.value = el as HTMLElement | null }"
                        :style="{ '--card-w': albumsLine.itemWidth.value + 'px', visibility: albumsLine.measured.value ? 'visible' : 'hidden' }"
                        @scroll="albumsLine.onScroll"
                    >
                        <DownloadAlbumCard v-for="al in albums" :key="al.id" :album="al" @open="remember" @download="remember" />
                    </div>
                </section>
            </template>

            <!-- Tracks -->
            <section v-else-if="tab === 'tracks'" class="block">
                <div class="trackList">
                    <DownloadTrackRow v-for="tr in tracks" :key="trackKey(tr.artist, tr.title, tr.album)" :track="tr" @download="remember" @redownload="confirmTrack = $event" />
                </div>
                <div v-if="loadingMore" class="loadMore"><span class="btnSpinner light"></span></div>
            </section>

            <!-- Albums -->
            <section v-else-if="tab === 'albums'" class="block">
                <div class="cardGrid">
                    <DownloadAlbumCard v-for="al in albums" :key="al.id" :album="al" @open="remember" @download="remember" />
                </div>
            </section>

            <!-- Artists -->
            <section v-else class="block">
                <div class="cardGrid artists">
                    <div v-for="a in artists" :key="a.id" class="artistCard" @click="openArtist(a)">
                        <div class="artistAvatar"><CoverImage :src="a.picture_url" :fallback="defaultCover" /></div>
                        <div class="artistName" v-truncate>{{ a.name }}</div>
                        <div class="artistSub">{{ a.nb_album ? albumsCountText(a.nb_album) : t('downloads.typeArtist') }}</div>
                    </div>
                </div>
            </section>
        </template>

        <ConfirmModal
            :open="confirmTrack !== null"
            :title="t('downloads.redownloadTitle')"
            :message="confirmMessage"
            :cancel-text="t('downloads.redownloadCancel')"
            :confirm-text="t('downloads.redownloadConfirm')"
            @cancel="confirmTrack = null"
            @confirm="confirmRedownload"
        />
        <ConfirmModal
            :open="installConfirm"
            :title="t('downloads.setup.confirmTitle')"
            :message="t('downloads.setup.confirmText')"
            :cancel-text="t('downloads.setup.cancel')"
            :confirm-text="t('downloads.setup.confirmOk')"
            @cancel="installConfirm = false"
            @confirm="runInstall"
        />
    </div>
</template>

<style scoped>
.dlPage {
    position: relative;
    width: 100%;
    height: 100%;
    overflow-y: auto;
    padding: 0 28px 40px;
    display: flex;
    flex-direction: column;
    background-color: var(--light-bg);
    background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
    background-repeat: no-repeat;
    background-attachment: local;
}
.dlPage::-webkit-scrollbar { width: 6px; }
.dlPage::-webkit-scrollbar-track { background: transparent; }
.dlPage::-webkit-scrollbar-thumb { background: hsla(0, 0%, 100%, 0.12); border-radius: 3px; }

/* ── Hero ─────────────────────────────────────────────────────────── */
.hero {
    padding: 20px 0 4px;
    display: flex;
    flex-direction: column;
    gap: 6px;
}
.heroTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 34px;
    font-weight: 900;
    line-height: 1.05;
    letter-spacing: -0.01em;
    color: hsla(0, 0%, 100%, 0.95);
    user-select: none;
}
.heroSub {
    max-width: 640px;
    font-size: 13.5px;
    font-weight: 500;
    line-height: 1.45;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 85%, 0.5);
    user-select: none;
}
.searchDock {
    position: sticky;
    top: 0;
    z-index: 5;
    margin: 0 -28px;
    padding: 14px 28px 12px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    transition: background-color .2s ease, box-shadow .2s ease;
}
.searchDock.stuck {
    background-color: var(--light-bg);
    box-shadow: 0 14px 24px -12px hsla(0, 0%, 0%, 0.45);
}
.searchBox {
    position: relative;
    display: flex;
    align-items: center;
    height: 46px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.04);
    transition: background-color .24s ease, box-shadow .24s ease;
}
.searchBox.link {
    box-shadow: inset 0 0 0 1.5px hsla(145, 50%, 60%, 0.45);
}
.searchBox.disabled { opacity: 0.5; }
.searchLead {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 42px;
    padding-left: 4px;
    height: 100%;
    flex-shrink: 0;
    cursor: text;
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.4);
    transition: color .24s ease;
}
.searchBox:hover .searchLead { color: hsla(var(--accent-h), var(--accent-s), 80%, 0.8); }
.searchBox.link .searchLead { color: hsla(145, 55%, 68%, 0.95); }
.searchInput {
    flex: 1;
    min-width: 0;
    height: 100%;
    background: transparent;
    font-size: 14px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.9);
}
.searchInput::placeholder { user-select: none; color: hsla(0, 0%, 100%, 0.3); font-size: 14px; font-weight: 600; }
.searchInput:disabled { cursor: not-allowed; }
.modeTag {
    flex-shrink: 0;
    margin-right: 8px;
    padding: 4px 10px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 800;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: hsla(145, 55%, 72%, 1);
    background: hsla(145, 45%, 50%, 0.14);
}
.searchSpinner {
    flex-shrink: 0;
    margin-right: 10px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid hsla(0, 0%, 100%, 0.12);
    border-top-color: hsla(var(--accent-h), var(--accent-s), 75%, 0.95);
    animation: spin .8s linear infinite;
}
.searchClear {
    flex-shrink: 0;
    margin-right: 5px;
    padding: 6px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.3);
    background: transparent;
    transition: .24s ease;
}
.searchClear:hover { color: hsla(0, 0%, 100%, 0.6); }

.tabs { display: flex; gap: 6px; flex-wrap: wrap; }
.tab {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 34px;
    padding: 0 15px;
    border-radius: 999px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.7);
    background-color: hsla(0, 0%, 100%, 0.06);
    transition: background-color .18s ease, color .18s ease;
    user-select: none;
}
.tab:hover:not(:disabled) { background-color: hsla(0, 0%, 100%, 0.11); color: #fff; }
.tab:disabled { opacity: 0.35; cursor: default; }
.tab.active {
    color: hsla(var(--accent-h), var(--accent-s), 16%, 1);
    background-color: hsla(0, 0%, 100%, 0.94);
}
.tabCount {
    font-size: 11px;
    font-weight: 800;
    font-variant-numeric: tabular-nums;
    opacity: 0.55;
}

/* ── Setup / update ───────────────────────────────────────────────── */
.setupCard {
    margin-top: 10px;
    display: flex;
    align-items: center;
    gap: 22px;
    padding: 24px;
    border-radius: 22px;
    background:
        radial-gradient(140% 120% at 0% 0%, hsla(var(--accent-h), var(--accent-s), 50%, 0.28), transparent 60%),
        hsla(var(--accent-h), min(var(--accent-s), 40%), 16%, 0.75);
}
.setupIcon {
    flex-shrink: 0;
    width: 60px;
    height: 60px;
    border-radius: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: hsla(var(--accent-h), var(--accent-s), 90%, 1);
    background: hsla(0, 0%, 100%, 0.08);
}
.setupBody { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
.setupTitle { font-size: 18px; font-weight: 800; color: hsla(0, 0%, 100%, 0.95); }
.setupText { font-size: 13px; line-height: 1.45; color: hsla(0, 0%, 100%, 0.55); max-width: 620px; }
.setupChips { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 2px; }
.setupChip {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 5px 11px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.6);
    background: hsla(0, 0%, 100%, 0.06);
}
.setupChip em { font-style: normal; font-weight: 500; opacity: 0.6; }
.chipDot { width: 7px; height: 7px; border-radius: 50%; background: hsla(0, 0%, 100%, 0.25); }
.setupChip.on { color: hsla(145, 50%, 75%, 0.95); }
.setupChip.on .chipDot { background: hsla(145, 60%, 55%, 1); }
.setupChip.busy .chipDot { background: hsla(var(--accent-h), var(--accent-s), 70%, 1); animation: pulse 1s ease-in-out infinite; }
@keyframes pulse { 50% { opacity: 0.3; } }
.setupProgress { display: flex; flex-direction: column; gap: 6px; margin-top: 4px; max-width: 520px; }
.setupBar { position: relative; height: 5px; border-radius: 3px; overflow: hidden; background: hsla(0, 0%, 100%, 0.1); }
.setupBarFill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 3px;
    background: hsla(var(--accent-h), min(calc(var(--accent-s) + 20%), 100%), 72%, 1);
    transition: width .25s ease;
}
.setupBarFill.indeterminate { width: 30%; animation: slide 1.2s ease-in-out infinite; }
@keyframes slide { 0% { left: -30%; } 100% { left: 100%; } }
.setupProgressText { font-size: 12px; font-weight: 600; color: hsla(0, 0%, 100%, 0.55); font-variant-numeric: tabular-nums; }
.setupError { font-size: 12.5px; font-weight: 600; color: hsla(2, 80%, 74%, 0.95); }

.updateBar {
    margin-top: 10px;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 14px 16px 14px 20px;
    border-radius: 16px;
    background: hsla(38, 70%, 50%, 0.1);
    box-shadow: inset 0 0 0 1px hsla(38, 80%, 60%, 0.16);
}
.updateText { flex: 1; display: flex; flex-direction: column; gap: 2px; font-size: 12.5px; color: hsla(0, 0%, 100%, 0.55); }
.updateText strong { font-size: 13.5px; color: hsla(38, 90%, 78%, 1); }

.primaryBtn {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 42px;
    padding: 0 22px;
    border-radius: 999px;
    cursor: pointer;
    font-size: 13.5px;
    font-weight: 800;
    color: hsla(var(--accent-h), var(--accent-s), 16%, 1);
    background: hsla(0, 0%, 100%, 0.95);
    transition: transform .2s ease, background-color .2s ease;
}
.primaryBtn:hover:not(:disabled) { background: #fff; transform: translateY(-1px); }
.primaryBtn:disabled { cursor: default; opacity: 0.6; }
.primaryBtn.small { height: 36px; padding: 0 16px; font-size: 12.5px; }
.ghostBtn {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    padding: 0 16px;
    border-radius: 999px;
    cursor: pointer;
    font-size: 12.5px;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.9);
    background: hsla(0, 0%, 100%, 0.1);
    transition: background-color .2s ease;
}
.ghostBtn:hover:not(:disabled) { background: hsla(0, 0%, 100%, 0.16); }
.btnSpinner {
    width: 15px;
    height: 15px;
    border-radius: 50%;
    border: 2px solid hsla(var(--accent-h), var(--accent-s), 20%, 0.25);
    border-top-color: hsla(var(--accent-h), var(--accent-s), 20%, 0.9);
    animation: spin .8s linear infinite;
}
.btnSpinner.light { border-color: hsla(0, 0%, 100%, 0.15); border-top-color: hsla(0, 0%, 100%, 0.85); }
@keyframes spin { to { transform: rotate(360deg); } }

/* ── Blocks ───────────────────────────────────────────────────────── */
.block { margin-top: 26px; display: flex; flex-direction: column; gap: 12px; }
.blockHead { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: 30px; }
.blockTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 20px;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.9);
    user-select: none;
}
.blockTitle.big { font-size: 24px; }
.textBtn {
    cursor: pointer;
    background: transparent;
    font-size: 12.5px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.5);
    transition: color .18s ease;
}
.textBtn:hover { color: #fff; }
.trackList { display: flex; flex-direction: column; gap: 3px; margin: 0 -8px; }
.eyebrow {
    font-size: 11px;
    font-weight: 800;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.6);
}
.linkHead { display: flex; flex-direction: column; gap: 4px; min-width: 0; }

/* Recent searches */
.recentChips { display: flex; flex-wrap: wrap; gap: 8px; }
.recentChip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    max-width: 280px;
    height: 36px;
    padding: 0 7px 0 13px;
    border-radius: 999px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.75);
    background-color: hsla(0, 0%, 100%, 0.06);
    transition: background-color .18s ease, color .18s ease;
    user-select: none;
}
.recentChip:hover { background-color: hsla(0, 0%, 100%, 0.11); color: #fff; }
.recentIcon { flex-shrink: 0; opacity: 0.5; }
.recentText { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.recentRemove {
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.3);
    background: transparent;
    transition: .18s ease;
}
.recentChip:hover .recentRemove { color: hsla(0, 0%, 100%, 0.6); }
.recentRemove:hover { color: #fff; background: hsla(0, 0%, 100%, 0.12); }

/* Tips */
.tips {
    margin-top: 30px;
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 14px;
}
.tip {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 18px;
    border-radius: 18px;
    background: hsla(0, 0%, 100%, 0.04);
    font-size: 12.5px;
    line-height: 1.45;
    color: hsla(0, 0%, 100%, 0.45);
    user-select: none;
}
.tip strong { font-size: 14px; font-weight: 700; color: hsla(0, 0%, 100%, 0.88); }
.tipIcon {
    width: 36px;
    height: 36px;
    margin-bottom: 6px;
    border-radius: 12px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: hsla(var(--accent-h), var(--accent-s), 85%, 0.95);
    background: hsla(var(--accent-h), var(--accent-s), 55%, 0.16);
}

/* ── Top result + top tracks ──────────────────────────────────────── */
.topGrid {
    margin-top: 18px;
    display: grid;
    grid-template-columns: minmax(280px, 0.85fr) minmax(0, 1.4fr);
    gap: 28px;
    align-items: start;
}
.bestWrap, .topTracks { display: flex; flex-direction: column; gap: 12px; min-width: 0; }
.bestCard {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 22px;
    min-height: 254px;
    box-sizing: border-box;
    border-radius: 22px;
    cursor: pointer;
    background:
        radial-gradient(120% 100% at 0% 0%, hsla(var(--accent-h), var(--accent-s), 55%, 0.2), transparent 65%),
        hsla(0, 0%, 100%, 0.045);
    transition: background-color .2s ease, transform .25s cubic-bezier(0.22, 1, 0.36, 1);
}
.bestCard:hover { background-color: hsla(0, 0%, 100%, 0.075); transform: translateY(-2px); }
.bestArt {
    width: 108px;
    height: 108px;
    margin-bottom: 14px;
    border-radius: 14px;
    overflow: hidden;
    background-color: #202020;
    box-shadow: 0 12px 30px hsla(0, 0%, 0%, 0.4);
}
.bestArt.round { border-radius: 50%; }
.bestName {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 28px;
    font-weight: 900;
    line-height: 1.15;
    color: hsla(0, 0%, 100%, 0.95);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    padding-right: 60px;
}
.bestMeta {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    padding-right: 60px;
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.6);
    white-space: nowrap;
}
.bestMeta .dim { color: hsla(0, 0%, 100%, 0.35); font-variant-numeric: tabular-nums; }
.typePill {
    flex-shrink: 0;
    padding: 3px 10px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.85);
    background: hsla(0, 0%, 100%, 0.1);
}
.bestGo {
    position: absolute;
    right: 20px;
    bottom: 20px;
    width: 50px;
    height: 50px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: hsla(var(--accent-h), var(--accent-s), 16%, 1);
    background: hsla(0, 0%, 100%, 0.95);
    box-shadow: 0 10px 26px hsla(0, 0%, 0%, 0.35);
    opacity: 0;
    transform: translateY(8px);
    transition: opacity .22s ease, transform .25s cubic-bezier(0.22, 1, 0.36, 1);
}
.bestCard:hover .bestGo,
.bestGo.done,
.bestGo.busy { opacity: 1; transform: translateY(0); }
.bestGo.done { color: hsla(145, 55%, 30%, 1); }

/* ── Artists ──────────────────────────────────────────────────────── */
.artistRow {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 16px;
}
.artistCard {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    padding: 12px 8px 14px;
    border-radius: 18px;
    cursor: pointer;
    min-width: 0;
    user-select: none;
    transition: background-color .2s ease;
}
.artistCard:hover { background-color: hsla(0, 0%, 100%, 0.045); }
.artistAvatar {
    width: 100%;
    max-width: 150px;
    aspect-ratio: 1;
    margin-bottom: 9px;
    border-radius: 50%;
    overflow: hidden;
    background-color: #202020;
    box-shadow: 0 10px 26px hsla(0, 0%, 0%, 0.3);
    transition: transform .25s cubic-bezier(0.22, 1, 0.36, 1);
}
.artistCard:hover .artistAvatar { transform: scale(1.03); }
.artistName {
    max-width: 100%;
    font-size: 14px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.artistSub { font-size: 12px; font-weight: 500; color: hsla(0, 0%, 100%, 0.4); }

/* ── Albums ───────────────────────────────────────────────────────── */
.albumLine {
    display: flex;
    gap: 18px;
    padding: 8px;
    margin: -8px;
    overflow-x: auto;
    scroll-behavior: smooth;
    scrollbar-width: none;
}
.albumLine::-webkit-scrollbar { display: none; }
.lineControls { display: flex; gap: 6px; }
.lineControls button {
    width: 30px;
    height: 30px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.7);
    background: hsla(0, 0%, 100%, 0.06);
    opacity: 0;
    transition: .2s ease;
}
.lineBlock:hover .lineControls button { opacity: 1; }
.lineControls button:hover:not(:disabled) { background: hsla(0, 0%, 100%, 0.12); color: #fff; }
.lineControls button:disabled { opacity: 0.3 !important; cursor: default; }
.cardGrid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 26px 20px;
    padding: 8px 0;
}
.cardGrid.artists { grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 10px; }

/* ── States ───────────────────────────────────────────────────────── */
.stateBox {
    flex: 1;
    min-height: 260px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    text-align: center;
    user-select: none;
}
.stateIcon {
    width: 44px;
    height: 44px;
    border-radius: 14px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 18px;
    font-weight: 900;
    color: #fff;
    background: hsla(0, 0%, 100%, 0.07);
}
.stateIcon.error { color: hsla(2, 85%, 75%, 1); background: hsla(2, 70%, 55%, 0.15); }
.stateText { font-size: 15px; font-weight: 700; color: hsla(0, 0%, 100%, 0.8); }
.stateDesc { max-width: 420px; font-size: 13px; line-height: 1.45; color: hsla(0, 0%, 100%, 0.4); }
.bigSpinner {
    width: 30px;
    height: 30px;
    border-radius: 50%;
    border: 3px solid hsla(0, 0%, 100%, 0.1);
    border-top-color: hsla(var(--accent-h), var(--accent-s), 75%, 0.95);
    animation: spin .8s linear infinite;
}
.loadMore { display: flex; justify-content: center; padding: 18px 0 4px; }

/* Skeleton while the first page loads */
.skeleton { margin-top: 18px; display: flex; flex-direction: column; gap: 10px; }
.skelHero { display: flex; gap: 18px; align-items: center; padding: 18px; border-radius: 22px; background: hsla(0, 0%, 100%, 0.035); margin-bottom: 10px; }
.skelCover { width: 96px; height: 96px; border-radius: 14px; }
.skelRow { display: flex; gap: 12px; align-items: center; height: 60px; padding: 0 8px; }
.skelThumb { width: 44px; height: 44px; border-radius: 8px; }
.skelLines { flex: 1; display: flex; flex-direction: column; gap: 8px; }
.skelLines i { display: block; height: 11px; border-radius: 6px; width: 46%; }
.skelLines i:nth-child(2) { width: 28%; }
.skelLines i:nth-child(3) { width: 18%; }
.skelCover, .skelThumb, .skelLines i {
    background: linear-gradient(90deg, hsla(0, 0%, 100%, 0.05) 0%, hsla(0, 0%, 100%, 0.1) 50%, hsla(0, 0%, 100%, 0.05) 100%);
    background-size: 200% 100%;
    animation: shimmer 1.3s ease-in-out infinite;
}
@keyframes shimmer { from { background-position: 100% 0; } to { background-position: -100% 0; } }

.fade-enter-active, .fade-leave-active { transition: opacity .18s ease; }
.fade-enter-from, .fade-leave-to { opacity: 0; }

/* ── Narrow windows / phone ───────────────────────────────────────── */
@media (max-width: 1100px) {
    .artistRow { grid-template-columns: repeat(4, minmax(0, 1fr)); }
    .artistRow .artistCard:nth-child(n + 5) { display: none; }
}
@media (max-width: 900px) {
    .topGrid { grid-template-columns: minmax(0, 1fr); }
    .tips { grid-template-columns: minmax(0, 1fr); }
}
@media (max-width: 768px), (pointer: coarse) and (max-width: 900px) {
    .dlPage { padding: 0 16px 80px; }
    .hero { padding-top: 16px; }
    .searchDock { margin: 0 -16px; padding: 12px 16px; }
    .searchBox { height: 48px; border-radius: 14px; }
    .heroTitle { font-size: 30px; }
    .heroSub { display: none; }
    .setupCard { flex-direction: column; align-items: flex-start; }
    .artistRow { grid-template-columns: repeat(3, minmax(0, 1fr)); }
    .artistRow .artistCard:nth-child(n + 4) { display: none; }
}
</style>
