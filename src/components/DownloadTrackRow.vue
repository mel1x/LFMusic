<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'

import CoverImage from './CoverImage.vue'
import DownloadIcon from './Icons/DownloadIcon.vue'
import defaultTrackCover from '../assets/trackCover.webp'

import { usePlayer } from '../composables/usePlayer'
import { useDownloads, type ExternalSearchTrack } from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'

// Candidate track row, shared by the search page and album/artist download pages.
// Artist and album are clickable (to their download pages) when the track has
// a matching Deezer id.
const props = withDefaults(
    defineProps<{
        track: ExternalSearchTrack
        /** Whether to show "· album" (redundant on the album's own page). */
        showAlbum?: boolean
    }>(),
    { showAlbum: true },
)

// redownload: request to redownload (track already in library); parent shows
// the confirm modal (one dialog per list). download: normal enqueue; parent
// uses it as a "query was useful" signal for search suggestions.
const emit = defineEmits<{
    (e: 'redownload', track: ExternalSearchTrack): void
    (e: 'download', track: ExternalSearchTrack): void
}>()

const { t } = useI18n()
const router = useRouter()
const { formatTime } = usePlayer()
const { queue, enqueue, trackKey, ytdlpMissing } = useDownloads()

const isQueued = computed(() => {
    const key = trackKey(props.track.artist, props.track.title, props.track.album)
    return queue.value.some(i => i.status !== 'error' && trackKey(i.artist, i.title, i.album) === key)
})

const artistClickable = computed(() => props.track.artist_id != null)
const albumClickable = computed(() => props.showAlbum && props.track.album != null && props.track.album_id != null)

function onButton() {
    if (isQueued.value || ytdlpMissing.value) return
    if (props.track.in_library) { emit('redownload', props.track); return }
    emit('download', props.track)
    void doDownload()
}

async function doDownload() {
    try {
        await enqueue(props.track)
    } catch (e) {
        console.error('Ошибка добавления в очередь:', e)
    }
}

function openArtist() {
    if (props.track.artist_id != null) router.push(`/downloads/artist/${props.track.artist_id}`)
}
function openAlbum() {
    if (props.track.album_id != null) router.push(`/downloads/album/${props.track.album_id}`)
}
</script>

<template>
    <div class="trackElem">
        <div class="trackCover">
            <CoverImage :src="track.cover_url" :fallback="defaultTrackCover" />
        </div>
        <div class="trackInfo">
            <div v-truncate class="trackName">{{ track.title }}</div>
            <div v-truncate class="trackMeta">
                <span
                    class="metaPart"
                    :class="{ clickable: artistClickable }"
                    @click.stop="artistClickable && openArtist()"
                >{{ track.artist }}</span>
                <template v-if="showAlbum && track.album"> · <span
                    class="metaPart"
                    :class="{ clickable: albumClickable }"
                    @click.stop="albumClickable && openAlbum()"
                >{{ track.album }}</span></template>
            </div>
        </div>
        <div class="trackTime">{{ track.duration_secs ? formatTime(track.duration_secs) : '' }}</div>
        <button
            class="dlBtn"
            :class="{ done: track.in_library && !isQueued, added: isQueued }"
            :disabled="isQueued || ytdlpMissing"
            @click="onButton"
        >
            <template v-if="isQueued">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                <span>{{ t('downloads.btnAdded') }}</span>
            </template>
            <template v-else-if="track.in_library">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                <span>{{ t('downloads.btnDownloaded') }}</span>
            </template>
            <template v-else>
                <DownloadIcon width="16" height="16" />
                <span>{{ t('downloads.btnDownload') }}</span>
            </template>
        </button>
    </div>
</template>

<style scoped>
.trackElem {
    position: relative;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 10px;
    padding: 8px;
    border-radius: 9px;
    height: 58px;
    box-sizing: border-box;
    transition: background-color .24s ease;
}
.trackElem:hover {
    background-color: hsla(0, 0%, 100%, 0.03);
}
.trackCover {
    width: 42px;
    height: 42px;
    background-color: #202020;
    border-radius: 5px;
    overflow: hidden;
    flex-shrink: 0;
}
.trackInfo {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
}
.trackName {
    user-select: none;
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.trackMeta {
    user-select: none;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.4);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.metaPart.clickable {
    cursor: pointer;
    transition: color .18s ease;
}
.metaPart.clickable:hover {
    color: hsla(0, 0%, 100%, 0.85);
}
.trackTime {
    user-select: none;
    font-size: 14px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.4);
    flex-shrink: 0;
}
.dlBtn {
    cursor: pointer;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 34px;
    padding: 0 16px;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 700;
    white-space: nowrap;
    color: hsla(var(--accent-h), min(calc(var(--accent-s) + 12%), 100%), 78%, 1);
    background-color: hsla(var(--accent-h), var(--accent-s), 55%, 0.16);
    transition: background-color .2s ease, color .2s ease, transform .2s ease;
}
.dlBtn svg {
    flex-shrink: 0;
}
.dlBtn:hover {
    background-color: hsla(var(--accent-h), var(--accent-s), 55%, 0.26);
    color: hsla(var(--accent-h), min(calc(var(--accent-s) + 12%), 100%), 86%, 1);
}
.dlBtn:disabled {
    cursor: default;
}
/* Already queued: neutral pill. */
.dlBtn.added {
    cursor: default;
    color: hsla(0, 0%, 100%, 0.55);
    background-color: hsla(0, 0%, 100%, 0.06);
}
.dlBtn.added:hover {
    background-color: hsla(0, 0%, 100%, 0.06);
}
/* Already in library: green pill, clickable for redownload. */
.dlBtn.done {
    cursor: pointer;
    color: hsla(145, 50%, 64%, 0.95);
    background-color: hsla(145, 45%, 50%, 0.12);
}
.dlBtn.done:hover {
    background-color: hsla(145, 45%, 50%, 0.2);
}
</style>
