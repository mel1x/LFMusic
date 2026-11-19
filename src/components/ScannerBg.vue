<script setup lang="ts">
/*
 * Retro CRT scanner backdrop, adapted from the "Scanner" background of Vue Bits
 * (https://vue-bits.dev/backgrounds/scanner).
 * Copyright (c) 2025 David Haz. MIT + Commons Clause License Condition v1.0:
 * free to use as part of an application; the component itself may not be
 * sold or redistributed on its own.
 *
 * Changes: plain WebGL2 instead of ogl, no mouse interaction, colors follow
 * the app accent, music energy speeds up the sweep and lifts the glow,
 * ~30fps at CSS pixel resolution.
 */
import { onMounted, onUnmounted, ref } from 'vue'
import { frameLoop } from '../utils/frameLoop'

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
uniform vec3 uColor1;
uniform vec3 uColor2;
uniform vec3 uColor3;
out vec4 fragColor;

const float TAU = 6.2831853;
const float SCALE = 1.5;
const float FREQUENCY = 2.0;
const float RIPPLE = 0.22;
const float SWEEP_WIDTH = 1.6;
const float SWEEP_FALLOFF = 6.0;
const float BAND_DENSITY = 11.0;
const float LINE_SHARPNESS = 5.5;
const float GLOW = 0.22;
const float COLOR_SPREAD = 0.7;
const float CONTRAST = 1.15;
const float SOFTNESS = 1.4;
const float VIGNETTE = 0.45;
const float GRAIN = 0.05;

float signalField(vec2 p, float t) {
  float w = sin(p.x * 1.3 + t * 0.7);
  w += sin(p.y * 1.7 - t * 0.52) * 0.8;
  w += sin((p.x + p.y) * 0.9 + t * 0.91) * 0.6;
  w += sin((p.x - p.y) * 1.53 - t * 0.63) * 0.42;
  return w * 0.35;
}

vec3 palette(float f) {
  f = pow(clamp(f, 0.0, 1.0), CONTRAST);
  vec3 c = mix(uColor1, uColor2, smoothstep(0.08, 0.6, f));
  return mix(c, uColor3, smoothstep(0.68, 1.0, f));
}

float scanBand(float x, float aa, float sharp) {
  float v = mix(0.5, 0.5 + 0.5 * cos(x * TAU), aa);
  return pow(v, sharp);
}

void main() {
  vec2 uv0 = (gl_FragCoord.xy * 2.0 - iResolution.xy) / iResolution.y;
  vec2 p = uv0 / SCALE;
  float t = iTime;

  float sig = signalField(p * FREQUENCY, t);
  float coord = p.y + sig * RIPPLE;

  // iTime is already scaled by speed and music energy in the render loop.
  float phase = coord / SWEEP_WIDTH - t * 0.25;
  float sweep = pow(0.5 + 0.5 * cos(phase * TAU), SWEEP_FALLOFF);

  float lc = coord * BAND_DENSITY;
  float aa = clamp(1.0 / (1.0 + SOFTNESS * fwidth(lc) * 3.0), 0.0, 1.0);

  float bodyBase = clamp(0.5 + 0.5 * sig, 0.0, 1.0);
  float body = bodyBase * bodyBase * (GLOW + uEnergy * 0.18) * sweep;

  float split = COLOR_SPREAD * 0.16;
  float fr = clamp(scanBand(lc + split, aa, LINE_SHARPNESS) * sweep + body, 0.0, 1.0);
  float fg = clamp(scanBand(lc, aa, LINE_SHARPNESS) * sweep + body, 0.0, 1.0);
  float fb = clamp(scanBand(lc - split, aa, LINE_SHARPNESS) * sweep + body, 0.0, 1.0);

  vec3 col = vec3(palette(fr).r, palette(fg).g, palette(fb).b);

  float inten = (fr + fg + fb) * 0.3333333 * (1.0 + uEnergy * 0.35);
  inten *= 1.0 - 0.18 * (0.5 + 0.5 * cos(gl_FragCoord.y * 1.7));
  float g = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233)) + iTime) * 43758.5453);
  inten += (g - 0.5) * GRAIN;
  inten *= clamp(1.0 - VIGNETTE * smoothstep(0.55, 1.65, length(uv0)), 0.0, 1.0);
  inten = clamp(inten, 0.0, 1.0);

  fragColor = vec4(clamp(col, 0.0, 1.0) * inten, inten);
}
`

const canvasEl = ref<HTMLCanvasElement | null>(null)

let gl: WebGL2RenderingContext | null = null
let prog: WebGLProgram | null = null
let uRes: WebGLUniformLocation | null = null
let uTime: WebGLUniformLocation | null = null
let uEnergy: WebGLUniformLocation | null = null
let uColors: (WebGLUniformLocation | null)[] = []
let time = 0
let energy = 0
let targetEnergy = 0
let lastColorRead = -Infinity
let accentKey = ''
let visible = true
let resizeObserver: ResizeObserver | null = null
let intersectionObserver: IntersectionObserver | null = null
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches

/** ~30fps: the sweep is slow, and every frame shades the whole panel. */
const FRAME_MS = 33
/** Base animation speed (vue-bits `speed`); the sweep runs at half of it. */
const SPEED = 0.5

/** Called by the page with the current music energy (0..1). */
function setEnergy(e: number) {
    targetEnergy = Math.min(1, Math.max(0, e))
}
defineExpose({ setEnergy })

function hslToRgb(h: number, s: number, l: number): [number, number, number] {
    s /= 100
    l /= 100
    const k = (n: number) => (n + h / 30) % 12
    const a = s * Math.min(l, 1 - l)
    const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)))
    return [f(0), f(8), f(4)]
}

/** Three palette stops from the live accent: deep base, light tint, near white. */
function applyAccentColors() {
    if (!gl) return
    const style = document.documentElement.style
    const rawH = parseFloat(style.getPropertyValue('--accent-h'))
    const rawS = parseFloat(style.getPropertyValue('--accent-s'))
    const h = Number.isFinite(rawH) ? rawH : 265
    const s = Number.isFinite(rawS) ? rawS : 60
    const key = `${h.toFixed(1)}:${s.toFixed(1)}`
    if (key === accentKey) return
    accentKey = key
    const sat = Math.min(100, Math.max(s, 8) * 1.35)
    const stops = [
        hslToRgb(h, sat, 52),
        hslToRgb((h + 28) % 360, Math.min(100, sat * 0.9), 80),
        hslToRgb(h, Math.min(sat, 30), 96),
    ]
    stops.forEach((c, i) => gl!.uniform3f(uColors[i], c[0], c[1], c[2]))
}

function resize() {
    const canvas = canvasEl.value
    if (!canvas || !gl) return
    const rect = canvas.getBoundingClientRect()
    const dpr = Math.min(window.devicePixelRatio || 1, 1)
    const w = Math.max(1, Math.round(rect.width * dpr))
    const h = Math.max(1, Math.round(rect.height * dpr))
    if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w
        canvas.height = h
    }
    gl.viewport(0, 0, w, h)
    gl.uniform2f(uRes, w, h)
    draw()
}

function draw() {
    if (!gl) return
    gl.uniform1f(uTime, time)
    gl.uniform1f(uEnergy, energy)
    gl.clear(gl.COLOR_BUFFER_BIT)
    gl.drawArrays(gl.TRIANGLES, 0, 3)
}

const loop = frameLoop((now, frameDt) => {
    const dt = Math.min(frameDt || FRAME_MS, 100)
    const steps = dt / (1000 / 60)
    energy += (targetEnergy - energy) * (1 - Math.pow(0.88, steps))
    time += dt / 1000 * SPEED * (1 + energy * 1.4)
    if (now - lastColorRead > 200) {
        lastColorRead = now
        applyAccentColors()
    }
    draw()
}, FRAME_MS)

function start() {
    if (loop.running || reducedMotion || !gl || !visible || document.hidden) return
    loop.start()
}

function stop() {
    loop.stop()
}

function onVisibility() {
    if (document.hidden) stop()
    else start()
}

function compile(type: number, src: string): WebGLShader | null {
    const sh = gl!.createShader(type)
    if (!sh) return null
    gl!.shaderSource(sh, src)
    gl!.compileShader(sh)
    if (!gl!.getShaderParameter(sh, gl!.COMPILE_STATUS)) {
        console.error('Scanner shader:', gl!.getShaderInfoLog(sh))
        return null
    }
    return sh
}

onMounted(() => {
    const canvas = canvasEl.value
    if (!canvas) return
    gl = canvas.getContext('webgl2', { alpha: true, premultipliedAlpha: true, antialias: false })
    if (!gl) return

    const vs = compile(gl.VERTEX_SHADER, VERT)
    const fs = compile(gl.FRAGMENT_SHADER, FRAG)
    if (!vs || !fs) return
    prog = gl.createProgram()
    gl.attachShader(prog, vs)
    gl.attachShader(prog, fs)
    gl.linkProgram(prog)
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
        console.error('Scanner program:', gl.getProgramInfoLog(prog))
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
    uColors = ['uColor1', 'uColor2', 'uColor3'].map(n => gl!.getUniformLocation(prog!, n))
    gl.clearColor(0, 0, 0, 0)

    time = 1.2
    applyAccentColors()

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
    <canvas ref="canvasEl" class="scannerBg" aria-hidden="true"></canvas>
</template>

<style scoped>
.scannerBg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
    pointer-events: none;
}
</style>
