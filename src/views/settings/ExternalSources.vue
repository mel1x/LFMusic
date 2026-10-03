<script setup lang="ts">
import { onMounted, computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useSettings, type SettingKey } from '../../composables/useSettings';
import { useDownloads, type ProxyMode, type CookiesMode } from '../../composables/useDownloads';
import { useI18n } from '../../composables/useI18n';
import { isAndroidRuntime } from '../../composables/usePlatform';
import SettingsShell from '../../components/SettingsShell.vue';
import SettingToggle from '../../components/SettingToggle.vue';
import ConfirmModal from '../../components/ConfirmModal.vue';
import FolderIcon from '../../components/Icons/FolderIcon.vue';

const { t } = useI18n();
const { settings, settingItems, load, save } = useSettings();
const {
    tools,
    toolsLoaded,
    dlSettings,
    downloadsDir,
    refreshTools,
    checkUpdates,
    installTools,
    removeTools,
    saveSettings,
    setDownloadsDir,
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

// ── Components (yt-dlp / Deno / ffmpeg) ─────────────────────────────────────

const info = computed(() => tools.value.info);
const toolError = ref<string | null>(null);
const busy = computed(() => tools.value.busy);
const required = computed(() => info.value.ytdlp.installed && info.value.deno.installed);
const anyManaged = computed(() => info.value.ytdlp.managed || info.value.deno.managed || info.value.ffmpeg.managed);
const anyMissing = computed(() => !info.value.ytdlp.installed || !info.value.deno.installed || !info.value.ffmpeg.installed);
const updateAvailable = computed(() =>
    (info.value.ytdlp.managed && info.value.ytdlp.update_available) || (info.value.deno.managed && info.value.deno.update_available),
);

type ToolsStateKind = 'busy' | 'missing' | 'update' | 'ok';
const stateKind = computed<ToolsStateKind>(() => {
    if (busy.value) return 'busy';
    if (!required.value) return 'missing';
    if (updateAvailable.value) return 'update';
    return 'ok';
});
const stateLabel = computed(() => ({
    busy: t('downloads.tools.stateBusy'),
    missing: t('downloads.tools.stateMissing'),
    update: t('downloads.tools.stateUpdate'),
    ok: t('downloads.tools.stateReady'),
})[stateKind.value]);

const cells = computed(() => [
    { name: 'yt-dlp', ...info.value.ytdlp },
    { name: 'Deno', ...info.value.deno },
    { name: 'ffmpeg', ...info.value.ffmpeg },
]);
const VER_MAX = 16;
function shortVer(v: string): string {
    return v.length > VER_MAX ? v.slice(0, VER_MAX) + '…' : v;
}

const detail = computed(() => {
    if (toolError.value || tools.value.error) return toolError.value || tools.value.error;
    if (tools.value.progress) return tools.value.progress.message;
    return t('downloads.tools.desc');
});

const confirmOpen = ref(false);
async function runInstall() {
    confirmOpen.value = false;
    toolError.value = null;
    try { await installTools(); } catch (e) { toolError.value = String(e); }
}
const checking = ref(false);
async function onCheck() {
    if (checking.value || busy.value) return;
    checking.value = true;
    toolError.value = null;
    try { await checkUpdates(true); } finally { checking.value = false; }
}
async function onRemove() {
    if (busy.value) return;
    toolError.value = null;
    try { await removeTools(); } catch (e) { toolError.value = String(e); }
}
async function toggleAutoUpdate() {
    try { await saveSettings({ auto_update: !dlSettings.value.auto_update }); } catch (e) { toolError.value = String(e); }
}

// ── Cookies ─────────────────────────────────────────────────────────────────

const cookieModes = computed<{ key: CookiesMode; label: string }[]>(() => [
    { key: 'none', label: t('downloads.cookies.modeNone') },
    { key: 'browser', label: t('downloads.cookies.modeBrowser') },
    { key: 'file', label: t('downloads.cookies.modeFile') },
]);
const cookieIndex = computed(() => Math.max(0, cookieModes.value.findIndex(m => m.key === dlSettings.value.cookies_mode)));
const isMac = navigator.userAgent.includes('Mac');
const browsers = computed(() => {
    const list = ['firefox', 'chrome', 'edge', 'brave', 'opera', 'vivaldi', 'chromium'];
    if (isMac) list.push('safari');
    return list;
});
const BROWSER_LABELS: Record<string, string> = {
    firefox: 'Firefox', chrome: 'Chrome', edge: 'Edge', brave: 'Brave', opera: 'Opera',
    vivaldi: 'Vivaldi', chromium: 'Chromium', safari: 'Safari',
};
const cookieError = ref<string | null>(null);

async function setCookieMode(mode: CookiesMode) {
    if (mode === dlSettings.value.cookies_mode) return;
    cookieError.value = null;
    if (mode === 'file' && !dlSettings.value.cookies_file) {
        await pickCookieFile();
        return;
    }
    try { await saveSettings({ cookies_mode: mode }); } catch (e) { cookieError.value = String(e); }
}
async function setBrowser(name: string) {
    cookieError.value = null;
    try { await saveSettings({ cookies_browser: name }); } catch (e) { cookieError.value = String(e); }
}
async function pickCookieFile() {
    cookieError.value = null;
    try {
        const res = await open({ multiple: false, directory: false, filters: [{ name: 'cookies.txt', extensions: ['txt'] }] });
        const path = typeof res === 'string' ? res : null;
        if (!path) return;
        await saveSettings({ cookies_mode: 'file', cookies_file: path });
    } catch (e) {
        cookieError.value = String(e);
    }
}
const cookieFileName = computed(() => {
    const f = dlSettings.value.cookies_file;
    return f ? f.split(/[\\/]/).pop() : t('downloads.cookies.noFile');
});

// ── Download folder ─────────────────────────────────────────────────────────
// Changing it re-registers the library folder (old unscanned, new scanned);
// the backend falls back to the default if the chosen one can't be created.
const folderBusy = ref(false);
const folderFellBack = ref(false);
const folderError = ref<string | null>(null);
const folderDisplay = computed(() => (downloadsDir.value || '').replace(/\\/g, '/'));

function norm(p: string): string {
    return p.replace(/[\\/]+$/, '').replace(/\\/g, '/').toLowerCase();
}

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

async function openDownloadFolder() {
    if (!downloadsDir.value) return;
    try {
        await invoke('open_folder_in_explorer', { path: downloadsDir.value });
    } catch (e) {
        console.error('Не удалось открыть папку:', e);
    }
}

// ── Proxy ───────────────────────────────────────────────────────────────────
// Useful against ISP blocking, e.g. routing through ByeDPI (socks5://127.0.0.1:1080).
const proxyModeOptions = computed<{ key: ProxyMode; label: string }[]>(() => [
    { key: 'system', label: t('downloads.proxy.modeSystem') },
    { key: 'none', label: t('downloads.proxy.modeNone') },
    { key: 'custom', label: t('downloads.proxy.modeCustom') },
]);
const proxyIndex = computed(() =>
    Math.max(0, proxyModeOptions.value.findIndex(m => m.key === dlSettings.value.proxy_mode)),
);
const proxyUrlInput = ref(dlSettings.value.proxy_url);
const proxyError = ref<string | null>(null);
watch(() => dlSettings.value.proxy_url, (v) => { proxyUrlInput.value = v; });

const proxyHint = computed(() => {
    switch (dlSettings.value.proxy_mode) {
        case 'none': return t('downloads.proxy.noneHint');
        case 'custom': return t('downloads.proxy.customHint');
        default: return t('downloads.proxy.systemHint');
    }
});

function isValidProxyUrl(u: string): boolean {
    return /^(socks(5h?|4a?)|https?):\/\/\S+/i.test(u.trim());
}

async function applyProxy(mode: ProxyMode, url: string) {
    proxyError.value = null;
    const u = url.trim();
    if (mode === 'custom' && u && !isValidProxyUrl(u)) {
        proxyError.value = t('downloads.proxy.invalidUrl');
        return;
    }
    try {
        await saveSettings({ proxy_mode: mode, proxy_url: mode === 'custom' ? u : dlSettings.value.proxy_url });
    } catch (e) {
        proxyError.value = String(e);
    }
}

function selectProxyMode(mode: ProxyMode) {
    if (mode === dlSettings.value.proxy_mode) return;
    void applyProxy(mode, mode === 'custom' ? proxyUrlInput.value : '');
}

function saveCustomUrl() {
    if (dlSettings.value.proxy_mode !== 'custom') return;
    if (proxyUrlInput.value.trim() === dlSettings.value.proxy_url) return;
    void applyProxy('custom', proxyUrlInput.value);
}

onMounted(async () => {
    await load();
    if (isAndroid) return;
    try {
        await refreshTools();
        await checkUpdates();
    } catch { /* offline — no update info */ }
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

        <!-- Downloader settings only make sense with the Downloads page enabled. -->
        <Transition name="dlZone">
        <div v-if="settings.show_downloads_page && !isAndroid" class="dlZone">
            <!-- Download folder -->
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

            <!-- Components + cookies -->
            <h2 class="sectionTitle">{{ t('downloads.tools.sectionTitle') }}</h2>
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
                </div>
            </div>
            <div v-else class="unifiedPanel" :class="stateKind">
                <div class="panelSection depSection">
                    <div class="depTop">
                        <div class="depHead">
                            <h3 class="subTitle">yt-dlp · Deno · ffmpeg</h3>
                            <p class="depDesc" :class="{ isError: !!(toolError || tools.error) }" v-truncate>{{ detail }}</p>
                        </div>
                        <div class="depState" :class="stateKind">
                            <span v-if="stateKind === 'busy'" class="miniSpinner"></span>
                            <span v-else class="stateDot"></span>
                            <span>{{ stateLabel }}</span>
                        </div>
                    </div>

                    <div class="toolRail">
                        <div v-for="c in cells" :key="c.name" class="toolCell" :class="{ on: c.installed, update: c.installed && c.managed && c.update_available }">
                            <span class="toolDot"><span class="toolDotCore"></span></span>
                            <span class="toolName">{{ c.name }}</span>
                            <span
                                v-if="c.installed && c.version"
                                class="toolVer"
                                v-tooltip="(c.version.length > VER_MAX ? c.version : '') + (c.managed ? '' : ` (${t('downloads.tools.system')})`)"
                            >{{ shortVer(c.version) }}</span>
                            <span v-else-if="!c.installed" class="toolVer off">-</span>
                        </div>
                    </div>

                    <Transition name="noteSlide">
                        <div v-if="busy" class="depBar">
                            <span
                                class="depBarFill"
                                :style="tools.progress?.fraction != null ? { width: Math.round(tools.progress.fraction * 100) + '%', animation: 'none' } : undefined"
                            ></span>
                        </div>
                    </Transition>

                    <div class="depActions">
                        <button v-if="anyMissing" class="actBtn primary" :disabled="busy" @click="confirmOpen = true">{{ t('downloads.tools.install') }}</button>
                        <button v-if="updateAvailable" class="actBtn primary" :disabled="busy" @click="runInstall">{{ t('downloads.tools.update') }}</button>
                        <button
                            v-if="info.ytdlp.installed"
                            class="actBtn ghost checkBtn"
                            :class="{ checking }"
                            :disabled="busy || checking"
                            @click="onCheck"
                        >
                            <span class="checkLabels">
                                <span class="checkLabel">{{ t('downloads.tools.check') }}</span>
                                <span class="checkLabel busy"><span class="miniSpinner"></span>{{ t('downloads.tools.checking') }}</span>
                            </span>
                        </button>
                        <button
                            class="actBtn ghost danger"
                            :disabled="busy || !anyManaged"
                            v-tooltip="t('downloads.tools.removeTooltip')"
                            @click="onRemove"
                        >{{ t('downloads.tools.remove') }}</button>
                    </div>
                    <div class="autoUpdateRow">
                        <SettingToggle
                            compact
                            :item="{ label: t('downloads.tools.autoUpdate'), description: t('downloads.tools.autoUpdateDesc') }"
                            :checked="dlSettings.auto_update"
                            @toggle="toggleAutoUpdate"
                        />
                    </div>
                </div>

                <div class="panelDivider"></div>

                <div class="panelSection cookiesSection">
                    <h3 class="subTitle">{{ t('downloads.cookies.sectionTitle') }}</h3>
                    <p class="subDesc">{{ t('downloads.cookies.desc') }}</p>
                    <div class="proxySeg cookieSeg" role="radiogroup" :style="{ '--seg-i': cookieIndex }">
                        <span class="segThumb" aria-hidden="true"></span>
                        <button
                            v-for="m in cookieModes"
                            :key="m.key"
                            type="button"
                            class="proxySegBtn"
                            :class="{ active: dlSettings.cookies_mode === m.key }"
                            :aria-pressed="dlSettings.cookies_mode === m.key"
                            @click="setCookieMode(m.key)"
                        ><span>{{ m.label }}</span></button>
                    </div>
                    <Transition name="noteSlide" mode="out-in">
                        <div v-if="dlSettings.cookies_mode === 'browser'" key="browser" class="cookieDetail">
                            <div class="browserChips">
                                <button
                                    v-for="b in browsers"
                                    :key="b"
                                    class="browserChip"
                                    :class="{ active: dlSettings.cookies_browser === b }"
                                    @click="setBrowser(b)"
                                >{{ BROWSER_LABELS[b] }}</button>
                            </div>
                            <p class="proxyHint">{{ t('downloads.cookies.browserHint') }}</p>
                        </div>
                        <div v-else-if="dlSettings.cookies_mode === 'file'" key="file" class="cookieDetail">
                            <div class="cookieFileRow">
                                <span class="cookieFileName" v-truncate>{{ cookieFileName }}</span>
                                <button class="actBtn ghost" @click="pickCookieFile">{{ t('downloads.cookies.pickFile') }}</button>
                            </div>
                            <p class="proxyHint">{{ t('downloads.cookies.fileHint') }}</p>
                        </div>
                    </Transition>
                    <Transition name="noteSlide">
                        <p v-if="cookieError" class="dlWarn">{{ cookieError }}</p>
                    </Transition>
                </div>
            </div>

            <!-- Proxy -->
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
                        :class="{ active: dlSettings.proxy_mode === m.key }"
                        :aria-pressed="dlSettings.proxy_mode === m.key"
                        @click="selectProxyMode(m.key)"
                    >
                        <span>{{ m.label }}</span>
                    </button>
                </div>
                <div class="proxyCustomWrap" :class="{ open: dlSettings.proxy_mode === 'custom' }" :inert="dlSettings.proxy_mode !== 'custom'">
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
                    <p class="proxyHint" :key="dlSettings.proxy_mode">{{ proxyHint }}</p>
                </Transition>
                <Transition name="noteSlide">
                    <p v-if="proxyError" class="dlWarn">{{ proxyError }}</p>
                </Transition>
            </div>

            <p class="regionNote">{{ t('downloads.vpnNote') }}</p>
        </div>
        </Transition>

        <ConfirmModal
            :open="confirmOpen"
            :title="t('downloads.setup.confirmTitle')"
            :message="t('downloads.setup.confirmText')"
            :confirmText="t('downloads.setup.confirmOk')"
            :cancelText="t('downloads.setup.cancel')"
            @confirm="runInstall"
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

/* Unified panel: components + cookies, one continuous
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
/* Update available for this specific tool overrides the green "on" dot with
   the same orange used by the overall .depState.update pill. */
.toolCell.update .toolDot {
    box-shadow: inset 0 0 0 1px hsla(38, 85%, 55%, 0.4);
}
.toolCell.update .toolDotCore {
    background-color: hsla(38, 90%, 58%, 0.95);
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

/* "Advanced" collapsible toggle (PO-token, optional) — same weight/color
   family as .subTitle so it reads as a sub-section header, not a stray link. */

/* Smooth height collapse for the PO-token section (grid-rows trick: content
   stays mounted, so no re-fetch/re-render on every open). */

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

/* Cookies: browser picker + file row (reuses the proxy segmented control). */
.cookieSeg { margin-top: 12px; }
.cookieDetail { margin-top: 12px; }
.browserChips { display: flex; flex-wrap: wrap; gap: 6px; }
.browserChip {
    cursor: pointer;
    padding: 7px 14px;
    border-radius: 999px;
    font-size: 12.5px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.65);
    background-color: hsla(0, 0%, 100%, 0.06);
    transition: background-color .18s ease, color .18s ease;
}
.browserChip:hover { color: #fff; background-color: hsla(0, 0%, 100%, 0.11); }
.browserChip.active {
    color: hsla(var(--accent-h), var(--accent-s), 16%, 1);
    background-color: hsla(0, 0%, 100%, 0.92);
}
.cookieFileRow { display: flex; align-items: center; gap: 12px; }
.cookieFileName {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.75);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.autoUpdateRow { margin-top: 10px; }
.depDesc.isError { color: hsla(2, 75%, 74%, 0.95); }
</style>
