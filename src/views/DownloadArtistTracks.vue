<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { goBackOrHome } from '../router'

import DownloadIcon from '../components/Icons/DownloadIcon.vue'
import TimeIcon from '../components/Icons/TimeIcon.vue'
import LoadingSpinner from '../components/LoadingSpinner.vue'
import ConfirmModal from '../components/ConfirmModal.vue'
import DownloadTrackRow from '../components/DownloadTrackRow.vue'

import { useDownloads, useKeepDownloadedInLibrary, useRedownloadConfirm, type ExternalSearchTrack } from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()
const { artistTracks, trackKey } = useDownloads()

const PAGE = 40
const MAX_EMPTY_PAGES = 6

const artistId = computed(() => {
    const raw = Array.isArray(route.params.id) ? route.params.id[0] : route.params.id
    return Number(raw)
})
const artistName = computed(() => {
    const raw = Array.isArray(route.query.name) ? route.query.name[0] : route.query.name
    return (raw as string | null | undefined) || ''
})

const pageEl = ref<HTMLElement | null>(null)
const sentinelEl = ref<HTMLElement | null>(null)
const tracks = ref<ExternalSearchTrack[]>([])
const loading = ref(true)
const loadingMore = ref(false)
const loadMoreError = ref(false)
const hasMore = ref(false)
const failed = ref(false)
let nextIndex = 0
let seq = 0
let emptyStreak = 0
let observer: IntersectionObserver | null = null

function append(page: ExternalSearchTrack[]): number {
    const seen = new Set(tracks.value.map(tr => trackKey(tr.artist, tr.title, tr.album)))
    let added = 0
    for (const tr of page) {
        const k = trackKey(tr.artist, tr.title, tr.album)
        if (seen.has(k)) continue
        seen.add(k)
        tracks.value.push(tr)
        added++
    }
    return added
}

function isNearBottom(): boolean {
    const el = pageEl.value
    if (!el) return false
    return el.scrollHeight - el.scrollTop - el.clientHeight < 600
}

async function load(id: number) {
    const mySeq = ++seq
    scrollY.value = 0
    pageEl.value?.scrollTo({ top: 0 })
    tracks.value = []
    hasMore.value = false
    loadingMore.value = false
    loadMoreError.value = false
    failed.value = false
    nextIndex = 0
    emptyStreak = 0
    loading.value = true
    if (!Number.isFinite(id)) { loading.value = false; failed.value = true; return }
    try {
        const prevIndex = nextIndex
        const page = await artistTracks(id, 0, PAGE, artistName.value)
        if (mySeq !== seq) return
        const added = append(page.tracks)
        emptyStreak = added > 0 ? 0 : 1
        nextIndex = page.next_index
        hasMore.value = page.has_more && page.next_index > prevIndex && emptyStreak < MAX_EMPTY_PAGES
    } catch (e) {
        console.error('Ошибка загрузки треков исполнителя:', e)
        if (mySeq === seq) failed.value = true
    } finally {
        if (mySeq === seq) loading.value = false
    }
    await nextTick()
    if (mySeq === seq && hasMore.value && isNearBottom()) void loadMore()
}

async function loadMore() {
    if (loading.value || loadingMore.value || !hasMore.value) return
    const mySeq = seq
    loadMoreError.value = false
    loadingMore.value = true
    try {
        let totalAdded = 0
        while (hasMore.value) {
            const prevIndex = nextIndex
            const page = await artistTracks(artistId.value, prevIndex, PAGE, artistName.value)
            if (mySeq !== seq) return
            const added = append(page.tracks)
            totalAdded += added
            emptyStreak = added > 0 ? 0 : emptyStreak + 1
            nextIndex = page.next_index
            hasMore.value = page.has_more && page.next_index > prevIndex && emptyStreak < MAX_EMPTY_PAGES
            if (totalAdded > 0 || !hasMore.value) break
        }
    } catch (e) {
        console.error('Ошибка подгрузки треков исполнителя:', e)
        if (mySeq === seq) loadMoreError.value = true
    } finally {
        if (mySeq === seq) loadingMore.value = false
    }
    await nextTick()
    if (mySeq === seq && hasMore.value && !loadMoreError.value && emptyStreak < MAX_EMPTY_PAGES && isNearBottom()) {
        void loadMore()
    }
}

function syncObserver() {
    observer?.disconnect()
    observer = null
    if (!pageEl.value || !sentinelEl.value || typeof IntersectionObserver === 'undefined') return
    observer = new IntersectionObserver(
        (entries) => {
            if (!loadMoreError.value && entries.some(e => e.isIntersecting)) void loadMore()
        },
        { root: pageEl.value, rootMargin: '0px 0px 600px 0px' },
    )
    observer.observe(sentinelEl.value)
}

const scrollY = ref(0)
const showStickyHeader = computed(() => scrollY.value > 140)
function scrollToTop() {
    pageEl.value?.scrollTo({ top: 0, behavior: 'smooth' })
}

function onScroll(e: Event) {
    const el = e.target as HTMLElement
    scrollY.value = el.scrollTop
    if (!loadMoreError.value && isNearBottom()) void loadMore()
}

watch(artistId, (id) => { void load(id) })
watch([pageEl, sentinelEl], syncObserver, { flush: 'post' })
onMounted(() => { void load(artistId.value) })
onUnmounted(() => {
    observer?.disconnect()
    observer = null
})

useKeepDownloadedInLibrary(() => [tracks.value])

function goBack() { goBackOrHome(router) }

const { confirmTrack, confirmMessage, confirmRedownload } = useRedownloadConfirm()
</script>

<template>
    <div class="pageRoot" ref="pageEl" @scroll="onScroll">
        <div class="stickyHeaderAnchor">
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div v-truncate class="stickyTitle">{{ artistName || t('downloads.artistLabel') }}</div>
                    <button type="button" class="toTopBtn" @click="scrollToTop">Вверх</button>
                </div>
            </Transition>
        </div>

        <button type="button" class="backBtn" @click="goBack">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>
            <span>{{ t('downloads.back') }}</span>
        </button>

        <div class="pageHead">
            <div class="listType">{{ t('downloads.artistAllTracks') }}</div>
            <h2 v-truncate class="listName">{{ artistName || t('downloads.artistLabel') }}</h2>
        </div>

        <LoadingSpinner v-if="loading || (tracks.length === 0 && hasMore)" />

        <div v-else-if="failed || tracks.length === 0" class="emptyState">
            <div class="emptyIcon"><DownloadIcon width="20" height="20" /></div>
            <div class="emptyText">{{ failed ? t('downloads.artistNotFound') : t('downloads.artistTracksEmpty') }}</div>
        </div>

        <template v-else>
            <div class="listHead">
                <span class="colTitle">{{ t('downloads.listColTitle') }}</span>
                <span class="colTime"><TimeIcon width="13" height="13" /></span>
                <span class="colAction" aria-hidden="true"></span>
            </div>
            <div class="tracksList">
                <DownloadTrackRow
                    v-for="track in tracks"
                    :key="trackKey(track.artist, track.title, track.album)"
                    :track="track"
                    @redownload="confirmTrack = $event"
                />
            </div>
            <div v-if="loadingMore || (loadMoreError && hasMore)" class="loadMore">
                <span v-if="loadingMore" class="btnSpinner"></span>
                <button v-else type="button" class="retryMoreBtn" @click="loadMore">{{ t('downloads.action.retry') }}</button>
            </div>
            <div v-if="hasMore" ref="sentinelEl" class="loadSentinel" aria-hidden="true"></div>
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
    </div>
</template>

<style scoped>
.pageRoot {
    padding: 12px 32px 48px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: var(--light-bg);
    overflow-y: auto;
}
.pageRoot::-webkit-scrollbar { width: 6px; }
.pageRoot::-webkit-scrollbar-track { background: transparent; }
.pageRoot::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
}

.backBtn {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
    width: fit-content;
    padding: 6px 14px 6px 10px;
    margin: 0 0 8px -10px;
    border-radius: 999px;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.8);
    font-size: 14px;
    cursor: pointer;
    user-select: none;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.backBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.06);
    color: #fff;
}
.backBtn svg { transition: transform var(--duration-hover) var(--ease-smooth); }
.backBtn:hover svg { transform: translateX(-2px); }

.pageHead {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    gap: 4px;
    margin: 6px 0 20px;
    min-width: 0;
}
.listType {
    user-select: none;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.45);
}
.listName {
    cursor: default;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 36px;
    font-weight: 900;
    letter-spacing: -0.02em;
    color: hsla(0, 0%, 100%, 0.96);
    line-height: 1.1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.stickyHeaderAnchor {
    position: sticky;
    top: -12px;
    height: 0;
    z-index: 10;
    margin: 0 -32px;
}
.stickyHeader {
    user-select: none;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 32px;
    background-color: var(--light-bg);
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 20px;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.94);
}
.stickyTitle {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.sticky-header-enter-active,
.sticky-header-leave-active {
    transition: opacity var(--duration-hover) var(--ease-smooth), transform var(--duration-hover) var(--ease-smooth);
}
.sticky-header-enter-from,
.sticky-header-leave-to {
    opacity: 0;
    transform: translateY(-8px);
}
.toTopBtn {
    flex-shrink: 0;
    cursor: pointer;
    padding: 6px 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.07);
    color: hsla(0, 0%, 100%, 1);
    font-family: "Inter", sans-serif;
    font-size: 13px;
    font-weight: 500;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.toTopBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
}

.listHead {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    gap: 14px;
    height: 36px;
    margin-bottom: 4px;
    padding: 0 16px;
    font-size: 11px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.42);
    user-select: none;
}
.listHead .colTitle {
    flex: 1;
    min-width: 0;
    padding-left: 62px;
}
.listHead .colTime {
    display: flex;
    justify-content: flex-end;
    width: 44px;
    flex-shrink: 0;
}
.listHead .colAction {
    width: 36px;
    flex-shrink: 0;
}

.tracksList {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    gap: 2px;
}
.loadMore { display: flex; justify-content: center; flex-shrink: 0; padding: 18px 0 4px; }
.retryMoreBtn {
    height: 32px;
    padding: 0 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.07);
    color: hsla(0, 0%, 100%, 0.9);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    user-select: none;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.retryMoreBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.13);
    color: #fff;
}
.loadSentinel { height: 1px; flex-shrink: 0; }
.btnSpinner {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid hsla(0, 0%, 100%, 0.15);
    border-top-color: hsla(0, 0%, 100%, 0.85);
    animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }

.emptyState {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding-bottom: 40px;
}
.emptyIcon {
    width: 44px;
    height: 44px;
    border-radius: 14px;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.8);
}
.emptyText {
    user-select: none;
    font-size: 14px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.7);
}
</style>
