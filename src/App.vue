<script setup lang="ts">
import { ref, onMounted, onUnmounted, watchEffect } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
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
import { useCustomCovers } from './composables/useCustomCovers';
import splashVideoWebm from './assets/splash_screen_dark.webm';
import splashVideoMp4 from './assets/splash_screen_dark.mp4';

const win = getCurrentWindow();
const { settings, load } = useSettings();
const { saveState, currentTrack, currentAccentColor } = usePlayer();
const { loadCustomCovers } = useCustomCovers();

const BASE_W = 1460;
const BASE_H = 860;
const MIN_SCALE = 0.78;
const MAX_SCALE = 2.0;

function applyUiScale() {
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

const isReload = sessionStorage.getItem('lfmusic_reloaded') === '1';
if (isReload) sessionStorage.removeItem('lfmusic_reloaded');

const showSplash = ref(!isReload);
const splashFading = ref(false);

function onSplashEnded() {
  splashFading.value = true;
  setTimeout(() => {
    showSplash.value = false;
  }, 600);
}

// Перехват F5 / Ctrl+R — сохраняем состояние перед перезагрузкой
async function handleReloadKey(e: KeyboardEvent) {
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

onMounted(() => {
  load();
  loadCustomCovers();
  applyUiScale();
  window.addEventListener('resize', applyUiScale);
  document.addEventListener('keydown', handleReloadKey);
  document.addEventListener('contextmenu', handleContextMenu);
});

onUnmounted(() => {
  window.removeEventListener('resize', applyUiScale);
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
      <a v-if="settings.show_github_badge" href="https://github.com/mel1x/LFMusic" target="_blank" rel="noopener noreferrer"><GithubIcon/>Github</a>
      <button class="controlsButton" @click="minimizeWindow"><MinimizeIcon /></button>
      <button class="controlsButton" @click="maximizeWindow"><MaximizeIcon /></button>
      <button class="controlsButton close" @click="closeWindow"><CloseIcon /></button>
    </div>
  </div>
  <MainBlock class="mainFrame" />
  <Player class="playerPart" />
  <TrackInfoModal />
  <TrackEditModal />
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
</style>
