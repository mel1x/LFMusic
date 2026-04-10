<script setup lang="ts">
import { onMounted, onUnmounted, watchEffect } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import MaximizeIcon from './components/Icons/MaximizeIcon.vue';
import MinimizeIcon from './components/Icons/MinimizeIcon.vue';
import CloseIcon from './components/Icons/CloseIcon.vue';

import MainBlock from './components/MainBlock.vue';
import Player from './components/Player.vue';
import GithubIcon from './components/Icons/GithubIcon.vue';
import { useSettings } from './composables/useSettings';
import { usePlayer } from './composables/usePlayer';

const win = getCurrentWindow();
const { settings, load } = useSettings();
const { saveState, currentTrack, currentAccentColor } = usePlayer();

// Перехват F5 / Ctrl+R — сохраняем состояние перед перезагрузкой
async function handleReloadKey(e: KeyboardEvent) {
  if (e.key === 'F5' || (e.ctrlKey && e.key === 'r')) {
    e.preventDefault();
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
  document.addEventListener('keydown', handleReloadKey);
  document.addEventListener('contextmenu', handleContextMenu);
});

onUnmounted(() => {
  document.removeEventListener('keydown', handleReloadKey);
  document.removeEventListener('contextmenu', handleContextMenu);
});

watchEffect(() => {
  const hex = currentAccentColor.value ?? '#1a1a1a';
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
</template>

<style>
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
  --dark-bg: hsl(var(--accent-h), var(--accent-s), 8%);
  --light-bg: hsla(var(--accent-h), calc(var(--accent-s) * 1.25), 15%, 1);

  background-color: var(--dark-bg);
  transition: .24s ease;
}
body {
  background-color: var(--dark-bg);
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
.mainFrame {
  width: 100%;
  height: calc(100vh - 38px - 104px);
}
.playerPart {
  width: 100%;
  height: 104px;
}
</style>
