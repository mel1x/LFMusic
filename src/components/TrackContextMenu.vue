<script setup lang="ts">
import { ref, onUnmounted } from 'vue'
import ContextMenu from './ContextMenu.vue'
import HeartIcon from './Icons/HeartIcon.vue'
import EditIcon from './Icons/EditIcon.vue'
import ArrowUpIcon from './Icons/ArrowUpIcon.vue'
import PlaylistIcon from './Icons/PlaylistIcon.vue'
import AlbumIcon from './Icons/AlbumIcon.vue'
import InfoIcon from './Icons/InfoIcon.vue'
import type { TrackMetadata } from '../composables/usePlayer'

const props = defineProps<{
    x: number
    y: number
    track: TrackMetadata
    alignRight?: boolean
    anchorBottom?: boolean
    centerX?: boolean
    centerY?: boolean
    submenuRight?: boolean
}>()

const emit = defineEmits<{
    close: []
    moveToTop: [track: TrackMetadata]
}>()

const showPlaylistSub = ref(false)
const subWrapperEl = ref<HTMLElement | null>(null)
const subMenuEl = ref<HTMLElement | null>(null)

const testPlaylists = [
    'Избранное', 'Для тренировки', 'Вечерний чилл', 'Рабочий фокус',
    'Дорога домой', 'Утренний микс', 'Летний вайб', 'Рок классика',
    'Электроника', 'Джаз & Блюз', 'Инди подборка', 'Ночной драйв',
]

function onSubEnter() {
    showPlaylistSub.value = true
    document.addEventListener('mousemove', trackSubMouse)
}

function trackSubMouse(e: MouseEvent) {
    const wrapper = subWrapperEl.value
    const submenu = subMenuEl.value
    if (!wrapper) return

    const wr = wrapper.getBoundingClientRect()
    const buf = 15

    let left = wr.left, right = wr.right, top = wr.top, bottom = wr.bottom

    if (submenu) {
        const sr = submenu.getBoundingClientRect()
        left = Math.min(left, sr.left)
        right = Math.max(right, sr.right)
        top = Math.min(top, sr.top)
        bottom = Math.max(bottom, sr.bottom)
    }

    if (e.clientX < left - buf || e.clientX > right + buf ||
        e.clientY < top - buf || e.clientY > bottom + buf) {
        showPlaylistSub.value = false
        document.removeEventListener('mousemove', trackSubMouse)
    }
}

onUnmounted(() => {
    document.removeEventListener('mousemove', trackSubMouse)
})

function onLike() {
    // без функционала
}

function onEdit() {
    // без функционала
}

function onMoveToTop() {
    emit('moveToTop', props.track)
    emit('close')
}

function onAddToPlaylist(_name: string) {
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
        <button class="ctx-item" @click="onLike">
            <HeartIcon width="16" height="16" />
            <span>Нравится</span>
        </button>
        <button class="ctx-item" @click="onEdit">
            <EditIcon width="16" height="16" />
            <span>Редактировать</span>
        </button>
        <button class="ctx-item" @click="onMoveToTop">
            <ArrowUpIcon width="16" height="16" />
            <span>Поднять вверх</span>
        </button>
        <div class="ctx-separator"></div>
        <div
            ref="subWrapperEl"
            class="ctx-submenu-wrapper"
            @mouseenter="onSubEnter"
        >
            <div class="ctx-item ctx-submenu-trigger">
                <PlaylistIcon width="16" height="16" />
                <span>Добавить в плейлист</span>
                <svg v-if="submenuRight" class="ctx-chevron" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="m9 18 6-6-6-6"/>
                </svg>
                <svg v-else class="ctx-chevron" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="m15 18-6-6 6-6"/>
                </svg>
            </div>
            <Transition name="sub">
                <div v-if="showPlaylistSub" ref="subMenuEl" class="ctx-submenu" :class="{ right: submenuRight }">
                    <button
                        v-for="pl in testPlaylists"
                        :key="pl"
                        class="ctx-item"
                        @click.stop="onAddToPlaylist(pl)"
                    >
                        <PlaylistIcon width="14" height="14" />
                        <span>{{ pl }}</span>
                    </button>
                </div>
            </Transition>
        </div>
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
</template>

<style scoped>
.ctx-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 12px;
    border-radius: 8px;
    background: none;
    border: none;
    color: hsla(0, 0%, 100%, 0.8);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background-color 0.15s ease;
    user-select: none;
    white-space: nowrap;
    text-align: left;
}
.ctx-item:hover {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: hsla(0, 0%, 100%, 1);
}
.ctx-item svg {
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
}
.ctx-submenu {
    position: absolute;
    right: calc(100% + 4px);
    top: -6px;
    min-width: 180px;
    max-height: 280px;
    overflow-y: auto;
    padding: 6px;
    border-radius: 12px;
    background-color: hsla(0, 0%, 12%, 0.95);
    backdrop-filter: blur(20px);
    border: 1px solid hsla(0, 0%, 100%, 0.08);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5), 0 2px 8px rgba(0, 0, 0, 0.3);
}
.ctx-submenu.right {
    right: auto;
    left: calc(100% + 4px);
}
.ctx-submenu::-webkit-scrollbar {
    width: 4px;
}
.ctx-submenu::-webkit-scrollbar-track {
    background: transparent;
}
.ctx-submenu::-webkit-scrollbar-thumb {
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
