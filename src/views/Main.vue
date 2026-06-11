<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue';
import { useRouter } from 'vue-router';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import ShuffleIcon from '../components/Icons/ShuffleIcon.vue';
import TuneIcon from '../components/Icons/TuneIcon.vue';
import CloseIcon from '../components/Icons/CloseIcon.vue';
import CatalogIcon from '../components/Icons/CatalogIcon.vue';
import MoodIcon from '../components/Icons/MoodIcon.vue';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { moods, useMixSettings, type Familiarity } from '../composables/useMix';
import {
    getRecommendations,
    getTasteProfile,
    getSummaryStats,
    type RecommendationOptions,
    type TasteProfile,
    type SummaryStats,
} from '../composables/useStatistics';
import { formatTotalDuration, tracksCountText } from '../utils/format';

const router = useRouter();

const {
    trackList,
    currentTrack,
    isPlaying,
    shuffled,
    playbackOrder,
    currentTrackIndex,
    activePlaylistSource,
    loadCoversForTracks,
    playTrack,
    togglePlayPause,
    setPlaylistSource,
} = usePlayer();

// ── Mix settings («Моя волна» style tuning) ──────────────────────────────────

const familiarityOptions: { id: Familiarity; label: string }[] = [
    { id: 'fresh', label: 'Незнакомое' },
    { id: 'balanced', label: 'Сбалансированно' },
    { id: 'favorite', label: 'Любимое' },
];

// Настройки микса — синглтон с сохранением в localStorage:
// переживают навигацию между страницами и перезапуск приложения.
const { selectedMood, familiarity, losslessOnly } = useMixSettings();
const settingsOpen = ref(false);
const isBuildingMix = ref(false);

const currentMood = computed(() => moods.find(m => m.id === selectedMood.value) ?? moods[0]);

const isMixActive = computed(() => activePlaylistSource.value === 'mix');
const isMixPlaying = computed(() => isMixActive.value && isPlaying.value);

const hasLibrary = computed(() => trackList.value.length > 0);

const mixOptions = computed<RecommendationOptions>(() => ({
    familiarity: familiarity.value,
    losslessOnly: losslessOnly.value || undefined,
}));

// ── Helpers ───────────────────────────────────────────────────────────────

/** Заменяет треки из БД на «живые» объекты из библиотеки (с обложками). */
function mergeWithLibrary(tracks: TrackMetadata[]): TrackMetadata[] {
    const map = new Map(trackList.value.map(t => [t.file_path, t]));
    return tracks.map(t => map.get(t.file_path) ?? t);
}

// ── Mix playback ────────────────────────────────────────────────────────────

async function buildMixTracks(mood: string): Promise<TrackMetadata[]> {
    let recs: TrackMetadata[] = [];
    try {
        recs = await getRecommendations(mood, 80, mixOptions.value);
    } catch {
        recs = [];
    }
    let merged = mergeWithLibrary(recs);
    // Если для выбранных настроек нет подходящих треков — берём всю библиотеку.
    if (merged.length === 0) {
        merged = [...trackList.value].sort(() => Math.random() - 0.5);
    }
    return merged;
}

/**
 * Запуск/обновление волны.
 * По умолчанию собирается полностью новая очередь и включается её первый трек —
 * перемешивание и смена настроения меняют в том числе текущий трек.
 * @param keepCurrent true — текущий трек продолжает играть, пересобирается
 *                    только хвост очереди (для смены состава волны/lossless).
 */
async function startMix(mood: string, keepCurrent = false) {
    if (!hasLibrary.value || isBuildingMix.value) return;
    isBuildingMix.value = true;
    try {
        const fresh = await buildMixTracks(mood);
        if (fresh.length === 0) return;

        // Детерминированная очередь волны (без глобального шаффла).
        shuffled.value = false;

        if (keepCurrent && isMixActive.value && currentTrack.value && currentTrackIndex.value >= 0) {
            // Мягкое обновление: текущий трек продолжает играть, хвост пересобирается.
            const keep = playbackOrder.value.slice(0, currentTrackIndex.value + 1);
            const keepPaths = new Set(keep.map(t => t.file_path));
            const tail = fresh.filter(t => !keepPaths.has(t.file_path));
            setPlaylistSource('mix', [...keep, ...tail]);
            loadCoversForTracks(tail.slice(0, 24));
        } else {
            // Новая волна: если рекомендации снова поставили играющий трек первым,
            // убираем его с первой позиции, чтобы текущий трек гарантированно сменился.
            const currentPath = currentTrack.value?.file_path;
            if (currentPath && fresh.length > 1 && fresh[0].file_path === currentPath) {
                const j = 1 + Math.floor(Math.random() * (fresh.length - 1));
                [fresh[0], fresh[j]] = [fresh[j], fresh[0]];
            }
            setPlaylistSource('mix', fresh);
            await playTrack(fresh[0], { manualListStart: true });
            loadCoversForTracks(fresh.slice(0, 24));
        }
    } finally {
        isBuildingMix.value = false;
    }
}

function onPlayMix() {
    if (isMixActive.value) {
        togglePlayPause();
        return;
    }
    void startMix(selectedMood.value);
}

/** Перемешать — новая очередь рекомендаций и новый текущий трек. */
function onShuffleMix() {
    void startMix(selectedMood.value);
}

function selectMood(id: string) {
    selectedMood.value = id;
    // Смена настроения перезапускает волну вместе с текущим треком.
    if (isMixActive.value) void startMix(id);
}

function selectFamiliarity(id: Familiarity) {
    familiarity.value = id;
    // Состав волны меняет только хвост — текущий трек продолжает играть.
    if (isMixActive.value) void startMix(selectedMood.value, true);
}

function toggleLossless() {
    losslessOnly.value = !losslessOnly.value;
    if (isMixActive.value) void startMix(selectedMood.value, true);
}

// ── Mix stage ───────────────────────────────────────────────────────────────

const familiarityLabel = computed(() => familiarityOptions.find(o => o.id === familiarity.value)?.label ?? '');

// ── Statistics modal ────────────────────────────────────────────────────────

const statsOpen = ref(false);
const statsLoading = ref(false);
const taste = ref<TasteProfile | null>(null);
const summary = ref<SummaryStats | null>(null);

async function openStats() {
    statsOpen.value = true;
    statsLoading.value = true;
    try {
        const [t, s] = await Promise.all([
            getTasteProfile().catch(() => null),
            getSummaryStats().catch(() => null),
        ]);
        taste.value = t;
        summary.value = s;
    } finally {
        statsLoading.value = false;
    }
}

const hasTaste = computed(() => !!taste.value && taste.value.analyzed_tracks > 0);
const analyzedTracks = computed(() => taste.value?.analyzed_tracks ?? 0);

function clamp01(v: number): number { return Math.min(1, Math.max(0, v)); }
function pct(v: number | null | undefined): number { return Math.round(clamp01(v ?? 0) * 100); }

const metrics = computed(() => {
    const t = taste.value;
    if (!t) return [];
    const bpm = t.bpm ?? 0;
    const loud = t.loudness ?? 0;
    return [
        { key: 'Энергичность', value: pct(t.energy), display: `${pct(t.energy)}%` },
        { key: 'Танцевальность', value: pct(t.danceability), display: `${pct(t.danceability)}%` },
        { key: 'Яркость звучания', value: pct((t.spectral_centroid ?? 0) / 4000), display: `${pct((t.spectral_centroid ?? 0) / 4000)}%` },
        { key: 'Темп', value: pct((bpm - 50) / 140), display: `${Math.round(bpm)} BPM` },
        { key: 'Громкость', value: pct((loud + 30) / 30), display: `${loud.toFixed(1)} dB` },
    ];
});

const character = computed(() => {
    const t = taste.value;
    if (!t || !hasTaste.value) return '';
    const parts: string[] = [];
    const e = t.energy ?? 0, d = t.danceability ?? 0, b = t.bpm ?? 0;
    if (e >= 0.55) parts.push('энергичная');
    else if (e < 0.4) parts.push('спокойная');
    if (d >= 0.6) parts.push('танцевальная');
    if (b >= 130) parts.push('быстрая');
    else if (b > 0 && b < 95) parts.push('неторопливая');
    if (parts.length === 0) parts.push('сбалансированная');
    return `Преимущественно ${parts.join(', ')} музыка`;
});

const favoriteDaypart = computed(() => {
    const s = summary.value;
    if (!s) return '—';
    const arr = [
        { k: 'Утро', v: s.morning_listens },
        { k: 'День', v: s.day_listens },
        { k: 'Вечер', v: s.evening_listens },
        { k: 'Ночь', v: s.night_listens },
    ].sort((a, b) => b.v - a.v);
    return arr[0].v > 0 ? arr[0].k : '—';
});

// ── Lifecycle ───────────────────────────────────────────────────────────────

function onKeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    if (statsOpen.value) statsOpen.value = false;
    else if (settingsOpen.value) settingsOpen.value = false;
}

onMounted(() => {
    window.addEventListener('keydown', onKeydown);
});

onUnmounted(() => {
    window.removeEventListener('keydown', onKeydown);
});
</script>

<template>
    <div class="mainRoot">
        <div class="pageRoot">
            <!-- ── Музыкальный профиль ──────────────────────────────────────── -->
            <button class="statsBtn" @click="openStats" v-tooltip:bottom="'Ваш музыкальный профиль'">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M3 3v16a2 2 0 0 0 2 2h16"/>
                    <path d="M18 17V9"/>
                    <path d="M13 17V5"/>
                    <path d="M8 17v-3"/>
                </svg>
                <span>Статистика</span>
            </button>

            <!-- ── «Мой микс»: персональная волна на всю страницу ───────────── -->
            <div v-if="hasLibrary" class="mixStage" :class="{ playing: isMixPlaying }">
                <div class="mixVisual">
                    <!-- Рябь от «источника звука»: кольца расходятся, когда волна
                         играет; на паузе замирают концентрической композицией. -->
                    <div class="mixRings" aria-hidden="true">
                        <span v-for="i in 4" :key="i" class="mixRing"></span>
                    </div>
                    <div class="mixCover" aria-hidden="true">
                        <Transition name="mood-swap" mode="out-in">
                            <MoodIcon :key="currentMood.id" class="mixMoodIcon" :name="currentMood.id" />
                        </Transition>
                    </div>
                </div>

                <div class="mixType">Персональная волна</div>
                <h1 class="mixName">Мой микс</h1>
                <Transition name="desc-fade" mode="out-in">
                    <p class="mixDesc" :key="currentMood.id">{{ currentMood.desc }}</p>
                </Transition>

                <div class="mixBadges">
                    <div class="badge" @click="settingsOpen = true" v-tooltip:bottom="'Настройки микса'">{{ currentMood.label }}</div>
                    <div class="badge" @click="settingsOpen = true" v-tooltip:bottom="'Настройки микса'">{{ familiarityLabel }}</div>
                    <div v-if="losslessOnly" class="badge" @click="settingsOpen = true" v-tooltip:bottom="'Настройки микса'">Lossless</div>
                </div>

                <div class="mixControls">
                    <button
                        class="playListBtn"
                        :class="{ building: isBuildingMix }"
                        :disabled="!hasLibrary"
                        @click="onPlayMix"
                    >
                        <PauseIcon v-if="isMixPlaying" width="16" height="16" />
                        <PlayIcon v-else width="15" height="15" style="margin-left:1px;" />
                        <span>{{ isMixPlaying ? 'Пауза' : 'Слушать' }}</span>
                    </button>
                    <button
                        class="roundBtn"
                        :disabled="!hasLibrary"
                        @click="onShuffleMix"
                        v-tooltip:bottom="'Перемешать микс'"
                    >
                        <ShuffleIcon width="18" height="18" />
                    </button>
                    <button
                        class="roundBtn"
                        :class="{ active: settingsOpen }"
                        @click="settingsOpen = true"
                        v-tooltip:bottom="'Настройки микса'"
                    >
                        <TuneIcon width="18" height="18" />
                    </button>
                </div>
            </div>

            <!-- ── Empty state (no library) ────────────────────────────────── -->
            <div v-else class="emptyState">
                <div class="emptyIcon"><CatalogIcon width="16" height="16" /></div>
                <div class="emptyText">Ваша библиотека пуста</div>
                <div class="emptyDesc">Добавьте папки с музыкой в каталоге, чтобы собрать персональный микс.</div>
                <button class="emptyBtn" @click="router.push('/catalog')">Открыть каталог</button>
            </div>
        </div>

        <!-- ── Settings panel (внутри viewport, без размытия) ──────────────── -->
        <Transition name="panel">
            <div v-if="settingsOpen" class="panelCatcher" @click="settingsOpen = false">
                <div class="settingsPanel" @click.stop>
                    <div class="panelHead">
                        <div class="panelTitle">Настройки микса</div>
                        <button class="panelClose" @click="settingsOpen = false" v-tooltip:left="'Закрыть'">
                            <CloseIcon width="18" height="18" />
                        </button>
                    </div>
                    <div class="panelBody">
                        <div class="settingGroup">
                            <div class="settingLabel">Настроение</div>
                            <div class="moodList">
                                <button
                                    v-for="mood in moods"
                                    :key="mood.id"
                                    class="moodRow"
                                    :class="{ active: selectedMood === mood.id }"
                                    @click="selectMood(mood.id)"
                                >
                                    <div class="moodIco">
                                        <MoodIcon :name="mood.id" width="18" height="18" />
                                    </div>
                                    <div class="moodText">
                                        <span class="moodName">{{ mood.label }}</span>
                                        <span v-truncate class="moodDesc">{{ mood.short_desc }}</span>
                                    </div>
                                    <span class="moodRadio"><span class="moodRadioDot"></span></span>
                                </button>
                            </div>
                        </div>

                        <div class="settingGroup">
                            <div class="settingLabel">Состав волны</div>
                            <div class="segmented">
                                <button
                                    v-for="opt in familiarityOptions"
                                    :key="opt.id"
                                    class="segment"
                                    :class="{ active: familiarity === opt.id }"
                                    @click="selectFamiliarity(opt.id)"
                                >
                                    {{ opt.label }}
                                </button>
                            </div>
                        </div>

                        <div class="settingGroup">
                            <button class="toggleRow" @click="toggleLossless">
                                <div class="toggleText">
                                    <div class="toggleName">Только Lossless</div>
                                    <div class="toggleHint">Только треки без потерь качества</div>
                                </div>
                                <span class="switch" :class="{ on: losslessOnly }"><span class="knob"></span></span>
                            </button>
                        </div>
                    </div>
                </div>
            </div>
        </Transition>

        <!-- ── Statistics modal ────────────────────────────────────────────── -->
        <Teleport to="body">
            <Transition name="stats">
                <div v-if="statsOpen" class="statsBackdrop" @click="statsOpen = false">
                    <div class="statsModal" @click.stop>
                        <button class="statsClose" @click="statsOpen = false" v-tooltip:left="'Закрыть'">
                            <CloseIcon width="20" height="20" />
                        </button>

                        <div v-if="statsLoading" class="statsLoading">Загрузка музыкального профиля…</div>

                        <template v-else-if="hasTaste">
                            <div class="statsHead">
                                <div class="statsKicker">Музыкальный профиль</div>
                                <div class="statsTitle">Ваш средний характер</div>
                                <div class="statsSubtitle">{{ character }}.</div>
                            </div>

                            <div class="statsSection">
                                <div class="statsSectionLabel">Характер звучания</div>
                                <div class="metricRows">
                                    <div v-for="m in metrics" :key="m.key" class="metricRow">
                                        <span class="metricName">{{ m.key }}</span>
                                        <span class="metricTrack"><span class="metricFill" :style="{ width: m.value + '%' }"></span></span>
                                        <span class="metricVal">{{ m.display }}</span>
                                    </div>
                                </div>
                            </div>

                            <div class="statsSection">
                                <div class="statsSectionLabel">Прослушивание</div>
                                <div class="summaryRows">
                                    <div class="summaryRow">
                                        <span class="summaryLabel">Прослушивания</span>
                                        <span class="summaryVal">{{ summary ? summary.total_listens : 0 }}</span>
                                    </div>
                                    <div class="summaryRow">
                                        <span class="summaryLabel">Время за музыкой</span>
                                        <span class="summaryVal">{{ summary ? formatTotalDuration(summary.total_listened_secs) : '0 мин' }}</span>
                                    </div>
                                    <div class="summaryRow">
                                        <span class="summaryLabel">Любимое время суток</span>
                                        <span class="summaryVal">{{ favoriteDaypart }}</span>
                                    </div>
                                </div>
                            </div>

                            <div class="statsFootNote">Проанализировано {{ tracksCountText(analyzedTracks) }} с аудио-анализом</div>
                        </template>

                        <div v-else class="statsEmpty">
                            Пока недостаточно данных. Слушайте музыку — и здесь появится ваш средний музыкальный характер,
                            на основе которого подбираются «Любимые» треки.
                        </div>
                    </div>
                </div>
            </Transition>
        </Teleport>
    </div>
</template>

<style scoped>
.mainRoot {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
}
.pageRoot {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    padding: 20px 28px;
    box-sizing: border-box;
    background-color: var(--light-bg);
    background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
    background-repeat: no-repeat;
    overflow: hidden;
}

/* ── «Статистика» ───────────────────────────────────────────────────────── */
.statsBtn {
    position: absolute;
    top: 20px;
    right: 28px;
    z-index: 3;
    user-select: none;
    cursor: pointer;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    padding: 9px 16px;
    border: none;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.75);
    font-size: 13px;
    font-weight: 600;
    transition: .24s ease;
}
.statsBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.11);
    color: hsla(0, 0%, 100%, 1);
}

/* ── Сцена волны ────────────────────────────────────────────────────────── */
.mixStage {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    width: 100%;
}

/* Визуальный центр: генеративная обложка волны + расходящаяся рябь. */
.mixVisual {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 30px;
}
/* Мягкий свет от «источника звука» позади обложки. */
.mixVisual::before {
    content: "";
    position: absolute;
    top: 50%;
    left: 50%;
    width: 680px;
    height: 680px;
    margin: -340px 0 0 -340px;
    border-radius: 50%;
    background: radial-gradient(circle, hsla(var(--accent-h), var(--accent-s), 58%, 0.12), transparent 62%);
    pointer-events: none;
}
.mixRings {
    --pulse-size: 620px;
    position: absolute;
    top: 50%;
    left: 50%;
    width: 0;
    height: 0;
    pointer-events: none;
}
.mixRing {
    position: absolute;
    top: calc(var(--pulse-size) / -2);
    left: calc(var(--pulse-size) / -2);
    width: var(--pulse-size);
    height: var(--pulse-size);
    border-radius: 50%;
    border: 1.5px solid hsla(var(--accent-h), var(--accent-s), 65%, 0.4);
    box-sizing: border-box;
    animation: mixRipple 5s ease-out infinite;
    animation-play-state: paused;
}
.mixRing:nth-child(1) { animation-delay: 0s; }
.mixRing:nth-child(2) { animation-delay: -1.25s; }
.mixRing:nth-child(3) { animation-delay: -2.5s; }
.mixRing:nth-child(4) { animation-delay: -3.75s; }
.mixStage.playing .mixRing { animation-play-state: running; }
@keyframes mixRipple {
    from { transform: scale(0.34); opacity: 0.9; }
    to { transform: scale(1); opacity: 0; }
}

.mixCover {
    position: relative;
    z-index: 1;
    width: 230px;
    height: 230px;
    border-radius: 22px;
    overflow: hidden;
    background:
        radial-gradient(130% 130% at 20% 15%, hsla(var(--accent-h), var(--accent-s), 58%, 0.5), transparent 58%),
        radial-gradient(150% 150% at 85% 92%, hsla(var(--accent-h), var(--accent-s), 42%, 0.38), transparent 62%),
        hsl(var(--accent-h), min(var(--accent-s), 52%), 13%);
    box-shadow:
        inset 0 1px 0 hsla(0, 0%, 100%, 0.07),
        0 24px 60px hsla(var(--accent-h), min(var(--accent-s), 50%), 6%, 0.5);
    user-select: none;
}
.mixMoodIcon {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 64px;
    height: 64px;
    margin: -32px 0 0 -32px;
    color: hsla(0, 0%, 100%, 0.92);
    filter: drop-shadow(0 2px 14px hsla(var(--accent-h), var(--accent-s), 25%, 0.55));
}
.mood-swap-enter-active,
.mood-swap-leave-active { transition: opacity .2s ease, transform .2s ease; }
.mood-swap-enter-from { opacity: 0; transform: scale(0.82); }
.mood-swap-leave-to { opacity: 0; transform: scale(1.12); }

/* ── Текстовый блок волны ───────────────────────────────────────────────── */
.mixType {
    position: relative;
    z-index: 1;
    user-select: none;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.4);
}
.mixName {
    position: relative;
    z-index: 1;
    user-select: none;
    cursor: default;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 54px;
    font-weight: 900;
    color: hsla(0, 0%, 100%, 0.95);
    line-height: 1.05;
    letter-spacing: -1.5px;
    margin: 4px 0 0;
}
.mixDesc {
    position: relative;
    z-index: 1;
    user-select: none;
    margin: 12px 0 0;
    font-size: 15px;
    line-height: 1.55;
    color: hsla(0, 0%, 100%, 0.55);
    max-width: 560px;
    min-height: 23px;
}
.desc-fade-enter-active,
.desc-fade-leave-active { transition: opacity .2s ease, transform .2s ease; }
.desc-fade-enter-from { opacity: 0; transform: translateY(4px); }
.desc-fade-leave-to { opacity: 0; transform: translateY(-4px); }

.mixBadges {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    margin-top: 18px;
    max-width: 100%;
}
.badge {
    user-select: none;
    cursor: pointer;
    padding: 4px 10px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    font-size: 12px;
    font-weight: 500;
    transition: .24s ease;
    max-width: 100%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.badge:hover {
    background-color: hsla(0, 0%, 100%, 0.1);
    color: hsla(0, 0%, 100%, 0.9);
}

/* ── Контролы волны ─────────────────────────────────────────────────────── */
.mixControls {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: center;
    gap: 10px;
    margin-top: 26px;
}
.playListBtn {
    user-select: none;
    cursor: pointer;
    box-sizing: border-box;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-width: 168px;
    padding: 14px 26px;
    border: none;
    border-radius: 9999px;
    background-color: hsla(0, 0%, 100%, 1);
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
    font-size: 14px;
    font-weight: 700;
    transition: .24s ease;
}
.playListBtn:hover:not(:disabled) { background-color: hsla(0, 0%, 100%, 0.84); }
.playListBtn:disabled {
    cursor: not-allowed;
    background-color: hsla(0, 0%, 100%, 0.3);
}
.playListBtn.building { cursor: progress; }

.roundBtn {
    user-select: none;
    cursor: pointer;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 50px;
    height: 50px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.78);
    box-sizing: border-box;
    transition: .24s ease;
}
.roundBtn:hover:not(:disabled) {
    background-color: hsla(0, 0%, 100%, 0.12);
    color: hsla(0, 0%, 100%, 1);
}
.roundBtn:disabled { cursor: not-allowed; opacity: 0.4; }
.roundBtn.active {
    background-color: hsla(var(--accent-h), var(--accent-s), 55%, 0.9);
    color: #fff;
}

/* ── Empty state ────────────────────────────────────────────────────────── */
.emptyState {
    display: flex;
    gap: 10px;
    flex-direction: column;
    align-items: center;
    justify-content: center;
}
.emptyIcon {
    padding: 6px;
    border-radius: 5px;
    background-color: hsla(0, 0%, 100%, 0.09);
    color: #ffffff;
    display: flex;
    align-items: center;
    justify-content: center;
}
.emptyText {
    user-select: none;
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.7);
}
.emptyDesc {
    user-select: none;
    text-align: center;
    max-width: 270px;
    font-size: 13px;
    font-weight: 400;
    color: hsla(0, 0%, 100%, 0.3);
}
.emptyBtn {
    user-select: none;
    cursor: pointer;
    margin-top: 8px;
    padding: 8px 18px;
    border: none;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.07);
    color: hsla(0, 0%, 100%, 0.85);
    font-size: 13px;
    font-weight: 500;
    transition: .24s ease;
}
.emptyBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
    color: hsla(0, 0%, 100%, 1);
}

/* ── Settings panel (within viewport, no blur, no borders) ──────────────── */
.panelCatcher {
    position: absolute;
    inset: 0;
    z-index: 50;
}
.settingsPanel {
    position: absolute;
    top: 12px;
    right: 12px;
    bottom: 12px;
    width: 40%;
    max-width: calc(100% - 24px);
    min-width: 320px;
    display: flex;
    flex-direction: column;
    border-radius: 24px;
    background: hsl(var(--accent-h), min(var(--accent-s), 48%), 12%);
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.45);
    box-sizing: border-box;
    overflow: hidden;
}
.panelHead {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    padding: 20px 22px 14px;
    flex-shrink: 0;
}
.panelTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 21px;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.95);
    user-select: none;
}
.panelClose {
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border: none;
    border-radius: 9999px;
    background: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    transition: .2s ease;
}
.panelClose:hover { background: hsla(0, 0%, 100%, 0.12); color: #fff; }
.panelBody {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 22px 20px;
    display: flex;
    flex-direction: column;
    gap: 26px;
}
.panelBody::-webkit-scrollbar { width: 6px; }
.panelBody::-webkit-scrollbar-thumb { background: hsla(0, 0%, 100%, 0.12); border-radius: 3px; }

.settingGroup {
    display: flex;
    flex-direction: column;
    gap: 12px;
}
.settingLabel {
    user-select: none;
    font-size: 12px;
    font-weight: 800;
    letter-spacing: 1px;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.4);
}

/* Mood list (вертикальный список с радио-индикатором) */
.moodList {
    display: flex;
    flex-direction: column;
    gap: 6px;
}
.moodRow {
    cursor: pointer;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 10px 12px;
    border: none;
    outline: none;
    box-shadow: none;
    border-radius: 14px;
    background: rgba(255, 255, 255, 0.03);
    text-align: left;
    transition: background 0.2s ease;
}
.moodRow:focus,
.moodRow:active,
.moodRow:focus-visible {
    outline: none;
    box-shadow: none;
    border: none;
}
.moodRow:hover {
    background: rgba(255, 255, 255, 0.06);
}
.moodRow.active {
    background: hsla(var(--accent-h), var(--accent-s), 55%, 0.16);
}
.moodIco {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.05);
    color: hsla(var(--accent-h), var(--accent-s), 70%, 1);
    transition: background .2s ease, color .2s ease;
    flex-shrink: 0;
}
.moodRow:hover .moodIco {
    background: rgba(255, 255, 255, 0.08);
}
.moodRow.active .moodIco {
    background: hsla(var(--accent-h), var(--accent-s), 55%, 0.9);
    color: #fff;
}
.moodText {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
}
.moodName {
    font-size: 13.5px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.92);
}
.moodDesc {
    font-size: 11.5px;
    font-weight: 500;
    color: rgba(255, 255, 255, 0.45);
    line-height: 1.35;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: color 0.2s ease;
}
.moodRow:hover .moodDesc { color: rgba(255, 255, 255, 0.6); }
.moodRow.active .moodName { color: #fff; }
.moodRow.active .moodDesc { color: rgba(255, 255, 255, 0.7); }
.moodRadio {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid hsla(0, 0%, 100%, 0.22);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    box-sizing: border-box;
    transition: border-color 0.2s ease;
}
.moodRow.active .moodRadio {
    border-color: hsla(var(--accent-h), var(--accent-s), 62%, 1);
}
.moodRadioDot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background-color: hsla(var(--accent-h), var(--accent-s), 62%, 1);
    transform: scale(0);
    transition: transform 0.2s cubic-bezier(0.32, 0.72, 0, 1);
}
.moodRow.active .moodRadioDot { transform: scale(1); }

.segmented {
    display: flex;
    flex-direction: row;
    gap: 4px;
    padding: 4px;
    border-radius: 14px;
    background: hsla(0, 0%, 100%, 0.04);
}
.segment {
    flex: 1;
    cursor: pointer;
    padding: 10px 6px;
    border: none;
    border-radius: 10px;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.6);
    font-size: 12.5px;
    font-weight: 700;
    transition: .2s ease;
}
.segment:hover { color: hsla(0, 0%, 100%, 0.9); }
.segment.active {
    background-color: hsla(var(--accent-h), var(--accent-s), 55%, 0.9);
    color: #fff;
}

.toggleRow {
    cursor: pointer;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    width: 100%;
    padding: 14px 16px;
    border: none;
    border-radius: 14px;
    background: hsla(0, 0%, 100%, 0.03);
    text-align: left;
    transition: background .2s ease;
}
.toggleRow:hover { background: hsla(0, 0%, 100%, 0.06); }
.toggleText { display: flex; flex-direction: column; gap: 3px; }
.toggleName { font-size: 14px; font-weight: 600; color: hsla(0, 0%, 100%, 0.9); }
.toggleHint { font-size: 12px; color: hsla(0, 0%, 100%, 0.4); }
.switch {
    flex-shrink: 0;
    position: relative;
    width: 42px;
    height: 24px;
    border-radius: 9999px;
    background: hsla(0, 0%, 100%, 0.14);
    transition: background .24s ease;
}
.switch.on { background: hsla(var(--accent-h), var(--accent-s), 55%, 1); }
.knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #fff;
    transition: transform .24s cubic-bezier(0.4, 0, 0.2, 1);
}
.switch.on .knob { transform: translateX(18px); }



.panel-enter-active, .panel-leave-active { transition: opacity .24s ease; }
.panel-enter-active .settingsPanel, .panel-leave-active .settingsPanel { transition: transform .28s cubic-bezier(0.32, 0.72, 0, 1); }
.panel-enter-from, .panel-leave-to { opacity: 0; }
.panel-enter-from .settingsPanel, .panel-leave-to .settingsPanel { transform: translateX(calc(100% + 12px)); }

/* ── Responsive ─────────────────────────────────────────────────────────── */
@media (max-width: 1100px) and (min-width: 721px) {
    .mixCover { width: 200px; height: 200px; }
    .mixRings { --pulse-size: 520px; }
    .mixVisual::before { width: 560px; height: 560px; margin: -280px 0 0 -280px; }
    .mixMoodIcon { width: 56px; height: 56px; margin: -28px 0 0 -28px; }
    .mixName { font-size: 46px; }
}

@media (max-width: 720px) {
    .pageRoot { padding: 16px; }
    .statsBtn { top: 14px; right: 16px; min-height: 40px; }
    .mixVisual { margin-bottom: 24px; }
    .mixCover { width: 168px; height: 168px; border-radius: 18px; }
    .mixRings { --pulse-size: 400px; }
    .mixVisual::before { width: 440px; height: 440px; margin: -220px 0 0 -220px; }
    .mixMoodIcon { width: 48px; height: 48px; margin: -24px 0 0 -24px; }
    .mixName { font-size: 38px; letter-spacing: -1px; }
    .mixDesc { font-size: 14px; max-width: 100%; }
    .playListBtn { min-width: 150px; min-height: 42px; padding: 13px 22px; font-size: 13px; }
    .roundBtn { width: 46px; height: 46px; }
    .settingsPanel { width: auto; left: 12px; min-width: 0; }
}
</style>

<!-- Statistics modal styles are global (teleported to body, outside scoped tree) -->
<style>
.statsBackdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: calc(100vw / var(--overlay-scale, 1));
    height: calc(100dvh / var(--overlay-scale, 1));
    z-index: 1000001;
    display: flex;
    align-items: center;
    justify-content: center;
    background-color: hsla(0, 0%, 0%, 0.55);
    backdrop-filter: blur(4px);
    padding: 24px;
    box-sizing: border-box;
}
.statsModal {
    position: relative;
    width: 480px;
    max-width: 100%;
    max-height: 86vh;
    overflow-y: auto;
    padding: 28px 28px 22px;
    border-radius: 24px;
    background-color: hsl(var(--accent-h), min(var(--accent-s), 48%), 12%);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
    border: none;
    outline: none;
    box-sizing: border-box;
}
.statsModal::-webkit-scrollbar { width: 6px; }
.statsModal::-webkit-scrollbar-thumb { background: hsla(0, 0%, 100%, 0.14); border-radius: 3px; }
.statsClose {
    position: absolute;
    top: 20px;
    right: 20px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 38px;
    height: 38px;
    border: none;
    outline: none;
    box-shadow: none;
    border-radius: 9999px;
    background: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    transition: .2s ease;
}
.statsClose:hover {
    background: hsla(0, 0%, 100%, 0.12);
    color: #fff;
    box-shadow: none;
}
.statsClose:focus,
.statsClose:focus-visible {
    outline: none;
    box-shadow: none;
}

/* Profile header */
.statsHead {
    display: flex;
    flex-direction: column;
    padding-right: 48px;
    margin-top: 2px;
}
.statsKicker {
    font-size: 11px;
    font-weight: 800;
    letter-spacing: 1px;
    text-transform: uppercase;
    color: hsla(var(--accent-h), var(--accent-s), 72%, 0.9);
    margin-bottom: 6px;
    user-select: none;
}
.statsTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 25px;
    font-weight: 900;
    color: #fff;
    letter-spacing: -0.5px;
    user-select: none;
}
.statsSubtitle {
    margin-top: 6px;
    font-size: 13px;
    line-height: 1.45;
    color: hsla(0, 0%, 100%, 0.55);
}
.statsLoading, .statsEmpty {
    margin-top: 18px;
    font-size: 14px;
    line-height: 1.55;
    color: hsla(0, 0%, 100%, 0.5);
}

/* Profile sections */
.statsSection {
    margin-top: 28px;
}
.statsSectionLabel {
    font-size: 11.5px;
    font-weight: 800;
    letter-spacing: 1px;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.38);
    margin-bottom: 14px;
    user-select: none;
}

/* Sound character: metric rows */
.metricRows {
    display: flex;
    flex-direction: column;
    gap: 13px;
}
.metricRow {
    display: grid;
    grid-template-columns: 132px minmax(0, 1fr) 76px;
    align-items: center;
    gap: 14px;
}
@media (max-width: 520px) {
    .metricRow {
        grid-template-columns: 110px minmax(0, 1fr) 70px;
        gap: 10px;
    }
}
.metricName {
    font-size: 13px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.75);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.metricTrack {
    display: block;
    height: 5px;
    border-radius: 9999px;
    background: hsla(0, 0%, 100%, 0.07);
    overflow: hidden;
}
.metricFill {
    display: block;
    height: 100%;
    border-radius: inherit;
    background-color: hsla(var(--accent-h), var(--accent-s), 58%, 1);
    transition: width .7s cubic-bezier(0.22, 1, 0.36, 1);
}
.metricVal {
    font-size: 12.5px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.92);
    font-variant-numeric: tabular-nums;
    text-align: right;
    white-space: nowrap;
}

/* Listening summary rows */
.summaryRows {
    display: flex;
    flex-direction: column;
}
.summaryRow {
    display: flex;
    flex-direction: row;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    padding: 9px 0;
}
.summaryRow:first-child { padding-top: 0; }
.summaryRow + .summaryRow { border-top: 1px solid hsla(0, 0%, 100%, 0.07); }
.summaryLabel {
    font-size: 13px;
    color: hsla(0, 0%, 100%, 0.55);
}
.summaryVal {
    font-size: 14px;
    font-weight: 700;
    color: #fff;
    font-variant-numeric: tabular-nums;
    text-align: right;
}

.statsFootNote {
    margin-top: 26px;
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.35);
    text-align: center;
}

.stats-enter-active, .stats-leave-active { transition: opacity .2s ease; }
.stats-enter-active .statsModal, .stats-leave-active .statsModal { transition: transform .24s cubic-bezier(0.32, 0.72, 0, 1), opacity .24s ease; }
.stats-enter-from, .stats-leave-to { opacity: 0; }
.stats-enter-from .statsModal, .stats-leave-to .statsModal { transform: scale(0.96) translateY(6px); opacity: 0; }
</style>
