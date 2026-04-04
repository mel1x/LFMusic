<script setup lang="ts">
import { onMounted } from 'vue';
import DeleteIcon from '../components/Icons/DeleteIcon.vue';
import FolderIcon from '../components/Icons/FolderIcon.vue';
import { useSettings } from '../composables/useSettings';
import { useFolders } from '../composables/useFolders';
import RefreshIcon from '../components/Icons/RefreshIcon.vue';
import PlusIcon from '../components/Icons/PlusIcon.vue';

const { settings, settingItems, load, save } = useSettings();
const {
    folders, isScanning, removingFolder, scanProgress,
    init, addFolder, removeFolder, rescanLibrary, folderDisplayName,
} = useFolders();

onMounted(async () => {
    await load();
    await init();
});

async function toggle(key: keyof typeof settings.value) {
    settings.value[key] = !settings.value[key];
    await save();
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
                    v-for="folder in folders"
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
    background-color: #101010;
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
    color: hsla(0, 0%, 44.1%, 1);
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
    color: hsla(0, 0%, 44.1%, 1);
    font-weight: 600;
    font-size: 16px;
}
.settingElem span {
    user-select: none;
    transition: .24s ease-in-out;

    &:hover {
        color: hsla(0, 0%, 60.1%, 1);
    }
}
.settingCheckbox {
    appearance: none;
    width: 32px;
    height: 19.2px;
    background: #1f1f1f;
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
    background: hsla(0, 0%, 10%, 1);
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
    background-color: #1f1f1f;
}
.folderElem .icon {
    padding: 0px 6px 0px 8px;
    color: hsla(0, 0%, 44.1%, 1);

    transition: .24s ease-in-out;

    &:hover {
        color: hsla(0, 0%, 60.1%, 1);
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
    color: hsla(0, 0%, 44.1%, 1);

    transition: .24s ease-in-out;

    &:hover {
        color: hsla(0, 0%, 60.1%, 1);
    }
}
.removeBtn {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(0, 0%, 44.1%, 1);
    border-radius: 4px;
    padding: 4px;
    background-color: hsla(0, 0%, 100%, 0.08);
    margin-left: auto;
    transition: .2s;

    &:hover {
        color: #ff6467;
        background-color: #2b1617;
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
    color: hsla(0, 0%, 44.1%, 1);
    font-weight: 500;
    font-size: 15px;
    cursor: pointer;
    transition: .2s;

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
    color: hsla(0, 0%, 44.1%, 1);
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
    background-color: #1a1a1a;
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
    color: hsla(0, 0%, 44.1%, 1);
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
</style>
