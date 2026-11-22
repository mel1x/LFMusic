<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'

import CoverImage from './CoverImage.vue'
import DownloadIcon from './Icons/DownloadIcon.vue'
import defaultTrackCover from '../assets/trackCover.webp'

import { usePlayer } from '../composables/usePlayer'
import { useDownloads, type ExternalSearchTrack } from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'

const props = withDefaults(
    defineProps<{
        track: ExternalSearchTrack
        /** Position number instead of the cover (album page). */
        index?: number | null
        showAlbum?: boolean
    }>(),
    { index: null, showAlbum: true },
)

const emit = defineEmits<{
    /** Already in the library - parent confirms before re-downloading. */
    (e: 'redownload', track: ExternalSearchTrack): void
    /** Queued; parents use it as a "this search was useful" signal. */
    (e: 'download', track: ExternalSearchTrack): void
}>()

const { t } = useI18n()
const router = useRouter()
const { formatTime } = usePlayer()
const { itemFor, enqueue, cancel, retry, ytdlpMissing, stageText } = useDownloads()

const item = computed(() => itemFor(props.track))
const stage = computed(() => item.value?.stage ?? null)
const active = computed(() => stage.value !== null && stage.value !== 'done' && stage.value !== 'error')
const inLibrary = computed(() => props.track.in_library || stage.value === 'done')

const RING = 2 * Math.PI * 15
const progressPct = computed(() => Math.round(Math.min(1, Math.max(0, item.value?.progress ?? 0)) * 100))
const ringOffset = computed(() => RING * (1 - Math.min(1, Math.max(0, item.value?.progress ?? 0))))

const statusLine = computed(() => (item.value && stage.value !== 'done' ? stageText(item.value) : ''))

const buttonTooltip = computed(() => {
    if (stage.value === 'error') return t('downloads.action.retry')
    if (active.value) return t('downloads.action.cancel')
    if (inLibrary.value) return t('downloads.action.redownload')
    return t('downloads.action.download')
})

async function onAction() {
    const it = item.value
    try {
        if (it && stage.value === 'error') { await retry([it.id]); return }
        if (it && active.value) { await cancel(it.id); return }
        if (ytdlpMissing.value) return
        if (inLibrary.value) { emit('redownload', props.track); return }
        emit('download', props.track)
        await enqueue(props.track)
    } catch (e) {
        console.error('Ошибка действия скачивания:', e)
    }
}

const artistClickable = computed(() => props.track.artist_id != null)
const albumClickable = computed(() => props.showAlbum && !!props.track.album && props.track.album_id != null)
function openArtist() {
    if (artistClickable.value) router.push(`/downloads/artist/${props.track.artist_id}`)
}
function openAlbum() {
    if (albumClickable.value) router.push(`/downloads/album/${props.track.album_id}`)
}
</script>

<template>
    <div class="dlRow" :class="{ active, failed: stage === 'error', owned: inLibrary && !active }" @dblclick="onAction">
        <div v-if="index != null" class="rowIndex">{{ index }}</div>
        <div v-else class="rowCover">
            <CoverImage :src="track.cover_url" :fallback="defaultTrackCover" />
        </div>

        <div class="rowInfo">
            <div v-truncate class="rowTitle">{{ track.title }}</div>
            <div class="rowMeta">
                <Transition name="statusSwap" mode="out-in">
                    <span v-if="statusLine" :key="'s'" class="rowStatus" :class="{ error: stage === 'error' }" v-truncate>{{ statusLine }}</span>
                    <span v-else :key="'m'" class="rowMetaText" v-truncate>
                        <span class="metaLink" :class="{ clickable: artistClickable }" @click.stop="openArtist">{{ track.artist }}</span>
                        <span v-if="showAlbum && track.album" class="metaAlbum metaLink" :class="{ clickable: albumClickable }" @click.stop="openAlbum">{{ track.album }}</span>
                    </span>
                </Transition>
            </div>
        </div>

        <div class="rowTime">{{ track.duration_secs ? formatTime(track.duration_secs) : '' }}</div>

        <button
            class="actionBtn"
            :class="[stage ?? 'idle', { owned: inLibrary && !active && stage !== 'error' }]"
            :disabled="!item && ytdlpMissing"
            v-tooltip:left="buttonTooltip"
            @click.stop="onAction"
            @dblclick.stop
        >
            <svg v-if="active && stage !== 'queued'" class="ring" :class="{ spin: stage !== 'downloading' }" viewBox="0 0 36 36">
                <circle class="ringTrack" cx="18" cy="18" r="15" />
                <circle
                    class="ringFill"
                    cx="18" cy="18" r="15"
                    :stroke-dasharray="RING"
                    :stroke-dashoffset="stage === 'downloading' ? ringOffset : RING * 0.72"
                />
            </svg>
            <span v-if="stage === 'queued'" class="queuedDots"><i></i><i></i><i></i></span>

            <span class="glyph">
                <svg v-if="active" class="cancelGlyph" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                <svg v-else-if="stage === 'error'" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="M19.08 8A8 8 0 1 0 20 12"/><polyline points="20 4 20 8 16 8"/></svg>
                <svg v-else-if="inLibrary" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                <DownloadIcon v-else width="17" height="17" />
            </span>
        </button>

        <div v-if="active" class="rowBar" aria-hidden="true">
            <span
                class="rowBarFill"
                :class="{ indeterminate: stage === 'searching' || stage === 'processing', queued: stage === 'queued' }"
                :style="stage === 'downloading' ? { width: progressPct + '%' } : undefined"
            ></span>
        </div>
    </div>
</template>

<style scoped>
.dlRow {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    height: 64px;
    padding: 0 12px;
    border-radius: 12px;
    box-sizing: border-box;
    overflow: hidden;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.dlRow:hover {
    background-color: hsla(0, 0%, 100%, 0.05);
}
.dlRow.active {
    background-color: hsla(0, 0%, 100%, 0.04);
}
.dlRow.active:hover {
    background-color: hsla(0, 0%, 100%, 0.065);
}
.dlRow.failed {
    background-color: hsla(2, 70%, 55%, 0.07);
}
.dlRow.failed:hover {
    background-color: hsla(2, 70%, 55%, 0.11);
}
.rowCover {
    width: 48px;
    height: 48px;
    border-radius: 6px;
    overflow: hidden;
    flex-shrink: 0;
    background-color: #202020;
}
.rowIndex {
    width: 22px;
    flex-shrink: 0;
    text-align: center;
    font-size: 14px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.42);
    user-select: none;
}
.rowInfo {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
}
.rowTitle {
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.94);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}
.rowMeta {
    height: 16px;
    font-size: 12px;
    user-select: none;
    min-width: 0;
}
.rowMetaText,
.rowStatus {
    display: block;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.rowMetaText { color: hsla(0, 0%, 100%, 0.45); }
.metaAlbum::before {
    content: "\00B7";
    margin: 0 5px;
    opacity: 0.6;
}
.metaLink.clickable { cursor: pointer; transition: color var(--duration-hover) var(--ease-smooth); }
.metaLink.clickable:hover { color: hsla(0, 0%, 100%, 0.9); }
.rowStatus {
    font-variant-numeric: tabular-nums;
    color: hsl(var(--accent-h), min(var(--accent-s), 70%), 72%);
}
.rowStatus.error { color: hsla(2, 78%, 72%, 0.95); }

.statusSwap-enter-active, .statusSwap-leave-active { transition: opacity var(--duration-hover) var(--ease-smooth), transform var(--duration-hover) var(--ease-smooth); }
.statusSwap-enter-from { opacity: 0; transform: translateY(3px); }
.statusSwap-leave-to { opacity: 0; transform: translateY(-3px); }

.rowTime {
    width: 44px;
    flex-shrink: 0;
    text-align: right;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.45);
    user-select: none;
}

.actionBtn {
    position: relative;
    flex-shrink: 0;
    width: 38px;
    height: 38px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.6);
    background-color: transparent;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth), transform 80ms var(--ease-smooth);
}
.actionBtn:hover:not(:disabled) {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: #fff;
}
.actionBtn:active:not(:disabled) {
    transform: scale(0.92);
}
.actionBtn:disabled { cursor: not-allowed; opacity: 0.4; }
.actionBtn.owned {
    color: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
}
.actionBtn.owned:hover:not(:disabled) { background-color: hsla(0, 0%, 100%, 0.08); color: hsl(var(--accent-h), min(var(--accent-s), 70%), 76%); }
.actionBtn.error {
    color: hsla(2, 80%, 72%, 1);
}
.actionBtn.error:hover:not(:disabled) { background-color: hsla(2, 70%, 55%, 0.16); color: hsla(2, 80%, 82%, 1); }
.glyph { position: relative; display: flex; align-items: center; justify-content: center; }
.cancelGlyph { opacity: 0; transition: opacity var(--duration-hover) var(--ease-smooth); color: hsla(0, 0%, 100%, 0.9); }
.actionBtn:hover .cancelGlyph { opacity: 1; }
.actionBtn:hover .queuedDots { opacity: 0; }

.ring {
    position: absolute;
    inset: 1px;
    width: 36px;
    height: 36px;
    transform: rotate(-90deg);
}
.ring.spin { animation: ringSpin 1s linear infinite; }
.ringTrack { fill: none; stroke: hsla(0, 0%, 100%, 0.1); stroke-width: 2.5; }
.ringFill {
    fill: none;
    stroke: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
    stroke-width: 2.5;
    stroke-linecap: round;
    transition: stroke-dashoffset .3s var(--ease-smooth);
}
@keyframes ringSpin { to { transform: rotate(270deg); } }

.queuedDots {
    position: absolute;
    display: flex;
    gap: 3px;
    transition: opacity var(--duration-hover) var(--ease-smooth);
}
.queuedDots i {
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: hsla(0, 0%, 100%, 0.55);
    animation: dotPulse 1.2s var(--ease-smooth-in-out) infinite;
}
.queuedDots i:nth-child(2) { animation-delay: .15s; }
.queuedDots i:nth-child(3) { animation-delay: .3s; }
@keyframes dotPulse {
    0%, 100% { opacity: 0.25; transform: translateY(0); }
    50% { opacity: 1; transform: translateY(-2px); }
}

.rowBar {
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: 0;
    height: 2.5px;
    border-radius: 2px;
    overflow: hidden;
    background: hsla(0, 0%, 100%, 0.05);
    pointer-events: none;
}
.rowBarFill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 2px;
    background: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
    transition: width .28s var(--ease-smooth);
}
.rowBarFill.queued {
    width: 0;
}
.rowBarFill.indeterminate {
    width: 30%;
    animation: rowBarSlide 1.25s var(--ease-smooth-in-out) infinite;
}
@keyframes rowBarSlide {
    0% { left: -30%; }
    100% { left: 100%; }
}
</style>
