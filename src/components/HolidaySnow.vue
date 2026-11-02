<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { currentUiScale } from '../composables/useUiScale';
import { isPhoneViewport } from '../composables/usePlatform';
import { PATHS } from '../utils/holidayArt';

// New Year snow over the whole window (click-through): soft dots with a few
// rotating snowflakes. One canvas, one rAF loop; paused while the window is hidden.

interface Flake {
  x: number;
  y: number;
  vy: number;
  size: number;
  rot: number;
  vr: number;
  alpha: number;
  swayAmp: number;
  swayFreq: number;
  phase: number;
  /** Drawn as a snowflake glyph instead of a dot. */
  glyph: boolean;
}

/** Flakes per 100k px^2 of window. */
const DENSITY = 3.2;
/** ~30 fps: plenty for slow snow, and a full-window canvas redraw isn't free on 120/144Hz screens. */
const FRAME_MS = 30;

const rand = (a: number, b: number) => a + Math.random() * (b - a);

const canvasRef = ref<HTMLCanvasElement | null>(null);
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

let flakes: Flake[] = [];
let ctx: CanvasRenderingContext2D | null = null;
let flakePath: Path2D | null = null;
let w = 0;
let h = 0;
let pxRatio = 1;
let raf: number | null = null;
let last = 0;
let time = 0;
let resizeObserver: ResizeObserver | null = null;

// Anywhere on the first fill, otherwise just above the window.
function spawn(initial: boolean): Flake {
  const glyph = Math.random() < 0.14;
  const size = glyph ? rand(8, 13) : rand(2, 5.5);
  return {
    x: rand(0, w),
    y: initial ? rand(-20, h) : rand(-40, -10),
    vy: glyph ? rand(22, 36) : 12 + size * rand(4, 7),
    size,
    rot: rand(0, Math.PI),
    vr: glyph ? rand(-0.6, 0.6) : 0,
    alpha: rand(0.25, 0.7),
    swayAmp: rand(8, 26),
    swayFreq: rand(0.4, 1.1),
    phase: rand(0, Math.PI * 2),
    glyph,
  };
}

function resize() {
  const canvas = canvasRef.value;
  if (!canvas) return;
  const prevW = w, prevH = h;
  w = canvas.clientWidth;
  h = canvas.clientHeight;
  pxRatio = Math.min(2.5, (window.devicePixelRatio || 1) * currentUiScale());
  canvas.width = Math.round(w * pxRatio);
  canvas.height = Math.round(h * pxRatio);
  // Keep the scene spread over the new size instead of refilling it.
  if (prevW > 0 && prevH > 0) {
    for (const f of flakes) {
      f.x *= w / prevW;
      f.y *= h / prevH;
    }
  }
  const target = Math.max(6, Math.min(70, Math.round((w * h) / 100000 * DENSITY * (isPhoneViewport() ? 0.6 : 1))));
  if (flakes.length > target) flakes.length = target;
  while (flakes.length < target) flakes.push(spawn(true));
}

function frame(now: number) {
  raf = requestAnimationFrame(frame);
  if (last && now - last < FRAME_MS) return;
  const dt = last ? Math.min((now - last) / 1000, 0.05) : 0;
  last = now;
  time += dt;
  const c = ctx;
  if (!c) return;

  c.setTransform(1, 0, 0, 1, 0, 0);
  c.clearRect(0, 0, c.canvas.width, c.canvas.height);
  c.fillStyle = '#eaf6ff';
  c.strokeStyle = '#eaf6ff';
  c.lineCap = 'round';

  for (let i = 0; i < flakes.length; i++) {
    let f = flakes[i];
    f.y += f.vy * dt;
    f.rot += f.vr * dt;
    if (f.y > h + 30) f = flakes[i] = spawn(false);

    const x = f.x + f.swayAmp * Math.sin(f.phase + time * f.swayFreq);
    c.setTransform(pxRatio, 0, 0, pxRatio, 0, 0);
    c.globalAlpha = f.alpha;
    if (f.glyph && flakePath) {
      const k = f.size / 24;
      c.translate(x, f.y);
      c.rotate(f.rot);
      c.scale(k, k);
      c.translate(-12, -12);
      c.lineWidth = 1.8;
      c.stroke(flakePath);
    } else {
      c.beginPath();
      c.arc(x, f.y, f.size / 2, 0, Math.PI * 2);
      c.fill();
    }
  }
}

function start() {
  if (raf !== null) return;
  last = 0;
  raf = requestAnimationFrame(frame);
}

function stop() {
  if (raf !== null) cancelAnimationFrame(raf);
  raf = null;
}

function onVisibility() {
  if (document.hidden) stop();
  else start();
}

onMounted(() => {
  const canvas = canvasRef.value;
  if (!canvas) return;
  ctx = canvas.getContext('2d');
  if (!ctx) return;
  flakePath = new Path2D(PATHS.flake);
  resize();
  resizeObserver = new ResizeObserver(resize);
  resizeObserver.observe(canvas);
  document.addEventListener('visibilitychange', onVisibility);
  if (!document.hidden) start();
});

onBeforeUnmount(() => {
  stop();
  resizeObserver?.disconnect();
  document.removeEventListener('visibilitychange', onVisibility);
  flakes = [];
});
</script>

<template>
  <canvas v-if="!reducedMotion" ref="canvasRef" class="holidaySnow" aria-hidden="true"></canvas>
</template>

<style scoped>
/* Above the fullscreen player (99999), below modals and menus (1000000+). */
.holidaySnow {
  position: fixed;
  top: 0;
  left: 0;
  width: calc(100vw / var(--overlay-scale, 1));
  height: calc(100dvh / var(--overlay-scale, 1));
  z-index: 100005;
  pointer-events: none;
}
</style>
