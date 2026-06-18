<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import MaximizeIcon from './Icons/MaximizeIcon.vue';
import MinimizeIcon from './Icons/MinimizeIcon.vue';
import CloseIcon from './Icons/CloseIcon.vue';
import GithubIcon from './Icons/GithubIcon.vue';

import { useSettings } from '../composables/useSettings';
import { usePlayer } from '../composables/usePlayer';
import { useI18n } from '../composables/useI18n';
import { isAndroidRuntime } from '../composables/usePlatform';

interface AudioFeatureProgressPayload {
    current: number;
    total: number;
    percent: number;
    active: boolean;
    // Активная фаза: анализ треков для рекомендаций или получение из интернета.
    phase?: 'analysis' | 'external' | 'done';
}

const win = getCurrentWindow();
const { settings } = useSettings();
const { t } = useI18n();
const { currentTrack, saveState } = usePlayer();

// ── Прогресс фонового воркера аудио-фич ─────────────────────────────────────
const featureProgressVisible = ref(false);
const featureProgress = ref<AudioFeatureProgressPayload>({
    current: 0,
    total: 0,
    percent: 0,
    active: false,
    phase: 'analysis',
});

// Идёт ли сейчас получение данных из интернета (внешние источники). По этой
// фазе выбираем текст: анализ рекомендаций ↔ получение из интернета.
const featureProgressIsExternal = computed(() => featureProgress.value.phase === 'external');

let unlistenAudioFeatureProgress: UnlistenFn | null = null;
let featureProgressHideTimer: ReturnType<typeof setTimeout> | null = null;

const featureProgressPercent = computed(() => {
    return Math.max(0, Math.min(100, featureProgress.value.percent || 0));
});

const featureProgressNumberWidth = computed(() => {
    const p = featureProgress.value;
    const digits = Math.max(1, String(Math.max(p.current, p.total)).length);
    return `${digits * 0.68}em`;
});

const featureProgressIsDone = computed(() => {
    const p = featureProgress.value;
    return p.total > 0 && (!p.active || p.current >= p.total);
});

const featureProgressTooltipHeading = computed(() => {
    const p = featureProgress.value;
    if (p.total <= 0) return t('audioFeatures.progressPendingTitle');
    if (!p.active || p.current >= p.total) {
        return t('audioFeatures.progressDoneTitle');
    }
    if (featureProgressIsExternal.value) return t('audioFeatures.externalActiveTitle');
    return t('audioFeatures.progressActiveTitle');
});

const featureProgressTooltipText = computed(() => {
    const p = featureProgress.value;
    if (p.total <= 0) return t('audioFeatures.progressStarting');
    if (!p.active || p.current >= p.total) {
        return t('audioFeatures.progressDone', { total: p.total });
    }
    if (featureProgressIsExternal.value) {
        return t('audioFeatures.externalActive', { current: p.current, total: p.total });
    }
    return t('audioFeatures.progressActive', { current: p.current, total: p.total });
});

const featureProgressAriaLabel = computed(() => {
    return `${featureProgressTooltipHeading.value}. ${featureProgressTooltipText.value}`;
});

function publishAndroidFeatureProgress(payload: AudioFeatureProgressPayload) {
    if (!isAndroidRuntime()) return false;
    const bridge = window.LFMusicAndroid?.updateAudioFeatureProgress;
    if (!bridge) return false;

    bridge(JSON.stringify({
        current: payload.current,
        total: payload.total,
        percent: payload.percent,
        active: payload.active,
        title: featureProgressTooltipHeading.value,
        text: featureProgressTooltipText.value,
    }));
    return true;
}

onMounted(() => {
    void listen<AudioFeatureProgressPayload>('audio_feature_progress', (event) => {
        const payload = event.payload;
        featureProgress.value = payload;

        if (featureProgressHideTimer) {
            clearTimeout(featureProgressHideTimer);
            featureProgressHideTimer = null;
        }

        if (publishAndroidFeatureProgress(payload)) {
            featureProgressVisible.value = false;
            return;
        }

        featureProgressVisible.value = payload.total > 0;
        if (payload.total > 0 && (!payload.active || payload.current >= payload.total)) {
            featureProgressHideTimer = setTimeout(() => {
                featureProgressVisible.value = false;
            }, 2200);
        }
    }).then((unlisten) => {
        unlistenAudioFeatureProgress = unlisten;
        return invoke<number>('library_resume_audio_features');
    }).then(() => {
        // Догружаем данные включённых внешних источников одним фоновым проходом
        // (его шаги дописываются к тому же прогресс-бару, что и анализ).
        return invoke('external_prepare');
    }).catch(() => {
        // Продолжение фоновой подготовки не должно мешать запуску интерфейса.
    });
});

onUnmounted(() => {
    if (featureProgressHideTimer) {
        clearTimeout(featureProgressHideTimer);
        featureProgressHideTimer = null;
    }
    if (unlistenAudioFeatureProgress) {
        unlistenAudioFeatureProgress();
        unlistenAudioFeatureProgress = null;
    }
});

// ── Window controls ─────────────────────────────────────────────────────────
const minimizeWindow = () => win.minimize();
// Через backend: в полноэкранном режиме (F11) клик выходит из него и
// разворачивает окно, иначе — обычное переключение maximized.
const maximizeWindow = () => invoke('toggle_window_maximize');
const closeWindow = async () => {
    await saveState();
    win.close();
};
</script>

<template>
    <div class="hat" data-tauri-drag-region>
        <div class="winTitle">
            <div class="mainTitle" data-tauri-drag-region>{{ currentTrack ? (currentTrack.title || currentTrack.file_path.split(/[\\/]/).pop()) : 'LFMusic' }}</div>
            <div class="authorTitle" data-tauri-drag-region>{{ currentTrack ? (currentTrack.artist || 'Неизвестный исполнитель') : 'by mel1x' }}</div>
        </div>
        <div class="winControls">
            <div
                v-if="featureProgressVisible"
                class="featureWorkerProgress"
                :aria-label="featureProgressAriaLabel"
            >
                <div class="featureWorkerTrack">
                    <div class="featureWorkerFill" :style="{ width: featureProgressPercent + '%' }"></div>
                </div>
                <span class="featureWorkerPercent">{{ Math.round(featureProgressPercent) }}%</span>
                <div class="featureWorkerTooltip" role="tooltip">
                    <div class="featureWorkerTooltipTitle">{{ featureProgressTooltipHeading }}</div>
                    <div class="featureWorkerTooltipText">
                        <template v-if="featureProgress.total <= 0">
                            {{ t('audioFeatures.progressStarting') }}
                        </template>
                        <template v-else-if="featureProgressIsDone">
                            {{ t('audioFeatures.progressDonePrefix') }}
                            <span class="featureWorkerTooltipNumber" :style="{ width: featureProgressNumberWidth }">{{ featureProgress.total }}</span>
                            {{ t('audioFeatures.progressDoneSuffix') }}
                        </template>
                        <template v-else-if="featureProgressIsExternal">
                            {{ t('audioFeatures.externalActivePrefix') }}
                            <span class="featureWorkerTooltipNumber" :style="{ width: featureProgressNumberWidth }">{{ featureProgress.current }}</span>
                            {{ t('audioFeatures.externalActiveMiddle') }}
                            <span class="featureWorkerTooltipNumber" :style="{ width: featureProgressNumberWidth }">{{ featureProgress.total }}</span>
                            {{ t('audioFeatures.externalActiveSuffix') }}
                        </template>
                        <template v-else>
                            {{ t('audioFeatures.progressActivePrefix') }}
                            <span class="featureWorkerTooltipNumber" :style="{ width: featureProgressNumberWidth }">{{ featureProgress.current }}</span>
                            {{ t('audioFeatures.progressActiveMiddle') }}
                            <span class="featureWorkerTooltipNumber" :style="{ width: featureProgressNumberWidth }">{{ featureProgress.total }}</span>
                            {{ t('audioFeatures.progressActiveSuffix') }}
                        </template>
                    </div>
                </div>
            </div>
            <a v-if="settings.show_github_badge" href="https://github.com/mel1x/LFMusic" target="_blank" rel="noopener noreferrer"><GithubIcon/>Github</a>
            <button class="controlsButton" @click="minimizeWindow"><MinimizeIcon /></button>
            <button class="controlsButton" @click="maximizeWindow"><MaximizeIcon /></button>
            <button class="controlsButton close" @click="closeWindow"><CloseIcon /></button>
        </div>
    </div>
</template>

<!-- Стили НЕ scoped: классы (.hat/.featureWorker*/.controlsButton) уникальны, а в
     App.vue остаются глобальные мобильные оверрайды (.hat { display:none }), которые
     должны матчить эти элементы с прежней специфичностью. -->
<style>
.hat {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    height: 38px;
    background: var(--dark-bg);
    transition: .24s ease;
    flex-shrink: 0;
}
.winTitle {
    margin-top: 8px;
    padding: 0px 20px;
    display: flex;
    flex-direction: column;
    gap: 3px;
}
.mainTitle {
    user-select: none;
    font-size: 15px;
    font-weight: 800;
    color: #fff;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    letter-spacing: -0.025em;
}
.authorTitle {
    user-select: none;
    font-size: 11px;
    line-height: 1;
    font-weight: 600;
    text-transform: uppercase;
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.55);
    transition: .24s ease;
}
.winControls {
    height: 100%;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: center;
}
.featureWorkerProgress {
    position: relative;
    width: 176px;
    height: 25px;
    margin-right: 10px;
    padding: 0 8px;
    border-radius: 999px;
    display: flex;
    align-items: center;
    gap: 8px;
    background: linear-gradient(172deg, hsla(var(--accent-h), var(--accent-s), 25%, 1), hsla(var(--accent-h), var(--accent-s), 15%, 1));
    color: hsla(0, 0%, 100%, 0.84);
    cursor: help;
    transition: background-color .24s ease, border-color .24s ease;
}
.featureWorkerProgress:hover {
    border-color: hsla(0, 0%, 100%, 0.14);
}
.featureWorkerTrack {
    position: relative;
    flex: 1;
    height: 4px;
    overflow: hidden;
    border-radius: 999px;
    background: hsla(0, 0%, 100%, 0.13);
}
.featureWorkerFill {
    height: 100%;
    min-width: 4px;
    border-radius: inherit;
    background: hsla(var(--accent-h), min(calc(var(--accent-s) + 16%), 100%), 68%, 0.95);
    transition: width .28s ease;
}
.featureWorkerPercent {
    min-width: 30px;
    text-align: right;
    font-size: 11px;
    line-height: 1;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    user-select: none;
}
.featureWorkerTooltip {
    position: absolute;
    top: calc(100% + 9px);
    right: 0;
    width: 290px;
    padding: 11px 12px 12px;
    border-radius: 14px;
    background: hsla(var(--accent-h), min(var(--accent-s), 42%), 10%, 0.98);
    border: 1px solid hsla(0, 0%, 100%, 0.1);
    box-shadow: 0 14px 38px rgba(0, 0, 0, 0.36);
    color: #fff;
    opacity: 0;
    pointer-events: none;
    transform: translateY(-3px);
    transition: opacity .16s ease, transform .16s ease;
    z-index: 20;
}
.featureWorkerTooltip::before {
    content: '';
    position: absolute;
    top: -5px;
    right: 24px;
    width: 10px;
    height: 10px;
    transform: rotate(45deg);
    background: inherit;
    border-left: 1px solid hsla(0, 0%, 100%, 0.1);
    border-top: 1px solid hsla(0, 0%, 100%, 0.1);
}
.featureWorkerProgress:hover .featureWorkerTooltip {
    opacity: 1;
    transform: translateY(0);
}
.featureWorkerTooltipTitle {
    position: relative;
    font-size: 12px;
    line-height: 1.2;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.95);
    user-select: none;
}
.featureWorkerTooltipText {
    position: relative;
    min-height: 34px;
    margin-top: 5px;
    font-size: 12px;
    line-height: 1.42;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.66);
    user-select: none;
}
.featureWorkerTooltipNumber {
    display: inline-block;
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.82);
}
.winControls a {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 3px;
    justify-content: center;
    color: #ffffffd5;
    font-size: 12px;
    font-weight: 600;
    margin-right: 20px;
    padding: 2px 8px 3px 3px;
    border-radius: 16px;
    background: linear-gradient(172deg, hsla(var(--accent-h), var(--accent-s), 25%, 1), hsla(var(--accent-h), var(--accent-s), 15%, 1));
    transition: .24s ease;
}
.controlsButton {
    cursor: pointer;
    width: 46px;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    background-color: transparent;
    transition: background-color 0.14s ease-in-out;

    &:hover {
        background-color: #ffffff0d;
    }
}
.controlsButton.close {
    transition: background-color 0.19s ease-in-out;

    &:hover {
        background-color: #f23f42;
    }
}
</style>
