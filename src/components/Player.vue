<script setup lang="ts">
import { ref } from 'vue';

const playerProgress = ref(0.0);
</script>

<template>
    <div class="player">
        <div class="asideWrapper">
            <div class="aside">
                sosi mne davai
            </div>
        </div>
        <div class="playerControlsWrapper">
            <input
                type="range"
                class="progress-slider"
                min="0"
                max="120"
                step="0.1"
                v-model="playerProgress"
            />
            <div class="playerControls">
                <div class="progress-bar-bg"></div>
                <div class="progress-bg" :style="{ width: (playerProgress / 1.2) + '%' }"></div>
                <div class="botPart">
                    <div class="trackInfo"></div>
                    <div class="controls"></div>
                    <div class="additionalControls"></div>
                </div>
            </div>
        </div>
    </div>
</template>

<style scoped>
.player {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    height: 104px;
}
.asideWrapper {
    width: 260px;
    height: 100%;
    padding: 12px;
}
.aside {
    width: 100%;
    height: 100%;
    border-radius: 12px;
    background-color: #1a1a1a;
}
.playerControlsWrapper {
    display: flex;
    flex-direction: column;
    width: calc(100% - 260px);
    height: 100%;
    padding: 12px 12px 12px 0px;
}
.progress-slider {
    overflow: visible;
    margin-bottom: -7px;
    margin-left: -4px;
    width: calc(100% + 4px * 2);
    height: 7px;
    background-color: transparent;
    appearance: none;
    -webkit-appearance: none;
    cursor: pointer;
    z-index: 99999;
}
.progress-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;

    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #fff;
    opacity: 0;
    transition: opacity 0.3s ease;
}
.progress-slider:hover::-webkit-slider-thumb {
    opacity: 1;
}
.playerControls {
    position: relative;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    border-radius: 12px;
    border: 1px solid #ffffff04;
    background-color: #1a1a1a;
}
.progress-bar-bg {
    height: 3px;
    width: 100%;
    background-color: hsla(0, 0%, 100%, 0.12);
    opacity: 0;
    transition: opacity 0.3s ease;
}
.progress-bg {
    position: absolute;
    top: 0;
    left: 0;
    height: 104px;
    width: 0%;
    background-color: hsla(0, 0%, 100%, 0.16);
}
.playerControlsWrapper:hover .progress-bar-bg {
    opacity: 1;
}
.botPart {
    height: calc(100% - 3px);
    width: 100%;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
}
</style>
