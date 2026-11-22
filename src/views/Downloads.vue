<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'

import SearchIcon from '../components/Icons/SearchIcon.vue'
import DownloadIcon from '../components/Icons/DownloadIcon.vue'
import CloseIcon from '../components/Icons/CloseIcon.vue'
import FolderIcon from '../components/Icons/FolderIcon.vue'
import SettingsIcon from '../components/Icons/SettingsIcon.vue'
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
import { albumsCountText, tracksCountText } from '../utils/format'
import { useCoverTint } from '../composables/useCoverTint'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const { formatTime } = usePlayer()
const {
    queue,
    activeCount,
    downloadsDir,
    tools,
    toolsLoaded,
    dlSettings,
    ytdlpMissing,
    denoMissing,
    search,
    resolveLink,
    enqueue,
    retry,
    cancelAll,
    clearFinished,
    itemFor,
    installTools,
    checkUpdates,
    refreshTools,
    trackKey,
} = useDownloads()

const searchInput = ref<HTMLInputElement | null>(null)
useTypeToSearch(searchInput)
const rawSearch = ref('')
const query = ref('')
const exactMatch = ref<{ artist: string; title: string } | null>(null)
let debounceTimer: ReturnType<typeof setTimeout> | null = null

const locked = computed(() => ytdlpMissing.value)

const downloadsDirName = computed(() => {
    const dir = (downloadsDir.value || '').replace(/[\\/]+$/, '')
    if (!dir) return ''
    return dir.split(/[\\/]/).pop() || dir
})

async function openDownloadsFolder() {
    if (!downloadsDir.value) return
    try {
        await invoke('open_folder_in_explorer', { path: downloadsDir.value })
    } catch (e) {
        console.error('Не удалось открыть папку загрузок:', e)
    }
}

function openDownloadSettings() {
    void router.push('/settings/external')
}

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

function isUrl(s: string): boolean {
    if (!/^https?:\/\/\S+$/i.test(s)) return false
    try { return !!new URL(s).hostname } catch { return false }
}
const linkMode = computed(() => isUrl(query.value))
const typingLink = computed(() => isUrl(rawSearch.value.trim()))

const RECENT_KEY = 'lfmusic_past_downloads'
const recent = ref<string[]>([])
function loadRecent() {
    try { recent.value = JSON.parse(localStorage.getItem(RECENT_KEY) || '[]') } catch { recent.value = [] }
}
function saveRecent() {
    try { localStorage.setItem(RECENT_KEY, JSON.stringify(recent.value)) } catch {}
}
/** Called whenever a result is actually used - only useful queries are kept. */
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
function onScroll(e: Event) {
    const el = e.target as HTMLElement
    if (tab.value === 'tracks' && el.scrollHeight - el.scrollTop - el.clientHeight < 400) void loadMore()
}

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
const linkCover = computed(() => {
    const first = link.value?.tracks[0]
    return first?.cover_url || first?.thumbnail_url || null
})
const linkTint = useCoverTint(() => linkCover.value)
const linkStyle = computed(() => linkTint.value ? { '--link-color': `rgb(${linkTint.value})` } : {})

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
const topTracks = computed(() => tracks.value.slice(0, 4))
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

const albumsLine = useScrollLine(168, 16)
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

const bestCover = computed(() => best.value?.kind === 'artist' ? best.value.artist.picture_url : best.value?.track.cover_url)
const bestTint = useCoverTint(() => bestCover.value)
const bestStyle = computed(() => bestTint.value ? { '--best-color': `rgb(${bestTint.value})` } : {})

const { confirmTrack, confirmMessage, confirmRedownload } = useRedownloadConfirm(remember)

useKeepDownloadedInLibrary(() => [tracks.value, link.value?.tracks ?? []])

type QueueItem = (typeof queue.value)[number]
function itemToTrack(i: QueueItem): ExternalSearchTrack {
    return {
        title: i.title,
        artist: i.artist,
        album: i.album,
        duration_secs: i.duration_secs,
        cover_url: i.cover_url || i.thumbnail_url,
        source: i.source || '',
        in_library: i.stage === 'done',
        artist_id: null,
        album_id: i.album_id,
        genre: i.genre,
        source_url: i.source_url,
        thumbnail_url: i.thumbnail_url,
    }
}

// Queued, running and failed items; finished ones move to "recently downloaded".
const activeDownloads = computed<ExternalSearchTrack[]>(() =>
    queue.value.filter(i => i.stage !== 'done').map(itemToTrack),
)
const failedCount = computed(() => queue.value.filter(i => i.stage === 'error').length)

const recentDownloads = computed<ExternalSearchTrack[]>(() =>
    queue.value
        .filter(i => i.stage === 'done')
        .slice(-8)
        .reverse()
        .map(itemToTrack),
)

const isIdleEmpty = computed(() =>
    !setupNeeded.value && !activeDownloads.value.length && !recentDownloads.value.length,
)

async function onRetryAll() {
    try { await retry() } catch (e) { console.error('Ошибка повтора загрузок:', e) }
}
async function onCancelAll() {
    try { await cancelAll() } catch (e) { console.error('Ошибка отмены загрузок:', e) }
}
async function onClearFinished() {
    try { await clearFinished() } catch (e) { console.error('Ошибка очистки истории загрузок:', e) }
}

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
const showUpdate = computed(() =>
    !setupNeeded.value && !dlSettings.value.auto_update && tools.value.info.ytdlp.managed && tools.value.info.ytdlp.update_available,
)
async function runUpdate() {
    try { await installTools(['ytdlp']) } catch (e) { installError.value = String(e) }
}

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
    try { await refreshTools() } catch {}
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
        <header class="dlHeader">
            <h1 class="dlTitle">{{ t('downloads.title') }}</h1>
            <div class="dlHeaderActions">
                <button
                    v-if="downloadsDirName"
                    type="button"
                    class="folderPill"
                    v-tooltip="downloadsDir"
                    @click="openDownloadsFolder"
                >
                    <FolderIcon width="15" height="15" />
                    <span v-truncate class="folderPillName">{{ downloadsDirName }}</span>
                </button>
                <button
                    type="button"
                    class="headerIconBtn"
                    v-tooltip="t('downloads.settings')"
                    :aria-label="t('downloads.settings')"
                    @click="openDownloadSettings"
                >
                    <SettingsIcon width="18" height="18" />
                </button>
            </div>
        </header>

        <div class="searchDock">
            <div class="searchBox" :class="{ link: typingLink, disabled: locked, filled: !!rawSearch }">
                <span class="searchLead" @click="searchInput?.focus()">
                    <svg v-if="typingLink" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>
                    <SearchIcon v-else width="18" height="18" />
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
                <button v-if="rawSearch && !locked" type="button" class="searchClear" @click="clearSearch">
                    <CloseIcon width="15" height="15" />
                </button>
            </div>

            <nav v-if="hasQuery && !linkMode && anyResults" class="tabs" role="tablist">
                <button
                    v-for="tb in tabs"
                    :key="tb.key"
                    type="button"
                    class="tab"
                    :class="{ active: tab === tb.key }"
                    :disabled="tb.key !== 'all' && tb.count == null"
                    role="tab"
                    @click="selectTab(tb.key)"
                >
                    <span>{{ tb.label }}</span>
                    <span v-if="tb.count != null" class="tabCount">{{ tb.count }}</span>
                </button>
            </nav>

            <div v-else-if="!hasQuery && recent.length && !locked" class="recentBar">
                <span v-for="r in recent" :key="r" class="recentChip" @click="useRecent(r)">
                    <span class="recentText">{{ r }}</span>
                    <button type="button" class="recentRemove" @click.stop="forgetRecent(r)">
                        <CloseIcon width="10" height="10" />
                    </button>
                </span>
                <button type="button" class="recentClearBtn" @click="clearRecent">{{ t('downloads.clearRecent') }}</button>
            </div>
        </div>

        <section v-if="setupNeeded" class="setupCard">
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
            <button type="button" class="primaryBtn" :disabled="installing" @click="installConfirm = true">
                <span v-if="installing" class="btnSpinner"></span>
                {{ installing ? t('downloads.setup.installing') : t('downloads.setup.install') }}
            </button>
        </section>

        <section v-else-if="showUpdate" class="updateBar">
            <div class="updateText">
                <strong>{{ t('downloads.setup.updateTitle') }}</strong>
                <span>{{ t('downloads.setup.updateText') }}</span>
            </div>
            <button type="button" class="ghostBtn" :disabled="installing" @click="runUpdate">
                <span v-if="installing" class="btnSpinner light"></span>{{ t('downloads.setup.update') }}
            </button>
        </section>

        <template v-if="!hasQuery">
            <section v-if="activeDownloads.length" class="block">
                <div class="blockHead">
                    <h3 class="blockTitle">
                        <span>{{ t('downloads.activeTitle') }}</span>
                        <span class="blockCount">{{ activeDownloads.length }}</span>
                    </h3>
                    <div class="blockActions">
                        <button v-if="failedCount > 0" type="button" class="actionPill" @click="onRetryAll">
                            {{ t('downloads.queue.retryAll') }}
                        </button>
                        <button v-if="activeCount > 1" type="button" class="actionPill danger" @click="onCancelAll">
                            {{ t('downloads.queue.cancelAll') }}
                        </button>
                    </div>
                </div>
                <div class="trackList activeList">
                    <DownloadTrackRow v-for="tr in activeDownloads" :key="trackKey(tr.artist, tr.title, tr.album)" :track="tr" @redownload="confirmTrack = $event" />
                </div>
            </section>

            <section v-if="recentDownloads.length" class="block">
                <div class="blockHead">
                    <h3 class="blockTitle">
                        <span>{{ t('downloads.recentDownloads') }}</span>
                        <span class="blockCount">{{ recentDownloads.length }}</span>
                    </h3>
                    <button type="button" class="textBtn" @click="onClearFinished">{{ t('downloads.queue.clear') }}</button>
                </div>
                <div class="trackList">
                    <DownloadTrackRow v-for="tr in recentDownloads" :key="trackKey(tr.artist, tr.title, tr.album)" :track="tr" @redownload="confirmTrack = $event" />
                </div>
            </section>

            <div v-if="isIdleEmpty" class="stateBox">
                <div class="stateIcon"><DownloadIcon width="20" height="20" /></div>
                <span class="stateText">{{ t('downloads.idleHintTitle') }}</span>
                <span class="stateDesc">{{ t('downloads.idleHintDesc') }}</span>
            </div>
        </template>

        <template v-else-if="linkMode">
            <div v-if="linkResolving" class="stateBox">
                <span class="bigSpinner"></span>
                <span class="stateText">{{ t('downloads.link.resolving') }}</span>
            </div>
            <div v-else-if="linkError" class="stateBox">
                <span class="stateText error">{{ t('downloads.link.errorTitle') }}</span>
                <span class="stateDesc">{{ linkError }}</span>
            </div>
            <section v-else-if="link" class="block">
                <div class="linkCard" :style="linkStyle">
                    <div class="linkCover">
                        <CoverImage :src="linkCover" :fallback="defaultTrackCover" />
                    </div>
                    <div class="linkInfo">
                        <span class="linkType">{{ link.tracks.length > 1 ? t('downloads.link.collection') : t('downloads.link.single') }}</span>
                        <h3 class="linkTitle" v-truncate>{{ link.collection || link.tracks[0]?.title || t('downloads.link.single') }}</h3>
                        <span class="linkMeta">{{ tracksCountText(link.tracks.length) }}</span>
                    </div>
                    <button v-if="link.tracks.length > 1" type="button" class="primaryBtn small" :disabled="!linkPending.length || locked" @click="downloadLinkAll">
                        <DownloadIcon width="15" height="15" />{{ t('downloads.link.downloadAll', { count: linkPending.length }) }}
                    </button>
                </div>
                <div class="trackList">
                    <DownloadTrackRow v-for="tr in link.tracks" :key="tr.source_url!" :track="tr" @redownload="confirmTrack = $event" />
                </div>
            </section>
        </template>

        <template v-else>
            <div v-if="searching && !anyResults" class="skeleton">
                <div class="skelTop">
                    <div class="skelHero">
                        <span class="skelCover"></span>
                        <span class="skelLines"><i></i><i></i><i></i></span>
                    </div>
                    <div class="skelList">
                        <div v-for="n in 4" :key="n" class="skelRow"><span class="skelThumb"></span><span class="skelLines"><i></i><i></i></span></div>
                    </div>
                </div>
            </div>

            <div v-else-if="searchError" class="stateBox">
                <span class="stateText error">{{ t('downloads.searchError') }}</span>
            </div>

            <div v-else-if="!searching && !anyResults" class="stateBox">
                <div class="stateIcon"><SearchIcon width="18" height="18" /></div>
                <span class="stateText">{{ t('downloads.emptyNothingTitle') }}</span>
                <span class="stateDesc">{{ t('downloads.emptyNothingDesc') }}</span>
            </div>

            <template v-else-if="tab === 'all'">
                <div class="topGrid">
                    <section v-if="best" class="bestWrap">
                        <div class="blockHead">
                            <h3 class="blockTitle">{{ t('downloads.topResult') }}</h3>
                        </div>
                        <div v-if="best.kind === 'artist'" class="bestCard" :style="bestStyle" @click="openArtist(best.artist)">
                            <div class="bestArt round"><CoverImage :src="best.artist.picture_url" :fallback="defaultCover" /></div>
                            <div class="bestBottom">
                                <div class="bestText">
                                    <div class="bestType">{{ t('downloads.typeArtist') }}</div>
                                    <div class="bestName" v-truncate>{{ best.artist.name }}</div>
                                    <div v-if="best.artist.nb_album" class="bestMeta">
                                        <span>{{ albumsCountText(best.artist.nb_album) }}</span>
                                    </div>
                                </div>
                                <span class="bestGo">
                                    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                                </span>
                            </div>
                        </div>
                        <div v-else class="bestCard" :style="bestStyle" @click="downloadBest">
                            <div class="bestArt"><CoverImage :src="best.track.cover_url" :fallback="defaultTrackCover" /></div>
                            <div class="bestBottom">
                                <div class="bestText">
                                    <div class="bestType">{{ t('downloads.typeTrack') }}</div>
                                    <div class="bestName" v-truncate>{{ best.track.title }}</div>
                                    <div class="bestMeta">
                                        <span v-truncate>{{ best.track.artist }}</span>
                                        <span v-if="best.track.duration_secs" class="dim">{{ formatTime(best.track.duration_secs) }}</span>
                                    </div>
                                </div>
                                <span class="bestGo download" :class="{ done: best.track.in_library || bestQueued?.stage === 'done', busy: bestQueued && bestQueued.stage !== 'done' && bestQueued.stage !== 'error' }">
                                    <span v-if="bestQueued && bestQueued.stage !== 'done' && bestQueued.stage !== 'error'" class="btnSpinner"></span>
                                    <svg v-else-if="best.track.in_library || bestQueued?.stage === 'done'" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                                    <DownloadIcon v-else width="20" height="20" />
                                </span>
                            </div>
                        </div>
                    </section>

                    <section v-if="topTracks.length" class="topTracks">
                        <div class="blockHead">
                            <h3 class="blockTitle">{{ t('downloads.tracksTitle') }}</h3>
                            <button v-if="tracks.length > topTracks.length" type="button" class="seeAllBtn" @click="selectTab('tracks')">
                                {{ t('downloads.showAll') }}
                                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                            </button>
                        </div>
                        <div class="trackList">
                            <DownloadTrackRow v-for="tr in topTracks" :key="trackKey(tr.artist, tr.title, tr.album)" :track="tr" @download="remember" @redownload="confirmTrack = $event" />
                        </div>
                    </section>
                </div>

                <section v-if="artists.length" class="block">
                    <div class="blockHead">
                        <h3 class="blockTitle">{{ t('downloads.artistsTitle') }}</h3>
                        <button v-if="artists.length > 5" type="button" class="seeAllBtn" @click="selectTab('artists')">
                            {{ t('downloads.showAll') }}
                            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                        </button>
                    </div>
                    <div class="artistRow">
                        <div v-for="a in artists.slice(0, 8)" :key="a.id" class="artistCard" @click="openArtist(a)">
                            <div class="artistAvatar"><CoverImage :src="a.picture_url" :fallback="defaultCover" /></div>
                            <div class="artistName" v-truncate>{{ a.name }}</div>
                            <div v-if="a.nb_album" class="artistSub">{{ albumsCountText(a.nb_album) }}</div>
                        </div>
                    </div>
                </section>

                <section v-if="albums.length" class="block lineBlock">
                    <div class="blockHead">
                        <h3 class="blockTitle">{{ t('downloads.albumsTitle') }}</h3>
                        <div class="blockActions">
                            <button v-if="albums.length > 4" type="button" class="seeAllBtn" @click="selectTab('albums')">
                                {{ t('downloads.showAll') }}
                                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                            </button>
                            <div v-if="albumsLine.canPrev.value || albumsLine.canNext.value" class="lineControls">
                                <button type="button" :disabled="!albumsLine.canPrev.value" @click="albumsLine.scrollByPage(-1)"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg></button>
                                <button type="button" :disabled="!albumsLine.canNext.value" @click="albumsLine.scrollByPage(1)"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                            </div>
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

            <section v-else-if="tab === 'tracks'" class="block compactTop">
                <div class="trackList">
                    <DownloadTrackRow v-for="tr in tracks" :key="trackKey(tr.artist, tr.title, tr.album)" :track="tr" @download="remember" @redownload="confirmTrack = $event" />
                </div>
                <div v-if="loadingMore" class="loadMore"><span class="btnSpinner light"></span></div>
            </section>

            <section v-else-if="tab === 'albums'" class="block compactTop">
                <div class="cardGrid">
                    <DownloadAlbumCard v-for="al in albums" :key="al.id" :album="al" @open="remember" @download="remember" />
                </div>
            </section>

            <section v-else class="block compactTop">
                <div class="cardGrid artists">
                    <div v-for="a in artists" :key="a.id" class="artistCard" @click="openArtist(a)">
                        <div class="artistAvatar"><CoverImage :src="a.picture_url" :fallback="defaultCover" /></div>
                        <div class="artistName" v-truncate>{{ a.name }}</div>
                        <div v-if="a.nb_album" class="artistSub">{{ albumsCountText(a.nb_album) }}</div>
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
    padding: 20px 32px 48px;
    display: flex;
    flex-direction: column;
    background-color: var(--light-bg);
}
.dlPage::-webkit-scrollbar { width: 6px; }
.dlPage::-webkit-scrollbar-track { background: transparent; }
.dlPage::-webkit-scrollbar-thumb { background: hsla(0, 0%, 100%, 0.12); border-radius: 3px; }

.dlHeader {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 4px 0 4px;
    flex-shrink: 0;
}
.dlTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 34px;
    font-weight: 900;
    line-height: 1.1;
    letter-spacing: -0.02em;
    color: hsla(0, 0%, 100%, 0.96);
    user-select: none;
}
.dlHeaderActions {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
}
.folderPill {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    max-width: 240px;
    padding: 0 14px 0 12px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.65);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    user-select: none;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.folderPill:hover {
    background-color: hsla(0, 0%, 100%, 0.09);
    color: hsla(0, 0%, 100%, 0.96);
}
.folderPill svg { flex-shrink: 0; opacity: 0.7; }
.folderPillName {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.headerIconBtn {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.65);
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth), transform 80ms var(--ease-smooth);
}
.headerIconBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.1);
    color: #fff;
}
.headerIconBtn:active { transform: scale(0.92); }

.searchDock {
    position: sticky;
    top: -20px;
    z-index: 6;
    margin: 0 -32px;
    padding: 14px 32px 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    background-color: var(--light-bg);
}
.searchBox {
    position: relative;
    display: flex;
    align-items: center;
    height: 50px;
    border-radius: 16px;
    background-color: hsla(0, 0%, 100%, 0.055);
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.searchBox:hover,
.searchBox:focus-within {
    background-color: hsla(0, 0%, 100%, 0.085);
}
.searchBox.disabled { opacity: 0.5; }
.searchLead {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 100%;
    flex-shrink: 0;
    cursor: text;
    color: hsla(0, 0%, 100%, 0.45);
    transition: color var(--duration-hover) var(--ease-smooth);
}
.searchBox:focus-within .searchLead { color: hsla(0, 0%, 100%, 0.85); }
.searchBox.link .searchLead { color: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%); }
.searchInput {
    flex: 1;
    min-width: 0;
    height: 100%;
    background: transparent;
    font-size: 15px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.94);
}
.searchInput::placeholder { user-select: none; color: hsla(0, 0%, 100%, 0.36); font-weight: 500; }
.searchInput:disabled { cursor: not-allowed; }
.modeTag {
    flex-shrink: 0;
    margin-right: 8px;
    padding: 4px 10px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 600;
    color: hsl(var(--accent-h), min(var(--accent-s), 70%), 74%);
    background: hsla(0, 0%, 100%, 0.07);
}
.searchSpinner {
    flex-shrink: 0;
    margin-right: 14px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid hsla(0, 0%, 100%, 0.14);
    border-top-color: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
    animation: spin .8s linear infinite;
}
.searchClear {
    flex-shrink: 0;
    margin-right: 8px;
    width: 34px;
    height: 34px;
    border-radius: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.42);
    background: transparent;
    transition: color var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth);
}
.searchClear:hover { color: #fff; background-color: hsla(0, 0%, 100%, 0.08); }

.tabs { display: flex; gap: 6px; flex-wrap: wrap; }
.tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 14px;
    border-radius: 999px;
    cursor: pointer;
    font-size: 12.5px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.7);
    background-color: hsla(0, 0%, 100%, 0.06);
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
    user-select: none;
}
.tab:hover:not(:disabled) { background-color: hsla(0, 0%, 100%, 0.1); color: #fff; }
.tab:disabled { opacity: 0.35; cursor: default; }
.tab.active {
    color: var(--light-bg);
    background-color: hsla(0, 0%, 100%, 0.94);
}
.tab.active:hover { background-color: #fff; color: var(--light-bg); }
.tabCount {
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.45);
}
.tab.active .tabCount {
    color: var(--light-bg);
    opacity: 0.65;
}

.recentBar {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow-x: auto;
    scrollbar-width: none;
    scroll-behavior: smooth;
}
.recentBar::-webkit-scrollbar { display: none; }
.recentChip {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 240px;
    height: 30px;
    padding: 0 4px 0 13px;
    border-radius: 999px;
    cursor: pointer;
    font-size: 12.5px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.68);
    background-color: hsla(0, 0%, 100%, 0.055);
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
    user-select: none;
}
.recentChip:hover { background-color: hsla(0, 0%, 100%, 0.1); color: #fff; }
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
    color: hsla(0, 0%, 100%, 0.32);
    background: transparent;
    transition: color var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth);
}
.recentChip:hover .recentRemove { color: hsla(0, 0%, 100%, 0.6); }
.recentRemove:hover { color: #fff; background: hsla(0, 0%, 100%, 0.12); }
.recentClearBtn {
    flex-shrink: 0;
    height: 30px;
    padding: 0 10px;
    border-radius: 999px;
    cursor: pointer;
    background: transparent;
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.42);
    user-select: none;
    transition: color var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth);
}
.recentClearBtn:hover { color: hsla(0, 0%, 100%, 0.9); background-color: hsla(0, 0%, 100%, 0.06); }

.setupCard {
    margin-top: 12px;
    display: flex;
    align-items: center;
    gap: 24px;
    padding: 20px 24px;
    border-radius: 20px;
    background-color: hsla(0, 0%, 100%, 0.04);
}
.setupBody { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
.setupTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 18px;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.96);
}
.setupText { font-size: 13px; line-height: 1.45; color: hsla(0, 0%, 100%, 0.55); max-width: 620px; }
.setupChips { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 4px; }
.setupChip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.6);
    background: hsla(0, 0%, 100%, 0.06);
}
.setupChip em { font-style: normal; opacity: 0.6; }
.chipDot { width: 6px; height: 6px; border-radius: 50%; background: hsla(0, 0%, 100%, 0.25); }
.setupChip.on { color: hsla(0, 0%, 100%, 0.92); }
.setupChip.on .chipDot { background: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%); }
.setupChip.busy .chipDot { background: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%); animation: pulse 1s var(--ease-smooth-in-out) infinite; }
@keyframes pulse { 50% { opacity: 0.3; } }
.setupProgress { display: flex; flex-direction: column; gap: 6px; margin-top: 4px; max-width: 520px; }
.setupBar { position: relative; height: 6px; border-radius: 3px; overflow: hidden; background: hsla(0, 0%, 100%, 0.06); }
.setupBarFill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 3px;
    background: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
    transition: width .25s var(--ease-smooth);
}
.setupBarFill.indeterminate { width: 30%; animation: slide 1.2s var(--ease-smooth-in-out) infinite; }
@keyframes slide { 0% { left: -30%; } 100% { left: 100%; } }
.setupProgressText { font-size: 12px; color: hsla(0, 0%, 100%, 0.58); font-variant-numeric: tabular-nums; }
.setupError { font-size: 12px; color: hsla(2, 80%, 74%, 0.95); }

.updateBar {
    margin-top: 12px;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 14px 16px 14px 20px;
    border-radius: 16px;
    background-color: hsla(0, 0%, 100%, 0.04);
}
.updateText { flex: 1; display: flex; flex-direction: column; gap: 2px; font-size: 12px; color: hsla(0, 0%, 100%, 0.55); }
.updateText strong { font-size: 14px; font-weight: 600; color: hsla(0, 0%, 100%, 0.94); }

.primaryBtn {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 44px;
    padding: 0 22px;
    border-radius: 999px;
    cursor: pointer;
    font-size: 14px;
    font-weight: 600;
    color: var(--light-bg);
    background: hsla(0, 0%, 100%, 0.96);
    transition: transform 80ms var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth);
}
.primaryBtn:hover:not(:disabled) { background: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%); }
.primaryBtn:active:not(:disabled) { transform: scale(0.96); }
.primaryBtn:disabled { cursor: default; opacity: 0.45; }
.primaryBtn.small { height: 38px; padding: 0 18px; font-size: 13px; }
.ghostBtn {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    padding: 0 16px;
    border-radius: 999px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.94);
    background: hsla(0, 0%, 100%, 0.08);
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.ghostBtn:hover:not(:disabled) { background: hsla(0, 0%, 100%, 0.14); }
.btnSpinner {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid hsla(0, 0%, 0%, 0.2);
    border-top-color: hsla(0, 0%, 0%, 0.75);
    animation: spin .8s linear infinite;
}
.btnSpinner.light { border-color: hsla(0, 0%, 100%, 0.15); border-top-color: hsla(0, 0%, 100%, 0.85); }
@keyframes spin { to { transform: rotate(360deg); } }

.block { margin-top: 28px; display: flex; flex-direction: column; gap: 12px; }
.block.compactTop { margin-top: 12px; }
.blockHead { display: flex; align-items: center; justify-content: space-between; gap: 16px; min-height: 28px; }
.blockTitle {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 18px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.96);
    user-select: none;
}
.blockCount {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 24px;
    height: 22px;
    padding: 0 8px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.07);
    font-size: 12px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.65);
}
.blockActions {
    display: flex;
    align-items: center;
    gap: 6px;
}
.actionPill {
    height: 30px;
    padding: 0 12px;
    border-radius: 999px;
    cursor: pointer;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.72);
    font-size: 12px;
    font-weight: 600;
    user-select: none;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.actionPill:hover { background-color: hsla(0, 0%, 100%, 0.11); color: #fff; }
.actionPill.danger:hover { background-color: hsla(2, 70%, 55%, 0.16); color: hsla(2, 85%, 78%, 1); }

.textBtn,
.seeAllBtn {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 5px 10px;
    border-radius: 999px;
    cursor: pointer;
    background: transparent;
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.55);
    user-select: none;
    transition: color var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth);
}
.seeAllBtn { padding-right: 8px; }
.textBtn:hover,
.seeAllBtn:hover { color: #fff; background-color: hsla(0, 0%, 100%, 0.06); }
.seeAllBtn svg { transition: transform var(--duration-hover) var(--ease-smooth); }
.seeAllBtn:hover svg { transform: translateX(2px); }

.trackList {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0 -12px;
}
.trackList.activeList {
    gap: 5px;
}

.linkCard {
    --link-color: hsl(var(--accent-h), min(var(--accent-s), 55%), 52%);
    display: flex;
    align-items: center;
    gap: 20px;
    padding: 18px 20px;
    border-radius: 20px;
    background-color: color-mix(in srgb, var(--link-color) 18%, var(--light-bg));
}
.linkCover {
    width: 84px;
    height: 84px;
    border-radius: 12px;
    overflow: hidden;
    flex-shrink: 0;
    background-color: #202020;
    box-shadow: 0 14px 32px -8px rgba(0, 0, 0, 0.4);
}
.linkInfo {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
}
.linkType {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.55);
    user-select: none;
}
.linkTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 24px;
    font-weight: 900;
    letter-spacing: -0.02em;
    line-height: 1.15;
    color: hsla(0, 0%, 100%, 0.96);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.linkMeta {
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.62);
    user-select: none;
}

.topGrid {
    margin-top: 14px;
    display: grid;
    grid-template-columns: minmax(270px, 340px) minmax(0, 1fr);
    gap: 28px;
    align-items: stretch;
}
.bestWrap, .topTracks {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
}
.bestCard {
    --best-color: hsl(var(--accent-h), min(var(--accent-s), 55%), 52%);
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 18px;
    padding: 20px;
    box-sizing: border-box;
    border-radius: 20px;
    cursor: pointer;
    background-color: color-mix(in srgb, var(--best-color) 18%, var(--light-bg));
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.bestCard:hover {
    background-color: color-mix(in srgb, var(--best-color) 26%, var(--light-bg));
}
.bestArt {
    width: 100px;
    height: 100px;
    border-radius: 12px;
    overflow: hidden;
    flex-shrink: 0;
    background-color: #202020;
    box-shadow: 0 18px 36px -10px rgba(0, 0, 0, 0.4);
}
.bestArt.round { border-radius: 50%; }
.bestBottom {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 14px;
    min-width: 0;
}
.bestText {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
}
.bestType {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.56);
    user-select: none;
}
.bestName {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 26px;
    font-weight: 900;
    letter-spacing: -0.02em;
    line-height: 1.15;
    color: hsla(0, 0%, 100%, 0.96);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.bestMeta {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-size: 13px;
    color: hsla(0, 0%, 100%, 0.65);
    white-space: nowrap;
}
.bestMeta .dim { color: hsla(0, 0%, 100%, 0.45); font-variant-numeric: tabular-nums; flex-shrink: 0; }
.bestGo {
    flex-shrink: 0;
    width: 48px;
    height: 48px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--light-bg);
    background: hsla(0, 0%, 100%, 0.96);
    transition: transform .2s cubic-bezier(0.34, 1.56, 0.64, 1), background-color var(--duration-hover) var(--ease-smooth);
}
.bestCard:hover .bestGo:not(.done):not(.busy) {
    transform: scale(1.06);
    background-color: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
}
.bestGo.done { color: hsl(var(--accent-h), min(var(--accent-s), 60%), 30%); }

.artistRow {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(128px, 1fr));
    grid-template-rows: auto;
    grid-auto-rows: 0;
    column-gap: 10px;
    overflow: hidden;
    margin: 0 -10px;
}
.artistCard {
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    padding: 12px 10px 14px;
    border-radius: 18px;
    cursor: pointer;
    user-select: none;
    text-align: center;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.artistCard:hover {
    background-color: hsla(0, 0%, 100%, 0.05);
}
.artistAvatar {
    width: 100%;
    max-width: 112px;
    aspect-ratio: 1;
    margin-bottom: 8px;
    border-radius: 50%;
    overflow: hidden;
    background-color: #202020;
    transition: filter var(--duration-hover) var(--ease-smooth);
}
.artistCard:hover .artistAvatar { filter: brightness(0.82); }
.artistName {
    max-width: 100%;
    font-size: 13.5px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.92);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.artistSub { font-size: 12px; font-variant-numeric: tabular-nums; color: hsla(0, 0%, 100%, 0.45); }

.albumLine {
    display: flex;
    gap: 16px;
    overflow-x: auto;
    scroll-behavior: smooth;
    scrollbar-width: none;
}
.albumLine::-webkit-scrollbar { display: none; }
.lineControls { display: flex; gap: 4px; }
.lineControls button {
    width: 30px;
    height: 30px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.72);
    background: hsla(0, 0%, 100%, 0.06);
    transition: opacity var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.lineControls button:hover:not(:disabled) { background: hsla(0, 0%, 100%, 0.12); color: #fff; }
.lineControls button:disabled { opacity: 0.28; cursor: default; }

.cardGrid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(164px, 1fr));
    gap: 24px 16px;
    padding: 4px 0;
}
.cardGrid.artists {
    grid-template-columns: repeat(auto-fill, minmax(136px, 1fr));
    gap: 12px 10px;
    margin: 0 -10px;
}

.stateBox {
    flex: 1;
    min-height: 240px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    text-align: center;
    user-select: none;
    padding-bottom: 24px;
}
.stateIcon {
    width: 44px;
    height: 44px;
    border-radius: 14px;
    display: flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 4px;
    background-color: hsla(0, 0%, 100%, 0.055);
    color: hsla(0, 0%, 100%, 0.78);
}
.stateText { font-size: 15px; font-weight: 600; color: hsla(0, 0%, 100%, 0.85); }
.stateText.error { color: hsla(2, 80%, 74%, 0.95); }
.stateDesc { max-width: 400px; font-size: 13px; line-height: 1.45; color: hsla(0, 0%, 100%, 0.42); }
.bigSpinner {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: 3px solid hsla(0, 0%, 100%, 0.1);
    border-top-color: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
    animation: spin .8s linear infinite;
}
.loadMore { display: flex; justify-content: center; padding: 16px 0 4px; }

.skeleton { margin-top: 20px; }
.skelTop {
    display: grid;
    grid-template-columns: minmax(270px, 340px) minmax(0, 1fr);
    gap: 28px;
    align-items: stretch;
}
.skelHero {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 20px;
    padding: 20px;
    border-radius: 20px;
    background: hsla(0, 0%, 100%, 0.035);
}
.skelCover { width: 100px; height: 100px; border-radius: 12px; }
.skelList { display: flex; flex-direction: column; gap: 4px; }
.skelRow { display: flex; gap: 12px; align-items: center; height: 64px; }
.skelThumb { width: 48px; height: 48px; border-radius: 6px; }
.skelLines { flex: 1; display: flex; flex-direction: column; gap: 8px; }
.skelLines i { display: block; height: 12px; border-radius: 6px; width: 52%; }
.skelLines i:nth-child(2) { width: 32%; }
.skelLines i:nth-child(3) { width: 22%; }
.skelCover, .skelThumb, .skelLines i {
    background: linear-gradient(90deg, hsla(0, 0%, 100%, 0.05) 0%, hsla(0, 0%, 100%, 0.1) 50%, hsla(0, 0%, 100%, 0.05) 100%);
    background-size: 200% 100%;
    animation: shimmer 1.3s var(--ease-smooth-in-out) infinite;
}
@keyframes shimmer { from { background-position: 100% 0; } to { background-position: -100% 0; } }

.fade-enter-active, .fade-leave-active { transition: opacity var(--duration-hover) var(--ease-smooth); }
.fade-enter-from, .fade-leave-to { opacity: 0; }

@media (max-width: 900px) {
    .topGrid,
    .skelTop { grid-template-columns: minmax(0, 1fr); }
}
@media (max-width: 768px), (pointer: coarse) and (max-width: 900px) {
    .dlPage { padding: 14px 16px 80px; }
    .dlTitle { font-size: 28px; }
    .folderPill { display: none; }
    .searchDock { top: -14px; margin: 0 -16px; padding: 12px 16px; }
    .searchBox { height: 46px; border-radius: 14px; }
    .tabs,
    .recentBar { flex-wrap: nowrap; overflow-x: auto; scrollbar-width: none; margin: 0 -16px; padding: 0 16px; }
    .tabs::-webkit-scrollbar { display: none; }
    .tab { flex-shrink: 0; }
    .trackList { margin: 0 -12px; }
    .setupCard { flex-direction: column; align-items: flex-start; }
    .linkCard { flex-wrap: wrap; }
    .bestArt { width: 84px; height: 84px; }
    .bestName { font-size: 22px; }
    .artistRow {
        display: flex;
        gap: 4px;
        margin: 0 -16px;
        padding: 0 16px;
        overflow-x: auto;
        scrollbar-width: none;
    }
    .artistRow::-webkit-scrollbar { display: none; }
    .artistCard { flex: 0 0 104px; padding: 8px 6px 10px; }
    .artistAvatar { max-width: 84px; }
    .lineControls { display: none; }
}
</style>
