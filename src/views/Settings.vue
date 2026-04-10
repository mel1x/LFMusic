<script setup lang="ts">
import { onMounted, computed } from 'vue';
import DeleteIcon from '../components/Icons/DeleteIcon.vue';
import FolderIcon from '../components/Icons/FolderIcon.vue';
import { useSettings } from '../composables/useSettings';
import { usePlayer } from '../composables/usePlayer';
import { useEqualizer, EQ_PRESETS, EQ_BAND_LABELS } from '../composables/useEqualizer';
import RefreshIcon from '../components/Icons/RefreshIcon.vue';
import PlusIcon from '../components/Icons/PlusIcon.vue';

const { settings, settingItems, load, save } = useSettings();
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
    settings.value[key] = !settings.value[key];
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
    background-color: hsla(var(--accent-h), var(--accent-s), 16%, 1);
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
    background-color: hsla(var(--accent-h), var(--accent-s), 16%, 1);
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
    background-color: hsla(var(--accent-h), var(--accent-s), 16%, 1);
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
    background-color: hsla(var(--accent-h), var(--accent-s), 16%, 1);
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
