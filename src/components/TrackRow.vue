<script lang="ts">
export const TRACK_ROW_HEIGHT = 52
const rowStyle = { height: TRACK_ROW_HEIGHT + 'px' }
</script>

<script setup lang="ts">
import { computed } from 'vue'
import type { TrackMetadata } from '../composables/usePlayer'
import { usePlayer } from '../composables/usePlayer'
import { usePlaylists } from '../composables/usePlaylists'
import TrackArtists from './TrackArtists.vue'
import PlayIcon from './Icons/PlayIcon.vue'
import PauseIcon from './Icons/PauseIcon.vue'
import MoreIcon from './Icons/MoreIcon.vue'
import HeartIcon from './Icons/HeartIcon.vue'
import defaultTrackCover from '../assets/trackCover.webp'

const props = withDefaults(
    defineProps<{
        track: TrackMetadata
        /** This row's file_path is the one currently loaded in the player. */
        active?: boolean
        /** Active and paused (vs. active and playing). */
        activePaused?: boolean
        /** This row's context menu is open (hides the time, pins the "..." button visible). */
        menuOpen?: boolean
        /** Position shown before the cover (collection pages). */
        rank?: number | null
        /** Collection layout with room for a rank and duration. */
        large?: boolean
    }>(),
    { active: false, activePaused: false, menuOpen: false, rank: null, large: false },
)

const emit = defineEmits<{
    (e: 'rowClick', event: MouseEvent): void
    (e: 'coverClick'): void
    (e: 'openMenu', event: MouseEvent): void
}>()

const { formatTime } = usePlayer()

const { isFavorite, toggleFavorite, ensureFavoritesLoaded } = usePlaylists()
void ensureFavoritesLoaded()

const isFav = computed(() => isFavorite(props.track.file_path))

function onToggleFav() {
    void toggleFavorite(props.track.file_path)
}

function displayName(track: TrackMetadata): string {
    return track.title || track.file_path.split(/[\\/]/).pop()?.replace(/\.[^/.]+$/, '') || ''
}
</script>

<template>
    <div
        class="trackElem"
        :style="rowStyle"
        :class="{ active, activePaused, large }"
        @click="emit('rowClick', $event)"
    >
        <span v-if="rank != null" class="trackRank">{{ rank }}</span>
        <div class="trackCover" @click.stop="emit('coverClick')">
            <img
                :src="track.cover_art || defaultTrackCover"
                alt=""
                draggable="false"
                class="trackCoverImg"
            />
            <span v-if="active && !activePaused" class="playingAnim"></span>
            <span class="hoverIcon">
                <PauseIcon v-if="active && !activePaused" width="12" height="12" />
                <PlayIcon v-else width="12" height="12" style="margin: 0; padding: 0;" />
            </span>
        </div>
        <div class="trackInfo">
            <div v-truncate class="trackName">{{ displayName(track) }}</div>
            <div class="trackArtist"><TrackArtists :artist="track.artist" /></div>
        </div>
        <button
            class="trackFavBtn"
            :class="{ liked: isFav }"
            @click.stop="onToggleFav"
            @mousedown.stop
        >
            <HeartIcon width="15" height="15" />
        </button>
        <div class="trackTime" :class="{ hidden: menuOpen }">{{ track.duration_secs ? formatTime(track.duration_secs) : '' }}</div>
        <div class="hoverOptionsBtn" :class="{ menuOpen }" @mousedown.stop="emit('openMenu', $event)" @click.stop><MoreIcon width="26" height="26" /></div>
    </div>
</template>

<style scoped>
.trackElem {
    position: relative;
    cursor: pointer;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    padding: 0 16px 0 8px;
    border-radius: 9px;
    background-color: hsla(0, 0%, 100%, 0);
    transition: .31s var(--ease-smooth-in-out);
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
    animation: ripple-expand 0.5s var(--ease-smooth) forwards;
    pointer-events: none;
}
@keyframes ripple-expand {
    to {
        transform: scale(1);
        opacity: 0;
    }
}
.trackCover {
    position: relative;
    width: 38px;
    height: 38px;
    background-color: #202020;
    border-radius: 5px;
    overflow: hidden;
    flex-shrink: 0;
}
.trackCoverImg {
    user-select: none;
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    transition: opacity .31s var(--ease-smooth);
}
.trackElem:hover .trackCoverImg,
.trackElem.activePaused .trackCoverImg {
    opacity: 0.5;
}
.trackElem.active .trackCoverImg {
    opacity: 0.45;
}
.hoverIcon {
    position: absolute;
    inset: 0;
    margin: auto;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    opacity: 0;
    border-radius: 50%;
    color: #242424;
    background-color: hsla(0, 0%, 100%, 1.0);
    transition: .31s var(--ease-smooth);
    z-index: 3;
}
.trackElem:hover .hoverIcon,
.trackElem.activePaused .hoverIcon {
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
    transition: .31s var(--ease-smooth);

    opacity: 0;

    &:hover {
        color: hsla(0, 0%, 100%, 0.7);
    }
}
.trackElem:hover .hoverOptionsBtn,
.hoverOptionsBtn.menuOpen {
    opacity: 1;
}
.trackFavBtn {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4px;
    margin-right: 4px;
    background: none;
    border: none;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.35);
    transition: color var(--duration-hover) var(--ease-smooth);
    z-index: 3;
}
.trackFavBtn:hover {
    color: hsla(0, 0%, 100%, 0.85);
}
.trackFavBtn.liked {
    color: hsla(0, 0%, 100%, 0.95);
}
.trackFavBtn.liked :deep(svg path) {
    fill: currentColor;
}
.trackTime.hidden {
    opacity: 0 !important;
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
    transition: var(--duration-hover) var(--ease-smooth);

    &:hover {
        color: hsla(0, 0%, 100%, 0.6);
    }
}
.trackTime {
    margin-left: auto;
    user-select: none;
    font-size: 14px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.45);
    transition: var(--duration-hover) var(--ease-smooth);
    flex-shrink: 0;

    opacity: 1;
}
.trackElem:hover .trackTime {
    opacity: 0;
}
.trackRank {
    flex-shrink: 0;
    width: 22px;
    text-align: center;
    user-select: none;
    font-size: 14px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.42);
}
.trackElem.active .trackRank {
    color: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
}
.trackElem.large {
    padding: 0 16px;
    gap: 12px;
    border-radius: 12px;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.trackElem.large:hover {
    background-color: hsla(0, 0%, 100%, 0.05);
}
.trackElem.large .trackCover {
    border-radius: 6px;
}
.trackElem.large .trackName {
    font-size: 14px;
    color: hsla(0, 0%, 100%, 0.94);
}
.trackElem.large .trackArtist,
.trackElem.large .trackTime {
    font-size: 12px;
}
.trackElem.large .trackTime {
    width: 44px;
    text-align: right;
    font-weight: 400;
}
:global(body.lfmusic-phone .trackRank) {
    display: none;
}
.playingAnim {
    position: absolute;
    inset: 0;
    margin: auto;
    width: 16px;
    height: 16px;
    background: #fff;
    border-radius: 50%;
    transform-origin: center;
    animation: pulse 0.8s infinite var(--ease-smooth-in-out);
    z-index: 2;
}

@keyframes pulse {
    0%, 100% { transform: scale(0.6); }
    50% { transform: scale(1); }
}
</style>
