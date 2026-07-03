<script setup lang="ts">
import type { TrackMetadata } from '../composables/usePlayer'
import { usePlayer } from '../composables/usePlayer'
import TrackArtists from './TrackArtists.vue'
import PlayIcon from './Icons/PlayIcon.vue'
import PauseIcon from './Icons/PauseIcon.vue'
import MoreIcon from './Icons/MoreIcon.vue'
import defaultTrackCover from '../assets/trackCover.webp'

// Shared catalog-style track row, used by Album/Artist/ArtistTracks/Playlist/Search.
// Purely presentational + event-forwarding: play/pause dispatch, ripple, phone-tap
// routing and menu positioning all stay in the parent (each screen's queue/source
// semantics differ), so this component only renders and re-emits raw DOM events.
withDefaults(
    defineProps<{
        track: TrackMetadata
        /** This row's file_path is the one currently loaded in the player. */
        active?: boolean
        /** Active and paused (vs. active and playing). */
        activePaused?: boolean
        /** This row's context menu is open (hides the time, pins the "..." button visible). */
        menuOpen?: boolean
    }>(),
    { active: false, activePaused: false, menuOpen: false },
)

const emit = defineEmits<{
    (e: 'rowClick', event: MouseEvent): void
    (e: 'coverClick'): void
    (e: 'openMenu', event: MouseEvent): void
}>()

const { formatTime } = usePlayer()

function displayName(track: TrackMetadata): string {
    return track.title || track.file_path.split(/[\\/]/).pop()?.replace(/\.[^/.]+$/, '') || ''
}
</script>

<template>
    <div
        class="trackElem"
        :class="{ active, activePaused }"
        @click="emit('rowClick', $event)"
    >
        <div class="hoverIcon" @click.stop="emit('coverClick')">
            <PauseIcon
                v-if="active && !activePaused"
                width="10" height="10"
                style="margin-left:0.5px;"
            />
            <PlayIcon
                v-else
                width="10" height="10"
                style="margin-left:1px;padding-top:0;"
            />
        </div>
        <div v-if="active" class="playingAnim"></div>
        <div class="trackCover" @click.stop="emit('coverClick')">
            <img
                :src="track.cover_art || defaultTrackCover"
                alt=""
                draggable="false"
                class="trackCoverImg"
            />
        </div>
        <div class="trackInfo">
            <div v-truncate class="trackName">{{ displayName(track) }}</div>
            <div class="trackArtist"><TrackArtists :artist="track.artist" /></div>
        </div>
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
.trackElem:hover .trackCover,
.trackElem.activePaused .trackCover {
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
    transition: .31s ease;

    opacity: 0;

    &:hover {
        color: hsla(0, 0%, 100%, 0.7);
    }
}
.trackElem:hover .hoverOptionsBtn,
.hoverOptionsBtn.menuOpen {
    opacity: 1;
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
    font-variant-numeric: tabular-nums;
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
    top: 21px;
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
</style>
