<script setup lang="ts">
import { ref, onMounted, onUnmounted, watchEffect } from 'vue';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';

import MainBlock from './components/MainBlock.vue';
import Player from './components/Player.vue';
import TitleBar from './components/TitleBar.vue';
import TrackInfoModal from './components/TrackInfoModal.vue';
import TrackEditModal from './components/TrackEditModal.vue';
import AndroidPermissionWarning from './components/AndroidPermissionWarning.vue';
import TracksReadyModal from './components/TracksReadyModal.vue';
import FullscreenCover from './components/FullscreenCover.vue';

import { useSettings } from './composables/useSettings';
import { usePlayer } from './composables/usePlayer';
import { useCustomCovers } from './composables/useCustomCovers';
import { useExternalSources } from './composables/useExternalSources';
import { startDomI18n } from './composables/useDomI18n';
import { useI18n } from './composables/useI18n';
import { useUiScale } from './composables/useUiScale';
import { useAccentTheme } from './composables/useAccentTheme';
import { useTaskbar } from './composables/useTaskbar';
import { useAndroidShell } from './composables/useAndroidShell';
import { isAndroidRuntime, isPhoneViewport } from './composables/usePlatform';
import splashVideoWebm from './assets/splash_screen_dark.webm';
import splashVideoMp4 from './assets/splash_screen_dark.mp4';

const win = getCurrentWindow();
const { load } = useSettings();
const { language } = useI18n();
const { currentTrack, isPlaying, saveState } = usePlayer();
const { loadCustomCovers } = useCustomCovers();
const { loadCachedAvatars } = useExternalSources();

// Кросс-секционные системы вынесены в composables (поведение без изменений):
// масштабирование UI, динамическое темирование, панель задач, Android-оболочка.
useUiScale();
useAccentTheme();
useTaskbar();
useAndroidShell();

// ── Splash ──────────────────────────────────────────────────────────────────
const isReload = sessionStorage.getItem('lfmusic_reloaded') === '1';
if (isReload) sessionStorage.removeItem('lfmusic_reloaded');

const showSplash = ref(!isReload && !isPhoneViewport());
const splashFading = ref(false);

function onSplashEnded() {
  splashFading.value = true;
  setTimeout(() => {
    showSplash.value = false;
  }, 600);
}

// ── Глобальные клавиши / блокировка контекстного меню ─────────────────────────
// Перехват F5 / Ctrl+R — сохраняем состояние перед перезагрузкой.
// (Escape для модалок «треки готовы» и оверлея обложки теперь обрабатывается
// в самих компонентах TracksReadyModal / FullscreenCover.)
async function handleReloadKey(e: KeyboardEvent) {
  // Гасим вход Windows в «меню-режим» по Alt (в т.ч. Alt+Shift при смене
  // раскладки клавиатуры). Иначе окно входит в menu mode, и следующая клавиша
  // трактуется как мнемоника меню: символ съедается, играет системный «бип» и
  // первая буква не печатается. Меню у окна нет (decorations: false), поэтому
  // блокировать дефолт Alt безопасно — системные Alt+F4/Alt+Tab обрабатываются
  // ОС по своим клавишам и этим не ломаются.
  if (e.key === 'Alt') {
    e.preventDefault();
    return;
  }

  // F11 — переключение полноэкранного режима окна (как в браузере).
  // Логика на стороне Rust: при входе снимает maximized (иначе на Windows
  // переход maximized→fullscreen ломается) и восстанавливает его при выходе.
  if (e.key === 'F11') {
    e.preventDefault();
    if (isAndroidRuntime()) return;
    try {
      await invoke('toggle_window_fullscreen');
    } catch {
      // Игнорируем — на платформах без поддержки fullscreen просто ничего не делаем.
    }
    return;
  }

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
  startDomI18n();
  loadCustomCovers();
  loadCachedAvatars();
  document.addEventListener('keydown', handleReloadKey);
  document.addEventListener('contextmenu', handleContextMenu);
});

onUnmounted(() => {
  document.removeEventListener('keydown', handleReloadKey);
  document.removeEventListener('contextmenu', handleContextMenu);
});

// Заголовок окна: текущий трек во время воспроизведения, иначе «LFMusic».
watchEffect(() => {
  const track = currentTrack.value;
  let title = 'LFMusic';
  if (isPlaying.value && track) {
    const name = track.title || (track.file_path.split(/[\\/]/).pop()?.replace(/\.[^/.]+$/, '') ?? '');
    title = track.artist ? `${name} - ${track.artist}` : name;
  }
  win.setTitle(title);
});
</script>

<template>
  <div data-tauri-drag-region v-if="showSplash" class="splash-overlay" :class="{ 'splash-fading': splashFading }">
    <video data-tauri-drag-region autoplay muted playsinline @ended="onSplashEnded" class="splash-video">
      <source :src="splashVideoWebm" type="video/webm" />
      <source :src="splashVideoMp4" type="video/mp4" />
    </video>
  </div>
  <TitleBar />
  <MainBlock :key="`main-${language}`" class="mainFrame" />
  <Player :key="`player-${language}`" />
  <TrackInfoModal :key="`info-${language}`" />
  <TrackEditModal :key="`edit-${language}`" />
  <AndroidPermissionWarning />
  <TracksReadyModal />
  <FullscreenCover />
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
  width: 286px;
  background-color: var(--dark-bg);
  border: 2px solid #ffffff20;
  border-radius: 9999px;
  padding: 6px;
  color: #fff;
  font-size: 13px;
  font-weight: 500;
  cursor: default;
  user-select: none;
  /* НЕ задаём здесь transform / transition: ими управляет сам vue-sonner на
     элементе [data-sonner-toast] (transform: var(--y) — выезд/стек, transition с
     height — анимация появления). Свой transform/transition тут перекрыл бы их
     (равная специфичность, но это правило идёт позже) и ломал бы высоту и
     анимацию появления тоста. */
}
.toast [data-close-button] {
  order: 99;
  margin-left: auto;
  margin-right: 0;
  width: 30px;
  height: 30px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent !important;
  color: #ffffffa0;
  cursor: pointer;
  border: none;
  outline: none;
  padding: 0;
  transition: background-color 0.2s ease, color 0.2s ease;
  flex-shrink: 0;
}
.toast [data-close-button]:hover {
  background: #ffffff20 !important;
  color: #ffffff;
}
.toast [data-close-button] svg,
.toast [data-close-button] svg * {
  width: 14px;
  height: 14px;
  stroke-width: 2px !important;
  will-change: transform;
  backface-visibility: hidden;
  transform: translate3d(0, 0, 0);
}
[data-sonner-toaster][data-x-position="center"] {
  left: calc(50% + 110px) !important;
  width: 286px !important;
}
body:has(.fullscreen-overlay) [data-sonner-toaster][data-x-position="center"] {
  left: 50% !important;
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
  /* Отдельный «быстрый» акцент для блока плеера: те же значения, но анимируются
     по своему (более короткому) таймингу в useAccentTheme, чтобы плеер менялся
     первым. Фолбэк на общий акцент — на случай до первого кадра анимации. */
  --player-accent-h: var(--accent-h);
  --player-accent-s: var(--accent-s);
  --player-accent-l: var(--accent-l);
  --player-light-bg: hsla(var(--player-accent-h), min(var(--player-accent-s), 45%), 15%, 1);
  --android-safe-top: 0px;
  --android-safe-right: 0px;
  --android-safe-bottom: 0px;
  --android-safe-left: 0px;
  --safe-top: var(--android-safe-top);
  --safe-right: var(--android-safe-right);
  --safe-bottom: var(--android-safe-bottom);
  --safe-left: var(--android-safe-left);
  --mobile-chrome-color: #101010;
  --mobile-chrome-bg: var(--mobile-chrome-color);
  --mobile-player-height: 84px;
  --mobile-nav-height: calc(68px + var(--safe-bottom));

  background: var(--dark-bg) !important;
  overflow: hidden;
  transition: .24s ease;
}
@supports (padding-top: max(0px, 1px)) {
  html {
    --safe-top: max(env(safe-area-inset-top, 0px), var(--android-safe-top));
    --safe-right: max(env(safe-area-inset-right, 0px), var(--android-safe-right));
    --safe-bottom: max(env(safe-area-inset-bottom, 0px), var(--android-safe-bottom));
    --safe-left: max(env(safe-area-inset-left, 0px), var(--android-safe-left));
  }
}
html::selection {
  color: #fff;
  background-color: var(--dark-bg);
}
body {
  background: var(--dark-bg) !important;
  overflow: hidden;
  transition: .24s ease;
}
#app {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  width: 100%;
  height: 100%;
}
.mainFrame {
  width: 100%;
  flex: 1;
  min-height: 0;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
  html,
  body {
    width: 100vw !important;
    height: 100dvh !important;
    min-width: 100vw !important;
    min-height: 100dvh !important;
    margin: 0 !important;
    padding: 0 !important;
    border: 0 !important;
    overflow: hidden !important;
  }

  body {
    position: fixed;
    inset: 0;
    touch-action: manipulation;
    -webkit-tap-highlight-color: transparent;
  }

  html.lfmusic-phone:not(.lfmusic-android),
  body.lfmusic-phone:not(.lfmusic-android) {
    --safe-right: 0px;
    --safe-bottom: 0px;
    --safe-left: 0px;
  }

  body.lfmusic-phone,
  body.lfmusic-phone * {
    scrollbar-width: none !important;
    -ms-overflow-style: none !important;
  }

  body.lfmusic-phone::-webkit-scrollbar,
  body.lfmusic-phone *::-webkit-scrollbar {
    width: 0 !important;
    height: 0 !important;
    display: none !important;
  }

  #app {
    position: fixed;
    top: var(--safe-top);
    right: 0;
    bottom: 0;
    left: 0;
    width: 100vw;
    height: calc(100dvh - var(--safe-top));
    margin: 0 !important;
    padding: 0;
    background: var(--dark-bg);
  }

  .hat {
    display: none !important;
  }

  .mainFrame {
    flex: 1 1 auto;
    height: auto;
    min-height: 0;
  }

  .splash-overlay,
  .scanReadyBackdrop {
    width: 100vw;
    height: 100dvh;
  }

  .scanReadyBackdrop,
  body.lfmusic-phone .modalBackdrop {
    width: 100vw !important;
    height: 100dvh !important;
    align-items: flex-end !important;
    justify-content: center !important;
  }

  .scanReadyModal,
  body.lfmusic-phone .modalCard {
    width: 100% !important;
    max-width: none !important;
    max-height: calc(100dvh - var(--safe-top) - 12px) !important;
    border-radius: 24px 24px 0 0 !important;
    overflow-y: auto !important;
  }

  .scanReadyModal {
    padding: 22px 18px calc(18px + var(--safe-bottom));
  }

  .scanReadyButton,
  body.lfmusic-phone .modalBtn,
  body.lfmusic-phone .btn {
    min-height: 44px !important;
    padding: 10px 16px !important;
  }

  .toast {
    width: min(360px, calc(100vw - 24px));
    border-radius: 18px;
  }

  [data-sonner-toaster][data-x-position="center"] {
    left: 50% !important;
    width: min(360px, calc(100vw - 24px)) !important;
  }

  .androidPermissionWarning {
    align-items: flex-end;
    padding: calc(var(--safe-top) + 14px) calc(var(--safe-right) + 12px) calc(var(--safe-bottom) + 12px) calc(var(--safe-left) + 12px);
  }

  .androidPermissionCard {
    width: 100%;
    border-radius: 24px;
    padding: 20px;
  }

  body.lfmusic-phone .pageRoot {
    min-height: 100% !important;
    padding: 0;
    overflow-y: auto !important;
    overflow-x: hidden !important;
    background-attachment: local !important;
  }

  body.lfmusic-phone .pageTitle {
    margin-bottom: 22px !important;
    font-size: 30px !important;
    line-height: 1.08 !important;
  }

  body.lfmusic-phone .pageHat {
    flex-direction: column !important;
    align-items: flex-start !important;
    gap: 16px !important;
    margin-bottom: 24px !important;
    position: relative;
  }

  body.lfmusic-phone .pageHat .coverWrap {
    width: 100%;
    height: 100%;
    border-radius: 0;

    &:hover {
      transform: scale(1) !important;
    }
  }

  body.lfmusic-phone .pageHat .cover,
  body.lfmusic-phone .pageHat .cover img {
    width: 100% !important;
    height: 100% !important;
    border-radius: 0;

    &:hover {
      transform: scale(1) !important;
    }
  }
  body.lfmusic-phone .pageHat .coverWrap::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: -40px;

    height: 160px;

    background: linear-gradient(
      to bottom,
      transparent 0%,
      var(--light-bg) 60%
    );

    pointer-events: none;
  }

  body.lfmusic-phone .pageHat .dataZone {
    position: absolute;
    top: 65% !important;
    left: 50% !important;
    transform: translate(-50%, -50%) !important;
  }

  body.lfmusic-phone .pageHat .dataZone,
  body.lfmusic-phone .pageHat .dataZone .info {
    width: 100% !important;
    min-width: 0 !important;
    text-align: center !important;
    align-self: center !important;
    justify-content: center !important;

  }

  body.lfmusic-phone .pageHat .dataZone .info .listTitle,
  body.lfmusic-phone .pageHat .dataZone .info .listTitle h1 {
    max-width: 100% !important;
    white-space: normal !important;
    overflow-wrap: anywhere !important;
  }

  body.lfmusic-phone .pageHat .dataZone .info .listTitle h1 {
    font-size: 32px !important;
    line-height: 1.08 !important;
  }

  body.lfmusic-phone .pageHat .dataZone .info .badges,
  body.lfmusic-phone .pageHat .dataZone .controls {
    flex-wrap: wrap !important;
    gap: 8px !important;
  }

  body.lfmusic-phone .pageHat .dataZone .controls button,
  body.lfmusic-phone .playListBtn,
  body.lfmusic-phone .pinBtn,
  body.lfmusic-phone .moreBtn,
  body.lfmusic-phone .cardMore,
  body.lfmusic-phone .overlayBtn {
    min-width: 42px !important;
    min-height: 42px !important;
  }

  body.lfmusic-phone .searchZone {
    width: 100% !important;
    height: 46px !important;
    margin: 0 0 16px !important;
    border-radius: 16px !important;
  }

  body.lfmusic-phone .search {
    min-width: 0 !important;
    font-size: 15px !important;
  }

  body.lfmusic-phone .stickyHeader {
    top: 0 !important;
    margin: 0 -14px 12px !important;
    padding-left: 14px !important;
    padding-right: 14px !important;
  }

  body.lfmusic-phone .lineList {
    width: calc(100vw - 28px) !important;
    overflow-x: auto !important;
    scroll-snap-type: x proximity;
    padding-bottom: 12px !important;
    --card-w: 132px !important;
  }

  body.lfmusic-phone .playlistItem {
    width: var(--card-w, 132px) !important;
    min-width: var(--card-w, 132px) !important;
    scroll-snap-align: start;
  }

  body.lfmusic-phone .playlistItem .overlayBtn,
  body.lfmusic-phone .playlistItem .cardMore,
  body.lfmusic-phone .playlistItem .moreBtn,
  body.lfmusic-phone .playlistItem .pinBtn {
    opacity: 1 !important;
  }

  body.lfmusic-phone .trackElem {
    min-height: 60px !important;
    padding: 7px 46px 7px 7px !important;
    border-radius: 12px !important;
    gap: 9px !important;
  }

  body.lfmusic-phone .trackCover,
  body.lfmusic-phone .trackCoverWrap,
  body.lfmusic-phone .trackElem .coverWrap {
    border-radius: 12px !important;
    width: 56px !important;
    height: 56px !important;
    flex-shrink: 0 !important;
  }

  body.lfmusic-phone .catalogPage .trackElem {
    min-height: 54px !important;
    padding: 5px 46px 5px 5px !important;
    border-radius: 10px !important;
    gap: 10px !important;
  }

  body.lfmusic-phone .catalogPage .trackCover {
    border-radius: 6px !important;
    width: 44px !important;
    height: 44px !important;
  }

  body.lfmusic-phone .trackName,
  body.lfmusic-phone .trackArtist,
  body.lfmusic-phone .playlistItem .title,
  body.lfmusic-phone .playlistItem .subtitle {
    max-width: 100% !important;
    min-width: 0 !important;
  }

  body.lfmusic-phone .trackTime {
    display: none !important;
  }

  body.lfmusic-phone .hoverOptionsBtn,
  body.lfmusic-phone .hoverOptions,
  body.lfmusic-phone .hoverIcon {
    opacity: 1 !important;
  }

  body.lfmusic-phone .hoverIcon {
    display: none !important;
  }

  body.lfmusic-phone .trackElem.activePaused .playingAnim {
    animation-play-state: paused !important;
    opacity: 0.82 !important;
  }

  body.lfmusic-phone .hoverOptionsBtn,
  body.lfmusic-phone .hoverOptions {
    right: 6px !important;
    width: 40px !important;
    height: 40px !important;
  }

  body.lfmusic-phone .emptyState,
  body.lfmusic-phone .emptyInfo {
    padding: 28px 12px !important;
  }

  body.lfmusic-phone .ctx-menu {
    left: 8px !important;
    right: 8px !important;
    top: auto !important;
    bottom: calc(var(--safe-bottom) + 10px) !important;
    width: auto !important;
    min-width: 0 !important;
    max-height: min(70dvh, 440px) !important;
    overflow-y: auto !important;
    border-radius: 20px !important;
    background-color: hsla(var(--accent-h), min(var(--accent-s), 48%), 12%, 0.96) !important;
    box-shadow: 0 24px 56px rgba(0, 0, 0, 0.45) !important;
  }

  body.lfmusic-phone .ctx-item {
    min-height: 46px !important;
    padding: 12px 14px !important;
    font-size: 14px !important;
  }

  body.lfmusic-phone .ctx-separator {
    margin: 6px 12px !important;
  }

  body.lfmusic-phone .ctx-submenu {
    left: 8px !important;
    right: 8px !important;
    top: auto !important;
    bottom: calc(var(--safe-bottom) + 10px) !important;
    width: auto !important;
  }

  body.lfmusic-phone .ctx-submenu-list {
    max-height: min(62dvh, 380px) !important;
    border-radius: 20px !important;
    background-color: hsla(var(--accent-h), min(var(--accent-s), 48%), 12%, 0.96) !important;
  }

  body.lfmusic-phone .modalActions,
  body.lfmusic-phone .footer {
    flex-wrap: wrap !important;
    gap: 8px !important;
  }

  body.lfmusic-phone .modalBtn,
  body.lfmusic-phone .footer .btn {
    flex: 1 1 auto !important;
  }

  body.lfmusic-phone .modalHeader,
  body.lfmusic-phone .editArea {
    flex-direction: column !important;
    align-items: stretch !important;
  }

  body.lfmusic-phone .sections {
    max-height: none !important;
  }

  body.lfmusic-phone .fieldRow {
    align-items: flex-start !important;
    flex-wrap: wrap !important;
  }

  body.lfmusic-phone .fieldLabel {
    width: auto !important;
  }

  body.lfmusic-phone .fieldValue {
    text-align: left !important;
    white-space: normal !important;
    overflow-wrap: anywhere !important;
  }
}
</style>
