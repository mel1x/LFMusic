<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'

import CoverImage from './CoverImage.vue'
import DownloadIcon from './Icons/DownloadIcon.vue'
import defaultCover from '../assets/cover.webp'

import { useDownloads, type ExternalSearchAlbum } from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'
import { tracksCountText } from '../utils/format'

// Catalog album card: the cover opens the album page, the floating button
// queues every track not already in the library. Width comes from the parent
// (grid cell, or `--card-w` in a horizontal row).
const props = defineProps<{ album: ExternalSearchAlbum }>()
const emit = defineEmits<{ (e: 'open'): void; (e: 'download'): void }>()

const { t } = useI18n()
const router = useRouter()
const { albumDetail, enqueue, ytdlpMissing } = useDownloads()

const busy = ref(false)
const queued = ref(false)

function openAlbum() {
    emit('open')
    router.push(`/downloads/album/${props.album.id}`)
}
function openArtist() {
    if (props.album.artist_id != null) router.push(`/downloads/artist/${props.album.artist_id}`)
}

async function downloadAlbum() {
    if (busy.value || queued.value || ytdlpMissing.value) return
    emit('download')
    busy.value = true
    try {
        const detail = await albumDetail(props.album.id)
        const pending = detail?.tracks.filter(tr => !tr.in_library) ?? []
        if (pending.length > 0) await enqueue(pending)
        queued.value = true
    } catch (err) {
        console.error('Ошибка скачивания альбома:', err)
    } finally {
        busy.value = false
    }
}
</script>

<template>
    <div class="albumCard" @click="openAlbum">
        <div class="coverWrap">
            <CoverImage class="cover" :src="album.cover_url" :fallback="defaultCover" />
            <div class="coverShade"></div>
            <button
                class="fab"
                :class="{ busy, queued }"
                :disabled="busy || queued || ytdlpMissing"
                v-tooltip="queued ? t('downloads.albumQueued') : t('downloads.downloadAll')"
                @click.stop="downloadAlbum"
            >
                <span v-if="busy" class="fabSpinner"></span>
                <svg v-else-if="queued" width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                <DownloadIcon v-else width="18" height="18" />
            </button>
        </div>
        <div v-truncate class="cardTitle">{{ album.title }}</div>
        <div class="cardSub">
            <span v-truncate class="subArtist" :class="{ clickable: album.artist_id != null }" @click.stop="openArtist">{{ album.artist }}</span>
            <span v-if="album.nb_tracks" class="subCount">· {{ tracksCountText(album.nb_tracks) }}</span>
        </div>
    </div>
</template>

<style scoped>
.albumCard {
    width: var(--card-w, 100%);
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px;
    margin: -8px;
    border-radius: 16px;
    cursor: pointer;
    user-select: none;
    box-sizing: content-box;
    transition: background-color .22s ease;
}
.albumCard:hover { background-color: hsla(0, 0%, 100%, 0.04); }
.coverWrap {
    position: relative;
    width: 100%;
    aspect-ratio: 1;
    border-radius: 12px;
    overflow: hidden;
    background-color: #202020;
    margin-bottom: 8px;
    box-shadow: 0 8px 24px hsla(0, 0%, 0%, 0.25);
}
.cover { width: 100%; height: 100%; }
.coverShade {
    position: absolute;
    inset: 0;
    background: linear-gradient(to top, hsla(0, 0%, 0%, 0.45), transparent 45%);
    opacity: 0;
    transition: opacity .22s ease;
}
.albumCard:hover .coverShade { opacity: 1; }
.fab {
    position: absolute;
    right: 10px;
    bottom: 10px;
    width: 42px;
    height: 42px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
    background-color: hsla(0, 0%, 100%, 0.95);
    box-shadow: 0 6px 18px hsla(0, 0%, 0%, 0.35);
    opacity: 0;
    transform: translateY(8px) scale(0.92);
    transition: opacity .2s ease, transform .22s cubic-bezier(0.22, 1, 0.36, 1), background-color .2s ease;
}
.albumCard:hover .fab,
.fab.busy,
.fab.queued {
    opacity: 1;
    transform: translateY(0) scale(1);
}
.fab:hover:not(:disabled) { background-color: #fff; transform: scale(1.07); }
.fab:disabled { cursor: default; }
.fab.queued {
    color: hsla(145, 55%, 32%, 1);
}
.fabSpinner {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid hsla(var(--accent-h), var(--accent-s), 20%, 0.25);
    border-top-color: hsla(var(--accent-h), var(--accent-s), 20%, 0.9);
    animation: spin .8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }
.cardTitle {
    font-size: 14px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.92);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.cardSub {
    display: flex;
    gap: 4px;
    min-width: 0;
    font-size: 12.5px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.45);
    white-space: nowrap;
}
.subArtist { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
.subArtist.clickable:hover { color: hsla(0, 0%, 100%, 0.9); }
.subCount { flex-shrink: 0; }
</style>
