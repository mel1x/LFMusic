<script setup lang="ts">
import { onMounted, onUnmounted, computed, ref, watch } from 'vue';
import DeleteIcon from '../components/Icons/DeleteIcon.vue';
import FolderIcon from '../components/Icons/FolderIcon.vue';
import { useSettings, ARTIST_SEPARATORS_DEFAULT } from '../composables/useSettings';
import { usePlayer } from '../composables/usePlayer';
import { usePlaylists } from '../composables/usePlaylists';
import { useEqualizer, EQ_PRESETS, EQ_BAND_LABELS } from '../composables/useEqualizer';
import RefreshIcon from '../components/Icons/RefreshIcon.vue';
import PlusIcon from '../components/Icons/PlusIcon.vue';

const { settings, settingItems, load, save } = useSettings();
const { rebuildArtists } = usePlaylists();
const {
    folders, isScanning, removingFolder, scanProgress,
    addFolder, removeFolder, rescanLibrary, folderDisplayName,
} = usePlayer();
const {
    eqEnabled, eqBands, eqGain, activePreset,
    load: loadEq, toggleEnabled: toggleEq,
    setBand, setGain, applyPreset, resetBands,
} = useEqualizer();

const sortedFolders = computed(() =>
    [...folders.value].sort((a, b) => b.track_count - a.track_count)
);

onMounted(async () => {
    await load();
    await loadEq();
});

async function toggle(key: keyof typeof settings.value) {
    if (typeof settings.value[key] !== 'boolean') return;
    (settings.value[key] as boolean) = !(settings.value[key] as boolean);
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

// ── HSV picker ───────────────────────────────────────────────────────────
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
    // Сохраняем hue при ахроматическом цвете, чтобы маркер не прыгал в 0
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
    if (!pickerOpen.value) return;
    const root = pickerRoot.value;
    if (root && root.contains(e.target as Node)) return;
    pickerOpen.value = false;
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

// Синхронизация HSV при внешнем изменении hex (hex-input / пресеты)
watch(() => settings.value.static_accent_color, (newHex) => {
    if (!pickerOpen.value) return;
    const currentHex = hsvToHex(hue.value, sat.value, val.value).toLowerCase();
    if (newHex.toLowerCase() === currentHex) return;
    syncHSVFromHex(newHex);
});

// Закрываем попап, если выключили статичный режим
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

function onBandInput(index: number, event: Event) {
    const value = parseFloat((event.target as HTMLInputElement).value);
    setBand(index, value);
}

function onGainInput(event: Event) {
    const value = parseFloat((event.target as HTMLInputElement).value);
    setGain(value);
}

function formatBandValue(value: number): string {
    const v = value.toFixed(1);
    return value > 0 ? `+${v}` : v;
}

// ── Artist separators ─────────────────────────────────────────────────────
const newSeparator = ref('');
const separatorInput = ref<HTMLInputElement | null>(null);

function visibleSeparator(sep: string): string {
    return `"${sep} "`;
}

async function addSeparator() {
    const raw = newSeparator.value;
    if (!raw) return;
    const list = settings.value.artist_separators;
    if (list.includes(raw)) {
        newSeparator.value = '';
        return;
    }
    settings.value.artist_separators = [...list, raw];
    newSeparator.value = '';
    await save();
    await rebuildArtists();
}

async function removeSeparator(sep: string) {
    const list = settings.value.artist_separators.filter(s => s !== sep);
    settings.value.artist_separators = list.length > 0 ? list : [...ARTIST_SEPARATORS_DEFAULT];
    await save();
    await rebuildArtists();
}

function onSeparatorKey(e: KeyboardEvent) {
    if (e.key === 'Enter') {
        e.preventDefault();
        addSeparator();
    }
}
</script>

<template>
    <div class="pageRoot">
        <h1 class="pageTitle">Настройки</h1>
        <div class="settingBlock">
            <div class="settingTitle">Настройки приложения</div>
            <div class="settingDesc">Кастомизация дополнительного функционала приложения</div>
            <div class="settingsBlock">
                <label v-for="item in settingItems" :key="item.key" class="settingElem">
                    <input type="checkbox" class="settingCheckbox"
                        :checked="settings[item.key]"
                        @change="toggle(item.key)">
                    <span>{{ item.label }}</span>
                </label>
                <div v-if="!settings.dynamic_accent_color" class="accentPicker" ref="pickerRoot">
                    <div class="accentPickerHead">
                        <button type="button"
                            class="accentSwatch"
                            :class="{ active: pickerOpen }"
                            :style="{ backgroundColor: settings.static_accent_color }"
                            @click="togglePicker"
                            aria-label="Выбрать цвет">
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

                    <div class="accentPresets">
                        <button v-for="color in ACCENT_PRESETS" :key="color"
                            type="button"
                            class="accentPresetBtn"
                            :class="{ active: isPresetActive(color) }"
                            :style="{ backgroundColor: color }"
                            :title="color"
                            @click="selectPreset(color)">
                        </button>
                    </div>
                </div>
            </div>
        </div>
        <div class="settingBlock">
            <div class="settingTitle">Эквалайзер</div>
            <div class="settingDesc">Настройка частотных полос аудио</div>

            <div class="eqBlock">
                <div class="eqHeader">
                    <label class="settingElem" style="margin: 0; padding: 0;">
                        <input type="checkbox" class="settingCheckbox"
                            :checked="eqEnabled"
                            @change="toggleEq">
                        <span>Включить эквалайзер</span>
                    </label>
                    <button class="eqResetBtn" @click="resetBands" :disabled="!eqEnabled">Сбросить</button>
                </div>

                <div class="eqPresets">
                    <button
                        v-for="preset in EQ_PRESETS"
                        :key="preset.name"
                        class="eqPresetBtn"
                        :class="{ active: activePreset === preset.name }"
                        :disabled="!eqEnabled"
                        @click="applyPreset(preset)"
                    >{{ preset.name }}</button>
                </div>

                <div class="eqGainRow" :class="{ disabled: !eqEnabled }">
                    <span class="eqGainLabel">Уровень</span>
                    <input
                        type="range"
                        class="eqGainSlider"
                        min="-12"
                        max="12"
                        step="0.1"
                        :value="eqGain"
                        :disabled="!eqEnabled"
                        @input="onGainInput($event)"
                    />
                    <span class="eqGainValue">{{ formatBandValue(eqGain) }} dB</span>
                </div>

                <div class="eqSliders" :class="{ disabled: !eqEnabled }">
                    <div class="eqBand" v-for="(label, i) in EQ_BAND_LABELS" :key="i">
                        <span class="eqBandValue">{{ formatBandValue(eqBands[i]) }}</span>
                        <div class="eqSliderWrap">
                            <input
                                type="range"
                                class="eqSlider"
                                min="-12"
                                max="12"
                                step="0.1"
                                :value="eqBands[i]"
                                :disabled="!eqEnabled"
                                @input="onBandInput(i, $event)"
                            />
                        </div>
                        <span class="eqBandLabel">{{ label }}</span>
                    </div>
                </div>
            </div>
        </div>

        <div class="settingBlock">
            <div class="settingTitle">Разделители исполнителей</div>
            <div class="settingDesc">Строки, по которым имена исполнителей разбиваются на несколько (например «; » или «, »)</div>
            <div class="separatorsBlock">
                <div class="separatorsList">
                    <div
                        v-for="sep in settings.artist_separators"
                        :key="sep"
                        class="separatorChip"
                        :title="JSON.stringify(sep)"
                    >
                        <span class="separatorText">{{ visibleSeparator(sep) }}</span>
                        <button class="separatorRemove" @click="removeSeparator(sep)" title="Удалить">
                            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                        </button>
                    </div>
                    <div class="separatorAddRow">
                        <input
                            ref="separatorInput"
                            type="text"
                            class="separatorInput"
                            placeholder="Новый разделитель"
                            v-model="newSeparator"
                            @keydown="onSeparatorKey"
                            spellcheck="false"
                        />
                        <button class="separatorAddBtn" :disabled="!newSeparator" @click="addSeparator">
                            <PlusIcon width="14" height="14" />
                            Добавить
                        </button>
                    </div>
                </div>
            </div>
        </div>

        <div class="settingBlock">
            <div class="settingTitle">Локальные папки</div>
            <div class="settingDesc">Папки сканируемые на аудио файлы</div>
            <div class="foldersBlock">
                <div class="foldersHat">
                    <button class="addFolder" @click="addFolder" :disabled="isScanning"><PlusIcon />Добавить папку</button>
                    <button class="rescanFolders" @click="rescanLibrary" :disabled="isScanning"><RefreshIcon />Пересканировать</button>
                </div>

                <div class="scanProgress" v-if="isScanning">
                    <div class="scanProgressHeader">
                        <span class="scanProgressLabel">
                            {{ scanProgress.total === 0 ? 'Подготовка...' : `${scanProgress.current} / ${scanProgress.total} файлов` }}
                        </span>
                        <span class="scanProgressPercent">{{ Math.round(scanProgress.percent) }}%</span>
                    </div>
                    <div class="scanProgressTrack">
                        <div class="scanProgressFill" :style="{ width: scanProgress.percent + '%' }" />
                    </div>
                </div>

                <div class="foldersEmpty" v-if="!folders.length && !isScanning">
                    <span>Нет добавленных папок</span>
                </div>

                <div
                    v-for="folder in sortedFolders"
                    :key="folder.path"
                    class="folderElem"
                    :class="{ 'folderMissing': !folder.exists }"
                >
                    <div class="icon"><FolderIcon /></div>
                    <div class="folderInfo">
                        <div class="folderPath">{{ folderDisplayName(folder.path) }}</div>
                        <div class="folderTracks" v-if="folder.exists">{{ folder.track_count }} Треков</div>
                        <div class="folderMissingLabel" v-else>Папка недоступна</div>
                    </div>
                    <button
                        class="removeBtn"
                        @click="removeFolder(folder.path)"
                        :disabled="removingFolder === folder.path"
                    >
                        <DeleteIcon />
                    </button>
                </div>
            </div>
        </div>
    </div>
</template>

<style scoped>
.pageRoot {
    padding: 20px 28px;
    display: flex;
    flex-direction: column;
    width: 100%;
    min-height: 100%;
    background-color: var(--light-bg);

    background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
    background-repeat: no-repeat;
    background-attachment: local;

    transition: .24s ease;
}
.pageTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 900;
    font-size: 36px;
    color: #ffffff;
    margin-bottom: 48px;
}
.settingBlock {
    display: flex;
    flex-direction: column;
    margin-bottom: 36px;
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
    font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
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
.settingElem {
    padding: 12px 0px;
    display: flex;
    flex-direction: row;
    align-items: start;
    width: 100%;
    gap: 16px;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    font-weight: 600;
    font-size: 16px;
}
.settingElem span {
    user-select: none;
    transition: .24s ease-in-out;

    &:hover {
        color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 90%, 0.62);
    }
}
.settingCheckbox {
    appearance: none;
    width: 32px;
    height: 19.2px;
    background: var(--dark-bg);
    border-radius: 31px;
    position: relative;
    cursor: pointer;
    transition: 0.24s ease;
}
.settingCheckbox::after {
    content: '';
    position: absolute;
    width: 16px;
    height: 16px;
    top: 1.5px;
    background: hsla(0, 0%, 89%, 1);
    border-radius: 50%;
    transition: 0.24s ease;
}
.settingCheckbox:checked {
    background: #fff;
}
.settingCheckbox:checked + span {
    color: hsla(0, 0%, 90.1%, 1);
}
.settingCheckbox:checked::after {
    transform: translateX(14px);
    background: var(--dark-bg);
}
.accentPicker {
    margin-top: 8px;
    padding: 14px 16px 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    background-color: hsla(var(--accent-h), min(calc(var(--accent-s) * 1.25), 50%), 16%, 1);
    border-radius: 10px;
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
    width: 52px;
    height: 52px;
    border-radius: 14px;
    cursor: pointer;
    overflow: hidden;
    transition: transform 0.18s ease, box-shadow 0.24s ease;
    flex-shrink: 0;
    padding: 0;
    border: 0;

    &:hover {
        box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.22);
    }
    &.active {
        box-shadow: inset 0 0 0 2px #ffffff, 0 4px 14px hsla(0, 0%, 0%, 0.4);
    }
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
.accentPresets {
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
    gap: 8px;
}
.accentPresetBtn {
    width: 28px;
    height: 28px;
    border-radius: 9px;
    cursor: pointer;
    background-color: transparent;
    transition: transform 0.18s ease, box-shadow 0.24s ease;

    &:hover {
        box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.22);
    }
    &.active {
        box-shadow: inset 0 0 0 2px #ffffff, 0 0 0 2px hsla(0, 0%, 0%, 0.35);
    }
}
.separatorsBlock {
    margin-top: 16px;
    max-width: 800px;
    width: 100%;
}
.separatorsList {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
}
.separatorChip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 5px 6px 5px 12px;
    border-radius: 999px;
    background-color: hsla(var(--accent-h), min(calc(var(--accent-s) * 1.25), 50%), 16%, 1);
    color: hsla(0, 0%, 100%, 0.85);
    font-family: "JetBrains Mono", "Consolas", monospace;
    font-size: 13px;
    font-weight: 500;
    transition: .24s ease;
}
.separatorChip:hover {
    background-color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
}
.separatorText {
    user-select: text;
    letter-spacing: 0.04em;
}
.separatorRemove {
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    color: hsla(0, 0%, 100%, 0.45);
    background-color: transparent;
    transition: .2s ease;
}
.separatorRemove:hover {
    color: #ff6467;
    background-color: hsla(0, 0%, 0%, 0.25);
}
.separatorAddRow {
    display: inline-flex;
    align-items: center;
    gap: 4px;
}
.separatorInput {
    padding: 6px 10px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 0%, 0.25);
    color: #ffffff;
    font-family: "JetBrains Mono", "Consolas", monospace;
    font-size: 13px;
    font-weight: 500;
    width: 160px;
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.04);
    transition: .2s ease;
}
.separatorInput:focus {
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.18);
    background-color: hsla(0, 0%, 0%, 0.35);
}
.separatorInput::placeholder {
    color: hsla(0, 0%, 100%, 0.3);
}
.separatorAddBtn {
    user-select: none;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 6px 12px 6px 9px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.6);
    font-weight: 500;
    font-size: 13px;
    cursor: pointer;
    transition: .24s ease;
}
.separatorAddBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: #ffffff;
}
.separatorAddBtn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
}
.foldersBlock {
    margin-top: 22px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 800px;
    width: 100%;
}
.folderElem {
    display: flex;
    flex-direction: row;
    align-items: center;
    width: 100%;
    gap: 16px;
    border-radius: 8px;
    padding: 6px 8px 4px;
    background-color: hsla(var(--accent-h), min(calc(var(--accent-s) * 1.25), 50%), 16%, 1);
    transition: .24s ease;

    &:hover {
        background-color: hsla(var(--accent-h), var(--accent-s), 17%, 1);
    }
}
.folderElem .icon {
    padding: 0px 6px 0px 8px;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);

    transition: .24s ease-in-out;

    &:hover {
        color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 90%, 0.62);
    }
}
.folderInfo {
    display: flex;
    flex-direction: column;
    gap: 2px;
}
.folderPath {
    font-weight: 400;
    font-size: 12px;
    color: #ffffff;
}
.folderTracks {
    user-select: none;
    font-weight: 600;
    font-size: 12px;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);

    transition: .24s ease-in-out;

    &:hover {
        color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 90%, 0.62);
    }
}
.removeBtn {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    border-radius: 4px;
    padding: 4px;
    background-color: hsla(var(--accent-h), var(--accent-s), 21%, 1);
    margin-left: auto;
    transition: .3s ease;

    &:hover {
        color: #ff6467;
        background-color: hsla(var(--accent-h), var(--accent-s), 13%, 1);
    }
}
.foldersHat {
    display: flex;
    flex-direction: row;
    gap: 7px;
    align-items: center;
    margin-bottom: 3px;
}
.addFolder {
    user-select: none;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 4px;
    padding: 6px 12px 6px 9px;
    border-radius: 8px;
    background-color: hsla(0, 0%, 100%, 0);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    font-weight: 500;
    font-size: 15px;
    cursor: pointer;
    transition: .24s ease;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.08);
        color: #ffffff;
    }
}
.rescanFolders {
    user-select: none;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 7px;
    padding: 6px 12px;
    border-radius: 8px;
    background-color: hsla(0, 0%, 100%, 0);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    font-weight: 500;
    font-size: 15px;
    cursor: pointer;
    transition: .2s;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.08);
        color: #ffffff;
    }
}
.addFolder:disabled,
.rescanFolders:disabled {
    opacity: 0.35;
    cursor: not-allowed;
}
.scanProgress {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 14px;
    background-color: var(--dark-bg);
    border-radius: 8px;
}
.scanProgressHeader {
    display: flex;
    justify-content: space-between;
    align-items: center;
}
.scanProgressLabel {
    font-size: 14px;
    font-weight: 500;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
}
.scanProgressPercent {
    font-size: 14px;
    font-weight: 700;
    color: #ffffff;
    font-variant-numeric: tabular-nums;
}
.scanProgressTrack {
    height: 4px;
    background: hsla(0, 0%, 100%, 0.08);
    border-radius: 999px;
    overflow: hidden;
}
.scanProgressFill {
    height: 100%;
    background: #ffffff;
    border-radius: 999px;
    transition: width 0.15s linear;
}
.foldersEmpty {
    padding: 16px;
    text-align: center;
    color: hsla(0, 0%, 44.1%, 1);
    font-weight: 500;
    font-size: 15px;
}
.folderMissing {
    border: 1px solid hsla(0, 80%, 60%, 0.2);
    background-color: hsla(0, 80%, 60%, 0.05);
}
.folderMissingLabel {
    font-weight: 600;
    font-size: 14px;
    color: #ff6467;
}
.eqBlock {
    margin-top: 16px;
    max-width: 800px;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 16px;
}
.eqHeader {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
}
.eqResetBtn {
    user-select: none;
    padding: 5px 14px;
    border-radius: 6px;
    background-color: hsla(0, 0%, 100%, 0);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    font-weight: 500;
    font-size: 13px;
    cursor: pointer;
    transition: .24s ease;

    &:hover {
        background-color: hsla(0, 0%, 100%, 0.08);
        color: #ffffff;
    }
    &:disabled {
        opacity: 0.35;
        cursor: not-allowed;
    }
}
.eqPresets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
}
.eqPresetBtn {
    user-select: none;
    padding: 5px 12px;
    border-radius: 6px;
    background-color: hsla(var(--accent-h), min(calc(var(--accent-s) * 1.25), 50%), 16%, 1);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    font-weight: 500;
    font-size: 13px;
    cursor: pointer;
    transition: .24s ease;

    &:hover {
        background-color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
        color: #ffffff;
    }
    &.active {
        background-color: hsla(0, 0%, 100%, 0.15);
        color: #ffffff;
    }
    &:disabled {
        opacity: 0.35;
        cursor: not-allowed;
    }
}
.eqGainRow {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    background-color: hsla(var(--accent-h), min(calc(var(--accent-s) * 1.25), 50%), 16%, 1);
    border-radius: 10px;
    transition: .24s ease;

    &.disabled {
        opacity: 0.35;
        pointer-events: none;
    }
}
.eqGainLabel {
    font-size: 13px;
    font-weight: 600;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    white-space: nowrap;
}
.eqGainSlider {
    appearance: none;
    flex: 1;
    height: 4px;
    background: hsla(0, 0%, 100%, 0.1);
    border-radius: 999px;
    cursor: pointer;
}
.eqGainSlider::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #ffffff;
    cursor: pointer;
    transition: transform 0.15s ease;
}
.eqGainSlider::-webkit-slider-thumb:hover {
    transform: scale(1.2);
}
.eqGainValue {
    font-size: 12px;
    font-weight: 600;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    font-variant-numeric: tabular-nums;
    min-width: 52px;
    text-align: right;
}
.eqSliders {
    display: flex;
    flex-direction: row;
    justify-content: space-between;
    align-items: center;
    gap: 4px;
    padding: 16px 12px;
    background-color: hsla(var(--accent-h), min(calc(var(--accent-s) * 1.25), 50%), 16%, 1);
    border-radius: 10px;
    transition: .24s ease;

    &.disabled {
        opacity: 0.35;
        pointer-events: none;
    }
}
.eqBand {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    flex: 1;
}
.eqBandValue {
    font-size: 11px;
    font-weight: 600;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    font-variant-numeric: tabular-nums;
    min-width: 28px;
    text-align: center;
}
.eqBandLabel {
    font-size: 11px;
    font-weight: 600;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
}
.eqSliderWrap {
    height: 120px;
    width: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    position: relative;
}
.eqSlider {
    appearance: none;
    width: 120px;
    height: 4px;
    background: hsla(0, 0%, 100%, 0.1);
    border-radius: 999px;
    cursor: pointer;
    transform: rotate(-90deg);
    transform-origin: center center;
}
.eqSlider::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #ffffff;
    cursor: pointer;
    transition: transform 0.15s ease;
}
.eqSlider::-webkit-slider-thumb:hover {
    transform: scale(1.2);
}
</style>
