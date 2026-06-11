<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted, watch, watchEffect } from 'vue';
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
import { toast } from 'vue-sonner';
import { isCoverResolved } from './composables/useCoverLoader';
import { useSettings } from './composables/useSettings';
import { usePlayer } from './composables/usePlayer';
import { useFolders } from './composables/useFolders';
import { useCustomCovers } from './composables/useCustomCovers';
import { startDomI18n } from './composables/useDomI18n';
import { useI18n } from './composables/useI18n';
import { isAndroidRuntime, isPhoneViewport, updateRuntimeClasses } from './composables/usePlatform';
import splashVideoWebm from './assets/splash_screen_dark.webm';
import splashVideoMp4 from './assets/splash_screen_dark.mp4';

const win = getCurrentWindow();
const { settings, load } = useSettings();
const { language, t } = useI18n();
const {
  saveState, currentTrack, currentAccentColor, isPlaying,
  togglePlayPause, playNext, playPrev, toggleShuffle, toggleRepeat,
  shuffled, repeatMode, playbackOrder, currentTrackIndex, getTrackByPath, loadCoversForTracks,
} = usePlayer();
const { showTracksReadyModal, dismissTracksReadyModal } = useFolders();
const { loadCustomCovers } = useCustomCovers();

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

interface AudioFeatureProgressPayload {
  current: number;
  total: number;
  percent: number;
  active: boolean;
}

interface AndroidPermissionStatus {
  mediaGranted: boolean;
  notificationsGranted: boolean;
  notificationsRequired: boolean;
}

interface AndroidSafeInsets {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

const BASE_W = 1460;
const BASE_H = 860;
const MIN_SCALE = 0.78;
const MAX_SCALE = 2.0;
let resizeRaf: number | null = null;
let unlistenAudioFeatureProgress: UnlistenFn | null = null;
let unlistenTaskbarMedia: UnlistenFn | null = null;
let featureProgressHideTimer: ReturnType<typeof setTimeout> | null = null;
let androidInsetsRetryTimer: ReturnType<typeof setTimeout> | null = null;
let androidInsetsRetryCount = 0;

function resetUiScale() {
  const body = document.body;
  (body.style as any).zoom = '';
  body.style.width = '100vw';
  body.style.height = '100dvh';
  document.documentElement.style.setProperty('--ui-scale', '1');
  (window as any).__uiScale = 1;
}

function applyUiScale() {
  resizeRaf = null;
  updateRuntimeClasses();

  if (isPhoneViewport()) {
    resetUiScale();
    return;
  }

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

const showSplash = ref(!isReload && !isPhoneViewport());
const splashFading = ref(false);

function onSplashEnded() {
  splashFading.value = true;
  setTimeout(() => {
    showSplash.value = false;
  }, 600);
}

const featureProgressVisible = ref(false);
const androidPermissionStatus = ref<AndroidPermissionStatus | null>(null);
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

const missingAndroidPermissions = computed(() => {
  const status = androidPermissionStatus.value;
  if (!isAndroidRuntime() || !status) return [];

  const missing: string[] = [];
  if (!status.mediaGranted) missing.push('доступ к медиафайлам');
  if (status.notificationsRequired && !status.notificationsGranted) missing.push('уведомления');
  return missing;
});

const showAndroidPermissionWarning = computed(() => missingAndroidPermissions.value.length > 0);

const androidPermissionWarningText = computed(() => {
  const missing = missingAndroidPermissions.value;
  if (missing.length === 0) return '';
  return `Не выданы разрешения: ${missing.join(', ')}. Без них LFMusic не сможет полноценно читать локальную музыку и показывать управление плеером в уведомлениях.`;
});

// Перехват F5 / Ctrl+R — сохраняем состояние перед перезагрузкой
async function handleReloadKey(e: KeyboardEvent) {
  if (e.key === 'Escape' && showTracksReadyModal.value) {
    e.preventDefault();
    dismissTracksReadyModal();
    return;
  }

  if (e.key === 'Escape' && fullscreenCoverSrc.value) {
    e.preventDefault();
    hideFullscreenCover();
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

function onTracksReadyBackdrop(e: MouseEvent) {
  if (e.target === e.currentTarget) dismissTracksReadyModal();
}

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

function applyAndroidPermissionStatus(status: AndroidPermissionStatus) {
  androidPermissionStatus.value = {
    mediaGranted: Boolean(status.mediaGranted),
    notificationsGranted: Boolean(status.notificationsGranted),
    notificationsRequired: Boolean(status.notificationsRequired),
  };
}

function refreshAndroidPermissions() {
  if (!isAndroidRuntime()) return;
  const raw = window.LFMusicAndroid?.getPermissionStatus?.();
  if (!raw) return;

  try {
    applyAndroidPermissionStatus(JSON.parse(raw) as AndroidPermissionStatus);
  } catch {
    androidPermissionStatus.value = null;
  }
}

function requestAndroidPermissions() {
  window.LFMusicAndroid?.requestRequiredPermissionsFromJs?.();
  setTimeout(refreshAndroidPermissions, 500);
}

function applyAndroidInsets(insets: AndroidSafeInsets) {
  const root = document.documentElement.style;
  root.setProperty('--android-safe-top', `${Math.max(0, insets.top || 0)}px`);
  root.setProperty('--android-safe-right', `${Math.max(0, insets.right || 0)}px`);
  root.setProperty('--android-safe-bottom', `${Math.max(0, insets.bottom || 0)}px`);
  root.setProperty('--android-safe-left', `${Math.max(0, insets.left || 0)}px`);
}

function refreshAndroidInsets() {
  if (!isAndroidRuntime()) return;
  const raw = window.LFMusicAndroid?.getSystemInsets?.();
  if (!raw) return;

  try {
    applyAndroidInsets(JSON.parse(raw) as AndroidSafeInsets);
  } catch {
    // Native inset updates are best-effort; CSS env() and fallback values stay in place.
  }
}

function scheduleAndroidInsetsRefresh() {
  if (!isAndroidRuntime()) return;
  refreshAndroidInsets();
  if (androidInsetsRetryCount >= 12) return;
  androidInsetsRetryCount += 1;
  if (androidInsetsRetryTimer) clearTimeout(androidInsetsRetryTimer);
  androidInsetsRetryTimer = setTimeout(scheduleAndroidInsetsRefresh, androidInsetsRetryCount < 5 ? 120 : 350);
}

function onAndroidPermissionsEvent(event: Event) {
  const detail = (event as CustomEvent<AndroidPermissionStatus>).detail;
  if (!detail) return;
  applyAndroidPermissionStatus(detail);
}

function onAndroidInsetsEvent(event: Event) {
  const detail = (event as CustomEvent<AndroidSafeInsets>).detail;
  if (detail) applyAndroidInsets(detail);
  refreshAndroidInsets();
}

function onVisibilityChange() {
  if (document.visibilityState === 'visible') {
    refreshAndroidPermissions();
    scheduleAndroidInsetsRefresh();
  }
}

function onWindowFocus() {
  refreshAndroidPermissions();
  scheduleAndroidInsetsRefresh();
}

onMounted(() => {
  load();
  startDomI18n();
  loadCustomCovers();
  applyUiScale();
  refreshAndroidPermissions();
  scheduleAndroidInsetsRefresh();
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
  }).catch(() => {
    // Продолжение фоновой подготовки не должно мешать запуску интерфейса.
  });
  window.addEventListener('resize', scheduleUiScale);
  window.addEventListener('focus', onWindowFocus);
  window.addEventListener('lfmusic:android-permissions', onAndroidPermissionsEvent as EventListener);
  window.addEventListener('lfmusic:android-insets', onAndroidInsetsEvent as EventListener);
  window.addEventListener('lfmusic:show-fullscreen-cover', handleFullscreenCoverEvent as EventListener);
  window.visualViewport?.addEventListener('resize', scheduleUiScale);
  window.visualViewport?.addEventListener('resize', scheduleAndroidInsetsRefresh);
  document.addEventListener('keydown', handleReloadKey);
  document.addEventListener('contextmenu', handleContextMenu);
  document.addEventListener('visibilitychange', onVisibilityChange);

  // Кнопки-миниатюры панели задач (Windows) → действия плеера.
  // Для shuffle/repeat показываем те же тосты, что и кнопки в плеере.
  void listen<string>('taskbar_media', (event) => {
    switch (event.payload) {
      case 'shuffle':
        toggleShuffle();
        toast.info(shuffled.value ? 'Случайный порядок' : 'Воспроизведение подряд');
        break;
      case 'prev': playPrev(); break;
      case 'playpause': togglePlayPause(); break;
      case 'next': playNext(); break;
      case 'repeat':
        toggleRepeat();
        toast.warning(repeatMode.value === 'track' ? 'Повтор трека' : 'Повтор плейлиста');
        break;
    }
  }).then((unlisten) => { unlistenTaskbarMedia = unlisten; }).catch(() => {});
});

// Соседние треки в очереди (свежие объекты из trackMap — учёт ленивой подгрузки).
// Очередь закольцована: кнопки prev/next на краях оборачиваются (см. playPrev/playNext),
// поэтому превью тоже берёт соседей по кругу, а не оставляет пустой слот на стыке —
// иначе в shuffle (где текущий трек стоит в начале playbackOrder) обложка предыдущего
// не показывалась, пока не переключишься назад.
function queueTrack(offset: number) {
  const len = playbackOrder.value.length;
  const i = currentTrackIndex.value;
  if (i < 0 || len === 0) return null;
  const wrapped = (((i + offset) % len) + len) % len;
  if (wrapped === i) return null; // очередь из одного трека — соседей нет
  const t = playbackOrder.value[wrapped];
  return t ? (getTrackByPath(t.file_path) ?? t) : null;
}
const previewTracks = computed(() => ({
  prev: queueTrack(-1),
  current: currentTrack.value ? (getTrackByPath(currentTrack.value.file_path) ?? currentTrack.value) : null,
  next: queueTrack(1),
}));

// Подгружаем обложки тройки, чтобы превью можно было показать целиком.
watch(previewTracks, ({ prev, current, next }) => {
  const tracks = [prev, current, next].filter(Boolean);
  if (tracks.length) loadCoversForTracks(tracks as any);
}, { immediate: true });

// Все 3 обложки считаются готовыми, когда каждая загружена или подтверждённо отсутствует.
const previewReady = computed(() =>
  [previewTracks.value.prev, previewTracks.value.current, previewTracks.value.next]
    .every(t => !t || isCoverResolved(t)),
);

// Состояние воспроизведения → иконка кнопки play/pause в панели задач.
watch(isPlaying, (playing) => {
  invoke('taskbar_set_playing', { playing }).catch(() => {});
}, { immediate: true });

// Состояния shuffle/repeat → иконки кнопок в панели задач.
watch([shuffled, repeatMode], ([sh, rep]) => {
  invoke('taskbar_set_modes', { shuffled: sh, repeatTrack: rep === 'track' }).catch(() => {});
}, { immediate: true });

// Превью в панели задач обновляем только когда все 3 обложки готовы —
// иначе при перемешивании соседние обложки сначала пропадали бы и подгружались с задержкой.
let lastPreviewPaths: { prev: string | null; current: string | null; next: string | null } = {
  prev: null, current: null, next: null,
};
watch([previewTracks, previewReady], () => {
  if (!previewReady.value) return; // держим прежнее превью, пока новые не загрузятся
  const { prev, current, next } = previewTracks.value;
  const paths = {
    prev: prev?.file_path ?? null,
    current: current?.file_path ?? null,
    next: next?.file_path ?? null,
  };

  // Определяем тип анимации: вперёд (next стал центром), назад (prev стал центром), иначе fade.
  let transition: 'forward' | 'backward' | 'fade' | 'none' = 'none';
  if (lastPreviewPaths.current && paths.current) {
    if (paths.current === lastPreviewPaths.next) transition = 'forward';
    else if (paths.current === lastPreviewPaths.prev) transition = 'backward';
    else transition = 'fade';
  }
  lastPreviewPaths = paths;

  invoke('taskbar_set_covers', {
    prev: prev?.cover_art ?? null,
    current: current?.cover_art ?? null,
    next: next?.cover_art ?? null,
    transition,
  }).catch(() => {});
}, { immediate: true });

onUnmounted(() => {
  window.removeEventListener('resize', scheduleUiScale);
  window.removeEventListener('focus', onWindowFocus);
  window.removeEventListener('lfmusic:android-permissions', onAndroidPermissionsEvent as EventListener);
  window.removeEventListener('lfmusic:android-insets', onAndroidInsetsEvent as EventListener);
  window.removeEventListener('lfmusic:show-fullscreen-cover', handleFullscreenCoverEvent as EventListener);
  window.visualViewport?.removeEventListener('resize', scheduleUiScale);
  window.visualViewport?.removeEventListener('resize', scheduleAndroidInsetsRefresh);
  if (resizeRaf !== null) {
    cancelAnimationFrame(resizeRaf);
    resizeRaf = null;
  }
  if (featureProgressHideTimer) {
    clearTimeout(featureProgressHideTimer);
    featureProgressHideTimer = null;
  }
  if (androidInsetsRetryTimer) {
    clearTimeout(androidInsetsRetryTimer);
    androidInsetsRetryTimer = null;
  }
  if (unlistenAudioFeatureProgress) {
    unlistenAudioFeatureProgress();
    unlistenAudioFeatureProgress = null;
  }
  if (unlistenTaskbarMedia) {
    unlistenTaskbarMedia();
    unlistenTaskbarMedia = null;
  }
  document.removeEventListener('keydown', handleReloadKey);
  document.removeEventListener('contextmenu', handleContextMenu);
  document.removeEventListener('visibilitychange', onVisibilityChange);
});

watchEffect(() => {
  const hex = settings.value.dynamic_accent_color
    ? (currentAccentColor.value ?? '#1a1a1a')
    : settings.value.static_accent_color;
  const { h, s, l } = hexToHSL(hex);
  const chromeHex = hslToHex(h, Math.min(s, 50), 8);
  const root = document.documentElement.style;
  root.setProperty('--accent-h', `${h}`);
  root.setProperty('--accent-s', `${s}%`);
  root.setProperty('--accent-l', `${l}%`);
  root.setProperty('--accent-color', h === 0 && s === 0 ? '#353535' : `hsl(${h}, ${s}%, ${l}%)`);
  root.setProperty('--mobile-chrome-color', chromeHex);
  window.LFMusicAndroid?.updateSystemBars?.(chromeHex);
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

function hslToHex(h: number, s: number, l: number) {
  s /= 100;
  l /= 100;

  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs((h / 60) % 2 - 1));
  const m = l - c / 2;
  let r = 0;
  let g = 0;
  let b = 0;

  if (h < 60) [r, g, b] = [c, x, 0];
  else if (h < 120) [r, g, b] = [x, c, 0];
  else if (h < 180) [r, g, b] = [0, c, x];
  else if (h < 240) [r, g, b] = [0, x, c];
  else if (h < 300) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];

  const toHex = (value: number) => Math.round((value + m) * 255).toString(16).padStart(2, '0');
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

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
  <Player :key="`player-${language}`" />
  <TrackInfoModal :key="`info-${language}`" />
  <TrackEditModal :key="`edit-${language}`" />
  <div v-if="showAndroidPermissionWarning" class="androidPermissionWarning" role="alertdialog" aria-modal="true">
    <div class="androidPermissionCard">
      <div class="androidPermissionKicker">Требуются разрешения</div>
      <div class="androidPermissionTitle">LFMusic нужен доступ</div>
      <div class="androidPermissionText">{{ androidPermissionWarningText }}</div>
      <div class="androidPermissionActions">
        <button class="androidPermissionButton primary" type="button" @click="requestAndroidPermissions">
          Выдать разрешения
        </button>
        <button class="androidPermissionButton" type="button" @click="refreshAndroidPermissions">
          Проверить снова
        </button>
      </div>
    </div>
  </div>
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
  transition: transform 400ms ease, opacity 400ms ease, background-color .24s ease;
  will-change: transform;
  backface-visibility: hidden;
  transform: translate3d(0, 0, 0);
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

.androidPermissionWarning {
  position: fixed;
  inset: 0;
  z-index: 1000001;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: calc(var(--safe-top) + 18px) calc(var(--safe-right) + 18px) calc(var(--safe-bottom) + 18px) calc(var(--safe-left) + 18px);
  background: hsla(0, 0%, 0%, 0.58);
  backdrop-filter: blur(18px);
}
.androidPermissionCard {
  width: min(420px, 100%);
  border: 1px solid hsla(0, 0%, 100%, 0.1);
  border-radius: 24px;
  padding: 22px;
  background: hsl(var(--accent-h), min(var(--accent-s), 48%), 11%);
  box-shadow: 0 26px 80px hsla(0, 0%, 0%, 0.5);
  color: #fff;
}
.androidPermissionKicker {
  margin-bottom: 8px;
  font-size: 12px;
  line-height: 1.2;
  font-weight: 800;
  letter-spacing: 0;
  color: hsla(0, 0%, 100%, 0.5);
}
.androidPermissionTitle {
  font-size: 24px;
  line-height: 1.12;
  font-weight: 900;
}
.androidPermissionText {
  margin-top: 12px;
  font-size: 14px;
  line-height: 1.45;
  font-weight: 600;
  color: hsla(0, 0%, 100%, 0.72);
}
.androidPermissionActions {
  display: flex;
  gap: 10px;
  margin-top: 20px;
}
.androidPermissionButton {
  min-height: 44px;
  flex: 1 1 0;
  min-width: 0;
  border-radius: 14px;
  padding: 10px 14px;
  background: hsla(0, 0%, 100%, 0.1);
  color: hsla(0, 0%, 100%, 0.88);
  font-size: 13px;
  font-weight: 800;
  cursor: pointer;
}
.androidPermissionButton.primary {
  background: #fff;
  color: #0f0f0f;
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

/* Fullscreen Cover Overlay */
.fullscreenCoverBackdrop {
  position: fixed;
  top: 0;
  left: 0;
  width: calc(100vw / var(--ui-scale, 1));
  height: calc(100dvh / var(--ui-scale, 1));
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
