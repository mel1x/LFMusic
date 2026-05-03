<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted, watchEffect } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import MaximizeIcon from './components/Icons/MaximizeIcon.vue';
import MinimizeIcon from './components/Icons/MinimizeIcon.vue';
import CloseIcon from './components/Icons/CloseIcon.vue';

import MainBlock from './components/MainBlock.vue';
import Player from './components/Player.vue';
import GithubIcon from './components/Icons/GithubIcon.vue';
import TrackInfoModal from './components/TrackInfoModal.vue';
import TrackEditModal from './components/TrackEditModal.vue';
import { useSettings } from './composables/useSettings';
import { usePlayer } from './composables/usePlayer';
import { useFolders } from './composables/useFolders';
import { useCustomCovers } from './composables/useCustomCovers';
import { startDomI18n } from './composables/useDomI18n';
import { useI18n } from './composables/useI18n';
import splashVideoWebm from './assets/splash_screen_dark.webm';
import splashVideoMp4 from './assets/splash_screen_dark.mp4';

const win = getCurrentWindow();
const { settings, load } = useSettings();
const { language, t } = useI18n();
const { saveState, currentTrack, currentAccentColor } = usePlayer();
const { showTracksReadyModal, dismissTracksReadyModal } = useFolders();
const { loadCustomCovers } = useCustomCovers();

interface AudioFeatureProgressPayload {
  current: number;
  total: number;
  percent: number;
  active: boolean;
}

const BASE_W = 1460;
const BASE_H = 860;
const MIN_SCALE = 0.78;
const MAX_SCALE = 2.0;
let resizeRaf: number | null = null;
let unlistenAudioFeatureProgress: UnlistenFn | null = null;
let featureProgressHideTimer: ReturnType<typeof setTimeout> | null = null;

function applyUiScale() {
  resizeRaf = null;
  const w = window.innerWidth;
  const h = window.innerHeight;
  const raw = Math.min(w / BASE_W, h / BASE_H);
  const scale = Math.max(MIN_SCALE, Math.min(MAX_SCALE, raw));
  const body = document.body;
  (body.style as any).zoom = String(scale);
  body.style.width = (w / scale) + 'px';
  body.style.height = (h / scale) + 'px';
  document.documentElement.style.setProperty('--ui-scale', String(scale));
  (window as any).__uiScale = scale;
}

function scheduleUiScale() {
  if (resizeRaf !== null) return;
  resizeRaf = requestAnimationFrame(applyUiScale);
}

const isReload = sessionStorage.getItem('lfmusic_reloaded') === '1';
if (isReload) sessionStorage.removeItem('lfmusic_reloaded');

const showSplash = ref(!isReload);
const splashFading = ref(false);
const featureProgressVisible = ref(false);
const featureProgress = ref<AudioFeatureProgressPayload>({
  current: 0,
  total: 0,
  percent: 0,
  active: false,
});

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
  return t('audioFeatures.progressActiveTitle');
});

const featureProgressTooltipText = computed(() => {
  const p = featureProgress.value;
  if (p.total <= 0) return t('audioFeatures.progressStarting');
  if (!p.active || p.current >= p.total) {
    return t('audioFeatures.progressDone', { total: p.total });
  }
  return t('audioFeatures.progressActive', { current: p.current, total: p.total });
});

const featureProgressAriaLabel = computed(() => {
  return `${featureProgressTooltipHeading.value}. ${featureProgressTooltipText.value}`;
});

function onSplashEnded() {
  splashFading.value = true;
  setTimeout(() => {
    showSplash.value = false;
  }, 600);
}

// Перехват F5 / Ctrl+R — сохраняем состояние перед перезагрузкой
async function handleReloadKey(e: KeyboardEvent) {
  if (e.key === 'Escape' && showTracksReadyModal.value) {
    e.preventDefault();
    dismissTracksReadyModal();
    return;
  }

  if (e.key === 'F5' || (e.ctrlKey && e.key === 'r')) {
    e.preventDefault();
    sessionStorage.setItem('lfmusic_reloaded', '1');
    await saveState();
    location.reload();
  }
}

// Блокируем стандартное контекстное меню браузера (содержит «Обновить»)
function handleContextMenu(e: MouseEvent) {
  e.preventDefault();
}

function onTracksReadyBackdrop(e: MouseEvent) {
  if (e.target === e.currentTarget) dismissTracksReadyModal();
}

onMounted(() => {
  load();
  startDomI18n();
  loadCustomCovers();
  applyUiScale();
  void listen<AudioFeatureProgressPayload>('audio_feature_progress', (event) => {
    const payload = event.payload;
    featureProgress.value = payload;

    if (featureProgressHideTimer) {
      clearTimeout(featureProgressHideTimer);
      featureProgressHideTimer = null;
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
  }).catch(() => {
    // Продолжение фоновой подготовки не должно мешать запуску интерфейса.
  });
  window.addEventListener('resize', scheduleUiScale);
  document.addEventListener('keydown', handleReloadKey);
  document.addEventListener('contextmenu', handleContextMenu);
});

onUnmounted(() => {
  window.removeEventListener('resize', scheduleUiScale);
  if (resizeRaf !== null) {
    cancelAnimationFrame(resizeRaf);
    resizeRaf = null;
  }
  if (featureProgressHideTimer) {
    clearTimeout(featureProgressHideTimer);
    featureProgressHideTimer = null;
  }
  if (unlistenAudioFeatureProgress) {
    unlistenAudioFeatureProgress();
    unlistenAudioFeatureProgress = null;
  }
  document.removeEventListener('keydown', handleReloadKey);
  document.removeEventListener('contextmenu', handleContextMenu);
});

watchEffect(() => {
  const hex = settings.value.dynamic_accent_color
    ? (currentAccentColor.value ?? '#1a1a1a')
    : settings.value.static_accent_color;
  const { h, s, l } = hexToHSL(hex);
  const root = document.documentElement.style;
  root.setProperty('--accent-h', `${h}`);
  root.setProperty('--accent-s', `${s}%`);
  root.setProperty('--accent-l', `${l}%`);
  root.setProperty('--accent-color', h === 0 && s === 0 ? '#353535' : `hsl(${h}, ${s}%, ${l}%)`);
});

function hexToHSL(hex: string) {
  const r = parseInt(hex.slice(1, 3), 16) / 255;
  const g = parseInt(hex.slice(3, 5), 16) / 255;
  const b = parseInt(hex.slice(5, 7), 16) / 255;

  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  const l = (max + min) / 2;
  let h = 0, s = 0;

  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6;
    else if (max === g) h = ((b - r) / d + 2) / 6;
    else h = ((r - g) / d + 4) / 6;
  }

  return { h: Math.round(h * 360), s: Math.round(s * 100), l: Math.round(l * 100) };
}

const minimizeWindow = () => win.minimize();
const maximizeWindow = () => win.toggleMaximize();
const closeWindow = async () => {
  await saveState();
  win.close();
};
</script>

<template>
  <div data-tauri-drag-region v-if="showSplash" class="splash-overlay" :class="{ 'splash-fading': splashFading }">
    <video data-tauri-drag-region autoplay muted playsinline @ended="onSplashEnded" class="splash-video">
      <source :src="splashVideoWebm" type="video/webm" />
      <source :src="splashVideoMp4" type="video/mp4" />
    </video>
  </div>
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
  <MainBlock :key="`main-${language}`" class="mainFrame" />
  <Player :key="`player-${language}`" class="playerPart" />
  <TrackInfoModal :key="`info-${language}`" />
  <TrackEditModal :key="`edit-${language}`" />
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

<style>
.splash-overlay {
  position: fixed;
  inset: 0;
  z-index: 99999;
  background: #000;
  display: flex;
  align-items: center;
  justify-content: center;
  opacity: 1;
  transition: opacity 0.6s ease;
}
.splash-overlay.splash-fading {
  opacity: 0;
}
.splash-video {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.toast {
  display: flex;
  flex-direction: row;
  gap: 9px;
  align-items: center;
  width: 300px;
  background-color: var(--dark-bg);
  border: 2px solid #ffffff20;
  border-radius: 9999px;
  padding: 6px;
  color: #fff;
  font-size: 13px;
  font-weight: 500;
  transition: transform 400ms ease, opacity 400ms ease, background-color .24s ease;
}
* {
  margin: 0;
  padding: 0;
  border: 0;
  font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  box-sizing: border-box;
  outline: none;
  text-decoration: none;
}
svg {
  display: block;
}
html {
  /* Фоны используют ту же насыщенность что и акцент (с потолком),
     чтобы все три тона --accent-color / --dark-bg / --light-bg стакались. */
  --dark-bg: hsl(var(--accent-h), min(var(--accent-s), 50%), 8%);
  --light-bg: hsla(var(--accent-h), min(var(--accent-s), 45%), 15%, 1);

  background-color: var(--dark-bg);
  overflow: hidden;
  transition: .24s ease;
}
html::selection {
  color: #fff;
  background-color: var(--dark-bg);
}
body {
  background-color: var(--dark-bg);
  overflow: hidden;
  transition: .24s ease;
}
.hat {
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  height: 38px;
  background: var(--dark-bg);
  transition: .24s ease;
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
#app {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  width: 100%;
  height: 100%;
}
.hat {
  flex-shrink: 0;
}
.mainFrame {
  width: 100%;
  flex: 1;
  min-height: 0;
}
.playerPart {
  width: 100%;
  height: 104px;
  flex-shrink: 0;
}
.scanReadyBackdrop {
  position: fixed;
  top: 0;
  left: 0;
  width: calc(100vw / var(--ui-scale, 1));
  height: calc(100vh / var(--ui-scale, 1));
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
