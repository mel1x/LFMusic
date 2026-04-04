<script setup lang="ts">
import { ref, computed, watch } from 'vue'

import MoreIcon from '../components/Icons/MoreIcon.vue';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import SearchIcon from '../components/Icons/SearchIcon.vue';
import defaultCover from '../assets/trackCover.png';

import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { createRipple } from '../composables/useRipple';
import { useVirtualScroll } from '../composables/useVirtualScroll';

const {
    trackList,
    currentTrack,
    isPlaying,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    syncPlaybackOrder,
    formatTime,
    loadCoversForTracks,
} = usePlayer();

// ── Search ──────────────────────────────────────────────────────────────────

const searchInput = ref<HTMLInputElement | null>(null)
const rawSearch = ref('')
const debouncedSearch = ref('')
let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null

function onSearchInput(e: Event) {
    const value = (e.target as HTMLInputElement).value
    rawSearch.value = value
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    searchDebounceTimer = setTimeout(() => {
        debouncedSearch.value = value
    }, 250)
}

const focusSearch = () => {
    searchInput.value?.focus()
}

const filteredTracks = computed(() => {
    const q = debouncedSearch.value.toLowerCase().trim()
    if (!q) return trackList.value
    return trackList.value.filter(t => {
        const title = (t.title || '').toLowerCase()
        const artist = (t.artist || '').toLowerCase()
        const fileName = t.file_path.replace(/\\/g, '/').split('/').pop()?.replace(/\.[^/.]+$/, '').toLowerCase() || ''
        return title.includes(q) || artist.includes(q) || fileName.includes(q)
    })
})

// ── Virtual scroll ──────────────────────────────────────────────────────────

const {
    containerEl: trackListEl,
    onScroll,
    visibleItems: visibleTracks,
    paddingTop: vsPaddingTop,
    paddingBottom: vsPaddingBottom,
    resetScroll,
} = useVirtualScroll(filteredTracks, { itemHeight: 58, buffer: 5 })

// Reset scroll on search change
watch(debouncedSearch, () => {
    resetScroll()
})

// Load covers for visible tracks
watch(visibleTracks, (tracks) => {
    loadCoversForTracks(tracks)
}, { immediate: true })

// ── Catalog play button ─────────────────────────────────────────────────────

const isCatalogActive = computed(() => activePlaylistSource.value === 'catalog')

function onPlayCatalog() {
    if (isCatalogActive.value) {
        togglePlayPause()
    } else {
        activePlaylistSource.value = 'catalog'
        syncPlaybackOrder()
        if (trackList.value.length > 0) {
            playTrack(trackList.value[0])
        }
    }
}

// ── Track click ─────────────────────────────────────────────────────────────

function onTrackClick(e: MouseEvent) {
    createRipple(e)
}

function onCoverClick(track: TrackMetadata) {
    if (currentTrack.value?.file_path === track.file_path) {
        togglePlayPause()
        return
    }
    if (!isCatalogActive.value) {
        activePlaylistSource.value = 'catalog'
        syncPlaybackOrder()
    }
    playTrack(track)
}


</script>

<template>
    <div class="pageRoot">
        <div class="pageHat">
            <div class="cover"></div>
            <div class="dataZone">
                <div class="info">
                    <div class="listType">Библиотека</div>
                    <h2 class="listName">Мой каталог</h2>
                    <div class="listCount" v-if="trackList.length > 0">{{ trackList.length }} треков</div>
                </div>
                <div class="controls">
                    <button class="playListBtn" @click="onPlayCatalog">
                        <PauseIcon v-if="isCatalogActive && isPlaying" width="14" height="14" />
                        <PlayIcon v-else width="14" height="14" style="margin-left:0;" />
                        Слушать
                    </button>
                    <button class="moreOptionsBtn"><MoreIcon width="22" height="22" /></button>
                </div>
            </div>
        </div>
        <div class="searchZone">
            <SearchIcon @click="focusSearch" />
            <input
                ref="searchInput"
                type="text"
                class="search"
                name="search"
                id="search"
                placeholder="Поиск трека"
                :value="rawSearch"
                @input="onSearchInput"
            />
        </div>
        <div
            class="tracksList"
            ref="trackListEl"
            @scroll="onScroll"
            v-if="filteredTracks.length > 0"
        >
            <div class="vsPad" :style="{ height: vsPaddingTop + 'px' }"></div>
            <div
                v-for="track in visibleTracks"
                :key="track.file_path"
                class="trackElem"
                :class="{ active: currentTrack?.file_path === track.file_path }"


                @click="onTrackClick($event)"
            >
                <div class="hoverIcon" @click.stop="onCoverClick(track)">
                    <PauseIcon
                        v-if="currentTrack?.file_path === track.file_path && isPlaying"
                        width="10" height="10"
                        style="margin-left:0.5px;"
                    />
                    <PlayIcon
                        v-else
                        width="10" height="10"
                        style="margin-left:1px;padding-top:0;"
                    />
                </div>
                <div
                    v-if="currentTrack?.file_path === track.file_path && isPlaying"
                    class="playingAnim"
                ></div>
                <div class="trackCover" @click.stop="onCoverClick(track)">
                    <img
                        :src="track.cover_art || defaultCover"
                        alt=""
                        draggable="false"
                        class="trackCoverImg"
                    />
                </div>
                <div class="trackInfo">
                    <div class="trackName">{{ track.title || track.file_path.split(/[\\/]/).pop()?.replace(/\.[^/.]+$/, '') }}</div>
                    <div class="trackArtist">{{ track.artist || 'Неизвестный исполнитель' }}</div>
                </div>
                <div class="trackTime">{{ track.duration_secs ? formatTime(track.duration_secs) : '' }}</div>
                <div class="hoverOptionsBtn"><MoreIcon width="26" height="26" /></div>
            </div>
            <div class="vsPad" :style="{ height: vsPaddingBottom + 'px' }"></div>
        </div>
        <div class="emptyState" v-else-if="debouncedSearch && filteredTracks.length === 0">
            <div class="emptyText">Ничего не найдено</div>
        </div>
        <div class="emptyState" v-else-if="trackList.length === 0">
            <div class="emptyText">Нет треков в библиотеке</div>
        </div>
    </div>
</template>

<style scoped>
.pageRoot {
    padding: 20px 28px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: #101010;
    overflow: hidden;
}
.pageHat {
    display: flex;
    flex-direction: row;
    gap: 30px;
    width: 100%;
    flex-shrink: 0;
}
.pageHat .cover {
    width: 200px;
    height: 200px;
    background-color: #202020;
    border-radius: 10px;
    transition: .24s ease;
    transform-origin: center;
    overflow: hidden;
    flex-shrink: 0;

    &:hover {
        transform: scale(1.02);
    }
}
.pageHat .dataZone {
    display: flex;
    flex-direction: column;
    gap: 35px;
    width: calc(100% - 230px);
}
.pageHat .dataZone .info {
    margin-top: 15px;
    width: 100%;
    display: flex;
    flex-direction: column;
}
.pageHat .dataZone .info .listType {
    user-select: none;
    margin-left: 1px;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.4);
    width: fit-content;
    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 0.6);
    }
}
.pageHat .dataZone .info .listName {
    cursor: default;
    margin-top: 2px;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 42px;
    font-weight: 900;
    color: hsla(0, 0%, 100%, 0.9);
}
.pageHat .dataZone .info .listCount {
    user-select: none;
    margin-left: 1px;
    margin-top: 14px;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.4);
    width: fit-content;
    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 0.6);
    }
}
.pageHat .dataZone .controls {
    margin-top: 0px;
    display: flex;
    flex-direction: row;
    gap: 12px;
}
.pageHat .dataZone .controls .playListBtn {
    user-select: none;
    cursor: pointer;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 6px;
    padding: 11px 16px 11px 14px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 1);
    color: hsla(0, 0%, 0%, 1);
    font-size: 12px;
    font-weight: 600;
    transition: .24s ease;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.8);
    }
}
.playListBtn svg {
    width: 16px;
}
.pageHat .dataZone .controls .moreOptionsBtn {
    cursor: pointer;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 8px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.07);
    color: hsla(0, 0%, 100%, 1);
    transition: .24s ease;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.12);
    }
}
.searchZone {
    position: relative;
    margin-top: 24px;
    width: 100%;
    flex-shrink: 0;
}
.searchZone svg {
    cursor: text;
    position: absolute;
    top: 50%;
    left: 12px;
    transform: translateY(-50%);
    color: #606060;
    transition: .24s ease;

    &:hover {
        color: #909090;
    }
}
.searchZone:hover svg {
    color: #909090;
}
.search {
    width: 100%;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.04);
    padding: 10px 16px 11px 40px;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.9);
}
.search::placeholder {
    user-select: none;
    color: hsla(0, 0%, 100%, 0.3);
    font-size: 13px;
    font-weight: 600;
}
.tracksList {
    margin-top: 28px;
    flex: 1;
    overflow-y: auto;
    min-height: 0;
}
.tracksList::-webkit-scrollbar { width: 6px; }
.tracksList::-webkit-scrollbar-track { background: transparent; }
.tracksList::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
}
.vsPad {
    width: 100%;
}
.trackElem {
    position: relative;
    cursor: pointer;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    padding: 8px 16px 8px 8px;
    border-radius: 9px;
    background-color: hsla(0, 0%, 100%, 0);
    transition: .31s ease-in-out;
    height: 58px;
    box-sizing: border-box;
    overflow: hidden;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.03);
    }
}
.trackElem :deep(.ripple) {
    position: absolute;
    border-radius: 50%;
    background: hsla(0, 0%, 100%, 0.08);
    transform: scale(0);
    animation: ripple-expand 0.5s ease-out forwards;
    pointer-events: none;
}
@keyframes ripple-expand {
    to {
        transform: scale(1);
        opacity: 0;
    }
}
.trackCover {
    width: 42px;
    height: 42px;
    background-color: #202020;
    border-radius: 5px;
    transition: .31s ease;
    overflow: hidden;
    flex-shrink: 0;
}
.trackCoverImg {
    user-select: none;
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
}
.trackElem:hover .trackCover {
    opacity: 0.5;
}
.trackElem.active .trackCover {
    opacity: 0.45;
}
.hoverIcon {
    position: absolute;
    top: 50%;
    left: 18.5px;
    transform: translateY(-50%);
    opacity: 0;
    padding: 5px;
    border-radius: 9999px;
    color: #242424;
    background-color: hsla(0, 0%, 100%, 1.0);
    transition: .31s ease;
    z-index: 3;
}
.trackElem:hover .hoverIcon {
    opacity: 1;
}
.hoverOptionsBtn {
    position: absolute;
    top: 50%;
    right: 12px;
    padding: 6px;
    transform: translateY(-50%);
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.45);
    transition: .31s ease;

    opacity: 0;

    &:hover {
        color: hsla(0, 0%, 100%, 0.7);
    }
}
.trackElem:hover .hoverOptionsBtn {
    opacity: 1;
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
.trackElem.active .trackName {
    color: hsla(0, 0%, 100%, 1);
}
.trackArtist {
    user-select: none;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.4);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: .24s ease;

    &:hover {
        color: hsla(0, 0%, 100%, 0.6);
    }
}
.trackTime {
    margin-left: auto;
    user-select: none;
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.45);
    transition: .24s ease;
    flex-shrink: 0;

    opacity: 1;
}
.trackElem:hover .trackTime {
    opacity: 0;
}
.playingAnim {
    position: absolute;
    left: 21px;
    width: 16px;
    height: 16px;
    background: #fff;
    border-radius: 50%;
    transform-origin: center;
    animation: pulse 0.8s infinite ease;
    z-index: 2;
}

@keyframes pulse {
    0%, 100% { transform: scale(0.6); }
    50% { transform: scale(1); }
}


.emptyState {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
}
.emptyText {
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.3);
}
</style>
