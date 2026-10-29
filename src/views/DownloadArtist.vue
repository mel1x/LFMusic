<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
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
import { useScrollLine } from '../composables/useScrollLine'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()
const { artistDetail, enqueue, notQueued, ytdlpMissing } = useDownloads()

const artistId = computed(() => {
    const raw = Array.isArray(route.params.id) ? route.params.id[0] : route.params.id
    return Number(raw)
})

const detail = ref<ExternalArtistDetail | null>(null)
const loading = ref(true)
const notFound = ref(false)

const albumsLine = useScrollLine(190, 16)

async function load(id: number) {
    loading.value = true
    notFound.value = false
    detail.value = null
    busyPopular.value = false
    donePopular.value = false
    popularRedownloadOpen.value = false
    if (!Number.isFinite(id)) { loading.value = false; notFound.value = true; return }
    try {
        const d = await artistDetail(id)
        detail.value = d
        notFound.value = d === null
    } catch (e) {
        console.error('Ошибка загрузки исполнителя:', e)
        notFound.value = true
    } finally {
        loading.value = false
    }
    // observe() must run only after loading=false, otherwise the albums section
    // isn't in the DOM yet (spinner shown), the row never measures, and cards
    // stay visibility:hidden, looking like an empty albums block.
    await nextTick()
    albumsLine.observe()
}

watch(artistId, (id) => { void load(id) })
onMounted(() => { void load(artistId.value) })
onUnmounted(() => { albumsLine.unobserve() })

useKeepDownloadedInLibrary(() => (detail.value ? [detail.value.top_tracks] : []))

function goBack() { goBackOrHome(router) }

const PREVIEW_COUNT = 5
const previewTracks = computed(() => detail.value?.top_tracks.slice(0, PREVIEW_COUNT) ?? [])

function openAllTracks() {
    const d = detail.value
    if (!d) return
    router.push({ path: `/downloads/artist/${d.id}/tracks`, query: { name: d.name } })
}

// Download popular: top-3 most popular tracks. Downloads whichever of the top-3
// aren't already in the library or queued; if all three are already downloaded,
// asks for confirmation and force-redownloads them.
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
    // Nothing missing. If all three are actually in the library, ask about a
    // redownload; otherwise the rest are already queued, so do nothing.
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

// Redownload confirmation (track already in library)
const { confirmTrack, confirmMessage, confirmRedownload } = useRedownloadConfirm()
</script>

<template>
    <div class="pageRoot">
        <button class="backBtn" @click="goBack">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
            <span>{{ t('downloads.back') }}</span>
        </button>

        <LoadingSpinner v-if="loading" />

        <div v-else-if="notFound || !detail" class="emptyAlbums">
            <div class="emptyIcon"><DownloadIcon width="22" height="22" /></div>
            <div class="emptyText">{{ t('downloads.artistNotFound') }}</div>
        </div>

        <template v-else>
            <div class="pageHat">
                <div class="coverWrap">
                    <CoverImage :src="detail.picture_url" :fallback="defaultCover" class="cover" alt="avatar" />
                </div>
                <div class="dataZone">
                    <div class="info">
                        <div class="listType">{{ t('downloads.artistLabel') }}</div>
                        <div class="listNameRow">
                            <h2 v-truncate class="listName">{{ detail.name }}</h2>
                        </div>
                        <div class="badges">
                            <div v-if="detail.albums.length" class="badge">{{ t('downloads.artistAlbumsCount', { count: detail.albums.length }) }}</div>
                        </div>
                    </div>
                    <div class="controls" v-if="detail.top_tracks.length">
                        <button
                            class="playListBtn"
                            :class="{ done: donePopular }"
                            :disabled="busyPopular || donePopular || ytdlpMissing"
                            @click="downloadPopular"
                        >
                            <span v-if="busyPopular" class="btnSpinner"></span>
                            <svg v-else-if="donePopular" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                            <DownloadIcon v-else width="16" height="16" />
                            {{ donePopular ? t('downloads.albumQueued') : t('downloads.downloadPopular') }}
                        </button>
                    </div>
                </div>
            </div>

            <!-- Popular tracks -->
            <div v-if="detail.top_tracks.length" class="pageLine">
                <div class="lineHat">
                    <div class="lineTitle" @click="openAllTracks">
                        {{ t('downloads.popularTracks') }}
                        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
                    </div>
                </div>
                <div class="tracksList">
                    <DownloadTrackRow
                        v-for="(track, i) in previewTracks"
                        :key="i"
                        :track="track"
                        @redownload="confirmTrack = $event"
                    />
                </div>
                <button class="showMoreBtn" @click="openAllTracks">{{ t('downloads.showMore') }}</button>
            </div>

            <!-- Albums (section hidden if none) -->
            <div v-if="detail.albums.length" class="pageLine">
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
                    <DownloadAlbumCard v-for="al in detail.albums" :key="al.id" :album="al" />
                </div>
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
.backBtn svg { color: hsla(0, 0%, 100%, 0.6); transition: .24s ease; }
.backBtn:hover svg { color: hsla(0, 0%, 100%, 1); }

.pageHat {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 30px;
    width: 100%;
    flex-shrink: 0;
}
.pageHat .coverWrap {
    position: relative;
    width: 200px;
    height: 200px;
    flex-shrink: 0;
    border-radius: 9999px;
    overflow: hidden;
    background-color: #202020;
}
.pageHat .cover {
    user-select: none;
    width: 200px;
    height: 200px;
    object-fit: cover;
}
.pageHat .dataZone {
    display: flex;
    flex-direction: column;
    gap: 22px;
    width: calc(100% - 230px);
}
.pageHat .dataZone .info {
    width: 100%;
    display: flex;
    flex-direction: column;
}
.listType {
    user-select: none;
    margin-left: 1px;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.4);
    width: fit-content;
}
.listNameRow {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    margin-top: 2px;
    width: fit-content;
    max-width: 100%;
}
.listName {
    cursor: default;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 42px;
    font-weight: 900;
    color: hsla(0, 0%, 100%, 0.9);
    line-height: 1.1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.badges {
    display: flex;
    flex-direction: row;
    gap: 8px;
    margin-top: 14px;
}
.badge {
    user-select: none;
    padding: 4px 10px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    font-size: 12px;
    font-weight: 500;
}
.controls {
    display: flex;
    flex-direction: row;
    gap: 12px;
}
.playListBtn {
    user-select: none;
    cursor: pointer;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 6px;
    padding: 11px 18px 11px 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 1);
    color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
    font-size: 12px;
    font-weight: 600;
    transition: .24s ease;
}
.playListBtn:hover { background-color: hsla(0, 0%, 100%, 0.86); }
.playListBtn:disabled {
    cursor: not-allowed;
    background-color: hsla(0, 0%, 100%, 0.3);
}
.playListBtn.done {
    cursor: default;
    color: hsla(0, 0%, 100%, 0.6);
    background-color: hsla(0, 0%, 100%, 0.1);
}
.btnSpinner {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid hsla(var(--accent-h), var(--accent-s), 20%, 0.3);
    border-top-color: hsla(var(--accent-h), var(--accent-s), 20%, 0.9);
    animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }

.pageLine {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 13px;
    margin-top: 36px;
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
    font-size: 24px;
}
.lineTitle {
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 2px;
    transition: .24s ease;
}
.lineTitle svg { color: hsla(0, 0%, 100%, 0.5); transition: .24s ease; }
.lineTitle:hover { color: hsla(0, 0%, 100%, 1); }
.lineTitle:hover svg { color: hsla(0, 0%, 100%, 0.9); transform: translateX(2px); }
.lineTitle.nonClickable { cursor: default; }
.showMoreBtn {
    align-self: flex-start;
    cursor: pointer;
    user-select: none;
    padding: 8px 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.75);
    font-size: 13px;
    font-weight: 600;
    transition: .24s ease;
}
.showMoreBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.1);
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
}
.lineControls button:hover {
    border: 2px solid hsla(0, 0%, 100%, 0.4);
    color: hsla(0, 0%, 100%, 0.95);
}
.lineControls button:disabled:hover {
    border: 2px solid hsla(0, 0%, 100%, 0.2);
    color: hsla(0, 0%, 100%, 0.6);
}
.pageLine:hover .lineControls button { opacity: 1; }
.pageLine:hover .lineControls button:disabled { opacity: 0.65; }

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

.tracksList {
    display: flex;
    flex-direction: column;
    gap: 3px;
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
.emptyAlbums.small {
    flex: none;
    padding: 36px 0;
}
.emptyIcon {
    padding: 10px;
    border-radius: 9px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: #fff;
}
.emptyText {
    user-select: none;
    font-size: 14px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.7);
}
/* Album cards bleed 8px for their hover background — give them room. */
.lineList {
    padding: 8px;
    margin: -8px;
}
</style>
