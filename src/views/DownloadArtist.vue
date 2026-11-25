<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { goBackOrHome } from '../router'

import CoverImage from '../components/CoverImage.vue'
import DownloadIcon from '../components/Icons/DownloadIcon.vue'
import LoadingSpinner from '../components/LoadingSpinner.vue'
import ConfirmModal from '../components/ConfirmModal.vue'
import DownloadTrackRow from '../components/DownloadTrackRow.vue'
import DownloadAlbumCard from '../components/DownloadAlbumCard.vue'

import defaultCover from '../assets/cover.webp'

import { useDownloads, useKeepDownloadedInLibrary, useRedownloadConfirm, type ExternalArtistDetail, type ExternalSearchTrack } from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'
import { albumsCountText, tracksCountText } from '../utils/format'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()
const { artistDetail, artistTracks, enqueue, notQueued, ytdlpMissing } = useDownloads()

const artistId = computed(() => {
    const raw = Array.isArray(route.params.id) ? route.params.id[0] : route.params.id
    return Number(raw)
})

const detail = ref<ExternalArtistDetail | null>(null)
const loading = ref(true)
const notFound = ref(false)
const trackCount = ref<number | null>(null)
let loadId = 0

async function load(id: number) {
    const requestId = ++loadId
    loading.value = true
    trackCount.value = null
    notFound.value = false
    detail.value = null
    busyPopular.value = false
    donePopular.value = false
    popularRedownloadOpen.value = false
    if (!Number.isFinite(id)) { loading.value = false; notFound.value = true; return }
    try {
        const d = await artistDetail(id)
        if (requestId !== loadId) return
        detail.value = d
        notFound.value = d === null
        if (d?.albums.length) {
            void artistTracks(id).then(
                tracks => {
                    if (requestId !== loadId || !detail.value) return
                    trackCount.value = tracks.length
                    if (!detail.value.top_tracks.length) {
                        detail.value.top_tracks = tracks.slice(0, PREVIEW_COUNT).map(track => ({ ...track }))
                    }
                },
                error => console.error('Ошибка загрузки каталога исполнителя:', error),
            )
        }
    } catch (e) {
        console.error('Ошибка загрузки исполнителя:', e)
        if (requestId === loadId) notFound.value = true
    } finally {
        if (requestId === loadId) loading.value = false
    }
}

watch(artistId, (id) => { void load(id) })
onMounted(() => { void load(artistId.value) })
onUnmounted(() => { loadId++ })

useKeepDownloadedInLibrary(() => (detail.value ? [detail.value.top_tracks] : []))

function goBack() { goBackOrHome(router) }

const PREVIEW_COUNT = 5
const previewTracks = computed(() => detail.value?.top_tracks.slice(0, PREVIEW_COUNT) ?? [])

const albumsLabel = computed(() => {
    const n = detail.value?.album_count ?? 0
    return albumsCountText(n).replace(/^\d+\s*/, '')
})
const tracksLabel = computed(() => {
    const n = trackCount.value ?? 0
    return tracksCountText(n).replace(/^\d+\s*/, '')
})

function openAllTracks() {
    const d = detail.value
    if (!d) return
    router.push({ path: `/downloads/artist/${d.id}/tracks`, query: { name: d.name } })
}

const POPULAR_COUNT = 3
const busyPopular = ref(false)
const donePopular = ref(false)
const popularRedownloadOpen = ref(false)

function top3(): ExternalSearchTrack[] {
    return detail.value?.top_tracks.slice(0, POPULAR_COUNT) ?? []
}

async function downloadPopular() {
    const d = detail.value
    if (!d || busyPopular.value || donePopular.value || ytdlpMissing.value) return
    const tracks = top3()
    if (tracks.length === 0) return
    const missing = notQueued(tracks)
    if (missing.length > 0) {
        busyPopular.value = true
        try {
            await enqueue(missing)
            donePopular.value = true
        } catch (e) {
            console.error('Ошибка скачивания популярных треков:', e)
        } finally {
            busyPopular.value = false
        }
        return
    }
    if (tracks.every(tr => tr.in_library)) {
        popularRedownloadOpen.value = true
    } else {
        donePopular.value = true
    }
}

async function confirmPopularRedownload() {
    popularRedownloadOpen.value = false
    const tracks = top3()
    if (tracks.length === 0) return
    busyPopular.value = true
    try {
        await enqueue(tracks, true)
        donePopular.value = true
    } catch (e) {
        console.error('Ошибка повторного скачивания популярных:', e)
    } finally {
        busyPopular.value = false
    }
}

const { confirmTrack, confirmMessage, confirmRedownload } = useRedownloadConfirm()
</script>

<template>
    <div class="pageRoot">
        <LoadingSpinner v-if="loading" />

        <template v-else-if="notFound || !detail">
            <button type="button" class="backBtn" @click="goBack">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>
                <span>{{ t('downloads.back') }}</span>
            </button>
            <div class="emptyAlbums">
                <div class="emptyIcon"><DownloadIcon width="20" height="20" /></div>
                <div class="emptyText">{{ t('downloads.artistNotFound') }}</div>
            </div>
        </template>

        <div v-else class="artistLayout">
            <aside class="artistAside">
                <button type="button" class="backBtn" @click="goBack">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>
                    <span>{{ t('downloads.back') }}</span>
                </button>
                <div class="pageHat">
                    <div class="coverWrap">
                        <CoverImage :src="detail.picture_url" :fallback="defaultCover" class="cover" alt="avatar" />
                    </div>
                    <div class="dataZone">
                        <div class="info">
                            <div class="listType">{{ t('downloads.artistLabel') }}</div>
                            <h2 v-truncate class="listName">{{ detail.name }}</h2>
                            <div v-if="detail.album_count || trackCount !== null" class="badges">
                                <div v-if="detail.album_count" class="badge">
                                    <b>{{ detail.album_count }}</b>
                                    <span>{{ albumsLabel }}</span>
                                </div>
                                <div v-if="trackCount !== null" class="badge">
                                    <b>{{ trackCount }}</b>
                                    <span>{{ tracksLabel }}</span>
                                </div>
                            </div>
                        </div>
                        <div v-if="detail.top_tracks.length" class="controls">
                            <button
                                type="button"
                                class="primaryBtn"
                                :class="{ done: donePopular }"
                                :disabled="busyPopular || donePopular || ytdlpMissing"
                                @click="downloadPopular"
                            >
                                <span v-if="busyPopular" class="btnSpinner"></span>
                                <svg v-else-if="donePopular" width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                                <DownloadIcon v-else width="17" height="17" />
                                <span>{{ donePopular ? t('downloads.albumQueued') : t('downloads.downloadPopular') }}</span>
                            </button>
                        </div>
                    </div>
                </div>
            </aside>

            <div class="artistMain">
                <section v-if="detail.top_tracks.length || detail.albums.length" class="pageLine">
                    <div v-if="detail.top_tracks.length" class="lineHat">
                        <h3 class="lineTitle">{{ t('downloads.popularTracks') }}</h3>
                    </div>
                    <div v-if="previewTracks.length" class="tracksList">
                        <DownloadTrackRow
                            v-for="(track, i) in previewTracks"
                            :key="i"
                            :track="track"
                            @redownload="confirmTrack = $event"
                        />
                    </div>
                    <button v-if="detail.albums.length" type="button" class="showMoreBtn" @click="openAllTracks">
                        <span class="showMoreIcon">
                            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                        </span>
                        <span class="showMoreText">{{ t('downloads.artistAllTracks') }}</span>
                    </button>
                </section>

                <section v-if="detail.albums.length" class="pageLine">
                    <div class="lineHat">
                        <h3 class="lineTitle">{{ t('downloads.artistReleases') }}</h3>
                    </div>
                    <div class="albumGrid">
                        <DownloadAlbumCard v-for="al in detail.albums" :key="al.id" :album="al" />
                    </div>
                </section>
            </div>
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

        <ConfirmModal
            :open="popularRedownloadOpen"
            :title="t('downloads.popularRedownloadTitle')"
            :message="t('downloads.popularRedownloadText')"
            :cancel-text="t('downloads.redownloadCancel')"
            :confirm-text="t('downloads.redownloadConfirm')"
            @cancel="popularRedownloadOpen = false"
            @confirm="confirmPopularRedownload"
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

.artistLayout {
    display: flex;
    align-items: flex-start;
    gap: 40px;
    width: 100%;
}
.artistAside {
    position: sticky;
    top: 0;
    flex-shrink: 0;
    width: clamp(240px, 28%, 320px);
    display: flex;
    flex-direction: column;
}
.artistMain {
    flex: 1;
    min-width: 0;
    padding-top: 44px;
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
    flex-direction: column;
    gap: 16px;
    width: 100%;
}
.pageHat .coverWrap {
    position: relative;
    width: 100%;
    aspect-ratio: 1;
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
    gap: 16px;
    min-width: 0;
}
.pageHat .dataZone .info {
    display: flex;
    flex-direction: column;
    min-width: 0;
}
.listType {
    user-select: none;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.45);
    margin-bottom: 4px;
}
.listName {
    cursor: default;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 36px;
    font-weight: 900;
    letter-spacing: -0.02em;
    line-height: 1.1;
    color: hsla(0, 0%, 100%, 0.96);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    overflow: hidden;
    overflow-wrap: anywhere;
}
.badges {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 12px;
}
.badge {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    padding: 8px 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.05);
    user-select: none;
    white-space: nowrap;
}
.badge b {
    font-size: 16px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.96);
}
.badge span {
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.6);
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
    font-size: 13.5px;
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

.pageLine {
    width: 100%;
    display: flex;
    flex-direction: column;
}
.pageLine + .pageLine { margin-top: 40px; }
.lineHat {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 12px;
}
.lineTitle {
    margin: 0;
    user-select: none;
    font-size: 18px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.96);
}

.tracksList {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0 -16px;
}
.showMoreBtn {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    margin-top: 12px;
    padding: 12px 16px 12px 12px;
    border-radius: 16px;
    background-color: hsla(0, 0%, 100%, 0.04);
    color: hsla(0, 0%, 100%, 0.94);
    cursor: pointer;
    user-select: none;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.showMoreBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.07);
}
.showMoreIcon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 10px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
}
.showMoreText {
    flex: 1;
    text-align: left;
    font-size: 14px;
    font-weight: 600;
}

.albumGrid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 24px 16px;
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
