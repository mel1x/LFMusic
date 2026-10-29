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

// Mood chips are split into two centered rows (3 + remainder).
const moodRowSplit = Math.ceil(moods.length / 2);
const moodRows = [moods.slice(0, moodRowSplit), moods.slice(moodRowSplit)];

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

// Background sine lines, shown while a track plays: the path is twice the
// 1200-wide viewBox and its period (200) divides 1200, so sliding by -1200 loops seamlessly.
function wavePath(amp: number): string {
    let d = `M0 50 Q 50 ${50 - amp} 100 50`;
    for (let x = 200; x <= 2400; x += 100) d += ` T ${x} 50`;
    return d;
}
const waveLines = [
    { d: wavePath(30), cls: 'w1' },
    { d: wavePath(18), cls: 'w2' },
];

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

// Sound character radar (SVG spider chart)
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
            <button class="statsBtn" @click="openStats" v-tooltip:bottom="'Ваш музыкальный профиль'">
                <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M3 3v16a2 2 0 0 0 2 2h16"/>
                    <path d="M18 17V9"/>
                    <path d="M13 17V5"/>
                    <path d="M8 17v-3"/>
                </svg>
            </button>

            <template v-if="hasLibrary">
                <div class="stageArea">
                    <svg class="waves" :class="{ live: isPlaying }" viewBox="0 0 1200 100" preserveAspectRatio="none" aria-hidden="true">
                        <path v-for="w in waveLines" :key="w.cls" :d="w.d" :class="['waveLine', w.cls]" />
                    </svg>
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
                    </div>
                </div>

                <div class="moodStrip">
                    <div class="moodStripLabel">Настроение</div>
                    <div class="moodChips">
                        <div v-for="(row, r) in moodRows" :key="r" class="moodChipsRow">
                            <button
                                v-for="mood in row"
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
                            <CloseIcon width="20" height="20" />
                        </button>

                        <div v-if="statsLoading" class="statsLoading">Загрузка музыкального профиля…</div>

                        <template v-else-if="hasTaste">
                            <div class="statsHead">
                                <div class="statsKicker">Музыкальный профиль</div>
                                <div class="statsTitle">Ваш характер</div>
                                <div class="statsSubtitle">{{ character }}.</div>
                            </div>

                            <div class="statsSection">
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
                                            <path d="M3 12h3l3 7 4-15 3 8h5"/>
                                        </svg>
                                    </span>
                                    <div class="statCardValue">{{ analyzedTracks.toLocaleString('ru-RU') }} треков</div>
                                    <div class="statCardLabel">Проанализировано</div>
                                </div>
                                <div class="statCard wide">
                                    <span class="statCardIcon">
                                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                            <circle cx="12" cy="12" r="9"/>
                                            <path d="M12 7v5l3.5 2"/>
                                        </svg>
                                    </span>
                                    <div class="statCardText">
                                        <div class="statCardValue">{{ summary ? formatTotalDuration(summary.total_listened_secs) : '0 мин' }}</div>
                                        <div class="statCardLabel">За музыкой</div>
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
    /* Ambient accent light: soft top/bottom glows + subtle vignette, all on accent vars so it follows the current track's color. */
    background-image:
        radial-gradient(120% 75% at 50% -12%, hsla(var(--accent-h), var(--accent-s), 62%, 0.20), transparent 60%),
        radial-gradient(100% 58% at 50% 116%, hsla(var(--accent-h), var(--accent-s), 55%, 0.17), transparent 62%),
        radial-gradient(72% 58% at 50% 44%, hsla(var(--accent-h), var(--accent-s), 55%, 0.06), transparent 75%),
        radial-gradient(140% 125% at 50% 50%, transparent 50%, hsla(var(--accent-h), min(var(--accent-s), 40%), 5%, 0.55) 100%);
    background-repeat: no-repeat;
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

/* Two slow sine lines running behind the play controls, fading in while a track plays. */
.waves {
    position: absolute;
    top: calc(50% + 52px);
    left: -28px;
    width: calc(100% + 56px);
    height: 150px;
    transform: translateY(-50%);
    pointer-events: none;
    opacity: 0;
    transition: opacity .8s ease;
    -webkit-mask-image: linear-gradient(90deg, transparent, #000 18%, #000 82%, transparent);
    mask-image: linear-gradient(90deg, transparent, #000 18%, #000 82%, transparent);
}
.waves.live { opacity: 1; }
.waveLine {
    fill: none;
    vector-effect: non-scaling-stroke;
    animation: waveSlide 12s linear infinite;
    animation-play-state: paused;
}
.waves.live .waveLine { animation-play-state: running; }
.waveLine.w1 { stroke: hsla(var(--accent-h), var(--accent-s), 65%, 0.22); stroke-width: 1.5; }
.waveLine.w2 { stroke: hsla(var(--accent-h), var(--accent-s), 75%, 0.13); stroke-width: 1; animation-duration: 8s; animation-direction: reverse; }
@keyframes waveSlide {
    from { transform: translateX(0); }
    to { transform: translateX(-1200px); }
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

/* Pinned to the bottom outside the flow, so the mix block centers on the whole page. */
.moodStrip {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 24px;
    z-index: 2;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
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
    flex-direction: column;
    align-items: center;
    gap: 8px;
    max-width: 620px;
}
.moodChipsRow {
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
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
    .moodStrip { bottom: 16px; }
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

.statsSection {
    margin-top: 28px;
}

.radarWrap {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    margin: -4px 0 2px;
}
/* Soft accent glow behind the radar, matching the page's orb style. */
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

/* Hero stat cards: top row has 2 cards, bottom row 1 wide card. */
.statCards {
    display: grid;
    grid-template-columns: 1fr 1fr;
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

/* Wide bottom card (listening time): icon on the left, text on the right. */
.statCard.wide {
    grid-column: 1 / -1;
    flex-direction: row;
    align-items: center;
    gap: 14px;
    padding: 15px 18px;
}
.statCard.wide .statCardIcon {
    margin-bottom: 0;
    width: 38px;
    height: 38px;
    border-radius: 12px;
}
.statCard.wide .statCardIcon svg { width: 20px; height: 20px; }
.statCardText {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
}
.statCard.wide .statCardLabel {
    margin-top: 0;
    padding-top: 0;
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

/* Phones: stats modal full-screen, no edge padding. */
@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .statsBackdrop { padding: 0; }
    .statsModal {
        width: 100%;
        max-width: none;
        height: 100%;
        max-height: none;
        border-radius: 0;
    }
}
</style>
