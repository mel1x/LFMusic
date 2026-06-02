<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { usePlayer, type TrackMetadata } from '../composables/usePlayer';
import { useCustomCovers } from '../composables/useCustomCovers';
import { useI18n } from '../composables/useI18n';
import {
    getRecentlyPlayed,
    getMostPlayed,
    getTopArtists,
    getTopAlbums,
    getSummaryStats,
    getRecommendations,
    TRACK_STATISTICS_UPDATED_EVENT,
    type TopArtist,
    type TopAlbum,
    type SummaryStats as StatsData
} from '../composables/useStatistics';
import defaultTrackCover from '../assets/trackCover.webp';
import defaultCover from '../assets/playlistCover.webp';
import PlayIcon from '../components/Icons/PlayIcon.vue';
import PauseIcon from '../components/Icons/PauseIcon.vue';
import SearchIcon from '../components/Icons/SearchIcon.vue';
import AnimatedCover from '../components/AnimatedCover.vue';
import MobileScrollHeader from '../components/MobileScrollHeader.vue';

const router = useRouter();
const { language } = useI18n();

const {
    trackList,
    getTrackByPath,
    loadCoversForTracks,
    isPlaying,
    currentTrack,
    activePlaylistSource,
    playTrack,
    togglePlayPause,
    setPlaylistSource,
} = usePlayer();

const { getCustomCover } = useCustomCovers();

// ── State variables ──────────────────────────────────────────────────────────
const recentlyPlayed = ref<TrackMetadata[]>([]);
const mostPlayed = ref<TrackMetadata[]>([]);
const topArtists = ref<TopArtist[]>([]);
const topAlbums = ref<TopAlbum[]>([]);
const summaryStats = ref<StatsData>({
    total_tracks: 0,
    total_folders: 0,
    total_listened_secs: 0,
    total_listens: 0,
    morning_listens: 0,
    day_listens: 0,
    evening_listens: 0,
    night_listens: 0,
});
const loading = ref(true);

// ── Horizontal scroll rows ───────────────────────────────────────────────────
function useScrollLine(baseItemWidth = 160, gap = 16) {
    const el = ref<HTMLElement | null>(null);
    const scrollLeft = ref(0);
    const scrollMax = ref(0);
    const itemWidth = ref(baseItemWidth);
    const measured = ref(false);
    let ro: ResizeObserver | null = null;

    async function update() {
        const node = el.value;
        if (!node) return;
        scrollLeft.value = node.scrollLeft;
        scrollMax.value = node.scrollWidth - node.clientWidth;
        if (node.clientWidth > 0 && !measured.value) {
            const count = Math.floor((node.clientWidth + gap) / (baseItemWidth + gap));
            const fits = Math.max(1, count);
            const w = Math.floor((node.clientWidth - (fits - 1) * gap) / fits);
            itemWidth.value = Math.max(baseItemWidth - 20, w);
            measured.value = true;
        }
    }

    function scrollByPage(dir: number) {
        const node = el.value;
        if (!node) return;
        const amount = (itemWidth.value + gap) * Math.max(1, Math.floor(node.clientWidth / (itemWidth.value + gap)));
        node.scrollBy({ left: amount * dir, behavior: 'smooth' });
    }

    function onScroll(e: Event) {
        scrollLeft.value = (e.target as HTMLElement).scrollLeft;
    }

    const canPrev = computed(() => scrollLeft.value > 2);
    const canNext = computed(() => scrollLeft.value < scrollMax.value - 2);

    onMounted(() => {
        const node = el.value;
        if (!node) return;
        ro = new ResizeObserver(() => {
            update();
        });
        ro.observe(node);
        node.addEventListener('scroll', update, { passive: true });
        update();
    });

    onUnmounted(() => {
        ro?.disconnect();
        el.value?.removeEventListener('scroll', update);
    });

    return {
        el,
        itemWidth,
        measured,
        canPrev,
        canNext,
        scrollByPage,
        onScroll,
        update,
    };
}

const recentLine = useScrollLine(160, 16);
const mostLine = useScrollLine(160, 16);

// ── Fetching Data ────────────────────────────────────────────────────────────
async function loadData() {
    try {
        const [recent, most, artists, albums, stats] = await Promise.all([
            getRecentlyPlayed(15),
            getMostPlayed(15),
            getTopArtists(5),
            getTopAlbums(5),
            getSummaryStats()
        ]);
        recentlyPlayed.value = recent;
        mostPlayed.value = most;
        topArtists.value = artists;
        topAlbums.value = albums;
        summaryStats.value = stats;
    } catch (e) {
        console.error('Ошибка загрузки статистики:', e);
    } finally {
        loading.value = false;
    }
}

// ── Cover Loader Fallbacks ────────────────────────────────────────────────────
function getCoverForTrack(track: TrackMetadata): string {
    const inList = getTrackByPath(track.file_path);
    if (inList?.cover_art) return inList.cover_art;
    if (track.cover_art) return track.cover_art;
    if (inList) {
        loadCoversForTracks([inList]);
    }
    return defaultTrackCover;
}

function getCoverForAlbum(album: TopAlbum): string {
    const custom = getCustomCover('album', album.name);
    if (custom) return custom;
    if (album.first_file_path) {
        const inList = getTrackByPath(album.first_file_path);
        if (inList?.cover_art) return inList.cover_art;
        if (inList) loadCoversForTracks([inList]);
    }
    return defaultTrackCover;
}

function getCoverForArtist(artist: TopArtist): string {
    const custom = getCustomCover('artist', artist.name);
    if (custom) return custom;
    const artistTracks = trackList.value.filter(t => t.artist === artist.name);
    if (artistTracks.length > 0) {
        const withCover = artistTracks.find(t => t.cover_art);
        if (withCover?.cover_art) return withCover.cover_art;
        loadCoversForTracks([artistTracks[0]]);
    }
    return defaultCover;
}

// ── Time of Day Greetings & Stats ─────────────────────────────────────────────
const timeOfDay = computed<'morning' | 'day' | 'evening' | 'night'>(() => {
    const hour = new Date().getHours();
    if (hour >= 5 && hour < 12) return 'morning';
    if (hour >= 12 && hour < 18) return 'day';
    if (hour >= 18 && hour < 23) return 'evening';
    return 'night';
});

const greetingText = computed(() => {
    const tod = timeOfDay.value;
    if (tod === 'morning') return 'Доброе утро';
    if (tod === 'day') return 'Добрый день';
    if (tod === 'evening') return 'Добрый вечер';
    return 'Доброй ночи';
});

function formatListenedTime(seconds: number): string {
    const mins = Math.floor(seconds / 60);
    const hrs = Math.floor(mins / 60);
    const isEn = language.value === 'en';
    if (hrs > 0) {
        const remainingMins = mins % 60;
        return isEn ? `${hrs} h ${remainingMins} min` : `${hrs} ч ${remainingMins} мин`;
    }
    return isEn ? `${mins} min` : `${mins} мин`;
}

// ── Habits Chart calculations ──────────────────────────────────────────────────
const maxListens = computed(() => {
    const stats = summaryStats.value;
    return Math.max(1, stats.morning_listens, stats.day_listens, stats.evening_listens, stats.night_listens);
});

const morningBarHeight = computed(() => (summaryStats.value.morning_listens / maxListens.value) * 100);
const dayBarHeight = computed(() => (summaryStats.value.day_listens / maxListens.value) * 100);
const eveningBarHeight = computed(() => (summaryStats.value.evening_listens / maxListens.value) * 100);
const nightBarHeight = computed(() => (summaryStats.value.night_listens / maxListens.value) * 100);

// ── Playback Controls ──────────────────────────────────────────────────────────
function isTrackPlaying(track: TrackMetadata): boolean {
    return currentTrack.value?.file_path === track.file_path && isPlaying.value;
}

function isTrackActive(track: TrackMetadata): boolean {
    return currentTrack.value?.file_path === track.file_path;
}

async function playTrackInQueue(track: TrackMetadata, sourceName: 'recently' | 'most') {
    const tracksList = sourceName === 'recently' ? recentlyPlayed.value : mostPlayed.value;
    if (isTrackActive(track)) {
        togglePlayPause();
        return;
    }
    const mapped = tracksList.map(t => getTrackByPath(t.file_path) ?? t);
    setPlaylistSource(`recommendation:${sourceName}`, mapped);
    await playTrack(getTrackByPath(track.file_path) ?? track);
}

// ── Mood Recommendations ───────────────────────────────────────────────────────
function isMoodActive(mood: string): boolean {
    return activePlaylistSource.value === `recommendation:mood:${mood}`;
}

async function playMood(mood: string) {
    if (isMoodActive(mood)) {
        togglePlayPause();
        return;
    }
    const recs = await getRecommendations(mood, 50);
    if (recs.length === 0) return;
    const mapped = recs.map(r => getTrackByPath(r.file_path) ?? r);
    setPlaylistSource(`recommendation:mood:${mood}`, mapped);
    await playTrack(mapped[0]);
}

// ── Lifecycle & Event Listening ───────────────────────────────────────────────
onMounted(() => {
    loadData();
    window.addEventListener(TRACK_STATISTICS_UPDATED_EVENT, loadData);
});

onUnmounted(() => {
    window.removeEventListener(TRACK_STATISTICS_UPDATED_EVENT, loadData);
});

// Update lines when track list loads to trigger scroll updates
watch(() => trackList.value.length, () => {
    recentLine.update();
    mostLine.update();
});

// ── Navigation helpers ────────────────────────────────────────────────────────
function goToCatalog() {
    router.push('/catalog');
}

// Sticky header scroll behavior
const pageRootEl = ref<HTMLElement | null>(null);
const scrollY = ref(0);
const showStickyHeader = computed(() => scrollY.value > 200);

function handleScroll(e: Event) {
    scrollY.value = (e.target as HTMLElement).scrollTop;
}

function scrollToTop() {
    pageRootEl.value?.scrollTo({ top: 0, behavior: 'smooth' });
}
</script>

<template>
    <div class="pageRoot" ref="pageRootEl" @scroll="handleScroll">
        <!-- Sticky Header -->
        <div class="stickyHeaderAnchor hasMobileScrollHeader">
            <MobileScrollHeader title="Главная" :scrolled="scrollY > 16" @top="scrollToTop" />
            <Transition name="sticky-header">
                <div v-if="showStickyHeader" class="stickyHeader">
                    <div>Главная</div>
                    <button class="toTopBtn" @click="scrollToTop">Вверх</button>
                </div>
            </Transition>
        </div>

        <div v-if="loading" class="loadingContainer">
            <div class="spinner"></div>
        </div>

        <div v-else-if="summaryStats.total_tracks === 0" class="emptyOnboarding">
            <div class="logoGlow"></div>
            <div class="emptyIcon"><SearchIcon width="36" height="36" /></div>
            <div class="emptyText">Добро пожаловать в LFMusic!</div>
            <div class="emptyDesc">
                Для работы плеера добавьте папки с музыкальными файлами в настройках или в каталоге, чтобы запустить сканирование медиатеки.
            </div>
            <button class="actionBtn" @click="goToCatalog">Добавить папки</button>
        </div>

        <div v-else class="dashboardContent">
            <!-- Top Welcome & Overview Header -->
            <div class="dashboard-header">
                <div class="welcome-text">
                    <h1>{{ greetingText }}</h1>
                    <p>Добро пожаловать в вашу музыкальную медиатеку</p>
                </div>
                <div class="stats-overview">
                    <div class="overview-item">
                        <span class="overview-val">{{ summaryStats.total_tracks }}</span>
                        <span class="overview-lbl">треков</span>
                    </div>
                    <div class="overview-sep"></div>
                    <div class="overview-item">
                        <span class="overview-val">{{ formatListenedTime(summaryStats.total_listened_secs) }}</span>
                        <span class="overview-lbl">времени</span>
                    </div>
                    <div class="overview-sep"></div>
                    <div class="overview-item">
                        <span class="overview-val">{{ summaryStats.total_listens }}</span>
                        <span class="overview-lbl">прослушиваний</span>
                    </div>
                </div>
            </div>

            <!-- Activity & Habits Row (No nested cards, flat and clean) -->
            <div class="activity-section">
                <div class="activity-details">
                    <div class="activity-info">
                        <span class="activity-title">Аналитика медиатеки</span>
                        <span class="activity-desc">Ваша музыкальная активность распределена по времени суток на основе истории воспроизведений. Всего просканировано {{ summaryStats.total_folders }} папок.</span>
                    </div>
                </div>
                <div class="habits-chart-flat">
                    <div class="habit-column" v-tooltip="`${summaryStats.morning_listens} прослушиваний`">
                        <div class="habit-gauge-container">
                            <div class="habit-gauge morning" :style="{ height: morningBarHeight + '%' }">
                                <div class="gauge-glow"></div>
                            </div>
                        </div>
                        <span class="habit-label">Утро</span>
                    </div>
                    <div class="habit-column" v-tooltip="`${summaryStats.day_listens} прослушиваний`">
                        <div class="habit-gauge-container">
                            <div class="habit-gauge day" :style="{ height: dayBarHeight + '%' }">
                                <div class="gauge-glow"></div>
                            </div>
                        </div>
                        <span class="habit-label">День</span>
                    </div>
                    <div class="habit-column" v-tooltip="`${summaryStats.evening_listens} прослушиваний`">
                        <div class="habit-gauge-container">
                            <div class="habit-gauge evening" :style="{ height: eveningBarHeight + '%' }">
                                <div class="gauge-glow"></div>
                            </div>
                        </div>
                        <span class="habit-label">Вечер</span>
                    </div>
                    <div class="habit-column" v-tooltip="`${summaryStats.night_listens} прослушиваний`">
                        <div class="habit-gauge-container">
                            <div class="habit-gauge night" :style="{ height: nightBarHeight + '%' }">
                                <div class="gauge-glow"></div>
                            </div>
                        </div>
                        <span class="habit-label">Ночь</span>
                    </div>
                </div>
            </div>

            <!-- Mood Recommendations Cards Grid -->
            <div class="section-container">
                <h2 class="section-header">Рекомендации под настроение</h2>
                <div class="mood-grid">
                    <div class="mood-card energetic" @click="playMood('energetic')" :class="{ active: isMoodActive('energetic') }">
                        <div class="mood-left">
                            <div class="mood-icon">
                                <svg class="mood-svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                    <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/>
                                </svg>
                            </div>
                            <div class="mood-text">
                                <div class="mood-name">Энергичный вайб</div>
                                <div class="mood-desc">Драйв и бодрость для активного дня</div>
                            </div>
                        </div>
                        <div class="mood-play-status">
                            <div v-if="isMoodActive('energetic') && isPlaying" class="bar-visualizer">
                                <div class="bar"></div><div class="bar"></div><div class="bar"></div>
                            </div>
                            <PauseIcon v-else-if="isMoodActive('energetic')" width="16" height="16" />
                            <PlayIcon v-else width="16" height="16" />
                        </div>
                    </div>

                    <div class="mood-card relax" @click="playMood('relax')" :class="{ active: isMoodActive('relax') }">
                        <div class="mood-left">
                            <div class="mood-icon">
                                <svg class="mood-svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M9.59 4.59A2 2 0 1 1 11 8H2m10.59 11.41A2 2 0 1 0 14 16H2m15.73-8.27A2.5 2.5 0 1 1 19.5 12H2"/>
                                </svg>
                            </div>
                            <div class="mood-text">
                                <div class="mood-name">Релакс и покой</div>
                                <div class="mood-desc">Спокойные мелодии для отдыха</div>
                            </div>
                        </div>
                        <div class="mood-play-status">
                            <div v-if="isMoodActive('relax') && isPlaying" class="bar-visualizer">
                                <div class="bar"></div><div class="bar"></div><div class="bar"></div>
                            </div>
                            <PauseIcon v-else-if="isMoodActive('relax')" width="16" height="16" />
                            <PlayIcon v-else width="16" height="16" />
                        </div>
                    </div>

                    <div class="mood-card dance" @click="playMood('dance')" :class="{ active: isMoodActive('dance') }">
                        <div class="mood-left">
                            <div class="mood-icon">
                                <svg class="mood-svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                    <circle cx="12" cy="12" r="10"/>
                                    <circle cx="12" cy="12" r="6"/>
                                    <circle cx="12" cy="12" r="2"/>
                                </svg>
                            </div>
                            <div class="mood-text">
                                <div class="mood-name">Танцевальный ритм</div>
                                <div class="mood-desc">Поднимите себе настроение и танцуйте</div>
                            </div>
                        </div>
                        <div class="mood-play-status">
                            <div v-if="isMoodActive('dance') && isPlaying" class="bar-visualizer">
                                <div class="bar"></div><div class="bar"></div><div class="bar"></div>
                            </div>
                            <PauseIcon v-else-if="isMoodActive('dance')" width="16" height="16" />
                            <PlayIcon v-else width="16" height="16" />
                        </div>
                    </div>

                    <div class="mood-card melancholic" @click="playMood('melancholic')" :class="{ active: isMoodActive('melancholic') }">
                        <div class="mood-left">
                            <div class="mood-icon">
                                <svg class="mood-svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>
                                </svg>
                            </div>
                            <div class="mood-text">
                                <div class="mood-name">Меланхолия</div>
                                <div class="mood-desc">Глубокие и эмоциональные треки</div>
                            </div>
                        </div>
                        <div class="mood-play-status">
                            <div v-if="isMoodActive('melancholic') && isPlaying" class="bar-visualizer">
                                <div class="bar"></div><div class="bar"></div><div class="bar"></div>
                            </div>
                            <PauseIcon v-else-if="isMoodActive('melancholic')" width="16" height="16" />
                            <PlayIcon v-else width="16" height="16" />
                        </div>
                    </div>

                    <div class="mood-card forgotten" @click="playMood('forgotten')" :class="{ active: isMoodActive('forgotten') }">
                        <div class="mood-left">
                            <div class="mood-icon">
                                <svg class="mood-svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M6 3h12l4 6-10 12L2 9z"/>
                                    <path d="M11 3 8 9l4 12 4-12-3-6"/>
                                    <path d="M2 9h20"/>
                                </svg>
                            </div>
                            <div class="mood-text">
                                <div class="mood-name">Забытые сокровища</div>
                                <div class="mood-desc">Треки, которые вы давно не слушали</div>
                            </div>
                        </div>
                        <div class="mood-play-status">
                            <div v-if="isMoodActive('forgotten') && isPlaying" class="bar-visualizer">
                                <div class="bar"></div><div class="bar"></div><div class="bar"></div>
                            </div>
                            <PauseIcon v-else-if="isMoodActive('forgotten')" width="16" height="16" />
                            <PlayIcon v-else width="16" height="16" />
                        </div>
                    </div>
                </div>
            </div>

            <!-- Recently Played horizontal scroll row -->
            <div class="pageLine" v-if="recentlyPlayed.length > 0">
                <div class="lineHat">
                    <div class="lineTitle">Недавнее прослушивание</div>
                    <div class="lineControls">
                        <button class="prevBtn" :disabled="!recentLine.canPrev.value" @click="recentLine.scrollByPage(-1)"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                        <button class="nextBtn" :disabled="!recentLine.canNext.value" @click="recentLine.scrollByPage(1)"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                    </div>
                </div>
                <div class="lineList" :ref="(el: any) => { recentLine.el.value = el as HTMLElement | null }" :style="{ '--card-w': recentLine.itemWidth.value + 'px', visibility: recentLine.measured.value ? 'visible' : 'hidden' }" @scroll="recentLine.onScroll">
                    <div v-for="track in recentlyPlayed" :key="track.file_path" class="trackItem" @click="playTrackInQueue(track, 'recently')">
                        <div class="coverWrap">
                            <AnimatedCover class="cover" :src="getCoverForTrack(track)" />
                            <div class="overlay" :class="{ visible: isTrackActive(track) }">
                                <button class="overlayPlayBtn" @click.stop="playTrackInQueue(track, 'recently')">
                                    <PauseIcon v-if="isTrackPlaying(track)" width="14" height="14" />
                                    <PlayIcon v-else width="14" height="14" style="margin-left:1px;" />
                                </button>
                            </div>
                        </div>
                        <div v-truncate class="trackTitle" :class="{ active: isTrackActive(track) }">{{ track.title || 'Без названия' }}</div>
                        <div v-truncate class="trackArtist">{{ track.artist || 'Неизвестный исполнитель' }}</div>
                    </div>
                </div>
            </div>

            <!-- Most Played horizontal scroll row -->
            <div class="pageLine" v-if="mostPlayed.length > 0">
                <div class="lineHat">
                    <div class="lineTitle">Самые прослушиваемые треки</div>
                    <div class="lineControls">
                        <button class="prevBtn" :disabled="!mostLine.canPrev.value" @click="mostLine.scrollByPage(-1)"><svg style="margin-right:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" transform="rotate(180 12 12)"/></svg></button>
                        <button class="nextBtn" :disabled="!mostLine.canNext.value" @click="mostLine.scrollByPage(1)"><svg style="margin-left:2px;" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg></button>
                    </div>
                </div>
                <div class="lineList" :ref="(el: any) => { mostLine.el.value = el as HTMLElement | null }" :style="{ '--card-w': mostLine.itemWidth.value + 'px', visibility: mostLine.measured.value ? 'visible' : 'hidden' }" @scroll="mostLine.onScroll">
                    <div v-for="track in mostPlayed" :key="track.file_path" class="trackItem" @click="playTrackInQueue(track, 'most')">
                        <div class="coverWrap">
                            <AnimatedCover class="cover" :src="getCoverForTrack(track)" />
                            <div class="overlay" :class="{ visible: isTrackActive(track) }">
                                <button class="overlayPlayBtn" @click.stop="playTrackInQueue(track, 'most')">
                                    <PauseIcon v-if="isTrackPlaying(track)" width="14" height="14" />
                                    <PlayIcon v-else width="14" height="14" style="margin-left:1px;" />
                                </button>
                            </div>
                        </div>
                        <div v-truncate class="trackTitle" :class="{ active: isTrackActive(track) }">{{ track.title || 'Без названия' }}</div>
                        <div v-truncate class="trackArtist">{{ track.artist || 'Неизвестный исполнитель' }}</div>
                    </div>
                </div>
            </div>

            <!-- Top Artists and Albums Section -->
            <div class="columns-section">
                <!-- Top Artists -->
                <div class="artists-col" v-if="topArtists.length > 0">
                    <h3 class="column-header">Популярные исполнители</h3>
                    <div class="artist-list">
                        <div v-for="artist in topArtists" :key="artist.name" class="artist-row" @click="router.push(`/artist/${encodeURIComponent(artist.name)}`)">
                            <div class="artist-avatar-wrap">
                                <AnimatedCover class="artist-avatar" :src="getCoverForArtist(artist)" />
                            </div>
                            <div class="artist-meta">
                                <div class="artist-name" v-truncate>{{ artist.name }}</div>
                                <div class="artist-listens">{{ artist.listen_count }} прослушиваний</div>
                            </div>
                        </div>
                    </div>
                </div>

                <!-- Top Albums -->
                <div class="albums-col" v-if="topAlbums.length > 0">
                    <h3 class="column-header">Популярные альбомы</h3>
                    <div class="album-list">
                        <div v-for="album in topAlbums" :key="album.name" class="album-row" @click="router.push(`/album/${encodeURIComponent(album.name)}`)">
                            <div class="album-art-wrap">
                                <AnimatedCover class="album-art" :src="getCoverForAlbum(album)" />
                            </div>
                            <div class="album-meta">
                                <div class="album-name" v-truncate>{{ album.name }}</div>
                                <div class="album-artist" v-truncate>{{ album.artist }}</div>
                                <div class="album-listens">{{ album.listen_count }} прослушиваний</div>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    </div>
</template>

<style scoped>
.pageRoot {
    padding: 20px 28px;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background-color: var(--light-bg);
    background-image: linear-gradient(to bottom, hsla(var(--accent-h), 42%, 70%, 0.08) 0px, hsla(var(--accent-h), 42%, 80%, 0.04) 95px, hsla(var(--accent-h), 42%, 70%, 0) 375px);
    background-repeat: no-repeat;
    background-attachment: local;
    overflow-y: scroll;
}

.pageRoot::-webkit-scrollbar {
    width: 6px;
}

.pageRoot::-webkit-scrollbar-track {
    background: transparent;
}

.pageRoot::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
}

.stickyHeaderAnchor {
    position: sticky;
    top: -20px;
    height: 0;
    z-index: 10;
    margin: 0 -28px;
}

.stickyHeader {
    user-select: none;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    padding: 18px 28px;
    background-color: hsla(var(--accent-h), var(--accent-s), 14%, 0.3);
    backdrop-filter: blur(12px);
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 22px;
    font-weight: 700;
    color: hsla(0, 0%, 100%, 0.9);
}

.sticky-header-enter-active,
.sticky-header-leave-active {
    transition: opacity 0.2s ease, transform 0.2s ease;
}

.sticky-header-enter-from,
.sticky-header-leave-to {
    opacity: 0;
    transform: translateY(-8px);
}

.toTopBtn {
    cursor: pointer;
    padding: 6px 16px;
    border-radius: 999px;
    background-color: hsla(0, 0%, 100%, 0.07);
    color: hsla(0, 0%, 100%, 1);
    font-size: 14px;
    font-weight: 500;
    transition: .24s ease;
}

.toTopBtn:hover {
    background-color: hsla(0, 0%, 100%, 0.12);
}

.loadingContainer {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
}

.spinner {
    width: 40px;
    height: 40px;
    border: 3px solid rgba(255, 255, 255, 0.06);
    border-top: 3px solid hsla(var(--accent-h), 85%, 65%, 0.95);
    border-radius: 50%;
    animation: spin 1s linear infinite;
}

@keyframes spin {
    0% { transform: rotate(0deg); }
    100% { transform: rotate(360deg); }
}

/* Onboarding view */
.emptyOnboarding {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 60px 20px;
    position: relative;
}

.logoGlow {
    position: absolute;
    width: 250px;
    height: 250px;
    background: radial-gradient(circle, hsla(var(--accent-h), 85%, 65%, 0.12) 0%, transparent 70%);
    z-index: 0;
    pointer-events: none;
}

.emptyIcon {
    position: relative;
    padding: 18px;
    border-radius: 50%;
    background-color: hsla(var(--accent-h), var(--accent-s), 50%, 0.06);
    color: hsla(var(--accent-h), 85%, 65%, 0.95);
    margin-bottom: 24px;
    z-index: 1;
}

.emptyText {
    position: relative;
    font-family: "Montserrat", "Inter", sans-serif;
    font-weight: 900;
    font-size: 26px;
    color: #ffffff;
    margin-bottom: 12px;
    z-index: 1;
}

.emptyDesc {
    position: relative;
    font-size: 14px;
    line-height: 1.5;
    color: hsla(0, 0%, 100%, 0.45);
    max-width: 420px;
    margin-bottom: 28px;
    z-index: 1;
}

.actionBtn {
    position: relative;
    cursor: pointer;
    font-family: "Montserrat", "Inter", sans-serif;
    font-weight: 700;
    font-size: 14px;
    padding: 12px 28px;
    border-radius: 999px;
    color: #ffffff;
    background: linear-gradient(135deg, hsla(var(--accent-h), 85%, 60%, 0.95) 0%, hsla(var(--accent-h), 85%, 50%, 0.95) 100%);
    box-shadow: 0 4px 15px hsla(var(--accent-h), 85%, 55%, 0.35);
    transition: all 0.28s ease;
    z-index: 1;
}

.actionBtn:hover {
    transform: translateY(-2px);
    box-shadow: 0 6px 20px hsla(var(--accent-h), 85%, 55%, 0.45);
    filter: brightness(1.08);
}

/* Dashboard content */
.dashboardContent {
    display: flex;
    flex-direction: column;
    gap: 36px;
    padding-bottom: 40px;
}

/* Dashboard Header */
.dashboard-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
    padding-bottom: 24px;
    gap: 24px;
}

@media (max-width: 900px) {
    .dashboard-header {
        flex-direction: column;
        align-items: flex-start;
        gap: 16px;
    }
}

.welcome-text h1 {
    font-family: "Montserrat", "Inter", sans-serif;
    font-weight: 900;
    font-size: 28px;
    color: #ffffff;
    margin: 0 0 6px 0;
    letter-spacing: -0.5px;
}

.welcome-text p {
    font-size: 13px;
    color: hsla(0, 0%, 100%, 0.45);
    margin: 0;
}

.stats-overview {
    display: flex;
    align-items: center;
    gap: 20px;
    background: rgba(255, 255, 255, 0.015);
    border: 1px solid rgba(255, 255, 255, 0.03);
    padding: 10px 20px;
    border-radius: 12px;
}

.overview-item {
    display: flex;
    flex-direction: column;
    align-items: center;
}

.overview-val {
    font-family: "Outfit", "Montserrat", sans-serif;
    font-weight: 800;
    font-size: 16px;
    color: hsla(var(--accent-h), 85%, 65%, 1);
}

.overview-lbl {
    font-size: 11px;
    color: hsla(0, 0%, 100%, 0.4);
    font-weight: 500;
}

.overview-sep {
    width: 1px;
    height: 24px;
    background-color: rgba(255, 255, 255, 0.06);
}

/* Activity & Habits section */
.activity-section {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background: rgba(255, 255, 255, 0.01);
    border: 1px solid rgba(255, 255, 255, 0.02);
    border-radius: 16px;
    padding: 24px;
    gap: 32px;
}

@media (max-width: 900px) {
    .activity-section {
        flex-direction: column;
        align-items: stretch;
        gap: 24px;
    }
}

.activity-details {
    flex: 1;
    min-width: 0;
}

.activity-info {
    display: flex;
    flex-direction: column;
    gap: 8px;
}

.activity-title {
    font-family: "Montserrat", "Inter", sans-serif;
    font-weight: 800;
    font-size: 16px;
    color: #ffffff;
    letter-spacing: 0.2px;
}

.activity-desc {
    font-size: 12px;
    line-height: 1.5;
    color: hsla(0, 0%, 100%, 0.45);
}

.habits-chart-flat {
    display: flex;
    gap: 20px;
    align-items: flex-end;
    height: 110px;
    flex-shrink: 0;
}

@media (max-width: 900px) {
    .habits-chart-flat {
        justify-content: space-around;
        height: 100px;
    }
}

.habit-column {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    height: 100%;
}

.habit-gauge-container {
    position: relative;
    width: 10px;
    height: 80px;
    background: rgba(255, 255, 255, 0.04);
    border-radius: 999px;
    overflow: hidden;
}

.habit-gauge {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    border-radius: 999px;
    transition: height 1.2s cubic-bezier(0.16, 1, 0.3, 1);
    min-height: 3px;
}

.habit-gauge.morning {
    background: linear-gradient(to top, hsl(35, 90%, 55%), hsl(45, 95%, 60%));
}

.habit-gauge.day {
    background: linear-gradient(to top, hsl(190, 90%, 55%), hsl(160, 95%, 50%));
}

.habit-gauge.evening {
    background: linear-gradient(to top, hsl(330, 90%, 55%), hsl(15, 95%, 55%));
}

.habit-gauge.night {
    background: linear-gradient(to top, hsl(260, 90%, 55%), hsl(230, 95%, 50%));
}

.gauge-glow {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 10px;
    border-radius: 50%;
    filter: blur(3px);
    opacity: 0.6;
}

.habit-gauge.morning .gauge-glow { background: hsl(45, 95%, 60%); }
.habit-gauge.day .gauge-glow { background: hsl(160, 95%, 50%); }
.habit-gauge.evening .gauge-glow { background: hsl(15, 95%, 55%); }
.habit-gauge.night .gauge-glow { background: hsl(230, 95%, 50%); }

.habit-label {
    font-size: 11px;
    font-weight: 600;
    color: hsla(0, 0%, 100%, 0.4);
    user-select: none;
}

.habit-column:hover .habit-label {
    color: hsla(0, 0%, 100%, 0.85);
}

.habit-column:hover .habit-gauge {
    filter: brightness(1.1) contrast(1.05);
}

/* Mood grid */
.section-container {
    display: flex;
    flex-direction: column;
    gap: 16px;
}

.section-header {
    font-family: "Montserrat", "Inter", sans-serif;
    font-weight: 900;
    font-size: 22px;
    color: #ffffff;
    margin: 0;
    letter-spacing: -0.3px;
}

.mood-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 16px;
}

.mood-card.energetic { --mood-color: 45, 100%, 60%; }
.mood-card.relax { --mood-color: 160, 90%, 55%; }
.mood-card.dance { --mood-color: 15, 95%, 55%; }
.mood-card.melancholic { --mood-color: 210, 95%, 60%; }
.mood-card.forgotten { --mood-color: 270, 90%, 65%; }

.mood-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-radius: 16px;
    background: rgba(255, 255, 255, 0.02);
    border: 1px solid rgba(255, 255, 255, 0.02);
    cursor: pointer;
    transition: all 0.28s cubic-bezier(0.4, 0, 0.2, 1);
    position: relative;
    overflow: hidden;
}

.mood-left {
    display: flex;
    align-items: center;
    gap: 16px;
}

.mood-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 38px;
    height: 38px;
    border-radius: 10px;
    background: hsla(var(--mood-color), 0.06);
    color: hsl(var(--mood-color));
    transition: all 0.28s ease;
}

.mood-svg {
    transition: transform 0.28s ease;
}

.mood-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
}

.mood-name {
    font-family: "Montserrat", "Inter", sans-serif;
    font-weight: 700;
    font-size: 15px;
    color: #ffffff;
}

.mood-desc {
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.45);
}

.mood-play-status {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.05);
    display: flex;
    align-items: center;
    justify-content: center;
    color: hsla(0, 0%, 100%, 0.8);
    opacity: 0;
    transform: scale(0.9);
    transition: all 0.24s ease;
}

.mood-card:hover .mood-play-status,
.mood-card.active .mood-play-status {
    opacity: 1;
    transform: scale(1);
}

.mood-card:hover {
    background: hsla(var(--mood-color), 0.04);
    border-color: hsla(var(--mood-color), 0.15);
    transform: translateY(-2px);
    box-shadow: 0 6px 20px hsla(var(--mood-color), 0.04);
}

.mood-card:hover .mood-icon {
    background: hsla(var(--mood-color), 0.12);
    transform: scale(1.05);
}

.mood-card:hover .mood-svg {
    transform: scale(1.1) rotate(4deg);
}

/* Specific mood glows */
.mood-card.active {
    background: hsla(var(--mood-color), 0.08);
    border-color: hsla(var(--mood-color), 0.35);
    box-shadow: 0 6px 24px hsla(var(--mood-color), 0.08);
}

/* Bar Visualizer */
.bar-visualizer {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 12px;
}

.bar-visualizer .bar {
    width: 2px;
    height: 100%;
    background-color: hsla(var(--accent-h), 85%, 65%, 1);
    animation: bounce 0.8s ease-in-out infinite alternate;
}

.bar-visualizer .bar:nth-child(2) {
    animation-delay: 0.2s;
}

.bar-visualizer .bar:nth-child(3) {
    animation-delay: 0.4s;
}

@keyframes bounce {
    0% { height: 2px; }
    100% { height: 12px; }
}

/* Horizontal line grids */
.pageLine {
    display: flex;
    flex-direction: column;
    margin-bottom: 8px;
}

.lineHat {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
}

.lineTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-weight: 800;
    font-size: 20px;
    color: #ffffff;
    user-select: none;
}

.lineControls {
    display: flex;
    flex-direction: row;
    gap: 8px;
}

.prevBtn, .nextBtn {
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.05);
    color: hsla(0, 0%, 100%, 0.8);
    transition: .24s ease;
}

.prevBtn:hover:not(:disabled), .nextBtn:hover:not(:disabled) {
    background-color: hsla(0, 0%, 100%, 0.1);
    color: hsla(0, 0%, 100%, 1);
}

.prevBtn:disabled, .nextBtn:disabled {
    cursor: default;
    opacity: 0.35;
}

.lineList {
    display: flex;
    flex-direction: row;
    gap: 16px;
    overflow-x: scroll;
    padding-bottom: 10px;
    scrollbar-width: none; /* Firefox */
}

.lineList::-webkit-scrollbar {
    display: none; /* Safari/Chrome */
}

.trackItem {
    width: var(--card-w);
    flex-shrink: 0;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 8px;
    user-select: none;
}

.coverWrap {
    position: relative;
    width: 100%;
    aspect-ratio: 1 / 1;
    border-radius: 12px;
    overflow: hidden;
    background-color: #202020;
}

.cover {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    transition: filter .24s ease;
}

.trackItem:hover .cover {
    filter: brightness(0.65);
}

.overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    transition: opacity .24s ease;
}

.trackItem:hover .overlay,
.overlay.visible {
    opacity: 1;
}

.overlayPlayBtn {
    cursor: pointer;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background-color: hsla(0, 0%, 100%, 0.95);
    color: hsla(var(--accent-h), var(--accent-s), 20%, 1);
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 4px 10px rgba(0,0,0,0.3);
    transition: transform 0.24s ease, background-color 0.24s ease;
}

.overlayPlayBtn:hover {
    transform: scale(1.08);
    background-color: #ffffff;
}

.trackTitle {
    font-weight: 600;
    font-size: 14px;
    color: hsla(0, 0%, 100%, 0.85);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.trackTitle.active {
    color: hsla(var(--accent-h), 85%, 65%, 1);
}

.trackArtist {
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.45);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

/* Two columns section */
.columns-section {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 32px;
}

@media (max-width: 768px) {
    .columns-section {
        grid-template-columns: 1fr;
    }
}

.column-header {
    font-family: "Montserrat", "Inter", sans-serif;
    font-weight: 900;
    font-size: 20px;
    color: #ffffff;
    margin: 0 0 16px 0;
    letter-spacing: -0.3px;
}

.artist-list, .album-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
}

.artist-row, .album-row {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 8px 12px;
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.01);
    cursor: pointer;
    transition: all 0.24s ease;
}

.artist-row:hover, .album-row:hover {
    background: rgba(255, 255, 255, 0.04);
}

.artist-avatar-wrap {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    overflow: hidden;
    background-color: #202020;
    flex-shrink: 0;
}

.artist-avatar {
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.artist-meta, .album-meta {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
}

.artist-name, .album-name {
    font-weight: 600;
    font-size: 14px;
    color: #ffffff;
}

.artist-listens, .album-listens {
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.4);
}

.album-art-wrap {
    width: 48px;
    height: 48px;
    border-radius: 8px;
    overflow: hidden;
    background-color: #202020;
    flex-shrink: 0;
}

.album-art {
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.album-artist {
    font-size: 12px;
    color: hsla(0, 0%, 100%, 0.55);
}
</style>