<script setup lang="ts">
import { onMounted, onUnmounted, computed, ref, watch } from 'vue';
import { useSettings, type SettingKey, type PlayerExtraBlockMode } from '../../composables/useSettings';
import { useI18n } from '../../composables/useI18n';
import SettingsShell from '../../components/SettingsShell.vue';
import SettingToggle from '../../components/SettingToggle.vue';

const { settings, settingItems, load, save } = useSettings();
const { t } = useI18n();

const dynamicItem = computed(() => settingItems.find(i => i.key === 'dynamic_accent_color')!);
const extraBlockItem = computed(() => settingItems.find(i => i.key === 'show_player_extra_block')!);
const githubItem = computed(() => settingItems.find(i => i.key === 'show_github_badge')!);

async function toggle(key: SettingKey) {
    settings.value[key] = !settings.value[key];
    await save();
}

onMounted(load);

const extraBlockModeOptions: { value: PlayerExtraBlockMode; labelKey: string; tooltipKey: string }[] = [
    { value: 'visualizer', labelKey: 'settings.extraBlockVisualizer', tooltipKey: 'settings.extraBlockVisualizerTooltip' },
    { value: 'next_track', labelKey: 'settings.extraBlockNextTrack', tooltipKey: 'settings.extraBlockNextTrack' },
];

async function chooseExtraBlockMode(mode: PlayerExtraBlockMode) {
    settings.value.player_extra_block_mode = mode;
    await save();
}

const ACCENT_PRESETS = [
    '#101010', '#3D3D3D', '#1A1A2E',
    '#2D1B3D', '#1B3D2D', '#3D2D1B',
    '#3D1B2D', '#1B2D3D', '#2D3D1B',
] as const;

const hexDisplay = computed(() =>
    settings.value.static_accent_color.replace('#', '').toUpperCase()
);

function isPresetActive(color: string): boolean {
    return settings.value.static_accent_color.toLowerCase() === color.toLowerCase();
}

function hexToRgb(hex: string): { r: number; g: number; b: number } {
    const n = parseInt(hex.replace('#', ''), 16);
    return { r: (n >> 16) & 0xff, g: (n >> 8) & 0xff, b: n & 0xff };
}
function rgbToHex(r: number, g: number, b: number): string {
    const to2 = (v: number) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, '0');
    return '#' + to2(r) + to2(g) + to2(b);
}
function rgbToHsv(r: number, g: number, b: number) {
    r /= 255; g /= 255; b /= 255;
    const max = Math.max(r, g, b), min = Math.min(r, g, b);
    const d = max - min;
    const v = max;
    const s = max === 0 ? 0 : d / max;
    let h = 0;
    if (d !== 0) {
        if (max === r) h = ((g - b) / d + (g < b ? 6 : 0));
        else if (max === g) h = (b - r) / d + 2;
        else h = (r - g) / d + 4;
        h *= 60;
    }
    return { h, s, v };
}
function hsvToRgb(h: number, s: number, v: number) {
    const c = v * s;
    const hp = (h % 360) / 60;
    const x = c * (1 - Math.abs(hp % 2 - 1));
    let r1 = 0, g1 = 0, b1 = 0;
    if (hp < 1)      { r1 = c; g1 = x; b1 = 0; }
    else if (hp < 2) { r1 = x; g1 = c; b1 = 0; }
    else if (hp < 3) { r1 = 0; g1 = c; b1 = x; }
    else if (hp < 4) { r1 = 0; g1 = x; b1 = c; }
    else if (hp < 5) { r1 = x; g1 = 0; b1 = c; }
    else             { r1 = c; g1 = 0; b1 = x; }
    const m = v - c;
    return { r: (r1 + m) * 255, g: (g1 + m) * 255, b: (b1 + m) * 255 };
}
function hsvToHex(h: number, s: number, v: number): string {
    const { r, g, b } = hsvToRgb(h, s, v);
    return rgbToHex(r, g, b);
}

// Mirrors the theme formulas in App.vue (--light-bg / --dark-bg / card surface).
function hexToHsl(hex: string): { h: number; s: number; l: number } {
    const { r, g, b } = hexToRgb(hex);
    const rn = r / 255, gn = g / 255, bn = b / 255;
    const max = Math.max(rn, gn, bn), min = Math.min(rn, gn, bn);
    const l = (max + min) / 2;
    let h = 0, s = 0;
    if (max !== min) {
        const d = max - min;
        s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
        if (max === rn) h = ((gn - bn) / d + (gn < bn ? 6 : 0)) / 6;
        else if (max === gn) h = ((bn - rn) / d + 2) / 6;
        else h = ((rn - gn) / d + 4) / 6;
    }
    return { h: Math.round(h * 360), s: Math.round(s * 100), l: Math.round(l * 100) };
}
interface AccentPalette { accent: string; light: string; surface: string; dark: string }
function paletteFromHex(hex: string): AccentPalette {
    const { h, s } = hexToHsl(hex);
    return {
        accent: hex,
        light: `hsl(${h}, ${Math.min(s, 45)}%, 15%)`,
        surface: `hsl(${h}, ${Math.min(Math.round(s * 1.25), 50)}%, 16%)`,
        dark: `hsl(${h}, ${Math.min(s, 50)}%, 8%)`,
    };
}
const currentPalette = computed(() => paletteFromHex(settings.value.static_accent_color));
const presetSwatches = ACCENT_PRESETS.map(color => ({ color, palette: paletteFromHex(color) }));

const pickerOpen = ref(false);
const hue = ref(0);
const sat = ref(0);
const val = ref(0);
const pickerRoot = ref<HTMLElement | null>(null);
const svArea = ref<HTMLElement | null>(null);
const hueArea = ref<HTMLElement | null>(null);

function syncHSVFromHex(hex: string) {
    const { r, g, b } = hexToRgb(hex);
    const hsv = rgbToHsv(r, g, b);
    // Keep the previous hue for achromatic colors so the marker doesn't jump to 0
    if (hsv.s > 0) hue.value = hsv.h;
    sat.value = hsv.s;
    val.value = hsv.v;
}

function togglePicker() {
    if (pickerOpen.value) { pickerOpen.value = false; return; }
    syncHSVFromHex(settings.value.static_accent_color);
    pickerOpen.value = true;
}

async function applyHSV(persist: boolean) {
    const hex = hsvToHex(hue.value, sat.value, val.value);
    settings.value.static_accent_color = hex;
    if (persist) await save();
}

function clientToRect(e: PointerEvent, el: HTMLElement) {
    const rect = el.getBoundingClientRect();
    const x = Math.max(0, Math.min(rect.width, e.clientX - rect.left));
    const y = Math.max(0, Math.min(rect.height, e.clientY - rect.top));
    return { x, y, w: rect.width, h: rect.height };
}

function startSVDrag(e: PointerEvent) {
    if (!svArea.value) return;
    const el = svArea.value;
    const update = (ev: PointerEvent) => {
        const { x, y, w, h } = clientToRect(ev, el);
        sat.value = w > 0 ? x / w : 0;
        val.value = h > 0 ? 1 - y / h : 0;
        applyHSV(false);
    };
    update(e);
    const move = (ev: PointerEvent) => update(ev);
    const up = () => {
        window.removeEventListener('pointermove', move);
        window.removeEventListener('pointerup', up);
        applyHSV(true);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
}

function startHueDrag(e: PointerEvent) {
    if (!hueArea.value) return;
    const el = hueArea.value;
    const update = (ev: PointerEvent) => {
        const { x, w } = clientToRect(ev, el);
        hue.value = w > 0 ? (x / w) * 360 : 0;
        applyHSV(false);
    };
    update(e);
    const move = (ev: PointerEvent) => update(ev);
    const up = () => {
        window.removeEventListener('pointermove', move);
        window.removeEventListener('pointerup', up);
        applyHSV(true);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
}

function onDocMouseDown(e: MouseEvent) {
    const target = e.target as Node;
    const root = pickerRoot.value;
    if (pickerOpen.value && (!root || !root.contains(target))) {
        pickerOpen.value = false;
    }
}
function onDocKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && pickerOpen.value) pickerOpen.value = false;
}

onMounted(() => {
    document.addEventListener('mousedown', onDocMouseDown);
    document.addEventListener('keydown', onDocKey);
});
onUnmounted(() => {
    document.removeEventListener('mousedown', onDocMouseDown);
    document.removeEventListener('keydown', onDocKey);
});

// Sync HSV when the hex changes externally (hex input / presets)
watch(() => settings.value.static_accent_color, (newHex) => {
    if (!pickerOpen.value) return;
    const currentHex = hsvToHex(hue.value, sat.value, val.value).toLowerCase();
    if (newHex.toLowerCase() === currentHex) return;
    syncHSVFromHex(newHex);
});

// Close the popup if static mode gets turned off
watch(() => settings.value.dynamic_accent_color, (isDynamic) => {
    if (isDynamic) pickerOpen.value = false;
});

const hueBase = computed(() => `hsl(${hue.value}, 100%, 50%)`);
const svMarkerStyle = computed(() => ({
    left: (sat.value * 100) + '%',
    top: ((1 - val.value) * 100) + '%',
    backgroundColor: settings.value.static_accent_color,
}));
const hueMarkerStyle = computed(() => ({
    left: (hue.value / 360 * 100) + '%',
    backgroundColor: `hsl(${hue.value}, 100%, 50%)`,
}));

async function onHexTextInput(event: Event) {
    const input = event.target as HTMLInputElement;
    const raw = input.value.replace(/[^0-9a-fA-F]/g, '').slice(0, 6);
    input.value = raw.toUpperCase();
    if (/^[0-9a-fA-F]{6}$/.test(raw)) {
        settings.value.static_accent_color = '#' + raw.toLowerCase();
        await save();
    }
}

function onHexBlur(event: Event) {
    const input = event.target as HTMLInputElement;
    const raw = input.value.replace(/[^0-9a-fA-F]/g, '');
    if (!/^[0-9a-fA-F]{6}$/.test(raw)) {
        input.value = hexDisplay.value;
    }
}

async function selectPreset(color: string) {
    settings.value.static_accent_color = color.toLowerCase();
    await save();
}
</script>

<template>
    <SettingsShell title="Оформление" description="Внешний вид интерфейса и цветовая тема">
        <div class="settingBlock">
            <div class="settingTitle">Интерфейс</div>
            <div class="settingDesc">Дополнительные элементы интерфейса приложения</div>
            <div class="settingsBlock">
                <SettingToggle
                    :item="githubItem"
                    :checked="settings.show_github_badge"
                    phoneHidden
                    @toggle="toggle('show_github_badge')"
                />

                <SettingToggle
                    :item="extraBlockItem"
                    :checked="settings.show_player_extra_block"
                    phoneHidden
                    @toggle="toggle('show_player_extra_block')"
                />
                <transition name="extraModePanel">
                    <div v-if="settings.show_player_extra_block" class="extraModePanel">
                        <span class="extraModeTitle">{{ t('settings.extraBlockMode') }}</span>
                        <div class="extraModeOptions" role="radiogroup" :aria-label="t('settings.extraBlockModeAria')">
                            <button
                                v-for="option in extraBlockModeOptions"
                                :key="option.value"
                                type="button"
                                class="extraModeBtn"
                                :class="{ active: settings.player_extra_block_mode === option.value }"
                                :aria-pressed="settings.player_extra_block_mode === option.value"
                                v-tooltip="option.value === 'visualizer' ? t(option.tooltipKey) : undefined"
                                @click="chooseExtraBlockMode(option.value)"
                            >
                                <span>{{ t(option.labelKey) }}</span>
                            </button>
                        </div>
                    </div>
                </transition>
            </div>
        </div>

        <div class="settingBlock">
            <div class="settingTitle">Цветовая тема</div>
            <div class="settingDesc">Акцентные цвета и фон приложения</div>
            <div class="settingsBlock">
                <SettingToggle
                    :item="dynamicItem"
                    :checked="settings.dynamic_accent_color"
                    @toggle="toggle('dynamic_accent_color')"
                />

                <div v-if="!settings.dynamic_accent_color" class="accentPicker" ref="pickerRoot">
                    <div class="accentPickerHead">
                        <button type="button"
                            class="accentSwatch"
                            :class="{ active: pickerOpen }"
                            @click="togglePicker"
                            v-tooltip="'Открыть палитру'"
                            aria-label="Выбрать цвет">
                            <span class="swatchGrid">
                                <span class="swatchCell" :style="{ background: currentPalette.accent }"></span>
                                <span class="swatchCell" :style="{ background: currentPalette.light }"></span>
                                <span class="swatchCell" :style="{ background: currentPalette.surface }"></span>
                                <span class="swatchCell" :style="{ background: currentPalette.dark }"></span>
                            </span>
                        </button>
                        <div class="accentPickerInfo">
                            <span class="accentPickerLabel">Статичный акцентный цвет</span>
                            <div class="accentHexField">
                                <span class="accentHexHash">#</span>
                                <input type="text" class="accentHexInput"
                                    :value="hexDisplay"
                                    @input="onHexTextInput($event)"
                                    @blur="onHexBlur($event)"
                                    maxlength="6"
                                    spellcheck="false">
                            </div>
                        </div>
                    </div>

                    <transition name="colorPickerPop">
                        <div v-if="pickerOpen" class="colorPickerPop">
                            <div class="svArea"
                                ref="svArea"
                                :style="{ '--picker-hue': hueBase }"
                                @pointerdown="startSVDrag">
                                <div class="svMarker" :style="svMarkerStyle"></div>
                            </div>
                            <div class="hueArea"
                                ref="hueArea"
                                @pointerdown="startHueDrag">
                                <div class="hueMarker" :style="hueMarkerStyle"></div>
                            </div>
                        </div>
                    </transition>

                    <span class="accentSubLabel">Готовые палитры</span>
                    <div class="accentPresets">
                        <button v-for="preset in presetSwatches" :key="preset.color"
                            type="button"
                            class="accentPresetBtn"
                            :class="{ active: isPresetActive(preset.color) }"
                            v-tooltip="preset.color"
                            @click="selectPreset(preset.color)">
                            <span class="swatchGrid">
                                <span class="swatchCell" :style="{ background: preset.palette.accent }"></span>
                                <span class="swatchCell" :style="{ background: preset.palette.light }"></span>
                                <span class="swatchCell" :style="{ background: preset.palette.surface }"></span>
                                <span class="swatchCell" :style="{ background: preset.palette.dark }"></span>
                            </span>
                        </button>
                    </div>
                </div>
            </div>
        </div>
    </SettingsShell>
</template>

<style scoped>
.settingBlock {
    display: flex;
    flex-direction: column;
    margin-bottom: 36px;
}
.settingBlock:last-child {
    margin-bottom: 8px;
}
.settingTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 700;
    font-size: 18px;
    color: #ffffff;
}
.settingDesc {
    user-select: none;
    margin-top: 3px;
    font-weight: 600;
    font-size: 15px;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
}
.settingsBlock {
    margin-top: 16px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-width: 800px;
    width: 100%;
}

.extraModePanel {
    margin: -4px 0 8px 48px;
    display: inline-flex;
    align-items: center;
    gap: 12px;
    width: fit-content;
    max-width: calc(100% - 48px);
    overflow: hidden;
}
.extraModeTitle {
    user-select: none;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.48);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
}
.extraModeOptions {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 3px;
    border-radius: 16px;
    background-color: var(--dark-bg);
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.06);
}
.extraModeBtn {
    user-select: none;
    position: relative;
    min-width: 118px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 12px;
    border-radius: 13px;
    background-color: transparent;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.35), 84%, 0.58);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: background-color .18s ease, color .18s ease;
}
.extraModeBtn:hover {
    color: hsla(0, 0%, 100%, 0.82);
}
.extraModeBtn.active {
    background-color: hsla(0, 0%, 100%, 0.09);
    color: #ffffff;
}
.extraModePanel-enter-active,
.extraModePanel-leave-active {
    transition: opacity .18s ease, transform .18s ease, max-height .18s ease;
    max-height: 36px;
}
.extraModePanel-enter-from,
.extraModePanel-leave-to {
    opacity: 0;
    transform: translateY(-4px);
    max-height: 0;
    margin-top: -10px;
    margin-bottom: -4px;
}

.accentPicker {
    margin-top: 8px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    background-color: hsla(var(--accent-h), min(calc(var(--accent-s) * 1.25), 50%), 16%, 1);
    border-radius: 14px;
    transition: .24s ease;
}
.accentPickerHead {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 14px;
}
.accentSwatch {
    position: relative;
    width: 58px;
    height: 58px;
    border-radius: 14px;
    cursor: pointer;
    overflow: hidden;
    transition: transform 0.18s ease, box-shadow 0.24s ease;
    flex-shrink: 0;
    padding: 0;
    border: 0;
    appearance: none;
    background-color: transparent;

    &:hover {
        box-shadow: 0 0 0 1px hsla(0, 0%, 100%, 0.28);
    }
    &.active {
        box-shadow: 0 0 0 2px #ffffff, 0 4px 14px hsla(0, 0%, 0%, 0.4);
    }
}
.swatchGrid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
    width: 100%;
    height: 100%;
}
.swatchCell {
    width: 100%;
    height: 100%;
}
.colorPickerPop {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px;
    background-color: hsla(0, 0%, 0%, 0.28);
    border-radius: 12px;
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.06);
}
.svArea {
    position: relative;
    width: 100%;
    height: 140px;
    border-radius: 8px;
    cursor: crosshair;
    touch-action: none;
    user-select: none;
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.08);
    background:
        linear-gradient(to bottom, transparent 0%, #000 100%),
        linear-gradient(to right, #ffffff 0%, transparent 100%),
        var(--picker-hue, hsl(0, 100%, 50%));
}
.svMarker {
    position: absolute;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    transform: translate(-50%, -50%);
    border: 2px solid #ffffff;
    box-shadow: 0 0 0 1px hsla(0, 0%, 0%, 0.55), 0 2px 6px hsla(0, 0%, 0%, 0.45);
    pointer-events: none;
}
.hueArea {
    position: relative;
    width: 100%;
    height: 12px;
    border-radius: 999px;
    cursor: pointer;
    touch-action: none;
    user-select: none;
    background: linear-gradient(to right,
        hsl(0, 100%, 50%) 0%,
        hsl(60, 100%, 50%) 16.66%,
        hsl(120, 100%, 50%) 33.33%,
        hsl(180, 100%, 50%) 50%,
        hsl(240, 100%, 50%) 66.66%,
        hsl(300, 100%, 50%) 83.33%,
        hsl(360, 100%, 50%) 100%);
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.08);
}
.hueMarker {
    position: absolute;
    top: 50%;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    transform: translate(-50%, -50%);
    border: 2px solid #ffffff;
    box-shadow: 0 0 0 1px hsla(0, 0%, 0%, 0.55), 0 2px 6px hsla(0, 0%, 0%, 0.4);
    pointer-events: none;
}
.colorPickerPop-enter-active,
.colorPickerPop-leave-active {
    transition: opacity 0.18s ease, transform 0.18s ease;
    transform-origin: top left;
}
.colorPickerPop-enter-from,
.colorPickerPop-leave-to {
    opacity: 0;
    transform: scale(0.96) translateY(-4px);
}
.accentPickerInfo {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: 1;
    min-width: 0;
}
.accentPickerLabel {
    user-select: none;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.6);
    font-weight: 600;
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
}
.accentHexField {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 2px;
    padding: 6px 10px;
    background-color: hsla(0, 0%, 0%, 0.25);
    border-radius: 8px;
    width: fit-content;
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.04);
    transition: .2s ease;

    &:focus-within {
        box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.18);
        background-color: hsla(0, 0%, 0%, 0.35);
    }
}
.accentHexHash {
    user-select: none;
    font-family: "JetBrains Mono", "Consolas", monospace;
    font-size: 14px;
    font-weight: 600;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
}
.accentHexInput {
    font-family: "JetBrains Mono", "Consolas", monospace;
    font-size: 14px;
    font-weight: 700;
    color: #ffffff;
    background: transparent;
    border: 0;
    outline: 0;
    width: 72px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    caret-color: #ffffff;
}
.accentSubLabel {
    user-select: none;
    margin-top: 2px;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.6);
    font-weight: 600;
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
}
.accentPresets {
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
    gap: 10px;
}
.accentPresetBtn {
    width: 38px;
    height: 38px;
    border-radius: 10px;
    cursor: pointer;
    overflow: hidden;
    padding: 0;
    border: 0;
    appearance: none;
    background-color: transparent;
    transition: transform 0.18s ease, box-shadow 0.24s ease;

    &:hover {
        transform: translateY(-1px);
        box-shadow: 0 0 0 1px hsla(0, 0%, 100%, 0.28);
    }
    &.active {
        box-shadow: 0 0 0 2px #ffffff;
    }
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .settingBlock {
        margin-bottom: 30px;
    }
    .settingsBlock {
        max-width: none;
    }
    .extraModePanel {
        display: none;
    }
    .accentPickerHead {
        flex-wrap: wrap;
    }
    .accentPicker {
        padding: 13px;
        border-radius: 14px;
    }
    .accentPickerInfo {
        width: min-content;
    }
    .accentPresets {
        gap: 12px;
    }
    .accentPresetBtn {
        width: 44px;
        height: 44px;
    }
}
</style>
