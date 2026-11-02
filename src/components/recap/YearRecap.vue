<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { loadYearRecap, recapOpen, recapYear, type YearRecap } from '../../composables/useYearRecap'
import { usePlayer } from '../../composables/usePlayer'
import { useI18n } from '../../composables/useI18n'
import { buildRecapCards, type RecapCard as Card } from './recapCards'
import { useRecapSound } from './useRecapSound'
import RecapCard from './RecapCard.vue'
import CloseIcon from '../Icons/CloseIcon.vue'
import PlayIcon from '../Icons/PlayIcon.vue'
import PauseIcon from '../Icons/PauseIcon.vue'
import VolumeMaxIcon from '../Icons/VolumeMaxIcon.vue'
import VolumeMinIcon from '../Icons/VolumeMinIcon.vue'
import VolumeZeroIcon from '../Icons/VolumeZeroIcon.vue'
import PrevIcon from '../Icons/PrevIcon.vue'
import NextIcon from '../Icons/NextIcon.vue'

// Year recap as a story: full-screen cards with progress segments, tap/swipe/
// keyboard navigation, hold to pause and a looping fragment of the featured track.

const { t } = useI18n()
const { playbackState, togglePlayPause } = usePlayer()
const sound = useRecapSound()

const recap = ref<YearRecap | null>(null)
const cards = ref<Card[]>([])
const index = ref(0)
const direction = ref<1 | -1>(1)
const loadError = ref(false)
const held = ref(false)
const userPaused = ref(false)
const hidden = ref(false)
/** Bumped to restart the active segment's timer (replay, same index). */
const cycle = ref(0)

let pausedPlayer = false

const current = computed(() => cards.value[index.value] ?? null)
const paused = computed(() => held.value || userPaused.value || hidden.value)
const backdropStyle = computed(() => {
  const c = current.value
  return c?.accent ? { '--rc-h': `${c.accent.hue}`, '--rc-s': `${c.accent.sat}%` } : {}
})

async function open() {
  sound.start()
  index.value = 0
  cycle.value++
  cards.value = []
  recap.value = null
  loadError.value = false
  userPaused.value = false
  pausedPlayer = playbackState.value === 'Playing'
  if (pausedPlayer) await togglePlayPause()
  try {
    const data = await loadYearRecap(recapYear.value)
    if (!recapOpen.value) return
    recap.value = data
    cards.value = buildRecapCards(data)
  } catch (e) {
    console.error('Ошибка загрузки итогов года:', e)
    loadError.value = true
  }
}

function close() {
  recapOpen.value = false
}

async function teardown() {
  void sound.stop()
  if (pausedPlayer && playbackState.value === 'Paused') await togglePlayPause()
  pausedPlayer = false
}

watch(recapOpen, (value, old) => {
  if (value) void open()
  else if (old) void teardown()
}, { immediate: true })

function go(next: number) {
  if (next < 0 || next >= cards.value.length || next === index.value) return
  direction.value = next > index.value ? 1 : -1
  index.value = next
  cycle.value++
}

function next() {
  go(index.value + 1)
}

function prev() {
  go(index.value - 1)
}

function replay() {
  go(0)
  userPaused.value = false
}

function onSegmentDone(i: number) {
  if (i === index.value && current.value?.duration) next()
}

watch(current, card => {
  if (!recapOpen.value) return
  void sound.play(card?.sound ?? null)
  sound.preload(cards.value[index.value + 1]?.sound)
})

// ── Pointer: tap sides to move, hold to pause, swipe on touch ──────────────

const HOLD_MS = 220
let pressAt = 0
let pressX = 0
let pressY = 0
let holdTimer: ReturnType<typeof setTimeout> | null = null

function isInteractive(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest('button, a, input, [data-interactive]')
}

// ── Volume: hover slider, drag, wheel, arrows ──────────────────────────────

const VOLUME_STEP = 0.05
const volumeRail = ref<HTMLElement | null>(null)
const volumeDragging = ref(false)
const shownVolume = computed(() => (sound.muted.value ? 0 : sound.volume.value))
const volumePercent = computed(() => Math.round(shownVolume.value * 100))

function volumeAt(clientX: number) {
  const rect = volumeRail.value?.getBoundingClientRect()
  if (!rect || rect.width === 0) return
  sound.setVolume((clientX - rect.left) / rect.width)
}

function onVolumePointerDown(e: PointerEvent) {
  if (e.button !== 0) return
  volumeDragging.value = true
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  volumeAt(e.clientX)
}

function onVolumePointerMove(e: PointerEvent) {
  if (volumeDragging.value) volumeAt(e.clientX)
}

function onVolumePointerUp() {
  volumeDragging.value = false
}

/** One mouse wheel notch (~100px) is one step; trackpads scroll in finer increments. */
function onVolumeWheel(e: WheelEvent) {
  const raw = Math.abs(e.deltaY) >= Math.abs(e.deltaX) ? -e.deltaY : e.deltaX
  const px = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? raw * 33 : e.deltaMode === WheelEvent.DOM_DELTA_PAGE ? raw * 300 : raw
  if (px === 0) return
  sound.setVolume(shownVolume.value + (px / 100) * VOLUME_STEP)
}

function onVolumeKey(e: KeyboardEvent) {
  const step = e.key === 'ArrowRight' || e.key === 'ArrowUp' ? VOLUME_STEP
    : e.key === 'ArrowLeft' || e.key === 'ArrowDown' ? -VOLUME_STEP : 0
  if (!step) return
  e.preventDefault()
  sound.setVolume(shownVolume.value + step)
}

function onPointerDown(e: PointerEvent) {
  if (isInteractive(e.target) || e.button !== 0) return
  pressAt = performance.now()
  pressX = e.clientX
  pressY = e.clientY
  holdTimer = setTimeout(() => { held.value = true }, HOLD_MS)
}

function onPointerUp(e: PointerEvent) {
  if (!pressAt) return
  if (holdTimer) clearTimeout(holdTimer)
  holdTimer = null
  const wasHeld = held.value
  held.value = false
  const dx = e.clientX - pressX
  const dy = e.clientY - pressY
  pressAt = 0
  if (dy > 90 && Math.abs(dy) > Math.abs(dx)) {
    close()
    return
  }
  if (Math.abs(dx) > 50 && Math.abs(dx) > Math.abs(dy)) {
    if (dx < 0) next()
    else prev()
    return
  }
  if (wasHeld || Math.abs(dx) > 10 || Math.abs(dy) > 10) return
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
  if (e.clientX - rect.left < rect.width * 0.3) prev()
  else next()
}

function onPointerCancel() {
  if (holdTimer) clearTimeout(holdTimer)
  holdTimer = null
  held.value = false
  pressAt = 0
}

let wheelLock = 0
function onWheel(e: WheelEvent) {
  if (Math.abs(e.deltaY) < 20 && Math.abs(e.deltaX) < 20) return
  const now = performance.now()
  if (now < wheelLock) return
  wheelLock = now + 450
  const delta = Math.abs(e.deltaY) > Math.abs(e.deltaX) ? e.deltaY : e.deltaX
  if (delta > 0) next()
  else prev()
}

function onKeydown(e: KeyboardEvent) {
  if (!recapOpen.value) return
  // Arrows on the focused volume slider change the volume, not the card.
  if (e.target instanceof Element && e.target.closest('[role="slider"]') && e.key.startsWith('Arrow')) return
  if (e.key === 'Escape') close()
  else if (e.key === 'ArrowRight' || e.key === 'ArrowDown') next()
  else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') prev()
  else if (e.key === ' ') userPaused.value = !userPaused.value
  else if (e.key.toLowerCase() === 'm') sound.toggleMute()
  else return
  e.preventDefault()
  e.stopPropagation()
}

function onVisibility() {
  hidden.value = document.visibilityState === 'hidden'
  sound.setPaused(hidden.value)
}

onMounted(() => {
  window.addEventListener('keydown', onKeydown, true)
  document.addEventListener('visibilitychange', onVisibility)
})

onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown, true)
  document.removeEventListener('visibilitychange', onVisibility)
  if (recapOpen.value) void teardown()
})
</script>

<template>
  <Teleport to="body">
    <Transition name="recapFade">
      <div v-if="recapOpen" class="recapRoot" :style="backdropStyle" @wheel.passive="onWheel">
        <div class="recapGlow" aria-hidden="true"></div>

        <button type="button" class="recapClose" :aria-label="t('recap.controls.close')" v-tooltip:left="t('recap.controls.close')" @click="close">
          <CloseIcon width="20" height="20" />
        </button>

        <button
          v-if="cards.length > 1"
          type="button"
          class="recapArrow left"
          :disabled="index === 0"
          :aria-label="t('recap.controls.prev')"
          @click="prev"
        >
          <PrevIcon width="18" height="18" />
        </button>

        <div
          class="recapFrame"
          :class="{ paused }"
          @pointerdown="onPointerDown"
          @pointerup="onPointerUp"
          @pointercancel="onPointerCancel"
          @pointerleave="onPointerCancel"
          @contextmenu.prevent
        >
          <Transition :name="direction === 1 ? 'recapNext' : 'recapPrev'">
            <RecapCard
              v-if="recap && current"
              :key="`${current.id}-${cycle}`"
              :card="current"
              :recap="recap"
              :playing-url="sound.playingUrl.value"
              @sound="url => sound.play(url)"
              @replay="replay"
            />
          </Transition>

          <div v-if="!recap" class="recapLoading">
            <div v-if="loadError" class="recapLoadingText">{{ t('recap.error') }}</div>
            <template v-else>
              <div class="recapEq" aria-hidden="true"><span></span><span></span><span></span></div>
              <div class="recapLoadingText">{{ t('recap.loading') }}</div>
            </template>
          </div>

          <div class="recapTop">
            <div class="recapSegments">
              <div v-for="(c, i) in cards" :key="c.id" class="recapSegment">
                <span
                  v-if="i === index"
                  :key="cycle"
                  class="fill running"
                  :style="{ animationDuration: `${c.duration || 1}ms` }"
                  :class="{ still: !c.duration }"
                  @animationend="onSegmentDone(i)"
                ></span>
                <span v-else-if="i < index" class="fill done"></span>
              </div>
            </div>
            <div class="recapTools">
              <button
                v-if="current?.duration"
                type="button"
                class="recapTool"
                :aria-label="userPaused ? t('recap.controls.play') : t('recap.controls.pause')"
                @click="userPaused = !userPaused"
              >
                <PlayIcon v-if="userPaused" width="14" height="14" />
                <PauseIcon v-else width="14" height="14" />
              </button>
              <div
                class="recapVolume"
                :class="{ open: volumeDragging }"
                data-interactive
                @wheel.stop.prevent="onVolumeWheel"
              >
                <div
                  class="recapVolumeSlider"
                  role="slider"
                  tabindex="0"
                  aria-valuemin="0"
                  aria-valuemax="100"
                  :aria-valuenow="volumePercent"
                  :aria-label="t('recap.controls.volume')"
                  @pointerdown="onVolumePointerDown"
                  @pointermove="onVolumePointerMove"
                  @pointerup="onVolumePointerUp"
                  @pointercancel="onVolumePointerUp"
                  @keydown="onVolumeKey"
                >
                  <div ref="volumeRail" class="recapVolumeRail" :style="{ '--vol': shownVolume }">
                    <span class="recapVolumeFill"></span>
                    <span class="recapVolumeThumb"></span>
                  </div>
                </div>
                <button
                  type="button"
                  class="recapTool"
                  :aria-label="sound.muted.value ? t('recap.controls.unmute') : t('recap.controls.mute')"
                  @click="sound.toggleMute()"
                >
                  <VolumeZeroIcon v-if="shownVolume === 0" width="16" height="16" />
                  <VolumeMinIcon v-else-if="shownVolume <= 0.4" width="16" height="16" />
                  <VolumeMaxIcon v-else width="16" height="16" />
                </button>
              </div>
            </div>
          </div>
        </div>

        <button
          v-if="cards.length > 1"
          type="button"
          class="recapArrow right"
          :disabled="index >= cards.length - 1"
          :aria-label="t('recap.controls.next')"
          @click="next"
        >
          <NextIcon width="18" height="18" />
        </button>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.recapRoot {
  --rc-h: var(--accent-h, 250);
  --rc-s: var(--accent-s, 45%);
  position: fixed;
  top: 0;
  left: 0;
  width: calc(100vw / var(--overlay-scale, 1));
  height: calc(100dvh / var(--overlay-scale, 1));
  z-index: 1000002;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 28px;
  background: hsla(0, 0%, 3%, 0.86);
  backdrop-filter: blur(16px);
  overscroll-behavior: contain;
}

/* One soft light of the current card's accent behind the frame. */
.recapGlow {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background:
    radial-gradient(42% 60% at 50% 50%, hsla(var(--rc-h), var(--rc-s), 45%, 0.14), transparent 72%);
  transition: background .8s ease;
}

.recapFrame {
  position: relative;
  height: min(800px, calc(100dvh / var(--overlay-scale, 1) - 64px));
  aspect-ratio: 9 / 16;
  max-width: calc(100vw / var(--overlay-scale, 1) - 32px);
  border-radius: 22px;
  overflow: hidden;
  background: hsl(var(--rc-h), min(var(--rc-s), 26%), 8%);
  box-shadow: 0 32px 90px hsla(0, 0%, 0%, 0.55), inset 0 0 0 1px hsla(0, 0%, 100%, 0.05);
  touch-action: none;
  cursor: pointer;
  isolation: isolate;
}

.recapTop {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  z-index: 5;
  padding: 14px 14px 0;
  background: linear-gradient(180deg, hsla(0, 0%, 0%, 0.28), transparent);
  pointer-events: none;
}
.recapSegments {
  display: flex;
  gap: 4px;
}
.recapSegment {
  flex: 1;
  height: 3px;
  border-radius: 999px;
  overflow: hidden;
  background: hsla(0, 0%, 100%, 0.28);
}
.fill {
  display: block;
  height: 100%;
  background: #fff;
  border-radius: inherit;
  transform-origin: left;
}
.fill.done {
  transform: scaleX(1);
}
.fill.running {
  animation-name: recapFill;
  animation-timing-function: linear;
  animation-fill-mode: both;
}
.fill.still {
  animation: none;
  transform: scaleX(1);
}
.paused .fill.running {
  animation-play-state: paused;
}
@keyframes recapFill {
  from { transform: scaleX(0); }
  to { transform: scaleX(1); }
}

.recapTools {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
  margin-top: 10px;
}
.recapTool {
  pointer-events: auto;
  width: 32px;
  height: 32px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  background: hsla(0, 0%, 0%, 0.25);
  backdrop-filter: blur(8px);
  cursor: pointer;
  transition: background-color .18s ease;
}
.recapTool:hover {
  background: hsla(0, 0%, 0%, 0.4);
}

/* Volume: the icon button grows a slider to its left on hover, focus or drag. */
.recapVolume {
  pointer-events: auto;
  display: flex;
  align-items: center;
  height: 32px;
  border-radius: 999px;
  background: hsla(0, 0%, 0%, 0.25);
  backdrop-filter: blur(8px);
  transition: background-color .18s ease;
}
.recapVolume:hover,
.recapVolume:focus-within,
.recapVolume.open {
  background: hsla(0, 0%, 0%, 0.4);
}
.recapVolume .recapTool,
.recapVolume .recapTool:hover {
  background: transparent;
  backdrop-filter: none;
}
.recapVolumeSlider {
  width: 0;
  height: 100%;
  overflow: hidden;
  display: flex;
  align-items: center;
  cursor: pointer;
  outline: none;
  touch-action: none;
  transition: width .2s ease, padding .2s ease;
}
.recapVolume:hover .recapVolumeSlider,
.recapVolume:focus-within .recapVolumeSlider,
.recapVolume.open .recapVolumeSlider {
  width: 92px;
  padding: 0 4px 0 14px;
}
.recapVolumeRail {
  position: relative;
  flex: 1;
  height: 4px;
  border-radius: 999px;
  background: hsla(0, 0%, 100%, 0.28);
}
.recapVolumeFill {
  position: absolute;
  inset: 0 auto 0 0;
  width: calc(var(--vol) * 100%);
  border-radius: inherit;
  background: #fff;
}
.recapVolumeThumb {
  position: absolute;
  top: 50%;
  left: calc(var(--vol) * 100%);
  width: 11px;
  height: 11px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 3px hsla(0, 0%, 0%, 0.35);
  transform: translate(-50%, -50%);
}
.recapVolumeSlider:focus-visible .recapVolumeThumb {
  box-shadow: 0 0 0 3px hsla(0, 0%, 100%, 0.35);
}

.recapClose {
  position: absolute;
  top: 18px;
  right: 20px;
  z-index: 3;
  width: 40px;
  height: 40px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: hsla(0, 0%, 100%, 0.8);
  background: hsla(0, 0%, 100%, 0.08);
  cursor: pointer;
  transition: background-color .2s ease, color .2s ease;
}
.recapClose:hover {
  background: hsla(0, 0%, 100%, 0.16);
  color: #fff;
}

.recapArrow {
  position: relative;
  z-index: 2;
  flex-shrink: 0;
  width: 46px;
  height: 46px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  background: hsla(0, 0%, 100%, 0.1);
  cursor: pointer;
  transition: background-color .2s ease, opacity .2s ease;
}
.recapArrow:hover:not(:disabled) {
  background: hsla(0, 0%, 100%, 0.2);
}
.recapArrow:disabled {
  opacity: 0.25;
  cursor: default;
}

.recapLoading {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 16px;
  background:
    radial-gradient(130% 62% at 50% -6%, hsla(var(--rc-h), var(--rc-s), 56%, 0.2), transparent 64%),
    hsl(var(--rc-h), min(var(--rc-s), 26%), 8%);
}
.recapLoadingText {
  font-size: 14px;
  font-weight: 600;
  color: hsla(0, 0%, 100%, 0.7);
}
.recapEq {
  display: flex;
  align-items: flex-end;
  gap: 4px;
  height: 26px;
}
.recapEq span {
  width: 5px;
  height: 100%;
  border-radius: 3px;
  background: #fff;
  animation: recapEq 0.9s ease-in-out infinite alternate;
}
.recapEq span:nth-child(2) { animation-delay: -0.3s; }
.recapEq span:nth-child(3) { animation-delay: -0.6s; }
@keyframes recapEq {
  from { transform: scaleY(0.25); }
  to { transform: scaleY(1); }
}

/* Card transitions: both cards stay absolutely positioned inside the frame. */
.recapNext-enter-active, .recapNext-leave-active,
.recapPrev-enter-active, .recapPrev-leave-active {
  transition: transform .45s cubic-bezier(.2, .8, .2, 1), opacity .35s ease;
}
.recapNext-enter-from { transform: translateX(8%) scale(0.97); opacity: 0; }
.recapNext-leave-to { transform: translateX(-8%) scale(0.97); opacity: 0; }
.recapPrev-enter-from { transform: translateX(-8%) scale(0.97); opacity: 0; }
.recapPrev-leave-to { transform: translateX(8%) scale(0.97); opacity: 0; }

.recapFade-enter-active, .recapFade-leave-active {
  transition: opacity .3s ease;
}
.recapFade-enter-active .recapFrame {
  transition: transform .45s cubic-bezier(.2, .9, .3, 1.1);
}
.recapFade-enter-from, .recapFade-leave-to {
  opacity: 0;
}
.recapFade-enter-from .recapFrame {
  transform: scale(0.92) translateY(20px);
}

/* Phones: the card is the whole screen. */
@media (max-width: 600px) {
  .recapRoot { gap: 0; }
  .recapFrame {
    width: 100%;
    height: 100%;
    max-width: none;
    aspect-ratio: auto;
    border-radius: 0;
  }
  .recapArrow { display: none; }
  .recapClose {
    /* Same row as the pause/mute tools: segments (12 + 3) + 10px gap. */
    top: calc(var(--safe-top, 0px) + 25px);
    right: 14px;
    z-index: 6;
    width: 32px;
    height: 32px;
    background: hsla(0, 0%, 0%, 0.25);
  }
  .recapTop { padding-top: calc(var(--safe-top, 0px) + 12px); }
  .recapTools { margin-right: 44px; }
}
</style>
