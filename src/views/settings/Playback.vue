<script setup lang="ts">
import { onMounted, onUnmounted, computed, ref } from 'vue';
import { useSettings, type SettingKey } from '../../composables/useSettings';
import { useEqualizer, EQ_PRESETS, EQ_BAND_LABELS } from '../../composables/useEqualizer';
import SettingsShell from '../../components/SettingsShell.vue';
import SettingToggle from '../../components/SettingToggle.vue';
import { useEnhance, DEVICE_PROFILES, type DeviceProfile } from '../../composables/useEnhance';
import { useI18n } from '../../composables/useI18n';

const { t } = useI18n();
const { settings, settingItems, load, save } = useSettings();
const { status: enhance, load: loadEnhance, toggleEnabled: toggleEnhance, setDevice } = useEnhance();
const enhanceToggleItem = computed(() => ({ label: t('settings.enhance.title') }));
const deviceLabel = (p: DeviceProfile | null) => p
    ? t(`settings.enhance.profiles.${p}`)
    : `${t('settings.enhance.auto')} (${t(`settings.enhance.profiles.${enhance.value.detected}`)})`;

const deviceMenuOpen = ref(false);
const deviceMenuRoot = ref<HTMLElement | null>(null);

function chooseDevice(p: DeviceProfile | null) {
    deviceMenuOpen.value = false;
    void setDevice(p);
}
function onDocMouseDown(e: MouseEvent) {
    if (deviceMenuOpen.value && !deviceMenuRoot.value?.contains(e.target as Node)) deviceMenuOpen.value = false;
}
function onDocKey(e: KeyboardEvent) {
    if (e.key === 'Escape') deviceMenuOpen.value = false;
}
onMounted(() => {
    document.addEventListener('mousedown', onDocMouseDown);
    document.addEventListener('keydown', onDocKey);
});
onUnmounted(() => {
    document.removeEventListener('mousedown', onDocMouseDown);
    document.removeEventListener('keydown', onDocKey);
});

const {
    eqEnabled, eqBands, eqGain, activePreset,
    load: loadEq, toggleEnabled: toggleEq,
    setBand, setGain, applyPreset, resetBands,
} = useEqualizer();

const playbackKeys: SettingKey[] = ['crossfade_on_seek', 'crossfade_between_tracks'];
const playbackItems = computed(() => playbackKeys.map(key => settingItems.find(i => i.key === key)!));
const eqToggleItem = { label: 'Включить эквалайзер' };

async function toggle(key: SettingKey) {
    settings.value[key] = !settings.value[key];
    await save();
}

onMounted(async () => {
    await load();
    await loadEq();
    await loadEnhance();
});

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
</script>

<template>
    <SettingsShell title="Воспроизведение" description="Поведение плеера и эквалайзер">
        <div class="settingBlock">
            <div class="settingsBlock">
                <SettingToggle
                    v-for="item in playbackItems"
                    :key="item.key"
                    :item="item"
                    :checked="settings[item.key]"
                    @toggle="toggle(item.key)"
                />
            </div>
        </div>

        <div class="settingBlock">
            <div class="enhanceRow">
                <SettingToggle compact :item="enhanceToggleItem" :checked="enhance.enabled" @toggle="toggleEnhance" />
                <div class="deviceDropdown" :class="{ disabled: !enhance.enabled }" ref="deviceMenuRoot">
                    <button
                        type="button"
                        class="deviceTrigger"
                        :class="{ open: deviceMenuOpen }"
                        :disabled="!enhance.enabled"
                        :title="enhance.device_name ?? undefined"
                        @click="deviceMenuOpen = !deviceMenuOpen"
                        :aria-expanded="deviceMenuOpen"
                        aria-haspopup="listbox"
                    >
                        <span class="deviceCurrent">{{ deviceLabel(enhance.device) }}</span>
                        <svg class="deviceChevron" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                            <path d="m6 9 6 6 6-6" />
                        </svg>
                    </button>
                    <transition name="deviceMenu">
                        <div v-if="deviceMenuOpen" class="deviceMenu" role="listbox">
                            <button
                                v-for="p in [null, ...DEVICE_PROFILES]"
                                :key="p ?? 'auto'"
                                type="button"
                                class="deviceOption"
                                :class="{ active: enhance.device === p }"
                                role="option"
                                :aria-selected="enhance.device === p"
                                @click="chooseDevice(p)"
                            >{{ deviceLabel(p) }}</button>
                        </div>
                    </transition>
                </div>
            </div>
        </div>

        <div class="settingBlock">
            <div class="settingTitle">Эквалайзер</div>
            <div class="settingDesc">Настройка частотных полос аудио</div>

            <div class="eqBlock">
                <div class="eqHeader">
                    <SettingToggle compact :item="eqToggleItem" :checked="eqEnabled" @toggle="toggleEq" />
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
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-width: 800px;
    width: 100%;
}
.enhanceRow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    max-width: 800px;
    width: 100%;
}
.deviceDropdown {
    position: relative;
    min-width: 200px;
    transition: opacity .24s ease;

    &.disabled {
        opacity: 0.35;
    }
}
.deviceTrigger {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 8px 12px;
    border-radius: 36px;
    background-color: hsla(var(--accent-h), min(calc(var(--accent-s) * 1.25), 50%), 16%, 1);
    color: #ffffff;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color .2s ease;
}
.deviceTrigger:hover:not(:disabled),
.deviceTrigger.open {
    background-color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
}
.deviceTrigger:disabled {
    cursor: not-allowed;
}
.deviceCurrent {
    flex: 1;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.deviceChevron {
    flex-shrink: 0;
    color: hsla(0, 0%, 100%, 0.48);
    transition: transform .18s ease;
}
.deviceTrigger.open .deviceChevron {
    transform: rotate(180deg);
}
.deviceMenu {
    display: flex;
    flex-direction: column;
    gap: 3px;
    position: absolute;
    top: calc(100% + 7px);
    right: 0;
    z-index: 20;
    width: 100%;
    padding: 5px;
    border-radius: 20px;
    background-color: hsla(var(--accent-h), min(calc(var(--accent-s) * 1.25), 50%), 12%, 0.98);
    box-shadow: 0 14px 32px hsla(0, 0%, 0%, 0.34), inset 0 0 0 1px hsla(0, 0%, 100%, 0.08);
    backdrop-filter: blur(14px);
}
.deviceOption {
    width: 100%;
    padding: 8px 10px;
    border-radius: 16px;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.68);
    font-size: 14px;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
    transition: background-color .18s ease, color .18s ease;
}
.deviceOption:hover,
.deviceOption.active {
    background-color: hsla(0, 0%, 100%, 0.09);
    color: #ffffff;
}
.deviceMenu-enter-active,
.deviceMenu-leave-active {
    transition: opacity .16s ease, transform .16s ease;
    transform-origin: top right;
}
.deviceMenu-enter-from,
.deviceMenu-leave-to {
    opacity: 0;
    transform: translateY(-4px) scale(0.98);
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

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .settingBlock {
        margin-bottom: 30px;
    }
    .settingsBlock,
    .eqBlock {
        max-width: none;
    }
    .eqHeader,
    .eqGainRow,
    .enhanceRow {
        flex-wrap: wrap;
    }
    .deviceDropdown {
        width: 100%;
    }
    .eqResetBtn,
    .eqPresetBtn {
        min-height: 44px;
        justify-content: center;
        border-radius: 12px;
        display: inline-flex;
        align-items: center;
    }
    .eqPresets {
        gap: 8px;
    }
    .eqGainRow {
        align-items: center;
        padding: 12px;
    }
    .eqGainSlider {
        min-width: 180px;
    }
    .eqSliders {
        justify-content: flex-start;
        gap: 10px;
        padding: 16px 10px;
        overflow-x: auto;
        border-radius: 14px;
    }
    .eqBand {
        flex: 0 0 38px;
    }
    .eqSliderWrap {
        height: 130px;
    }
}
</style>
