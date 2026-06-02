<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onBeforeUnmount } from 'vue'
import { useRouter } from 'vue-router'
import ContextMenu from './ContextMenu.vue'
import AnimatedCover from './AnimatedCover.vue'
import ArrowUpIcon from './Icons/ArrowUpIcon.vue'
import PlaylistIcon from './Icons/PlaylistIcon.vue'
import PlusIcon from './Icons/PlusIcon.vue'
import TuneIcon from './Icons/TuneIcon.vue'
import AlbumIcon from './Icons/AlbumIcon.vue'
import InfoIcon from './Icons/InfoIcon.vue'
import DeleteIcon from './Icons/DeleteIcon.vue'
import defaultPlaylistCover from '../assets/playlistCover.webp'
import type { TrackMetadata } from '../composables/usePlayer'
import { usePlayer } from '../composables/usePlayer'
import { usePlaylists } from '../composables/usePlaylists'
import { useCustomCovers } from '../composables/useCustomCovers'
import { isPhoneViewport } from '../composables/usePlatform'
import { useTrackInfoModal } from '../composables/useTrackInfoModal'
import { useTrackEditModal } from '../composables/useTrackEditModal'

const props = defineProps<{
    x: number
    y: number
    track: TrackMetadata
    alignRight?: boolean
    anchorBottom?: boolean
    centerX?: boolean
    centerY?: boolean
    submenuRight?: boolean
    inPlaylist?: boolean
}>()

const emit = defineEmits<{
    close: []
    moveToTop: [track: TrackMetadata]
    removeFromPlaylist: [track: TrackMetadata]
}>()

const router = useRouter()
const { userPlaylists, albums, loadPlaylists, createPlaylist, addTrackToPlaylist, getPlaylistIdsForTrack } = usePlaylists()
const { getTrackByPath, loadCoversForTracks } = usePlayer()
const { getCustomCover } = useCustomCovers()
const { openTrackInfo } = useTrackInfoModal()
const { openTrackEdit } = useTrackEditModal()

// "Перейти к альбому" имеет смысл только если у трека есть альбом и в нём
// больше одного трека. Бэкенд создаёт album-плейлист только при count > 1,
// поэтому достаточно проверить наличие соответствующей записи.
const trackAlbumName = computed(() => (props.track.album || '').trim())
const hasAlbum = computed(() => {
    const name = trackAlbumName.value
    if (!name) return false
    return albums.value.some(a => a.name === name)
})

const trackPlaylistIds = ref<Set<number>>(new Set())

onMounted(async () => {
    if (!userPlaylists.value.length) await loadPlaylists()
    const ids = await getPlaylistIdsForTrack(props.track.file_path)
    trackPlaylistIds.value = new Set(ids)
    const coverTracks = userPlaylists.value
        .map(pl => pl.first_file_path ? getTrackByPath(pl.first_file_path) : undefined)
        .filter((track): track is TrackMetadata => !!track && !track.cover_art)
    if (coverTracks.length) loadCoversForTracks(coverTracks)
    window.addEventListener('resize', onWindowResize)
})

const showPlaylistSub = ref(false)
const showPlaylistSheet = ref(false)
const subReady = ref(false)
const subTriggerEl = ref<HTMLElement | null>(null)
const subMenuEl = ref<HTMLElement | null>(null)
const playlistSheetEl = ref<HTMLElement | null>(null)
const subStyle = ref<Record<string, string>>({})
const pinnedSub = ref(false)
let playlistSheetDragStartY = 0
let playlistSheetDragging = false

function positionSubmenu() {
    const trigger = subTriggerEl.value
    if (!trigger) return
    const scale = (window as any).__uiScale || 1
    const tr = trigger.getBoundingClientRect()
    const gap = 4

    let left: number
    if (props.submenuRight) {
        left = tr.right + gap
    } else {
        left = tr.left - gap
    }

    let top = tr.top - 6

    subStyle.value = {
        left: (left / scale) + 'px',
        top: (top / scale) + 'px',
    }
}

async function clampSubmenu() {
    await nextTick()
    const el = subMenuEl.value
    if (!el) return
    const scale = (window as any).__uiScale || 1
    const rect = el.getBoundingClientRect()
    const pad = 8
    const trigger = subTriggerEl.value
    if (!trigger) return
    const tr = trigger.getBoundingClientRect()

    let left = parseFloat(subStyle.value.left) * scale
    let top = parseFloat(subStyle.value.top) * scale

    if (!props.submenuRight) {
        if (left - rect.width < pad) {
            left = tr.right + 4
        } else {
            left = left - rect.width
        }
    }

    if (left + rect.width > window.innerWidth - pad) {
        left = window.innerWidth - rect.width - pad
    }
    if (left < pad) left = pad

    if (top + rect.height > window.innerHeight - pad) {
        top = window.innerHeight - rect.height - pad
    }
    if (top < pad) top = pad

    subStyle.value = {
        left: (left / scale) + 'px',
        top: (top / scale) + 'px',
    }
    subReady.value = true
}

let leaveTimer: ReturnType<typeof setTimeout> | null = null

function cancelLeave() {
    if (leaveTimer !== null) {
        clearTimeout(leaveTimer)
        leaveTimer = null
    }
}

function openSub() {
    if (isPhoneViewport()) {
        showPlaylistSheet.value = true
        return
    }
    cancelLeave()
    subReady.value = false
    showPlaylistSub.value = true
    positionSubmenu()
    clampSubmenu()
}

function closeSub() {
    cancelLeave()
    subReady.value = false
    showPlaylistSub.value = false
    pinnedSub.value = false
}

function scheduleClose() {
    if (isPhoneViewport()) return
    if (pinnedSub.value) return
    cancelLeave()
    leaveTimer = setTimeout(() => {
        leaveTimer = null
        subReady.value = false
        showPlaylistSub.value = false
    }, 120)
}

function onSubClick() {
    if (isPhoneViewport()) {
        showPlaylistSheet.value = true
        return
    }
    if (pinnedSub.value) {
        closeSub()
    } else {
        openSub()
        pinnedSub.value = true
    }
}

function onSubEnter() {
    if (isPhoneViewport()) return
    cancelLeave()
    if (!pinnedSub.value) {
        openSub()
    }
}

function onSubLeave() {
    if (isPhoneViewport()) return
    scheduleClose()
}

function onSubMenuLeave() {
    if (isPhoneViewport()) return
    scheduleClose()
}

function onWindowResize() {
    if (showPlaylistSub.value) {
        positionSubmenu()
        clampSubmenu()
    }
}

onBeforeUnmount(() => {
    cancelLeave()
    window.removeEventListener('resize', onWindowResize)
    window.removeEventListener('pointermove', onPlaylistSheetDragMove)
    window.removeEventListener('pointerup', onPlaylistSheetDragEnd)
    window.removeEventListener('pointercancel', onPlaylistSheetDragEnd)
})

function onEdit() {
    openTrackEdit(props.track)
    emit('close')
}

function onMoveToTop() {
    emit('moveToTop', props.track)
    emit('close')
}

async function onAddToPlaylist(playlistId: number) {
    if (trackPlaylistIds.value.has(playlistId)) return
    await addTrackToPlaylist(playlistId, props.track.file_path)
    trackPlaylistIds.value.add(playlistId)
    emit('close')
}

function nextPlaylistName(): string {
    const base = 'Новый плейлист'
    const re = new RegExp(`^${base}\\s+(\\d+)$`)
    let maxN = 0
    for (const pl of userPlaylists.value) {
        const m = pl.name.match(re)
        if (!m) continue
        const n = parseInt(m[1], 10)
        if (n > maxN) maxN = n
    }
    return `${base} ${maxN + 1}`
}

async function onCreatePlaylistAndAdd() {
    const playlist = await createPlaylist(nextPlaylistName())
    if (!playlist) return
    await onAddToPlaylist(playlist.id)
}

function playlistCover(pl: { id: number; first_file_path: string | null }) {
    const custom = getCustomCover('playlist', pl.id)
    if (custom) return custom
    if (!pl.first_file_path) return defaultPlaylistCover
    return getTrackByPath(pl.first_file_path)?.cover_art || defaultPlaylistCover
}

function onPlaylistSheetDragStart(e: PointerEvent) {
    e.preventDefault()
    playlistSheetDragging = true
    playlistSheetDragStartY = e.clientY
    if (playlistSheetEl.value) playlistSheetEl.value.style.transition = 'none'
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
    window.addEventListener('pointermove', onPlaylistSheetDragMove)
    window.addEventListener('pointerup', onPlaylistSheetDragEnd)
    window.addEventListener('pointercancel', onPlaylistSheetDragEnd)
}

function onPlaylistSheetDragMove(e: PointerEvent) {
    if (!playlistSheetDragging || !playlistSheetEl.value) return
    e.preventDefault()
    const delta = Math.max(0, e.clientY - playlistSheetDragStartY)
    playlistSheetEl.value.style.transform = `translateY(${delta}px)`
}

function onPlaylistSheetDragEnd(e: PointerEvent) {
    if (!playlistSheetDragging) return
    playlistSheetDragging = false
    window.removeEventListener('pointermove', onPlaylistSheetDragMove)
    window.removeEventListener('pointerup', onPlaylistSheetDragEnd)
    window.removeEventListener('pointercancel', onPlaylistSheetDragEnd)
    const delta = Math.max(0, e.clientY - playlistSheetDragStartY)
    if (delta > 64) {
        emit('close')
        return
    }
    if (playlistSheetEl.value) {
        playlistSheetEl.value.style.transition = ''
        playlistSheetEl.value.style.transform = ''
    }
}

function onRemoveFromPlaylist() {
    emit('removeFromPlaylist', props.track)
    emit('close')
}

function onGoToAlbum() {
    if (!hasAlbum.value) return
    router.push(`/album/${encodeURIComponent(trackAlbumName.value)}`)
    emit('close')
}

function onAbout() {
    openTrackInfo(props.track)
    emit('close')
}
</script>

<template>
    <ContextMenu
        v-if="!showPlaylistSheet"
        :x="x"
        :y="y"
        :alignRight="alignRight"
        :anchorBottom="anchorBottom"
        :centerX="centerX"
        :centerY="centerY"
        mobileSheet="content"
        @close="emit('close')"
    >
        <div class="trackSheetActions">
            <button class="ctx-item trackSheetAction" @click="onEdit">
                <TuneIcon width="16" height="16" />
                <span>Редактировать</span>
            </button>
            <button class="ctx-item trackSheetAction" @click="onMoveToTop">
                <ArrowUpIcon width="16" height="16" />
                <span>Поднять</span>
            </button>
        </div>
        <div class="ctx-separator"></div>
        <div
            ref="subTriggerEl"
            class="ctx-submenu-wrapper"
            @mouseenter="onSubEnter"
            @mouseleave="onSubLeave"
        >
            <button type="button" class="ctx-item ctx-submenu-trigger ctx-add-playlist-row" @click.stop="onSubClick">
                <PlaylistIcon width="16" height="16" />
                <span>Добавить в плейлист</span>
                <svg v-if="submenuRight" class="ctx-chevron" :class="{ open: showPlaylistSub }" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="m9 18 6-6-6-6"/>
                </svg>
                <svg v-else class="ctx-chevron" :class="{ open: showPlaylistSub }" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="m15 18-6-6 6-6"/>
                </svg>
            </button>
        </div>
        <button v-if="inPlaylist" class="ctx-item" @click="onRemoveFromPlaylist">
            <DeleteIcon width="16" height="16" />
            <span>Удалить из плейлиста</span>
        </button>
        <div class="ctx-separator"></div>
        <button v-if="hasAlbum" class="ctx-item" @click="onGoToAlbum">
            <AlbumIcon width="16" height="16" />
            <span>Перейти к альбому</span>
        </button>
        <button class="ctx-item" @click="onAbout">
            <InfoIcon width="16" height="16" />
            <span>О треке</span>
        </button>
    </ContextMenu>
    <Teleport to="body">
        <Transition name="sub">
            <div
                v-if="showPlaylistSub"
                ref="subMenuEl"
                class="ctx-submenu"
                :class="{ ready: subReady }"
                :style="subStyle"
                @mousedown.stop
                @mouseenter="onSubEnter"
                @mouseleave="onSubMenuLeave"
            >
                <div class="ctx-submenu-buffer"></div>
                <div class="ctx-submenu-list">
                    <button
                        class="ctx-item ctx-create-playlist-item"
                        @click.stop="onCreatePlaylistAndAdd"
                    >
                        <PlusIcon width="14" height="14" />
                        <span>Создать плейлист</span>
                    </button>
                    <button
                        v-for="pl in userPlaylists"
                        :key="pl.id"
                        class="ctx-item"
                        :disabled="trackPlaylistIds.has(pl.id)"
                        @click.stop="onAddToPlaylist(pl.id)"
                    >
                        <PlaylistIcon width="14" height="14" />
                        <span>{{ pl.name }}</span>
                    </button>
                </div>
            </div>
        </Transition>
    </Teleport>
    <Teleport to="body">
        <Transition name="playlist-sheet">
            <div
                v-if="showPlaylistSheet"
                class="mobilePlaylistBackdrop"
                @mousedown="emit('close')"
            ></div>
        </Transition>
        <Transition name="playlist-sheet">
            <div
                v-if="showPlaylistSheet"
                ref="playlistSheetEl"
                class="mobilePlaylistSheet"
                @mousedown.stop
                @pointerdown.self="onPlaylistSheetDragStart"
            >
                <div class="mobilePlaylistSheetHandle" @pointerdown="onPlaylistSheetDragStart"></div>
                <div class="mobilePlaylistSheetTitle">Добавить в плейлист</div>
                <div class="mobilePlaylistList" @pointerdown.self="onPlaylistSheetDragStart">
                    <button
                        class="mobilePlaylistRow mobilePlaylistCreateRow"
                        @click="onCreatePlaylistAndAdd"
                    >
                        <span class="mobilePlaylistCreateIcon">
                            <PlusIcon width="20" height="20" />
                        </span>
                        <span class="mobilePlaylistName">Создать плейлист</span>
                    </button>
                    <button
                        v-for="pl in userPlaylists"
                        :key="pl.id"
                        class="mobilePlaylistRow"
                        :disabled="trackPlaylistIds.has(pl.id)"
                        @click="onAddToPlaylist(pl.id)"
                    >
                        <AnimatedCover class="mobilePlaylistCover" :src="playlistCover(pl)" />
                        <span class="mobilePlaylistName">{{ pl.name }}</span>
                        <span v-if="trackPlaylistIds.has(pl.id)" class="mobilePlaylistState">Уже добавлен</span>
                    </button>
                </div>
            </div>
        </Transition>
    </Teleport>
</template>

<style scoped>
.trackSheetActions {
    display: contents;
}

.ctx-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 12px;
    background: none;
    border: none;
    color: hsla(0, 0%, 100%, 0.8);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: 0.24s ease;
    user-select: none;
    white-space: nowrap;
    text-align: left;
}
.ctx-item:hover {
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 1);
}
.ctx-item:disabled {
    cursor: default;
    color: hsla(0, 0%, 100%, 0.3);
}
.ctx-item:disabled svg {
    color: hsla(0, 0%, 100%, 0.25);
}
.ctx-item:disabled:hover {
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.3);
}
.ctx-item:disabled:hover svg {
    color: hsla(0, 0%, 100%, 0.25);
}
.ctx-item svg {
    transition: 0.24s ease;
    flex-shrink: 0;
    color: hsla(0, 0%, 100%, 0.5);
}
.ctx-item:hover svg {
    color: hsla(0, 0%, 100%, 0.8);
}
.ctx-separator {
    height: 1px;
    margin: 4px 8px;
    background-color: hsla(0, 0%, 100%, 0.06);
}
.ctx-submenu-wrapper {
    position: relative;
}
.ctx-submenu-trigger {
    position: relative;
}
.ctx-add-playlist-row {
    display: flex;
}
.ctx-chevron {
    margin-left: auto;
    transition: transform 0.2s ease;
}
.ctx-chevron.open {
    transform: rotate(-90deg);
}
.ctx-submenu-trigger:hover .ctx-chevron:not(.open) {
    transform: rotate(-90deg);
}
.ctx-submenu {
    position: fixed;
    z-index: 1000001;
    min-width: 180px;
    visibility: hidden;
}
.ctx-submenu.ready {
    visibility: visible;
}
.ctx-submenu-buffer {
    position: absolute;
    inset: -20px;
    z-index: -1;
}
.ctx-submenu-list {
    max-height: 280px;
    overflow-y: auto;
    border-radius: 12px;
    background-color: hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0.15);
    backdrop-filter: blur(17px);
    box-shadow: 0 0px 16px rgba(0, 0, 0, 0.1);
}
.ctx-submenu-list::-webkit-scrollbar {
    width: 4px;
}
.ctx-submenu-list::-webkit-scrollbar-track {
    background: transparent;
}
.ctx-submenu-list::-webkit-scrollbar-thumb {
    background: hsla(0, 0%, 100%, 0.12);
    border-radius: 2px;
}
.ctx-create-playlist-item {
    font-weight: 700;
}
.sub-enter-active,
.sub-leave-active {
    transition: opacity 0.12s ease, transform 0.12s ease;
}
.sub-enter-from,
.sub-leave-to {
    opacity: 0;
    transform: translateX(4px);
}

.mobilePlaylistSheet {
    display: none;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .trackSheetActions {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 12px;
        padding: 0 16px 14px;
    }

    .trackSheetAction.ctx-item {
        min-height: 116px !important;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 10px;
        border-radius: 24px;
        background: hsla(0, 0%, 100%, 0.08);
        color: hsla(0, 0%, 100%, 0.94);
        font-size: 13px;
        font-weight: 800;
        text-align: center;
    }

    .trackSheetAction.ctx-item svg {
        width: 20px;
        height: 20px;
        padding: 9px;
        box-sizing: content-box;
        border-radius: 999px;
        background: hsla(0, 0%, 100%, 0.12);
        color: hsla(0, 0%, 100%, 0.94);
    }

    .ctx-item:not(.trackSheetAction) {
        min-height: 52px !important;
        padding: 14px 18px;
        font-size: 15px;
    }

    .ctx-add-playlist-row.ctx-item {
        display: flex !important;
        visibility: visible !important;
        opacity: 1 !important;
    }

    .ctx-separator {
        margin: 8px 16px;
        background-color: hsla(0, 0%, 100%, 0.08);
    }

    .ctx-submenu-trigger .ctx-chevron {
        display: none;
    }

    .mobilePlaylistBackdrop {
        position: fixed;
        inset: 0;
        z-index: 1000001;
        background: transparent;
    }

    .mobilePlaylistSheet {
        position: fixed;
        left: 0;
        right: 0;
        bottom: 0;
        z-index: 1000002;
        display: flex;
        flex-direction: column;
        width: 100vw;
        height: 50dvh;
        padding: 0 14px calc(var(--safe-bottom) + 14px);
        border-radius: 28px 28px 0 0;
        background: hsl(var(--accent-h), min(var(--accent-s), 48%), 11%);
        box-shadow: 0 -22px 60px rgba(0, 0, 0, 0.46);
        overflow: hidden;
        touch-action: pan-y;
        transition: transform .2s ease;
        will-change: transform;
    }

    .mobilePlaylistSheetHandle {
        display: flex;
        flex: 0 0 auto;
        height: 34px;
        align-items: center;
        justify-content: center;
        cursor: grab;
        touch-action: none;
    }

    .mobilePlaylistSheetHandle::before {
        content: "";
        width: 42px;
        height: 4px;
        border-radius: 999px;
        background-color: hsla(0, 0%, 100%, 0.28);
    }

    .mobilePlaylistSheetTitle {
        flex: 0 0 auto;
        padding: 0 4px 12px;
        font-size: 17px;
        line-height: 1.2;
        font-weight: 800;
        color: hsla(0, 0%, 100%, 0.94);
        user-select: none;
    }

    .mobilePlaylistList {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
        overscroll-behavior: contain;
        touch-action: pan-y;
        padding-bottom: 4px;
    }

    .mobilePlaylistRow {
        width: 100%;
        min-height: 62px;
        display: grid;
        grid-template-columns: 46px minmax(0, 1fr) auto;
        align-items: center;
        gap: 12px;
        padding: 8px 4px;
        border-radius: 16px;
        background: transparent;
        color: hsla(0, 0%, 100%, 0.88);
        cursor: pointer;
        text-align: left;
    }

    .mobilePlaylistRow:disabled {
        cursor: default;
        color: hsla(0, 0%, 100%, 0.42);
    }

    .mobilePlaylistCreateRow {
        grid-template-columns: 46px minmax(0, 1fr);
        margin-bottom: 6px;
        color: hsla(0, 0%, 100%, 0.95);
        font-weight: 800;
    }

    .mobilePlaylistCreateIcon {
        width: 46px;
        height: 46px;
        display: flex;
        align-items: center;
        justify-content: center;
        border-radius: 999px;
        background: hsla(0, 0%, 100%, 0.12);
        color: hsla(0, 0%, 100%, 0.95);
    }

    .mobilePlaylistCover {
        width: 46px;
        height: 46px;
        border-radius: 10px;
        object-fit: cover;
        background: hsla(0, 0%, 100%, 0.06);
    }

    .mobilePlaylistName {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-size: 14px;
        font-weight: 700;
    }

    .mobilePlaylistState {
        font-size: 11px;
        font-weight: 700;
        color: hsla(0, 0%, 100%, 0.38);
        white-space: nowrap;
    }

    .playlist-sheet-enter-active,
    .playlist-sheet-leave-active {
        transition: transform .2s ease, opacity .2s ease;
    }

    .playlist-sheet-enter-from,
    .playlist-sheet-leave-to {
        opacity: 0;
        transform: translateY(20px);
    }
}
</style>
