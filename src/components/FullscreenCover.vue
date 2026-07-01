<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue';
import CloseIcon from './Icons/CloseIcon.vue';

// Global fullscreen cover overlay, opened via the `lfmusic:show-fullscreen-cover`
// window event (dispatched from cover cards/viewers).
const fullscreenCoverSrc = ref<string | null>(null);

function showFullscreenCover(src: string) {
    fullscreenCoverSrc.value = src;
}

function hideFullscreenCover() {
    fullscreenCoverSrc.value = null;
}

function handleFullscreenCoverEvent(e: Event) {
    const detail = (e as CustomEvent<string>).detail;
    if (detail) {
        showFullscreenCover(detail);
    }
}

// Escape closes (previously handled centrally in App.vue).
function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && fullscreenCoverSrc.value) {
        e.preventDefault();
        hideFullscreenCover();
    }
}

onMounted(() => {
    window.addEventListener('lfmusic:show-fullscreen-cover', handleFullscreenCoverEvent as EventListener);
    document.addEventListener('keydown', onKeydown);
});

onUnmounted(() => {
    window.removeEventListener('lfmusic:show-fullscreen-cover', handleFullscreenCoverEvent as EventListener);
    document.removeEventListener('keydown', onKeydown);
});
</script>

<template>
    <Teleport to="body">
        <Transition name="fullscreen-cover-fade">
            <div
                v-if="fullscreenCoverSrc"
                class="fullscreenCoverBackdrop"
                @click="hideFullscreenCover"
            >
                <div class="fullscreenCoverContainer" @click.stop>
                    <img :src="fullscreenCoverSrc" class="fullscreenCoverImage" alt="fullscreen cover" />
                    <button class="fullscreenCoverCloseBtn" @click="hideFullscreenCover">
                        <CloseIcon width="24" height="24" />
                    </button>
                </div>
            </div>
        </Transition>
    </Teleport>
</template>

<!-- Styles NOT scoped: .fullscreenCover* was global in App.vue; kept global for consistency. -->
<style>
.fullscreenCoverBackdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: calc(100vw / var(--overlay-scale, 1));
    height: calc(100dvh / var(--overlay-scale, 1));
    width: calc(100vw / var(--overlay-scale, 1));
    height: calc(100dvh / var(--overlay-scale, 1));
    background-color: rgba(0, 0, 0, 0.7);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 99999;
}
.fullscreenCoverContainer {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    max-width: 80dvw;
    max-height: 80dvh;
}
.fullscreenCoverImage {
    width: 70vmin;
    height: 70vmin;
    object-fit: cover;
    border-radius: 24px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.6);
    border: none;
    outline: none;
    user-select: none;
    pointer-events: auto;
}
.fullscreenCoverCloseBtn {
    position: fixed;
    top: 24px;
    right: 24px;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background-color: rgba(255, 255, 255, 0.1);
    border: none;
    outline: none;
    color: rgba(255, 255, 255, 0.8);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: background-color .24s ease, color .24s ease;
    z-index: 100000;
}
.fullscreenCoverCloseBtn:hover {
    background-color: rgba(255, 255, 255, 0.2);
    color: #fff;
}
.fullscreenCoverCloseBtn:active {
    background-color: rgba(255, 255, 255, 0.3);
}

.fullscreen-cover-fade-enter-active,
.fullscreen-cover-fade-leave-active {
    transition: opacity 0.28s ease;
}
.fullscreen-cover-fade-enter-active .fullscreenCoverImage,
.fullscreen-cover-fade-leave-active .fullscreenCoverImage {
    transition: transform 0.28s cubic-bezier(0.34, 1.56, 0.64, 1);
}
.fullscreen-cover-fade-enter-from {
    opacity: 0;
}
.fullscreen-cover-fade-enter-from .fullscreenCoverImage {
    transform: scale(0.9) translateY(10px);
}
.fullscreen-cover-fade-leave-to {
    opacity: 0;
}
.fullscreen-cover-fade-leave-to .fullscreenCoverImage {
    transform: scale(0.95);
}
</style>
