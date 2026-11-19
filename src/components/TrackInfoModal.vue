<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { useTrackInfoModal } from '../composables/useTrackInfoModal'
import { usePlaylists } from '../composables/usePlaylists'
import {
    TRACK_STATISTICS_UPDATED_EVENT,
    getTrackStatistics,
    type TrackStatisticsUpdateEventDetail,
} from '../composables/useStatistics'
import CloseIcon from './Icons/CloseIcon.vue'
import EmptyIcon from './Icons/EmptyIcon.vue'
import { useSheetModal } from '../composables/useSheetModal'

const router = useRouter()
const { isOpen, currentTrack, closeTrackInfo } = useTrackInfoModal()
const { albums, loadPlaylists } = usePlaylists()
const listenCount = ref(0)
const modalCardEl = ref<HTMLElement | null>(null)
const { onBackdropMousedown, onSheetDragStart } = useSheetModal(modalCardEl, closeTrackInfo)
let listenCountRequestId = 0

const trackAlbumName = computed(() => (currentTrack.value?.album || '').trim())
const hasAlbum = computed(() => {
    const name = trackAlbumName.value
    if (!name) return false
    return albums.value.some(a => a.name === name)
})

async function refreshListenCount() {
    const filePath = currentTrack.value?.file_path
    const requestId = ++listenCountRequestId
    if (!filePath) {
        listenCount.value = 0
        return
    }

    try {
        const stats = await getTrackStatistics(filePath)
        if (requestId === listenCountRequestId) listenCount.value = stats.listen_count
    } catch (e) {
        console.error('Ошибка чтения статистики прослушиваний трека:', e)
    }
}

function onTrackStatisticsUpdated(event: Event) {
    const detail = (event as CustomEvent<TrackStatisticsUpdateEventDetail>).detail
    if (!detail?.listenCountDelta) return
    if (detail.filePath !== currentTrack.value?.file_path) return
    void refreshListenCount()
}

watch(isOpen, (open) => {
    if (open && !albums.value.length) loadPlaylists()
    if (open) void refreshListenCount()
})

watch(() => currentTrack.value?.file_path, () => {
    if (isOpen.value) void refreshListenCount()
})

function openAlbum() {
    if (!hasAlbum.value) return
    router.push(`/album/${encodeURIComponent(trackAlbumName.value)}`)
    closeTrackInfo()
}

const fileName = computed(() => {
    const p = currentTrack.value?.file_path || ''
    return p.split(/[\\/]/).pop() || p
})

const fileFolder = computed(() => {
    const p = currentTrack.value?.file_path || ''
    const parts = p.split(/[\\/]/)
    parts.pop()
    return parts.join('\\')
})

function formatDuration(secs: number | null): string {
    if (secs == null) return '-'
    const h = Math.floor(secs / 3600)
    const m = Math.floor((secs % 3600) / 60)
    const s = Math.floor(secs % 60)
    if (h > 0) return `${h}:${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`
    return `${m}:${s.toString().padStart(2, '0')}`
}

function formatSampleRate(hz: number | null): string {
    if (hz == null) return '-'
    return `${(hz / 1000).toFixed(1)} кГц`
}

function formatChannels(ch: number | null): string {
    if (ch == null) return '-'
    if (ch === 1) return 'Моно'
    if (ch === 2) return 'Стерео'
    return `${ch} каналов`
}

const statTiles = computed(() => {
    const t = currentTrack.value
    if (!t) return []
    return [
        { label: 'Длительность', value: formatDuration(t.duration_secs) },
        { label: 'Прослушано', value: String(listenCount.value) },
        { label: 'Битрейт', value: t.bitrate ? `${Math.round(t.bitrate / 1000)}` : '-', unit: t.bitrate ? 'кбит/с' : '' },
    ]
})

const soundRows = computed(() => {
    const t = currentTrack.value
    if (!t) return []
    const rows = [
        { label: 'Частота', value: formatSampleRate(t.sample_rate) },
        { label: 'Разрядность', value: t.bit_depth ? `${t.bit_depth} бит` : '-' },
        { label: 'Каналы', value: formatChannels(t.channels) },
    ]
    if (t.bpm) rows.push({ label: 'Темп', value: `${Math.round(t.bpm)} BPM` })
    if (t.key) rows.push({ label: 'Тональность', value: t.key })
    return rows
})

function pct(v: number | null | undefined): number {
    return Math.round(Math.min(1, Math.max(0, v ?? 0)) * 100)
}

/** Analysed audio features as thin bars; empty until the track was analysed. */
const featureBars = computed(() => {
    const t = currentTrack.value
    if (!t || t.energy == null) return []
    return [
        { label: 'Энергичность', value: pct(t.energy) },
        { label: 'Танцевальность', value: pct(t.danceability) },
        { label: 'Яркость', value: pct((t.spectral_centroid ?? 0) / 4000) },
    ]
})

const copiedPath = ref(false)
let copyTimer: ReturnType<typeof setTimeout> | null = null

async function copyPath() {
    if (!currentTrack.value?.file_path) return
    try {
        await navigator.clipboard.writeText(currentTrack.value.file_path)
        copiedPath.value = true
        if (copyTimer) clearTimeout(copyTimer)
        copyTimer = setTimeout(() => { copiedPath.value = false }, 1400)
    } catch {
    }
}

function onKeydown(e: KeyboardEvent) {
    if (!isOpen.value) return
    if (e.key === 'Escape') {
        e.preventDefault()
        closeTrackInfo()
    }
}

onMounted(() => {
    document.addEventListener('keydown', onKeydown)
    window.addEventListener(TRACK_STATISTICS_UPDATED_EVENT, onTrackStatisticsUpdated)
})

onUnmounted(() => {
    document.removeEventListener('keydown', onKeydown)
    window.removeEventListener(TRACK_STATISTICS_UPDATED_EVENT, onTrackStatisticsUpdated)
    if (copyTimer) clearTimeout(copyTimer)
})
</script>

<template>
    <Teleport to="body">
        <Transition name="modal">
            <div
                v-if="isOpen && currentTrack"
                class="modalBackdrop"
                @mousedown="onBackdropMousedown"
            >
                <div ref="modalCardEl" class="modalCard" @mousedown.stop>
                    <div class="sheetHandle" @pointerdown="onSheetDragStart"></div>

                    <button class="closeBtn" @click="closeTrackInfo" aria-label="Закрыть">
                        <CloseIcon />
                    </button>

                    <div class="modalHeader">
                        <div class="cover">
                            <img v-if="currentTrack.cover_art" :src="currentTrack.cover_art" alt="" />
                            <div v-else class="coverPlaceholder"><EmptyIcon /></div>
                        </div>
                        <div class="headerText">
                            <div class="trackTitle">{{ currentTrack.title || fileName }}</div>
                            <div class="trackArtist">{{ currentTrack.artist || 'Неизвестный исполнитель' }}</div>
                            <button v-if="hasAlbum" class="albumLink" @click="openAlbum">
                                <span class="albumName">{{ trackAlbumName }}</span>
                                <svg class="albumChevron" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" /></svg>
                            </button>
                            <div v-else-if="trackAlbumName" class="albumText">{{ trackAlbumName }}</div>
                            <div class="badges">
                                <span class="chip">{{ (currentTrack.format || '-').toUpperCase() }}</span>
                                <span class="chip" :class="{ chipAccent: currentTrack.is_lossless }">{{ currentTrack.is_lossless ? 'Lossless' : 'Lossy' }}</span>
                                <span v-if="currentTrack.genre" class="chip chipMuted">{{ currentTrack.genre }}</span>
                            </div>
                        </div>
                    </div>

                    <div class="body">
                        <div class="statTiles">
                            <div v-for="s in statTiles" :key="s.label" class="statTile">
                                <div class="statValue">{{ s.value }}<span v-if="s.unit" class="statUnit">{{ s.unit }}</span></div>
                                <div class="statLabel">{{ s.label }}</div>
                            </div>
                        </div>

                        <section class="section">
                            <div class="sectionTitle">Звук</div>
                            <div class="rows">
                                <div v-for="r in soundRows" :key="r.label" class="row">
                                    <span class="rowLabel">{{ r.label }}</span>
                                    <span class="rowValue">{{ r.value }}</span>
                                </div>
                            </div>
                            <div v-if="featureBars.length" class="bars">
                                <div v-for="b in featureBars" :key="b.label" class="bar">
                                    <div class="barHead">
                                        <span class="rowLabel">{{ b.label }}</span>
                                        <span class="barValue">{{ b.value }}%</span>
                                    </div>
                                    <div class="barTrack"><div class="barFill" :style="{ width: b.value + '%' }"></div></div>
                                </div>
                            </div>
                        </section>

                        <section class="section">
                            <div class="sectionTitle">Файл</div>
                            <button class="pathTile" @click="copyPath">
                                <span class="pathText">
                                    <span class="pathName">{{ fileName }}</span>
                                    <span v-if="fileFolder" class="pathFolder">{{ fileFolder }}</span>
                                </span>
                                <span class="copyHint" :class="{ copied: copiedPath }">{{ copiedPath ? 'Скопировано' : 'Копировать' }}</span>
                            </button>
                        </section>
                    </div>
                </div>
            </div>
        </Transition>
    </Teleport>
</template>

<style scoped>
.modalBackdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: calc(100vw / var(--overlay-scale, 1));
    height: calc(100vh / var(--overlay-scale, 1));
    z-index: 1000001;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsla(0, 0%, 0%, 0.4);
    backdrop-filter: blur(6px);
}
.modalCard {
    --tile: hsl(var(--accent-h), min(var(--accent-s), 45%), 19%);
    position: relative;
    width: 460px;
    max-width: calc(100vw - 40px);
    padding: 24px;
    border-radius: 28px;
    background-color: var(--light-bg);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.32);
    display: flex;
    flex-direction: column;
    gap: 22px;
    overflow: hidden;
}
.sheetHandle {
    display: none;
}
.closeBtn {
    position: absolute;
    top: 16px;
    right: 16px;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: none;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.55);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: var(--duration-hover) var(--ease-smooth);
    z-index: 2;
}
.closeBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: #fff;
}

.modalHeader {
    display: flex;
    flex-direction: row;
    gap: 18px;
    align-items: center;
    padding-right: 28px;
}
.cover {
    flex-shrink: 0;
    width: 116px;
    height: 116px;
    border-radius: 16px;
    overflow: hidden;
    background-color: var(--tile);
    box-shadow: 0 12px 28px rgba(0, 0, 0, 0.35);
}
.cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
}
.coverPlaceholder {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: hsla(0, 0%, 100%, 0.35);
}
.coverPlaceholder svg {
    width: 34px;
    height: 34px;
}
.headerText {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
}
.trackTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 21px;
    line-height: 1.2;
    color: #fff;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
}
.trackArtist,
.albumText {
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.7);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.albumText {
    font-size: 13px;
    color: hsla(0, 0%, 100%, 0.45);
}
.albumLink {
    align-self: flex-start;
    max-width: 100%;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 0;
    border: none;
    background: none;
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.45);
    cursor: pointer;
    transition: color var(--duration-hover) var(--ease-smooth);
}
.albumLink:hover {
    color: #fff;
}
.albumName {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.albumChevron {
    flex-shrink: 0;
    transition: transform var(--duration-hover) var(--ease-smooth);
}
.albumLink:hover .albumChevron {
    transform: translateX(2px);
}
.badges {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
}
.chip {
    display: inline-flex;
    align-items: center;
    height: 22px;
    padding: 0 9px;
    border-radius: 999px;
    background-color: var(--tile);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.5px;
    color: hsla(0, 0%, 100%, 0.8);
    white-space: nowrap;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}
.chipAccent {
    color: hsl(var(--accent-h), max(var(--accent-s), 55%), 72%);
}
.chipMuted {
    font-weight: 600;
    letter-spacing: 0;
    color: hsla(0, 0%, 100%, 0.6);
}

.body {
    display: flex;
    flex-direction: column;
    gap: 22px;
    max-height: min(540px, calc(100vh - 240px));
    overflow-y: auto;
    margin-right: -10px;
    padding-right: 10px;
}
.body::-webkit-scrollbar {
    width: 4px;
}
.body::-webkit-scrollbar-thumb {
    background: hsla(0, 0%, 100%, 0.1);
    border-radius: 2px;
}

.statTiles {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
}
.statTile {
    min-width: 0;
    padding: 12px 14px;
    border-radius: 16px;
    background-color: var(--tile);
    display: flex;
    flex-direction: column;
    gap: 4px;
}
.statValue {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 19px;
    font-weight: 800;
    color: #fff;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.statUnit {
    margin-left: 4px;
    font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 11px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.45);
}
.statLabel {
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.5);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}
.sectionTitle {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.8px;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.42);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}

.section {
    display: flex;
    flex-direction: column;
    gap: 10px;
}
.rows {
    display: flex;
    flex-direction: column;
    gap: 9px;
}
.row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
}
.rowLabel {
    flex-shrink: 0;
    font-size: 13px;
    color: hsla(0, 0%, 100%, 0.5);
    user-select: none;
}
.rowValue {
    min-width: 0;
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.9);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.bars {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 6px;
}
.barHead {
    display: flex;
    justify-content: space-between;
    margin-bottom: 6px;
}
.barValue {
    font-size: 12px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.75);
    font-variant-numeric: tabular-nums;
}
.barTrack {
    height: 6px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    overflow: hidden;
}
.barFill {
    height: 100%;
    border-radius: inherit;
    background-color: hsl(var(--accent-h), max(var(--accent-s), 55%), 62%);
}

.pathTile {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 12px 14px;
    border: none;
    border-radius: 16px;
    background-color: var(--tile);
    font-family: inherit;
    text-align: left;
    color: inherit;
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.pathTile:hover {
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 23%);
}
.pathText {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
}
.pathName {
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.pathFolder {
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.42);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
}
.copyHint {
    flex-shrink: 0;
    font-size: 12px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.45);
    transition: color var(--duration-hover) var(--ease-smooth);
}
.pathTile:hover .copyHint {
    color: #fff;
}
.copyHint.copied {
    color: hsl(var(--accent-h), max(var(--accent-s), 55%), 72%);
}

.modal-enter-active, .modal-leave-active {
    transition: opacity var(--duration-modal) var(--ease-smooth);
}
.modal-enter-active .modalCard, .modal-leave-active .modalCard {
    transition: transform var(--duration-modal) var(--ease-smooth), opacity var(--duration-modal) var(--ease-smooth);
}
.modal-enter-from, .modal-leave-to {
    opacity: 0;
}
.modal-enter-from .modalCard, .modal-leave-to .modalCard {
    transform: translateY(8px) scale(.98);
    opacity: 0;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .modalBackdrop {
        align-items: flex-end !important;
        justify-content: center !important;
        padding: calc(var(--safe-top) + 12px) 0 0 !important;
    }

    .modalCard {
        width: 100vw !important;
        max-width: none !important;
        max-height: calc(100dvh - var(--safe-top) - 12px) !important;
        margin-left: 0 !important;
        margin-right: 0 !important;
        padding: 0 18px calc(20px + var(--safe-bottom));
        gap: 20px;
        border-radius: 28px 28px 0 0 !important;
        overflow-y: auto !important;
        overscroll-behavior: contain;
        touch-action: pan-y;
        will-change: transform;
    }

    .sheetHandle {
        display: flex;
        flex: 0 0 auto;
        height: 30px;
        margin: 0 -18px -8px;
        align-items: center;
        justify-content: center;
        cursor: grab;
        touch-action: none;
    }

    .sheetHandle::before {
        content: "";
        width: 42px;
        height: 4px;
        border-radius: 999px;
        background-color: hsla(0, 0%, 100%, 0.28);
    }

    .closeBtn {
        display: none;
    }

    .modalHeader {
        flex-direction: column;
        align-items: center;
        padding-right: 0;
        gap: 14px;
    }

    .cover {
        align-self: center;
        width: min(46vw, 168px);
        height: min(46vw, 168px);
        border-radius: 18px;
    }

    .headerText {
        align-items: center;
        text-align: center;
    }

    .albumLink {
        align-self: center;
    }

    .badges {
        justify-content: center;
    }

    .statTile {
        padding: 12px;
    }

    .body {
        max-height: none;
        overflow: visible;
        margin-right: 0;
        padding-right: 0;
    }

    .copyHint {
        display: none;
    }

    .modal-enter-active .modalCard {
        transition: transform var(--duration-modal) var(--ease-smooth);
    }

    .modal-leave-active,
    .modal-leave-active .modalCard {
        transition: transform .5s var(--ease-sheet), opacity .5s var(--ease-sheet);
    }

    .modal-enter-from .modalCard,
    .modal-leave-to .modalCard {
        transform: translateY(100%);
        opacity: 1;
    }

    :global(body.lfmusic-phone) .modalBackdrop {
        padding: calc(var(--safe-top) + 12px) 0 0 !important;
    }

    :global(body.lfmusic-phone) .modalCard {
        width: 100vw !important;
        max-width: none !important;
        margin-left: 0 !important;
        margin-right: 0 !important;
        border-radius: 28px 28px 0 0 !important;
    }
}
</style>
