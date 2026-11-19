<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import MaximizeIcon from './Icons/MaximizeIcon.vue';
import MinimizeIcon from './Icons/MinimizeIcon.vue';
import CloseIcon from './Icons/CloseIcon.vue';
import GithubIcon from './Icons/GithubIcon.vue';
import HeartIcon from './Icons/HeartIcon.vue';
import DonateModal from './DonateModal.vue';
import DownloadQueueIndicator from './DownloadQueueIndicator.vue';
import TitleSearch from './TitleSearch.vue';

import { useSettings } from '../composables/useSettings';
import { usePlayer } from '../composables/usePlayer';
import { useI18n } from '../composables/useI18n';
import { isAndroidRuntime } from '../composables/usePlatform';
import { activeHoliday, holidayModalOpen } from '../composables/useHoliday';
import { useUpdater } from '../composables/useUpdater';
import { holidayBadgeSvg } from '../utils/holidayArt';
import { splitArtistsList } from '../utils/format';
import { vLiveWidth } from '../directives/live';
import { frameLoop } from '../utils/frameLoop';

interface AudioFeatureProgressPayload {
    current: number;
    total: number;
    percent: number;
    active: boolean;
    phase?: 'analysis' | 'external' | 'download' | 'done';
}

const win = getCurrentWindow();
const { settings } = useSettings();
const { t } = useI18n();
const { currentTrack, saveState } = usePlayer();
const updater = useUpdater();
const donateOpen = ref(false);

function installUpdate() {
    updater.install(saveState);
}

const headerArtist = computed(() => {
    const artist = currentTrack.value?.artist;
    if (!artist) return 'Неизвестный исполнитель';
    return splitArtistsList(artist, settings.value.artist_separators).join(', ') || artist;
});

const featureProgressVisible = ref(false);
const featureProgress = ref<AudioFeatureProgressPayload>({
    current: 0,
    total: 0,
    percent: 0,
    active: false,
    phase: 'analysis',
});

const featureProgressIsExternal = computed(() => featureProgress.value.phase === 'external');
const featureProgressIsDownload = computed(() => featureProgress.value.phase === 'download');

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
    if (featureProgressIsDownload.value) return t('audioFeatures.downloadActiveTitle');
    if (featureProgressIsExternal.value) return t('audioFeatures.externalActiveTitle');
    return t('audioFeatures.progressActiveTitle');
});

const featureProgressTooltipText = computed(() => {
    const p = featureProgress.value;
    if (p.total <= 0) return t('audioFeatures.progressStarting');
    if (!p.active || p.current >= p.total) {
        return t('audioFeatures.progressDone', { total: p.total });
    }
    if (featureProgressIsDownload.value) {
        return t('audioFeatures.downloadActive', { current: p.current, total: p.total });
    }
    if (featureProgressIsExternal.value) {
        return t('audioFeatures.externalActive', { current: p.current, total: p.total });
    }
    return t('audioFeatures.progressActive', { current: p.current, total: p.total });
});

const featureProgressAriaLabel = computed(() => {
    return `${featureProgressTooltipHeading.value}. ${featureProgressTooltipText.value}`;
});

const displayPercent = ref(0);
const liveDisplayPercent = () => displayPercent.value;
const displayPercentRounded = computed(() => Math.round(displayPercent.value));
let trickle = 0;
let lastTarget = 0;
let stalledMs = 0;

const progressLoop = frameLoop((_ts, frameDt) => {
    const dt = frameDt ? Math.min(120, frameDt) : 16;

    const p = featureProgress.value;
    const target = featureProgressPercent.value;

    // Compare against lastTarget, not displayPercent: the trickle overshoot would falsely reset
    // the bar.
    if (p.active && target < lastTarget - 25) {
        displayPercent.value = target;
        trickle = target;
        stalledMs = 0;
    }

    if (Math.abs(target - lastTarget) > 0.1) {
        stalledMs = 0;
    } else {
        stalledMs += dt;
    }
    lastTarget = target;
    trickle = Math.max(trickle, target);

    let goal: number;
    if (!p.active) {
        goal = 100;
    } else if (featureProgressIsDownload.value) {
        if (stalledMs > 350) {
            const slotEnd = p.total > 0
                ? ((Math.min(p.current, p.total) + 1) / p.total) * 100
                : 95;
            const ceiling = Math.min(95, slotEnd);
            if (trickle < ceiling) {
                trickle += (ceiling - trickle) * (1 - Math.pow(0.75, dt / 1000));
            }
        }
        goal = Math.max(target, trickle);
    } else {
        goal = target;
    }

    const k = 1 - Math.pow(0.018, dt / 1000);
    displayPercent.value += (goal - displayPercent.value) * k;
    if (Math.abs(goal - displayPercent.value) < 0.05) displayPercent.value = goal;
}, 33);

function startProgressAnim(initial: number) {
    displayPercent.value = initial;
    trickle = initial;
    lastTarget = initial;
    stalledMs = 0;
    progressLoop.start();
}

function stopProgressAnim() {
    progressLoop.stop();
}

watch(featureProgressVisible, (vis) => {
    if (vis) startProgressAnim(featureProgressPercent.value);
    else stopProgressAnim();
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
        if (isAndroidRuntime()) return;
        return invoke('external_prepare');
    }).catch(() => {
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
    stopProgressAnim();
});

const minimizeWindow = () => win.minimize();
const maximizeWindow = () => invoke('toggle_window_maximize');
const closeWindow = () => win.close();
</script>

<template>
    <div class="hat" data-tauri-drag-region>
        <div class="winTitle">
            <div class="mainTitle" data-tauri-drag-region>{{ currentTrack ? (currentTrack.title || currentTrack.file_path.split(/[\\/]/).pop()) : 'LFMusic' }}</div>
            <div class="authorTitle" data-tauri-drag-region>{{ currentTrack ? headerArtist : 'by mel1x' }}</div>
        </div>
        <TitleSearch />
        <div class="winControls">
            <DownloadQueueIndicator />
            <div
                v-if="featureProgressVisible"
                class="featureWorkerProgress"
                :aria-label="featureProgressAriaLabel"
            >
                <div class="featureWorkerTrack">
                    <div class="featureWorkerFill" v-live-width="liveDisplayPercent"></div>
                </div>
                <span class="featureWorkerPercent">{{ displayPercentRounded }}%</span>
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
                        <template v-else-if="featureProgressIsDownload">
                            {{ t('audioFeatures.downloadActivePrefix') }}
                            <span class="featureWorkerTooltipNumber" :style="{ width: featureProgressNumberWidth }">{{ featureProgress.current }}</span>
                            {{ t('audioFeatures.downloadActiveMiddle') }}
                            <span class="featureWorkerTooltipNumber" :style="{ width: featureProgressNumberWidth }">{{ featureProgress.total }}</span>
                            {{ t('audioFeatures.downloadActiveSuffix') }}
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
            <button
                v-if="activeHoliday"
                type="button"
                class="hatBadge"
                @click="holidayModalOpen = true"
            >
                <span class="hatBadgeIcon" v-html="holidayBadgeSvg(activeHoliday)"></span>
                <span>{{ t(`holiday.${activeHoliday}`) }}</span>
            </button>
            <button
                v-if="updater.supported.value && updater.availableVersion.value"
                type="button"
                class="hatBadge updateBadge"
                :disabled="updater.status.value === 'downloading'"
                @click="installUpdate"
            >
                <span class="hatBadgeIcon">
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                        <path d="M12 4v11" />
                        <path d="m7 10 5 5 5-5" />
                        <path d="M5 20h14" />
                    </svg>
                </span>
                <span v-if="updater.status.value === 'downloading'" class="updateBadgeProgress">{{ updater.progress.value }}%</span>
                <span v-else>{{ t('updater.badge', { version: updater.availableVersion.value }) }}</span>
            </button>
            <button v-if="settings.show_donate_badge" type="button" class="hatBadge" @click="donateOpen = true">
                <span class="hatBadgeIcon"><HeartIcon width="16" height="16" /></span>
                <span>{{ t('donate.badge') }}</span>
            </button>
            <a v-if="settings.show_github_badge" class="hatBadge" href="https://github.com/mel1x/LFMusic" target="_blank" rel="noopener noreferrer">
                <span class="hatBadgeIcon"><GithubIcon /></span>
                <span>Github</span>
            </a>
            <button class="controlsButton" @click="minimizeWindow"><MinimizeIcon /></button>
            <button class="controlsButton" @click="maximizeWindow"><MaximizeIcon /></button>
            <button class="controlsButton close" @click="closeWindow"><CloseIcon /></button>
        </div>
        <DonateModal :open="donateOpen" @close="donateOpen = false" />
    </div>
</template>

<!-- Not scoped: class names are unique, and App.vue's global mobile overrides
     (.hat { display:none }) need to match these elements at the same specificity. -->
<style>
:root {
    --hat-height: 52px;
}
.hat {
    position: relative;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    height: var(--hat-height);
    background: transparent;
    transition: var(--duration-hover) var(--ease-smooth);
    flex-shrink: 0;
}
.winTitle {
    padding: 0px 20px;
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    max-width: calc(50% - min(170px, 17vw) - 12px);
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
    transition: var(--duration-hover) var(--ease-smooth);
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
    height: 34px;
    margin-right: 8px;
    padding: 0 12px;
    border-radius: 999px;
    display: flex;
    align-items: center;
    gap: 8px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.84);
    cursor: help;
    transition: background-color var(--duration-hover) var(--ease-smooth), border-color var(--duration-hover) var(--ease-smooth);
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
    background: hsla(var(--accent-h), min(calc(var(--accent-s) + 16% * var(--accent-k, 1)), 100%), 68%, 0.95);
    /* Width is driven by script; a CSS width transition would double the smoothing. */
    transition: background var(--duration-hover) var(--ease-smooth);
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
    transition: opacity var(--duration-hover) var(--ease-smooth), transform var(--duration-hover) var(--ease-smooth);
    z-index: 20;
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
.hatBadge {
    height: 34px;
    margin-right: 8px;
    padding: 0 14px 0 10px;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 7px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.75);
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    user-select: none;
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.hatBadge:hover {
    background-color: hsla(0, 0%, 100%, 0.1);
    color: #fff;
}
.hatBadge + .controlsButton {
    margin-left: 4px;
}
.updateBadge {
    color: hsl(var(--accent-h), min(var(--accent-s), 80%), 80%);
}
.updateBadge:disabled {
    cursor: default;
}
.updateBadgeProgress {
    min-width: 3ch;
    text-align: right;
    font-variant-numeric: tabular-nums;
}
.hatBadgeIcon {
    width: 18px;
    height: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
}
.hatBadgeIcon svg {
    width: 16px;
    height: 16px;
}
.hatBadgeIcon svg.lfi-github {
    width: 19px;
    height: 19px;
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
    transition: background-color 0.14s var(--ease-smooth-in-out);

    &:hover {
        background-color: #ffffff0d;
    }
}
.controlsButton.close {
    transition: background-color var(--duration-hover) var(--ease-smooth-in-out);

    &:hover {
        background-color: #f23f42;
    }
}
</style>
