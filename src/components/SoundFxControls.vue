<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import SettingToggle from './SettingToggle.vue'
import { useSoundFx, formatSpeed, SPEED_MIN, SPEED_MAX, SPEED_PRESETS, type FxEffect, type FxSettings } from '../composables/useSoundFx'
import { useSleepTimer, SLEEP_PRESETS } from '../composables/useSleepTimer'
import { useEqualizer, EQ_PRESETS, EQ_BAND_LABELS } from '../composables/useEqualizer'

export type SoundSection = 'eq' | 'space' | 'crusher' | 'vinyl' | 'speed' | 'sleep'

const props = defineProps<{ section: SoundSection; compact?: boolean }>()

const { fx, update, toggle, setSpeed } = useSoundFx()
const sleep = useSleepTimer()
const { eqEnabled, eqBands, activePreset, toggleEnabled: toggleEq, setBand, applyPreset, resetBands } = useEqualizer()

interface SliderDef {
    label: string
    min: number
    max: number
    step: number
    get: (s: FxSettings) => number
    set: (s: FxSettings, v: number) => void
    format: (v: number) => string
}

const pct = (v: number) => `${Math.round(v * 100)}%`

const SLIDERS: Record<FxEffect, SliderDef[]> = {
    space: [
        { label: 'Объём', min: 0, max: 1, step: 0.01, get: s => s.space.mix, set: (s, v) => { s.space.mix = v }, format: pct },
        { label: 'Размер', min: 0, max: 1, step: 0.01, get: s => s.space.size, set: (s, v) => { s.space.size = v }, format: pct },
    ],
    crusher: [
        { label: 'Разрядность', min: 2, max: 16, step: 1, get: s => s.crusher.bits, set: (s, v) => { s.crusher.bits = v }, format: v => `${v} бит` },
        { label: 'Понижение частоты', min: 1, max: 8, step: 0.1, get: s => s.crusher.downsample, set: (s, v) => { s.crusher.downsample = v }, format: v => `${v.toFixed(1)}×` },
    ],
    vinyl: [
        { label: 'Треск', min: 0, max: 1, step: 0.01, get: s => s.vinyl.crackle, set: (s, v) => { s.vinyl.crackle = v }, format: pct },
        { label: 'Износ', min: 0, max: 1, step: 0.01, get: s => s.vinyl.wear, set: (s, v) => { s.vinyl.wear = v }, format: pct },
    ],
}

const effect = computed<FxEffect | null>(() =>
    props.section === 'space' || props.section === 'crusher' || props.section === 'vinyl' ? props.section : null)

const toggleItem = { label: 'Включить' }

function fill(def: { min: number; max: number }, v: number) {
    return `${((v - def.min) / (def.max - def.min)) * 100}%`
}

/** Moving a slider of a switched-off effect turns it on. */
function onSlider(def: SliderDef, e: Event) {
    const v = parseFloat((e.target as HTMLInputElement).value)
    const eff = effect.value
    update(s => {
        def.set(s, v)
        if (eff) s[eff].enabled = true
    })
}

/** EQ bands are bipolar: fill from the 0 dB line to the value. */
function bandFill(v: number) {
    const p = ((v + 12) / 24) * 100
    return { '--lo': `${Math.min(50, p)}%`, '--hi': `${Math.max(50, p)}%` }
}

function onSpeed(e: Event) {
    setSpeed(parseFloat((e.target as HTMLInputElement).value))
}

function onBand(i: number, e: Event) {
    setBand(i, parseFloat((e.target as HTMLInputElement).value))
}

function sleepLabel(min: number) {
    return min < 60 ? `${min} мин` : `${min / 60} ч`
}

const customMinutes = ref(
    sleep.mode.value === 'time' && !SLEEP_PRESETS.includes(sleep.minutes.value) ? sleep.minutes.value : 20)
function startCustom() {
    const m = Math.round(Number(customMinutes.value))
    if (!Number.isFinite(m) || m < 1) return
    customMinutes.value = Math.min(600, m)
    sleep.startTime(customMinutes.value)
}
const customActive = computed(() =>
    sleep.mode.value === 'time' && !SLEEP_PRESETS.includes(sleep.minutes.value))

const presetsEl = ref<HTMLElement | null>(null)
const presetsFade = ref({ start: false, end: false })
function updatePresetsFade() {
    const el = presetsEl.value
    if (!el) return
    presetsFade.value = {
        start: el.scrollLeft > 2,
        end: el.scrollLeft + el.clientWidth < el.scrollWidth - 2,
    }
}
function onPresetsWheel(e: WheelEvent) {
    const el = presetsEl.value
    if (!el || el.scrollWidth <= el.clientWidth) return
    const delta = Math.abs(e.deltaY) > Math.abs(e.deltaX) ? e.deltaY : e.deltaX
    if (!delta) return
    e.preventDefault()
    el.scrollLeft += delta
    updatePresetsFade()
}
function onPresetClick(p: (typeof EQ_PRESETS)[number], e: MouseEvent) {
    applyPreset(p)
    ;(e.currentTarget as HTMLElement).scrollIntoView({ behavior: 'smooth', block: 'nearest', inline: 'center' })
}
onMounted(() => {
    void nextTick(() => {
        const active = presetsEl.value?.querySelector<HTMLElement>('.chip.active')
        if (active && presetsEl.value) {
            presetsEl.value.scrollLeft = active.offsetLeft - (presetsEl.value.clientWidth - active.offsetWidth) / 2
        }
        updatePresetsFade()
    })
})
</script>

<template>
    <div class="fx" :class="{ compact }">
        <template v-if="effect">
            <SettingToggle compact :item="toggleItem" :checked="fx[effect].enabled" @toggle="toggle(effect)" />
            <div class="sliders" :class="{ off: !fx[effect].enabled }">
                <label v-for="def in SLIDERS[effect]" :key="def.label" class="sliderRow">
                    <span class="sliderHead">
                        <span class="sliderLabel">{{ def.label }}</span>
                        <span class="sliderValue">{{ def.format(def.get(fx)) }}</span>
                    </span>
                    <input
                        type="range"
                        class="slider"
                        :min="def.min"
                        :max="def.max"
                        :step="def.step"
                        :value="def.get(fx)"
                        :style="{ '--fill': fill(def, def.get(fx)) }"
                        @input="onSlider(def, $event)"
                    />
                </label>
            </div>
        </template>

        <template v-else-if="section === 'speed'">
            <div class="speedHead">
                <span class="speedValue">{{ formatSpeed(fx.speed) }}</span>
                <button v-if="fx.speed !== 1" class="textBtn" @click="setSpeed(1)">Сбросить</button>
            </div>
            <input
                type="range"
                class="slider"
                :min="SPEED_MIN"
                :max="SPEED_MAX"
                step="0.05"
                :value="fx.speed"
                :style="{ '--fill': fill({ min: SPEED_MIN, max: SPEED_MAX }, fx.speed) }"
                @input="onSpeed"
            />
            <div class="chips">
                <button
                    v-for="p in SPEED_PRESETS"
                    :key="p"
                    class="chip"
                    :class="{ active: Math.abs(fx.speed - p) < 0.001 }"
                    @click="setSpeed(p)"
                >{{ formatSpeed(p) }}</button>
            </div>
        </template>

        <template v-else-if="section === 'sleep'">
            <div v-if="sleep.mode.value !== 'off'" class="sleepStatus">
                <span class="sleepText">
                    <template v-if="sleep.mode.value === 'time'">Пауза через <b>{{ sleep.remaining.value }}</b></template>
                    <template v-else>Пауза после этого трека</template>
                </span>
                <button class="textBtn" @click="sleep.cancel()">Отменить</button>
            </div>
            <div class="chips">
                <button
                    v-for="m in SLEEP_PRESETS"
                    :key="m"
                    class="chip"
                    :class="{ active: sleep.mode.value === 'time' && sleep.minutes.value === m }"
                    @click="sleep.startTime(m)"
                >{{ sleepLabel(m) }}</button>
                <button
                    class="chip"
                    :class="{ active: sleep.mode.value === 'track' }"
                    @click="sleep.startAfterTrack()"
                >До конца трека</button>
            </div>
            <form class="customRow" @submit.prevent="startCustom">
                <span class="sliderLabel">Своё время</span>
                <span class="minField" :class="{ active: customActive }">
                    <input
                        v-model.number="customMinutes"
                        class="minInput"
                        type="number"
                        min="1"
                        max="600"
                        inputmode="numeric"
                        aria-label="Минуты"
                    />
                    <span class="minUnit">мин</span>
                </span>
                <button type="submit" class="chip">Запустить</button>
            </form>
        </template>

        <template v-else-if="section === 'eq'">
            <div class="eqHead">
                <SettingToggle compact :item="toggleItem" :checked="eqEnabled" @toggle="toggleEq" />
            </div>
            <div
                ref="presetsEl"
                class="chips eqPresets"
                :class="{ fadeStart: presetsFade.start, fadeEnd: presetsFade.end }"
                @wheel="onPresetsWheel"
                @scroll.passive="updatePresetsFade"
            >
                <button
                    v-for="p in EQ_PRESETS"
                    :key="p.name"
                    class="chip"
                    :class="{ active: activePreset === p.name }"
                    :disabled="!eqEnabled"
                    @click="onPresetClick(p, $event)"
                >{{ p.name }}</button>
            </div>
            <div class="bands" :class="{ off: !eqEnabled }">
                <div v-for="(label, i) in EQ_BAND_LABELS" :key="i" class="band">
                    <input
                        type="range"
                        class="bandSlider"
                        min="-12"
                        max="12"
                        step="0.5"
                        :value="eqBands[i]"
                        :disabled="!eqEnabled"
                        :style="bandFill(eqBands[i])"
                        @input="onBand(i, $event)"
                    />
                    <span class="bandLabel">{{ label }}</span>
                </div>
            </div>
            <button class="textBtn eqReset" :disabled="!eqEnabled" @click="resetBands">Сбросить</button>
        </template>
    </div>
</template>

<style scoped>
.fx {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 800px;
    width: 100%;
}
.fx.compact {
    gap: 12px;
}

.sliders,
.bands {
    display: flex;
    flex-direction: column;
    gap: 14px;
    transition: opacity var(--duration-hover) var(--ease-smooth);
}
.sliders.off,
.bands.off {
    opacity: 0.45;
}
.sliderRow {
    display: flex;
    flex-direction: column;
    gap: 8px;
}
.sliderHead {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: 13px;
}
.sliderLabel {
    color: hsla(0, 0%, 100%, 0.6);
}
.sliderValue {
    color: #fff;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
}

.slider {
    appearance: none;
    -webkit-appearance: none;
    width: 100%;
    height: 6px;
    margin: 4px 0;
    border-radius: 999px;
    background: linear-gradient(
        to right,
        hsl(var(--accent-h), max(var(--accent-s), 55%), 62%) var(--fill, 0%),
        hsla(0, 0%, 100%, 0.08) var(--fill, 0%)
    );
    cursor: pointer;
}
.slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.3);
    transition: transform .18s var(--ease-smooth);
}
.slider:hover::-webkit-slider-thumb,
.slider:active::-webkit-slider-thumb {
    transform: scale(1.15);
}

.speedHead,
.sleepStatus {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
}
.speedValue {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 26px;
    font-weight: 800;
    color: #fff;
    font-variant-numeric: tabular-nums;
}
.sleepText {
    font-size: 14px;
    color: hsla(0, 0%, 100%, 0.7);
}
.sleepText b {
    color: hsl(var(--accent-h), max(var(--accent-s), 55%), 72%);
    font-variant-numeric: tabular-nums;
}

.chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
}
.chip {
    height: 30px;
    padding: 0 12px;
    border: none;
    border-radius: 999px;
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 19%);
    color: hsla(0, 0%, 100%, 0.7);
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.chip:hover:not(:disabled) {
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 24%);
    color: #fff;
}
.chip.active {
    background-color: hsl(var(--accent-h), max(var(--accent-s), 55%), 62%);
    color: #fff;
}
.chip:disabled {
    opacity: 0.4;
    cursor: default;
}

.textBtn {
    flex-shrink: 0;
    padding: 6px 10px;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.55);
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.textBtn:hover:not(:disabled) {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: #fff;
}
.textBtn:disabled {
    opacity: 0.4;
    cursor: default;
}

.eqPresets {
    flex-wrap: nowrap;
    overflow-x: auto;
    scrollbar-width: none;
    margin: 0 -2px;
    padding: 0 2px;
}
.eqPresets::-webkit-scrollbar {
    display: none;
}
.eqPresets.fadeStart {
    mask-image: linear-gradient(to right, transparent, #000 32px);
}
.eqPresets.fadeEnd {
    mask-image: linear-gradient(to left, transparent, #000 32px);
}
.eqPresets.fadeStart.fadeEnd {
    mask-image: linear-gradient(to right, transparent, #000 32px, #000 calc(100% - 32px), transparent);
}
.fx:not(.compact) .eqPresets {
    mask-image: none;
}

.customRow {
    display: flex;
    align-items: center;
    gap: 8px;
}
.customRow .sliderLabel {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    white-space: nowrap;
}
.minField {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 30px;
    padding: 0 12px;
    border-radius: 999px;
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 19%);
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.minField:focus-within {
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 24%);
}
.minField.active .minInput {
    color: hsl(var(--accent-h), max(var(--accent-s), 55%), 72%);
}
.minInput {
    width: 34px;
    border: none;
    outline: none;
    background: transparent;
    color: #fff;
    font-family: inherit;
    font-size: 13px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    text-align: right;
    appearance: textfield;
    -moz-appearance: textfield;
}
.minInput::-webkit-outer-spin-button,
.minInput::-webkit-inner-spin-button {
    -webkit-appearance: none;
    margin: 0;
}
.minUnit {
    font-size: 13px;
    color: hsla(0, 0%, 100%, 0.5);
}
.fx:not(.compact) .eqPresets {
    flex-wrap: wrap;
}
.bands {
    flex-direction: row;
    justify-content: space-between;
    gap: 4px;
}
.band {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
}
.bandSlider {
    appearance: none;
    -webkit-appearance: none;
    writing-mode: vertical-lr;
    direction: rtl;
    width: 6px;
    height: 110px;
    border-radius: 999px;
    background: linear-gradient(
        to top,
        hsla(0, 0%, 100%, 0.08) var(--lo, 50%),
        hsl(var(--accent-h), max(var(--accent-s), 55%), 62%) var(--lo, 50%),
        hsl(var(--accent-h), max(var(--accent-s), 55%), 62%) var(--hi, 50%),
        hsla(0, 0%, 100%, 0.08) var(--hi, 50%)
    );
    cursor: pointer;
}
.bandSlider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #fff;
}
.bandSlider:disabled {
    cursor: default;
}
.bandLabel {
    font-size: 10px;
    color: hsla(0, 0%, 100%, 0.42);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
}
.eqReset {
    align-self: flex-end;
}
</style>
