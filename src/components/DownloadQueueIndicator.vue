<script setup lang="ts">
import { computed } from 'vue'

import CoverImage from './CoverImage.vue'
import defaultTrackCover from '../assets/trackCover.webp'

import { useDownloads } from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'

const { t } = useI18n()
const { queue, remove, retry, clearFinished } = useDownloads()

// Активные (в очереди + качаются) и завершённые (готово/ошибка) элементы.
const activeQueue = computed(() =>
    queue.value.filter((i) => i.status === 'queued' || i.status === 'downloading'),
)
const finishedQueue = computed(() =>
    queue.value.filter((i) => i.status === 'done' || i.status === 'error'),
)
// Сначала активные, затем завершённые — общий порядок для списка.
const allItems = computed(() => [...activeQueue.value, ...finishedQueue.value])
// Сколько треков сейчас в работе — цифра в кружке-бейдже.
const activeCount = computed(() => activeQueue.value.length)
// Бейдж не вмещает большие числа — сжимаем «99+».
const badgeText = computed(() => (activeCount.value > 99 ? '99+' : String(activeCount.value)))
// Индикатор показываем, пока в очереди есть хоть что-то (активное или
// завершённое, которое пользователь ещё не очистил).
const hasQueue = computed(() => queue.value.length > 0)
const hasFinished = computed(() => finishedQueue.value.length > 0)

// Подсказка кнопки справа: активное — «Отменить», завершённое — «Убрать».
function removeTooltip(status: string): string {
    return status === 'queued' || status === 'downloading'
        ? t('downloads.cancel')
        : t('downloads.remove')
}

async function onRemove(id: number) {
    try {
        await remove(id)
    } catch (e) {
        console.error('Ошибка удаления из очереди:', e)
    }
}

async function onRetry(id: number) {
    try {
        await retry(id)
    } catch (e) {
        console.error('Ошибка повтора скачивания:', e)
    }
}

async function onClear() {
    try {
        await clearFinished()
    } catch (e) {
        console.error('Ошибка очистки очереди:', e)
    }
}
</script>

<template>
    <div v-if="hasQueue" class="dlIndicator">
        <button
            class="dlIndicatorBtn"
            :class="{ active: activeCount > 0 }"
            :aria-label="t('downloads.queueTitle')"
        >
            <!-- Иконка скачивания: нижняя «полка» статична, стрелка «парит». -->
            <svg
                class="dlDownloadIcon"
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
            >
                <path class="dlArrow" d="M12 4v11m0 0 4-4m-4 4-4-4" />
                <path class="dlTray" d="M6 20h12" />
            </svg>
            <span v-if="activeCount > 0" class="dlBadge">{{ badgeText }}</span>
        </button>

        <!-- Выпадающий список очереди (по наведению) -->
        <div class="dlDropdown" role="tooltip">
            <div class="dlDropdownCard">
                <div class="dlDropdownHead">
                    <span class="dlDropdownTitle">{{ t('downloads.queueTitle') }}</span>
                    <button v-if="hasFinished" class="dlClearBtn" @click="onClear">
                        {{ t('downloads.clear') }}
                    </button>
                </div>

                <div class="dlList">
                    <div
                        v-for="item in allItems"
                        :key="item.id"
                        class="dlItem"
                        :class="{ error: item.status === 'error' }"
                    >
                        <div class="dlCover">
                            <CoverImage :src="item.cover_url" :fallback="defaultTrackCover" />
                        </div>
                        <div class="dlInfo">
                            <div v-truncate class="dlName">{{ item.title }}</div>
                            <div v-truncate class="dlArtist">{{ item.artist }}</div>
                        </div>

                        <!-- При ошибке: кнопка «Попробовать снова» слева от «Убрать»
                             (тот же круг-стиль, что у статус-кнопки). -->
                        <button
                            v-if="item.status === 'error'"
                            class="dlStatusBtn dlRetryBtn"
                            @click="onRetry(item.id)"
                            v-tooltip="t('downloads.retry')"
                        >
                            <svg class="dlRetryGlyph" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                                <path d="M19.08 8A8 8 0 1 0 20 12"/>
                                <polyline points="20 4 20 8 16 8"/>
                            </svg>
                        </button>

                        <!-- Круг-статус: спинер (качается) → зелёная галочка (готово)
                             → красный крест (ошибка); при наведении — крест (отмена). -->
                        <button
                            class="dlStatusBtn"
                            :class="{ done: item.status === 'done', error: item.status === 'error' }"
                            @click="onRemove(item.id)"
                            v-tooltip="removeTooltip(item.status)"
                        >
                            <span v-if="item.status === 'downloading'" class="dlSpinner"></span>
                            <span v-else-if="item.status === 'queued'" class="dlSpinner queued"></span>
                            <svg v-else-if="item.status === 'done'" class="dlStatusGlyph dlStatusCheck" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round">
                                <polyline points="20 6 9 17 4 12"/>
                            </svg>
                            <svg class="dlStatusGlyph dlStatusCross" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round">
                                <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                            </svg>
                        </button>

                        <!-- Только при ошибке: блок трека затемнён, поверх — красный текст.
                             v-truncate: при обрезании (2 строки) показывает полный текст в tooltip. -->
                        <div v-if="item.status === 'error'" class="dlErrorOverlay">
                            <span v-truncate class="dlErrorText">{{ item.error || t('downloads.finishedErrorFallback') }}</span>
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
    height: 25px;
    margin-right: 10px;
    display: flex;
    align-items: center;
}

/* ── Круглая кнопка-иконка с бейджем ─────────────────────────────────────── */
.dlIndicatorBtn {
    position: relative;
    cursor: pointer;
    width: 25px;
    height: 25px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: hsla(0, 0%, 100%, 0.78);
    background: linear-gradient(172deg, hsla(var(--accent-h), var(--accent-s), 25%, 1), hsla(var(--accent-h), var(--accent-s), 15%, 1));
    transition: color .2s ease, background-color .2s ease;
}
.dlIndicatorBtn:hover {
    color: #fff;
}
/* Стрелка «парит» во время скачивания; нижняя полка остаётся статичной. */
.dlDownloadIcon .dlArrow {
    transform-box: fill-box;
    transform-origin: center;
}
.dlIndicatorBtn.active .dlArrow {
    animation: dlArrowFloat 1.4s ease-in-out infinite;
}
@keyframes dlArrowFloat {
    0%, 100% { transform: translateY(-1px); }
    50% { transform: translateY(2px); }
}
.dlBadge {
    position: absolute;
    top: -4px;
    right: -4px;
    min-width: 13px;
    height: 13px;
    padding: 0 3px;
    box-sizing: border-box;
    border-radius: 999px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 7px;
    line-height: 1;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: #fff;
    background: hsla(var(--accent-h), min(calc(var(--accent-s) + 16%), 100%), 56%, 1);
    border: 1.5px solid var(--dark-bg);
    user-select: none;
}

/* ── Выпадающий список ───────────────────────────────────────────────────── */
.dlDropdown {
    position: absolute;
    top: 100%;
    right: 0;
    /* Прозрачный «мостик» сверху: курсор переходит с кнопки на карточку, не
       теряя :hover (gap не разрывает наведение, т.к. он внутри элемента). */
    padding-top: 9px;
    width: 320px;
    opacity: 0;
    pointer-events: none;
    transform: translateY(-3px);
    transition: opacity .16s ease, transform .16s ease;
    z-index: 20;
}
.dlIndicator:hover .dlDropdown {
    opacity: 1;
    pointer-events: auto;
    transform: translateY(0);
}
.dlDropdownCard {
    padding: 10px 8px 8px;
    border-radius: 14px;
    background: hsla(var(--accent-h), min(var(--accent-s), 42%), 10%, 0.98);
    border: 1px solid hsla(0, 0%, 100%, 0.1);
    box-shadow: 0 14px 38px rgba(0, 0, 0, 0.36);
}
.dlDropdownHead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 0 6px 8px;
}
.dlDropdownTitle {
    font-size: 12px;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.95);
    user-select: none;
}
.dlClearBtn {
    cursor: pointer;
    flex-shrink: 0;
    padding: 4px 10px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.6);
    font-size: 11px;
    font-weight: 600;
    transition: .2s ease;
}
.dlClearBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
    color: hsla(0, 0%, 100%, 0.9);
}
.dlList {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: min(360px, calc(80vh / var(--ui-scale, 1)));
    overflow-y: auto;
}
.dlList::-webkit-scrollbar { width: 5px; }
.dlList::-webkit-scrollbar-thumb {
    background: hsla(0, 0%, 100%, 0.14);
    border-radius: 3px;
}

/* ── Элемент очереди ─────────────────────────────────────────────────────── */
.dlItem {
    position: relative;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 10px;
    padding: 7px 6px;
    border-radius: 9px;
    transition: background-color .2s ease;
}
.dlItem:hover {
    background-color: hsla(0, 0%, 100%, 0.04);
}
.dlCover {
    width: 38px;
    height: 38px;
    border-radius: 6px;
    overflow: hidden;
    flex-shrink: 0;
    background-color: #202020;
    transition: opacity .2s ease;
}
.dlInfo {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    transition: opacity .2s ease;
}
.dlName {
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.dlArtist {
    font-size: 11px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.4);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

/* ── Круг-статус справа (спинер / галочка / крест) ───────────────────────── */
.dlStatusBtn {
    position: relative;
    z-index: 2;
    cursor: pointer;
    flex-shrink: 0;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: hsla(0, 0%, 100%, 0.55);
    /* Тёмный кружок (а не белый): чуть темнее карточки дропдауна. */
    background-color: hsla(var(--accent-h), min(var(--accent-s), 42%), 5%, 0.55);
    transition: background-color .2s ease;
}
.dlStatusBtn:hover {
    background-color: hsla(var(--accent-h), min(var(--accent-s), 42%), 3%, 0.8);
}
/* Кнопка «Попробовать снова» — тот же круг, что и «Убрать», с иконкой-стрелкой. */
.dlRetryBtn {
    color: hsla(0, 0%, 100%, 0.72);
}
.dlRetryBtn:hover {
    color: #fff;
}
.dlRetryGlyph {
    margin-left: -1px;
    display: block;
    width: 14px;
    height: 14px;
}
.dlSpinner {
    width: 15px;
    height: 15px;
    border-radius: 50%;
    box-sizing: border-box;
    border: 2px solid hsla(0, 0%, 100%, 0.14);
    border-top-color: hsla(var(--accent-h), min(calc(var(--accent-s) + 16%), 100%), 68%, 0.95);
    animation: dlSpin 0.8s linear infinite;
    transition: opacity .15s ease;
}
.dlSpinner.queued {
    border-color: hsla(0, 0%, 100%, 0.1);
    border-top-color: hsla(0, 0%, 100%, 0.34);
    animation-duration: 1.7s;
}
@keyframes dlSpin {
    to { transform: rotate(360deg); }
}
.dlStatusGlyph {
    position: absolute;
    /* Центрируем через inset+margin:auto (а не translate(-50%,-50%)): под зумом
       WebView2 дробный translate даёт сдвиг на пиксель — крест казался съехавшим. */
    inset: 0;
    margin: auto;
    width: 15px;
    height: 15px;
    transition: opacity .15s ease;
}
/* Галочка «готово» — зелёная. */
.dlStatusCheck {
    color: hsla(145, 55%, 60%, 0.95);
}
/* Крест по умолчанию скрыт; проступает при наведении. */
.dlStatusCross {
    color: hsla(0, 0%, 100%, 0.85);
    opacity: 0;
}
.dlStatusBtn:hover .dlSpinner,
.dlStatusBtn:hover .dlStatusCheck {
    opacity: 0;
}
.dlStatusBtn:hover .dlStatusCross {
    opacity: 1;
}
/* Ошибка: всегда красный крест (спинер/галочка не отрисовываются). */
.dlStatusBtn.error .dlStatusCross {
    opacity: 1;
    color: hsla(2, 78%, 64%, 0.95);
}

/* ── Ошибка: затемняем содержимое и кладём поверх красный текст ───────────── */
.dlItem.error .dlCover,
.dlItem.error .dlInfo {
    opacity: 0.16;
}
.dlErrorOverlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    /* Справа оставляем место под обе кнопки (повтор + убрать). */
    padding: 0 76px 0 14px;
    pointer-events: none;
}
.dlErrorText {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    text-align: center;
    font-size: 11px;
    line-height: 1.3;
    font-weight: 600;
    color: hsla(2, 80%, 68%, 0.95);
    /* Оверлей не ловит события (клики проходят к кнопкам), но сам текст должен —
       чтобы v-truncate показал полный текст ошибки в tooltip при наведении. */
    pointer-events: auto;
}
</style>
