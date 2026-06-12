<script setup lang="ts">
import { onMounted, computed, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useSettings, ARTIST_SEPARATORS_DEFAULT } from '../../composables/useSettings';
import { usePlayer } from '../../composables/usePlayer';
import { usePlaylists } from '../../composables/usePlaylists';
import SettingsShell from '../../components/SettingsShell.vue';
import DeleteIcon from '../../components/Icons/DeleteIcon.vue';
import FolderIcon from '../../components/Icons/FolderIcon.vue';
import RefreshIcon from '../../components/Icons/RefreshIcon.vue';
import PlusIcon from '../../components/Icons/PlusIcon.vue';
import ConfirmModal from '../../components/ConfirmModal.vue';

const { settings, load, save } = useSettings();
const { rebuildArtists } = usePlaylists();
const {
    folders, isScanning, removingFolder, scanProgress,
    addFolder, removeFolder, rescanLibrary, folderDisplayName,
    refreshLibrary, folderAddError,
} = usePlayer();

const sortedFolders = computed(() =>
    [...folders.value].sort((a, b) => b.track_count - a.track_count)
);

onMounted(load);

// ── Сброс редактирования треков ────────────────────────────────────────────
const showResetEditsConfirm = ref(false);
const resettingEdits = ref(false);

function openResetEditsConfirm() {
    if (resettingEdits.value || isScanning.value) return;
    showResetEditsConfirm.value = true;
}

async function confirmResetEdits() {
    showResetEditsConfirm.value = false;
    if (resettingEdits.value) return;
    resettingEdits.value = true;
    try {
        await invoke<number>('track_clear_all_overrides');
        // После сброса плейлисты исполнителей всё ещё содержат переименованные
        // имена — пересобираем их по оригинальному tracks.artist.
        await rebuildArtists();
        // Перезагружаем библиотеку — get_all_tracks теперь отдаст оригиналы.
        await refreshLibrary();
    } catch (e) {
        console.error('Не удалось сбросить редактирование:', e);
    } finally {
        resettingEdits.value = false;
    }
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
    <SettingsShell title="Библиотека" description="Папки с музыкой и обработка имён исполнителей">
        <div class="settingBlock">
            <div class="settingTitle">Разделители исполнителей</div>
            <div class="settingDesc">Строки, по которым имена исполнителей разбиваются на несколько (например «; » или «, »)</div>
            <div class="separatorsBlock">
                <div class="separatorsList">
                    <div
                        v-for="sep in settings.artist_separators"
                        :key="sep"
                        class="separatorChip"
                        v-tooltip="JSON.stringify(sep)"
                    >
                        <span class="separatorText">{{ visibleSeparator(sep) }}</span>
                        <button class="separatorRemove" @click="removeSeparator(sep)" v-tooltip="'Удалить'">
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
                    <button
                        class="resetEditsBtn"
                        @click="openResetEditsConfirm"
                        :disabled="isScanning || resettingEdits"
                        v-tooltip="'Вернуть всем трекам оригинальные название, исполнителя и обложку'"
                    >
                        <DeleteIcon width="16" height="16" />
                        {{ resettingEdits ? 'Сброс…' : 'Сбросить редактирование' }}
                    </button>
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

                <div v-if="folderAddError" class="folderAddError">
                    {{ folderAddError }}
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

        <ConfirmModal
            :open="showResetEditsConfirm"
            title="Сбросить редактирование треков?"
            message="Все пользовательские правки названий, исполнителей и обложек будут удалены. Треки вернутся к оригинальным значениям из файлов. Это действие нельзя отменить."
            confirmText="Сбросить"
            cancelText="Отмена"
            @confirm="confirmResetEdits"
            @cancel="showResetEditsConfirm = false"
        />
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
.rescanFolders:disabled,
.resetEditsBtn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
}
.resetEditsBtn {
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
}
.resetEditsBtn:hover:not(:disabled) {
    background-color: hsla(0, 70%, 50%, 0.12);
    color: hsl(0, 70%, 80%);
}
.resetEditsBtn svg {
    flex-shrink: 0;
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
.folderAddError {
    padding: 11px 14px;
    border-radius: 10px;
    background-color: hsla(0, 70%, 50%, 0.12);
    color: hsl(0, 70%, 82%);
    font-size: 13px;
    font-weight: 600;
    line-height: 1.35;
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

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .settingBlock {
        margin-bottom: 30px;
    }
    .separatorsBlock,
    .foldersBlock {
        max-width: none;
    }
    .foldersHat,
    .separatorAddRow {
        flex-wrap: wrap;
    }
    .separatorAddRow,
    .separatorInput,
    .separatorAddBtn {
        width: 100%;
    }
    .separatorChip {
        min-height: 34px;
    }
    .separatorRemove {
        width: 28px;
        height: 28px;
    }
    .foldersHat {
        align-items: stretch;
        gap: 8px;
    }
    .addFolder,
    .rescanFolders,
    .resetEditsBtn,
    .separatorAddBtn {
        min-height: 44px;
        justify-content: center;
        border-radius: 12px;
    }
    .addFolder,
    .rescanFolders {
        flex: 1 1 calc(50% - 4px);
    }
    .resetEditsBtn {
        flex: 1 1 100%;
    }
    .folderElem {
        align-items: flex-start;
        gap: 10px;
        padding: 10px;
        border-radius: 12px;
    }
    .folderElem .icon {
        padding: 4px 0 0;
        flex-shrink: 0;
    }
    .folderInfo {
        flex: 1;
        min-width: 0;
    }
    .folderPath {
        font-size: 13px;
        line-height: 1.35;
        overflow-wrap: anywhere;
    }
    .removeBtn {
        width: 40px;
        height: 40px;
        border-radius: 12px;
        padding: 8px;
    }
    .scanProgress,
    .folderAddError {
        border-radius: 12px;
    }
}
</style>
