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
import { timesCountText } from '../utils/format'
import CloseIcon from './Icons/CloseIcon.vue'

const router = useRouter()
const { isOpen, currentTrack, closeTrackInfo } = useTrackInfoModal()
const { albums, loadPlaylists } = usePlaylists()
const listenCount = ref(0)
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
    if (secs == null) return '—'
    const h = Math.floor(secs / 3600)
    const m = Math.floor((secs % 3600) / 60)
    const s = Math.floor(secs % 60)
    if (h > 0) return `${h}:${m.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`
    return `${m}:${s.toString().padStart(2, '0')}`
}

function formatSampleRate(hz: number | null): string {
    if (hz == null) return '—'
    return `${(hz / 1000).toFixed(1)} кГц`
}

function formatBitrate(br: number | null): string {
    if (br == null) return '—'
    return `${Math.round(br / 1000)} кбит/с`
}

function formatChannels(ch: number | null): string {
    if (ch == null) return '—'
    if (ch === 1) return 'Моно'
    if (ch === 2) return 'Стерео'
    return `${ch} каналов`
}

const generalFields = computed(() => {
    const t = currentTrack.value
    if (!t) return []
    return [
        { label: 'Жанр', value: t.genre || '—' },
        { label: 'Длительность', value: formatDuration(t.duration_secs) },
        { label: 'Прослушано', value: timesCountText(listenCount.value) },
    ]
})

const audioFields = computed(() => {
    const t = currentTrack.value
    if (!t) return []
    return [
        { label: 'Частота', value: formatSampleRate(t.sample_rate) },
        { label: 'Разрядность', value: t.bit_depth ? `${t.bit_depth} бит` : '—' },
        { label: 'Битрейт', value: formatBitrate(t.bitrate) },
        { label: 'Каналы', value: formatChannels(t.channels) },
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
        // ничего
    }
}

function onBackdropMousedown(e: MouseEvent) {
    if (e.target === e.currentTarget) closeTrackInfo()
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
                <div class="modalCard" @mousedown.stop>
                    <div class="accentGlow"></div>

                    <button class="closeBtn" @click="closeTrackInfo" aria-label="Закрыть">
                        <CloseIcon />
                    </button>

                    <div class="modalHeader">
                        <div class="coverWrap">
                            <div class="coverHalo"></div>
                            <div class="cover">
                                <img v-if="currentTrack.cover_art" :src="currentTrack.cover_art" alt="" />
                                <div v-else class="coverPlaceholder"></div>
                            </div>
                        </div>
                        <div class="headerText">
                            <div class="overline">О треке</div>
                            <div class="trackTitle">{{ currentTrack.title || fileName }}</div>
                            <div class="trackArtist">{{ currentTrack.artist || 'Неизвестный исполнитель' }}</div>
                        <div class="badges">
                            <span class="chip chipFormat" v-tooltip:bottom="'Аудио формат файла — определяет тип сжатия и совместимость'">{{ (currentTrack.format || '—').toUpperCase() }}</span>
                            <span class="chip" :class="currentTrack.is_lossless ? 'chipLossless' : 'chipLossy'" v-tooltip:bottom="currentTrack.is_lossless ? 'Lossless — сжатие без потери качества (FLAC, ALAC, WAV)' : 'Lossy — меньший размер файла с некоторой потерей качества (MP3, AAC)'">
                                {{ currentTrack.is_lossless ? 'Lossless' : 'Lossy' }}
                            </span>
                        </div>
                        </div>
                    </div>

                    <div class="sections">
                        <section class="section">
                            <div class="sectionTitle">Описание</div>
                            <div class="fieldsList">
                                <div v-if="hasAlbum" class="fieldRow">
                                    <span class="fieldLabel">Альбом</span>
                                    <span class="fieldValue albumValue">{{ trackAlbumName }}</span>
                                    <button class="openBtn" @click="openAlbum">Открыть</button>
                                </div>
                                <div v-for="f in generalFields" :key="f.label" class="fieldRow">
                                    <span class="fieldLabel">{{ f.label }}</span>
                                    <span class="fieldValue">{{ f.value }}</span>
                                </div>
                            </div>
                        </section>

                        <section class="section">
                            <div class="sectionTitle">Звук</div>
                            <div class="fieldsList">
                                <div v-for="f in audioFields" :key="f.label" class="fieldRow">
                                    <span class="fieldLabel">{{ f.label }}</span>
                                    <span class="fieldValue">{{ f.value }}</span>
                                </div>
                            </div>
                        </section>

                        <section class="section">
                            <div class="sectionTitle">Файл</div>
                                <div class="pathBlock" @click="copyPath">
                                <div class="pathText">
                                    <div class="pathName">{{ fileName }}</div>
                                    <div v-if="fileFolder" class="pathFolder">{{ fileFolder }}</div>
                                </div>
                                <span class="copyHint">{{ copiedPath ? 'Скопировано' : 'Копировать' }}</span>
                            </div>
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
    width: calc(100vw / var(--ui-scale, 1));
    height: calc(100vh / var(--ui-scale, 1));
    z-index: 1000001;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsla(0, 0%, 0%, 0.35);
    backdrop-filter: blur(6px);
}
.modalCard {
    position: relative;
    width: 480px;
    max-width: calc(100vw - 40px);
    padding: 22px 22px 22px;
    border-radius: 28px;
    background-color: var(--light-bg);
    box-shadow:
        0 24px 60px rgba(0, 0, 0, 0.25),
        0 0 0 1px hsla(0, 0%, 100%, 0.04) inset;
    display: flex;
    flex-direction: column;
    gap: 22px;
    overflow: hidden;
}
.accentGlow {
    pointer-events: none;
    position: absolute;
    top: -60%;
    left: -20%;
    right: -20%;
    height: 220px;
    background: radial-gradient(
        ellipse at center,
        hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0.28) 0%,
        hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0) 70%
    );
    filter: blur(8px);
    z-index: 0;
}
.closeBtn {
    position: absolute;
    top: 16px;
    right: 16px;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: none;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: .24s ease;
    z-index: 2;
}
.closeBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.14);
    color: #fff;
}
.modalHeader {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: row;
    gap: 18px;
    align-items: center;
}
.coverWrap {
    position: relative;
    flex-shrink: 0;
    width: 96px;
    height: 96px;
}
.coverHalo {
    position: absolute;
    inset: -10px;
    border-radius: 22px;
    background: hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0.45);
    filter: blur(20px);
    opacity: 0.7;
    z-index: 0;
}
.cover {
    position: relative;
    width: 100%;
    height: 100%;
    border-radius: 16px;
    overflow: hidden;
    background-color: hsla(0, 0%, 100%, 0.06);
    box-shadow:
        0 8px 24px rgba(0, 0, 0, 0.35),
        0 0 0 1px hsla(0, 0%, 100%, 0.06) inset;
    z-index: 1;
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
    background: linear-gradient(135deg, hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0.45), hsla(0, 0%, 100%, 0.04));
}
.headerText {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    flex: 1;
}
.overline {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 1.2px;
    text-transform: uppercase;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.7), 75%, 0.85);
    margin-bottom: 2px;
    user-select: none;
}
.trackTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 700;
    font-size: 19px;
    line-height: 1.2;
    color: hsla(0, 0%, 100%, 0.96);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}
.trackArtist {
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.72);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}
.badges {
    display: flex;
    flex-direction: row;
    gap: 6px;
    margin-top: 8px;
    flex-wrap: wrap;
}
.chip {
    display: inline-flex;
    align-items: center;
    padding: 3px 9px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.6px;
    text-transform: uppercase;
    user-select: none;
    line-height: 1.4;
}
.chipFormat {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: hsla(0, 0%, 100%, 0.85);
}
.chipLossless {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.18);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 82%, 1);
    box-shadow: 0 0 0 1px hsla(var(--accent-h), var(--accent-s), 60%, 0.25) inset;
}
.chipLossy {
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.55);
}
.sections {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    gap: 18px;
    max-height: 390px;
    overflow-y: auto;
    padding-right: 4px;
    margin-right: -4px;
}
.sections::-webkit-scrollbar {
    width: 4px;
}
.sections::-webkit-scrollbar-track {
    background: transparent;
}
.sections::-webkit-scrollbar-thumb {
    background: hsla(0, 0%, 100%, 0.1);
    border-radius: 2px;
}
.section {
    display: flex;
    flex-direction: column;
    gap: 6px;
}
.sectionTitle {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 1.2px;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.4);
    user-select: none;
    padding-left: 2px;
}
.fieldsList {
    display: flex;
    flex-direction: column;
    border-radius: 14px;
    background-color: hsla(0, 0%, 100%, 0.03);
    overflow: hidden;
}
.fieldRow {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 12px;
    padding: 9px 14px;
    border-bottom: 1px solid hsla(0, 0%, 100%, 0.04);
    transition: background-color .18s ease;
}
.fieldRow:last-child {
    border-bottom: none;
}
.fieldRow:hover {
    background-color: hsla(0, 0%, 100%, 0.025);
}
.fieldLabel {
    flex-shrink: 0;
    width: 110px;
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.5);
    user-select: none;
}
.fieldValue {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.88);
    text-align: right;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: text;
}
.albumValue {
    text-align: right;
}
.openBtn {
    flex-shrink: 0;
    border: none;
    padding: 4px 10px;
    border-radius: 999px;
    font-family: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.6px;
    text-transform: uppercase;
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.16);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 85%, 1);
    cursor: pointer;
    transition: .24s ease;
    user-select: none;
}
.openBtn:hover {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.28);
    color: #fff;
}
.pathBlock {
    position: relative;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border-radius: 14px;
    background-color: hsla(0, 0%, 100%, 0.03);
    cursor: pointer;
    transition: background-color .18s ease;
    overflow: hidden;
}
.pathBlock:hover {
    background-color: hsla(0, 0%, 100%, 0.06);
}
.pathText {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
}
.pathName {
    font-family: "JetBrains Mono", "Fira Code", Consolas, monospace;
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.85);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.pathFolder {
    font-family: "JetBrains Mono", "Fira Code", Consolas, monospace;
    font-size: 11px;
    color: hsla(0, 0%, 100%, 0.45);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
}
.copyHint {
    flex-shrink: 0;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.8px;
    text-transform: uppercase;
    padding: 4px 9px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.55);
    transition: .24s ease;
    user-select: none;
}
.pathBlock:hover .copyHint {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.2);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 85%, 1);
}

.modal-enter-active, .modal-leave-active {
    transition: opacity .22s ease;
}
.modal-enter-active .modalCard, .modal-leave-active .modalCard {
    transition: transform .26s cubic-bezier(0.22, 1.2, 0.36, 1), opacity .22s ease;
}
.modal-enter-from, .modal-leave-to {
    opacity: 0;
}
.modal-enter-from .modalCard, .modal-leave-to .modalCard {
    transform: scale(0.94) translateY(8px);
    opacity: 0;
}
</style>
