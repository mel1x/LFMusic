import { watchEffect } from "vue";
import { useSettings } from "./useSettings";
import { usePlayer } from "./usePlayer";

// Dynamic theming: derives --accent-* CSS vars on :root from the current track's
// accent color (or the static setting); also sets the Android/mobile system bar color.

function hexToHSL(hex: string) {
    const r = parseInt(hex.slice(1, 3), 16) / 255;
    const g = parseInt(hex.slice(3, 5), 16) / 255;
    const b = parseInt(hex.slice(5, 7), 16) / 255;

    const max = Math.max(r, g, b), min = Math.min(r, g, b);
    const l = (max + min) / 2;
    let h = 0, s = 0;

    if (max !== min) {
        const d = max - min;
        s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
        if (max === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6;
        else if (max === g) h = ((b - r) / d + 2) / 6;
        else h = ((r - g) / d + 4) / 6;
    }

    return { h: Math.round(h * 360), s: Math.round(s * 100), l: Math.round(l * 100) };
}

function hexToRgb(hex: string) {
    return {
        r: parseInt(hex.slice(1, 3), 16),
        g: parseInt(hex.slice(3, 5), 16),
        b: parseInt(hex.slice(5, 7), 16),
    };
}

function rgbToHsl(r: number, g: number, b: number) {
    r /= 255;
    g /= 255;
    b /= 255;
    const max = Math.max(r, g, b), min = Math.min(r, g, b);
    const l = (max + min) / 2;
    let h = 0, s = 0;
    if (max !== min) {
        const d = max - min;
        s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
        if (max === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6;
        else if (max === g) h = ((b - r) / d + 2) / 6;
        else h = ((r - g) / d + 4) / 6;
    }
    return { h: Math.round(h * 360), s: Math.round(s * 100), l: Math.round(l * 100) };
}

function hslToHex(h: number, s: number, l: number) {
    s /= 100;
    l /= 100;

    const c = (1 - Math.abs(2 * l - 1)) * s;
    const x = c * (1 - Math.abs((h / 60) % 2 - 1));
    const m = l - c / 2;
    let r = 0;
    let g = 0;
    let b = 0;

    if (h < 60) [r, g, b] = [c, x, 0];
    else if (h < 120) [r, g, b] = [x, c, 0];
    else if (h < 180) [r, g, b] = [0, c, x];
    else if (h < 240) [r, g, b] = [0, x, c];
    else if (h < 300) [r, g, b] = [x, 0, c];
    else [r, g, b] = [c, 0, x];

    const toHex = (value: number) => Math.round((value + m) * 255).toString(16).padStart(2, "0");
    return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

type Rgb = { r: number; g: number; b: number };

const ACCENT_TRANSITION_MS = 350;
// Player bar leads the color change with its own shorter timing.
const PLAYER_TRANSITION_MS = 150;

const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
const easeInOut = (t: number) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2);

export function useAccentTheme() {
    const { settings } = useSettings();
    const { currentAccentColor } = usePlayer();

    // Animate in RGB space via requestAnimationFrame rather than lerping HSL directly:
    // interpolating hue (--accent-h) linearly arcs through the color wheel, so
    // blue->beige would pass through saturated green/purple. RGB lerp avoids that.
    // Each frame the intermediate RGB is converted back to HSL for --accent-*.
    const root = document.documentElement.style;

    // Displayed colors: base (-> --accent-h/s/l), accent (-> --accent-color),
    // and player (-> --player-accent-h/s/l, same target but faster).
    let curBase: Rgb | null = null;
    let curAccent: Rgb | null = null;
    let curPlayer: Rgb | null = null;
    let rafId: number | null = null;

    const applyBase = (rgb: Rgb) => {
        const { h, s, l } = rgbToHsl(rgb.r, rgb.g, rgb.b);
        root.setProperty("--accent-h", `${h}`);
        root.setProperty("--accent-s", `${s}%`);
        root.setProperty("--accent-l", `${l}%`);
        const chromeHex = hslToHex(h, Math.min(s, 50), 8);
        root.setProperty("--mobile-chrome-color", chromeHex);
        window.LFMusicAndroid?.updateSystemBars?.(chromeHex);
    };

    const applyPlayer = (rgb: Rgb) => {
        const { h, s, l } = rgbToHsl(rgb.r, rgb.g, rgb.b);
        root.setProperty("--player-accent-h", `${h}`);
        root.setProperty("--player-accent-s", `${s}%`);
        root.setProperty("--player-accent-l", `${l}%`);
    };

    const applyAccent = (rgb: Rgb) => {
        const toHex = (v: number) => Math.round(v).toString(16).padStart(2, "0");
        root.setProperty("--accent-color", `#${toHex(rgb.r)}${toHex(rgb.g)}${toHex(rgb.b)}`);
    };

    const animateTo = (targetBase: Rgb, targetAccent: Rgb) => {
        if (rafId !== null) cancelAnimationFrame(rafId);

        // First run: apply instantly instead of animating from black.
        if (!curBase || !curAccent || !curPlayer) {
            curBase = targetBase;
            curAccent = targetAccent;
            curPlayer = targetBase;
            applyBase(targetBase);
            applyAccent(targetAccent);
            applyPlayer(targetBase);
            return;
        }

        const fromBase = curBase;
        const fromAccent = curAccent;
        const fromPlayer = curPlayer;
        const start = performance.now();

        const mix = (from: Rgb, to: Rgb, e: number): Rgb => ({
            r: lerp(from.r, to.r, e),
            g: lerp(from.g, to.g, e),
            b: lerp(from.b, to.b, e),
        });

        const step = (now: number) => {
            const elapsed = now - start;
            // Two timelines sharing a start time; player finishes first (shorter).
            const eBase = easeInOut(Math.min(elapsed / ACCENT_TRANSITION_MS, 1));
            const ePlayer = easeInOut(Math.min(elapsed / PLAYER_TRANSITION_MS, 1));

            curBase = mix(fromBase, targetBase, eBase);
            curAccent = mix(fromAccent, targetAccent, eBase);
            curPlayer = mix(fromPlayer, targetBase, ePlayer);
            applyBase(curBase);
            applyAccent(curAccent);
            applyPlayer(curPlayer);

            if (elapsed < ACCENT_TRANSITION_MS) {
                rafId = requestAnimationFrame(step);
            } else {
                rafId = null;
            }
        };
        rafId = requestAnimationFrame(step);
    };

    watchEffect(() => {
        const hex = settings.value.dynamic_accent_color
            ? (currentAccentColor.value ?? "#1a1a1a")
            : settings.value.static_accent_color;
        const { h, s } = hexToHSL(hex);
        // Near-black gray fallback: lighten to #353535 so it doesn't match the base color.
        const accentHex = h === 0 && s === 0 ? "#353535" : hex;
        animateTo(hexToRgb(hex), hexToRgb(accentHex));
    });
}
