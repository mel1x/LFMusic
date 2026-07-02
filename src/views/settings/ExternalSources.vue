<script setup lang="ts">
import { onMounted, computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useSettings, type SettingKey } from '../../composables/useSettings';
import { useDownloads, type ProxyMode } from '../../composables/useDownloads';
import { useI18n } from '../../composables/useI18n';
import { isAndroidRuntime } from '../../composables/usePlatform';
import SettingsShell from '../../components/SettingsShell.vue';
import SettingToggle from '../../components/SettingToggle.vue';
import ConfirmModal from '../../components/ConfirmModal.vue';
import CookieBridgeWizard from '../../components/CookieBridgeWizard.vue';
import FolderIcon from '../../components/Icons/FolderIcon.vue';
import ArrowUpIcon from '../../components/Icons/ArrowUpIcon.vue';

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
    potUpdateAvailable,
    validateCookies,
    clearYoutubeCookies,
    cookiesAuthorized,
    cookiesChecking,
    cookiesValid,
    cookiesMessage,
    downloadsDir,
    setDownloadsDir,
    proxyConfig,
    setProxy,
} = useDownloads();

const isAndroid = isAndroidRuntime();

const externalKeys: SettingKey[] = ['external_album_order', 'external_supplement_missing', 'external_track_covers', 'external_artist_avatars', 'show_downloads_page'];
const externalItems = computed(() => externalKeys.map(key => settingItems.find(i => i.key === key)!));

// Toggling a network source restarts the background prepare pass: enabling
// fetches data with progress, disabling stops the current pass (cache untouched).
const networkSourceKeys: SettingKey[] = ['external_album_order', 'external_supplement_missing', 'external_track_covers', 'external_artist_avatars'];

async function toggle(key: SettingKey) {
    settings.value[key] = !settings.value[key];
    await save();
    if (networkSourceKeys.includes(key)) {
        invoke('external_prepare').catch(() => { /* don't block the UI */ });
    }
}

// Download components (yt-dlp + ffmpeg + Deno) as one block: installed/updated/
// removed together, so settings expose a single "install all / update / delete".
const busy = ref(false);
const depError = ref<string | null>(null);

// Install progress arrives via tools_progress events keyed by ytdlp/ffmpeg.
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

// Short overall-state label for the status pill at the top-right of the block.
const depStateLabel = computed(() => {
    switch (depStatusKind.value) {
        case 'busy': return t('downloads.comp.loading');
        case 'missing': return t('downloads.deps.stateMissing');
        case 'update': return t('downloads.deps.stateUpdate');
        default: return t('downloads.deps.stateReady');
    }
});

// Tooltip on the "update available" pill: which components will actually
// update. Empty (no tooltip) in every other state.
const updateTooltip = computed(() => {
    if (depStatusKind.value !== 'update') return '';
    const names: string[] = [];
    if (toolsInfo.value.ytdlp.update_available) names.push('yt-dlp');
    if (toolsInfo.value.ffmpeg.update_available) names.push('ffmpeg');
    if (!names.length) return '';
    return t('downloads.deps.updateTooltip', { list: names.join(', ') });
});

// The three components as status cells, so the user sees what's missing.
const depComponents = computed(() => [
    { name: 'yt-dlp', on: toolsInfo.value.ytdlp.installed, version: toolsInfo.value.ytdlp.version },
    { name: 'ffmpeg', on: toolsInfo.value.ffmpeg.installed, version: toolsInfo.value.ffmpeg.version },
    { name: 'Deno', on: toolsInfo.value.deno.installed, version: toolsInfo.value.deno.version },
]);

// Detail label under the title (install progress / version / error).
const depDetail = computed(() => {
    if (depError.value) return depError.value;
    if (installProgress.value) return installProgress.value;
    if (busy.value) return t('downloads.comp.loading');
    if (!depAllInstalled.value) return t('downloads.deps.desc');
    const v = toolsInfo.value.ytdlp.version;
    return v ? t('downloads.comp.statusVersion', { version: v }) : t('downloads.comp.statusInstalled');
});

// Whether anything is installed at all (picks the button label and whether to
// show "check updates" / "delete" — pointless on an empty install).
const depAnyInstalled = computed(() =>
    toolsInfo.value.ytdlp.installed || toolsInfo.value.ffmpeg.installed || toolsInfo.value.deno.installed,
);
// Something already installed -> "install missing"; nothing installed -> "install all".
const installLabel = computed(() =>
    depAnyInstalled.value ? t('downloads.comp.btnInstallNeeded') : t('downloads.comp.btnInstallAll'),
);

// Long versions (e.g. ffmpeg rolling builds) break the cell layout — truncate,
// show the full string in the tooltip.
const VER_MAX = 16;
function shortVer(v: string): string {
    return v.length > VER_MAX ? v.slice(0, VER_MAX) + '…' : v;
}

// Manual update check (network). Separate from install.
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

// Confirmation for downloading executables (install/update). One modal serves
// both actions — the mode picks title, text, button label, and confirm handler.
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

// Update = force-reinstall whichever tools have a newer version available.
// tools_install_all only installs what's missing (won't re-fetch an already
// installed tool), so it can't be reused here — call tools_install per outdated tool.
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
        // yt-dlp is removed together with Deno; ffmpeg is a separate tool.
        await deleteTool('ytdlp');
        await deleteTool('ffmpeg');
    } catch (e) {
        depError.value = String(e);
    } finally {
        busy.value = false;
    }
}

// YouTube cookies (optional; only needed behind a bot-wall). Imported via a
// companion browser extension: its popup reads the browser's live cookie jar
// through the privileged chrome.cookies API and posts it to a local bridge
// server this app runs (browser_bridge.rs) — no in-app login window, no
// direct access to the browser's cookie database (both tried before and
// dropped: the former got flagged by YouTube's bot-detection, the latter
// needs the browser closed and fails outright on Chrome/Edge since 2024).
type CookieStatusKind = 'none' | 'checking' | 'valid' | 'invalid';
const cookieStatusKind = computed<CookieStatusKind>(() => {
    if (!cookiesAuthorized.value) return 'none';
    if (cookiesChecking.value) return 'checking';
    if (cookiesValid.value) return 'valid';
    return 'invalid';
});

const cookieStatusText = computed(() => {
    switch (cookieStatusKind.value) {
        case 'none': return t('downloads.cookies.statusNone');
        case 'checking': return t('downloads.cookies.statusChecking');
        case 'valid': return t('downloads.cookies.statusValid');
        case 'invalid': return cookiesMessage.value || t('downloads.cookies.statusInvalid');
    }
    return '';
});

// Opening the extension folder now happens inside CookieBridgeWizard.vue
// (self-contained: it calls useDownloads() itself).
const wizardOpen = ref(false);
const cookiesActionBusy = ref(false);
function recheck() {
    if (cookiesChecking.value || !cookiesAuthorized.value) return;
    void validateCookies();
}
async function clearCookies() {
    if (cookiesActionBusy.value) return;
    cookiesActionBusy.value = true;
    try {
        await clearYoutubeCookies();
    } finally {
        cookiesActionBusy.value = false;
    }
}

// PO-token provider (bgutil-pot): optional, hidden behind "Advanced" since most
// downloads work fine without it — only bypasses YouTube's bot-wall, which can
// trigger even with valid cookies (it's IP/client-attestation based, not
// account-based — see downloads.rs::is_bot_wall). Installed, updated and
// removed independently of the required yt-dlp/ffmpeg/Deno stack.
const advancedOpen = ref(false);
const potInstallBusy = ref(false);
const potError = ref<string | null>(null);
const potProgress = computed(() => toolsProgress.value.pot);
const potActionBusy = computed(() => potInstallBusy.value || !!potProgress.value);

const potStatusClass = computed(() => {
    if (potActionBusy.value) return '';
    return toolsInfo.value.pot.installed ? 'valid' : 'none';
});

const potStatusText = computed(() => {
    if (potError.value) return potError.value;
    if (potProgress.value) return potProgress.value;
    if (potActionBusy.value) return t('downloads.comp.loading');
    if (!toolsInfo.value.pot.installed) return t('downloads.pot.statusMissing');
    if (potUpdateAvailable.value) return t('downloads.pot.statusUpdate');
    const v = toolsInfo.value.pot.version;
    return v ? t('downloads.pot.statusReadyVersion', { version: v }) : t('downloads.pot.statusReady');
});

const potConfirmOpen = ref(false);
function requestInstallPot() {
    if (potActionBusy.value) return;
    potConfirmOpen.value = true;
}
function confirmInstallPot() {
    potConfirmOpen.value = false;
    void runInstallPot();
}
async function runInstallPot() {
    potInstallBusy.value = true;
    potError.value = null;
    try {
        await installTool('pot');
        await checkToolUpdates(true);
    } catch (e) {
        potError.value = String(e);
    } finally {
        potInstallBusy.value = false;
    }
}
async function onDeletePot() {
    if (potActionBusy.value || !toolsInfo.value.pot.managed) return;
    potInstallBusy.value = true;
    potError.value = null;
    try {
        await deleteTool('pot');
    } catch (e) {
        potError.value = String(e);
    } finally {
        potInstallBusy.value = false;
    }
}

// Download folder: path lives in downloadsDir (loaded in useDownloads.init).
// Changed via downloads_set_dir: the backend re-registers the folder in the
// library (unscans the old one, scans the new) and falls back to the default
// folder on any error — in that case the applied path won't match the chosen one.
const folderBusy = ref(false);
const folderFellBack = ref(false);
const folderError = ref<string | null>(null);

// downloadsDir is seeded synchronously from a localStorage cache (see
// useDownloads.ts), so it's already populated by the time this renders —
// no loading shimmer needed.
const folderDisplay = computed(() => (downloadsDir.value || '').replace(/\\/g, '/'));

async function chooseDownloadFolder() {
    if (folderBusy.value) return;
    folderError.value = null;
    folderFellBack.value = false;
    let picked: string | null = null;
    try {
        const res = await open({ directory: true, multiple: false });
        picked = typeof res === 'string' ? res : null;
    } catch (e) {
        folderError.value = String(e);
        return;
    }
    if (!picked) return;
    folderBusy.value = true;
    try {
        const resolved = await setDownloadsDir(picked);
        // The backend couldn't prepare the chosen folder and fell back to the default.
        folderFellBack.value = norm(resolved) !== norm(picked);
    } catch (e) {
        folderError.value = String(e);
    } finally {
        folderBusy.value = false;
    }
}

async function resetDownloadFolder() {
    if (folderBusy.value) return;
    folderError.value = null;
    folderFellBack.value = false;
    folderBusy.value = true;
    try {
        await setDownloadsDir(null);
    } catch (e) {
        folderError.value = String(e);
    } finally {
        folderBusy.value = false;
    }
}

function norm(p: string): string {
    return p.replace(/[\\/]+$/, '').replace(/\\/g, '/').toLowerCase();
}

async function openDownloadFolder() {
    if (!downloadsDir.value) return;
    try {
        await invoke('open_folder_in_explorer', { path: downloadsDir.value });
    } catch (e) {
        console.error('Не удалось открыть папку:', e);
    }
}

// Proxy for yt-dlp: System (default) / None (direct) / Custom (own socks5/http).
// Useful against ISP blocking, e.g. routing through ByeDPI (usually socks5://127.0.0.1:1080).
const proxyModeOptions = computed<{ key: ProxyMode; label: string }[]>(() => [
    { key: 'system', label: t('downloads.proxy.modeSystem') },
    { key: 'none', label: t('downloads.proxy.modeNone') },
    { key: 'custom', label: t('downloads.proxy.modeCustom') },
]);
// Index of the active mode — drives the sliding thumb of the segmented control.
const proxyIndex = computed(() =>
    Math.max(0, proxyModeOptions.value.findIndex(m => m.key === proxyConfig.value.mode)),
);
// Local copy of the URL for input; synced once proxyConfig loads.
const proxyUrlInput = ref(proxyConfig.value.url);
const proxyError = ref<string | null>(null);
watch(() => proxyConfig.value.url, (v) => { proxyUrlInput.value = v; });

const proxyHint = computed(() => {
    switch (proxyConfig.value.mode) {
        case 'none': return t('downloads.proxy.noneHint');
        case 'custom': return t('downloads.proxy.customHint');
        default: return t('downloads.proxy.systemHint');
    }
});

// Proxy schemes yt-dlp understands: socks5/socks5h/socks4/socks4a/http/https.
function isValidProxyUrl(u: string): boolean {
    return /^(socks(5h?|4a?)|https?):\/\/\S+/i.test(u.trim());
}

async function applyProxy(mode: ProxyMode, url: string) {
    proxyError.value = null;
    if (mode === 'custom') {
        const u = url.trim();
        // Empty URL is allowed (backend falls back to system); non-empty gets validated.
        if (u && !isValidProxyUrl(u)) {
            proxyError.value = t('downloads.proxy.invalidUrl');
            return;
        }
    }
    try {
        await setProxy(mode, url);
    } catch (e) {
        proxyError.value = String(e);
    }
}

function selectProxyMode(mode: ProxyMode) {
    if (mode === proxyConfig.value.mode) return;
    void applyProxy(mode, mode === 'custom' ? proxyUrlInput.value : '');
}

function saveCustomUrl() {
    if (proxyConfig.value.mode !== 'custom') return;
    if (proxyUrlInput.value.trim() === proxyConfig.value.url) return;
    void applyProxy('custom', proxyUrlInput.value);
}

onMounted(async () => {
    await load();
    try {
        await refreshToolsInfo();
        await checkToolUpdates();
    } catch { /* no network — just skip showing updates */ }
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

        <!-- Everything below (folder -> components -> login -> proxy -> footnote)
             only shows when the "Downloads" page is enabled — otherwise these
             settings are meaningless. -->
        <Transition name="dlZone">
        <div v-if="settings.show_downloads_page" class="dlZone">
        <!-- Download folder -->
        <template v-if="!isAndroid">
            <h2 class="sectionTitle">{{ t('downloads.folder.sectionTitle') }}</h2>
            <div class="dlFolder">
                <div class="dlFolderRow">
                    <button
                        class="dlFolderTile"
                        type="button"
                        v-tooltip="t('downloads.folder.open')"
                        @click="openDownloadFolder"
                    >
                        <FolderIcon />
                    </button>
                    <div class="dlFolderInfo">
                        <div class="dlFolderPath" v-truncate>{{ folderDisplay }}</div>
                        <div class="dlFolderHint">{{ t('downloads.folder.desc') }}</div>
                    </div>
                    <div class="dlFolderActions">
                        <button
                            class="actBtn primary"
                            :class="{ loading: folderBusy }"
                            :disabled="folderBusy"
                            @click="chooseDownloadFolder"
                        >
                            <span class="btnLabel">{{ t('downloads.folder.choose') }}</span>
                            <span class="miniSpinner btnSpinner" aria-hidden="true"></span>
                        </button>
                        <button
                            class="actBtn ghost"
                            :disabled="folderBusy"
                            @click="resetDownloadFolder"
                            v-tooltip="t('downloads.folder.resetTooltip')"
                        >{{ t('downloads.folder.reset') }}</button>
                    </div>
                </div>
                <Transition name="noteSlide">
                    <p v-if="folderFellBack" class="dlWarn">{{ t('downloads.folder.fallback') }}</p>
                    <p v-else-if="folderError" class="dlWarn">{{ folderError }}</p>
                </Transition>
            </div>
        </template>

        <!-- Download components + cookies + advanced: one unified panel -->
        <h2 class="sectionTitle">{{ t('downloads.comp.sectionTitle') }}</h2>
        <div v-if="!toolsLoaded" class="unifiedPanel skeletonPanel" aria-hidden="true">
            <div class="panelSection">
                <div class="skelRow">
                    <span class="skel" style="width: 210px; height: 15px;"></span>
                    <span class="skel skelPill" style="width: 88px; height: 26px; margin-left: auto;"></span>
                </div>
                <span class="skel" style="width: 55%; height: 11px; margin-top: 11px;"></span>
                <div class="skelRow" style="margin-top: 16px; gap: 6px;">
                    <span class="skel skelPill" style="flex: 1; height: 42px;"></span>
                    <span class="skel skelPill" style="flex: 1; height: 42px;"></span>
                    <span class="skel skelPill" style="flex: 1; height: 42px;"></span>
                </div>
                <div class="skelRow" style="margin-top: 18px;">
                    <span class="skel skelPill" style="width: 168px; height: 35px;"></span>
                    <span class="skel skelPill" style="width: 132px; height: 35px;"></span>
                </div>
            </div>
        </div>
        <div v-else class="unifiedPanel" :class="depStatusKind">
            <div class="panelSection depSection">
                <div class="depTop">
                    <div class="depHead">
                        <h3 class="subTitle">{{ t('downloads.deps.title') }}</h3>
                        <p class="depDesc" v-truncate>{{ depDetail }}</p>
                    </div>
                    <div class="depState" :class="depStatusKind" v-tooltip:bottom="updateTooltip">
                        <span v-if="depStatusKind === 'busy'" class="miniSpinner"></span>
                        <span v-else class="stateDot"></span>
                        <span>{{ depStateLabel }}</span>
                    </div>
                </div>

                <div class="toolRail">
                    <div v-for="c in depComponents" :key="c.name" class="toolCell" :class="{ on: c.on }">
                        <span class="toolDot"><span class="toolDotCore"></span></span>
                        <span class="toolName">{{ c.name }}</span>
                        <span
                            v-if="c.on && c.version"
                            class="toolVer"
                            v-tooltip="c.version.length > VER_MAX ? c.version : ''"
                        >{{ shortVer(c.version) }}</span>
                        <span v-else-if="!c.on" class="toolVer off">—</span>
                    </div>
                </div>

                <Transition name="noteSlide">
                    <div v-if="depBusy" class="depBar"><span class="depBarFill"></span></div>
                </Transition>

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

            <div class="panelDivider"></div>

            <!-- YouTube cookies (optional) -->
            <div class="panelSection cookiesSection" :class="cookieStatusKind">
                <h3 class="subTitle">{{ t('downloads.cookies.sectionTitle') }}</h3>
                <p class="subDesc">{{ t('downloads.cookies.desc') }}</p>
                <div class="statusRow">
                    <div class="statusLine">
                        <span v-if="cookieStatusKind === 'checking'" class="miniSpinner"></span>
                        <span v-else class="stateDot"></span>
                        <span class="statusText" v-truncate>{{ cookieStatusText }}</span>
                    </div>
                    <div class="rowActions">
                        <button
                            v-if="!cookiesAuthorized"
                            class="actBtn primary"
                            @click="wizardOpen = true"
                        >{{ t('downloads.cookies.getCookie') }}</button>
                        <template v-else>
                            <button
                                class="actBtn ghost"
                                :disabled="cookiesChecking"
                                @click="recheck"
                            >{{ t('downloads.cookies.recheck') }}</button>
                            <button
                                class="actBtn ghost danger"
                                :disabled="cookiesActionBusy"
                                @click="clearCookies"
                            >{{ t('downloads.cookies.clear') }}</button>
                        </template>
                    </div>
                </div>
            </div>

            <template v-if="!isAndroid">
                <div class="panelDivider"></div>

                <button
                    class="advancedToggle"
                    :class="{ open: advancedOpen }"
                    type="button"
                    :aria-expanded="advancedOpen"
                    @click="advancedOpen = !advancedOpen"
                >
                    <span>{{ t('downloads.pot.advancedToggle') }}</span>
                    <ArrowUpIcon class="advancedChevron" :class="{ open: advancedOpen }" />
                </button>

                <!-- PO-token provider (optional; bypasses YouTube's bot-wall even with valid cookies) -->
                <div class="advCollapse" :class="{ open: advancedOpen }" :inert="!advancedOpen">
                    <div class="advInner">
                        <div class="panelSection potSection" :class="potStatusClass">
                            <div class="statusLine">
                                <span v-if="potActionBusy" class="miniSpinner"></span>
                                <span v-else class="stateDot"></span>
                                <span class="statusText" v-truncate>{{ potStatusText }}</span>
                            </div>
                            <p class="subDesc">{{ t('downloads.pot.desc') }}</p>
                            <div class="rowActions potActions">
                                <button
                                    v-if="!toolsInfo.pot.installed"
                                    class="actBtn primary"
                                    :disabled="potActionBusy"
                                    @click="requestInstallPot"
                                >{{ t('downloads.comp.btnInstall') }}</button>
                                <button
                                    v-else-if="potUpdateAvailable"
                                    class="actBtn primary"
                                    :disabled="potActionBusy"
                                    @click="requestInstallPot"
                                >{{ t('downloads.comp.btnUpdate') }}</button>
                                <button
                                    v-if="toolsInfo.pot.managed"
                                    class="actBtn ghost danger"
                                    :disabled="potActionBusy"
                                    @click="onDeletePot"
                                >{{ t('downloads.comp.btnDelete') }}</button>
                            </div>
                        </div>
                    </div>
                </div>
            </template>
        </div>

        <CookieBridgeWizard :open="wizardOpen" @close="wizardOpen = false" />

        <!-- Proxy for yt-dlp (at the very bottom) -->
        <template v-if="!isAndroid">
            <h2 class="sectionTitle">{{ t('downloads.proxy.sectionTitle') }}</h2>
            <div class="dlProxy">
                <p class="dlProxyDesc">{{ t('downloads.proxy.desc') }}</p>
                <div class="proxySeg" role="radiogroup" :style="{ '--seg-i': proxyIndex }">
                    <span class="segThumb" aria-hidden="true"></span>
                    <button
                        v-for="m in proxyModeOptions"
                        :key="m.key"
                        type="button"
                        class="proxySegBtn"
                        :class="{ active: proxyConfig.mode === m.key }"
                        :aria-pressed="proxyConfig.mode === m.key"
                        @click="selectProxyMode(m.key)"
                    >
                        <span>{{ m.label }}</span>
                    </button>
                </div>
                <div class="proxyCustomWrap" :class="{ open: proxyConfig.mode === 'custom' }" :inert="proxyConfig.mode !== 'custom'">
                    <div class="proxyCustomInner">
                        <input
                            class="proxyInput"
                            type="text"
                            :placeholder="t('downloads.proxy.customPlaceholder')"
                            v-model="proxyUrlInput"
                            spellcheck="false"
                            autocapitalize="off"
                            autocomplete="off"
                            @blur="saveCustomUrl"
                            @keydown.enter="saveCustomUrl"
                        />
                    </div>
                </div>
                <Transition name="hintFade" mode="out-in">
                    <p class="proxyHint" :key="proxyConfig.mode">{{ proxyHint }}</p>
                </Transition>
                <Transition name="noteSlide">
                    <p v-if="proxyError" class="dlWarn">{{ proxyError }}</p>
                </Transition>
            </div>
        </template>

        <!-- Regional blocking: downloading may be unavailable -->
        <p class="regionNote">{{ t('downloads.vpnNote') }}</p>
        </div>
        </Transition>

        <ConfirmModal
            :open="confirmOpen"
            :title="confirmTitle"
            :message="confirmMessage"
            :confirmText="confirmOkLabel"
            :cancelText="t('downloads.comp.confirmCancel')"
            @confirm="confirmInstall"
            @cancel="confirmOpen = false"
        />
        <ConfirmModal
            :open="potConfirmOpen"
            :title="t('downloads.comp.confirmTitle')"
            :message="t('downloads.comp.confirmText', { name: 'bgutil-pot', source: 'GitHub' })"
            :confirmText="t('downloads.comp.confirmOk')"
            :cancelText="t('downloads.comp.confirmCancel')"
            @confirm="confirmInstallPot"
            @cancel="potConfirmOpen = false"
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
    margin-top: 6px;
    max-width: 800px;
    font-weight: 500;
    font-size: 13px;
    line-height: 1.5;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.42);
}

/* The whole downloads zone appears/disappears with the master toggle above. */
.dlZone {
    display: flex;
    flex-direction: column;
    width: 100%;
}
.dlZone-enter-active {
    transition: opacity .32s ease, transform .32s cubic-bezier(0.22, 1, 0.36, 1);
}
.dlZone-leave-active {
    transition: opacity .16s ease;
}
.dlZone-enter-from {
    opacity: 0;
    transform: translateY(12px);
}
.dlZone-leave-to {
    opacity: 0;
}

/* Bottom footnote about regional blocking/VPN — quiet, no call to action. */
.regionNote {
    margin-top: 30px;
    padding-top: 18px;
    max-width: 800px;
    width: 100%;
    border-top: 1px solid hsla(0, 0%, 100%, 0.05);
    font-weight: 500;
    font-size: 12.5px;
    line-height: 1.5;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.38);
}

/* Warning/error notes shared by folder and proxy blocks; animated in. */
.dlWarn {
    margin: 10px 0 0;
    font-size: 12.5px;
    font-weight: 600;
    line-height: 1.4;
    color: hsla(38, 85%, 72%, 0.95);
}
.noteSlide-enter-active {
    transition: opacity .22s ease, transform .22s cubic-bezier(0.22, 1, 0.36, 1);
}
.noteSlide-leave-active {
    transition: opacity .14s ease;
}
.noteSlide-enter-from {
    opacity: 0;
    transform: translateY(-4px);
}
.noteSlide-leave-to {
    opacity: 0;
}

/* Section titles of the downloads zone */
.sectionTitle {
    margin-top: 40px;
    margin-bottom: 16px;
    max-width: 800px;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 19px;
    color: hsla(0, 0%, 100%, 0.92);
}

/* Download folder */
.dlFolder {
    max-width: 800px;
    width: 100%;
}
.dlFolderRow {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 14px;
    padding: 12px 16px 12px 12px;
    border-radius: 18px;
    background: hsla(0, 0%, 100%, 0.028);
    transition: background-color .2s ease;
}
.dlFolderRow:hover {
    background: hsla(0, 0%, 100%, 0.04);
}
/* Accent-tinted tile; opens the folder in the system explorer. */
.dlFolderTile {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 42px;
    height: 42px;
    border-radius: 13px;
    cursor: pointer;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 78%, 0.8);
    background: hsla(var(--accent-h), var(--accent-s), 62%, 0.1);
    transition: background-color .2s ease, color .2s ease, transform .2s ease;
}
.dlFolderTile:hover {
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.5), 88%, 1);
    background: hsla(var(--accent-h), var(--accent-s), 62%, 0.16);
    transform: translateY(-1px);
}
.dlFolderTile:active {
    transform: translateY(0) scale(0.96);
}
.dlFolderInfo {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
}
.dlFolderPath {
    font-family: "JetBrains Mono", "SF Mono", ui-monospace, "Cascadia Code", Consolas, monospace;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: hsla(0, 0%, 100%, 0.9);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.dlFolderHint {
    font-size: 12px;
    font-weight: 500;
    line-height: 1.4;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.45);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
.dlFolderActions {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 8px;
}

/* Proxy for yt-dlp */
.dlProxy {
    max-width: 800px;
    width: 100%;
}
.dlProxyDesc {
    margin: 0 0 14px;
    font-size: 12.5px;
    font-weight: 500;
    line-height: 1.5;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.5);
}
/* Mode switcher — dark container with an inset border and a sliding thumb
   behind the active option (equal-width segments make the slide exact). */
.proxySeg {
    position: relative;
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 2px;
    padding: 3px;
    width: min(100%, 340px);
    border-radius: 16px;
    background-color: var(--dark-bg);
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.06);
}
.segThumb {
    position: absolute;
    top: 3px;
    bottom: 3px;
    left: 3px;
    width: calc((100% - 6px - 4px) / 3);
    border-radius: 13px;
    background-color: hsla(0, 0%, 100%, 0.09);
    transform: translateX(calc(var(--seg-i, 0) * (100% + 2px)));
    transition: transform .26s cubic-bezier(0.22, 1, 0.36, 1);
    will-change: transform;
}
.proxySegBtn {
    user-select: none;
    position: relative;
    z-index: 1;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 10px;
    border-radius: 13px;
    background-color: transparent;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.35), 84%, 0.58);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: color .18s ease;
}
.proxySegBtn:hover {
    color: hsla(0, 0%, 100%, 0.82);
}
.proxySegBtn.active {
    color: #ffffff;
}
/* Custom URL field appears only in "custom" mode — smooth grid collapse. */
.proxyCustomWrap {
    display: grid;
    grid-template-rows: 0fr;
    transition: grid-template-rows .3s cubic-bezier(0.22, 1, 0.36, 1);
}
.proxyCustomWrap.open {
    grid-template-rows: 1fr;
}
.proxyCustomInner {
    overflow: hidden;
    min-height: 0;
    opacity: 0;
    transform: translateY(-4px);
    transition: opacity .22s ease, transform .3s cubic-bezier(0.22, 1, 0.36, 1);
}
.proxyCustomWrap.open .proxyCustomInner {
    opacity: 1;
    transform: none;
}
.proxyInput {
    width: 100%;
    max-width: 420px;
    margin-top: 12px;
    padding: 10px 16px;
    border-radius: 999px;
    border: none;
    background: hsla(0, 0%, 100%, 0.04);
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0);
    color: hsla(0, 0%, 100%, 0.92);
    font-family: "JetBrains Mono", "SF Mono", ui-monospace, "Cascadia Code", Consolas, monospace;
    font-size: 13px;
    outline: none;
    transition: background-color .18s ease, box-shadow .18s ease;
}
.proxyInput:focus {
    background: hsla(0, 0%, 100%, 0.07);
    box-shadow: inset 0 0 0 1px hsla(var(--accent-h), var(--accent-s), 70%, 0.35);
}
.proxyHint {
    margin: 10px 0 0;
    font-size: 12px;
    font-weight: 500;
    line-height: 1.45;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.42);
}
.hintFade-enter-active,
.hintFade-leave-active {
    transition: opacity .14s ease;
}
.hintFade-enter-from,
.hintFade-leave-to {
    opacity: 0;
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

/* Unified panel: components + cookies + advanced (PO-token), one continuous
   card with thin dividers between sub-sections instead of separate boxes. */
.unifiedPanel {
    max-width: 800px;
    width: 100%;
    border-radius: 24px;
    background: hsla(0, 0%, 100%, 0.028);
    overflow: hidden;
}
/* Something missing / update available — a soft accent top nudges toward
   action (no large bright surfaces, just a thin gradient at the top). */
.unifiedPanel.missing,
.unifiedPanel.update {
    background:
        linear-gradient(180deg, hsla(var(--accent-h), var(--accent-s), 50%, 0.07), hsla(0, 0%, 100%, 0.028) 70px),
        hsla(0, 0%, 100%, 0.028);
}
/* Staggered entrance when the panel replaces the skeleton. */
.unifiedPanel:not(.skeletonPanel) > * {
    animation: panelRise .4s cubic-bezier(0.16, 1, 0.3, 1) backwards;
}
.unifiedPanel:not(.skeletonPanel) > *:nth-child(2) { animation-delay: .04s; }
.unifiedPanel:not(.skeletonPanel) > *:nth-child(3) { animation-delay: .07s; }
.unifiedPanel:not(.skeletonPanel) > *:nth-child(4) { animation-delay: .1s; }
.unifiedPanel:not(.skeletonPanel) > *:nth-child(5) { animation-delay: .13s; }
.unifiedPanel:not(.skeletonPanel) > *:nth-child(6) { animation-delay: .16s; }
@keyframes panelRise {
    from {
        opacity: 0;
        transform: translateY(7px);
    }
    to {
        opacity: 1;
        transform: none;
    }
}
.panelSection {
    padding: 18px 20px 16px;
}
.panelDivider {
    height: 1px;
    margin: 0 20px;
    background: hsla(0, 0%, 100%, 0.05);
}

/* Skeleton while tool info loads — mirrors the real panel's layout. */
.skeletonPanel {
    min-height: 168px;
}
.skelRow {
    display: flex;
    align-items: center;
    gap: 8px;
}
.skel {
    display: block;
    border-radius: 7px;
    background: linear-gradient(
        90deg,
        hsla(0, 0%, 100%, 0.04) 25%,
        hsla(0, 0%, 100%, 0.085) 50%,
        hsla(0, 0%, 100%, 0.04) 75%
    );
    background-size: 200% 100%;
    animation: shimmer 1.4s linear infinite;
}
.skelPill {
    border-radius: 999px;
}
@keyframes shimmer {
    from { background-position: 200% 0; }
    to { background-position: -200% 0; }
}

.depTop {
    display: flex;
    flex-direction: row;
    align-items: flex-start;
    gap: 16px;
}
.depHead { flex: 1; min-width: 0; }
.subTitle {
    margin: 0;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 15px;
    color: hsla(0, 0%, 100%, 0.95);
}
.subDesc {
    margin: 5px 0 0;
    font-size: 12px;
    font-weight: 500;
    line-height: 1.45;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.4), 80%, 0.45);
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
    transition: color .24s ease, background-color .24s ease;
}
.stateDot {
    flex-shrink: 0;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.3);
    transition: background-color .24s ease;
}
.depState.ok { color: hsla(145, 55%, 70%, 0.95); background: hsla(145, 45%, 45%, 0.12); }
.depState.ok .stateDot { background-color: hsla(145, 60%, 55%, 0.95); }
.depState.update { color: hsla(38, 90%, 70%, 1); background: hsla(38, 85%, 50%, 0.14); cursor: help; }
.depState.update .stateDot {
    background-color: hsla(38, 90%, 58%, 0.95);
    animation: dotPulse 2.2s ease-in-out infinite;
}
.depState.missing { color: hsla(0, 0%, 100%, 0.5); }
.depState.missing .stateDot { background-color: hsla(0, 0%, 100%, 0.28); }
@keyframes dotPulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
}

/* The three tools as one rail of equal cells with hairline separators. */
.toolRail {
    display: flex;
    flex-direction: row;
    margin-top: 14px;
    border-radius: 13px;
    background: hsla(0, 0%, 0%, 0.16);
    overflow: hidden;
}
.toolCell {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
}
.toolCell + .toolCell {
    border-left: 1px solid hsla(0, 0%, 100%, 0.05);
}
/* Ring + core dot: the core pops green when the tool becomes installed. */
.toolDot {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1px hsla(0, 0%, 100%, 0.14);
    transition: box-shadow .24s ease;
}
.toolDotCore {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.22);
    transition: background-color .24s ease;
}
.toolCell.on .toolDot {
    box-shadow: inset 0 0 0 1px hsla(145, 50%, 55%, 0.4);
}
.toolCell.on .toolDotCore {
    background-color: hsla(145, 60%, 55%, 0.95);
    animation: dotPop .35s cubic-bezier(0.22, 1, 0.36, 1);
}
@keyframes dotPop {
    0% { transform: scale(0.4); }
    60% { transform: scale(1.35); }
    100% { transform: scale(1); }
}
.toolName {
    font-family: "JetBrains Mono", "SF Mono", ui-monospace, "Cascadia Code", Consolas, monospace;
    font-size: 12.5px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: hsla(0, 0%, 100%, 0.45);
    transition: color .24s ease;
    white-space: nowrap;
}
.toolCell.on .toolName { color: hsla(0, 0%, 100%, 0.9); }
.toolVer {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.35);
}
.toolVer.off { color: hsla(0, 0%, 100%, 0.2); }

/* Thin indicator bar during install. */
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

/* Action buttons (shared across the zone) */
.actBtn {
    cursor: pointer;
    position: relative;
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
.actBtn:not(:disabled):active {
    transform: scale(0.97);
}
.actBtn.primary {
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
    background-color: hsla(0, 0%, 100%, 0.95);
}
.actBtn.primary:not(:disabled):hover {
    background-color: #fff;
    transform: translateY(-1px);
}
.actBtn.primary:not(:disabled):active {
    transform: translateY(0) scale(0.97);
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
/* Spinner sits ON TOP of the text (absolute); text is just faded via opacity,
   so button width never changes. Both stay in the DOM and cross-fade via
   opacity instead of flicking with v-if. */
.actBtn .btnLabel {
    opacity: 1;
    transition: opacity .18s ease;
}
.actBtn.loading .btnLabel {
    opacity: 0;
}
/* Centered via negative margins, not translate — the spin animation owns
   the transform property. */
.actBtn .btnSpinner {
    position: absolute;
    top: 50%;
    left: 50%;
    margin: -6.5px 0 0 -6.5px;
    opacity: 0;
    pointer-events: none;
    transition: opacity .18s ease;
}
.actBtn.loading .btnSpinner {
    opacity: 1;
}

/* "Check updates" button: both labels sit in the same grid cell, so width is
   fixed to the longest one and doesn't shift when switching to the "Checking…"
   spinner label (the other label stays in flow, just hidden). */
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

/* Cookies/PO-token sections: quieter than the components section (both
   optional). A sub-title + short description, then one row with the live
   status on the left and actions on the right. */
.statusRow {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 12px;
}
.statusLine {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
}
.statusText {
    min-width: 0;
    font-size: 13px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.85);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    transition: color .24s ease;
}
.rowActions {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 8px;
}
.panelSection.valid .stateDot { background-color: hsla(145, 60%, 55%, 0.95); }
.panelSection.valid .statusText { color: hsla(145, 45%, 75%, 0.95); }
.panelSection.invalid .statusText { color: hsla(2, 60%, 74%, 0.92); }
.panelSection.invalid .stateDot { background-color: hsla(2, 70%, 60%, 0.95); }
.panelSection.none .stateDot { background-color: hsla(0, 0%, 100%, 0.28); }

/* PO-token section: status line first (it IS the title there), then the
   description, then right-aligned actions. */
.potSection .statusLine { margin-top: 0; }
.potActions {
    justify-content: flex-end;
    margin-top: 12px;
}

/* "Advanced" collapsible toggle (PO-token, optional) — same weight/color
   family as .subTitle so it reads as a sub-section header, not a stray link. */
.advancedToggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 14px 20px;
    border: none;
    background: transparent;
    cursor: pointer;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 13.5px;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.6);
    transition: color .2s ease, background-color .2s ease;
}
.advancedToggle:hover {
    color: hsla(0, 0%, 100%, 0.85);
    background-color: hsla(0, 0%, 100%, 0.025);
}
.advancedToggle.open {
    color: hsla(0, 0%, 100%, 0.9);
}
.advancedChevron {
    width: 14px;
    height: 14px;
    opacity: 0.55;
    transform: rotate(180deg);
    transition: transform .26s cubic-bezier(0.22, 1, 0.36, 1), opacity .2s ease;
}
.advancedChevron.open {
    opacity: 0.8;
    transform: rotate(0deg);
}

/* Smooth height collapse for the PO-token section (grid-rows trick: content
   stays mounted, so no re-fetch/re-render on every open). */
.advCollapse {
    display: grid;
    grid-template-rows: 0fr;
    transition: grid-template-rows .3s cubic-bezier(0.22, 1, 0.36, 1);
}
.advCollapse.open {
    grid-template-rows: 1fr;
}
.advInner {
    overflow: hidden;
    min-height: 0;
    opacity: 0;
    transform: translateY(-4px);
    transition: opacity .22s ease, transform .3s cubic-bezier(0.22, 1, 0.36, 1);
}
.advCollapse.open .advInner {
    opacity: 1;
    transform: none;
}
.advCollapse .potSection {
    padding-top: 4px;
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .settingsBlock,
    .hint,
    .sectionTitle,
    .dlFolder,
    .dlProxy,
    .unifiedPanel,
    .regionNote {
        max-width: none;
    }
    .dlFolderRow { flex-wrap: wrap; }
    .dlFolderActions { flex: 1 0 100%; }
    .dlFolderActions .actBtn { flex: 1; }
    .toolRail { flex-direction: column; }
    .toolCell + .toolCell {
        border-left: none;
        border-top: 1px solid hsla(0, 0%, 100%, 0.05);
    }
    .statusRow { flex-wrap: wrap; }
    .rowActions { flex: 1 0 100%; }
    .depActions { flex-wrap: wrap; }
    .actBtn { flex: 1; padding: 12px 18px; }
    .proxySeg { width: 100%; }
}
</style>
