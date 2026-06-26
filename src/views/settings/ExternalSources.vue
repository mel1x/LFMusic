<script setup lang="ts">
import { onMounted, computed, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useSettings, type SettingKey } from '../../composables/useSettings';
import { useDownloads } from '../../composables/useDownloads';
import { useI18n } from '../../composables/useI18n';
import SettingsShell from '../../components/SettingsShell.vue';
import SettingToggle from '../../components/SettingToggle.vue';
import ConfirmModal from '../../components/ConfirmModal.vue';
import LoadingSpinner from '../../components/LoadingSpinner.vue';

const { t } = useI18n();
const { settings, settingItems, load, save } = useSettings();
const {
    toolsInfo,
    toolsProgress,
    toolsLoaded,
    refreshToolsInfo,
    checkToolUpdates,
    installAll,
    installTool,
    deleteTool,
    denoMissing,
    openYoutubeLogin,
    validateCookies,
    logoutYoutube,
    cookiesAuthorized,
    cookiesChecking,
    cookiesValid,
    cookiesLoginOpen,
    cookiesMessage,
} = useDownloads();

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

// ── Компоненты для скачивания (yt-dlp + ffmpeg + Deno) — единый блок ──────────
// Ставятся/обновляются/удаляются вместе: одна установка тянет все три, поэтому в
// настройках — один блок с действиями «Установить всё / Обновить / Удалить».
const busy = ref(false);                // идёт установка/обновление/удаление блока
const depError = ref<string | null>(null);

// Прогресс установки приходит событиями tools_progress под ключами ytdlp/ffmpeg.
const installProgress = computed(() => toolsProgress.value.ytdlp || toolsProgress.value.ffmpeg);
const depBusy = computed(() => busy.value || !!installProgress.value);

const depAllInstalled = computed(() =>
    toolsInfo.value.ytdlp.installed && toolsInfo.value.ffmpeg.installed && !denoMissing.value,
);
const depUpdateAvailable = computed(() =>
    toolsInfo.value.ytdlp.update_available || toolsInfo.value.ffmpeg.update_available,
);
const depDeletable = computed(() =>
    toolsInfo.value.ytdlp.managed || toolsInfo.value.ffmpeg.managed || toolsInfo.value.deno.managed,
);

type DepStatusKind = 'busy' | 'missing' | 'update' | 'ok';
const depStatusKind = computed<DepStatusKind>(() => {
    if (depBusy.value) return 'busy';
    if (!depAllInstalled.value) return 'missing';
    if (depUpdateAvailable.value) return 'update';
    return 'ok';
});

// Короткая подпись общего состояния (для статус-метки справа в шапке блока).
const depStateLabel = computed(() => {
    switch (depStatusKind.value) {
        case 'busy': return t('downloads.comp.loading');
        case 'missing': return t('downloads.deps.stateMissing');
        case 'update': return t('downloads.deps.stateUpdate');
        default: return t('downloads.deps.stateReady');
    }
});

// Три компонента как статус-чипы — пользователю видно, чего именно не хватает.
const depComponents = computed(() => [
    { name: 'yt-dlp', on: toolsInfo.value.ytdlp.installed, version: toolsInfo.value.ytdlp.version },
    { name: 'ffmpeg', on: toolsInfo.value.ffmpeg.installed, version: toolsInfo.value.ffmpeg.version },
    { name: 'Deno', on: toolsInfo.value.deno.installed, version: toolsInfo.value.deno.version },
]);

// Подробная подпись (прогресс установки / версия / ошибка) — под чипами.
const depDetail = computed(() => {
    if (depError.value) return depError.value;
    if (installProgress.value) return installProgress.value;
    if (busy.value) return t('downloads.comp.loading');
    if (!depAllInstalled.value) return t('downloads.deps.desc');
    const v = toolsInfo.value.ytdlp.version;
    return v ? t('downloads.comp.statusVersion', { version: v }) : t('downloads.comp.statusInstalled');
});

// Установлено ли хоть что-то (для выбора подписи кнопки и показа «Проверить
// обновления» / «Удалить» — их нет смысла показывать на пустой установке).
const depAnyInstalled = computed(() =>
    toolsInfo.value.ytdlp.installed || toolsInfo.value.ffmpeg.installed || toolsInfo.value.deno.installed,
);
// Часть уже стоит → «Установить необходимое» (доустановить недостающее); ничего
// нет → «Установить всё».
const installLabel = computed(() =>
    depAnyInstalled.value ? t('downloads.comp.btnInstallNeeded') : t('downloads.comp.btnInstallAll'),
);

// Длинные версии (например, rolling-сборка ffmpeg) ломают верстку чипа —
// обрезаем, полную показываем в тултипе.
const VER_MAX = 16;
function shortVer(v: string): string {
    return v.length > VER_MAX ? v.slice(0, VER_MAX) + '…' : v;
}

// Ручная проверка обновлений (сеть). Отдельно от установки.
const checkingUpdates = ref(false);
async function onCheckUpdates() {
    if (depBusy.value || checkingUpdates.value) return;
    checkingUpdates.value = true;
    depError.value = null;
    try {
        await checkToolUpdates(true);
    } catch (e) {
        depError.value = String(e);
    } finally {
        checkingUpdates.value = false;
    }
}

// ── Подтверждение скачивания исполняемых файлов (установка/обновление) ────────
// Одна модалка обслуживает оба действия — режим выбирает заголовок, текст,
// подпись кнопки и какой обработчик запустить по подтверждению.
const confirmOpen = ref(false);
const confirmMode = ref<'install' | 'update'>('install');
const confirmTitle = computed(() =>
    confirmMode.value === 'update' ? t('downloads.comp.confirmUpdateTitle') : t('downloads.comp.confirmTitle'),
);
const confirmMessage = computed(() =>
    confirmMode.value === 'update' ? t('downloads.comp.confirmUpdateText') : t('downloads.comp.confirmAllText'),
);
const confirmOkLabel = computed(() =>
    confirmMode.value === 'update' ? t('downloads.comp.btnUpdate') : t('downloads.comp.confirmOk'),
);
function requestInstallAll() {
    if (depBusy.value) return;
    confirmMode.value = 'install';
    confirmOpen.value = true;
}
function requestUpdate() {
    if (depBusy.value || !depUpdateAvailable.value) return;
    confirmMode.value = 'update';
    confirmOpen.value = true;
}
function confirmInstall() {
    confirmOpen.value = false;
    if (confirmMode.value === 'update') void runUpdate();
    else void runInstallAll();
}

async function runInstallAll() {
    busy.value = true;
    depError.value = null;
    try {
        await installAll();
        await checkToolUpdates(true);
    } catch (e) {
        depError.value = String(e);
    } finally {
        busy.value = false;
    }
}

// Обновление = принудительная переустановка тех утилит, у которых есть новая
// версия. tools_install_all ставит только отсутствующее (не перекачивает уже
// установленное), поэтому для обновления он не подходит — дёргаем точечную
// (пере)установку каждой устаревшей утилиты через tools_install.
async function runUpdate() {
    busy.value = true;
    depError.value = null;
    try {
        if (toolsInfo.value.ytdlp.update_available) await installTool('ytdlp');
        if (toolsInfo.value.ffmpeg.update_available) await installTool('ffmpeg');
        await checkToolUpdates(true);
    } catch (e) {
        depError.value = String(e);
    } finally {
        busy.value = false;
    }
}

async function onDeleteAll() {
    if (depBusy.value || !depDeletable.value) return;
    busy.value = true;
    depError.value = null;
    try {
        // yt-dlp удаляется вместе с Deno; ffmpeg — отдельной утилитой.
        await deleteTool('ytdlp');
        await deleteTool('ffmpeg');
    } catch (e) {
        depError.value = String(e);
    } finally {
        busy.value = false;
    }
}

// ── Авторизация YouTube (необязательна; нужна только при бот-стене) ───────────
// Состояние входа помнится с диска (бэкенд: snapshot/refresh_tools_status),
// поэтому успешный вход показывается сразу и не перепроверяется на каждом старте.
type CookieStatusKind = 'none' | 'login' | 'checking' | 'valid' | 'invalid';
const cookieStatusKind = computed<CookieStatusKind>(() => {
    if (cookiesLoginOpen.value) return 'login';
    if (!cookiesAuthorized.value) return 'none';
    if (cookiesChecking.value) return 'checking';
    if (cookiesValid.value) return 'valid';
    return 'invalid';
});

const cookieStatusText = computed(() => {
    switch (cookieStatusKind.value) {
        case 'login': return t('downloads.cookies.statusLogin');
        case 'none': return t('downloads.cookies.statusNone');
        case 'checking': return t('downloads.cookies.statusChecking');
        case 'valid': return t('downloads.cookies.statusValid');
        case 'invalid': return cookiesMessage.value || t('downloads.cookies.statusInvalid');
    }
    return '';
});

function authorize() {
    if (cookiesLoginOpen.value) return;
    void openYoutubeLogin();
}
function recheck() {
    if (cookiesChecking.value || cookiesLoginOpen.value || !cookiesAuthorized.value) return;
    void validateCookies();
}
function signOut() {
    if (cookiesLoginOpen.value) return;
    void logoutYoutube();
}

onMounted(async () => {
    await load();
    try {
        await refreshToolsInfo();
        await checkToolUpdates();
    } catch { /* без сети просто не покажем обновления */ }
});
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

        <!-- Компоненты для скачивания: один блок (yt-dlp + ffmpeg + Deno) -->
        <h2 class="sectionTitle">{{ t('downloads.comp.sectionTitle') }}</h2>
        <div v-if="!toolsLoaded" class="toolLoading">
            <LoadingSpinner />
        </div>
        <div v-else class="depPanel" :class="depStatusKind">
            <div class="depTop">
                <div class="depHead">
                    <h3 class="depTitle">{{ t('downloads.deps.title') }}</h3>
                    <p class="depDesc" v-truncate>{{ depDetail }}</p>
                </div>
                <div class="depState" :class="depStatusKind">
                    <span v-if="depStatusKind === 'busy'" class="miniSpinner"></span>
                    <span v-else class="stateDot"></span>
                    <span>{{ depStateLabel }}</span>
                </div>
            </div>

            <div class="depChips">
                <span v-for="c in depComponents" :key="c.name" class="depChip" :class="{ on: c.on }">
                    <span class="chipDot"></span>
                    <span class="chipName">{{ c.name }}</span>
                    <span
                        v-if="c.on && c.version"
                        class="chipVer"
                        v-tooltip="c.version.length > VER_MAX ? c.version : ''"
                    >{{ shortVer(c.version) }}</span>
                </span>
            </div>

            <div v-if="depBusy" class="depBar"><span class="depBarFill"></span></div>

            <div class="depActions">
                <button
                    v-if="!depAllInstalled"
                    class="actBtn primary"
                    :disabled="depBusy"
                    @click="requestInstallAll"
                >{{ installLabel }}</button>
                <button
                    v-else
                    class="actBtn primary"
                    :disabled="depBusy || !depUpdateAvailable"
                    @click="requestUpdate"
                >{{ t('downloads.comp.btnUpdate') }}</button>
                <button
                    v-if="depAnyInstalled"
                    class="actBtn ghost checkBtn"
                    :class="{ checking: checkingUpdates }"
                    :disabled="depBusy || checkingUpdates"
                    @click="onCheckUpdates"
                >
                    <span class="checkLabels">
                        <span class="checkLabel">{{ t('downloads.comp.btnCheckUpdates') }}</span>
                        <span class="checkLabel busy">
                            <span class="miniSpinner"></span>
                            {{ t('downloads.comp.checkingUpdates') }}
                        </span>
                    </span>
                </button>
                <button
                    class="actBtn ghost"
                    :disabled="depBusy || !depDeletable"
                    @click="onDeleteAll"
                    v-tooltip="t('downloads.comp.deleteTooltip')"
                >{{ t('downloads.comp.btnDelete') }}</button>
            </div>
        </div>

        <!-- Авторизация YouTube (необязательна) -->
        <h2 class="sectionTitle">{{ t('downloads.cookies.sectionTitle') }}</h2>
        <div class="ytAuth" :class="cookieStatusKind">
            <div class="ytAuthMain">
                <div class="ytAuthStatusRow">
                    <span v-if="cookieStatusKind === 'checking' || cookieStatusKind === 'login'" class="miniSpinner"></span>
                    <span v-else class="stateDot"></span>
                    <span class="ytAuthState">{{ cookieStatusText }}</span>
                </div>
                <p class="ytAuthDesc">{{ t('downloads.cookies.desc') }}</p>
            </div>
            <div class="ytAuthActions">
                <button
                    v-if="cookieStatusKind === 'valid'"
                    class="actBtn ghost"
                    :disabled="cookiesChecking || cookiesLoginOpen"
                    @click="recheck"
                >{{ t('downloads.cookies.recheck') }}</button>
                <button
                    v-else
                    class="actBtn primary"
                    :disabled="cookiesLoginOpen"
                    @click="authorize"
                >{{ cookiesAuthorized ? t('downloads.cookies.reauthorize') : t('downloads.cookies.authorize') }}</button>
                <button
                    v-if="cookiesAuthorized"
                    class="actBtn ghost danger"
                    :disabled="cookiesLoginOpen"
                    @click="signOut"
                >{{ t('downloads.cookies.signOut') }}</button>
            </div>
        </div>
        <p class="hint">{{ t('downloads.cookies.hint') }}</p>

        <ConfirmModal
            :open="confirmOpen"
            :title="confirmTitle"
            :message="confirmMessage"
            :confirmText="confirmOkLabel"
            :cancelText="t('downloads.comp.confirmCancel')"
            @confirm="confirmInstall"
            @cancel="confirmOpen = false"
        />
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

/* ── Необходимые компоненты ──────────────────────────────────────── */
.sectionTitle {
    margin-top: 40px;
    margin-bottom: 16px;
    max-width: 800px;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 19px;
    color: hsla(0, 0%, 100%, 0.92);
}
.toolLoading {
    display: flex;
    align-items: center;
    justify-content: center;
    max-width: 800px;
    width: 100%;
    min-height: 150px;
}
.miniSpinner {
    flex-shrink: 0;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    border: 2px solid hsla(0, 0%, 100%, 0.15);
    border-top-color: hsla(var(--accent-h), var(--accent-s), 70%, 0.95);
    animation: spin 0.8s linear infinite;
}
@keyframes spin {
    to { transform: rotate(360deg); }
}

/* ── Блок зависимостей: чипы компонентов + действия ──────────────── */
.depPanel {
    max-width: 800px;
    width: 100%;
    padding: 18px 20px 16px;
    border-radius: 16px;
    background: hsla(0, 0%, 100%, 0.028);
    border: 1px solid hsla(0, 0%, 100%, 0.08);
    transition: border-color .24s ease;
}
/* Чего-то не хватает / есть обновление — мягкий акцентный верх зовёт к
   действию (без больших ярких поверхностей: тонкий градиент сверху). */
.depPanel.missing,
.depPanel.update {
    background:
        linear-gradient(180deg, hsla(var(--accent-h), var(--accent-s), 50%, 0.07), hsla(0, 0%, 100%, 0.028) 70px),
        hsla(0, 0%, 100%, 0.028);
    border-color: hsla(var(--accent-h), var(--accent-s), 55%, 0.18);
}
.depTop {
    display: flex;
    flex-direction: row;
    align-items: flex-start;
    gap: 16px;
}
.depHead { flex: 1; min-width: 0; }
.depTitle {
    margin: 0;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 15px;
    color: hsla(0, 0%, 100%, 0.95);
}
.depDesc {
    margin: 4px 0 0;
    font-size: 12.5px;
    font-weight: 500;
    line-height: 1.4;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.depState {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 5px 11px 5px 9px;
    border-radius: 999px;
    background: hsla(0, 0%, 100%, 0.05);
    font-size: 11.5px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.6);
    white-space: nowrap;
}
.stateDot {
    flex-shrink: 0;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.3);
}
.depState.ok { color: hsla(145, 55%, 70%, 0.95); background: hsla(145, 45%, 45%, 0.12); }
.depState.ok .stateDot { background-color: hsla(145, 60%, 55%, 0.95); }
.depState.update { color: hsla(38, 90%, 70%, 1); background: hsla(38, 85%, 50%, 0.14); }
.depState.update .stateDot { background-color: hsla(38, 90%, 58%, 0.95); }
.depState.missing { color: hsla(0, 0%, 100%, 0.5); }
.depState.missing .stateDot { background-color: hsla(0, 0%, 100%, 0.28); }

.depChips {
    display: flex;
    flex-flow: row wrap;
    gap: 8px;
    margin-top: 14px;
}
.depChip {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 6px 11px;
    border-radius: 9px;
    background: hsla(0, 0%, 100%, 0.035);
    border: 1px solid hsla(0, 0%, 100%, 0.06);
}
.chipDot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.25);
    transition: background-color .24s ease;
}
.depChip.on .chipDot { background-color: hsla(145, 60%, 55%, 0.95); }
.chipName {
    font-family: "JetBrains Mono", "SF Mono", ui-monospace, "Cascadia Code", Consolas, monospace;
    font-size: 12.5px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: hsla(0, 0%, 100%, 0.45);
}
.depChip.on .chipName { color: hsla(0, 0%, 100%, 0.9); }
.chipVer {
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.35);
}

/* Тонкая индикаторная полоса во время установки. */
.depBar {
    position: relative;
    height: 2px;
    margin-top: 14px;
    border-radius: 999px;
    overflow: hidden;
    background: hsla(0, 0%, 100%, 0.08);
}
.depBarFill {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 38%;
    border-radius: inherit;
    background: hsla(var(--accent-h), min(calc(var(--accent-s) + 14%), 100%), 66%, 0.95);
    animation: depSlide 1.2s ease-in-out infinite;
}
@keyframes depSlide {
    0% { transform: translateX(-110%); }
    100% { transform: translateX(330%); }
}

.depActions {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    margin-top: 16px;
}

/* ── Кнопки действий (общие для обоих блоков) ────────────────────── */
.actBtn {
    cursor: pointer;
    padding: 9px 18px;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 700;
    transition: background-color .2s ease, color .2s ease, opacity .2s ease, transform .2s ease;
}
.actBtn:disabled {
    cursor: default;
    opacity: 0.35;
}
.actBtn.primary {
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
    background-color: hsla(0, 0%, 100%, 0.95);
}
.actBtn.primary:not(:disabled):hover {
    background-color: #fff;
    transform: translateY(-1px);
}
.actBtn.ghost {
    color: hsla(0, 0%, 100%, 0.6);
    background-color: hsla(0, 0%, 100%, 0.05);
}
.actBtn.ghost:not(:disabled):hover {
    color: hsla(0, 0%, 100%, 0.9);
    background-color: hsla(0, 0%, 100%, 0.1);
}
.actBtn.ghost.danger { color: hsla(2, 60%, 72%, 0.85); }
.actBtn.ghost.danger:not(:disabled):hover {
    color: hsla(2, 72%, 80%, 1);
    background-color: hsla(2, 60%, 50%, 0.16);
}

/* Кнопка «Проверить обновления»: обе подписи лежат в одной grid-ячейке, поэтому
   её ширина фиксируется по самой длинной и не дёргается при переключении на
   «Проверка…» со спиннером (вторая подпись остаётся в потоке, лишь скрывается). */
.checkBtn .checkLabels {
    display: grid;
    align-items: center;
    justify-items: center;
}
.checkBtn .checkLabel {
    grid-area: 1 / 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    white-space: nowrap;
}
.checkBtn .checkLabel.busy { visibility: hidden; }
.checkBtn.checking .checkLabel:not(.busy) { visibility: hidden; }
.checkBtn.checking .checkLabel.busy { visibility: visible; }

/* ── Авторизация YouTube — тише блока зависимостей (вход необязателен) ─ */
.ytAuth {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 18px;
    max-width: 800px;
    width: 100%;
    padding: 15px 18px;
    border-radius: 14px;
    background: hsla(0, 0%, 100%, 0.022);
    border: 1px solid hsla(0, 0%, 100%, 0.06);
}
.ytAuthMain { flex: 1; min-width: 0; }
.ytAuthStatusRow {
    display: flex;
    align-items: center;
    gap: 8px;
}
.ytAuthState {
    min-width: 0;
    font-size: 13px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.85);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.ytAuth.valid .stateDot { background-color: hsla(145, 60%, 55%, 0.95); }
.ytAuth.valid .ytAuthState { color: hsla(145, 45%, 75%, 0.95); }
.ytAuth.invalid .ytAuthState { color: hsla(2, 60%, 74%, 0.92); }
.ytAuth.invalid .stateDot { background-color: hsla(2, 70%, 60%, 0.95); }
.ytAuth.none .stateDot { background-color: hsla(0, 0%, 100%, 0.28); }
.ytAuth.login .ytAuthState { color: hsla(var(--accent-h), calc(var(--accent-s) * 0.7), 78%, 0.9); }
.ytAuthDesc {
    margin: 5px 0 0;
    font-size: 12px;
    font-weight: 500;
    line-height: 1.45;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.45);
}
.ytAuthActions {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 8px;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .settingsBlock,
    .hint,
    .sectionTitle,
    .depPanel,
    .ytAuth {
        max-width: none;
    }
    .ytAuth { flex-wrap: wrap; }
    .depActions,
    .ytAuthActions { flex-wrap: wrap; }
    .ytAuthActions { flex: 1 0 100%; justify-content: flex-end; }
    .actBtn { flex: 1; padding: 12px 18px; }
}
</style>
