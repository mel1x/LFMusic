<script setup lang="ts">
import { onMounted, computed } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useSettings, type SettingKey } from '../../composables/useSettings';
import SettingsShell from '../../components/SettingsShell.vue';
import SettingToggle from '../../components/SettingToggle.vue';

const { settings, settingItems, load, save } = useSettings();

const externalKeys: SettingKey[] = ['external_album_order', 'external_supplement_missing', 'external_track_covers', 'external_artist_avatars', 'show_downloads_page'];
const externalItems = computed(() => externalKeys.map(key => settingItems.find(i => i.key === key)!));

// Сетевые источники: их переключение (вкл/выкл) перезапускает фоновый проход
// подготовки. При включении он догружает данные с прогрессом; при выключении —
// останавливает текущий проход (кэш не трогается).
const networkSourceKeys: SettingKey[] = ['external_album_order', 'external_supplement_missing', 'external_track_covers', 'external_artist_avatars'];

async function toggle(key: SettingKey) {
    settings.value[key] = !settings.value[key];
    await save();
    if (networkSourceKeys.includes(key)) {
        invoke('external_prepare').catch(() => { /* не мешаем UI */ });
    }
}

onMounted(load);
</script>

<template>
    <SettingsShell
        title="Внешние источники"
        description="Дополнение локальной библиотеки данными из открытых сервисов"
    >
        <div class="settingsBlock">
            <SettingToggle
                v-for="item in externalItems"
                :key="item.key"
                :item="item"
                :checked="settings[item.key]"
                :phoneHidden="item.key === 'show_downloads_page'"
                @toggle="toggle(item.key)"
            />
        </div>
        <p class="hint">
            Данные берутся из открытых источников. Повторные открытия не обращаются к сети.
        </p>
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
.hint {
    margin-top: 22px;
    max-width: 800px;
    font-weight: 500;
    font-size: 13px;
    line-height: 1.5;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.42);
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .settingsBlock,
    .hint {
        max-width: none;
    }
}
</style>
