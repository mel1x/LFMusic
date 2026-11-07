<script setup lang="ts">
import { onMounted, onUnmounted, computed, ref, watch } from 'vue';
import { useSettings, type Language, type SettingKey } from '../../composables/useSettings';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from '../../composables/useI18n';
import { usePlayer } from '../../composables/usePlayer';
import { useUpdater } from '../../composables/useUpdater';
import { getVersion } from '@tauri-apps/api/app';
import SettingsShell from '../../components/SettingsShell.vue';
import SettingToggle from '../../components/SettingToggle.vue';

const { settings, settingItems, devModeActive, load, save } = useSettings();
const { t, setLanguage } = useI18n();

const updater = useUpdater();
const { saveState } = usePlayer();

const generalKeys: SettingKey[] = ['discord_rpc_enabled', 'auto_update_check', 'dev_mode'];
const generalItems = computed(() => generalKeys
    .filter(key => key !== 'auto_update_check' || updater.supported.value)
    .map(key => settingItems.find(i => i.key === key)!));

const updateStatusText = computed(() => {
    switch (updater.status.value) {
        case 'checking': return t('updater.checking');
        case 'latest': return t('updater.latest');
        case 'error': return t('updater.error');
        case 'downloading': return t('updater.downloading', { percent: updater.progress.value });
        default: return updater.availableVersion.value
            ? t('updater.available', { version: updater.availableVersion.value })
            : '';
    }
});

function onUpdateClick() {
    if (updater.availableVersion.value) updater.install(saveState);
    else updater.check();
}

const appVersion = ref('');
onMounted(async () => {
    appVersion.value = await getVersion().catch(() => '');
});

async function toggle(key: SettingKey) {
    settings.value[key] = !settings.value[key];
    await save();
}

// Dev mode is read once at startup, so offer a restart while it differs.
const devRestartPending = computed(() => settings.value.dev_mode !== devModeActive.value);

async function restartApp() {
    await saveState();
    await invoke('restart_app');
}

onMounted(load);

const languageDropdownOpen = ref(false);
const languageDropdownRoot = ref<HTMLElement | null>(null);
const languageOptions: { value: Language; labelKey: string; short: string }[] = [
    { value: 'ru', labelKey: 'settings.russian', short: 'RU' },
    { value: 'en', labelKey: 'settings.english', short: 'EN' },
];

const selectedLanguage = computed(() =>
    languageOptions.find(option => option.value === settings.value.language) ?? languageOptions[0]
);

function toggleLanguageDropdown() {
    languageDropdownOpen.value = !languageDropdownOpen.value;
}

async function chooseLanguage(language: Language) {
    languageDropdownOpen.value = false;
    await setLanguage(language);
}

function onDocMouseDown(e: MouseEvent) {
    const target = e.target as Node;
    const langRoot = languageDropdownRoot.value;
    if (languageDropdownOpen.value && (!langRoot || !langRoot.contains(target))) {
        languageDropdownOpen.value = false;
    }
}
function onDocKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && languageDropdownOpen.value) languageDropdownOpen.value = false;
}

onMounted(() => {
    document.addEventListener('mousedown', onDocMouseDown);
    document.addEventListener('keydown', onDocKey);
});
onUnmounted(() => {
    document.removeEventListener('mousedown', onDocMouseDown);
    document.removeEventListener('keydown', onDocKey);
});

watch(() => settings.value.language, () => {
    languageDropdownOpen.value = false;
});
</script>

<template>
    <SettingsShell title="Общее" description="Язык интерфейса и интеграции с другими приложениями">
        <div class="settingsBlock">
            <div v-for="item in generalItems" :key="item.key" class="toggleRow" :class="{ phoneHiddenRow: item.key === 'discord_rpc_enabled' }">
                <SettingToggle
                    :item="item"
                    :checked="settings[item.key]"
                    @toggle="toggle(item.key)"
                />
                <button
                    v-if="item.key === 'dev_mode' && devRestartPending"
                    type="button"
                    class="restartBtn"
                    @click="restartApp"
                >{{ t('dev.restart') }}</button>
            </div>

            <div v-if="appVersion" class="settingElem languageSetting">
                <span class="languageText">
                    <span>{{ t('updater.version', { version: appVersion }) }}</span>
                    <small v-if="updateStatusText">{{ updateStatusText }}</small>
                </span>
                <button
                    v-if="updater.supported.value"
                    type="button"
                    class="restartBtn"
                    :disabled="updater.status.value === 'checking' || updater.status.value === 'downloading'"
                    @click="onUpdateClick"
                >{{ updater.availableVersion.value ? t('updater.install') : t('updater.check') }}</button>
            </div>

            <div class="settingElem languageSetting">
                <span class="languageText">
                    <span>{{ t('settings.language') }}</span>
                    <small>{{ t('settings.languageDescription') }}</small>
                </span>
                <div class="languageDropdown" ref="languageDropdownRoot">
                    <button
                        type="button"
                        class="languageTrigger"
                        :class="{ open: languageDropdownOpen }"
                        @click="toggleLanguageDropdown"
                        :aria-expanded="languageDropdownOpen"
                        aria-haspopup="listbox"
                    >
                        <span class="languageCode">{{ selectedLanguage.short }}</span>
                        <span class="languageCurrent">{{ t(selectedLanguage.labelKey) }}</span>
                        <svg class="languageChevron" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                            <path d="m6 9 6 6 6-6" />
                        </svg>
                    </button>
                    <transition name="languageMenu">
                        <div v-if="languageDropdownOpen" class="languageMenu" role="listbox">
                            <button
                                v-for="option in languageOptions"
                                :key="option.value"
                                type="button"
                                class="languageOption"
                                :class="{ active: settings.language === option.value }"
                                role="option"
                                :aria-selected="settings.language === option.value"
                                @click="chooseLanguage(option.value)"
                            >
                                <span class="languageCode">{{ option.short }}</span>
                                <span>{{ t(option.labelKey) }}</span>
                            </button>
                        </div>
                    </transition>
                </div>
            </div>
        </div>
    </SettingsShell>
</template>

<style scoped>
.settingsBlock {
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
    align-items: center;
    width: 100%;
    gap: 16px;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    font-weight: 600;
    font-size: 16px;
}
.toggleRow {
    display: flex;
    align-items: center;
    gap: 12px;
}
.restartBtn {
    flex-shrink: 0;
    height: 32px;
    padding: 0 15px;
    border-radius: 999px;
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.16);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 85%, 1);
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
    transition: background-color .18s ease, color .18s ease;
}
.restartBtn:disabled {
    opacity: 0.5;
    cursor: default;
}
.restartBtn:hover:not(:disabled) {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.28);
    color: #fff;
}
.languageSetting {
    align-items: center;
    justify-content: space-between;
    gap: 18px;
}
.languageText {
    color: #ffffff;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    user-select: none;
}
.languageText small {
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.42);
    font-size: 13px;
    font-weight: 500;
}
.languageDropdown {
    position: relative;
    min-width: 168px;
}
.languageTrigger {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 8px 10px;
    border-radius: 36px;
    background-color: hsla(var(--accent-h), min(calc(var(--accent-s) * 1.25), 50%), 16%, 1);
    color: #ffffff;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color .2s ease, box-shadow .2s ease;
}
.languageTrigger:hover,
.languageTrigger.open {
    background-color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
}
.languageCode {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 32px;
    height: 22px;
    border-radius: 16px;
    background-color: hsla(0, 0%, 100%, 0.11);
    color: hsla(0, 0%, 100%, 0.86);
    font-size: 12px;
    font-weight: 800;
}
.languageCurrent {
    flex: 1;
    text-align: left;
}
.languageChevron {
    flex-shrink: 0;
    color: hsla(0, 0%, 100%, 0.48);
    transition: transform .18s ease;
}
.languageTrigger.open .languageChevron {
    transform: rotate(180deg);
}
.languageMenu {
    display: flex;
    flex-direction: column;
    gap: 3px;
    position: absolute;
    top: calc(100% + 7px);
    right: 0;
    z-index: 20;
    width: 100%;
    padding: 5px;
    border-radius: 24px;
    background-color: hsla(var(--accent-h), min(calc(var(--accent-s) * 1.25), 50%), 12%, 0.98);
    box-shadow: 0 14px 32px hsla(0, 0%, 0%, 0.34), inset 0 0 0 1px hsla(0, 0%, 100%, 0.08);
    backdrop-filter: blur(14px);
}
.languageOption {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 8px 8px 9px;
    border-radius: 20px;
    background-color: transparent;
    color: hsla(0, 0%, 100%, 0.68);
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color .18s ease, color .18s ease;
}
.languageOption:hover,
.languageOption.active {
    background-color: hsla(0, 0%, 100%, 0.09);
    color: #ffffff;
}
.languageMenu-enter-active,
.languageMenu-leave-active {
    transition: opacity .16s ease, transform .16s ease;
    transform-origin: top right;
}
.languageMenu-enter-from,
.languageMenu-leave-to {
    opacity: 0;
    transform: translateY(-4px) scale(0.98);
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .settingsBlock {
        max-width: none;
    }
    .settingElem {
        min-height: 46px;
        gap: 12px;
        font-size: 15px;
    }
    .languageSetting {
        flex-wrap: wrap;
    }
    .phoneHiddenRow {
        display: none;
    }
    .languageDropdown {
        width: 100%;
    }
    .languageMenu {
        position: static;
        margin-top: 7px;
    }
}
</style>
