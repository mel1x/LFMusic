<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, nextTick } from 'vue';

const mainRef = ref<HTMLElement | null>(null);
const thumbRef = ref<HTMLElement | null>(null);
const trackRef = ref<HTMLElement | null>(null);

const isScrollable = ref(false);

let hideTimer: ReturnType<typeof setTimeout> | null = null;
let isDragging = false;
let dragStartY = 0;
let dragStartScrollTop = 0;
let resizeObserver: ResizeObserver | null = null;
let mutationObserver: MutationObserver | null = null;

async function updateThumb() {
  const el = mainRef.value;
  if (!el) return;

  const { scrollTop, scrollHeight, clientHeight } = el;
  const maxScroll = Math.max(0, scrollHeight - clientHeight);
  
  isScrollable.value = maxScroll > 2;
  
  if (!isScrollable.value) {
    if (thumbRef.value) thumbRef.value.style.opacity = '0';
    return;
  }

  await nextTick();

  const thumb = thumbRef.value;
  const track = trackRef.value;
  if (!thumb || !track) return;

  const trackHeight = track.clientHeight;
  const thumbHeight = Math.max((clientHeight / scrollHeight) * trackHeight, 32);

  const maxThumbTop = trackHeight - thumbHeight;
  const thumbTop = maxScroll > 0 ? (scrollTop / maxScroll) * maxThumbTop : 0;

  thumb.style.height = `${thumbHeight}px`;
  thumb.style.transform = `translateY(${thumbTop}px)`;
}

function setThumbVisible(visible: boolean) {
  if (!isScrollable.value) return;
  const thumb = thumbRef.value;
  if (!thumb) return;
  thumb.style.opacity = visible ? '1' : '0';
}

function showThumbTemporary() {
  if (hideTimer) clearTimeout(hideTimer);
  if (!isScrollable.value) return;
  
  setThumbVisible(true);
  hideTimer = setTimeout(() => {
    if (!isDragging) setThumbVisible(false);
  }, 1200);
}

function onScroll() {
  updateThumb();
  showThumbTemporary();
}

function onTrackMouseEnter() {
  if (!isScrollable.value) return;
  if (hideTimer) clearTimeout(hideTimer);
  setThumbVisible(true);
}

function onTrackMouseLeave() {
  if (isDragging || !isScrollable.value) return;
  if (hideTimer) clearTimeout(hideTimer);
  setThumbVisible(false);
}

function startDrag(e: MouseEvent) {
  if (!isScrollable.value) return;
  isDragging = true;
  dragStartY = e.clientY;
  dragStartScrollTop = mainRef.value?.scrollTop ?? 0;

  document.addEventListener('mousemove', onDragMove);
  document.addEventListener('mouseup', onDragEnd);

  setThumbVisible(true);
  e.preventDefault();
}

function onThumbMouseDown(e: MouseEvent) {
  startDrag(e);
}

function onTrackMouseDown(e: MouseEvent) {
  startDrag(e);
}

function onDragMove(e: MouseEvent) {
  const el = mainRef.value;
  const thumb = thumbRef.value;
  const track = trackRef.value;
  if (!el || !thumb || !track || !isScrollable.value) return;

  const { scrollHeight, clientHeight } = el;
  const trackHeight = track.clientHeight;
  const thumbHeight = parseFloat(thumb.style.height || '0');
  const delta = e.clientY - dragStartY;
  const ratio = delta / Math.max(trackHeight - thumbHeight, 1);

  el.scrollTop = dragStartScrollTop + ratio * (scrollHeight - clientHeight);
}

function onDragEnd() {
  isDragging = false;
  document.removeEventListener('mousemove', onDragMove);
  document.removeEventListener('mouseup', onDragEnd);

  showThumbTemporary();
}

onMounted(() => {
  updateThumb();
  
  if (mainRef.value) {
    resizeObserver = new ResizeObserver(() => updateThumb());
    resizeObserver.observe(mainRef.value);

    mutationObserver = new MutationObserver(() => updateThumb());
    mutationObserver.observe(mainRef.value, { childList: true, subtree: true, characterData: true });
  }
});

onBeforeUnmount(() => {
  document.removeEventListener('mousemove', onDragMove);
  document.removeEventListener('mouseup', onDragEnd);
  
  if (resizeObserver) resizeObserver.disconnect();
  if (mutationObserver) mutationObserver.disconnect();
});
</script>

<template>
  <div class="scrollContainer">
    <main ref="mainRef" @scroll="onScroll">
      <slot />
    </main>

    <div
      v-show="isScrollable"
      ref="trackRef"
      class="scrollTrack"
      @mouseenter="onTrackMouseEnter"
      @mouseleave="onTrackMouseLeave"
      @mousedown="onTrackMouseDown"
    >
      <div
        ref="thumbRef"
        class="scrollThumb"
        @mousedown.stop="onThumbMouseDown"
      />
    </div>
  </div>
</template>

<style scoped>
.scrollContainer {
  position: relative;
  width: 100%;
  height: 100%;
}

main {
  width: 100%;
  height: 100%;
  overflow: auto;
  border-radius: 16px;
  border: 0.0625rem solid hsla(0, 0%, 100%, 0.04);
  scrollbar-width: none;
}
main::-webkit-scrollbar {
  display: none;
}

.scrollTrack {
  position: absolute;
  top: 10px;
  right: 0;
  bottom: 10px;
  width: 21px;
  pointer-events: all;
}

.scrollThumb {
  position: absolute;
  top: 0;
  left: 8px;
  width: 5px;
  background: #ffffff20;
  border-radius: 999px;
  opacity: 0;
  pointer-events: all;
  transition: opacity 0.25s ease, background 0.15s ease;
}

.scrollThumb:hover {
  background: #ffffff30;
}
.scrollThumb:active {
  background: #ffffff50;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
  main {
    border-radius: 0;
    border: 0;
    overscroll-behavior: contain;
  }

  .scrollTrack {
    display: none;
  }
}
</style>
