<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue';
import { useFolders } from '../composables/useFolders';
import { useI18n } from '../composables/useI18n';

const { showTracksReadyModal, dismissTracksReadyModal } = useFolders();
const { t } = useI18n();

function onTracksReadyBackdrop(e: MouseEvent) {
    if (e.target === e.currentTarget) dismissTracksReadyModal();
}

// Закрытие по Escape (раньше обрабатывалось централизованно в App.vue).
function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && showTracksReadyModal.value) {
        e.preventDefault();
        dismissTracksReadyModal();
    }
}

onMounted(() => document.addEventListener('keydown', onKeydown));
onUnmounted(() => document.removeEventListener('keydown', onKeydown));
</script>

<template>
    <Teleport to="body">
        <Transition name="scan-ready">
            <div
                v-if="showTracksReadyModal"
                class="scanReadyBackdrop"
                @mousedown="onTracksReadyBackdrop"
            >
                <div class="scanReadyModal" @mousedown.stop>
                    <div class="scanReadyTitle">{{ t('audioFeatures.tracksReadyTitle') }}</div>
                    <div class="scanReadyText">
                        {{ t('audioFeatures.tracksReadyText') }}
                    </div>
                    <button class="scanReadyButton" @click="dismissTracksReadyModal">{{ t('audioFeatures.tracksReadyButton') }}</button>
                </div>
            </div>
        </Transition>
    </Teleport>
</template>

<!-- Стили НЕ scoped: в App.vue остаются глобальные мобильные оверрайды
     (.scanReadyModal { padding } и т.п.) без !important, поэтому база должна
     сохранять прежнюю (глобальную) специфичность. -->
<style>
.scanReadyBackdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: calc(100vw / var(--overlay-scale, 1));
    height: calc(100vh / var(--overlay-scale, 1));
    width: calc(100vw / var(--overlay-scale, 1));
    height: calc(100vh / var(--overlay-scale, 1));
    z-index: 1000001;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsla(0, 0%, 0%, 0.42);
    backdrop-filter: blur(5px);
}
.scanReadyModal {
    width: 430px;
    max-width: calc(100vw - 42px);
    padding: 26px 26px 18px;
    border-radius: 28px;
    background: linear-gradient(165deg, var(--light-bg), hsla(var(--accent-h), var(--accent-s), 10%, 0.98));
    box-shadow: 0 18px 48px rgba(0, 0, 0, 0.38);
    display: flex;
    flex-direction: column;
    gap: 11px;
}
.scanReadyTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 22px;
    line-height: 1.15;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.95);
    user-select: none;
}
.scanReadyText {
    font-size: 15px;
    line-height: 1.45;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.64);
    user-select: none;
}
.scanReadyButton {
    align-self: flex-end;
    margin-top: 8px;
    padding: 7px 16px;
    border-radius: 18px;
    background: hsla(0, 0%, 100%, 0.08);
    color: hsla(0, 0%, 100%, 0.9);
    font-size: 14px;
    font-weight: 700;
    cursor: pointer;
    transition: background-color .2s ease, color .2s ease;
}
.scanReadyButton:hover {
    background: hsla(0, 0%, 100%, 0.14);
    color: #fff;
}
.scan-ready-enter-active, .scan-ready-leave-active {
    transition: opacity .18s ease;
}
.scan-ready-enter-active .scanReadyModal, .scan-ready-leave-active .scanReadyModal {
    transition: transform .18s ease, opacity .18s ease;
}
.scan-ready-enter-from, .scan-ready-leave-to {
    opacity: 0;
}
.scan-ready-enter-from .scanReadyModal, .scan-ready-leave-to .scanReadyModal {
    transform: scale(0.96) translateY(4px);
    opacity: 0;
}
</style>
