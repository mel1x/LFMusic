<script setup lang="ts">
import { computed } from 'vue';
import { activeHoliday } from '../composables/useHoliday';
import { TILE_SCALE, TILE_SIZE, holidayTile } from '../utils/holidayArt';

const style = computed(() => {
  const id = activeHoliday.value;
  if (!id) return null;
  return {
    '--holiday-tile': holidayTile(id),
    '--holiday-tile-size': `${TILE_SIZE * TILE_SCALE}px`,
  };
});
</script>

<template>
  <Transition name="holidayFade">
    <div v-if="style" :key="activeHoliday ?? ''" class="holidayPattern" :style="style" aria-hidden="true">
      <div class="holidayTile"></div>
    </div>
  </Transition>
</template>

<style scoped>
.holidayPattern {
  position: fixed;
  top: 0;
  left: 0;
  width: calc(100vw / var(--overlay-scale, 1));
  height: calc(100dvh / var(--overlay-scale, 1));
  z-index: -1;
  pointer-events: none;
}

.holidayTile {
  position: absolute;
  inset: 0;
  background: hsl(var(--accent-h), min(calc(var(--accent-s) + 16% * var(--accent-k, 1)), 100%), 68%);
  opacity: 0.075;
  -webkit-mask-image: var(--holiday-tile);
  mask-image: var(--holiday-tile);
  -webkit-mask-size: var(--holiday-tile-size);
  mask-size: var(--holiday-tile-size);
  -webkit-mask-repeat: repeat;
  mask-repeat: repeat;
}

.holidayFade-enter-active,
.holidayFade-leave-active {
  transition: opacity 0.6s var(--ease-smooth);
}
.holidayFade-enter-from,
.holidayFade-leave-to {
  opacity: 0;
}
</style>
