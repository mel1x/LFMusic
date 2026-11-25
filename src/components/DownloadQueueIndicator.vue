<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'

import CoverImage from './CoverImage.vue'
import defaultTrackCover from '../assets/trackCover.webp'

import { useDownloads, type DownloadItem } from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'

const { t } = useI18n()
const router = useRouter()
const { queue, activeCount, cancel, retry, clearFinished, cancelAll, stageText } = useDownloads()

function openDownloads(e?: MouseEvent) {
    ;(e?.currentTarget as HTMLElement | undefined)?.blur()
    void router.push('/downloads')
}

const STAGE_ORDER: Record<DownloadItem['stage'], number> = {
    downloading: 0, processing: 0, searching: 1, queued: 2, error: 3, done: 4,
}
const items = computed(() =>
    [...queue.value].sort((a, b) => STAGE_ORDER[a.stage] - STAGE_ORDER[b.stage] || b.id - a.id),
)
const failedCount = computed(() => queue.value.filter(i => i.stage === 'error').length)
const finishedCount = computed(() => queue.value.filter(i => i.stage === 'done' || i.stage === 'error').length)

const overall = computed(() => {
    const list = queue.value.filter(i => i.stage !== 'error')
    if (list.length === 0) return 0
    const sum = list.reduce((acc, i) => acc + i.progress, 0)
    return sum / list.length
})
const RING = 2 * Math.PI * 10.5
const ringOffset = computed(() => RING * (1 - overall.value))

const summary = computed(() => {
    const total = queue.value.filter(i => i.stage !== 'error').length
    const done = queue.value.filter(i => i.stage === 'done').length
    return t('downloads.queue.summary', { done, total })
})

function isActive(i: DownloadItem) {
    return i.stage !== 'done' && i.stage !== 'error'
}

async function run(fn: () => Promise<unknown>) {
    try { await fn() } catch (e) { console.error('Ошибка очереди загрузок:', e) }
}
</script>

<template>
    <div v-if="queue.length > 0" class="dlIndicator">
        <button type="button" class="dlButton" :class="{ busy: activeCount > 0, hasError: failedCount > 0 && activeCount === 0 }" :aria-label="t('downloads.queue.title')" @click="openDownloads($event)">
            <svg class="dlRing" viewBox="0 0 25 25">
                <circle class="dlRingTrack" cx="12.5" cy="12.5" r="10.5" />
                <circle class="dlRingFill" cx="12.5" cy="12.5" r="10.5" :stroke-dasharray="RING" :stroke-dashoffset="ringOffset" />
            </svg>
            <svg class="dlArrowIcon" width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round">
                <path class="dlArrow" d="M12 4v11m0 0 4-4m-4 4-4-4" />
                <path d="M6 20h12" />
            </svg>
        </button>

        <div class="dlPopover" role="dialog">
            <div class="dlCard">
                <div class="dlHead">
                    <div class="dlHeadText">
                        <button type="button" class="dlTitle" @click="openDownloads($event)">{{ t('downloads.queue.title') }}</button>
                        <span class="dlSummary">{{ summary }}</span>
                    </div>
                    <div class="dlHeadActions">
                        <button v-if="failedCount > 0" class="chip" @click="run(() => retry())">{{ t('downloads.queue.retryAll') }}</button>
                        <button v-if="finishedCount > 0" class="chip" @click="run(clearFinished)">{{ t('downloads.queue.clear') }}</button>
                        <button v-if="activeCount > 1" class="chip danger" @click="run(cancelAll)">{{ t('downloads.queue.cancelAll') }}</button>
                    </div>
                </div>

                <div class="dlList">
                    <div v-for="item in items" :key="item.id" class="dlItem" :class="item.stage">
                        <div class="dlCover">
                            <CoverImage :src="item.cover_url || item.thumbnail_url" :fallback="defaultTrackCover" />
                        </div>
                        <div class="dlInfo">
                            <div v-truncate class="dlName">{{ item.title }}</div>
                            <div v-truncate class="dlLine" :class="{ error: item.stage === 'error', done: item.stage === 'done' }">
                                <template v-if="item.stage === 'done'">{{ item.artist }}</template>
                                <template v-else>{{ stageText(item) }}</template>
                            </div>
                            <div v-if="isActive(item) && item.stage !== 'queued'" class="dlBar">
                                <span class="dlBarFill" :style="{ width: Math.round(item.progress * 100) + '%' }"></span>
                            </div>
                        </div>
                        <button
                            v-if="item.stage === 'error'"
                            class="iconBtn"
                            v-tooltip="t('downloads.action.retry')"
                            @click="run(() => retry([item.id]))"
                        >
                            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="M19.08 8A8 8 0 1 0 20 12"/><polyline points="20 4 20 8 16 8"/></svg>
                        </button>
                        <div class="dlEndAction">
                            <span v-if="item.stage === 'done'" class="doneMark">
                                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                            </span>
                            <button
                                class="iconBtn remove"
                                v-tooltip="isActive(item) ? t('downloads.action.cancel') : t('downloads.action.remove')"
                                @click="run(() => cancel(item.id))"
                            >
                                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    </div>
</template>

<style scoped>
.dlIndicator {
    position: relative;
    width: 34px;
    height: 34px;
    flex-shrink: 0;
    margin-right: 10px;
    display: flex;
    align-items: center;
}
.dlButton {
    position: relative;
    width: 100%;
    height: 100%;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.8);
    background: hsla(var(--accent-h), var(--accent-s), 22%, 1);
    transition: color var(--duration-hover) var(--ease-smooth);
}
.dlButton:hover { color: #fff; }
.dlRing {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    transform: rotate(-90deg);
}
.dlRingTrack { fill: none; stroke: hsla(0, 0%, 100%, 0.1); stroke-width: 1.25; }
.dlRingFill {
    fill: none;
    stroke: hsla(var(--accent-h), min(calc(var(--accent-s) + 20% * var(--accent-k, 1)), 100%), 70%, 1);
    stroke-width: 1.25;
    stroke-linecap: round;
    transition: stroke-dashoffset .4s var(--ease-smooth);
}
.dlButton.hasError .dlRingFill { stroke: hsla(2, 75%, 64%, 1); }
.dlArrow { transform-box: fill-box; transform-origin: center; }
.dlButton.busy .dlArrow { animation: arrowFloat 1.4s var(--ease-smooth-in-out) infinite; }
@keyframes arrowFloat {
    0%, 100% { transform: translateY(-1px); }
    50% { transform: translateY(2px); }
}
.dlPopover {
    position: absolute;
    top: 100%;
    right: -8px;
    padding-top: 10px;
    width: 424px;
    opacity: 0;
    pointer-events: none;
    transform: translateY(-4px);
    transition: opacity var(--duration-hover) var(--ease-smooth), transform var(--duration-hover) var(--ease-smooth);
    z-index: 30;
}
.dlIndicator:hover .dlPopover,
.dlIndicator:focus-within .dlPopover {
    opacity: 1;
    pointer-events: auto;
    transform: translateY(0);
}
.dlCard {
    padding: 14px 10px 10px;
    border-radius: 20px;
    background-color: var(--light-bg);
    box-shadow: 0 20px 56px rgba(0, 0, 0, 0.48);
}
.dlHead {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 8px;
    padding: 0 6px 10px;
}
.dlHeadText { display: flex; flex-direction: column; gap: 1px; }
.dlTitle {
    padding: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 14px;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.95);
    user-select: none;
    transition: color var(--duration-hover) var(--ease-smooth);
}
.dlTitle:hover { color: #fff; }
.dlSummary { font-size: 11px; font-weight: 600; color: hsla(0, 0%, 100%, 0.4); font-variant-numeric: tabular-nums; }
.dlHeadActions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 4px; }
.chip {
    cursor: pointer;
    padding: 4px 10px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.65);
    background-color: hsla(0, 0%, 100%, 0.06);
    transition: var(--duration-hover) var(--ease-smooth);
}
.chip:hover { color: #fff; background-color: hsla(0, 0%, 100%, 0.12); }
.chip.danger:hover { color: hsla(2, 85%, 78%, 1); background-color: hsla(2, 70%, 55%, 0.16); }

.dlList {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: min(400px, calc(75vh / var(--ui-scale, 1)));
    overflow-y: auto;
}
.dlList::-webkit-scrollbar { width: 5px; }
.dlList::-webkit-scrollbar-thumb { background: hsla(0, 0%, 100%, 0.14); border-radius: 3px; }

.dlItem {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 6px;
    border-radius: 10px;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.dlItem:hover { background-color: hsla(0, 0%, 100%, 0.04); }
.dlCover {
    width: 40px;
    height: 40px;
    border-radius: 7px;
    overflow: hidden;
    flex-shrink: 0;
    background-color: #202020;
}
.dlItem.error .dlCover { opacity: 0.5; }
.dlInfo { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.dlName {
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.92);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.dlLine {
    font-size: 11.5px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    color: hsla(var(--accent-h), min(calc(var(--accent-s) + 15% * var(--accent-k, 1)), 100%), 78%, 0.9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.dlLine.done { color: hsla(0, 0%, 100%, 0.42); }
.dlLine.error { color: hsla(2, 78%, 70%, 0.95); }
.dlBar {
    position: relative;
    height: 4px;
    margin-top: 3px;
    border-radius: 999px;
    overflow: hidden;
    background: hsla(0, 0%, 100%, 0.07);
}
.dlBarFill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 999px;
    background: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
    transition: width .3s var(--ease-smooth);
}
.iconBtn {
    flex-shrink: 0;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.6);
    background-color: hsla(0, 0%, 100%, 0.05);
    transition: var(--duration-hover) var(--ease-smooth);
}
.iconBtn:hover { color: #fff; background-color: hsla(0, 0%, 100%, 0.12); }
.dlEndAction {
    flex-shrink: 0;
    display: grid;
    place-items: center;
}
.dlEndAction > * { grid-area: 1 / 1; }
.iconBtn.remove { opacity: 0; }
.dlItem:hover .iconBtn.remove,
.dlEndAction:focus-within .iconBtn.remove,
.dlItem.error .iconBtn.remove { opacity: 1; }
.doneMark {
    display: flex;
    pointer-events: none;
    color: hsl(var(--accent-h), min(var(--accent-s), 70%), 68%);
}
.dlItem:hover .doneMark,
.dlEndAction:focus-within .doneMark { visibility: hidden; }
</style>
