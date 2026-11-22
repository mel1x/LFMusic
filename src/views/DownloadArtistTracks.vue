<script setup lang="ts">
import { ref, computed, watch, onMounted, nextTick } from 'vue'
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

const artistId = computed(() => {
    const raw = Array.isArray(route.params.id) ? route.params.id[0] : route.params.id
    return Number(raw)
})
const artistName = computed(() => {
    const raw = Array.isArray(route.query.name) ? route.query.name[0] : route.query.name
    return (raw as string | null | undefined) || ''
})

const pageEl = ref<HTMLElement | null>(null)
const tracks = ref<ExternalSearchTrack[]>([])
const loading = ref(true)
const loadingMore = ref(false)
const hasMore = ref(false)
const failed = ref(false)
let nextIndex = 0
let seq = 0

function append(page: ExternalSearchTrack[]) {
    const seen = new Set(tracks.value.map(tr => trackKey(tr.artist, tr.title, tr.album)))
    for (const tr of page) {
        const k = trackKey(tr.artist, tr.title, tr.album)
        if (seen.has(k)) continue
        seen.add(k)
        tracks.value.push(tr)
    }
}

async function load(id: number) {
    const mySeq = ++seq
    tracks.value = []
    hasMore.value = false
    loadingMore.value = false
    failed.value = false
    nextIndex = 0
    loading.value = true
    if (!Number.isFinite(id)) { loading.value = false; failed.value = true; return }
    try {
        const page = await artistTracks(id, 0, PAGE, artistName.value)
        if (mySeq !== seq) return
        append(page.tracks)
        nextIndex = page.next_index
        hasMore.value = page.has_more
    } catch (e) {
        console.error('Ошибка загрузки треков исполнителя:', e)
        if (mySeq === seq) failed.value = true
    } finally {
        if (mySeq === seq) loading.value = false
    }
    await nextTick()
    void fillViewport()
}

async function loadMore() {
    if (loading.value || loadingMore.value || !hasMore.value) return
    const mySeq = seq
    loadingMore.value = true
    try {
        const page = await artistTracks(artistId.value, nextIndex, PAGE, artistName.value)
        if (mySeq !== seq) return
        append(page.tracks)
        nextIndex = page.next_index
        hasMore.value = page.has_more
    } catch (e) {
        console.error('Ошибка подгрузки треков исполнителя:', e)
        if (mySeq === seq) hasMore.value = false
    } finally {
        if (mySeq === seq) loadingMore.value = false
    }
}

// A short first page may not overflow: keep loading until the list scrolls or ends.
async function fillViewport() {
    const el = pageEl.value
    if (!el || !hasMore.value) return
    if (el.scrollHeight - el.clientHeight < 400) {
        await loadMore()
        await nextTick()
        void fillViewport()
    }
}

function onScroll(e: Event) {
    const el = e.target as HTMLElement
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 400) void loadMore()
}

watch(artistId, (id) => { void load(id) })
onMounted(() => { void load(artistId.value) })

useKeepDownloadedInLibrary(() => [tracks.value])

function goBack() { goBackOrHome(router) }

const { confirmTrack, confirmMessage, confirmRedownload } = useRedownloadConfirm()
</script>

<template>
    <div class="pageRoot" ref="pageEl" @scroll="onScroll">
        <button type="button" class="backBtn" @click="goBack">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
            <span>{{ t('downloads.back') }}</span>
        </button>

        <div class="pageHead">
            <div class="listType">{{ t('downloads.artistAllTracks') }}</div>
            <h2 v-truncate class="listName">{{ artistName || t('downloads.artistLabel') }}</h2>
        </div>

        <LoadingSpinner v-if="loading" />

        <div v-else-if="failed || tracks.length === 0" class="emptyState">
            <div class="emptyIcon"><DownloadIcon width="20" height="20" /></div>
            <div class="emptyText">{{ failed ? t('downloads.artistNotFound') : t('downloads.artistTracksEmpty') }}</div>
        </div>

        <template v-else>
            <div class="listHead">
                <span class="colTitle">{{ t('downloads.listColTitle') }}</span>
                <span class="colTime"><TimeIcon width="15" height="15" /></span>
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
            <div v-if="loadingMore" class="loadMore"><span class="btnSpinner"></span></div>
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

.listHead {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 36px;
    margin-bottom: 4px;
    padding: 0 12px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.42);
    user-select: none;
}
.listHead .colTitle {
    flex: 1;
    min-width: 0;
    padding-left: 60px;
}
.listHead .colTime {
    display: flex;
    justify-content: flex-end;
    width: 44px;
    flex-shrink: 0;
}
.listHead .colAction {
    width: 38px;
    flex-shrink: 0;
}

.tracksList {
    display: flex;
    flex-direction: column;
    gap: 2px;
}
.loadMore { display: flex; justify-content: center; padding: 18px 0 4px; }
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

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px) {
    .pageRoot { padding: 12px 16px 80px; }
    .listName { font-size: 28px; }
    .listHead { display: none; }
    .tracksList { margin: 0 -8px; }
}
</style>
