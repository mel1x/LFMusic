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
import { formatTotalDuration } from '../utils/format';
import { useI18n } from '../composables/useI18n';

const router = useRouter();
const { t } = useI18n();

const {
    catalogPaths,
    currentTrack,
    isPlaying,
    shuffled,
    playbackOrder,
    currentTrackIndex,
    activePlaylistSource,
    loadCoversForTracks,
    playTrack,
    togglePlayPause,
    setMixSourceFromPaths,
    cacheTracks,
    fetchTracksByPaths,
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

const hasLibrary = computed(() => catalogPaths.value.length > 0);

const mixOptions = computed<RecommendationOptions>(() => ({
    familiarity: familiarity.value,
    losslessOnly: losslessOnly.value || undefined,
}));

// ── Mix playback ────────────────────────────────────────────────────────────

/** Пути очереди микса. Рекомендации (движок учитывает всю библиотеку) → их пути;
 *  если рекомендаций нет — фолбэк на ВЕСЬ каталог вперемешку (только пути, объекты
 *  резолвятся лениво через кэш). */
async function buildMixPaths(mood: string): Promise<string[]> {
    let recs: TrackMetadata[] = [];
    try {
        recs = await getRecommendations(mood, 80, mixOptions.value);
    } catch {
        recs = [];
    }
    if (recs.length > 0) {
        cacheTracks(recs);
        return recs.map(t => t.file_path);
    }
    // Фолбэк: вся библиотека вперемешку (как было до пагинации).
    return [...catalogPaths.value].sort(() => Math.random() - 0.5);
}

/** Прогрев обложек первых N путей очереди (резолв в объекты + загрузка обложек). */
async function warmCovers(paths: string[]) {
    const tracks = await fetchTracksByPaths(paths);
    if (tracks.length) loadCoversForTracks(tracks);
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
        const fresh = await buildMixPaths(mood);
        if (fresh.length === 0) return;

        // Детерминированная очередь волны (без глобального шаффла).
        shuffled.value = false;

        if (keepCurrent && isMixActive.value && currentTrack.value && currentTrackIndex.value >= 0) {
            // Мягкое обновление: текущий трек продолжает играть, хвост пересобирается.
            const keepPaths = playbackOrder.value.slice(0, currentTrackIndex.value + 1);
            const keepSet = new Set(keepPaths);
            const tail = fresh.filter(p => !keepSet.has(p));
            setMixSourceFromPaths([...keepPaths, ...tail]);
            void warmCovers(tail.slice(0, 24));
        } else {
            // Новая волна: если рекомендации снова поставили играющий трек первым,
            // убираем его с первой позиции, чтобы текущий трек гарантированно сменился.
            const currentPath = currentTrack.value?.file_path;
            if (currentPath && fresh.length > 1 && fresh[0] === currentPath) {
                const j = 1 + Math.floor(Math.random() * (fresh.length - 1));
                [fresh[0], fresh[j]] = [fresh[j], fresh[0]];
            }
            setMixSourceFromPaths(fresh);
            await playTrack(fresh[0], { manualListStart: true });
            void warmCovers(fresh.slice(0, 24));
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
        { key: 'Энергичность', short: 'Энергия', value: pct(t.energy), display: `${pct(t.energy)}%` },
        { key: 'Танцевальность', short: 'Танец', value: pct(t.danceability), display: `${pct(t.danceability)}%` },
        { key: 'Яркость', short: 'Яркость', value: pct((t.spectral_centroid ?? 0) / 4000), display: `${pct((t.spectral_centroid ?? 0) / 4000)}%` },
        { key: 'Темп', short: 'Темп', value: pct((bpm - 50) / 140), display: `${Math.round(bpm)} BPM` },
        { key: 'Громкость', short: 'Громкость', value: pct((loud + 30) / 30), display: `${loud.toFixed(1)} dB` },
    ];
});

// ── Радар характера звучания (SVG spider chart) ──────────────────────────────
const RADAR = { w: 360, h: 248, cx: 180, cy: 118, r: 86, labelR: 110 };

const radarAxes = computed(() => {
    const n = metrics.value.length || 5;
    return metrics.value.map((m, i) => {
        const ang = (-90 + (360 / n) * i) * Math.PI / 180;
        const cos = Math.cos(ang), sin = Math.sin(ang);
        return {
            short: m.short,
            display: m.display,
            value: m.value,
            cos, sin,
            ax: RADAR.cx + RADAR.r * cos,
            ay: RADAR.cy + RADAR.r * sin,
            px: RADAR.cx + RADAR.r * (m.value / 100) * cos,
            py: RADAR.cy + RADAR.r * (m.value / 100) * sin,
            lx: RADAR.cx + RADAR.labelR * cos,
            ly: RADAR.cy + RADAR.labelR * sin + (sin < -0.5 ? 0 : 4),
            anchor: cos > 0.25 ? 'start' : cos < -0.25 ? 'end' : 'middle',
        };
    });
});

const radarPolygon = computed(() =>
    radarAxes.value.map(a => `${a.px.toFixed(1)},${a.py.toFixed(1)}`).join(' ')
);

const radarRings = computed(() =>
    [0.25, 0.5, 0.75, 1].map(f =>
        radarAxes.value.map(a => {
            const x = RADAR.cx + RADAR.r * f * a.cos;
            const y = RADAR.cy + RADAR.r * f * a.sin;
            return `${x.toFixed(1)},${y.toFixed(1)}`;
        }).join(' ')
    )
);

const character = computed(() => {
    const tp = taste.value;
    if (!tp || !hasTaste.value) return '';
    const keys: string[] = [];
    const e = tp.energy ?? 0, d = tp.danceability ?? 0, b = tp.bpm ?? 0;
    if (e >= 0.55) keys.push('stats.traitEnergetic');
    else if (e < 0.4) keys.push('stats.traitCalm');
    if (d >= 0.6) keys.push('stats.traitDanceable');
    if (b >= 130) keys.push('stats.traitFast');
    else if (b > 0 && b < 95) keys.push('stats.traitSlow');
    if (keys.length === 0) keys.push('stats.traitBalanced');
    const traits = keys.map(k => t(k)).join(', ');
    return t('stats.characterSentence', { traits });
});

// Разбивка прослушиваний по времени суток для мини-графика.
const dayparts = computed(() => {
    const s = summary.value;
    const raw = [
        { k: 'Утро', v: s?.morning_listens ?? 0 },
        { k: 'День', v: s?.day_listens ?? 0 },
        { k: 'Вечер', v: s?.evening_listens ?? 0 },
        { k: 'Ночь', v: s?.night_listens ?? 0 },
    ];
    const max = Math.max(1, ...raw.map(d => d.v));
    const top = raw.reduce((a, b) => (b.v > a.v ? b : a));
    return raw.map(d => ({
        ...d,
        h: Math.round((d.v / max) * 100),
        peak: d.v > 0 && d.k === top.k,
    }));
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
                <!-- Центральный визуал: светящийся орб волны с вращающейся дугой
                     и расходящейся рябью от «источника звука». -->
                <div class="mixVisual">
                    <div class="mixRings" aria-hidden="true">
                        <span v-for="i in 5" :key="i" class="mixRing"></span>
                    </div>
                    <div class="orb" aria-hidden="true">
                        <div class="orbCore">
                            <Transition name="mood-swap" mode="out-in">
                                <MoodIcon :key="currentMood.id" class="mixMoodIcon" :name="currentMood.id" />
                            </Transition>
                        </div>
                    </div>

                    <!-- Что сейчас в волне -->
                    <Transition name="chip-fade">
                        <div v-if="isMixActive && currentTrack" class="nowChip">
                            <span class="eqMini" :class="{ paused: !isPlaying }"><i></i><i></i><i></i></span>
                            <span v-truncate class="nowChipText">{{ currentTrack.title || currentTrack.file_path.split(/[\\/]/).pop()?.replace(/\.[^/.]+$/, '') }}</span>
                        </div>
                    </Transition>
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

                <!-- Быстрый выбор настроения (детальные настройки — в сайдбаре) -->
                <div class="moodStrip">
                    <div class="moodStripLabel">Настроение</div>
                    <div class="moodChips">
                        <button
                            v-for="mood in moods"
                            :key="mood.id"
                            class="moodChip"
                            :class="{ active: selectedMood === mood.id }"
                            @click="selectMood(mood.id)"
                            v-tooltip:top="mood.short_desc"
                        >
                            <MoodIcon :name="mood.id" width="15" height="15" />
                            <span>{{ mood.label }}</span>
                        </button>
                    </div>
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
                                <div class="statsTitle">Ваш характер</div>
                                <div class="statsSubtitle">{{ character }}.</div>
                            </div>

                            <div class="statCards">
                                <div class="statCard">
                                    <span class="statCardIcon">
                                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                            <path d="M4 14v-1a8 8 0 0 1 16 0v1"/>
                                            <rect x="2.5" y="14" width="4.5" height="6.5" rx="2"/>
                                            <rect x="17" y="14" width="4.5" height="6.5" rx="2"/>
                                        </svg>
                                    </span>
                                    <div class="statCardValue">{{ summary ? summary.total_listens.toLocaleString('ru-RU') : 0 }}</div>
                                    <div class="statCardLabel">Прослушиваний</div>
                                </div>
                                <div class="statCard">
                                    <span class="statCardIcon">
                                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                            <circle cx="12" cy="12" r="9"/>
                                            <path d="M12 7v5l3.5 2"/>
                                        </svg>
                                    </span>
                                    <div class="statCardValue">{{ summary ? formatTotalDuration(summary.total_listened_secs) : '0 мин' }}</div>
                                    <div class="statCardLabel">За музыкой</div>
                                </div>
                                <div class="statCard">
                                    <span class="statCardIcon">
                                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                            <path d="M3 12h3l3 7 4-15 3 8h5"/>
                                        </svg>
                                    </span>
                                    <div class="statCardValue">{{ analyzedTracks.toLocaleString('ru-RU') }} треков</div>
                                    <div class="statCardLabel">Проанализировано</div>
                                </div>
                            </div>

                            <div class="statsSection">
                                <div class="statsSectionLabel">Характер звучания</div>
                                <div class="radarWrap">
                                    <svg class="radar" :viewBox="`0 0 ${RADAR.w} ${RADAR.h}`">
                                        <polygon
                                            v-for="(ring, i) in radarRings"
                                            :key="'ring' + i"
                                            :points="ring"
                                            class="radarRing"
                                        />
                                        <line
                                            v-for="(a, i) in radarAxes"
                                            :key="'axis' + i"
                                            :x1="RADAR.cx" :y1="RADAR.cy" :x2="a.ax" :y2="a.ay"
                                            class="radarAxis"
                                        />
                                        <polygon :points="radarPolygon" class="radarShape" />
                                        <circle
                                            v-for="(a, i) in radarAxes"
                                            :key="'dot' + i"
                                            :cx="a.px" :cy="a.py" r="3.4"
                                            class="radarDot"
                                        />
                                        <g v-for="(a, i) in radarAxes" :key="'lbl' + i">
                                            <text :x="a.lx" :y="a.ly" :text-anchor="a.anchor" class="radarLabel">{{ a.short }}</text>
                                            <text :x="a.lx" :y="a.ly + 15" :text-anchor="a.anchor" class="radarValue">{{ a.display }}</text>
                                        </g>
                                    </svg>
                                </div>
                            </div>

                            <div class="statsSection">
                                <div class="statsSectionLabel">Когда вы слушаете</div>
                                <div class="daypartChart">
                                    <div v-for="d in dayparts" :key="d.k" class="daypartCol" :class="{ peak: d.peak }">
                                        <div class="daypartBarTrack">
                                            <div class="daypartBar" :style="{ height: Math.max(d.h, 4) + '%' }"></div>
                                        </div>
                                        <span class="daypartName">{{ d.k }}</span>
                                    </div>
                                </div>
                            </div>
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
    padding: 24px 28px;
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

/* ── Сцена волны (центрированная, на весь viewport) ─────────────────────── */
.mixStage {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    width: 100%;
    max-width: 760px;
}

/* ── Центральный визуал: светящийся орб + рябь ──────────────────────────── */
.mixVisual {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 34px;
}
/* Мягкий свет от «источника звука» позади орба. */
.mixVisual::before {
    content: "";
    position: absolute;
    top: 50%;
    left: 50%;
    width: 720px;
    height: 720px;
    margin: -360px 0 0 -360px;
    border-radius: 50%;
    background: radial-gradient(circle, hsla(var(--accent-h), var(--accent-s), 58%, 0.16), transparent 60%);
    pointer-events: none;
}
.mixRings {
    --pulse-size: 660px;
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
    border: 1.5px solid hsla(var(--accent-h), var(--accent-s), 65%, 0.35);
    box-sizing: border-box;
    animation: mixRipple 6s ease-out infinite;
    animation-play-state: paused;
}
.mixRing:nth-child(1) { animation-delay: 0s; }
.mixRing:nth-child(2) { animation-delay: -1.2s; }
.mixRing:nth-child(3) { animation-delay: -2.4s; }
.mixRing:nth-child(4) { animation-delay: -3.6s; }
.mixRing:nth-child(5) { animation-delay: -4.8s; }
.mixStage.playing .mixRing { animation-play-state: running; }
@keyframes mixRipple {
    from { transform: scale(0.26); opacity: 0.85; }
    to { transform: scale(1); opacity: 0; }
}

/* Орб */
.orb {
    position: relative;
    z-index: 1;
    width: 248px;
    height: 248px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    animation: orbFloat 7s ease-in-out infinite;
}
@keyframes orbFloat {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(-9px); }
}

/* Светящееся ядро орба. */
.orbCore {
    position: relative;
    width: 100%;
    height: 100%;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    background:
        radial-gradient(120% 120% at 30% 22%, hsla(var(--accent-h), var(--accent-s), 64%, 0.6), transparent 58%),
        radial-gradient(135% 135% at 78% 95%, hsla(var(--accent-h), var(--accent-s), 44%, 0.55), transparent 60%),
        hsl(var(--accent-h), min(var(--accent-s), 52%), 13%);
    box-shadow:
        inset 0 2px 1px hsla(0, 0%, 100%, 0.16),
        inset 0 -26px 50px hsla(var(--accent-h), min(var(--accent-s), 50%), 6%, 0.55),
        0 30px 70px hsla(var(--accent-h), min(var(--accent-s), 50%), 6%, 0.5);
    transition: transform .5s cubic-bezier(0.32, 0.72, 0, 1);
}
.mixStage.playing .orbCore { transform: scale(1.04); }
/* Блик сверху. */
.orbCore::before {
    content: "";
    position: absolute;
    top: 8%;
    left: 16%;
    width: 56%;
    height: 40%;
    border-radius: 50%;
    background: radial-gradient(circle at 50% 40%, hsla(0, 0%, 100%, 0.22), transparent 70%);
    pointer-events: none;
}
.mixMoodIcon {
    position: relative;
    z-index: 1;
    width: 78px;
    height: 78px;
    color: hsla(0, 0%, 100%, 0.95);
    filter: drop-shadow(0 3px 18px hsla(var(--accent-h), var(--accent-s), 22%, 0.6));
}
.mood-swap-enter-active,
.mood-swap-leave-active { transition: opacity .2s ease, transform .2s ease; }
.mood-swap-enter-from { opacity: 0; transform: scale(0.82); }
.mood-swap-leave-to { opacity: 0; transform: scale(1.12); }

/* Чип «что сейчас в волне» — плавает под орбом. */
.nowChip {
    position: absolute;
    z-index: 3;
    bottom: -16px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 9px;
    max-width: 280px;
    padding: 7px 15px 7px 12px;
    border-radius: 999px;
    background-color: hsl(var(--accent-h), min(var(--accent-s), 44%), 16%);
    box-shadow: 0 10px 28px hsla(var(--accent-h), min(var(--accent-s), 50%), 4%, 0.5);
}
.nowChipText {
    user-select: none;
    font-size: 12.5px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
}
.eqMini {
    display: flex;
    flex-direction: row;
    align-items: flex-end;
    gap: 2.5px;
    height: 13px;
    flex-shrink: 0;
}
.eqMini i {
    display: block;
    width: 2.5px;
    height: 4px;
    border-radius: 2px;
    background-color: hsla(var(--accent-h), var(--accent-s), 72%, 1);
    animation: eqBar 0.9s ease-in-out infinite;
}
.eqMini i:nth-child(1) { animation-delay: 0s; }
.eqMini i:nth-child(2) { animation-delay: 0.3s; }
.eqMini i:nth-child(3) { animation-delay: 0.6s; }
.eqMini.paused i { animation-play-state: paused; height: 4px; }
@keyframes eqBar {
    0%, 100% { height: 3px; }
    50% { height: 13px; }
}
.chip-fade-enter-active,
.chip-fade-leave-active { transition: opacity .25s ease, transform .25s ease; }
.chip-fade-enter-from,
.chip-fade-leave-to { opacity: 0; transform: translate(-50%, 8px); }

/* ── Текстовый блок волны ───────────────────────────────────────────────── */
.mixType {
    user-select: none;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 1.4px;
    text-transform: uppercase;
    color: hsla(var(--accent-h), var(--accent-s), 72%, 0.85);
}
.mixName {
    user-select: none;
    cursor: default;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 56px;
    font-weight: 900;
    color: hsla(0, 0%, 100%, 0.96);
    line-height: 1.03;
    letter-spacing: -1.6px;
    margin: 8px 0 0;
}
.mixDesc {
    user-select: none;
    margin: 14px auto 0;
    font-size: 15px;
    line-height: 1.55;
    color: hsla(0, 0%, 100%, 0.55);
    max-width: 520px;
    min-height: 23px;
}
.desc-fade-enter-active,
.desc-fade-leave-active { transition: opacity .2s ease, transform .2s ease; }
.desc-fade-enter-from { opacity: 0; transform: translateY(4px); }
.desc-fade-leave-to { opacity: 0; transform: translateY(-4px); }

.mixBadges {
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
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 10px;
    margin-top: 28px;
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

/* ── Быстрый выбор настроения ───────────────────────────────────────────── */
.moodStrip {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    margin-top: 34px;
}
.moodStripLabel {
    user-select: none;
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 11px;
    font-weight: 800;
    letter-spacing: 1px;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.38);
}
.moodStripLabel::before,
.moodStripLabel::after {
    content: "";
    width: 28px;
    height: 1px;
    background-color: hsla(0, 0%, 100%, 0.12);
}
.moodChips {
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    max-width: 620px;
}
.moodChip {
    user-select: none;
    cursor: pointer;
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 7px;
    padding: 8px 14px 8px 12px;
    border: none;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.6);
    font-size: 12.5px;
    font-weight: 600;
    transition: .2s ease;
}
.moodChip svg { opacity: 0.7; transition: opacity .2s ease; }
.moodChip:hover {
    background-color: hsla(0, 0%, 100%, 0.09);
    color: hsla(0, 0%, 100%, 0.95);
}
.moodChip:hover svg { opacity: 0.95; }
.moodChip.active {
    background-color: hsla(var(--accent-h), var(--accent-s), 55%, 0.92);
    color: #fff;
}
.moodChip.active svg { opacity: 1; }

/* ── Empty state ────────────────────────────────────────────────────────── */
.emptyState {
    flex: 1;
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
@media (max-height: 800px) {
    .orb { width: 208px; height: 208px; }
    .mixVisual { margin-bottom: 26px; }
    .mixMoodIcon { width: 66px; height: 66px; }
    .mixName { font-size: 48px; }
    .moodStrip { margin-top: 26px; }
}

@media (max-width: 1100px) and (min-width: 721px) {
    .mixName { font-size: 48px; }
    .mixRings { --pulse-size: 560px; }
    .mixVisual::before { width: 600px; height: 600px; margin: -300px 0 0 -300px; }
}

@media (max-width: 720px) {
    .pageRoot { padding: 16px; }
    .statsBtn { top: 14px; right: 16px; min-height: 40px; }
    .mixVisual { margin-bottom: 28px; }
    .orb { width: 184px; height: 184px; }
    .mixMoodIcon { width: 58px; height: 58px; }
    .mixRings { --pulse-size: 420px; }
    .mixVisual::before { width: 460px; height: 460px; margin: -230px 0 0 -230px; }
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
    width: 500px;
    max-width: 100%;
    max-height: 88vh;
    overflow-y: auto;
    padding: 28px 30px 24px;
    border-radius: 26px;
    background:
        radial-gradient(130% 90% at 50% -10%, hsla(var(--accent-h), var(--accent-s), 40%, 0.18), transparent 55%),
        hsl(var(--accent-h), min(var(--accent-s), 48%), 12%);
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.45);
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

/* Sound character: radar (spider) chart */
.radarWrap {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    margin: -4px 0 2px;
}
/* Мягкий accent-свет за радаром — в духе орба на странице. */
.radarWrap::before {
    content: "";
    position: absolute;
    top: 50%;
    left: 50%;
    width: 220px;
    height: 220px;
    transform: translate(-50%, -50%);
    border-radius: 50%;
    background: radial-gradient(circle, hsla(var(--accent-h), var(--accent-s), 58%, 0.18), transparent 62%);
    pointer-events: none;
}
.radar {
    position: relative;
    width: 100%;
    max-width: 360px;
    height: auto;
    overflow: visible;
}
.radarRing {
    fill: none;
    stroke: hsla(0, 0%, 100%, 0.08);
    stroke-width: 1;
}
.radarRing:last-of-type { stroke: hsla(0, 0%, 100%, 0.14); }
.radarAxis {
    stroke: hsla(0, 0%, 100%, 0.07);
    stroke-width: 1;
}
.radarShape {
    fill: hsla(var(--accent-h), var(--accent-s), 58%, 0.28);
    stroke: hsla(var(--accent-h), var(--accent-s), 64%, 1);
    stroke-width: 2;
    stroke-linejoin: round;
    filter: drop-shadow(0 0 12px hsla(var(--accent-h), var(--accent-s), 55%, 0.45));
    transform-box: fill-box;
    transform-origin: center;
    animation: radarPop .6s cubic-bezier(0.22, 1, 0.36, 1);
}
@keyframes radarPop {
    from { transform: scale(0.6); opacity: 0; }
    to { transform: scale(1); opacity: 1; }
}
.radarDot {
    fill: #fff;
    stroke: hsla(var(--accent-h), var(--accent-s), 50%, 1);
    stroke-width: 2;
}
.radarLabel {
    fill: hsla(0, 0%, 100%, 0.9);
    font-size: 12px;
    font-weight: 700;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
}
.radarValue {
    fill: hsla(var(--accent-h), var(--accent-s), 74%, 0.95);
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
}

/* Hero stat cards */
.statCards {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 10px;
    margin-top: 22px;
}
.statCard {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: 14px 14px 13px;
    border-radius: 18px;
    background:
        radial-gradient(140% 120% at 100% 0%, hsla(var(--accent-h), var(--accent-s), 56%, 0.14), transparent 58%),
        hsla(0, 0%, 100%, 0.045);
    box-shadow: inset 0 1px 0 hsla(0, 0%, 100%, 0.05);
    overflow: hidden;
}
.statCardIcon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    margin-bottom: 10px;
    border-radius: 10px;
    background: hsla(var(--accent-h), var(--accent-s), 55%, 0.16);
    color: hsla(var(--accent-h), var(--accent-s), 74%, 1);
}
.statCardIcon svg { width: 17px; height: 17px; }
.statCardValue {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 16px;
    font-weight: 800;
    color: #fff;
    letter-spacing: -0.3px;
    font-variant-numeric: tabular-nums;
    line-height: 1.2;
}
.statCardLabel {
    margin-top: auto;
    padding-top: 4px;
    font-size: 11.5px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.45);
    user-select: none;
}

/* When you listen: daypart bar chart */
.daypartChart {
    display: flex;
    flex-direction: row;
    align-items: flex-end;
    gap: 12px;
    height: 110px;
}
.daypartCol {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    height: 100%;
}
.daypartBarTrack {
    flex: 1;
    width: 100%;
    display: flex;
    align-items: flex-end;
    justify-content: center;
}
.daypartBar {
    width: 100%;
    max-width: 56px;
    min-height: 4px;
    border-radius: 9px;
    background: hsla(0, 0%, 100%, 0.1);
    transition: height .7s cubic-bezier(0.22, 1, 0.36, 1), background .3s ease;
}
.daypartCol.peak .daypartBar {
    background-image: linear-gradient(to top, hsla(var(--accent-h), var(--accent-s), 46%, 1), hsla(var(--accent-h), var(--accent-s), 64%, 1));
    box-shadow: 0 6px 18px hsla(var(--accent-h), var(--accent-s), 50%, 0.35);
}
.daypartName {
    font-size: 12px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.45);
    user-select: none;
}
.daypartCol.peak .daypartName { color: hsla(0, 0%, 100%, 0.95); }

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
