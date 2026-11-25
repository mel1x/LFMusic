<script setup lang="ts">
/*
 * Procedural WebGL2 slide backdrops for Year Recap.
 * `sceneHalftone` is adapted from uibrik's Halftone background
 * (https://uibrik.com/components/halftone, free for commercial use) and uses
 * 3D simplex noise by Ian McEwan, Ashima Arts, and Stefan Gustavson (MIT).
 */
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
  hueShift: 22,
  paused: false,
})

const SCENE_ID: Record<BgScene, number> = {
  silk: 0,
  aurora: 1,
  scanner: 2,
  prism: 3,
  beams: 4,
  halftone: 5,
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
uniform float uDpr;
uniform vec2 uPointer;
uniform float uPointerAmt;
uniform vec3 uTrailPts[16];
uniform int uTrailCount;
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

// 3D simplex noise by Ian McEwan, Ashima Arts, and Stefan Gustavson (MIT).
vec3 mod289(vec3 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec4 mod289(vec4 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec4 permute(vec4 x) { return mod289(((x * 34.0) + 1.0) * x); }
vec4 taylorInvSqrt(vec4 r) { return 1.79284291400159 - 0.85373472095314 * r; }
float snoise(vec3 v) {
  const vec2 C = vec2(1.0 / 6.0, 1.0 / 3.0);
  const vec4 D = vec4(0.0, 0.5, 1.0, 2.0);
  vec3 i = floor(v + dot(v, C.yyy));
  vec3 x0 = v - i + dot(i, C.xxx);
  vec3 g = step(x0.yzx, x0.xyz);
  vec3 l = 1.0 - g;
  vec3 i1 = min(g.xyz, l.zxy);
  vec3 i2 = max(g.xyz, l.zxy);
  vec3 x1 = x0 - i1 + C.xxx;
  vec3 x2 = x0 - i2 + C.yyy;
  vec3 x3 = x0 - D.yyy;
  i = mod289(i);
  vec4 p = permute(permute(permute(i.z + vec4(0.0, i1.z, i2.z, 1.0)) + i.y + vec4(0.0, i1.y, i2.y, 1.0)) + i.x + vec4(0.0, i1.x, i2.x, 1.0));
  float n_ = 0.142857142857;
  vec3 ns = n_ * D.wyz - D.xzx;
  vec4 j = p - 49.0 * floor(p * ns.z * ns.z);
  vec4 x_ = floor(j * ns.z);
  vec4 y_ = floor(j - 7.0 * x_);
  vec4 x = x_ * ns.x + ns.yyyy;
  vec4 y = y_ * ns.x + ns.yyyy;
  vec4 h = 1.0 - abs(x) - abs(y);
  vec4 b0 = vec4(x.xy, y.xy);
  vec4 b1 = vec4(x.zw, y.zw);
  vec4 s0 = floor(b0) * 2.0 + 1.0;
  vec4 s1 = floor(b1) * 2.0 + 1.0;
  vec4 sh = -step(h, vec4(0.0));
  vec4 a0 = b0.xzyw + s0.xzyw * sh.xxyy;
  vec4 a1 = b1.xzyw + s1.xzyw * sh.zzww;
  vec3 p0 = vec3(a0.xy, h.x);
  vec3 p1 = vec3(a0.zw, h.y);
  vec3 p2 = vec3(a1.xy, h.z);
  vec3 p3 = vec3(a1.zw, h.w);
  vec4 norm = taylorInvSqrt(vec4(dot(p0, p0), dot(p1, p1), dot(p2, p2), dot(p3, p3)));
  p0 *= norm.x; p1 *= norm.y; p2 *= norm.z; p3 *= norm.w;
  vec4 m = max(0.6 - vec4(dot(x0, x0), dot(x1, x1), dot(x2, x2), dot(x3, x3)), 0.0);
  m = m * m;
  return 42.0 * dot(m * m, vec4(dot(p0, x0), dot(p1, x1), dot(p2, x2), dot(p3, x3)));
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

float cbrtSafe(float x) {
  return x > 1e-7 ? pow(x, 1.0 / 3.0) : 0.0;
}

vec3 srgbToLinear(vec3 c) {
  c = clamp(c, 0.0, 1.0);
  return mix(c / 12.92, pow(max(c + 0.055, 0.0) / 1.055, vec3(2.4)), step(0.04045, c));
}

vec3 linearToSrgb(vec3 c) {
  c = clamp(c, 0.0, 1.0);
  return mix(c * 12.92, 1.055 * pow(max(c, vec3(0.0031308)), vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

vec3 linearToOklab(vec3 c) {
  float l = cbrtSafe(0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b);
  float m = cbrtSafe(0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b);
  float s = cbrtSafe(0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b);
  return vec3(
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s
  );
}

vec3 oklabToLinear(vec3 c) {
  float l = c.x + 0.3963377774 * c.y + 0.2158037573 * c.z;
  float m = c.x - 0.1055613458 * c.y - 0.0638541728 * c.z;
  float s = c.x - 0.0894841775 * c.y - 1.2914855480 * c.z;
  l = l * l * l; m = m * m * m; s = s * s * s;
  return vec3(
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
  );
}

vec3 mixLch(vec3 a, vec3 b, float t) {
  float ca = length(a.yz);
  float cb = length(b.yz);
  float ha = ca > 1e-4 ? atan(a.z, a.y) : 0.0;
  float hb = cb > 1e-4 ? atan(b.z, b.y) : 0.0;
  if (ca < 0.02) ha = hb;
  if (cb < 0.02) hb = ha;
  float dh = mod(hb - ha + 3.14159265, 6.2831853) - 3.14159265;
  float h = ha + dh * t;
  float c = mix(ca, cb, t);
  return vec3(mix(a.x, b.x, t), c * cos(h), c * sin(h));
}

vec3 halftoneRamp(float x, vec3 s0, vec3 s1, vec3 s2, vec3 s3) {
  float f = clamp(x, 0.0, 1.0) * 3.0;
  if (f < 1.0) return mixLch(s0, s1, smoothstep(0.0, 1.0, f));
  if (f < 2.0) return mixLch(s1, s2, smoothstep(0.0, 1.0, f - 1.0));
  return mixLch(s2, s3, smoothstep(0.0, 1.0, f - 2.0));
}

float fbm3(vec3 p) {
  return snoise(p) * 0.78 + snoise(p * 2.03 + 7.1) * 0.22;
}

float halftoneTone(vec2 q, float t, float energy) {
  float frequency = 1.42;
  float flow = 1.2;
  float bandWidth = 1.02 + energy * 0.14;
  float contrast = 1.1;
  vec2 p = q * 0.55 * frequency;
  vec2 w = vec2(fbm3(vec3(p * 0.5, t)), fbm3(vec3(p * 0.5 + 5.2, t)));
  float n = fbm3(vec3(p + w * flow * 0.7, t * 0.7));
  float bw = 0.2 * bandWidth;
  float v = exp(-n * n / (bw * bw));
  v *= 0.65 + 0.35 * (snoise(vec3(p * 0.35, t * 0.5)) * 0.5 + 0.5);
  return clamp((v - 0.5) * contrast + 0.5, 0.0, 1.0);
}

float liquidTrail(vec2 p, float R, float t) {
  vec2 q = p / (R * 2.4);
  p += vec2(snoise(vec3(q, t)), snoise(vec3(q + 7.3, t))) * R * 0.5;
  float best = 0.0;
  for (int i = 0; i < 15; i++) {
    if (i + 1 >= uTrailCount) break;
    vec3 a = uTrailPts[i];
    vec3 b = uTrailPts[i + 1];
    vec2 ab = b.xy - a.xy;
    float h = clamp(dot(p - a.xy, ab) / max(dot(ab, ab), 1e-4), 0.0, 1.0);
    float s = mix(a.z, b.z, h);
    float r = R * mix(1.2, 0.8, s);
    float d = length(p - a.xy - ab * h);
    best = max(best, exp(-d * d / (r * r)) * s);
  }
  if (uTrailCount == 1) {
    float d = length(p - uTrailPts[0].xy);
    best = exp(-d * d / (R * R)) * uTrailPts[0].z;
  }
  return mix(best, smoothstep(0.05, 0.8, best), 0.7);
}

// 5: Lit halftone dot screen (adapted from uibrik Halftone)
vec3 sceneHalftone(vec2 frag, float t, float energy) {
  float ht = (t + 12.0) * 0.095;
  float cs = 7.0 * max(uDpr, 0.5);
  vec2 g = frag / cs;
  vec2 cell = floor(g) + 0.5;
  vec2 center = cell * cs;

  float tone = halftoneTone(center / iResolution.y, ht, energy);
  if (uPointerAmt > 0.001) {
    float R = 70.0 * max(uDpr, 0.5);
    vec2 dp = (center - uPointer) / R;
    float rest = exp(-dot(dp, dp));
    float trail = uPointerAmt * max(liquidTrail(center, R, (t + 12.0) * 0.5), rest * 0.6);
    tone = clamp(tone + trail * 0.85, 0.0, 1.0);
  }

  float dotSize = 0.95;
  float minDot = 0.30;
  float r = 0.5 * cs * dotSize * mix(minDot, 1.0, sqrt(tone));
  float soft = 0.6;
  float d = length((g - cell) * cs);
  float m = 1.0 - smoothstep(r - soft, r + soft, d);

  vec3 htBg = srgbToLinear(uBase * 0.40);
  vec3 s0 = linearToOklab(srgbToLinear(mix(uBase * 1.4, uColor1, 0.24)));
  vec3 s1 = linearToOklab(srgbToLinear(uColor1));
  vec3 s2 = linearToOklab(srgbToLinear(mix(uColor2, vec3(0.88, 0.90, 0.93), 0.42)));
  vec3 s3 = linearToOklab(srgbToLinear(mix(uColor3, vec3(0.96, 0.93, 0.87), 0.48)));

  vec3 dotColor = oklabToLinear(halftoneRamp(tone, s0, s1, s2, s3));
  float glow = 0.5 + energy * 0.22;
  vec3 base = htBg + dotColor * glow * 0.12 * tone * tone;
  return linearToSrgb(mix(base, dotColor, m));
}

vec3 evalScene(int mode, vec2 uv, vec2 frag, float t, float energy) {
  if (mode == 0) return sceneSilk(uv, t, energy);
  if (mode == 1) return sceneAurora(uv, t, energy);
  if (mode == 2) return sceneScanner(uv, t, energy);
  if (mode == 3) return scenePrism(uv, t, energy);
  if (mode == 4) return sceneBeams(uv, t, energy);
  return sceneHalftone(frag, t, energy);
}

void main() {
  vec2 uv = (gl_FragCoord.xy * 2.0 - iResolution.xy) / iResolution.y;
  float t = iTime;

  vec3 col;
  float htWeight = 0.0;
  if (uMix >= 0.999 || uModeA == uModeB) {
    col = evalScene(uModeB, uv, gl_FragCoord.xy, t, uEnergy);
    htWeight = uModeB == 5 ? 1.0 : 0.0;
  } else {
    float k = uMix * uMix * (3.0 - 2.0 * uMix);
    col = mix(
      evalScene(uModeA, uv, gl_FragCoord.xy, t, uEnergy),
      evalScene(uModeB, uv, gl_FragCoord.xy, t, uEnergy),
      k
    );
    htWeight = mix(uModeA == 5 ? 1.0 : 0.0, uModeB == 5 ? 1.0 : 0.0, k);
  }

  // Soft legibility shaping: keep top controls and bottom content grounded while center breathes.
  float vigAmt = mix(0.38, 0.28, htWeight);
  float vig = 1.0 - vigAmt * smoothstep(0.55, 1.55, length(uv * vec2(1.05, 0.78)));
  float topShade = mix(0.76, 1.0, smoothstep(1.02, 0.35, uv.y));
  float botShade = mix(0.66, 1.0, smoothstep(-1.05, -0.25, uv.y));
  col *= vig * topShade * botShade;

  // Fine film grain to avoid banding on dark tones; gentler on crisp halftone dots.
  float g = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233)) + iTime * 1.7) * 43758.5453);
  col += (g - 0.5) * mix(0.032, 0.007, htWeight);

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
let uDpr: WebGLUniformLocation | null = null
let uPointer: WebGLUniformLocation | null = null
let uPointerAmt: WebGLUniformLocation | null = null
let uTrailPts: WebGLUniformLocation | null = null
let uTrailCount: WebGLUniformLocation | null = null
let uModeA: WebGLUniformLocation | null = null
let uModeB: WebGLUniformLocation | null = null
let uMix: WebGLUniformLocation | null = null
let uBase: WebGLUniformLocation | null = null
let uColor1: WebGLUniformLocation | null = null
let uColor2: WebGLUniformLocation | null = null
let uColor3: WebGLUniformLocation | null = null

let time = 1.4
let energy = 0
let dprScale = 1
let modeA = SCENE_ID[props.scene]
let modeB = SCENE_ID[props.scene]
let mixT = 1

const TRAIL_MAX = 16
const TRAIL_LIFE = 0.9
const FOLLOW = 0.55
const pointer = { x: 0, y: 0, tx: 0, ty: 0, amt: 0, inside: false, seen: false }
const trail: { x: number; y: number; t: number }[] = []
const trailData = new Float32Array(TRAIL_MAX * 3)

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
  const sat = Math.min(82, Math.max(s, 18) * 1.12)
  return {
    base: hslToRgb(h, Math.min(sat * 0.42, 32), 7.5),
    c1: hslToRgb(h, sat, 44),
    c2: hslToRgb(h + shift, Math.min(86, sat * 0.94), 55),
    c3: hslToRgb(h + shift * 0.5, Math.min(sat, 58), 87),
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
  trailData.fill(0)
  const nowSec = performance.now() / 1000
  for (let i = 0; i < trail.length; i++) {
    const p = trail[i]
    const a = Math.min(1, Math.max(0, (nowSec - p.t) / TRAIL_LIFE))
    trailData[i * 3] = p.x
    trailData[i * 3 + 1] = p.y
    trailData[i * 3 + 2] = 1 - a * a * (3 - 2 * a)
  }
  gl.uniform1f(uTime, time)
  gl.uniform1f(uEnergy, energy)
  gl.uniform1f(uDpr, dprScale)
  gl.uniform2f(uPointer, pointer.x, pointer.y)
  gl.uniform1f(uPointerAmt, pointer.amt)
  gl.uniform3fv(uTrailPts, trailData)
  gl.uniform1i(uTrailCount, trail.length)
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
  dprScale = w / Math.max(1, canvas.clientWidth)
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w
    canvas.height = h
    trail.length = 0
    pointer.seen = false
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

  if (!props.paused && !reducedMotion && (modeA === 5 || modeB === 5)) {
    const ease = (from: number, to: number, rate: number, snap: number) => {
      const v = from + (to - from) * (1 - Math.exp(-rate * dt))
      return Math.abs(to - v) < snap ? to : v
    }
    const amtTo = pointer.inside ? 1 : 0
    pointer.amt = ease(pointer.amt, amtTo, 3.5, 0.002)
    const followRate = 28 * Math.pow(1 / 9, FOLLOW)
    pointer.x = ease(pointer.x, pointer.tx, followRate, 0.25)
    pointer.y = ease(pointer.y, pointer.ty, followRate, 0.25)
    const nowSec = performance.now() / 1000
    while (trail.length > 0 && nowSec - trail[trail.length - 1].t > TRAIL_LIFE) trail.pop()
    if (pointer.amt > 0) {
      const head = trail[0]
      if (!head || Math.hypot(head.x - pointer.x, head.y - pointer.y) > 6 * dprScale) {
        trail.unshift({ x: pointer.x, y: pointer.y, t: nowSec })
        if (trail.length > TRAIL_MAX) trail.pop()
      }
    }
  } else {
    pointer.amt = 0
    trail.length = 0
  }

  const colorAlpha = 1 - Math.exp(-dt * 5.2)
  lerpRgb(curPal.base, targetPal.base, colorAlpha)
  lerpRgb(curPal.c1, targetPal.c1, colorAlpha)
  lerpRgb(curPal.c2, targetPal.c2, colorAlpha)
  lerpRgb(curPal.c3, targetPal.c3, colorAlpha)

  draw()
}, FRAME_MS)

function onPointerMove(e: PointerEvent) {
  const canvas = canvasEl.value
  if (!canvas || reducedMotion) return
  const rect = canvas.getBoundingClientRect()
  if (rect.width <= 0 || rect.height <= 0) return
  const inside =
    e.clientX >= rect.left &&
    e.clientX <= rect.right &&
    e.clientY >= rect.top &&
    e.clientY <= rect.bottom
  pointer.inside = inside
  if (!inside) return
  pointer.tx = ((e.clientX - rect.left) / rect.width) * canvas.width
  pointer.ty = ((rect.bottom - e.clientY) / rect.height) * canvas.height
  if (!pointer.seen || pointer.amt === 0) {
    pointer.x = pointer.tx
    pointer.y = pointer.ty
    pointer.seen = true
  }
}

function onPointerLeave() {
  pointer.inside = false
}

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
  uDpr = gl.getUniformLocation(prog, 'uDpr')
  uPointer = gl.getUniformLocation(prog, 'uPointer')
  uPointerAmt = gl.getUniformLocation(prog, 'uPointerAmt')
  uTrailPts = gl.getUniformLocation(prog, 'uTrailPts')
  uTrailCount = gl.getUniformLocation(prog, 'uTrailCount')
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
  window.addEventListener('pointermove', onPointerMove, { passive: true })
  document.documentElement.addEventListener('pointerleave', onPointerLeave)
  window.addEventListener('blur', onPointerLeave)
  start()
})

onUnmounted(() => {
  stop()
  resizeObserver?.disconnect()
  intersectionObserver?.disconnect()
  document.removeEventListener('visibilitychange', onVisibility)
  window.removeEventListener('pointermove', onPointerMove)
  document.documentElement.removeEventListener('pointerleave', onPointerLeave)
  window.removeEventListener('blur', onPointerLeave)
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
  background: hsl(var(--rc-h, 216), min(var(--rc-s, 45%), 28%), 8%);
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
    radial-gradient(120% 65% at 25% 15%, hsla(var(--rc-h, 216), var(--rc-s, 45%), 52%, 0.42), transparent 64%),
    radial-gradient(100% 60% at 85% 80%, hsla(calc(var(--rc-h, 216) - 14), var(--rc-s, 45%), 46%, 0.34), transparent 68%),
    hsl(var(--rc-h, 216), min(var(--rc-s, 45%), 28%), 7%);
}
</style>
