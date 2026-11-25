<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { goBackOrHome } from '../router'

import CoverImage from '../components/CoverImage.vue'
import DownloadIcon from '../components/Icons/DownloadIcon.vue'
import TimeIcon from '../components/Icons/TimeIcon.vue'
import LoadingSpinner from '../components/LoadingSpinner.vue'
import ConfirmModal from '../components/ConfirmModal.vue'
import DownloadTrackRow from '../components/DownloadTrackRow.vue'

import defaultCover from '../assets/cover.webp'

import { useDownloads, useKeepDownloadedInLibrary, useRedownloadConfirm, type ExternalAlbumDetail } from '../composables/useDownloads'
import { useCoverTint } from '../composables/useCoverTint'
import { useI18n } from '../composables/useI18n'
import { tracksCountText, formatTotalDuration } from '../utils/format'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()
const { albumDetail, enqueue, notQueued, ytdlpMissing } = useDownloads()

const albumId = computed(() => {
    const raw = Array.isArray(route.params.id) ? route.params.id[0] : route.params.id
    return Number(raw)
})

const detail = ref<ExternalAlbumDetail | null>(null)
const loading = ref(true)
const notFound = ref(false)

const heroTint = useCoverTint(() => detail.value?.cover_url)
const heroStyle = computed(() => heroTint.value ? { '--hero-tint': heroTint.value } : {})

const totalDurationSecs = computed(() =>
    (detail.value?.tracks ?? []).reduce((sum, tr) => sum + (tr.duration_secs ?? 0), 0),
)

async function load(id: number) {
    scrollY.value = 0
    pageEl.value?.scrollTo({ top: 0 })
    loading.value = true
    notFound.value = false
    detail.value = null
    busyAll.value = false
    doneAll.value = false
    if (!Number.isFinite(id)) { loading.value = false; notFound.value = true; return }
    try {
        const d = await albumDetail(id)
        detail.value = d
        notFound.value = d === null
    } catch (e) {
        console.error('Ошибка загрузки альбома:', e)
        notFound.value = true
    } finally {
        loading.value = false
    }
}

watch(albumId, (id) => { void load(id) })
onMounted(() => { void load(albumId.value) })

useKeepDownloadedInLibrary(() => (detail.value ? [detail.value.tracks] : []))

function goBack() { goBackOrHome(router) }

function openArtist() {
    if (detail.value?.artist_id != null) router.push(`/downloads/artist/${detail.value.artist_id}`)
}

const busyAll = ref(false)
const doneAll = ref(false)
const allReady = computed(() =>
    doneAll.value || (!!detail.value?.tracks.length && notQueued(detail.value.tracks).length === 0),
)

const pageEl = ref<HTMLElement | null>(null)
const scrollY = ref(0)
const showStickyHeader = computed(() => scrollY.value > 300)
function onScroll(e: Event) {
    scrollY.value = (e.target as HTMLElement).scrollTop
}
function scrollToTop() {
    pageEl.value?.scrollTo({ top: 0, behavior: 'smooth' })
}

async function downloadAll() {
    const d = detail.value
    if (!d || busyAll.value || allReady.value || ytdlpMissing.value) return
    busyAll.value = true
    try {
        const pending = notQueued(d.tracks)
        if (pending.length > 0) await enqueue(pending)
        doneAll.value = true
    } catch (e) {
        console.error('Ошибка скачивания альбома:', e)
    } finally {
        busyAll.value = false
    }
}

const { confirmTrack, confirmMessage, confirmRedownload } = useRedownloadConfirm()
</script>

<template>
    <div class="pageRoot" ref="pageEl" :style="heroStyle" @scroll="onScroll">
        <div class="stickyHeaderAnchor">
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div v-truncate class="stickyTitle">{{ detail?.title || t('downloads.albumLabel') }}</div>
                    <button type="button" class="toTopBtn" @click="scrollToTop">Вверх</button>
                </div>
            </Transition>
        </div>

        <button type="button" class="backBtn" @click="goBack">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>
            <span>{{ t('downloads.back') }}</span>
        </button>

        <LoadingSpinner v-if="loading" />

        <div v-else-if="notFound || !detail" class="emptyAlbums">
            <div class="emptyIcon"><DownloadIcon width="20" height="20" /></div>
            <div class="emptyText">{{ t('downloads.albumNotFound') }}</div>
        </div>

        <template v-else>
            <div class="pageHat">
                <div class="coverWrap">
                    <CoverImage :src="detail.cover_url" :fallback="defaultCover" class="cover" alt="cover" />
                </div>
                <div class="dataZone">
                    <div class="info">
                        <div class="listType">{{ t('downloads.albumLabel') }}</div>
                        <div class="listNameRow">
                            <h2 v-truncate class="listName">{{ detail.title }}</h2>
                        </div>
                        <div
                            v-truncate
                            class="heroArtist"
                            :class="{ clickable: detail.artist_id != null }"
                            @click="detail.artist_id != null && openArtist()"
                        >{{ detail.artist }}</div>
                        <div class="badges">
                            <div class="badge">{{ tracksCountText(detail.tracks.length) }}</div>
                            <div v-if="totalDurationSecs > 0" class="badge">{{ formatTotalDuration(totalDurationSecs) }}</div>
                        </div>
                    </div>
                    <div class="controls">
                        <button
                            type="button"
                            class="primaryBtn"
                            :class="{ done: allReady }"
                            :disabled="busyAll || allReady || ytdlpMissing || detail.tracks.length === 0"
                            @click="downloadAll"
                        >
                            <span v-if="busyAll" class="btnSpinner"></span>
                            <svg v-else-if="allReady" width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                            <DownloadIcon v-else width="17" height="17" />
                            <span>{{ allReady ? t('downloads.albumQueued') : t('downloads.downloadAll') }}</span>
                        </button>
                    </div>
                </div>
            </div>

            <div v-if="detail.tracks.length" class="listHead">
                <span class="colRank">#</span>
                <span class="colTitle">{{ t('downloads.listColTitle') }}</span>
                <span class="colTime"><TimeIcon width="13" height="13" /></span>
                <span class="colAction" aria-hidden="true"></span>
            </div>

            <div class="tracksList">
                <DownloadTrackRow
                    v-for="(track, i) in detail.tracks"
                    :key="i"
                    :track="track"
                    :index="i + 1"
                    :show-album="false"
                    @redownload="confirmTrack = $event"
                />
            </div>
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
    --hero-tint: 120, 120, 120;
    padding: 12px 32px 48px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: var(--light-bg);
    background-image: linear-gradient(
        to bottom,
        rgba(var(--hero-tint), 0.42) 0px,
        rgba(var(--hero-tint), 0.14) 240px,
        rgba(var(--hero-tint), 0) 460px
    );
    background-repeat: no-repeat;
    background-attachment: local;
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

.pageHat {
    display: flex;
    flex-direction: row;
    align-items: flex-end;
    gap: 32px;
    width: 100%;
    flex-shrink: 0;
    padding: 8px 0 0;
}
.pageHat .coverWrap {
    position: relative;
    width: 232px;
    height: 232px;
    flex-shrink: 0;
    border-radius: 16px;
    overflow: hidden;
    background-color: #202020;
    box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.35);
}
.pageHat .cover {
    user-select: none;
    width: 100%;
    height: 100%;
    object-fit: cover;
}
.pageHat .dataZone {
    display: flex;
    flex-direction: column;
    gap: 20px;
    flex: 1;
    min-width: 0;
}
.pageHat .dataZone .info {
    display: flex;
    flex-direction: column;
    min-width: 0;
}
.listType {
    user-select: none;
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.7);
}
.listNameRow {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin-top: 2px;
    max-width: 100%;
}
.listName {
    cursor: default;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 48px;
    font-weight: 900;
    letter-spacing: -0.02em;
    line-height: 1.1;
    color: hsla(0, 0%, 100%, 0.96);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    overflow: hidden;
    overflow-wrap: anywhere;
}
.heroArtist {
    margin-top: 10px;
    max-width: 100%;
    width: fit-content;
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.92);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
    transition: color var(--duration-hover) var(--ease-smooth);
}
.heroArtist.clickable { cursor: pointer; }
.heroArtist.clickable:hover { color: hsl(var(--accent-h), min(var(--accent-s), 70%), 74%); }

.badges {
    display: flex;
    flex-wrap: wrap;
    margin-top: 4px;
    max-width: 100%;
}
.badge {
    user-select: none;
    font-size: 14px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.7);
    white-space: nowrap;
}
.badge + .badge::before {
    content: "\00B7";
    margin: 0 6px;
}

.controls {
    display: flex;
    align-items: center;
    gap: 10px;
}
.primaryBtn {
    user-select: none;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 46px;
    padding: 0 22px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.96);
    color: var(--light-bg);
    font-size: 14px;
    font-weight: 600;
    transition: transform .2s cubic-bezier(0.34, 1.56, 0.64, 1), background-color var(--duration-hover) var(--ease-smooth);
}
.primaryBtn:hover:not(:disabled) {
    transform: scale(1.03);
    background-color: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
}
.primaryBtn:active:not(:disabled) {
    transform: scale(0.96);
    transition-duration: 80ms;
}
.primaryBtn:disabled {
    cursor: not-allowed;
    opacity: 0.35;
}
.primaryBtn.done {
    cursor: default;
    opacity: 1;
    color: hsla(0, 0%, 100%, 0.72);
    background-color: hsla(0, 0%, 100%, 0.1);
}
.btnSpinner {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid hsla(0, 0%, 0%, 0.2);
    border-top-color: hsla(0, 0%, 0%, 0.75);
    animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }

.listHead {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 36px;
    margin-top: 32px;
    margin-bottom: 4px;
    padding: 0 16px;
    font-size: 11px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.42);
    user-select: none;
}
.listHead .colRank {
    width: 22px;
    text-align: center;
    flex-shrink: 0;
}
.listHead .colTitle {
    flex: 1;
    min-width: 0;
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

.tracksList {
    display: flex;
    flex-direction: column;
    gap: 2px;
}

.emptyAlbums {
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
