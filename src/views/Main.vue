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
import ScannerBg from '../components/ScannerBg.vue';

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

const familiarityOptions: { id: Familiarity; label: string }[] = [
    { id: 'fresh', label: 'Незнакомое' },
    { id: 'balanced', label: 'Сбалансированно' },
    { id: 'favorite', label: 'Любимое' },
];

const { selectedMood, familiarity, losslessOnly } = useMixSettings();
const settingsOpen = ref(false);
const isBuildingMix = ref(false);

const currentMood = computed(() => moods.find(m => m.id === selectedMood.value) ?? moods[0]);

const scanner = ref<InstanceType<typeof ScannerBg> | null>(null);
const spectrum = useSpectrum();

function driveFlow(levels: Float32Array) {
    let bass = 0;
    for (let i = 0; i < 5; i++) bass += levels[i];
    bass /= 5;
    let all = 0;
    for (let i = 0; i < SPECTRUM_BARS; i++) all += levels[i];
    all /= SPECTRUM_BARS;
    scanner.value?.setEnergy(Math.min(1, bass * 0.7 + all * 0.3));
}

const isMixActive = computed(() => activePlaylistSource.value === 'mix');
const isMixPlaying = computed(() => isMixActive.value && isPlaying.value);

const hasLibrary = computed(() => catalogPaths.value.length > 0);

const mixOptions = computed<RecommendationOptions>(() => ({
    familiarity: familiarity.value,
    losslessOnly: losslessOnly.value || undefined,
}));

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

/** Starts/refreshes the wave; `keepCurrent` keeps the current track and rebuilds only the tail. */
async function startMix(mood: string, keepCurrent = false) {
    if (!hasLibrary.value || isBuildingMix.value) return;
    isBuildingMix.value = true;
    try {
        const fresh = await buildMixPaths(mood);
        if (fresh.length === 0) return;

        shuffled.value = false;

        if (keepCurrent && isMixActive.value && currentTrack.value && currentTrackIndex.value >= 0) {
            const keepPaths = playbackOrder.value.slice(0, currentTrackIndex.value + 1);
            const keepSet = new Set(keepPaths);
            const tail = fresh.filter(p => !keepSet.has(p));
            setMixSourceFromPaths([...keepPaths, ...tail]);
            void warmCovers(tail.slice(0, 24));
        } else {
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

/** Shuffle - new recommendation queue and new current track. */
function onShuffleMix() {
    void startMix(selectedMood.value);
}

function selectMood(id: string) {
    selectedMood.value = id;
    if (isMixActive.value) void startMix(id);
}

function selectFamiliarity(id: Familiarity) {
    familiarity.value = id;
    if (isMixActive.value) void startMix(selectedMood.value, true);
}

function toggleLossless() {
    losslessOnly.value = !losslessOnly.value;
    if (isMixActive.value) void startMix(selectedMood.value, true);
}

const familiarityLabel = computed(() => familiarityOptions.find(o => o.id === familiarity.value)?.label ?? '');

const statsOpen = ref(false);
const statsLoading = ref(false);
const taste = ref<TasteProfile | null>(null);
const summary = ref<SummaryStats | null>(null);
const topTracks = ref<{ track: TrackMetadata; listens: number }[]>([]);

async function loadTopTracks() {
    const raw = await getMostPlayed(4);
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

const STATS_ICONS = {
    time: '<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>',
    listens: '<path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"/>',
    library: '<path d="m16 6 4 14"/><path d="M12 6v14"/><path d="M8 8v12"/><path d="M4 4v16"/>',
    analyzed: '<path d="M2 10v3"/><path d="M6 6v11"/><path d="M10 3v18"/><path d="M14 8v7"/><path d="M18 5v13"/><path d="M22 10v3"/>',
    morning: '<path d="M12 2v8"/><path d="m4.93 10.93 1.41 1.41"/><path d="M2 18h2"/><path d="M20 18h2"/><path d="m19.07 10.93-1.41 1.41"/><path d="M22 22H2"/><path d="m8 6 4-4 4 4"/><path d="M16 18a4 4 0 0 0-8 0"/>',
    day: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/>',
    evening: '<path d="M12 10V2"/><path d="m4.93 10.93 1.41 1.41"/><path d="M2 18h2"/><path d="M20 18h2"/><path d="m19.07 10.93-1.41 1.41"/><path d="M22 22H2"/><path d="m16 6-4 4-4-4"/><path d="M16 18a4 4 0 0 0-8 0"/>',
    night: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
} as const;

const statsFacts = computed(() => {
    const s = summary.value;
    const secs = s?.total_listened_secs ?? 0;
    const hours = Math.floor(secs / 3600);
    return [
        hours > 0
            ? { value: hours.toLocaleString('ru-RU'), unit: t('stats.hoursShort'), label: 'За музыкой', icon: STATS_ICONS.time }
            : { value: String(Math.floor(secs / 60)), unit: t('stats.minutesShort'), label: 'За музыкой', icon: STATS_ICONS.time },
        { value: (s?.total_listens ?? 0).toLocaleString('ru-RU'), unit: '', label: 'Прослушиваний', icon: STATS_ICONS.listens },
        { value: (s?.total_tracks ?? 0).toLocaleString('ru-RU'), unit: '', label: t('stats.libraryTracks'), icon: STATS_ICONS.library },
        { value: analyzedTracks.value.toLocaleString('ru-RU'), unit: '', label: 'Проанализировано', icon: STATS_ICONS.analyzed },
    ];
});

const dayParts = computed(() => {
    const s = summary.value;
    if (!s) return [];
    const parts = [
        { id: 'morning', label: t('stats.morning'), count: s.morning_listens, icon: STATS_ICONS.morning },
        { id: 'day', label: t('stats.day'), count: s.day_listens, icon: STATS_ICONS.day },
        { id: 'evening', label: t('stats.evening'), count: s.evening_listens, icon: STATS_ICONS.evening },
        { id: 'night', label: t('stats.night'), count: s.night_listens, icon: STATS_ICONS.night },
    ];
    const total = parts.reduce((sum, p) => sum + p.count, 0);
    if (total === 0) return [];
    const max = Math.max(...parts.map(p => p.count));
    return parts.map(p => ({
        ...p,
        share: p.count / total,
        pct: Math.round((p.count / total) * 100),
        rel: max > 0 ? Math.round((p.count / max) * 100) : 0,
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

const seasonYear = seasonalRecapYear();

function onKeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    if (statsOpen.value) statsOpen.value = false;
    else if (settingsOpen.value) settingsOpen.value = false;
}

onMounted(() => {
    window.addEventListener('keydown', onKeydown);
    if (!window.matchMedia('(prefers-reduced-motion: reduce)').matches) spectrum.start(driveFlow);
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
                <div class="stageArea">
                    <div class="flowBg" aria-hidden="true">
                        <ScannerBg ref="scanner" />
                    </div>
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

                        <div v-if="statsLoading" class="statsLoading">Загрузка музыкального профиля...</div>

                        <template v-else-if="hasTaste || hasListens">
                            <div class="statsHead">
                                <div class="statsKicker">Музыкальный профиль</div>
                                <div class="statsTitle">Ваш характер</div>
                                <div v-if="character" class="statsSubtitle">{{ character }}.</div>
                            </div>

                            <div class="statsFacts">
                                <div v-for="f in statsFacts" :key="f.label" class="statsCard statsFact">
                                    <span class="statsIcon">
                                        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" v-html="f.icon"></svg>
                                    </span>
                                    <div class="statsFactValue">{{ f.value }}<span v-if="f.unit" class="statsFactUnit">{{ f.unit }}</span></div>
                                    <div class="statsFactLabel">{{ f.label }}</div>
                                </div>
                            </div>

                            <div class="statsGrid" :class="{ single: !hasTaste || !dayParts.length }">
                                <div v-if="hasTaste" class="statsCard">
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

                                <div v-if="dayParts.length" class="statsCard">
                                    <div class="statsLabel">{{ t('stats.dayTime') }}</div>
                                    <div class="dayGrid">
                                        <div v-for="p in dayParts" :key="p.id" class="dayTile" :class="{ peak: p.peak }">
                                            <svg class="dayIcon" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" v-html="p.icon"></svg>
                                            <div class="dayPct">{{ p.pct }}<span>%</span></div>
                                            <div class="dayName">{{ p.label }}</div>
                                            <div class="dayTrack"><span class="dayFill" :style="{ width: `${Math.max(p.rel, 3)}%` }"></span></div>
                                        </div>
                                    </div>
                                </div>

                                <div v-if="topTracks.length" class="statsCard statsWide">
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

.stageArea {
    position: relative;
    flex: 1;
    min-height: 0;
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
}

.flowBg {
    position: absolute;
    inset: 8px 0 12px;
    border-radius: 20px;
    overflow: hidden;
    pointer-events: none;
    z-index: 0;
    background-color: hsla(0, 0%, 100%, 0.022);
}
:global(body:not(.lfmusic-phone) .flowBg) {
    -webkit-mask:
        url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='84' height='84'%3E%3Cpath d='M0 0H11.16A18 18 0 0 1 29.1 19.41A33 33 0 0 0 64.59 54.9A18 18 0 0 1 84 72.84V84H0Z'/%3E%3C/svg%3E") right top / 84px 84px no-repeat,
        linear-gradient(#000 0 0) left top / calc(100% - 84px) 100% no-repeat,
        linear-gradient(#000 0 0) left bottom / 100% calc(100% - 84px) no-repeat;
    mask:
        url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='84' height='84'%3E%3Cpath d='M0 0H11.16A18 18 0 0 1 29.1 19.41A33 33 0 0 0 64.59 54.9A18 18 0 0 1 84 72.84V84H0Z'/%3E%3C/svg%3E") right top / 84px 84px no-repeat,
        linear-gradient(#000 0 0) left top / calc(100% - 84px) 100% no-repeat,
        linear-gradient(#000 0 0) left bottom / 100% calc(100% - 84px) no-repeat;
}

.statsBtn {
    position: absolute;
    top: 34px;
    right: 30px;
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
    transition: var(--duration-hover) var(--ease-smooth);
}
.statsBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.11);
    color: hsla(0, 0%, 100%, 1);
}

.recapBadge {
    position: absolute;
    top: 34px;
    right: 84px;
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
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.recapBadge:hover {
    background-color: hsla(var(--accent-h), var(--accent-s), 60%, 1);
}
.recapBadgeDot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #fff;
    animation: recapBadgePulse 2.4s var(--ease-smooth-in-out) infinite;
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
.desc-fade-leave-active { transition: opacity var(--duration-hover) var(--ease-smooth), transform var(--duration-hover) var(--ease-smooth); }
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
    transition: var(--duration-hover) var(--ease-smooth);
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
    height: 48px;
    padding: 0 26px;
    border: none;
    border-radius: 9999px;
    background-color: hsla(0, 0%, 100%, 1);
    color: hsla(var(--accent-h), var(--accent-s), 18%, 1);
    font-size: 14px;
    font-weight: 700;
    transition: var(--duration-hover) var(--ease-smooth);
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
    transition: var(--duration-hover) var(--ease-smooth);
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

.moodGrid {
    position: relative;
    z-index: 2;
    flex-shrink: 0;
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 10px;
    width: 100%;
}
.moodTile {
    user-select: none;
    cursor: pointer;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 3px;
    padding: 12px 14px 11px;
    border: none;
    border-radius: 20px;
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 18.5%);
    color: hsla(0, 0%, 100%, 0.9);
    font-family: inherit;
    text-align: left;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.moodTile:hover {
    background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 22%);
}
.moodTileIcon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    margin-bottom: 6px;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsla(var(--accent-h), var(--accent-s), 72%, 1);
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
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
    transition: color var(--duration-hover) var(--ease-smooth);
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
    transition: var(--duration-hover) var(--ease-smooth);
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
    transition: var(--duration-hover) var(--ease-smooth);
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
    transition: background var(--duration-hover) var(--ease-smooth);
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
    transition: background var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
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
    transition: color var(--duration-hover) var(--ease-smooth);
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
    transition: border-color var(--duration-hover) var(--ease-smooth);
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
    transition: transform var(--duration-hover) cubic-bezier(0.32, 0.72, 0, 1);
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
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
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
    transition: background var(--duration-hover) var(--ease-smooth);
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
    transition: background var(--duration-hover) var(--ease-smooth);
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
    transition: transform .22s var(--icon-ease);
}
.switch.on .knob { transform: translateX(18px); }



.panel-enter-active { transition: opacity .26s var(--ease-smooth); }
.panel-leave-active { transition: opacity .24s var(--ease-smooth-in-out); }
.panel-enter-active .settingsPanel { transition: transform .26s var(--ease-smooth); }
.panel-leave-active .settingsPanel { transition: transform .24s var(--ease-smooth-in-out); }
.panel-enter-from, .panel-leave-to { opacity: 0; }
.panel-enter-from .settingsPanel, .panel-leave-to .settingsPanel { transform: translateX(calc(100% + 12px)); }

@media (max-height: 800px) {
    .mixName { font-size: 40px; }
}

@media (max-width: 1100px) and (min-width: 769px) {
    .mixName { font-size: 42px; }
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .pageRoot { padding: 16px 10px; }
    /* Waves fill the whole page instead of a framed card. */
    .stageArea { position: static; }
    .flowBg {
        inset: 0;
        border-radius: 0;
        background-color: transparent;
        -webkit-mask-image: linear-gradient(to bottom, #000 0%, #000 72%, transparent 100%);
        mask-image: linear-gradient(to bottom, #000 0%, #000 72%, transparent 100%);
    }
    /* Mood picker: one dock of icons, the active mood opens into a labelled pill. */
    .moodGrid {
        display: flex;
        flex-wrap: nowrap;
        justify-content: center;
        gap: 4px;
        width: auto;
        max-width: 100%;
        margin-bottom: 8px;
        padding: 6px;
        border-radius: 999px;
        background-color: hsl(var(--accent-h), min(var(--accent-s), 45%), 14%);
    }
    .moodTile {
        flex: 0 0 auto;
        flex-direction: row;
        align-items: center;
        gap: 0;
        height: 44px;
        min-width: 44px;
        padding: 0;
        border-radius: 999px;
        background-color: transparent;
        color: hsla(0, 0%, 100%, 0.7);
    }
    .moodTile:hover { background-color: transparent; }
    .moodTileIcon {
        width: 44px;
        height: 44px;
        margin-bottom: 0;
        background-color: transparent;
        color: inherit;
    }
    .moodTileName {
        max-width: 0;
        opacity: 0;
        font-size: 13px;
        font-weight: 600;
        transition: max-width .32s var(--ease-smooth), opacity .2s var(--ease-smooth), padding .32s var(--ease-smooth);
    }
    .moodTileDesc { display: none; }
    .moodTile.active {
        background-color: hsl(var(--accent-h), min(var(--accent-s), 70%), 52%);
        color: #fff;
    }
    .moodTile.active .moodTileIcon {
        background-color: transparent;
        color: #fff;
    }
    .moodTile.active .moodTileName {
        max-width: 130px;
        opacity: 1;
        padding-right: 16px;
    }
    .mixNowTitle { max-width: 240px; }
    .statsBtn { top: 14px; right: 16px; min-height: 40px; }
    .mixName { font-size: 34px; letter-spacing: -0.8px; }
    .mixDesc { font-size: 14px; max-width: 100%; padding: 0 18px; box-sizing: border-box; }
    .playListBtn { min-width: 150px; min-height: 42px; padding: 13px 22px; font-size: 13px; }
    .roundBtn { width: 46px; height: 46px; }
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
.statsModal {
    --stats-s: max(min(var(--accent-s), 70%), calc(35% * var(--accent-k, 1)));
    position: relative;
    width: 760px;
    max-width: 100%;
    max-height: 88vh;
    overflow-y: auto;
    padding: 24px 26px;
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
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
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
    gap: 6px;
    padding-right: 48px;
}
.statsKicker {
    user-select: none;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 1.2px;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.42);
}
.statsTitle {
    user-select: none;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 30px;
    font-weight: 800;
    line-height: 1.1;
    color: #fff;
}
.statsSubtitle {
    font-size: 14px;
    font-weight: 500;
    line-height: 1.4;
    color: hsl(var(--accent-h), min(var(--stats-s), 60%), 80%);
}
.statsLoading, .statsEmpty {
    padding: 48px 24px;
    text-align: center;
    font-size: 14px;
    font-weight: 500;
    line-height: 1.5;
    color: hsla(0, 0%, 100%, 0.55);
}

.statsCard {
    min-width: 0;
    padding: 16px 18px;
    border-radius: 22px;
    background-color: hsla(0, 0%, 100%, 0.035);
    box-sizing: border-box;
}
.statsLabel {
    user-select: none;
    margin-bottom: 12px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 1px;
    text-transform: uppercase;
    color: hsla(0, 0%, 100%, 0.42);
}

.statsFacts {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 10px;
    margin-top: 18px;
}
.statsFact {
    display: flex;
    flex-direction: column;
    padding: 12px 14px 13px;
}
.statsIcon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    margin-bottom: 10px;
    border-radius: 10px;
    background-color: hsla(0, 0%, 100%, 0.06);
    color: hsl(var(--accent-h), min(var(--stats-s), 70%), 80%);
}
.statsFactValue {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 24px;
    font-weight: 800;
    line-height: 1.1;
    color: #fff;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.statsFactUnit {
    margin-left: 3px;
    font-size: 14px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.5);
}
.statsFactLabel {
    margin-top: 4px;
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.48);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.statsGrid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
    margin-top: 10px;
}
.statsGrid.single {
    grid-template-columns: minmax(0, 1fr);
}
.statsWide {
    grid-column: 1 / -1;
}

.metricList {
    display: flex;
    flex-direction: column;
    gap: 11px;
}
.metricHead {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 6px;
}
.metricName {
    font-size: 13px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.82);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.metricValue {
    flex-shrink: 0;
    font-size: 12px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.5);
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
}

.dayGrid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
}
.dayTile {
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 10px 12px 12px;
    border-radius: 16px;
    background-color: hsla(0, 0%, 100%, 0.035);
}
.dayTile.peak {
    background-color: hsla(var(--accent-h), var(--stats-s), 60%, 0.12);
}
.dayIcon {
    color: hsla(0, 0%, 100%, 0.45);
}
.dayTile.peak .dayIcon {
    color: hsl(var(--accent-h), var(--stats-s), 78%);
}
.dayPct {
    margin-top: 8px;
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 22px;
    font-weight: 800;
    line-height: 1;
    color: hsla(0, 0%, 100%, 0.82);
    font-variant-numeric: tabular-nums;
}
.dayPct span {
    margin-left: 1px;
    font-size: 13px;
    color: hsla(0, 0%, 100%, 0.45);
}
.dayTile.peak .dayPct {
    color: #fff;
}
.dayName {
    margin-top: 4px;
    font-size: 12px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.5);
}
.dayTrack {
    margin-top: 8px;
    height: 6px;
    border-radius: 9999px;
    background-color: hsla(0, 0%, 100%, 0.06);
    overflow: hidden;
}
.dayFill {
    display: block;
    height: 100%;
    border-radius: 9999px;
    background-color: hsla(0, 0%, 100%, 0.3);
}
.dayTile.peak .dayFill {
    background-color: hsl(var(--accent-h), var(--stats-s), 68%);
}

.topList {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 2px 14px;
}
.topRow {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
    padding: 6px;
    border-radius: 14px;
}
.topRank {
    flex-shrink: 0;
    width: 16px;
    text-align: center;
    font-size: 13px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.35);
}
.topRow:first-child .topRank {
    color: hsl(var(--accent-h), var(--stats-s), 75%);
}
.topCover {
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    border-radius: 9px;
    object-fit: cover;
    background-color: hsla(0, 0%, 100%, 0.06);
}
.topText {
    flex: 1;
    min-width: 0;
}
.topTitle, .topArtist {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.topTitle {
    font-size: 14px;
    font-weight: 600;
    color: #fff;
}
.topArtist {
    margin-top: 2px;
    font-size: 12px;
    font-weight: 500;
    color: hsla(0, 0%, 100%, 0.48);
}
.topCount {
    flex-shrink: 0;
    font-size: 12px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: hsla(0, 0%, 100%, 0.45);
}

.stats-enter-active, .stats-leave-active { transition: opacity var(--duration-modal) var(--ease-smooth); }
.stats-enter-active .statsModal, .stats-leave-active .statsModal { transition: transform var(--duration-modal) var(--ease-smooth), opacity var(--duration-modal) var(--ease-smooth); }
.stats-enter-from, .stats-leave-to { opacity: 0; }
.stats-enter-from .statsModal, .stats-leave-to .statsModal { transform: translateY(8px) scale(.98); opacity: 0; }

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .statsBackdrop { padding: 0; }
    .statsModal {
        width: 100%;
        max-width: none;
        height: 100%;
        max-height: none;
        border-radius: 0;
        padding: calc(var(--safe-top, 0px) + 26px) 16px calc(var(--safe-bottom, 0px) + 28px);
    }
    .statsClose { top: calc(var(--safe-top, 0px) + 18px); }
    .statsTitle { font-size: 26px; }
    .statsFacts {
        grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .statsGrid {
        grid-template-columns: minmax(0, 1fr);
    }
    .topList {
        grid-template-columns: minmax(0, 1fr);
    }
    .statsCard {
        padding: 16px;
    }
}
</style>
