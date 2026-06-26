<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import SearchIcon from '../components/Icons/SearchIcon.vue'
import DownloadIcon from '../components/Icons/DownloadIcon.vue'
import LoadingSpinner from '../components/LoadingSpinner.vue'
import ConfirmModal from '../components/ConfirmModal.vue'
import CoverImage from '../components/CoverImage.vue'

import defaultTrackCover from '../assets/trackCover.webp'

import { usePlayer } from '../composables/usePlayer'
import { useDownloads, type ExternalSearchTrack } from '../composables/useDownloads'
import { useI18n } from '../composables/useI18n'
import { useVirtualScroll } from '../composables/useVirtualScroll'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const { formatTime } = usePlayer()
const {
    queue,
    search,
    enqueue,
    refreshToolsInfo,
    checkToolUpdates,
    depsMissing,
    depsUpdateAvailable,
    ffmpegMissing,
    ytdlpMissing,
    denoMissing,
    cookiesAuthorized,
    trackKey,
} = useDownloads()

// ── Гейт: вход в YouTube необязателен. Скачивание/поиск блокирует ТОЛЬКО
// отсутствие обязательного yt-dlp. Cookies нужны лишь при «бот-стене» (см.
// botWallHit) — тогда показываем мягкую подсказку войти, но не блокируем. ──────
const downloadsLocked = computed(() => ytdlpMissing.value)
const searchDisabledText = computed(() => t('downloads.searchDisabledPlaceholder'))

// Был ли среди ошибок очереди признак «бот-стены» YouTube (маркер из downloads.rs
// `BOT_WALL_HINT`). Если да и пользователь ещё не входил — предложим авторизацию.
const botWallHit = computed(() =>
    queue.value.some(i => i.status === 'error' && !!i.error && i.error.includes('YouTube требует вход')),
)

// ── Search input (debounced, network) ───────────────────────────────────────
import { useTypeToSearch } from '../composables/useTypeToSearch'
const searchInput = ref<HTMLInputElement | null>(null)
useTypeToSearch(searchInput)
const rawSearch = ref('')
const debouncedSearch = ref('')
// Точные исполнитель/название из клика по «фантому» альбома: пока ввод не тронут,
// по ним идёт точный поиск по полям Deezer (иначе трек с «общим» названием, напр.
// «Интро», теряется в свободной выдаче). Любой ручной ввод сбрасывает — дальше
// обычный свободный поиск.
const exactMatch = ref<{ artist: string; title: string } | null>(null)
let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null

const focusSearch = () => { if (!downloadsLocked.value) searchInput.value?.focus() }

function onSearchInput(e: Event) {
    const value = (e.target as HTMLInputElement).value
    rawSearch.value = value
    exactMatch.value = null
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    searchDebounceTimer = setTimeout(() => {
        debouncedSearch.value = value.trim()
    }, 300)
}

function clearSearch() {
    rawSearch.value = ''
    debouncedSearch.value = ''
    exactMatch.value = null
    if (searchDebounceTimer) {
        clearTimeout(searchDebounceTimer)
        searchDebounceTimer = null
    }
    searchInput.value?.focus()
}

// Enter — мгновенный поиск, не дожидаясь дебаунса.
function onSearchEnter() {
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    debouncedSearch.value = rawSearch.value.trim()
}

const query = computed(() => debouncedSearch.value.trim())
const hasQuery = computed(() => !!query.value)

// ── Results (постранично, с подгрузкой при прокрутке) ────────────────────────
const PAGE_SIZE = 30
const results = ref<ExternalSearchTrack[]>([])
const searching = ref(false)     // первичная загрузка для нового запроса
const loadingMore = ref(false)   // подгрузка следующей страницы
const hasMore = ref(false)
let nextIndex = 0                 // смещение (Deezer index) для следующей страницы
let searchReqId = 0

// ── Виртуализация списка результатов ─────────────────────────────────────────
// Скролл-контейнер — сам pageRoot; над списком переменная по высоте «шапка»
// (поиск, баннер, очереди), поэтому смещение списка измеряем динамически и
// отдаём как scrollOffset (вычитается из scrollTop при расчёте видимого окна).
const resultsListEl = ref<HTMLElement | null>(null)
// Контейнер «шапки» (поиск, баннеры, очереди) — его высоту отслеживает
// ResizeObserver, чтобы точно пересчитывать смещение виртуального списка при
// любом изменении DOM-высоты шапки (без хрупкого watch по куче реактивных полей).
const headerEl = ref<HTMLElement | null>(null)
const resultsOffset = ref(0)
const {
    containerEl: pageRootEl,
    onScroll: vsOnScroll,
    visibleItems: visibleResults,
    paddingTop: vsPadTop,
    paddingBottom: vsPadBottom,
    resetScroll: vsResetScroll,
} = useVirtualScroll(results, { itemHeight: 58, buffer: 6, scrollOffset: resultsOffset })

/** Пересчитать смещение списка результатов от верха скролл-контейнера. */
function measureResultsOffset() {
    const list = resultsListEl.value
    if (!list || !pageRootEl.value) { resultsOffset.value = 0; return }
    // offsetTop относительно pageRoot (position: relative) = высота «шапки» выше списка.
    resultsOffset.value = list.offsetTop
}

/** Склеить страницы, отбросив дубликаты по «исполнитель + название» — как между
 *  страницами, так и ВНУТРИ новой пачки (set пополняется по ходу фильтрации). */
function dedupeInto(base: ExternalSearchTrack[], extra: ExternalSearchTrack[]): ExternalSearchTrack[] {
    const seen = new Set(base.map(t => trackKey(t.artist, t.title, t.album)))
    const fresh = extra.filter(t => {
        const k = trackKey(t.artist, t.title, t.album)
        if (seen.has(k)) return false
        seen.add(k)
        return true
    })
    return [...base, ...fresh]
}

watch(query, (q) => { void runSearch(q) })

async function runSearch(q: string) {
    const reqId = ++searchReqId
    results.value = []
    nextIndex = 0
    hasMore.value = false
    loadingMore.value = false
    // Новый запрос — возвращаем виртуальный список в начало, иначе при прокрутке
    // вниз старой выдачи новая открылась бы с середины или с пустым окном.
    vsResetScroll()
    if (!q) {
        searching.value = false
        return
    }
    searching.value = true
    try {
        const page = await search(q, 0, PAGE_SIZE, exactMatch.value ?? undefined)
        if (reqId !== searchReqId) return
        results.value = dedupeInto([], page.tracks)
        nextIndex = page.next_index
        hasMore.value = page.has_more
    } catch (e) {
        console.error('Ошибка поиска треков:', e)
        if (reqId === searchReqId) results.value = []
    } finally {
        if (reqId === searchReqId) searching.value = false
    }
}

async function loadMore() {
    if (loadingMore.value || searching.value || !hasMore.value || !query.value) return
    const reqId = searchReqId
    loadingMore.value = true
    try {
        const page = await search(query.value, nextIndex, PAGE_SIZE, exactMatch.value ?? undefined)
        if (reqId !== searchReqId) return
        results.value = dedupeInto(results.value, page.tracks)
        nextIndex = page.next_index
        hasMore.value = page.has_more
    } catch (e) {
        console.error('Ошибка подгрузки треков:', e)
    } finally {
        if (reqId === searchReqId) loadingMore.value = false
    }
}

function onScroll(e: Event) {
    vsOnScroll(e)
    const el = e.target as HTMLElement
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 360) void loadMore()
}

// Ключи треков, уже стоящих в очереди (исключая ошибочные — их можно перекачать).
const queuedKeys = computed(() => {
    const set = new Set<string>()
    for (const item of queue.value) {
        if (item.status !== 'error') set.add(trackKey(item.artist, item.title, item.album))
    }
    return set
})

function isQueued(t: ExternalSearchTrack): boolean {
    return queuedKeys.value.has(trackKey(t.artist, t.title, t.album))
}

async function onDownload(track: ExternalSearchTrack) {
    if (track.in_library || isQueued(track)) return
    if (downloadsLocked.value) return  // защита: гейт yt-dlp/cookies
    try {
        await enqueue(track)
    } catch (e) {
        console.error('Ошибка добавления в очередь:', e)
    }
}

// Нажатие на кнопку результата: уже в очереди → ничего (защита от дублей, даже
// если трек уже в библиотеке); в библиотеке → подтверждение повторного
// скачивания; иначе → обычное добавление.
function onResultButton(track: ExternalSearchTrack) {
    if (isQueued(track)) return
    if (track.in_library) {
        confirmTrack.value = track
        return
    }
    void onDownload(track)
}

// ── Повторное скачивание (модалка «уже в библиотеке») ────────────────────────
const confirmTrack = ref<ExternalSearchTrack | null>(null)
const confirmMessage = computed(() =>
    confirmTrack.value
        ? t('downloads.redownloadText', { title: confirmTrack.value.title })
        : '',
)

async function confirmRedownload() {
    const track = confirmTrack.value
    confirmTrack.value = null
    if (!track) return
    try {
        await enqueue(track, true)
    } catch (e) {
        console.error('Ошибка повторного скачивания:', e)
    }
}

// Когда трек докачался — помечаем совпадающий результат как «в библиотеке», чтобы
// после «Очистить» кнопка осталась «Скачано», а не снова «Скачать».
watch(queue, (items) => {
    const doneKeys = new Set(
        items.filter(i => i.status === 'done').map(i => trackKey(i.artist, i.title, i.album)),
    )
    if (doneKeys.size === 0) return
    for (const r of results.value) {
        if (!r.in_library && doneKeys.has(trackKey(r.artist, r.title, r.album))) r.in_library = true
    }
})

// Очередь скачивания переехала в шапку приложения (DownloadQueueIndicator) —
// она глобальна и видна с любой страницы. Здесь оставляем только `queue` для
// пометки результатов «уже в очереди»/«в библиотеке» (см. queuedKeys и watch).

// ── Tools status ──────────────────────────────────────────────────────────────
// Уведомление о компонентах. Приоритет: установить обязательный yt-dlp →
// выбрать браузер с валидными cookies (обязательно для скачивания) → обновить
// установленное → рекомендовать ffmpeg (необязательный, для качества).
// Кнопка ведёт на страницу настроек, где всё это настраивается.
type DepsBannerMode = 'install' | 'deno' | 'botwall' | 'update' | 'recommend' | 'none'
const depsBannerMode = computed<DepsBannerMode>(() => {
    if (depsMissing.value) return 'install'
    // yt-dlp есть, а Deno нет — YouTube-загрузки нестабильны; зовём доустановить.
    if (denoMissing.value) return 'deno'
    // Бот-стена и не входили — мягко предлагаем авторизацию (не блокируя).
    if (botWallHit.value && !cookiesAuthorized.value) return 'botwall'
    if (depsUpdateAvailable.value) return 'update'
    if (ffmpegMissing.value) return 'recommend'
    return 'none'
})
const showDepsBanner = computed(() => depsBannerMode.value !== 'none')

function goToToolsSettings() {
    router.push('/settings/external')
}

// Список монтируется, только когда есть результаты (иначе показан спиннер/пустое
// состояние). На его появление — один пересчёт смещения (ResizeObserver на шапке
// в этот момент не срабатывает, т.к. высота самой шапки не менялась).
watch(resultsListEl, (el) => { if (el) void nextTick(measureResultsOffset) })

// Любое изменение высоты шапки (баннер, очереди, перенос строк при ресайзе окна)
// триггерит точный пересчёт смещения — надёжнее ручного watch по набору полей.
let headerRO: ResizeObserver | null = null

onMounted(async () => {
    // Предзаполнение поиска из query-параметра (переход с фантомного трека альбома).
    const pickParam = (v: typeof route.query[string]) => (Array.isArray(v) ? v[0] : v) || ''
    const q = pickParam(route.query.q)
    const dlArtist = pickParam(route.query.dlArtist)
    const dlTitle = pickParam(route.query.dlTitle)
    if (q) {
        rawSearch.value = q
        // Точные поля задаём ДО debouncedSearch: watch(query) → runSearch прочитает
        // их и включит точный поиск по полям Deezer.
        if (dlArtist && dlTitle) exactMatch.value = { artist: dlArtist, title: dlTitle }
        debouncedSearch.value = q.trim()
    }
    // Локальный статус утилит (мгновенно, без сети) определяет, доступен ли
    // поиск. Дожидаемся его, прежде чем ставить фокус, — иначе фокус попадёт на
    // поле, которое тут же окажется заблокированным (yt-dlp отсутствует).
    try {
        await refreshToolsInfo()
    } catch { /* статус останется оптимистичным — фокусируемся как обычно */ }
    if (!downloadsLocked.value) {
        await nextTick()
        searchInput.value?.focus()
    }
    // Проверка обновлений (сеть) для баннера — не блокирует фокус.
    void checkToolUpdates().catch(() => { /* без сети просто без обновлений */ })
    await nextTick()
    measureResultsOffset()
    if (headerEl.value) {
        headerRO = new ResizeObserver(() => measureResultsOffset())
        headerRO.observe(headerEl.value)
    }
})

onUnmounted(() => {
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer)
    headerRO?.disconnect()
    headerRO = null
})
</script>

<template>
    <div class="pageRoot downloadsPage" ref="pageRootEl" @scroll="onScroll">
        <!-- «Шапка»: всё над списком результатов. Её высоту отслеживает ResizeObserver. -->
        <div class="downloadsHeader" ref="headerEl">
        <h1 class="pageTitle">{{ t('downloads.title') }}</h1>

        <div class="searchZone" :class="{ disabled: downloadsLocked }">
            <SearchIcon class="searchIcon" @click="focusSearch" />
            <input
                ref="searchInput"
                type="text"
                class="search"
                name="downloadSearch"
                id="downloadSearch"
                :placeholder="downloadsLocked ? searchDisabledText : t('downloads.searchPlaceholder')"
                autocomplete="off"
                :disabled="downloadsLocked"
                :value="rawSearch"
                @input="onSearchInput"
                @keydown.enter="onSearchEnter"
            />
            <div class="searchClear" v-if="rawSearch && !downloadsLocked" @click="clearSearch">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
            </div>
        </div>

        <!-- Dependencies banner -->
        <div v-if="showDepsBanner" class="toolsBanner" :class="{ soft: depsBannerMode === 'update' || depsBannerMode === 'recommend' }">
            <div class="toolsIcon">
                <DownloadIcon width="18" height="18" />
            </div>
            <div class="toolsInfo">
                <div class="toolsTitle">
                    <template v-if="depsBannerMode === 'install'">{{ t('downloads.depsInstallTitle') }}</template>
                    <template v-else-if="depsBannerMode === 'deno'">{{ t('downloads.depsDenoTitle') }}</template>
                    <template v-else-if="depsBannerMode === 'botwall'">{{ t('downloads.depsBotwallTitle') }}</template>
                    <template v-else-if="depsBannerMode === 'update'">{{ t('downloads.depsUpdateTitle') }}</template>
                    <template v-else>{{ t('downloads.depsRecommendTitle') }}</template>
                </div>
                <div class="toolsText">
                    <template v-if="depsBannerMode === 'install'">{{ t('downloads.depsInstallText') }}</template>
                    <template v-else-if="depsBannerMode === 'deno'">{{ t('downloads.depsDenoText') }}</template>
                    <template v-else-if="depsBannerMode === 'botwall'">{{ t('downloads.depsBotwallText') }}</template>
                    <template v-else-if="depsBannerMode === 'update'">{{ t('downloads.depsUpdateText') }}</template>
                    <template v-else>{{ t('downloads.depsRecommendText') }}</template>
                </div>
            </div>
            <button class="toolsBtn" @click="goToToolsSettings">{{ t('downloads.depsOpenSettings') }}</button>
        </div>

        </div>
        <!-- /шапка -->

        <!-- Results -->
        <section v-if="hasQuery" class="pageLine resultsLine">
            <div class="lineHat">
                <div class="lineTitle nonClickable">{{ t('downloads.resultsTitle') }}</div>
            </div>

            <LoadingSpinner v-if="searching && results.length === 0" />

            <div v-else-if="results.length === 0" class="emptyState small">
                <div class="emptyIcon"><SearchIcon width="16" height="16" /></div>
                <div class="emptyText">{{ t('downloads.emptyNothingTitle') }}</div>
                <div class="emptyDesc">{{ t('downloads.emptyNothingDesc') }}</div>
            </div>

            <div v-else class="tracksList" ref="resultsListEl">
                <div class="vsPad" :style="{ height: vsPadTop + 'px' }"></div>
                <div v-for="track in visibleResults" :key="trackKey(track.artist, track.title, track.album)" class="trackElem">
                    <div class="trackCover">
                        <CoverImage :src="track.cover_url" :fallback="defaultTrackCover" />
                    </div>
                    <div class="trackInfo">
                        <div v-truncate class="trackName">{{ track.title }}</div>
                        <div v-truncate class="trackArtist">{{ track.artist }}<template v-if="track.album"> · {{ track.album }}</template></div>
                    </div>
                    <div class="trackTime">{{ track.duration_secs ? formatTime(track.duration_secs) : '' }}</div>
                    <button
                        class="dlBtn"
                        :class="{ done: track.in_library && !isQueued(track), added: isQueued(track) }"
                        :disabled="isQueued(track)"
                        @click="onResultButton(track)"
                    >
                        <template v-if="isQueued(track)">
                            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                            <span>{{ t('downloads.btnAdded') }}</span>
                        </template>
                        <template v-else-if="track.in_library">
                            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                            <span>{{ t('downloads.btnDownloaded') }}</span>
                        </template>
                        <template v-else>
                            <DownloadIcon width="16" height="16" />
                            <span>{{ t('downloads.btnDownload') }}</span>
                        </template>
                    </button>
                </div>
                <div class="vsPad" :style="{ height: vsPadBottom + 'px' }"></div>
            </div>

            <div v-if="loadingMore" class="loadMoreRow"><div class="miniSpinner"></div></div>
        </section>

        <!-- Idle empty state -->
        <div v-else class="emptyState">
            <div class="emptyIcon"><DownloadIcon width="22" height="22" /></div>
            <div class="emptyText">{{ t('downloads.idleTitle') }}</div>
            <div class="emptyDesc">{{ t('downloads.idleDesc') }}</div>
        </div>

        <ConfirmModal
            :open="confirmTrack !== null"
            :title="t('downloads.redownloadTitle')"
            :message="confirmMessage"
            :cancel-text="t('downloads.redownloadCancel')"
            :confirm-text="t('downloads.redownloadConfirm')"
            @cancel="confirmTrack = null"
            @confirm="confirmRedownload"
        />
    </div>
</template>

<style scoped>
.pageRoot {
    position: relative;
    padding: 20px 28px 28px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: var(--light-bg);
    background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
    background-repeat: no-repeat;
    background-attachment: local;
    overflow-y: auto;
}
.pageRoot::-webkit-scrollbar { width: 6px; }
.pageRoot::-webkit-scrollbar-track { background-color: var(--light-bg); }
.pageRoot::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
}
/* Контейнер «шапки»: тот же flex-column, что и pageRoot, чтобы отступы детей
   (поиск/баннер/очереди) остались прежними. Сам по себе не сжимается. */
.downloadsHeader {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    width: 100%;
}

/* ── Search input (mirrors Search.vue) ───────────────────────────── */
.searchZone {
    position: relative;
    width: 100%;
    flex-shrink: 0;
    margin-bottom: 16px;
}
.searchIcon {
    cursor: text;
    position: absolute;
    top: 50%;
    left: 14px;
    transform: translateY(-50%);
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.4);
    transition: .24s ease;
}
.searchZone:hover .searchIcon {
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.8);
}
.searchClear {
    position: absolute;
    height: max-content;
    width: max-content;
    top: 50%;
    right: 11px;
    transform: translateY(-50%);
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.3);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 6px;
    border-radius: 50%;
    transition: .24s ease;
}
.searchClear:hover {
    color: hsla(0, 0%, 100%, 0.6);
}
.search {
    width: 100%;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.04);
    padding: 13px 38px 14px 42px;
    font-size: 14px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.9);
}
.search::placeholder {
    user-select: none;
    color: hsla(0, 0%, 100%, 0.3);
    font-size: 14px;
    font-weight: 600;
}
/* Без yt-dlp поиск недоступен — приглушаем зону и блокируем ввод. */
.searchZone.disabled {
    opacity: 0.55;
}
.searchZone.disabled .searchIcon {
    cursor: default;
}
.search:disabled {
    cursor: not-allowed;
}

/* ── Tools banner ────────────────────────────────────────────────── */
.toolsBanner {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 14px;
    padding: 14px 16px;
    margin-bottom: 24px;
    border-radius: 999px;
    background: linear-gradient(172deg, hsla(var(--accent-h), var(--accent-s), 22%, 0.6), hsla(var(--accent-h), var(--accent-s), 14%, 0.6));
    flex-shrink: 0;
}
/* Не хватает только ffmpeg — некритичная подсказка, оформляем нейтрально. */
.toolsBanner.soft {
    background: hsla(0, 0%, 100%, 0.04);
}
.toolsIcon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 38px;
    height: 38px;
    border-radius: 24px;
    background-color: hsla(0, 0%, 100%, 0.07);
    color: hsla(var(--accent-h), var(--accent-s), 80%, 0.95);
    flex-shrink: 0;
}
.miniSpinner {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid hsla(0, 0%, 100%, 0.15);
    border-top-color: hsla(var(--accent-h), var(--accent-s), 70%, 0.95);
    animation: spin 0.8s linear infinite;
}
@keyframes spin {
    to { transform: rotate(360deg); }
}
.toolsInfo {
    flex: 1;
    min-width: 0;
}
.toolsTitle {
    font-size: 13px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.92);
}
.toolsText {
    margin-top: 3px;
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.5);
    line-height: 1.35;
}
.toolsBtn {
    cursor: pointer;
    flex-shrink: 0;
    padding: 9px 18px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.95);
    color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
    font-size: 13px;
    font-weight: 700;
    transition: .2s ease;
}
.toolsBtn:hover {
    background-color: #fff;
    transform: translateY(-1px);
}

/* ── Sections ────────────────────────────────────────────────────── */
.pageLine {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 13px;
    margin-bottom: 32px;
}
.resultsLine {
    margin-bottom: 0;
}
.lineHat {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
}
.lineTitle {
    user-select: none;
    color: hsla(0, 0%, 100%, 0.85);
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 22px;
}
.lineTitle.nonClickable { cursor: default; }

/* ── Results track list ──────────────────────────────────────────── */
.tracksList {
    display: flex;
    flex-direction: column;
}
/* Распорки виртуального скролла (резервируют высоту вне видимого окна). */
.vsPad {
    width: 100%;
    flex-shrink: 0;
}
.trackElem {
    position: relative;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 8px;
    border-radius: 9px;
    height: 58px;
    box-sizing: border-box;
    transition: background-color .24s ease;
}
.trackElem:hover {
    background-color: hsla(0, 0%, 100%, 0.03);
}
.trackCover {
    width: 42px;
    height: 42px;
    background-color: #202020;
    border-radius: 5px;
    overflow: hidden;
    flex-shrink: 0;
}
.trackCoverImg {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    user-select: none;
}
.trackInfo {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
}
.trackName {
    user-select: none;
    font-size: 14px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.trackArtist {
    user-select: none;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.4);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.trackTime {
    user-select: none;
    font-size: 14px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.4);
    flex-shrink: 0;
}
.dlBtn {
    cursor: pointer;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 34px;
    padding: 0 16px;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 700;
    white-space: nowrap;
    color: hsla(var(--accent-h), min(calc(var(--accent-s) + 12%), 100%), 78%, 1);
    background-color: hsla(var(--accent-h), var(--accent-s), 55%, 0.16);
    transition: background-color .2s ease, color .2s ease, transform .2s ease;
}
.dlBtn svg {
    flex-shrink: 0;
}
.dlBtn:hover {
    background-color: hsla(var(--accent-h), var(--accent-s), 55%, 0.26);
    color: hsla(var(--accent-h), min(calc(var(--accent-s) + 12%), 100%), 86%, 1);
}
/* Уже в очереди — нейтральная «таблетка». */
.dlBtn.added {
    cursor: default;
    color: hsla(0, 0%, 100%, 0.55);
    background-color: hsla(0, 0%, 100%, 0.06);
}
/* Уже в библиотеке — зелёная «таблетка» (кликабельна: повторное скачивание). */
.dlBtn.done {
    cursor: pointer;
    color: hsla(145, 50%, 64%, 0.95);
    background-color: hsla(145, 45%, 50%, 0.12);
}
.dlBtn.done:hover {
    background-color: hsla(145, 45%, 50%, 0.2);
}
/* Уже в очереди — неактивная «таблетка». */
.dlBtn.added:hover {
    background-color: hsla(0, 0%, 100%, 0.06);
}

.loadMoreRow {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 18px 0 6px;
}

/* ── Empty states ────────────────────────────────────────────────── */
.emptyState {
    flex: 1;
    display: flex;
    gap: 10px;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding-bottom: 40px;
}
.emptyState.small {
    flex: none;
    padding: 40px 0;
}
.emptyIcon {
    padding: 10px;
    border-radius: 9px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: #ffffff;
}
.emptyText {
    user-select: none;
    font-size: 14px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.7);
}
.emptyDesc {
    user-select: none;
    text-align: center;
    max-width: 340px;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.3);
    line-height: 1.4;
}

/* ── Title (desktop hides; phone shows, mirrors Search.vue) ───────── */
.pageTitle { display: none; }

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .pageTitle {
        display: block !important;
        margin-top: 12px;
        margin-bottom: 22px !important;
        font-size: 30px !important;
        line-height: 1.08 !important;
        font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
        font-weight: 800;
        color: hsla(0, 0%, 100%, 0.9);
    }
    .downloadsPage.pageRoot {
        padding: 0 16px 80px !important;
    }
    .searchZone {
        height: 48px !important;
        border-radius: 14px !important;
        margin-bottom: 16px !important;
    }
    .search {
        height: 100% !important;
        padding: 0 44px 0 42px !important;
        border-radius: 14px !important;
    }
}
</style>
