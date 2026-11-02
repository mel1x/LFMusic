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
import OnboardingTour from './components/OnboardingTour.vue';
import HolidayPattern from './components/HolidayPattern.vue';
import HolidaySnow from './components/HolidaySnow.vue';
import HolidayModal from './components/HolidayModal.vue';
import YearRecap from './components/recap/YearRecap.vue';

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
import { useHoliday } from './composables/useHoliday';
import { isAndroidRuntime, isPhoneViewport } from './composables/usePlatform';
import splashVideoWebm from './assets/splash_screen_dark.webm';
import splashVideoMp4 from './assets/splash_screen_dark.mp4';

const win = getCurrentWindow();
const { load } = useSettings();
const { language } = useI18n();
const { currentTrack, isPlaying, saveState } = usePlayer();
const { loadCustomCovers } = useCustomCovers();
const { loadCachedAvatars } = useExternalSources();

useUiScale();
useAccentTheme();
useTaskbar();
useAndroidShell();
const { activeHoliday } = useHoliday();

const isReload = sessionStorage.getItem('lfmusic_reloaded') === '1';
if (isReload) sessionStorage.removeItem('lfmusic_reloaded');

const showSplash = ref(!isReload && !isPhoneViewport());
const splashFading = ref(false);

// The overlay is opaque and sits above everything, so it must never depend on
// `ended` alone: if WebView2 fails to load, start or finish the video, the app
// would stay a black window forever. The fallback timer covers that.
let splashTimer: ReturnType<typeof setTimeout> | null = null;

function armSplashTimer(ms: number) {
  if (splashTimer) clearTimeout(splashTimer);
  splashTimer = setTimeout(onSplashEnded, ms);
}

function onSplashEnded() {
  if (splashTimer) clearTimeout(splashTimer);
  splashTimer = null;
  if (splashFading.value || !showSplash.value) return;
  splashFading.value = true;
  setTimeout(() => {
    showSplash.value = false;
  }, 600);
}

// Once the video actually plays, wait for its remaining length plus a margin.
function onSplashPlaying(e: Event) {
  const video = e.target as HTMLVideoElement;
  const left = Number.isFinite(video.duration) ? video.duration - video.currentTime : 3;
  armSplashTimer(Math.max(0, left) * 1000 + 1500);
}

if (showSplash.value) armSplashTimer(4000);

// Escape for the "tracks ready" modal and cover overlay is handled inside
// TracksReadyModal / FullscreenCover themselves.
async function handleReloadKey(e: KeyboardEvent) {
  // Suppress Windows' Alt menu-mode (incl. Alt+Shift layout switch): otherwise
  // the window enters menu mode and the next keystroke is eaten as a menu
  // mnemonic (system beep, first letter lost). Safe since there's no window
  // menu (decorations: false); system Alt+F4/Alt+Tab are handled by the OS
  // regardless.
  if (e.key === 'Alt') {
    e.preventDefault();
    return;
  }

  // F11 toggles fullscreen (browser-like). Rust side un-maximizes on enter
  // (else Windows breaks the maximized->fullscreen transition) and restores
  // maximized on exit.
  if (e.key === 'F11') {
    e.preventDefault();
    if (isAndroidRuntime()) return;
    try {
      await invoke('toggle_window_fullscreen');
    } catch {
      // No-op on platforms without fullscreen support.
    }
    return;
  }

  if (e.key === 'F5' || (e.ctrlKey && e.key === 'r')) {
    e.preventDefault();
    await saveState();
    location.reload();
  }
}

// Blocks the browser's default context menu (has "Reload").
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
    <video data-tauri-drag-region autoplay muted playsinline @playing="onSplashPlaying" @ended="onSplashEnded" @error="onSplashEnded" class="splash-video">
      <source :src="splashVideoWebm" type="video/webm" />
      <source :src="splashVideoMp4" type="video/mp4" />
    </video>
  </div>
  <HolidayPattern />
  <TitleBar />
  <MainBlock :key="`main-${language}`" class="mainFrame" />
  <Player :key="`player-${language}`" />
  <TrackInfoModal :key="`info-${language}`" />
  <TrackEditModal :key="`edit-${language}`" />
  <AndroidPermissionWarning />
  <TracksReadyModal />
  <FullscreenCover />
  <OnboardingTour />
  <HolidayModal />
  <YearRecap />
  <HolidaySnow v-if="activeHoliday === 'new_year' && !showSplash" />
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
  /* Don't set transform/transition here: vue-sonner controls them on
     [data-sonner-toast] itself (transform: var(--y) for slide/stack, transition
     with height for the entry animation). Overriding them here (same
     specificity, but this rule comes later) would break the toast's height
     and entry animation. */
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
  /* Backgrounds reuse the accent's saturation (capped) so --accent-color /
     --dark-bg / --light-bg all stack visually as the same hue family. */
  --dark-bg: hsl(var(--accent-h), min(var(--accent-s), 50%), 8%);
  --light-bg: hsla(var(--accent-h), min(var(--accent-s), 45%), 15%, 1);
  /* Separate "fast" accent for the player bar: same values, but animated on
     its own (shorter) timing in useAccentTheme so the player leads the color
     change. Falls back to the shared accent before the first animation frame. */
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
  /* Own stacking context so the holiday pattern (z-index: -1) sits above the
     body background but under every app surface. */
  isolation: isolate;
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

  /* Pages without a cover header (.mobilePage on .pageRoot): room for the fixed
     MobileScrollHeader bar on top, catalog side gutters, big title under the bar. */
  body.lfmusic-phone .pageRoot.mobilePage {
    height: 100%;
    padding: 56px 14px 28px !important;
    overflow-y: auto !important;
    overflow-x: hidden !important;
  }

  body.lfmusic-phone .mobilePage .pageTitle {
    margin-top: 4px !important;
  }

  body.lfmusic-phone .mobilePage .lineList {
    width: calc(100% + 28px) !important;
    margin-left: -14px !important;
    margin-right: -14px !important;
    padding: 0 14px 12px !important;
    scroll-padding-left: 14px;
  }

  body.lfmusic-phone .mobilePage .grid {
    grid-template-columns: repeat(2, minmax(0, 1fr)) !important;
    gap: 18px 12px !important;
  }

  body.lfmusic-phone .mobilePage .grid .playlistItem,
  body.lfmusic-phone .mobilePage .grid .createPlaylistItem {
    width: auto !important;
    min-width: 0 !important;
  }

  /* Full-bleed cover header (catalog style) for pages with .mobileHeroPage on .pageRoot. */
  body.lfmusic-phone .mobileHeroPage {
    padding: 0 14px !important;
    overflow-y: auto !important;
    overflow-x: hidden !important;
  }

  body.lfmusic-phone .mobileHeroPage .scrollHeader {
    --hero-cover-height: clamp(290px, 86vw, 390px);
    --hero-header-overlap: 56px;
    position: relative;
    isolation: isolate;
    display: flex;
    flex-direction: column;
    align-items: center;
    width: 100%;
  }

  body.lfmusic-phone .mobileHeroPage .scrollHeader::after {
    content: "";
    position: absolute;
    left: -14px;
    right: -14px;
    top: calc(var(--hero-cover-height) - 18px);
    bottom: 0;
    z-index: 0;
    pointer-events: none;
    background:
        linear-gradient(
            to bottom,
            var(--light-bg) 0%,
            hsla(var(--accent-h), min(var(--accent-s), 52%), 15%, 0.42) 42%,
            hsla(var(--accent-h), min(var(--accent-s), 48%), 14%, 0.22) 70%,
            var(--light-bg) 100%
        );
  }

  body.lfmusic-phone .mobileHeroPage .pageHat {
    position: relative;
    z-index: 1;
    align-items: center !important;
    justify-content: flex-start !important;
    width: calc(100% + 28px) !important;
    min-height: calc(var(--hero-cover-height) + var(--hero-header-overlap));
    margin: calc(var(--hero-header-overlap) * -1) -14px 18px !important;
    gap: 0 !important;
    overflow: hidden;
    border-radius: 0 !important;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .coverWrap {
    width: 100% !important;
    height: calc(var(--hero-cover-height) + var(--hero-header-overlap)) !important;
    overflow: hidden;
    border-radius: 0 !important;
    -webkit-mask-image: linear-gradient(to bottom, #000 0%, #000 72%, transparent 100%);
    mask-image: linear-gradient(to bottom, #000 0%, #000 72%, transparent 100%);
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .cover,
  body.lfmusic-phone .mobileHeroPage .pageHat .cover img {
    width: 100% !important;
    height: 100% !important;
    border-radius: 0 !important;
    object-fit: cover;
    filter: saturate(1.05) brightness(0.86);
    transform: none !important;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .coverWrap::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    bottom: 0;
    z-index: 2;
    pointer-events: none;
    background:
        radial-gradient(circle 180px at 50% 84%, hsla(var(--accent-h), min(var(--accent-s), 70%), 58%, 0.24), transparent),
        linear-gradient(
            to bottom,
            hsla(var(--accent-h), min(var(--accent-s), 50%), 10%, 0) 0%,
            hsla(var(--accent-h), min(var(--accent-s), 50%), 10%, 0) 26%,
            hsla(var(--accent-h), min(var(--accent-s), 58%), 14%, 0.12) 54%,
            hsla(var(--accent-h), min(var(--accent-s), 52%), 15%, 0.38) 81%,
            hsla(var(--accent-h), min(var(--accent-s), 52%), 15%, 0.5) 100%
        );
  }

  body.lfmusic-phone .mobileHeroPage .coverEditButtons {
    display: none !important;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone {
    position: absolute;
    top: auto !important;
    bottom: 20px !important;
    left: 50% !important;
    z-index: 3;
    width: min(270px, calc(100% - 36px)) !important;
    gap: 12px !important;
    align-items: center !important;
    transform: translateX(-50%) !important;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone,
  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .info {
    min-width: 0 !important;
    text-align: center !important;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .info {
    align-items: center !important;
    margin-top: 0 !important;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .info .listType {
    margin-left: 0;
    font-size: 11px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.56);
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .info .listName {
    max-width: 100%;
    margin-top: 1px;
    font-size: 28px;
    line-height: 1.04;
    white-space: normal;
    overflow-wrap: anywhere;
    color: hsla(0, 0%, 100%, 0.96);
    text-shadow: 0 2px 18px rgba(0, 0, 0, 0.28);
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .info .badges {
    justify-content: center;
    margin-top: 9px;
    gap: 6px !important;
  }

  body.lfmusic-phone .mobileHeroPage .badge {
    padding: 3px 9px;
    font-size: 11px;
    background-color: hsla(0, 0%, 100%, 0.1);
    color: hsla(0, 0%, 100%, 0.72);
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls {
    justify-content: center;
    gap: 10px !important;
    margin-top: 0;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls .playListBtn,
  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls .moreOptionsBtn,
  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls .moreBtn {
    width: 46px !important;
    height: 46px !important;
    min-width: 46px !important;
    min-height: 46px !important;
    padding: 0 !important;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.22);
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls .playListBtn {
    background-color: hsla(0, 0%, 100%, 0.94);
    color: hsla(var(--accent-h), min(var(--accent-s), 62%), 15%, 1);
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls .moreOptionsBtn,
  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls .moreBtn {
    flex-direction: row;
    background-color: hsla(0, 0%, 100%, 0.12);
    color: hsla(0, 0%, 100%, 0.94);
    backdrop-filter: blur(12px);
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls .playListBtn:hover {
    background-color: hsla(0, 0%, 100%, 1);
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls .moreOptionsBtn:hover,
  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls .moreBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.18);
  }

  body.lfmusic-phone .mobileHeroPage .playListText {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls svg {
    width: 18px !important;
    height: 18px !important;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .controls .playListBtn:disabled {
    opacity: 0.45;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .cover {
    margin: 0 !important;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .info .listNameRow {
    justify-content: center;
    max-width: 100%;
  }

  body.lfmusic-phone .mobileHeroPage .pageHat .dataZone .info .editNameBtn {
    display: none;
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

  /* Cards on phones: only a small play button stays on the cover; pin and the menu
     move to a long press (the cover's contextmenu handler opens the same menu). */
  body.lfmusic-phone .playlistItem .pinBtn,
  body.lfmusic-phone .playlistItem .moreBtn,
  body.lfmusic-phone .playlistItem .cardMore,
  body.lfmusic-phone .artistItem .pinBtn,
  body.lfmusic-phone .artistItem .moreBtn {
    display: none !important;
  }

  body.lfmusic-phone .playlistItem .playBtn,
  body.lfmusic-phone .artistItem .playBtn {
    left: auto !important;
    right: 8px !important;
    bottom: 8px !important;
    width: 34px !important;
    height: 34px !important;
    min-width: 34px !important;
    min-height: 34px !important;
    padding: 0 !important;
    opacity: 1 !important;
    transform: none !important;
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.28);
  }

  body.lfmusic-phone .artistItem .playBtn {
    right: 4% !important;
    bottom: 4% !important;
  }

  body.lfmusic-phone .playlistItem .playBtn:disabled {
    display: none !important;
  }

  body.lfmusic-phone .playlistItem .coverWrap,
  body.lfmusic-phone .playlistItem .cover,
  body.lfmusic-phone .createPlaylistItem .cover {
    border-radius: 14px !important;
  }

  /* No hover dimming on touch: it sticks after a tap. */
  body.lfmusic-phone .playlistItem .cover,
  body.lfmusic-phone .artistItem .artistCover {
    filter: none !important;
  }

  body.lfmusic-phone .playlistItem .title,
  body.lfmusic-phone .playlistItem .subtitle {
    margin-left: 2px !important;
  }

  body.lfmusic-phone .playlistItem .title {
    font-size: 13px !important;
    color: hsla(0, 0%, 100%, 0.9) !important;
  }

  body.lfmusic-phone .artistItem .artistTitle {
    font-size: 13px !important;
  }

  /* Section rows (Collections, Artist page, Search): compact heading, no subtitle. */
  body.lfmusic-phone .mobilePage .pageLine,
  body.lfmusic-phone .mobileHeroPage .pageLine {
    margin-bottom: 26px !important;
  }

  body.lfmusic-phone .lineTitle {
    font-size: 20px !important;
  }

  body.lfmusic-phone .lineDesc {
    display: none !important;
  }

  body.lfmusic-phone .lineHat {
    margin-bottom: 10px !important;
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

  body.lfmusic-phone .catalogPage .trackElem,
  body.lfmusic-phone .mobileHeroPage .trackElem {
    min-height: 54px !important;
    padding: 5px 46px 5px 5px !important;
    border-radius: 10px !important;
    gap: 10px !important;
  }

  body.lfmusic-phone .catalogPage .trackCover,
  body.lfmusic-phone .mobileHeroPage .trackCover {
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

  /* Playing dot (16px) centered on the row cover: padding + cover / 2 - 8. */
  body.lfmusic-phone .trackElem .playingAnim {
    left: 27px !important;
    top: 50% !important;
    margin-top: -8px !important;
  }

  body.lfmusic-phone .catalogPage .trackElem .playingAnim,
  body.lfmusic-phone .mobileHeroPage .trackElem .playingAnim {
    left: 19px !important;
  }

  body.lfmusic-phone .searchPage .trackElem .playingAnim {
    left: 17px !important;
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
