<script setup lang="ts">
import { ref, nextTick, onMounted, onBeforeUnmount } from 'vue'
import ContextMenu from './ContextMenu.vue'
import EditIcon from './Icons/EditIcon.vue'
import ArrowUpIcon from './Icons/ArrowUpIcon.vue'
import PlaylistIcon from './Icons/PlaylistIcon.vue'
import AlbumIcon from './Icons/AlbumIcon.vue'
import InfoIcon from './Icons/InfoIcon.vue'
import DeleteIcon from './Icons/DeleteIcon.vue'
import type { TrackMetadata } from '../composables/usePlayer'
import { usePlaylists } from '../composables/usePlaylists'

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

const { userPlaylists, loadPlaylists, addTrackToPlaylist, getPlaylistIdsForTrack } = usePlaylists()

const trackPlaylistIds = ref<Set<number>>(new Set())

onMounted(async () => {
    if (!userPlaylists.value.length) await loadPlaylists()
    const ids = await getPlaylistIdsForTrack(props.track.file_path)
    trackPlaylistIds.value = new Set(ids)
    window.addEventListener('resize', onWindowResize)
})

const showPlaylistSub = ref(false)
const subReady = ref(false)
const subTriggerEl = ref<HTMLElement | null>(null)
const subMenuEl = ref<HTMLElement | null>(null)
const subStyle = ref<Record<string, string>>({})
const pinnedSub = ref(false)

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
    if (pinnedSub.value) return
    cancelLeave()
    leaveTimer = setTimeout(() => {
        leaveTimer = null
        subReady.value = false
        showPlaylistSub.value = false
    }, 120)
}

function onSubClick() {
    if (pinnedSub.value) {
        closeSub()
    } else {
        openSub()
        pinnedSub.value = true
    }
}

function onSubEnter() {
    cancelLeave()
    if (!pinnedSub.value) {
        openSub()
    }
}

function onSubLeave() {
    scheduleClose()
}

function onSubMenuLeave() {
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
})

function onEdit() {
    // без функционала
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

function onRemoveFromPlaylist() {
    emit('removeFromPlaylist', props.track)
    emit('close')
}

function onGoToAlbum() {
    // без функционала
}

function onAbout() {
    // без функционала
}
</script>

<template>
    <ContextMenu :x="x" :y="y" :alignRight="alignRight" :anchorBottom="anchorBottom" :centerX="centerX" :centerY="centerY" @close="emit('close')">
        <button class="ctx-item" @click="onEdit">
            <EditIcon width="16" height="16" />
            <span>Редактировать</span>
        </button>
        <button class="ctx-item" @click="onMoveToTop">
            <ArrowUpIcon width="16" height="16" />
            <span>Поднять вверх</span>
        </button>
        <template v-if="userPlaylists.length > 0">
            <div class="ctx-separator"></div>
            <div
                ref="subTriggerEl"
                class="ctx-submenu-wrapper"
                @mouseenter="onSubEnter"
                @mouseleave="onSubLeave"
            >
                <div class="ctx-item ctx-submenu-trigger" @click.stop="onSubClick">
                    <PlaylistIcon width="16" height="16" />
                    <span>Добавить в плейлист</span>
                    <svg v-if="submenuRight" class="ctx-chevron" :class="{ open: showPlaylistSub }" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="m9 18 6-6-6-6"/>
                    </svg>
                    <svg v-else class="ctx-chevron" :class="{ open: showPlaylistSub }" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="m15 18-6-6 6-6"/>
                    </svg>
                </div>
            </div>
        </template>
        <button v-if="inPlaylist" class="ctx-item" @click="onRemoveFromPlaylist">
            <DeleteIcon width="16" height="16" />
            <span>Удалить из плейлиста</span>
        </button>
        <div class="ctx-separator"></div>
        <button class="ctx-item" @click="onGoToAlbum">
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
</template>

<style scoped>
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
.sub-enter-active,
.sub-leave-active {
    transition: opacity 0.12s ease, transform 0.12s ease;
}
.sub-enter-from,
.sub-leave-to {
    opacity: 0;
    transform: translateX(4px);
}
</style>
