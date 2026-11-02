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
    getMostPlayed,
    getTrackStatistics,
    type RecommendationOptions,
    type TasteProfile,
    type SummaryStats,
} from '../composables/useStatistics';
import { timesCountText } from '../utils/format';
import defaultTrackCover from '../assets/trackCover.webp';
import { useI18n } from '../composables/useI18n';
import { openYearRecap, seasonalRecapYear } from '../composables/useYearRecap';
import { useSpectrum, SPECTRUM_BARS } from '../composables/useSpectrum';

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
    getTrackByPath,
} = usePlayer();

// Mix settings ("wave" style tuning)

const familiarityOptions: { id: Familiarity; label: string }[] = [
    { id: 'fresh', label: 'Незнакомое' },
    { id: 'balanced', label: 'Сбалансированно' },
    { id: 'favorite', label: 'Любимое' },
];

// Mix settings singleton persisted to localStorage: survives navigation and app restart.
const { selectedMood, familiarity, losslessOnly } = useMixSettings();
const settingsOpen = ref(false);
const isBuildingMix = ref(false);

const currentMood = computed(() => moods.find(m => m.id === selectedMood.value) ?? moods[0]);

// Background spectrum while music plays: mirrored, so the bass sits in the middle.
// Bars are written straight to the DOM every frame, outside Vue.
const AMBIENT_BARS = SPECTRUM_BARS * 2;
const ambientBox = ref<HTMLElement | null>(null);
const spectrum = useSpectrum();

function drawAmbient(levels: Float32Array) {
    const els = ambientBox.value?.children as HTMLCollectionOf<HTMLElement> | undefined;
    if (!els || els.length !== AMBIENT_BARS) return;
    for (let i = 0; i < SPECTRUM_BARS; i++) {
        const t = `scaleY(${Math.max(0.03, levels[i])})`;
        els[SPECTRUM_BARS - 1 - i].style.transform = t;
        els[SPECTRUM_BARS + i].style.transform = t;
    }
}

const isMixActive = computed(() => activePlaylistSource.value === 'mix');
const isMixPlaying = computed(() => isMixActive.value && isPlaying.value);

const hasLibrary = computed(() => catalogPaths.value.length > 0);

const mixOptions = computed<RecommendationOptions>(() => ({
    familiarity: familiarity.value,
    losslessOnly: losslessOnly.value || undefined,
}));

// Mix playback

/** Mix queue paths: recommendation engine results, or shuffled full catalog as fallback (paths only, resolved lazily via cache). */
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
    return [...catalogPaths.value].sort(() => Math.random() - 0.5);
}

/** Warm up covers for the first N queue paths (resolve to objects + load covers). */
async function warmCovers(paths: string[]) {
    const tracks = await fetchTracksByPaths(paths);
    if (tracks.length) loadCoversForTracks(tracks);
}

/**
 * Start/refresh the wave.
 * By default builds a fully new queue and plays its first track — shuffle and
 * mood changes also change the current track.
 * @param keepCurrent true — current track keeps playing, only the queue tail
 *                    is rebuilt (used for wave composition/lossless changes).
 */
async function startMix(mood: string, keepCurrent = false) {
    if (!hasLibrary.value || isBuildingMix.value) return;
    isBuildingMix.value = true;
    try {
        const fresh = await buildMixPaths(mood);
        if (fresh.length === 0) return;

        // Deterministic wave queue (no global shuffle).
        shuffled.value = false;

        if (keepCurrent && isMixActive.value && currentTrack.value && currentTrackIndex.value >= 0) {
            const keepPaths = playbackOrder.value.slice(0, currentTrackIndex.value + 1);
            const keepSet = new Set(keepPaths);
            const tail = fresh.filter(p => !keepSet.has(p));
            setMixSourceFromPaths([...keepPaths, ...tail]);
            void warmCovers(tail.slice(0, 24));
        } else {
            // If recommendations put the currently playing track first again,
            // move it off position 0 so the current track is guaranteed to change.
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

/** Shuffle — new recommendation queue and new current track. */
function onShuffleMix() {
    void startMix(selectedMood.value);
}

function selectMood(id: string) {
    selectedMood.value = id;
    // Mood change restarts the wave along with the current track.
    if (isMixActive.value) void startMix(id);
}

function selectFamiliarity(id: Familiarity) {
    familiarity.value = id;
    // Changing wave composition only affects the tail — current track keeps playing.
    if (isMixActive.value) void startMix(selectedMood.value, true);
}

function toggleLossless() {
    losslessOnly.value = !losslessOnly.value;
    if (isMixActive.value) void startMix(selectedMood.value, true);
}

const familiarityLabel = computed(() => familiarityOptions.find(o => o.id === familiarity.value)?.label ?? '');

// Statistics modal

const statsOpen = ref(false);
const statsLoading = ref(false);
const taste = ref<TasteProfile | null>(null);
const summary = ref<SummaryStats | null>(null);
const topTracks = ref<{ track: TrackMetadata; listens: number }[]>([]);

async function loadTopTracks() {
    const raw = await getMostPlayed(4);
    // Live cache objects, so lazily loaded covers show up.
    cacheTracks(raw);
    const list = raw.map(tr => getTrackByPath(tr.file_path) ?? tr);
    loadCoversForTracks(list);
    const counts = await Promise.all(list.map(tr =>
        getTrackStatistics(tr.file_path).then(st => st.listen_count).catch(() => 0)
    ));
    return list.map((track, i) => ({ track, listens: counts[i] }));
}

async function openStats() {
    statsOpen.value = true;
    statsLoading.value = true;
    try {
        const [tp, s, top] = await Promise.all([
            getTasteProfile().catch(() => null),
            getSummaryStats().catch(() => null),
            loadTopTracks().catch(() => []),
        ]);
        taste.value = tp;
        summary.value = s;
        topTracks.value = top;
    } finally {
        statsLoading.value = false;
    }
}

const hasTaste = computed(() => !!taste.value && taste.value.analyzed_tracks > 0);
const hasListens = computed(() => (summary.value?.total_listens ?? 0) > 0);
const analyzedTracks = computed(() => taste.value?.analyzed_tracks ?? 0);

const statsFacts = computed(() => {
    const s = summary.value;
    const secs = s?.total_listened_secs ?? 0;
    const hours = Math.floor(secs / 3600);
    return [
        hours > 0
            ? { value: hours.toLocaleString('ru-RU'), unit: t('stats.hoursShort'), label: 'За музыкой' }
            : { value: String(Math.floor(secs / 60)), unit: t('stats.minutesShort'), label: 'За музыкой' },
        { value: (s?.total_listens ?? 0).toLocaleString('ru-RU'), unit: '', label: 'Прослушиваний' },
        { value: (s?.total_tracks ?? 0).toLocaleString('ru-RU'), unit: '', label: t('stats.libraryTracks') },
        { value: analyzedTracks.value.toLocaleString('ru-RU'), unit: '', label: 'Проанализировано' },
    ];
});

// Listening by time of day: shares for the segmented bar, the peak is highlighted.
const dayParts = computed(() => {
    const s = summary.value;
    if (!s) return [];
    const parts = [
        { id: 'morning', label: t('stats.morning'), count: s.morning_listens },
        { id: 'day', label: t('stats.day'), count: s.day_listens },
        { id: 'evening', label: t('stats.evening'), count: s.evening_listens },
        { id: 'night', label: t('stats.night'), count: s.night_listens },
    ];
    const total = parts.reduce((sum, p) => sum + p.count, 0);
    if (total === 0) return [];
    const max = Math.max(...parts.map(p => p.count));
    return parts.map(p => ({
        ...p,
        share: p.count / total,
        pct: Math.round((p.count / total) * 100),
        peak: p.count === max,
    }));
});

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
        { key: 'Яркость', value: pct((t.spectral_centroid ?? 0) / 4000), display: `${pct((t.spectral_centroid ?? 0) / 4000)}%` },
        { key: 'Темп', value: pct((bpm - 50) / 140), display: `${Math.round(bpm)} BPM` },
        { key: 'Громкость', value: pct((loud + 30) / 30), display: `${loud.toFixed(1)} dB` },
    ];
});

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

// Year recap: only offered as a badge in December/January.
const seasonYear = seasonalRecapYear();

function onKeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    if (statsOpen.value) statsOpen.value = false;
    else if (settingsOpen.value) settingsOpen.value = false;
}

onMounted(() => {
    window.addEventListener('keydown', onKeydown);
    // The bars are hidden with reduced motion (see CSS): no need to tap the FFT for them.
    if (!window.matchMedia('(prefers-reduced-motion: reduce)').matches) spectrum.start(drawAmbient);
});

onUnmounted(() => {
    window.removeEventListener('keydown', onKeydown);
    spectrum.stop();
});
</script>

<template>
    <div class="mainRoot">
        <div class="pageRoot">
            <button class="statsBtn" @click="openStats" v-tooltip:bottom="'Ваш музыкальный профиль'">
                <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M3 3v16a2 2 0 0 0 2 2h16"/>
                    <path d="M18 17V9"/>
                    <path d="M13 17V5"/>
                    <path d="M8 17v-3"/>
                </svg>
            </button>
            <button v-if="seasonYear !== null && hasLibrary" class="recapBadge" @click="openYearRecap(seasonYear)">
                <span class="recapBadgeDot"></span>
                {{ t('recap.open', { year: seasonYear }) }}
            </button>

            <template v-if="hasLibrary">
                <div ref="ambientBox" class="ambientBars" :class="{ on: isPlaying }" aria-hidden="true">
                    <span v-for="i in AMBIENT_BARS" :key="i" class="ambientBar"></span>
                </div>

                <div class="stageArea">
                    <div class="mixStage">
                        <div class="mixHeader">
                            <div class="mixType">Персональная волна</div>
                            <h1 class="mixName">Мой микс</h1>
                            <Transition name="desc-fade" mode="out-in">
                                <p class="mixDesc" :key="currentMood.id">{{ currentMood.desc }}</p>
                            </Transition>
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

                        <div class="mixMeta">
                            <button class="badge" @click="settingsOpen = true" v-tooltip:bottom="'Настройки микса'">{{ familiarityLabel }}</button>
                            <button v-if="losslessOnly" class="badge" @click="settingsOpen = true" v-tooltip:bottom="'Настройки микса'">Lossless</button>
                        </div>

                        <!-- Fixed slot, so the stage doesn't jump when the mix starts. -->
                        <div class="mixNowSlot">
                            <Transition name="desc-fade">
                                <div v-if="isMixActive && currentTrack" class="mixNow">
                                    <img class="mixNowCover" :src="currentTrack.cover_art || defaultTrackCover" alt="" draggable="false" />
                                    <div class="mixNowText">
                                        <div class="mixNowLabel">{{ t('mix.nowPlaying') }}</div>
                                        <div class="mixNowTitle">{{ currentTrack.title || currentTrack.file_path.split(/[\\/]/).pop() }}<span v-if="currentTrack.artist" class="mixNowArtist"> - {{ currentTrack.artist }}</span></div>
                                    </div>
                                </div>
                            </Transition>
                        </div>
                    </div>
                </div>

                <div class="moodGrid">
                    <button
                        v-for="mood in moods"
                        :key="mood.id"
                        class="moodTile"
                        :class="{ active: selectedMood === mood.id }"
                        @click="selectMood(mood.id)"
                    >
                        <span class="moodTileIcon"><MoodIcon :name="mood.id" width="18" height="18" /></span>
                        <span class="moodTileName">{{ mood.label }}</span>
                        <span class="moodTileDesc">{{ mood.short_desc }}</span>
                    </button>
                </div>
            </template>

            <div v-else class="emptyState">
                <div class="emptyIcon"><CatalogIcon width="16" height="16" /></div>
                <div class="emptyText">Ваша библиотека пуста</div>
                <div class="emptyDesc">Добавьте папки с музыкой в каталоге, чтобы собрать персональный микс.</div>
                <button class="emptyBtn" @click="router.push('/catalog')">Открыть каталог</button>
            </div>
        </div>

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

        <Teleport to="body">
            <Transition name="stats">
                <div v-if="statsOpen" class="statsBackdrop" @click="statsOpen = false">
                    <div class="statsModal" @click.stop>
                        <button class="statsClose" @click="statsOpen = false" v-tooltip:left="'Закрыть'">
                            <CloseIcon width="18" height="18" />
                        </button>

                        <div v-if="statsLoading" class="statsLoading">Загрузка музыкального профиля…</div>

                        <template v-else-if="hasTaste || hasListens">
                            <div class="statsHead">
                                <div class="statsKicker">Музыкальный профиль</div>
                                <div class="statsTitle">Ваш характер</div>
                                <div v-if="character" class="statsSubtitle">{{ character }}.</div>
                            </div>

                            <div class="statsFacts">
                                <div v-for="f in statsFacts" :key="f.label" class="statsFact">
                                    <div class="statsFactValue">{{ f.value }}<span v-if="f.unit" class="statsFactUnit">{{ f.unit }}</span></div>
                                    <div class="statsFactLabel">{{ f.label }}</div>
                                </div>
                            </div>

                            <div class="statsGrid" :class="{ single: !hasTaste }">
                                <div v-if="hasTaste" class="statsCol">
                                    <div class="statsBlock">
                                        <div class="statsLabel">{{ t('stats.sound') }}</div>
                                        <div class="metricList">
                                            <div v-for="m in metrics" :key="m.key" class="metricRow">
                                                <div class="metricHead">
                                                    <span class="metricName">{{ m.key }}</span>
                                                    <span class="metricValue">{{ m.display }}</span>
                                                </div>
                                                <div class="metricTrack">
                                                    <span class="metricFill" :style="{ width: `${Math.max(m.value, 2)}%` }"></span>
                                                </div>
                                            </div>
                                        </div>
                                    </div>
                                </div>

                                <div class="statsCol">
                                    <div v-if="dayParts.length" class="statsBlock">
                                        <div class="statsLabel">{{ t('stats.dayTime') }}</div>
                                        <div class="dayBar">
                                            <span
                                                v-for="p in dayParts.filter(d => d.count > 0)"
                                                :key="p.id"
                                                class="daySeg"
                                                :class="[p.id, { peak: p.peak }]"
                                                :style="{ flexGrow: p.share }"
                                            ></span>
                                        </div>
                                        <div class="dayLegend">
                                            <div v-for="p in dayParts" :key="p.id" class="dayItem" :class="{ peak: p.peak }">
                                                <span class="dayDot" :class="p.id"></span>
                                                <span class="dayName">{{ p.label }}</span>
                                                <span class="dayPct">{{ p.pct }}%</span>
                                            </div>
                                        </div>
                                    </div>

                                    <div v-if="topTracks.length" class="statsBlock">
                                        <div class="statsLabel">{{ t('stats.topTracks') }}</div>
                                        <div class="topList">
                                            <div v-for="(item, i) in topTracks" :key="item.track.file_path" class="topRow">
                                                <span class="topRank">{{ i + 1 }}</span>
                                                <img
                                                    class="topCover"
                                                    :src="(getTrackByPath(item.track.file_path) ?? item.track).cover_art || defaultTrackCover"
                                                    alt=""
                                                    draggable="false"
                                                />
                                                <div class="topText">
                                                    <div class="topTitle">{{ item.track.title || item.track.file_path.split(/[\\/]/).pop() }}</div>
                                                    <div class="topArtist">{{ item.track.artist || t('recap.unknownArtist') }}</div>
                                                </div>
                                                <span v-if="item.listens > 0" class="topCount">{{ timesCountText(item.listens) }}</span>
                                            </div>
                                        </div>
                                    </div>
                                </div>
                            </div>
                        </template>

                        <div v-else class="statsEmpty">
                            Пока недостаточно данных. Слушайте музыку - и здесь появится ваш средний музыкальный характер,
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
    justify-content: flex-start;
    width: 100%;
    height: 100%;
    padding: 24px 28px;
    box-sizing: border-box;
    background-color: var(--light-bg);
    overflow: hidden;
}

/* Live spectrum behind the page: flat, faint accent bars that appear only while music plays. */
.ambientBars {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 46%;
    z-index: 0;
    display: flex;
    align-items: flex-end;
    gap: 6px;
    padding: 0 28px;
    box-sizing: border-box;
    pointer-events: none;
    opacity: 0;
    transition: opacity .6s ease;
}
.ambientBars.on {
    opacity: 1;
}
.ambientBar {
    flex: 1;
    height: 100%;
    border-radius: 8px 8px 0 0;
    background-color: hsla(var(--accent-h), var(--accent-s), 62%, 0.09);
    transform: scaleY(0.03);
    transform-origin: center bottom;
    will-change: transform;
}
@media (prefers-reduced-motion: reduce) {
    .ambientBars { display: none; }
}

.stageArea {
    position: relative;
    flex: 1;
    min-height: 0;
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
}

.statsBtn {
    position: absolute;
    top: 20px;
    right: 28px;
    z-index: 3;
    user-select: none;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.75);
    transition: .24s ease;
}
.statsBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.11);
    color: hsla(0, 0%, 100%, 1);
}

.recapBadge {
    position: absolute;
    top: 20px;
    right: 78px;
    z-index: 3;
    user-select: none;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    padding: 0 16px 0 14px;
    border: none;
    border-radius: 999px;
    font-size: 13px;
    font-weight: 700;
    color: #fff;
    background-color: hsla(var(--accent-h), var(--accent-s), 55%, 0.92);
    transition: background-color .2s ease;
}
.recapBadge:hover {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 1);
}
.recapBadgeDot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #fff;
    animation: recapBadgePulse 2.4s ease-in-out infinite;
}
@keyframes recapBadgePulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: .45; transform: scale(.7); }
}

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

.mixHeader {
    display: flex;
    flex-direction: column;
    align-items: center;
}
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
    font-size: 46px;
    font-weight: 900;
    color: hsla(0, 0%, 100%, 0.96);
    line-height: 1.04;
    letter-spacing: -1.2px;
    margin: 10px 0 0;
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

.mixMeta {
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    margin-top: 16px;
    max-width: 100%;
}
.badge {
    user-select: none;
    cursor: pointer;
    border: none;
    font-family: inherit;
    padding: 5px 12px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    font-size: 12px;
    font-weight: 600;
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

/* Current mix track: one small cover, shown only while the mix is the active source. */
.mixNowSlot {
    display: flex;
    justify-content: center;
    max-width: 100%;
    height: 52px;
    margin-top: 26px;
}
.mixNow {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 12px;
    max-width: 100%;
    padding: 6px 18px 6px 6px;
    border-radius: 999px;
    /* Opaque (--light-bg lifted a step), so the background spectrum stays behind it. */
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 19%);
    box-sizing: border-box;
    text-align: left;
}
.mixNowCover {
    width: 40px;
    height: 40px;
    flex-shrink: 0;
    border-radius: 50%;
    object-fit: cover;
    user-select: none;
    background-color: hsla(0, 0%, 100%, 0.06);
}
.mixNowText {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
}
.mixNowLabel {
    user-select: none;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: .08em;
    text-transform: uppercase;
    color: hsla(var(--accent-h), var(--accent-s), 72%, 0.85);
}
.mixNowTitle {
    max-width: 420px;
    font-size: 13.5px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.92);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.mixNowArtist {
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.5);
}

/* Mood tiles: the main way to steer the mix, so they get real size. */
.moodGrid {
    position: relative;
    z-index: 2;
    flex-shrink: 0;
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 10px;
    width: 100%;
    max-width: 920px;
}
.moodTile {
    user-select: none;
    cursor: pointer;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    padding: 16px 16px 15px;
    border: none;
    border-radius: 20px;
    /* Opaque like .mixNow: the background spectrum passes behind, not through. */
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 18.5%);
    color: hsla(0, 0%, 100%, 0.9);
    font-family: inherit;
    text-align: left;
    transition: background-color .2s ease, color .2s ease;
}
.moodTile:hover {
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 22%);
}
.moodTileIcon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    margin-bottom: 10px;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(var(--accent-h), var(--accent-s), 72%, 1);
    transition: background-color .2s ease, color .2s ease;
}
.moodTileName {
    max-width: 100%;
    font-size: 14px;
    font-weight: 700;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.moodTileDesc {
    max-width: 100%;
    font-size: 12px;
    font-weight: 500;
    line-height: 1.35;
    color: hsla(0, 0%, 100%, 0.45);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    min-height: calc(2 * 1.35em);
    transition: color .2s ease;
}
.moodTile.active {
    background-color: hsl(var(--accent-h), var(--accent-s), 52%);
    color: #fff;
}
.moodTile.active .moodTileIcon {
    background-color: hsla(0, 0%, 100%, 0.2);
    color: #fff;
}
.moodTile.active .moodTileDesc {
    color: hsla(0, 0%, 100%, 0.78);
}

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
    padding: 10px 18px 10px 10px;
    border: none;
    outline: none;
    box-shadow: none;
    border-radius: 999px;
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
    border-radius: 999px;
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
    gap: 2px;
    padding: 3px;
    border-radius: 999px;
    background-color: var(--dark-bg);
}
.segment {
    flex: 1;
    cursor: pointer;
    height: 34px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 10px;
    border: none;
    border-radius: 999px;
    background-color: transparent;
    color: hsla(var(--accent-h), calc(var(--accent-s) * 0.35), 84%, 0.58);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    transition: background-color .18s ease, color .18s ease;
}
.segment:hover { color: hsla(0, 0%, 100%, 0.82); }
.segment.active {
    background-color: hsla(0, 0%, 100%, 0.09);
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
    padding: 14px 20px;
    border: none;
    border-radius: 999px;
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

@media (max-height: 800px) {
    .mixName { font-size: 40px; }
}

@media (max-width: 1100px) and (min-width: 769px) {
    .mixName { font-size: 42px; }
}

/* Phone breakpoint (canonical for this app; see other views too). */
@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .pageRoot { padding: 16px 10px; }
    /* Phones: tiles shrink to icon + name chips that wrap. */
    .moodGrid {
        display: flex;
        flex-wrap: wrap;
        justify-content: center;
        gap: 8px;
    }
    .moodTile {
        flex-direction: row;
        align-items: center;
        gap: 8px;
        padding: 6px 14px 6px 6px;
        border-radius: 999px;
    }
    .moodTileIcon { width: 30px; height: 30px; margin-bottom: 0; }
    .moodTileName { font-size: 13px; font-weight: 600; }
    .moodTileDesc { display: none; }
    .mixNowTitle { max-width: 240px; }
    .statsBtn { top: 14px; right: 16px; min-height: 40px; }
    .mixName { font-size: 34px; letter-spacing: -0.8px; }
    /* Own padding so text doesn't hit the edges even with the page's small padding. */
    .mixDesc { font-size: 14px; max-width: 100%; padding: 0 18px; box-sizing: border-box; }
    .playListBtn { min-width: 150px; min-height: 42px; padding: 13px 22px; font-size: 13px; }
    .roundBtn { width: 46px; height: 46px; }
    /* Mix settings full-screen, no edge padding. */
    .settingsPanel {
        top: 0;
        right: 0;
        bottom: 0;
        left: 0;
        width: auto;
        max-width: none;
        min-width: 0;
        border-radius: 0;
    }
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
    background-color: hsla(0, 0%, 0%, 0.6);
    backdrop-filter: blur(8px);
    padding: 24px;
    box-sizing: border-box;
}
/* Flat surface like the other modals; sections are separated by spacing only. */
.statsModal {
    --stats-s: max(min(var(--accent-s), 70%), 35%);
    position: relative;
    width: 760px;
    max-width: 100%;
    max-height: 88vh;
    overflow-y: auto;
    padding: 28px 30px 26px;
    border-radius: 28px;
    background-color: var(--light-bg);
    box-shadow: 0 32px 90px hsla(0, 0%, 0%, 0.5);
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
    width: 32px;
    height: 32px;
    border: none;
    outline: none;
    box-shadow: none;
    border-radius: 9999px;
    background: hsla(0, 0%, 100%, 0.06);
    color: hsla(0, 0%, 100%, 0.7);
    transition: background-color .18s ease, color .18s ease;
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

.statsHead {
    display: flex;
    flex-direction: column;
    padding-right: 44px;
}
.statsKicker {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: .12em;
    text-transform: uppercase;
    color: hsla(var(--accent-h), min(var(--accent-s), 60%), 82%, 0.8);
    user-select: none;
}
.statsTitle {
    margin-top: 8px;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 26px;
    line-height: 1.15;
    font-weight: 800;
    letter-spacing: -0.01em;
    color: hsla(0, 0%, 100%, 0.96);
    user-select: none;
}
.statsSubtitle {
    margin-top: 6px;
    font-size: 14px;
    line-height: 1.5;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.6);
}
.statsLoading, .statsEmpty {
    padding-right: 44px;
    font-size: 14px;
    line-height: 1.55;
    color: hsla(0, 0%, 100%, 0.55);
}

/* Four numbers in one row. */
.statsFacts {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    column-gap: 24px;
    margin-top: 24px;
}
.statsFact {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
}
.statsFactValue {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 26px;
    line-height: 1.1;
    font-weight: 800;
    letter-spacing: -0.01em;
    color: hsla(0, 0%, 100%, 0.96);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.statsFact:first-child .statsFactValue {
    color: hsl(var(--accent-h), var(--stats-s), 80%);
}
.statsFactUnit {
    margin-left: 4px;
    font-size: 15px;
    font-weight: 700;
    opacity: 0.6;
}
.statsFactLabel {
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.5);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}

.statsGrid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    column-gap: 40px;
    margin-top: 34px;
}
.statsGrid.single {
    grid-template-columns: minmax(0, 1fr);
}
.statsCol {
    min-width: 0;
    display: flex;
    flex-direction: column;
}
.statsLabel {
    margin-bottom: 14px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: .1em;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.42);
    user-select: none;
}
.statsBlock + .statsBlock {
    margin-top: 24px;
}

/* Sound metrics: same thin accent bars as the time-of-day strip. */
.metricList {
    display: flex;
    flex-direction: column;
    gap: 28px;
}
.metricHead {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 8px;
}
.metricName {
    min-width: 0;
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.85);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}
.metricValue {
    flex-shrink: 0;
    font-size: 12px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.5);
    font-variant-numeric: tabular-nums;
}
.metricTrack {
    height: 6px;
    border-radius: 9999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    overflow: hidden;
}
.metricFill {
    display: block;
    height: 100%;
    border-radius: 9999px;
    background-color: hsl(var(--accent-h), var(--stats-s), 68%);
    transform-origin: left center;
    animation: daySegIn .5s cubic-bezier(.2, .8, .2, 1);
}

/* Time of day: light-to-dark accent tones, like the sun going down. */
.dayBar {
    display: flex;
    gap: 3px;
    height: 10px;
}
.daySeg {
    flex-basis: 0;
    min-width: 6px;
    border-radius: 3px;
    transform-origin: left center;
    animation: daySegIn .5s cubic-bezier(.2, .8, .2, 1);
}
.daySeg:first-child { border-radius: 9999px 3px 3px 9999px; }
.daySeg:last-child { border-radius: 3px 9999px 9999px 3px; }
@keyframes daySegIn {
    from { transform: scaleX(0.6); opacity: 0; }
    to { transform: scaleX(1); opacity: 1; }
}
.daySeg.morning, .dayDot.morning { background: hsl(var(--accent-h), var(--stats-s), 84%); }
.daySeg.day, .dayDot.day { background: hsl(var(--accent-h), var(--stats-s), 68%); }
.daySeg.evening, .dayDot.evening { background: hsl(var(--accent-h), var(--stats-s), 52%); }
.daySeg.night, .dayDot.night { background: hsl(var(--accent-h), var(--stats-s), 36%); }
.dayLegend {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 8px;
    margin-top: 14px;
}
.dayItem {
    min-width: 0;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: center;
    column-gap: 6px;
    row-gap: 3px;
}
.dayDot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
}
.dayName {
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.5);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
}
.dayPct {
    grid-column: 1 / -1;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 16px;
    font-weight: 800;
    color: hsla(0, 0%, 100%, 0.55);
    font-variant-numeric: tabular-nums;
}
.dayItem.peak .dayName { color: hsla(0, 0%, 100%, 0.85); }
.dayItem.peak .dayPct { color: hsla(0, 0%, 100%, 0.96); }

.topList {
    display: flex;
    flex-direction: column;
    gap: 12px;
}
.topRow {
    min-width: 0;
    display: grid;
    grid-template-columns: 14px 40px minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 12px;
}
.topRank {
    font-size: 12px;
    font-weight: 700;
    text-align: center;
    color: hsla(0, 0%, 100%, 0.35);
    font-variant-numeric: tabular-nums;
    user-select: none;
}
.topRow:first-child .topRank {
    color: hsl(var(--accent-h), var(--stats-s), 78%);
}
.topCover {
    width: 40px;
    height: 40px;
    border-radius: 6px;
    object-fit: cover;
    display: block;
    background: hsla(0, 0%, 100%, 0.06);
    user-select: none;
}
.topText {
    min-width: 0;
}
.topTitle, .topArtist {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.topTitle {
    font-size: 13.5px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.92);
}
.topArtist {
    margin-top: 2px;
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.5);
}
.topCount {
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.42);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
}

.stats-enter-active, .stats-leave-active { transition: opacity .2s ease; }
.stats-enter-active .statsModal, .stats-leave-active .statsModal { transition: transform .24s cubic-bezier(0.32, 0.72, 0, 1), opacity .24s ease; }
.stats-enter-from, .stats-leave-to { opacity: 0; }
.stats-enter-from .statsModal, .stats-leave-to .statsModal { transform: scale(0.96) translateY(6px); opacity: 0; }

/* Phones: stats modal full-screen, no edge padding, one column. */
@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .statsBackdrop { padding: 0; }
    .statsModal {
        width: 100%;
        max-width: none;
        height: 100%;
        max-height: none;
        border-radius: 0;
        padding: calc(var(--safe-top, 0px) + 26px) 20px calc(var(--safe-bottom, 0px) + 28px);
    }
    .statsClose { top: calc(var(--safe-top, 0px) + 18px); }
    .statsFacts {
        grid-template-columns: repeat(2, minmax(0, 1fr));
        row-gap: 18px;
    }
    .statsGrid {
        grid-template-columns: minmax(0, 1fr);
        row-gap: 32px;
    }
}
</style>
