import { ref, watch, type Ref } from "vue";

/** "r, g, b" per cover src; null when the image could not be read. */
const cache = new Map<string, string | null>();
const pending = new Map<string, Promise<string | null>>();

const SAMPLE = 24;

function rgbToHsl(r: number, g: number, b: number): [number, number, number] {
    r /= 255; g /= 255; b /= 255;
    const max = Math.max(r, g, b), min = Math.min(r, g, b);
    const l = (max + min) / 2;
    if (max === min) return [0, 0, l];
    const d = max - min;
    const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    let h: number;
    if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    return [h / 6, s, l];
}

function hslToRgb(h: number, s: number, l: number): [number, number, number] {
    if (s === 0) return [l * 255, l * 255, l * 255];
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
    const p = 2 * l - q;
    const hue = (t: number) => {
        if (t < 0) t += 1;
        if (t > 1) t -= 1;
        if (t < 1 / 6) return p + (q - p) * 6 * t;
        if (t < 1 / 2) return q;
        if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
        return p;
    };
    return [hue(h + 1 / 3) * 255, hue(h) * 255, hue(h - 1 / 3) * 255];
}

/**
 * Average of the cover weighted toward saturated mid-tone pixels, then pulled to a fixed
 * lightness and capped saturation so every header tint reads the same on the dark background.
 */
function extract(img: HTMLImageElement): string | null {
    const canvas = document.createElement("canvas");
    canvas.width = SAMPLE;
    canvas.height = SAMPLE;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return null;
    ctx.drawImage(img, 0, 0, SAMPLE, SAMPLE);
    let data: Uint8ClampedArray;
    try {
        data = ctx.getImageData(0, 0, SAMPLE, SAMPLE).data;
    } catch {
        return null;
    }
    let r = 0, g = 0, b = 0, w = 0;
    for (let i = 0; i < data.length; i += 4) {
        const [, s, l] = rgbToHsl(data[i], data[i + 1], data[i + 2]);
        const mid = 1 - Math.abs(l - 0.5) * 2;
        const weight = 0.04 + s * s * mid;
        r += data[i] * weight;
        g += data[i + 1] * weight;
        b += data[i + 2] * weight;
        w += weight;
    }
    if (w === 0) return null;
    const [h, s] = rgbToHsl(r / w, g / w, b / w);
    const [R, G, B] = hslToRgb(h, Math.min(s, 0.55), 0.52);
    return `${Math.round(R)}, ${Math.round(G)}, ${Math.round(B)}`;
}

export function coverTint(src: string): Promise<string | null> {
    if (cache.has(src)) return Promise.resolve(cache.get(src) ?? null);
    const inflight = pending.get(src);
    if (inflight) return inflight;
    const p = new Promise<string | null>((resolve) => {
        const img = new Image();
        img.crossOrigin = "anonymous";
        img.decoding = "async";
        img.onload = () => resolve(extract(img));
        img.onerror = () => resolve(null);
        img.src = src;
    }).then((tint) => {
        cache.set(src, tint);
        pending.delete(src);
        return tint;
    });
    pending.set(src, p);
    return p;
}

/** Reactive "r, g, b" tint of a cover; keeps the previous value while the next one loads. */
export function useCoverTint(src: () => string | null | undefined): Ref<string | null> {
    const tint = ref<string | null>(null);
    watch(src, (value) => {
        if (!value) { tint.value = null; return; }
        const known = cache.get(value);
        if (known !== undefined) { tint.value = known; return; }
        void coverTint(value).then((t) => {
            if (src() === value) tint.value = t;
        });
    }, { immediate: true });
    return tint;
}
