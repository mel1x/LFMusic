<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch } from 'vue';
import { HOLIDAYS, activeHoliday, holidayModalOpen } from '../composables/useHoliday';
import { hexToHSL } from '../composables/useAccentTheme';
import { useI18n } from '../composables/useI18n';
import { HOLIDAY_ART, TILE_SIZE, holidayBadgeSvg, holidayTile } from '../utils/holidayArt';
import CloseIcon from './Icons/CloseIcon.vue';

// Holiday postcard opened from the titlebar badge: a hero in the holiday's own
// color with its pattern and a centered solid icon, then dates, name and two sentences.

const { t, language } = useI18n();

const holiday = computed(() => activeHoliday.value);

const cardStyle = computed(() => {
  const id = holiday.value;
  if (!id) return {};
  const { h, s } = hexToHSL(HOLIDAY_ART[id].accent);
  return {
    '--hc-h': `${h}`,
    '--hc-s': `${s}%`,
    '--holiday-tile': holidayTile(id),
    '--holiday-tile-size': `${TILE_SIZE * 1.2}px`,
  };
});

const dates = computed(() => {
  const def = HOLIDAYS.find(h => h.id === holiday.value);
  if (!def) return '';
  const fmt = new Intl.DateTimeFormat(language.value === 'en' ? 'en-US' : 'ru-RU', { day: 'numeric', month: 'long' });
  // Any leap year works: only day and month are shown.
  const day = ([m, d]: [number, number]) => fmt.format(new Date(2028, m - 1, d));
  return `${day(def.from)} - ${day(def.to)}`;
});

function close() {
  holidayModalOpen.value = false;
}

function onBackdrop(e: MouseEvent) {
  if (e.target === e.currentTarget) close();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && holidayModalOpen.value) {
    e.preventDefault();
    close();
  }
}

// The holiday can end (or the setting be turned off) while the card is open.
watch(holiday, id => {
  if (!id) close();
});

onMounted(() => document.addEventListener('keydown', onKeydown));
onUnmounted(() => document.removeEventListener('keydown', onKeydown));
</script>

<template>
  <Teleport to="body">
    <Transition name="holidayModal">
      <div v-if="holidayModalOpen && holiday" class="holidayBackdrop" @mousedown="onBackdrop">
        <div class="holidayCard" :style="cardStyle" role="dialog" aria-modal="true" :aria-label="t(`holiday.${holiday}`)" @mousedown.stop>
          <div class="holidayHero">
            <div class="holidayHeroPattern" aria-hidden="true"></div>
            <button type="button" class="holidayClose" aria-label="Закрыть" @click="close"><CloseIcon /></button>
            <div class="holidayHeroIcon" v-html="holidayBadgeSvg(holiday, 64)"></div>
          </div>
          <div class="holidayBody">
            <div class="holidayDates">{{ dates }}</div>
            <div class="holidayTitle">{{ t(`holiday.${holiday}`) }}</div>
            <p class="holidayText">{{ t(`holiday.text.${holiday}`) }}</p>
            <button type="button" class="holidayButton" @click="close">{{ t('holiday.close') }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.holidayBackdrop {
  position: fixed;
  top: 0;
  left: 0;
  width: calc(100vw / var(--overlay-scale, 1));
  height: calc(100vh / var(--overlay-scale, 1));
  z-index: 1000001;
  display: flex;
  align-items: center;
  justify-content: center;
  background-color: hsla(0, 0%, 0%, 0.5);
  backdrop-filter: blur(6px);
}

.holidayCard {
  width: 360px;
  max-width: calc(100vw - 42px);
  border-radius: 26px;
  overflow: hidden;
  background: hsl(var(--hc-h), min(var(--hc-s), 30%), 9%);
  box-shadow:
    0 24px 64px rgba(0, 0, 0, 0.5),
    inset 0 0 0 1px hsla(var(--hc-h), 60%, 75%, 0.09);
  user-select: none;
}

.holidayHero {
  position: relative;
  height: 150px;
  background: radial-gradient(
    120% 100% at 50% 0%,
    hsl(var(--hc-h), var(--hc-s), 40%) 0%,
    hsl(var(--hc-h), var(--hc-s), 22%) 55%,
    hsl(var(--hc-h), min(var(--hc-s), 30%), 9%) 100%
  );
}

/* The holiday's own pattern, fading out towards the icon. */
.holidayHeroPattern {
  position: absolute;
  inset: 0;
  background: hsl(var(--hc-h), 80%, 90%);
  opacity: 0.2;
  -webkit-mask-image: linear-gradient(to bottom, #000 20%, transparent 100%), var(--holiday-tile);
  mask-image: linear-gradient(to bottom, #000 20%, transparent 100%), var(--holiday-tile);
  -webkit-mask-size: 100% 100%, var(--holiday-tile-size);
  mask-size: 100% 100%, var(--holiday-tile-size);
  -webkit-mask-repeat: no-repeat, repeat;
  mask-repeat: no-repeat, repeat;
  -webkit-mask-composite: source-in;
  mask-composite: intersect;
}

.holidayClose {
  position: absolute;
  top: 12px;
  right: 12px;
  width: 30px;
  height: 30px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: hsla(0, 0%, 100%, 0.8);
  background: hsla(0, 0%, 0%, 0.22);
  cursor: pointer;
  transition: background-color .2s ease, color .2s ease;
}
.holidayClose:hover {
  background: hsla(0, 0%, 0%, 0.38);
  color: #fff;
}

.holidayHeroIcon {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  pointer-events: none;
  color: hsl(var(--hc-h), 95%, 90%);
  filter: drop-shadow(0 8px 22px hsla(var(--hc-h), var(--hc-s), 50%, 0.55));
}

.holidayBody {
  padding: 20px 26px 24px;
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
}

.holidayDates {
  padding: 4px 11px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: .1em;
  text-transform: uppercase;
  color: hsl(var(--hc-h), 90%, 82%);
  background: hsla(var(--hc-h), 70%, 60%, 0.14);
}

.holidayTitle {
  margin-top: 12px;
  font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  font-size: 24px;
  line-height: 1.15;
  font-weight: 800;
  color: hsla(0, 0%, 100%, 0.96);
}

.holidayText {
  margin-top: 8px;
  max-width: 290px;
  font-size: 14px;
  line-height: 1.55;
  font-weight: 500;
  color: hsla(0, 0%, 100%, 0.62);
}

.holidayButton {
  margin-top: 22px;
  width: 100%;
  height: 42px;
  border-radius: 14px;
  background: hsl(var(--hc-h), var(--hc-s), 44%);
  color: #fff;
  font-size: 14px;
  font-weight: 700;
  cursor: pointer;
  transition: background-color .2s ease;
}
.holidayButton:hover {
  background: hsl(var(--hc-h), var(--hc-s), 50%);
}

.holidayModal-enter-active,
.holidayModal-leave-active {
  transition: opacity .22s ease;
}
.holidayModal-enter-active .holidayCard {
  transition: transform .32s cubic-bezier(.2, .9, .3, 1.15), opacity .22s ease;
}
.holidayModal-leave-active .holidayCard {
  transition: transform .18s ease, opacity .18s ease;
}
.holidayModal-enter-active .holidayHeroIcon {
  transition: transform .38s cubic-bezier(.2, .9, .3, 1.3) .06s;
}
.holidayModal-enter-from,
.holidayModal-leave-to {
  opacity: 0;
}
.holidayModal-enter-from .holidayCard,
.holidayModal-leave-to .holidayCard {
  transform: scale(0.94) translateY(8px);
  opacity: 0;
}
.holidayModal-enter-from .holidayHeroIcon {
  transform: scale(0.6);
}
</style>
