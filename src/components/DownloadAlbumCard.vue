<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'

import CoverImage from './CoverImage.vue'
import DownloadIcon from './Icons/DownloadIcon.vue'
import defaultCover from '../assets/cover.webp'

import { useDownloads, type ExternalSearchAlbum } from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'

// External-results album card (mirrors regular search card style): cover opens
// the album download page, the circle button queues the whole album (fetching
// its tracklist by Deezer id). Width comes from the parent .lineList's --card-w.
const props = defineProps<{ album: ExternalSearchAlbum }>()

// open/download: "card was used" signals for search suggestions (listened to on
// the search page; not on the artist page).
const emit = defineEmits<{ (e: 'open'): void; (e: 'download'): void }>()

const { t } = useI18n()
const router = useRouter()
const { albumDetail, enqueueMany, queue, trackKey, ytdlpMissing } = useDownloads()

const busy = ref(false)
const done = ref(false)

function openAlbum() {
    emit('open')
    router.push(`/downloads/album/${props.album.id}`)
}
function openArtist() {
    if (props.album.artist_id != null) router.push(`/downloads/artist/${props.album.artist_id}`)
}

async function downloadAll(e: MouseEvent) {
    e.stopPropagation()
    if (busy.value || done.value || ytdlpMissing.value) return
    emit('download')
    busy.value = true
    try {
        const detail = await albumDetail(props.album.id)
        if (!detail) return
        const queued = new Set(
            queue.value.filter(i => i.status !== 'error').map(i => trackKey(i.artist, i.title, i.album)),
        )
        const pending = detail.tracks.filter(
            tr => !tr.in_library && !queued.has(trackKey(tr.artist, tr.title, tr.album)),
        )
        if (pending.length > 0) await enqueueMany(pending)
        done.value = true
    } catch (err) {
        console.error('Ошибка скачивания альбома:', err)
    } finally {
        busy.value = false
    }
}
</script>

<template>
    <div class="playlistItem albumItem" @click="openAlbum">
        <div class="coverWrap">
            <CoverImage class="cover" :src="album.cover_url" :fallback="defaultCover" />
            <button
                class="overlayBtn dlBtn"
                :class="{ busy, done }"
                :disabled="busy || done || ytdlpMissing"
                @click="downloadAll"
                v-tooltip="done ? t('downloads.albumQueued') : t('downloads.downloadAll')"
            >
                <span v-if="busy" class="cardSpinner"></span>
                <svg v-else-if="done" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                <DownloadIcon v-else width="16" height="16" />
            </button>
        </div>
        <div v-truncate class="title">{{ album.title }}</div>
        <div
            v-truncate
            class="subtitle"
            :class="{ clickable: album.artist_id != null }"
            @click.stop="album.artist_id != null && openArtist()"
        >{{ album.artist }}</div>
    </div>
</template>

<style scoped>
.playlistItem {
    flex-shrink: 0;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
}
.playlistItem .coverWrap {
    position: relative;
    width: var(--card-w, 190px);
    height: var(--card-w, 190px);
    border-radius: 18px;
    overflow: hidden;
    background-color: #202020;
}
.playlistItem .cover {
    display: block;
    width: var(--card-w, 190px);
    height: var(--card-w, 190px);
    border-radius: 18px;
    object-fit: cover;
    transition: filter .24s ease;
    user-select: none;
}
.playlistItem:hover .cover {
    filter: brightness(0.6);
}
.playlistItem .overlayBtn {
    position: absolute;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    border-radius: 9999px;
    color: hsla(0, 0%, 100%, 0.95);
    background-color: hsla(0, 0%, 0%, 0.45);
    backdrop-filter: blur(8px);
    opacity: 0;
    transform: translateY(4px);
    transition: opacity .24s ease, transform .24s ease, background-color .24s ease, color .24s ease;
    z-index: 2;
}
.playlistItem:hover .overlayBtn,
.playlistItem .overlayBtn.busy,
.playlistItem .overlayBtn.done {
    opacity: 1;
    transform: translateY(0);
}
.playlistItem .dlBtn {
    right: 12px;
    bottom: 12px;
    padding: 12px;
    color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
    background-color: hsla(0, 0%, 100%, 0.95);
}
.playlistItem .dlBtn:hover {
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
    background-color: hsla(0, 0%, 100%, 1);
}
.playlistItem .dlBtn:disabled {
    cursor: default;
}
.playlistItem .dlBtn.done {
    color: hsla(0, 0%, 100%, 0.85);
    background-color: hsla(0, 0%, 0%, 0.55);
}
.cardSpinner {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid hsla(var(--accent-h), var(--accent-s), 20%, 0.3);
    border-top-color: hsla(var(--accent-h), var(--accent-s), 20%, 0.9);
    animation: cardSpin 0.8s linear infinite;
}
@keyframes cardSpin {
    to { transform: rotate(360deg); }
}
.playlistItem .title {
    color: hsla(0, 0%, 100%, 0.85);
    font-weight: 500;
    font-size: 14px;
    margin-left: 4px;
    max-width: var(--card-w, 190px);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.playlistItem .subtitle {
    color: hsla(0, 0%, 100%, 0.4);
    font-weight: 500;
    font-size: 12px;
    margin-left: 4px;
    margin-top: -4px;
    max-width: var(--card-w, 190px);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.playlistItem .subtitle.clickable {
    cursor: pointer;
}
.playlistItem .subtitle.clickable:hover {
    color: hsla(0, 0%, 100%, 0.75);
}
</style>
