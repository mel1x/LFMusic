<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import CoverImage from '../components/CoverImage.vue'
import DownloadIcon from '../components/Icons/DownloadIcon.vue'
import LoadingSpinner from '../components/LoadingSpinner.vue'
import ConfirmModal from '../components/ConfirmModal.vue'
import DownloadTrackRow from '../components/DownloadTrackRow.vue'

import defaultCover from '../assets/cover.webp'

import { useDownloads, type ExternalAlbumDetail, type ExternalSearchTrack } from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'
import { tracksCountText } from '../utils/format'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()
const { albumDetail, enqueueMany, enqueue, queue, trackKey, ytdlpMissing } = useDownloads()

const albumId = computed(() => {
    const raw = Array.isArray(route.params.id) ? route.params.id[0] : route.params.id
    return Number(raw)
})

const detail = ref<ExternalAlbumDetail | null>(null)
const loading = ref(true)
const notFound = ref(false)

async function load(id: number) {
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

// Once a track finishes downloading, mark the matching track as "in library" so
// its button stays "Downloaded" after the queue's "done" entry is cleared,
// instead of reverting to "Download" (in_library is a one-shot snapshot from
// the initial albumDetail() fetch, never refreshed on its own).
watch(queue, (items) => {
    const doneKeys = new Set(
        items.filter(i => i.status === 'done').map(i => trackKey(i.artist, i.title, i.album)),
    )
    if (doneKeys.size === 0 || !detail.value) return
    for (const tr of detail.value.tracks) {
        if (!tr.in_library && doneKeys.has(trackKey(tr.artist, tr.title, tr.album))) tr.in_library = true
    }
})

function goBack() { router.back() }

function openArtist() {
    if (detail.value?.artist_id != null) router.push(`/downloads/artist/${detail.value.artist_id}`)
}

// Download all tracks
const busyAll = ref(false)
const doneAll = ref(false)

async function downloadAll() {
    const d = detail.value
    if (!d || busyAll.value || doneAll.value || ytdlpMissing.value) return
    busyAll.value = true
    try {
        const queued = new Set(
            queue.value.filter(i => i.status !== 'error').map(i => trackKey(i.artist, i.title, i.album)),
        )
        const pending = d.tracks.filter(
            tr => !tr.in_library && !queued.has(trackKey(tr.artist, tr.title, tr.album)),
        )
        if (pending.length > 0) await enqueueMany(pending)
        doneAll.value = true
    } catch (e) {
        console.error('Ошибка скачивания альбома:', e)
    } finally {
        busyAll.value = false
    }
}

// Redownload confirmation (track already in library)
const confirmTrack = ref<ExternalSearchTrack | null>(null)
const confirmMessage = computed(() =>
    confirmTrack.value ? t('downloads.redownloadText', { title: confirmTrack.value.title }) : '',
)
async function confirmRedownload() {
    const track = confirmTrack.value
    confirmTrack.value = null
    if (!track) return
    try { await enqueue(track, true) } catch (e) { console.error('Ошибка повторного скачивания:', e) }
}
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
                        <div class="badges">
                            <div
                                v-truncate
                                class="badge"
                                :class="{ clickable: detail.artist_id != null }"
                                @click="detail.artist_id != null && openArtist()"
                            >{{ detail.artist }}</div>
                            <div class="badge">{{ tracksCountText(detail.tracks.length) }}</div>
                        </div>
                    </div>
                    <div class="controls">
                        <button
                            class="playListBtn"
                            :class="{ done: doneAll }"
                            :disabled="busyAll || doneAll || ytdlpMissing || detail.tracks.length === 0"
                            @click="downloadAll"
                        >
                            <span v-if="busyAll" class="btnSpinner"></span>
                            <svg v-else-if="doneAll" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                            <DownloadIcon v-else width="16" height="16" />
                            {{ doneAll ? t('downloads.albumQueued') : t('downloads.downloadAll') }}
                        </button>
                    </div>
                </div>
            </div>

            <div class="pageLine">
                <div class="tracksList">
                    <DownloadTrackRow
                        v-for="(track, i) in detail.tracks"
                        :key="i"
                        :track="track"
                        :show-album="false"
                        @redownload="confirmTrack = $event"
                    />
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
    gap: 30px;
    width: 100%;
    flex-shrink: 0;
}
.pageHat .coverWrap {
    position: relative;
    width: 200px;
    height: 200px;
    flex-shrink: 0;
    border-radius: 16px;
    overflow: hidden;
    background-color: #202020;
    box-shadow: 0 12px 34px hsla(0, 0%, 0%, 0.4);
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
    justify-content: flex-end;
    gap: 18px;
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
    min-width: 0;
}
.badge {
    user-select: none;
    max-width: 320px;
    padding: 4px 10px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: .24s ease;
}
.badge.clickable { cursor: pointer; }
.badge.clickable:hover {
    background-color: hsla(0, 0%, 100%, 0.1);
    color: hsla(0, 0%, 100%, 0.9);
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
.tracksList {
    display: flex;
    flex-direction: column;
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
</style>
