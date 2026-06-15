<script setup lang="ts">
import { Toaster } from 'vue-sonner';
import 'vue-sonner/style.css';

import ShuffleIcon from './Icons/ShuffleIcon.vue';
import RepeatIcon from './Icons/RepeatIcon.vue';

import AsideVisualizer from './player/AsideVisualizer.vue';
import PlaybackControls from './player/PlaybackControls.vue';
import PlayerFullscreenOverlay from './player/PlayerFullscreenOverlay.vue';

import { usePlayer } from '../composables/usePlayer';
import { usePlayerUi } from '../composables/usePlayerUi';
import { useAndroidMediaSession } from '../composables/useAndroidMediaSession';
import { useLinuxMediaSession } from '../composables/useLinuxMediaSession';
import { useDiscordRpc } from '../composables/useDiscordRpc';

const {
    currentTrack,
    currentCoverArt,
    playbackState,
    positionSecs,
    durationSecs,
    playPrev,
    playNext,
    togglePlayPause,
} = usePlayer();

const { discordSourceLabel } = usePlayerUi();

useAndroidMediaSession({
    currentTrack,
    currentCoverArt,
    playbackState,
    positionSecs,
    durationSecs,
    playPrev,
    playNext,
    togglePlayPause,
});

useLinuxMediaSession({
    currentTrack,
    currentCoverArt,
    playbackState,
    positionSecs,
    durationSecs,
    playPrev,
    playNext,
    togglePlayPause,
});

useDiscordRpc({
    currentTrack,
    playbackState,
    positionSecs,
    durationSecs,
    sourceLabel: discordSourceLabel,
});
</script>

<template>
    <Toaster
        position="bottom-center"
        :offset="{ bottom: '106px' }"
        style="z-index: 0;"
        :toastOptions="{
          unstyled: true,
          class: 'toast',
        }"
        :visibleToasts="1"
        theme="dark"
        :closeButton="true"
    >
        <template #info-icon>
            <ShuffleIcon width="30" height="30" style="background-color:#ffffff20;padding:7px;border-radius:999px;color:#ffffffa0" />
        </template>
        <template #warning-icon>
            <RepeatIcon width="30" height="30" style="background-color:#ffffff20;padding:7px;border-radius:999px;color:#ffffffa0" />
        </template>
    </Toaster>
    <div class="player">
        <AsideVisualizer />
        <PlaybackControls />
    </div>
    <PlayerFullscreenOverlay />
</template>

<style scoped>
.player {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    height: 104px;
    z-index: 1;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .player {
        height: calc(var(--mobile-player-height) + var(--mobile-nav-height));
        margin-bottom: 0;
        flex-shrink: 0;
        align-items: stretch;
        background:
            radial-gradient(
                ellipse 100% 75% at 50% 0%,
                hsla(var(--player-accent-h), min(var(--player-accent-s), 70%), 58%, 0.24),
                transparent 95%
            ),
            hsla(var(--player-accent-h), min(var(--player-accent-s), 50%), 8%, 0.72) !important;
        backdrop-filter: blur(20px);
        border-top: 1px solid hsla(var(--player-accent-h), min(var(--player-accent-s), 90%), 75%, 0.15);
    }
}
</style>
