<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'

import CoverImage from './CoverImage.vue'
import DownloadIcon from './Icons/DownloadIcon.vue'
import PlayIcon from './Icons/PlayIcon.vue'
import PauseIcon from './Icons/PauseIcon.vue'
import defaultTrackCover from '../assets/trackCover.webp'

import { usePlayer } from '../composables/usePlayer'
import { useDownloads, type DownloadItem, type ExternalSearchTrack } from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'

const props = withDefaults(
    defineProps<{
        track: ExternalSearchTrack
        /** Position number instead of the cover (album page). */
        index?: number | null
        showAlbum?: boolean
        /** Explicit queue item when rendered inside the downloads queue. */
        queueItem?: DownloadItem | null
        /** Enables queue-specific hover actions (remove completed entry). */
        queueMode?: boolean
    }>(),
    { index: null, showAlbum: true, queueItem: null, queueMode: false },
)

const emit = defineEmits<{
    /** Already in the library - parent confirms before re-downloading. */
    (e: 'redownload', track: ExternalSearchTrack): void
    /** Queued; parents use it as a "this search was useful" signal. */
    (e: 'download', track: ExternalSearchTrack): void
}>()

const { t } = useI18n()
const router = useRouter()
const { currentTrack, isPlaying, playTrack, togglePlayPause, formatTime } = usePlayer()
const { itemFor, enqueue, cancel, retry, ytdlpMissing, stageText, formatSpeed } = useDownloads()

const item = computed(() => props.queueItem ?? itemFor(props.track))
const stage = computed(() => item.value?.stage ?? null)
const running = computed(() => stage.value === 'downloading' || stage.value === 'searching' || stage.value === 'processing')
const queued = computed(() => stage.value === 'queued')
const active = computed(() => running.value || queued.value)
const failed = computed(() => stage.value === 'error')
const inLibrary = computed(() => props.track.in_library || stage.value === 'done')

const playablePath = computed(() => (stage.value === 'done' && item.value?.file_path) ? item.value.file_path : null)
const isCurrent = computed(() => !!playablePath.value && currentTrack.value?.file_path === playablePath.value)
const isCurrentPlaying = computed(() => isCurrent.value && isPlaying.value)
const rowInteractive = computed(() => !!playablePath.value || !!item.value || !ytdlpMissing.value)

const progressPct = computed(() => Math.round(Math.min(1, Math.max(0, item.value?.progress ?? 0)) * 100))
const speedText = computed(() => item.value?.stage === 'downloading' && item.value.speed_bps ? formatSpeed(item.value.speed_bps) : '')
const errorMessage = computed(() => (item.value && failed.value) ? stageText(item.value) : '')
const runningLabel = computed(() => (item.value && (stage.value === 'searching' || stage.value === 'processing')) ? stageText(item.value) : '')
const showDismiss = computed(() => !!item.value && (failed.value || (props.queueMode && stage.value === 'done')))

const buttonTooltip = computed(() => {
    if (failed.value) return t('downloads.action.retry')
    if (active.value) return t('downloads.action.cancel')
    if (inLibrary.value) return t('downloads.action.redownload')
    return t('downloads.action.download')
})

async function onAction() {
    const it = item.value
    try {
        if (it && failed.value) { await retry([it.id]); return }
        if (it && active.value) { await cancel(it.id); return }
        if (ytdlpMissing.value) return
        if (inLibrary.value) { emit('redownload', props.track); return }
        emit('download', props.track)
        await enqueue(props.track)
    } catch (e) {
        console.error('Ошибка действия скачивания:', e)
    }
}

function onCoverPlay() {
    if (!playablePath.value) return
    if (isCurrent.value) {
        void togglePlayPause()
        return
    }
    void playTrack(playablePath.value)
}

async function onDismiss() {
    if (!item.value) return
    try {
        await cancel(item.value.id)
    } catch (e) {
        console.error('Ошибка удаления элемента очереди:', e)
    }
}

function onRowClick() {
    if (playablePath.value) {
        onCoverPlay()
        return
    }
    void onAction()
}

const artistClickable = computed(() => props.track.artist_id != null)
const albumClickable = computed(() => props.showAlbum && !!props.track.album && props.track.album_id != null)
function onArtistClick(e: MouseEvent) {
    if (!artistClickable.value) return
    e.stopPropagation()
    router.push(`/downloads/artist/${props.track.artist_id}`)
}
function onAlbumClick(e: MouseEvent) {
    if (!albumClickable.value) return
    e.stopPropagation()
    router.push(`/downloads/album/${props.track.album_id}`)
}
</script>

<template>
    <div
        class="dlRow"
        :class="{ running, failed, playable: !!playablePath, playing: isCurrent, paused: isCurrent && !isPlaying, clickable: rowInteractive }"
        @click="onRowClick"
    >
        <div v-if="index != null" class="rowIndex">{{ index }}</div>
        <div v-else class="rowCoverWrap">
            <div class="rowCover">
                <CoverImage :src="track.cover_url" :fallback="defaultTrackCover" />
            </div>
            <template v-if="playablePath">
                <div v-if="isCurrentPlaying" class="playingDot"></div>
                <button
                    type="button"
                    class="coverPlayBtn"
                    :aria-label="isCurrentPlaying ? 'Пауза' : 'Воспроизвести'"
                    @click.stop="onCoverPlay"
                >
                    <PauseIcon v-if="isCurrentPlaying" width="10" height="10" style="margin-left:0.5px;" />
                    <PlayIcon v-else width="10" height="10" style="margin-left:1px;" />
                </button>
            </template>
        </div>

        <div class="rowInfo">
            <div v-truncate class="rowTitle">{{ track.title }}</div>
            <div class="rowMeta">
                <span class="rowMetaText" v-truncate>
                    <span v-if="track.artist" class="metaLink" :class="{ clickable: artistClickable }" @click="onArtistClick">{{ track.artist }}</span>
                    <span v-if="failed" class="metaErr">{{ errorMessage }}</span>
                    <span v-else-if="showAlbum && track.album" class="metaAlbum metaLink" :class="{ clickable: albumClickable }" @click="onAlbumClick">{{ track.album }}</span>
                </span>
            </div>
        </div>

        <div v-if="running" class="rowProgress">
            <div class="progHead">
                <span v-if="runningLabel" class="progStage" v-truncate>{{ runningLabel }}</span>
                <span v-else-if="speedText" class="progDim">{{ speedText }}</span>
                <span v-if="stage !== 'processing'" class="progPct">{{ progressPct }}%</span>
            </div>
            <div class="progTrack">
                <span class="progFill" :style="{ width: progressPct + '%' }"></span>
            </div>
        </div>
        <span v-else-if="queued" class="rowQueued">{{ t('downloads.stage.queued') }}</span>

        <div class="rowTime">{{ track.duration_secs ? formatTime(track.duration_secs) : '' }}</div>

        <div class="rowActions">
            <button
                type="button"
                class="actionBtn"
                :class="[stage ?? 'idle', { owned: inLibrary && !active && !failed }]"
                :disabled="!item && ytdlpMissing"
                v-tooltip:left="buttonTooltip"
                @click.stop="onAction"
            >
                <svg v-if="active" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                <svg v-else-if="failed" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="M19.08 8A8 8 0 1 0 20 12"/><polyline points="20 4 20 8 16 8"/></svg>
                <svg v-else-if="inLibrary" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                <DownloadIcon v-else width="17" height="17" />
            </button>
            <button
                v-if="showDismiss"
                type="button"
                class="dismissBtn"
                :class="{ visible: failed }"
                v-tooltip:left="t('downloads.action.remove')"
                @click.stop="onDismiss"
            >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
            </button>
        </div>
    </div>
</template>

<style scoped>
.dlRow {
    position: relative;
    display: flex;
    align-items: center;
    gap: 14px;
    height: 64px;
    flex-shrink: 0;
    padding: 0 16px;
    border-radius: 12px;
    box-sizing: border-box;
    overflow: hidden;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.dlRow.clickable {
    cursor: pointer;
}
.dlRow:hover {
    background-color: hsla(0, 0%, 100%, 0.05);
}
.dlRow.clickable:active {
    background-color: hsla(0, 0%, 100%, 0.075);
}
.dlRow.running {
    background-color: hsla(0, 0%, 100%, 0.035);
}
.dlRow.running:hover {
    background-color: hsla(0, 0%, 100%, 0.06);
}
.dlRow.running.clickable:active {
    background-color: hsla(0, 0%, 100%, 0.085);
}
.dlRow.failed {
    background-color: hsla(2, 70%, 55%, 0.06);
}
.dlRow.failed:hover {
    background-color: hsla(2, 70%, 55%, 0.1);
}
.dlRow.failed.clickable:active {
    background-color: hsla(2, 70%, 55%, 0.14);
}
.rowCoverWrap {
    position: relative;
    width: 48px;
    height: 48px;
    flex-shrink: 0;
}
.rowCover {
    width: 100%;
    height: 100%;
    border-radius: 6px;
    overflow: hidden;
    background-color: #202020;
    transition: opacity var(--duration-hover) var(--ease-smooth);
}
.dlRow.playable:hover .rowCover,
.dlRow.playing .rowCover {
    opacity: 0.45;
}
.playingDot {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 14px;
    height: 14px;
    margin: -7px 0 0 -7px;
    border-radius: 50%;
    background: #fff;
    animation: playingPulse 0.8s infinite var(--ease-smooth-in-out);
    pointer-events: none;
    z-index: 2;
    transition: opacity var(--duration-hover) var(--ease-smooth);
}
.dlRow.playable:hover .playingDot {
    opacity: 0;
}
@keyframes playingPulse {
    0%, 100% { transform: scale(0.6); }
    50% { transform: scale(1); }
}
.coverPlayBtn {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 24px;
    height: 24px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: #242424;
    background-color: hsla(0, 0%, 100%, 0.96);
    opacity: 0;
    z-index: 3;
    transition: opacity var(--duration-hover) var(--ease-smooth), transform 80ms var(--ease-smooth);
}
.coverPlayBtn:active {
    transform: translate(-50%, -50%) scale(0.92);
}
.dlRow.playable:hover .coverPlayBtn,
.dlRow.playing.paused .coverPlayBtn {
    opacity: 1;
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
.rowMetaText {
    display: block;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: hsla(0, 0%, 100%, 0.45);
}
.metaLink + .metaAlbum::before,
.metaLink + .metaErr::before {
    content: "\00B7";
    margin: 0 5px;
    opacity: 0.6;
}
.metaLink.clickable { cursor: pointer; transition: color var(--duration-hover) var(--ease-smooth); }
.metaLink.clickable:hover { color: hsla(0, 0%, 100%, 0.9); }
.metaErr {
    color: hsla(2, 80%, 74%, 0.95);
}

.rowProgress {
    width: 196px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    user-select: none;
}
.progHead {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
}
.progDim {
    color: hsla(0, 0%, 100%, 0.45);
    white-space: nowrap;
}
.progPct {
    margin-left: auto;
    font-weight: 600;
    color: hsl(var(--accent-h), min(var(--accent-s), 70%), 72%);
}
.progStage {
    min-width: 0;
    color: hsl(var(--accent-h), min(var(--accent-s), 70%), 72%);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.progTrack {
    position: relative;
    height: 5px;
    border-radius: 999px;
    overflow: hidden;
    background: hsla(0, 0%, 100%, 0.07);
}
.progFill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 999px;
    background: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
    transition: width .28s var(--ease-smooth);
}
.rowQueued {
    flex-shrink: 0;
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.38);
    user-select: none;
}

.rowTime {
    width: 44px;
    flex-shrink: 0;
    text-align: right;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.45);
    user-select: none;
}

.rowActions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
}
.actionBtn {
    position: relative;
    flex-shrink: 0;
    width: 36px;
    height: 36px;
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
    color: hsla(2, 80%, 74%, 1);
}
.actionBtn.error:hover:not(:disabled) { background-color: hsla(2, 70%, 55%, 0.16); color: hsla(2, 80%, 82%, 1); }

.dismissBtn {
    flex-shrink: 0;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.45);
    background-color: transparent;
    opacity: 0;
    transition: opacity var(--duration-hover) var(--ease-smooth), background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth), transform 80ms var(--ease-smooth);
}
.dismissBtn.visible {
    opacity: 1;
    color: hsla(0, 0%, 100%, 0.55);
}
.dlRow:hover .dismissBtn {
    opacity: 1;
}
.dismissBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: #fff;
}
.dismissBtn:active {
    transform: scale(0.92);
}
@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .dlRow.running {
        display: grid;
        grid-template-columns: auto minmax(0, 1fr) auto auto;
        align-content: center;
        gap: 4px 10px;
    }
    .running .rowCoverWrap,
    .running .rowIndex { grid-area: 1 / 1 / 3 / 2; }
    .running .rowInfo { grid-area: 1 / 2; }
    .running .rowProgress { grid-area: 2 / 2; width: auto; gap: 3px; }
    .running .rowTime { grid-area: 1 / 3 / 3 / 4; }
    .running .rowActions { grid-area: 1 / 4 / 3 / 5; }
}
</style>
