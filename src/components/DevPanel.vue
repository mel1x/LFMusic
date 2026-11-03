<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from '../composables/useI18n';
import { usePlayer } from '../composables/usePlayer';
import { currentUiScale } from '../composables/useUiScale';
import {
    devCalls, devStats, devErrors, devStartedAt,
    clearDevCalls, clearDevErrors, type DevError,
} from '../composables/useDevTools';
import CloseIcon from './Icons/CloseIcon.vue';

// Developer panel, shown only when dev mode was on at startup. Capture data is
// plain arrays filled by useDevTools; while the panel is open it re-reads them
// on a short timer instead of making every invoke reactive.

interface DevInfo {
    version: string;
    os: string;
    arch: string;
    debug_build: boolean;
    data_dir: string;
}

type Tab = 'overview' | 'calls' | 'errors';

const { t } = useI18n();
const { saveState } = usePlayer();

const open = ref(false);
const tab = ref<Tab>('overview');
const filter = ref('');
const info = ref<DevInfo | null>(null);
const copied = ref(false);

// Bumped by the poll timer; computeds read it to pick up new capture data.
const tick = ref(0);
const errorCount = ref(devErrors.length);

const fps = ref(0);
const memoryMb = ref<number | null>(null);
const domNodes = ref(0);
const viewport = ref('');

let pollTimer: ReturnType<typeof setInterval> | null = null;
let fpsRaf: number | null = null;
let fpsFrames = 0;
let fpsSince = 0;

function sample() {
    tick.value++;
    errorCount.value = devErrors.length;
    if (!open.value) return;
    const mem = (performance as any).memory;
    memoryMb.value = mem ? mem.usedJSHeapSize / 1048576 : null;
    domNodes.value = document.getElementsByTagName('*').length;
    viewport.value = `${window.innerWidth} x ${window.innerHeight}`;
    const now = performance.now();
    if (fpsSince && now > fpsSince) fps.value = Math.round(fpsFrames * 1000 / (now - fpsSince));
    fpsFrames = 0;
    fpsSince = now;
}

function fpsLoop() {
    fpsFrames++;
    fpsRaf = requestAnimationFrame(fpsLoop);
}

function startPolling(ms: number) {
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = setInterval(sample, ms);
}

watch(open, isOpen => {
    if (isOpen) {
        if (!info.value) invoke<DevInfo>('dev_info').then(v => { info.value = v; }).catch(() => {});
        fpsFrames = 0;
        fpsSince = performance.now();
        fpsRaf = requestAnimationFrame(fpsLoop);
        sample();
        startPolling(500);
    } else {
        if (fpsRaf !== null) cancelAnimationFrame(fpsRaf);
        fpsRaf = null;
        // Closed: only the error badge needs updates.
        startPolling(2000);
    }
});

const uptime = computed(() => {
    void tick.value;
    const s = Math.floor((Date.now() - devStartedAt) / 1000);
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const pad = (n: number) => String(n).padStart(2, '0');
    return h > 0 ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
});

const filterText = computed(() => filter.value.trim().toLowerCase());

const stats = computed(() => {
    void tick.value;
    const q = filterText.value;
    return [...devStats.values()]
        .filter(s => !q || s.cmd.toLowerCase().includes(q))
        .sort((a, b) => b.totalMs - a.totalMs)
        .map(s => ({ ...s, avgMs: s.totalMs / s.count }));
});

const recent = computed(() => {
    void tick.value;
    const q = filterText.value;
    const out = [];
    for (let i = devCalls.length - 1; i >= 0 && out.length < 100; i--) {
        const c = devCalls[i];
        if (!q || c.cmd.toLowerCase().includes(q)) out.push({ ...c });
    }
    return out;
});

const errors = computed(() => {
    void tick.value;
    return [...devErrors].reverse();
});

const formatMs = (ms: number) => (ms >= 100 ? ms.toFixed(0) : ms.toFixed(1));

function clockTime(ts: number) {
    const d = new Date(ts);
    const pad = (n: number, l = 2) => String(n).padStart(l, '0');
    return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}

function callTime(start: number) {
    return clockTime(performance.timeOrigin + start);
}

const sourceLabel: Record<DevError['source'], string> = {
    invoke: 'invoke',
    error: 'error',
    promise: 'promise',
    console: 'console',
};

function clearCurrent() {
    if (tab.value === 'errors') clearDevErrors();
    else clearDevCalls();
    sample();
}

async function openDataDir() {
    try {
        await invoke('dev_open_data_dir');
    } catch (e) {
        console.error('dev_open_data_dir:', e);
    }
}

async function openDevtools() {
    try {
        await invoke('dev_open_devtools');
    } catch (e) {
        console.error('dev_open_devtools:', e);
    }
}

async function reloadUi() {
    await saveState();
    sessionStorage.setItem('lfmusic_reloaded', '1');
    location.reload();
}

async function copyLog() {
    const lines = [
        `LFMusic ${info.value?.version ?? ''} ${info.value?.os ?? ''} ${info.value?.arch ?? ''}`.trim(),
        '',
        '# Errors',
        ...devErrors.map(e => `${clockTime(e.time)} [${e.source}] ${e.message}`),
        '',
        '# Calls',
        ...devCalls.map(c => `${callTime(c.start)} ${c.cmd} ${c.ms === null ? 'pending' : `${formatMs(c.ms)}ms`}${c.error ? ` ERROR ${c.error}` : ''} ${c.args}`),
    ];
    try {
        await navigator.clipboard.writeText(lines.join('\n'));
        copied.value = true;
        setTimeout(() => { copied.value = false; }, 1400);
    } catch {
        // Clipboard unavailable.
    }
}

function onKeydown(e: KeyboardEvent) {
    if (e.key === 'F12' || (e.ctrlKey && e.shiftKey && e.code === 'KeyI')) {
        e.preventDefault();
        openDevtools();
        return;
    }
    if (open.value && e.key === 'Escape') {
        e.preventDefault();
        open.value = false;
    }
}

onMounted(() => {
    startPolling(2000);
    document.addEventListener('keydown', onKeydown);
});
onBeforeUnmount(() => {
    if (pollTimer) clearInterval(pollTimer);
    if (fpsRaf !== null) cancelAnimationFrame(fpsRaf);
    document.removeEventListener('keydown', onKeydown);
});

const tabs = computed<{ id: Tab; label: string; count?: number }[]>(() => [
    { id: 'overview', label: t('dev.overview') },
    { id: 'calls', label: t('dev.calls'), count: devStats.size || undefined },
    { id: 'errors', label: t('dev.errors'), count: errorCount.value || undefined },
]);

const overviewRows = computed(() => {
    const i = info.value;
    return [
        { label: t('dev.version'), value: i?.version ?? '-' },
        { label: t('dev.build'), value: i ? (i.debug_build ? 'debug' : 'release') : '-' },
        { label: t('dev.platform'), value: i ? `${i.os} ${i.arch}` : '-' },
        { label: t('dev.viewport'), value: viewport.value },
        { label: t('dev.uiScale'), value: (void tick.value, currentUiScale().toFixed(2)) },
        { label: t('dev.fps'), value: String(fps.value) },
        { label: t('dev.memory'), value: memoryMb.value === null ? '-' : `${memoryMb.value.toFixed(1)} MB` },
        { label: t('dev.domNodes'), value: String(domNodes.value) },
        { label: t('dev.uptime'), value: uptime.value },
    ];
});
</script>

<template>
    <Teleport to="body">
        <button
            type="button"
            class="devToggle"
            :class="{ active: open }"
            :aria-label="t('dev.title')"
            v-tooltip="open ? undefined : t('dev.title')"
            @click="open = !open"
        >
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="m8 7-5 5 5 5M16 7l5 5-5 5M13.5 4.5l-3 15" />
            </svg>
            <span v-if="errorCount > 0 && !open" class="devBadge">{{ errorCount > 99 ? '99+' : errorCount }}</span>
        </button>

        <Transition name="devPanel">
            <div v-if="open" class="devPanel" role="dialog" :aria-label="t('dev.title')">
                <div class="devHead">
                    <div class="devTitle">{{ t('dev.title') }}</div>
                    <button type="button" class="devClose" aria-label="Закрыть" @click="open = false">
                        <CloseIcon width="16" height="16" />
                    </button>
                </div>

                <div class="devTabs">
                    <button
                        v-for="item in tabs"
                        :key="item.id"
                        type="button"
                        class="devTab"
                        :class="{ active: tab === item.id }"
                        @click="tab = item.id"
                    >
                        {{ item.label }}
                        <span v-if="item.count" class="devTabCount">{{ item.count }}</span>
                    </button>
                </div>

                <div v-if="tab === 'overview'" class="devBody">
                    <div class="devGrid">
                        <div v-for="row in overviewRows" :key="row.label" class="devCell">
                            <div class="devLabel">{{ row.label }}</div>
                            <div class="devValue">{{ row.value }}</div>
                        </div>
                    </div>
                    <div class="devCell devWide">
                        <div class="devLabel">{{ t('dev.dataDir') }}</div>
                        <div class="devValue devPath" v-truncate>{{ info?.data_dir ?? '-' }}</div>
                    </div>
                    <div class="devActions">
                        <button type="button" class="devBtn" @click="openDevtools">DevTools</button>
                        <button type="button" class="devBtn" @click="openDataDir">{{ t('dev.openDataDir') }}</button>
                        <button type="button" class="devBtn" @click="reloadUi">{{ t('dev.reloadUi') }}</button>
                        <button type="button" class="devBtn" @click="copyLog">{{ copied ? t('donate.copied') : t('dev.copyLog') }}</button>
                    </div>
                </div>

                <div v-else-if="tab === 'calls'" class="devBody">
                    <div class="devToolbar">
                        <input v-model="filter" class="devFilter" type="text" spellcheck="false" :placeholder="t('dev.filter')">
                        <button type="button" class="devBtn" @click="clearCurrent">{{ t('dev.clear') }}</button>
                    </div>
                    <div v-if="stats.length === 0" class="devEmpty">{{ t('dev.noCalls') }}</div>
                    <template v-else>
                        <div class="devStatRow devStatHead">
                            <span>cmd</span><span>n</span><span>avg</span><span>max</span><span>err</span>
                        </div>
                        <div v-for="s in stats" :key="s.cmd" class="devStatRow">
                            <span class="devCmd" v-truncate>{{ s.cmd }}</span>
                            <span>{{ s.count }}</span>
                            <span>{{ formatMs(s.avgMs) }}</span>
                            <span>{{ formatMs(s.maxMs) }}</span>
                            <span :class="{ devBad: s.errors > 0 }">{{ s.errors }}</span>
                        </div>
                        <div class="devSection">{{ t('dev.recent') }}</div>
                        <div v-for="c in recent" :key="c.id" class="devCall">
                            <span class="devTime">{{ callTime(c.start) }}</span>
                            <span class="devCmd" v-truncate>{{ c.cmd }}</span>
                            <span class="devMs" :class="{ devBad: c.error }">{{ c.ms === null ? t('dev.pending') : `${formatMs(c.ms)} ms` }}</span>
                            <span v-if="c.args" class="devArgs">{{ c.args }}</span>
                            <span v-if="c.error" class="devArgs devBad">{{ c.error }}</span>
                        </div>
                    </template>
                </div>

                <div v-else class="devBody">
                    <div class="devToolbar devToolbarEnd">
                        <button type="button" class="devBtn" @click="clearCurrent">{{ t('dev.clear') }}</button>
                    </div>
                    <div v-if="errors.length === 0" class="devEmpty">{{ t('dev.noErrors') }}</div>
                    <div v-for="(e, i) in errors" :key="`${e.time}-${i}`" class="devError">
                        <div class="devErrorHead">
                            <span class="devTime">{{ clockTime(e.time) }}</span>
                            <span class="devSource">{{ sourceLabel[e.source] }}</span>
                        </div>
                        <div class="devErrorText">{{ e.message }}</div>
                    </div>
                </div>
            </div>
        </Transition>
    </Teleport>
</template>

<style scoped>
.devToggle {
    position: fixed;
    right: 18px;
    bottom: 118px;
    z-index: 100010;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 19%);
    color: hsla(0, 0%, 100%, 0.78);
    box-shadow: 0 10px 28px hsla(0, 0%, 0%, 0.35);
    cursor: pointer;
    transition: background-color .18s ease, color .18s ease, transform .18s ease;
}
.devToggle:hover,
.devToggle.active {
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 25%);
    color: #fff;
}
.devToggle:active {
    transform: scale(0.94);
}
.devBadge {
    position: absolute;
    top: -3px;
    right: -3px;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 9px;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsl(0, 70%, 56%);
    color: #fff;
    font-size: 10px;
    font-weight: 800;
    font-variant-numeric: tabular-nums;
}

.devPanel {
    position: fixed;
    right: 18px;
    bottom: 172px;
    z-index: 100010;
    width: 460px;
    max-width: calc(100vw / var(--overlay-scale, 1) - 36px);
    max-height: min(560px, calc(100vh / var(--overlay-scale, 1) - 220px));
    display: flex;
    flex-direction: column;
    padding: 18px 18px 0;
    border-radius: 24px;
    background-color: var(--light-bg);
    box-shadow: 0 24px 70px hsla(0, 0%, 0%, 0.5);
    box-sizing: border-box;
    color: #fff;
    user-select: text;
}
.devHead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
}
.devTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 18px;
    font-weight: 800;
    letter-spacing: -0.01em;
    user-select: none;
}
.devClose {
    width: 30px;
    height: 30px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    background: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    cursor: pointer;
    transition: background-color .18s ease, color .18s ease;
}
.devClose:hover {
    background: hsla(0, 0%, 100%, 0.12);
    color: #fff;
}

.devTabs {
    display: flex;
    gap: 4px;
    margin-top: 14px;
    user-select: none;
}
.devTab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px;
    border-radius: 15px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.55);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color .18s ease, color .18s ease;
}
.devTab:hover {
    color: #fff;
}
.devTab.active {
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 19%);
    color: #fff;
}
.devTabCount {
    font-size: 11px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: hsla(var(--accent-h), min(var(--accent-s), 60%), 78%, 0.9);
}

.devBody {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 14px -18px 0;
    padding: 0 18px 18px;
}
.devBody::-webkit-scrollbar { width: 6px; }
.devBody::-webkit-scrollbar-thumb { background: hsla(0, 0%, 100%, 0.14); border-radius: 3px; }

.devGrid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 14px 12px;
}
.devCell {
    min-width: 0;
}
.devWide {
    margin-top: 18px;
}
.devLabel,
.devSection {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: .1em;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.42);
    user-select: none;
}
.devSection {
    margin: 18px 0 8px;
}
.devValue {
    margin-top: 4px;
    font-size: 15px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.devPath {
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.75);
}
.devActions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 20px;
}
.devBtn {
    flex-shrink: 0;
    height: 32px;
    padding: 0 14px;
    border-radius: 16px;
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.16);
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.6), 85%, 1);
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
    user-select: none;
    transition: background-color .18s ease, color .18s ease;
}
.devBtn:hover {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 0.28);
    color: #fff;
}

.devToolbar {
    display: flex;
    gap: 8px;
    margin-bottom: 12px;
}
.devToolbarEnd {
    justify-content: flex-end;
}
.devFilter {
    flex: 1;
    min-width: 0;
    height: 32px;
    padding: 0 14px;
    border-radius: 16px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: #fff;
    font-size: 13px;
    font-weight: 500;
    outline: none;
}
.devFilter::placeholder {
    color: hsla(0, 0%, 100%, 0.35);
}
.devEmpty {
    padding: 24px 0;
    text-align: center;
    font-size: 13px;
    color: hsla(0, 0%, 100%, 0.4);
}

.devStatRow {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 44px 52px 52px 32px;
    gap: 8px;
    padding: 4px 0;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.75);
}
.devStatRow > span:not(:first-child) {
    text-align: right;
}
.devStatHead {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: .08em;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.4);
    user-select: none;
}
.devCmd {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: Consolas, "Cascadia Mono", Menlo, monospace;
    color: #fff;
}
.devCall {
    display: grid;
    grid-template-columns: 92px minmax(0, 1fr) auto;
    column-gap: 10px;
    padding: 5px 0;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
}
.devTime {
    color: hsla(0, 0%, 100%, 0.4);
    font-variant-numeric: tabular-nums;
}
.devMs {
    color: hsla(var(--accent-h), min(var(--accent-s), 60%), 78%, 0.95);
}
.devArgs {
    grid-column: 2 / -1;
    margin-top: 2px;
    font-family: Consolas, "Cascadia Mono", Menlo, monospace;
    font-size: 11px;
    color: hsla(0, 0%, 100%, 0.45);
    word-break: break-all;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
}
.devBad {
    color: hsl(0, 75%, 68%) !important;
}

.devError {
    padding: 8px 0;
}
.devError + .devError {
    margin-top: 4px;
}
.devErrorHead {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
}
.devSource {
    padding: 1px 8px;
    border-radius: 8px;
    background-color: hsla(0, 70%, 56%, 0.16);
    color: hsl(0, 75%, 72%);
    font-size: 11px;
    font-weight: 700;
}
.devErrorText {
    margin-top: 4px;
    font-family: Consolas, "Cascadia Mono", Menlo, monospace;
    font-size: 11px;
    line-height: 1.5;
    color: hsla(0, 0%, 100%, 0.75);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 120px;
    overflow-y: auto;
}

.devPanel-enter-active,
.devPanel-leave-active {
    transition: opacity .18s ease, transform .18s ease;
    transform-origin: bottom right;
}
.devPanel-enter-from,
.devPanel-leave-to {
    opacity: 0;
    transform: translateY(6px) scale(0.98);
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .devToggle {
        right: 12px;
        bottom: calc(var(--mobile-player-height) + var(--mobile-nav-height) + 12px);
        width: 40px;
        height: 40px;
    }
    .devPanel {
        right: 12px;
        left: 12px;
        width: auto;
        max-width: none;
        bottom: calc(var(--mobile-player-height) + var(--mobile-nav-height) + 62px);
        max-height: min(520px, calc(100dvh - var(--mobile-player-height) - var(--mobile-nav-height) - 90px));
    }
    .devGrid {
        grid-template-columns: repeat(2, minmax(0, 1fr));
    }
}
</style>
