<script setup lang="ts">
import { computed, ref, watch, onMounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import ViewFrame from './ViewFrame.vue';
import SearchIcon from './Icons/SearchIcon.vue';
import CatalogIcon from './Icons/CatalogIcon.vue';
import CollectionIcon from './Icons/CollectionIcon.vue';
import SettingsIcon from './Icons/SettingsIcon.vue';
import HomeIcon from './Icons/HomeIcon.vue';
import MoreIcon from './Icons/MoreIcon.vue';
import PlaylistContextMenu from './PlaylistContextMenu.vue';
import AnimatedCover from './AnimatedCover.vue';
import defaultPlaylistCover from '../assets/playlistCover.webp';
import defaultTrackCover from '../assets/trackCover.webp';
import catalogCover from '../assets/cover.webp';
import { usePins, type ResolvedPin } from '../composables/usePins';
import { usePlayer } from '../composables/usePlayer';
import { useSettings } from '../composables/useSettings';
import { useCustomCovers } from '../composables/useCustomCovers';
import { tracksCountText } from '../utils/format';

const route = useRoute();
const router = useRouter();
const isCollectionsActive = computed(() => route.path.startsWith('/playlist') || route.path.startsWith('/album') || route.path.startsWith('/collections'));

const { resolvedPins, unpin, movePinToTop, loadPins } = usePins();
const { loadCoversForTracks, getTrackByPath } = usePlayer();
const { settings } = useSettings();
const { getCustomCover } = useCustomCovers();

// Высота блока плеера = 84px. Когда дополнительный блок плеера выключен,
// растягиваем aside до самого низа окна, иначе оставляем место под плеер.
const asideBottom = computed(() => settings.value.show_player_extra_block ? '84px' : '0px');

function pinCover(p: ResolvedPin): string {
    const custom = getCustomCover(p.kind, p.key);
    if (custom) return custom;
    if (p.kind === 'catalog') return catalogCover;
    if (p.cover) return p.cover;
    if (p.kind === 'playlist') return defaultPlaylistCover;
    return defaultTrackCover;
}

function isPinActive(p: ResolvedPin): boolean {
    if (p.kind === 'catalog') return route.path === '/catalog';
    if (p.kind === 'playlist') return route.path === `/playlist/${p.key}`;
    if (p.kind === 'album') return route.path === `/album/${encodeURIComponent(p.key)}`;
    if (p.kind === 'artist') return route.path === `/artist/${encodeURIComponent(p.key)}`;
    return false;
}

function openPin(p: ResolvedPin) {
    router.push(p.route);
}

const pinMenu = ref<{ x: number; y: number; pin: ResolvedPin } | null>(null);

function openPinMenu(e: MouseEvent, p: ResolvedPin) {
    e.stopPropagation();
    if (pinMenu.value?.pin.key === p.key && pinMenu.value.pin.kind === p.kind) {
        pinMenu.value = null;
        return;
    }
    const btn = e.currentTarget as HTMLElement;
    const rect = btn.getBoundingClientRect();
    pinMenu.value = { x: rect.right, y: rect.bottom + 6, pin: p };
}

function onUnpinFromMenu() {
    const p = pinMenu.value?.pin;
    if (!p) return;
    unpin(p.kind, p.key);
}

function onMovePinToTopFromMenu() {
    const p = pinMenu.value?.pin;
    if (!p) return;
    movePinToTop(p.kind, p.key);
}

// Подгрузка обложек для закреплений, у которых обложка ещё не получена.
function requestPinCovers() {
    const pending = [];
    for (const p of resolvedPins.value) {
        if (p.cover || !p.firstPath) continue;
        const t = getTrackByPath(p.firstPath);
        if (t && !t.cover_art) pending.push(t);
    }
    if (pending.length) loadCoversForTracks(pending);
}

onMounted(async () => {
    await loadPins();
    requestPinCovers();
});
watch(resolvedPins, () => requestPinCovers());
</script>

<template>
  <div class="mainWrapper">
    <div class="asideSpacer"></div>
    <aside class="navAside" :style="{ '--mainblock-aside-bottom': asideBottom }">
      <nav class="navList">
        <RouterLink to="/search" class="navLink"><SearchIcon width="24" height="24" stroke-width="2.25" />Поиск</RouterLink>
        <RouterLink to="/" class="navLink"><HomeIcon width="24" height="24" stroke-width="2.25" />Главная</RouterLink>
        <RouterLink to="/collections" class="navLink" :class="{ 'router-link-active': isCollectionsActive }"><CollectionIcon width="24" height="24" stroke-width="2.25" />Коллекция</RouterLink>
        <RouterLink to="/catalog" class="navLink"><CatalogIcon width="24" height="24" stroke-width="2.25" />Каталог</RouterLink>
        <RouterLink to="/settings" class="navLink"><SettingsIcon width="24" height="24" stroke-width="2.25" />Настройки</RouterLink>
      </nav>

      <div class="pinsBlock" v-if="resolvedPins.length > 0">
        <div class="pinsDivider"></div>
        <div class="pinsList">
          <div
            v-for="p in resolvedPins"
            :key="p.kind + ':' + p.key"
            class="pinItem"
            :class="{ active: isPinActive(p), menuOpen: pinMenu?.pin.kind === p.kind && pinMenu?.pin.key === p.key }"
            @click="openPin(p)"
            v-tooltip="p.name"
          >
            <AnimatedCover
              class="pinCover"
              :class="{ artist: p.kind === 'artist' }"
              :src="pinCover(p)"
              alt=""
            />
            <div class="pinInfo">
              <div v-truncate class="pinName">{{ p.name }}</div>
              <div class="pinMeta">{{ p.exists ? tracksCountText(p.trackCount) : 'Недоступно' }}</div>
            </div>
            <button
              class="pinMoreBtn"
              :class="{ menuOpen: pinMenu?.pin.kind === p.kind && pinMenu?.pin.key === p.key }"
              @mousedown.stop="openPinMenu($event, p)"
              @click.stop
              v-tooltip="'Меню'"
            >
              <MoreIcon width="18" height="18" />
            </button>
          </div>
        </div>
      </div>
    </aside>
    <div class="frameWrapper">
      <ViewFrame>
        <RouterView v-slot="{ Component, route }">
          <Transition name="fade" mode="out-in">
            <component :is="Component" :key="route.fullPath" />
          </Transition>
        </RouterView>
      </ViewFrame>
    </div>
    <PlaylistContextMenu
      v-if="pinMenu"
      :x="pinMenu.x"
      :y="pinMenu.y"
      alignRight
      :isPinned="true"
      :showDelete="false"
      showMoveToTop
      moveToTopUp
      @close="pinMenu = null"
      @togglePin="onUnpinFromMenu"
      @moveToTop="onMovePinToTopFromMenu"
    />
  </div>
</template>

<style scoped>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.24s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.mainWrapper {
  display: flex;
  flex-direction: row;
  width: 100%;
  height: 100%;
  position: relative;
}
.asideSpacer {
  width: 240px;
  flex-shrink: 0;
}
.navAside {
  position: fixed;
  top: 38px;
  left: 0;
  bottom: var(--mainblock-aside-bottom, 84px);
  width: 240px;
  display: flex;
  flex-direction: column;
  padding: 32px 14px 20px;
  gap: 8px;
  box-sizing: border-box;
  z-index: 5;
  transition: bottom 0.24s ease;
}
.navList {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex-shrink: 0;
}
.navLink {
  user-select: none;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 12px;
  color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
  width: 100%;
  font-weight: 600;
  padding: 10px 14px;
  font-size: 14px;
  box-sizing: border-box;

  transition: .2s ease;

  &:hover {
    color: hsla(0, 0%, 100%, 0.95);
  }
}
.navLink svg {
  color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
  transition: .2s ease;
}
.navLink:hover svg {
  color: #fff;
}
.navLink.router-link-active {
  color: hsla(0, 0%, 100%, 0.95);
}
.navLink.router-link-active svg {
  color: #fff;
}

/* ── Pinned items ───────────────────────────────────────────────────────── */

.pinsBlock {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 4px;
  min-height: 0;
  flex: 1;
}
.pinsDivider {
  height: 1px;
  margin: 4px 14px 6px;
  background-color: hsla(0, 0%, 100%, 0.06);
  flex-shrink: 0;
}
.pinsList {
  display: flex;
  flex-direction: column;
  gap: 2px;
  overflow-y: overlay;
  overflow-x: hidden;
  padding: 0 4px 8px;
  scrollbar-gutter: stable;
}
.pinsList::-webkit-scrollbar {
  width: 4px;
}
.pinsList::-webkit-scrollbar-track {
  background: transparent;
  margin: 3px 0;
}
.pinsList::-webkit-scrollbar-thumb {
  background-color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 70%, 0.1);
  border-radius: 9999px;
  transition: background-color 0.2s ease;
}
.pinsList:hover::-webkit-scrollbar-thumb {
  background-color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 75%, 0.15);
}
.pinsList::-webkit-scrollbar-thumb:hover {
  background-color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 80%, 0.2);
}
.pinItem {
  position: relative;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
  padding: 6px 8px;
  border-radius: 8px;
  cursor: pointer;
  user-select: none;
  background-color: transparent;
  transition: background-color 0.2s ease;
  min-width: 0;
}
.pinItem:hover,
.pinItem.menuOpen {
  background-color: hsla(0, 0%, 100%, 0.05);
}
.pinItem.active {
  background-color: hsla(0, 0%, 100%, 0.04);
}
.pinCover {
  width: 38px;
  height: 38px;
  border-radius: 5px;
  object-fit: cover;
  background-color: #202020;
  flex-shrink: 0;
}
.pinCover.artist {
  border-radius: 9999px;
}
.pinInfo {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 1;
  min-width: 0;
}
.pinName {
  font-size: 13px;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.85);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.pinItem.active .pinName {
  color: hsla(0, 0%, 100%, 1);
}
.pinMeta {
  font-size: 11px;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.4);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.pinMoreBtn {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 4px;
  border-radius: 9999px;
  cursor: pointer;
  color: hsla(0, 0%, 100%, 0.5);
  background-color: transparent;
  border: none;
  opacity: 0;
  transition: opacity 0.2s ease, color 0.2s ease, background-color 0.2s ease;
  flex-shrink: 0;
}
.pinMoreBtn:hover {
  color: hsla(0, 0%, 100%, 0.9);
  background-color: hsla(0, 0%, 100%, 0.08);
}
.pinItem:hover .pinMoreBtn,
.pinMoreBtn.menuOpen {
  opacity: 1;
}

.frameWrapper {
  border-radius: 16px;
  background-color: var(--light-bg);
  background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
  background-repeat: no-repeat;
  background-attachment: local;
  width: calc(100% - 240px - 12px);
  height: 100%;
  transition: .24s ease;
}
</style>
