import { watchEffect } from "vue";
import { useSettings } from "./useSettings";
import { usePlayer } from "./usePlayer";

// ──────────────────────────────────────────────────────────────────────────────
// Динамическое темирование: из акцентного цвета текущего трека (или статической
// настройки) выставляет CSS-переменные --accent-* на :root, из которых строятся
// все цвета компонентов. Также задаёт цвет системных панелей (Android/mobile).
// Вынесено из App.vue без изменения поведения; вызывается один раз в корне.
// ──────────────────────────────────────────────────────────────────────────────

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

export function useAccentTheme() {
    const { settings } = useSettings();
    const { currentAccentColor } = usePlayer();

    watchEffect(() => {
        const hex = settings.value.dynamic_accent_color
            ? (currentAccentColor.value ?? "#1a1a1a")
            : settings.value.static_accent_color;
        const { h, s, l } = hexToHSL(hex);
        const chromeHex = hslToHex(h, Math.min(s, 50), 8);
        const root = document.documentElement.style;
        root.setProperty("--accent-h", `${h}`);
        root.setProperty("--accent-s", `${s}%`);
        root.setProperty("--accent-l", `${l}%`);
        root.setProperty("--accent-color", h === 0 && s === 0 ? "#353535" : `hsl(${h}, ${s}%, ${l}%)`);
        root.setProperty("--mobile-chrome-color", chromeHex);
        window.LFMusicAndroid?.updateSystemBars?.(chromeHex);
    });
}
