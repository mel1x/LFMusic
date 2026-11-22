<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { frameLoop } from '../../utils/frameLoop'
import type { BgScene } from './recapCards'

const props = withDefaults(defineProps<{
  scene: BgScene
  hue: number
  sat: number
  hueShift?: number
  paused?: boolean
  getEnergy?: () => number
}>(), {
  hueShift: 36,
  paused: false,
})

const SCENE_ID: Record<BgScene, number> = {
  silk: 0,
  aurora: 1,
  scanner: 2,
  prism: 3,
  beams: 4,
  vortex: 5,
}

const VERT = `#version 300 es
in vec2 position;
void main() {
  gl_Position = vec4(position, 0.0, 1.0);
}
`

const FRAG = `#version 300 es
precision highp float;

uniform vec2 iResolution;
uniform float iTime;
uniform float uEnergy;
uniform int uModeA;
uniform int uModeB;
uniform float uMix;
uniform vec3 uBase;
uniform vec3 uColor1;
uniform vec3 uColor2;
uniform vec3 uColor3;

out vec4 fragColor;

const float TAU = 6.2831853;

float hash21(vec2 p) {
  p = fract(p * vec2(123.34, 456.21));
  p += dot(p, p + 45.32);
  return fract(p.x * p.y);
}

float vnoise(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  float a = hash21(i);
  float b = hash21(i + vec2(1.0, 0.0));
  float c = hash21(i + vec2(0.0, 1.0));
  float d = hash21(i + vec2(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

mat2 rot(float a) {
  float c = cos(a);
  float s = sin(a);
  return mat2(c, -s, s, c);
}

float fbm(vec2 p) {
  float v = 0.0;
  float amp = 0.5;
  mat2 m = mat2(0.8, 0.6, -0.6, 0.8);
  for (int i = 0; i < 4; i++) {
    v += amp * vnoise(p);
    p = m * p * 2.02 + vec2(1.7, 9.2);
    amp *= 0.5;
  }
  return v;
}

vec3 grad3(float t) {
  t = clamp(t, 0.0, 1.0);
  vec3 c = mix(uColor1, uColor2, smoothstep(0.08, 0.62, t));
  return mix(c, uColor3, smoothstep(0.65, 1.0, t));
}

// 0: Liquid silk ribbons (Color Bends / Liquid Ether inspired)
vec3 sceneSilk(vec2 uv, float t, float energy) {
  vec2 p = uv * 1.18;
  p = rot(-0.28) * p;

  vec2 q = vec2(
    fbm(p + vec2(t * 0.16, -t * 0.11)),
    fbm(p + vec2(-t * 0.14 + 3.2, t * 0.13 + 1.4))
  );
  vec2 r = vec2(
    fbm(p + 2.3 * q + vec2(1.7, 9.2) + t * 0.12),
    fbm(p + 2.3 * q + vec2(8.3, 2.8) - t * 0.10)
  );
  float f = fbm(p + 1.85 * r);

  float ribbon = sin((p.x * 2.4 + p.y * 1.4 + f * 5.2 - t * 0.55));
  float crest = pow(0.5 + 0.5 * ribbon, 3.2);
  float glow = smoothstep(0.22, 0.92, f);

  vec3 col = mix(uBase, uColor1, glow * 0.72);
  col = mix(col, uColor2, crest * (0.58 + energy * 0.22));
  col += uColor3 * pow(crest, 4.0) * (0.28 + energy * 0.22);
  return col;
}

// 1: Northern veil (Aurora / Dark Veil inspired)
vec3 sceneAurora(vec2 uv, float t, float energy) {
  vec2 p = uv;
  float drift = t * 0.22;

  float w1 = sin(p.x * 2.4 + drift * 1.1 + fbm(vec2(p.x * 1.6, drift * 0.45)) * 2.8);
  float band1 = exp(-abs(p.y - 0.18 - w1 * 0.26) * 3.1);

  float w2 = sin(p.x * 3.3 - drift * 0.9 + fbm(vec2(p.x * 2.1 + 4.0, drift * 0.38)) * 2.4);
  float band2 = exp(-abs(p.y + 0.16 - w2 * 0.22) * 3.6);

  float curtain = fbm(vec2(p.x * 4.5 + drift * 0.4, p.y * 1.2 - drift * 0.7));
  float shimmer = 0.65 + 0.35 * sin(p.x * 11.0 + curtain * 5.5 - t * 0.9);

  float a1 = band1 * (0.45 + 0.75 * curtain) * shimmer;
  float a2 = band2 * (0.4 + 0.7 * (1.0 - curtain));

  vec3 col = uBase * 0.9;
  col += uColor1 * a1 * (0.85 + energy * 0.3);
  col += uColor2 * a2 * (0.72 + energy * 0.28);
  col += uColor3 * pow(clamp(a1 + a2 * 0.7, 0.0, 1.0), 3.0) * (0.34 + energy * 0.2);
  return col;
}

// 2: Harmonic CRT scanner (sister to Main's ScannerBg)
vec3 sceneScanner(vec2 uv, float t, float energy) {
  vec2 p = uv / 1.35;
  float w = sin(p.x * 1.4 + t * 0.65);
  w += sin(p.y * 1.8 - t * 0.48) * 0.8;
  w += sin((p.x + p.y) * 0.95 + t * 0.82) * 0.6;
  w += sin((p.x - p.y) * 1.5 - t * 0.56) * 0.42;
  float sig = w * 0.35;

  float coord = p.y + sig * 0.24;
  float phase = coord / 1.55 - t * 0.22;
  float sweep = pow(0.5 + 0.5 * cos(phase * TAU), 5.2);

  float lc = coord * 10.5;
  float aa = clamp(1.0 / (1.0 + 1.4 * fwidth(lc) * 3.0), 0.0, 1.0);

  float bodyBase = clamp(0.5 + 0.5 * sig, 0.0, 1.0);
  float body = bodyBase * bodyBase * (0.26 + energy * 0.22) * sweep;

  float split = 0.11;
  float vr = mix(0.5, 0.5 + 0.5 * cos((lc + split) * TAU), aa);
  float vg = mix(0.5, 0.5 + 0.5 * cos(lc * TAU), aa);
  float vb = mix(0.5, 0.5 + 0.5 * cos((lc - split) * TAU), aa);

  float fr = clamp(pow(vr, 5.0) * sweep + body, 0.0, 1.0);
  float fg = clamp(pow(vg, 5.0) * sweep + body, 0.0, 1.0);
  float fb = clamp(pow(vb, 5.0) * sweep + body, 0.0, 1.0);

  vec3 band = vec3(grad3(fr).r, grad3(fg).g, grad3(fb).b);
  float inten = (fr + fg + fb) * 0.3333333 * (1.0 + energy * 0.35);
  inten *= 1.0 - 0.15 * (0.5 + 0.5 * cos(gl_FragCoord.y * 1.6));

  return mix(uBase, band, clamp(inten * 0.95, 0.0, 1.0));
}

// 3: Liquid iridescence (Iridescence / Liquid Chrome inspired)
vec3 scenePrism(vec2 uv, float t, float energy) {
  vec2 p = uv * 1.35;
  float d = -t * 0.28;
  float a = 0.0;
  for (int i = 0; i < 4; i++) {
    float fi = float(i) + 1.0;
    a += cos(fi - d - a * p.x);
    d += sin(p.y * fi + a);
  }
  d += t * 0.24;

  float fold = 0.5 + 0.5 * sin(d * 1.45 + a * 0.85);
  float ridge = pow(clamp(0.5 + 0.5 * cos(p.x * 2.0 + d - a), 0.0, 1.0), 3.0);
  float spec = pow(fold * ridge, 1.6);

  vec3 col = mix(uBase, uColor1, smoothstep(0.15, 0.88, fold) * 0.78);
  col = mix(col, uColor2, ridge * (0.55 + energy * 0.2));
  col += uColor3 * spec * (0.48 + energy * 0.28);
  return col;
}

// 4: Stage light pillars (Beams / Light Rays inspired)
vec3 sceneBeams(vec2 uv, float t, float energy) {
  vec2 origin = vec2(0.08 * sin(t * 0.3), 1.38);
  vec2 d = uv - origin;
  float dist = length(d);
  float angle = atan(d.x, -d.y);

  float rays = 0.0;
  rays += pow(0.5 + 0.5 * sin(angle * 7.0 + t * 0.42), 3.0) * 0.55;
  rays += pow(0.5 + 0.5 * sin(angle * 13.0 - t * 0.31 + 1.4), 4.0) * 0.35;
  rays += pow(0.5 + 0.5 * sin(angle * 21.0 + t * 0.55 + 2.7), 5.0) * 0.22;

  float haze = fbm(vec2(uv.x * 2.2 + t * 0.14, uv.y * 1.6 - t * 0.18));
  float falloff = exp(-dist * 0.72) * smoothstep(-1.1, 0.9, uv.y + 0.35);

  float beam = rays * falloff * (0.65 + 0.65 * haze);
  float core = exp(-dist * 1.45) * (0.42 + energy * 0.24);

  vec3 col = uBase * 0.92;
  col += uColor1 * (beam * 0.95 + haze * 0.16);
  col += uColor2 * (beam * haze * 0.85 + core * 0.65);
  col += uColor3 * (pow(clamp(beam, 0.0, 1.0), 2.4) * 0.42 + core * 0.35) * (1.0 + energy * 0.35);
  return col;
}

// 5: Sonic vortex / vinyl warp (Balatro / Ripple inspired)
vec3 sceneVortex(vec2 uv, float t, float energy) {
  vec2 c = uv - vec2(0.0, 0.06);
  float r = length(c);
  float a = atan(c.y, c.x);

  float swirl = a + 2.4 / (r + 0.42) - t * 0.38;
  vec2 wp = vec2(cos(swirl), sin(swirl)) * r;

  float n = fbm(wp * 2.2 + vec2(t * 0.15, -t * 0.12));
  float bands = 0.5 + 0.5 * sin(swirl * 3.0 + n * 4.2);
  float rings = pow(0.5 + 0.5 * cos(r * 15.0 - t * 1.35 + n * 2.0), 4.0) * exp(-r * 0.75);

  vec3 col = mix(uBase, uColor1, smoothstep(0.18, 0.88, bands) * 0.76);
  col = mix(col, uColor2, smoothstep(0.35, 0.95, n) * (0.52 + energy * 0.2));
  col += uColor3 * rings * (0.42 + energy * 0.32);
  col += uColor2 * exp(-r * 2.6) * (0.28 + energy * 0.22);
  return col;
}

vec3 evalScene(int mode, vec2 uv, float t, float energy) {
  if (mode == 0) return sceneSilk(uv, t, energy);
  if (mode == 1) return sceneAurora(uv, t, energy);
  if (mode == 2) return sceneScanner(uv, t, energy);
  if (mode == 3) return scenePrism(uv, t, energy);
  if (mode == 4) return sceneBeams(uv, t, energy);
  return sceneVortex(uv, t, energy);
}

void main() {
  vec2 uv = (gl_FragCoord.xy * 2.0 - iResolution.xy) / iResolution.y;
  float t = iTime;

  vec3 col;
  if (uMix >= 0.999 || uModeA == uModeB) {
    col = evalScene(uModeB, uv, t, uEnergy);
  } else {
    float k = uMix * uMix * (3.0 - 2.0 * uMix);
    col = mix(evalScene(uModeA, uv, t, uEnergy), evalScene(uModeB, uv, t, uEnergy), k);
  }

  // Soft legibility shaping: keep top controls and bottom content grounded while center breathes.
  float vig = 1.0 - 0.38 * smoothstep(0.55, 1.55, length(uv * vec2(1.05, 0.78)));
  float topShade = mix(0.76, 1.0, smoothstep(1.02, 0.35, uv.y));
  float botShade = mix(0.66, 1.0, smoothstep(-1.05, -0.25, uv.y));
  col *= vig * topShade * botShade;

  // Fine film grain to avoid banding on dark tones.
  float g = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233)) + iTime * 1.7) * 43758.5453);
  col += (g - 0.5) * 0.032;

  fragColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
`

const canvasEl = ref<HTMLCanvasElement | null>(null)
const webglReady = ref(true)

let gl: WebGL2RenderingContext | null = null
let prog: WebGLProgram | null = null
let uRes: WebGLUniformLocation | null = null
let uTime: WebGLUniformLocation | null = null
let uEnergy: WebGLUniformLocation | null = null
let uModeA: WebGLUniformLocation | null = null
let uModeB: WebGLUniformLocation | null = null
let uMix: WebGLUniformLocation | null = null
let uBase: WebGLUniformLocation | null = null
let uColor1: WebGLUniformLocation | null = null
let uColor2: WebGLUniformLocation | null = null
let uColor3: WebGLUniformLocation | null = null

let time = 1.4
let energy = 0
let modeA = SCENE_ID[props.scene]
let modeB = SCENE_ID[props.scene]
let mixT = 1

type Rgb = [number, number, number]
interface Palette {
  base: Rgb
  c1: Rgb
  c2: Rgb
  c3: Rgb
}

function hslToRgb(h: number, s: number, l: number): Rgb {
  h = ((h % 360) + 360) % 360
  s /= 100
  l /= 100
  const k = (n: number) => (n + h / 30) % 12
  const a = s * Math.min(l, 1 - l)
  const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)))
  return [f(0), f(8), f(4)]
}

function buildPalette(h: number, s: number, shift: number): Palette {
  const sat = Math.min(92, Math.max(s, 28) * 1.2)
  return {
    base: hslToRgb(h, Math.min(sat * 0.42, 34), 7.5),
    c1: hslToRgb(h, sat, 44),
    c2: hslToRgb(h + shift, Math.min(95, sat * 0.96), 58),
    c3: hslToRgb(h + shift * 0.5, Math.min(sat, 62), 88),
  }
}

function clonePalette(p: Palette): Palette {
  return {
    base: [...p.base],
    c1: [...p.c1],
    c2: [...p.c2],
    c3: [...p.c3],
  }
}

let curPal: Palette = buildPalette(props.hue, props.sat, props.hueShift)
let targetPal: Palette = clonePalette(curPal)

const FRAME_MS = 33
const TRANSITION_SECS = 0.65
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches

let visible = true
let resizeObserver: ResizeObserver | null = null
let intersectionObserver: IntersectionObserver | null = null

function lerpRgb(cur: Rgb, dst: Rgb, alpha: number) {
  cur[0] += (dst[0] - cur[0]) * alpha
  cur[1] += (dst[1] - cur[1]) * alpha
  cur[2] += (dst[2] - cur[2]) * alpha
}

function uploadUniforms() {
  if (!gl) return
  gl.uniform1f(uTime, time)
  gl.uniform1f(uEnergy, energy)
  gl.uniform1i(uModeA, modeA)
  gl.uniform1i(uModeB, modeB)
  gl.uniform1f(uMix, mixT)
  gl.uniform3f(uBase, curPal.base[0], curPal.base[1], curPal.base[2])
  gl.uniform3f(uColor1, curPal.c1[0], curPal.c1[1], curPal.c1[2])
  gl.uniform3f(uColor2, curPal.c2[0], curPal.c2[1], curPal.c2[2])
  gl.uniform3f(uColor3, curPal.c3[0], curPal.c3[1], curPal.c3[2])
}

function draw() {
  if (!gl) return
  uploadUniforms()
  gl.drawArrays(gl.TRIANGLES, 0, 3)
}

function resize() {
  const canvas = canvasEl.value
  if (!canvas || !gl) return
  const dpr = Math.min(window.devicePixelRatio || 1, 1.5)
  const w = Math.max(1, Math.round(canvas.clientWidth * dpr))
  const h = Math.max(1, Math.round(canvas.clientHeight * dpr))
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w
    canvas.height = h
  }
  gl.viewport(0, 0, w, h)
  gl.uniform2f(uRes, w, h)
  draw()
}

const loop = frameLoop((_now, frameDt) => {
  const dt = Math.min(frameDt || FRAME_MS, 100) / 1000
  const rawEnergy = props.paused ? 0 : (props.getEnergy?.() ?? 0)
  const steps = (dt * 1000) / (1000 / 60)
  energy += (rawEnergy - energy) * (1 - Math.pow(0.86, steps))

  if (!props.paused && !reducedMotion) {
    time += dt * (0.78 + energy * 0.95)
  }

  if (mixT < 1) {
    mixT = Math.min(1, mixT + dt / TRANSITION_SECS)
  }

  const colorAlpha = 1 - Math.exp(-dt * 5.2)
  lerpRgb(curPal.base, targetPal.base, colorAlpha)
  lerpRgb(curPal.c1, targetPal.c1, colorAlpha)
  lerpRgb(curPal.c2, targetPal.c2, colorAlpha)
  lerpRgb(curPal.c3, targetPal.c3, colorAlpha)

  draw()
}, FRAME_MS)

function start() {
  if (loop.running || !gl || !visible || document.hidden) return
  if (reducedMotion) {
    curPal = clonePalette(targetPal)
    mixT = 1
    draw()
    return
  }
  loop.start()
}

function stop() {
  loop.stop()
}

function onVisibility() {
  if (document.hidden) stop()
  else start()
}

watch(() => props.scene, (next) => {
  const nextId = SCENE_ID[next]
  if (nextId === modeB) return
  modeA = modeB
  modeB = nextId
  mixT = reducedMotion ? 1 : 0
  if (reducedMotion) draw()
})

watch([() => props.hue, () => props.sat, () => props.hueShift], ([h, s, shift]) => {
  targetPal = buildPalette(h, s, shift)
  if (reducedMotion) {
    curPal = clonePalette(targetPal)
    draw()
  }
})

function compile(type: number, src: string): WebGLShader | null {
  const sh = gl!.createShader(type)
  if (!sh) return null
  gl!.shaderSource(sh, src)
  gl!.compileShader(sh)
  if (!gl!.getShaderParameter(sh, gl!.COMPILE_STATUS)) {
    console.error('RecapBg shader:', gl!.getShaderInfoLog(sh))
    return null
  }
  return sh
}

onMounted(() => {
  const canvas = canvasEl.value
  if (!canvas) return
  gl = canvas.getContext('webgl2', { alpha: false, antialias: false, powerPreference: 'default' })
  if (!gl) {
    webglReady.value = false
    return
  }

  const vs = compile(gl.VERTEX_SHADER, VERT)
  const fs = compile(gl.FRAGMENT_SHADER, FRAG)
  if (!vs || !fs) {
    webglReady.value = false
    gl = null
    return
  }

  prog = gl.createProgram()
  gl.attachShader(prog, vs)
  gl.attachShader(prog, fs)
  gl.linkProgram(prog)
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    console.error('RecapBg program:', gl.getProgramInfoLog(prog))
    webglReady.value = false
    gl = null
    return
  }
  gl.useProgram(prog)

  const buf = gl.createBuffer()
  gl.bindBuffer(gl.ARRAY_BUFFER, buf)
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW)
  const loc = gl.getAttribLocation(prog, 'position')
  gl.enableVertexAttribArray(loc)
  gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0)

  uRes = gl.getUniformLocation(prog, 'iResolution')
  uTime = gl.getUniformLocation(prog, 'iTime')
  uEnergy = gl.getUniformLocation(prog, 'uEnergy')
  uModeA = gl.getUniformLocation(prog, 'uModeA')
  uModeB = gl.getUniformLocation(prog, 'uModeB')
  uMix = gl.getUniformLocation(prog, 'uMix')
  uBase = gl.getUniformLocation(prog, 'uBase')
  uColor1 = gl.getUniformLocation(prog, 'uColor1')
  uColor2 = gl.getUniformLocation(prog, 'uColor2')
  uColor3 = gl.getUniformLocation(prog, 'uColor3')

  resizeObserver = new ResizeObserver(resize)
  resizeObserver.observe(canvas)
  resize()

  intersectionObserver = new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting
    if (visible) start()
    else stop()
  })
  intersectionObserver.observe(canvas)
  document.addEventListener('visibilitychange', onVisibility)
  start()
})

onUnmounted(() => {
  stop()
  resizeObserver?.disconnect()
  intersectionObserver?.disconnect()
  document.removeEventListener('visibilitychange', onVisibility)
  gl?.getExtension('WEBGL_lose_context')?.loseContext()
  gl = null
})
</script>

<template>
  <div class="recapBgWrap" :class="{ fallback: !webglReady }" aria-hidden="true">
    <canvas v-if="webglReady" ref="canvasEl" class="recapBgCanvas"></canvas>
  </div>
</template>

<style scoped>
.recapBgWrap {
  position: absolute;
  inset: 0;
  pointer-events: none;
  z-index: 0;
  overflow: hidden;
  background: hsl(var(--rc-h, 250), min(var(--rc-s, 45%), 28%), 8%);
}

.recapBgCanvas {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  display: block;
}

.recapBgWrap.fallback {
  background:
    radial-gradient(120% 65% at 25% 15%, hsla(var(--rc-h, 250), var(--rc-s, 45%), 52%, 0.42), transparent 64%),
    radial-gradient(100% 60% at 85% 80%, hsla(calc(var(--rc-h, 250) + 38), var(--rc-s, 45%), 46%, 0.34), transparent 68%),
    hsl(var(--rc-h, 250), min(var(--rc-s, 45%), 28%), 7%);
}
</style>
