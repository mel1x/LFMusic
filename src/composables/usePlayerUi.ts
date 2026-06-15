import { ref, computed, watch, watchEffect, effectScope } from "vue";
import { invoke } from "@tauri-apps/api/core";

import { usePlayer, type TrackMetadata } from "./usePlayer";
import { usePlaylists } from "./usePlaylists";
import { useAlbumTrackOrder } from "./useAlbumTrackOrder";
import { useI18n } from "./useI18n";
import { isCoverResolved } from "./useCoverLoader";

// ──────────────────────────────────────────────────────────────────────────────
// Общее UI-состояние плеера, разделяемое между мини-баром (PlaybackControls) и
// полноэкранным оверлеем (PlayerFullscreenOverlay): режим оверлея, кроссфейд
// обложек, очередь, подписи источника, перенос трека «в начало», громкость.
//
// Раньше всё это жило локальными ref внутри монолитного Player.vue. После
// разбиения на под-компоненты состояние должно быть общим, поэтому вынесено в
// composable-синглтон (тот же паттерн effectScope, что и usePlayer/usePins).
// ──────────────────────────────────────────────────────────────────────────────

type FsMode = null | "player" | "queue";

const QUEUE_DISPLAY_COUNT = 7;

let playerUiInstance: ReturnType<typeof createPlayerUi> | null = null;

export function usePlayerUi() {
    if (!playerUiInstance) {
        playerUiInstance = effectScope(true).run(createPlayerUi)!;
    }
    return playerUiInstance;
}

function createPlayerUi() {
    const {
        currentTrack,
        currentCoverArt,
        shuffled,
        volume,
        setVolume,
        playbackOrder,
        currentTrackIndex,
        activePlaylistSource,
        sourcePaths,
        sourceTrackList,
        getTrackByPath,
        fetchTracksByPaths,
        moveCatalogTrackToTop,
        syncPlaybackOrder,
        loadCoversForTracks,
    } = usePlayer();
    const { playlists, loadPlaylists } = usePlaylists();
    const { t } = useI18n();

    // ── Подписи источника очереди ───────────────────────────────────────────
    const queueSourcePrefix = computed(() => {
        const src = activePlaylistSource.value;
        if (!src) return "Сейчас играет из";
        if (src.startsWith("catalog")) return "Сейчас играет из";
        if (src.startsWith("album:")) return "Сейчас играет из альбома";
        if (src.startsWith("artist:")) return "Сейчас играет из подборки артиста";
        if (src.startsWith("playlist:")) return "Сейчас играет из плейлиста";
        if (src.startsWith("search:")) return "Сейчас играет из";
        if (src.startsWith("mix")) return "Сейчас играет из";
        return "Сейчас играет из";
    });

    const queueSourceName = computed(() => {
        const src = activePlaylistSource.value;
        if (!src) return "Мой каталог";
        if (src.startsWith("catalog")) return "Мой каталог";
        if (src === "mix") return "Мой микс";
        if (src.startsWith("album:")) {
            const name = src.slice("album:".length);
            return name || "Без названия";
        }
        if (src.startsWith("artist:")) {
            const name = src.slice("artist:".length);
            return name || "Неизвестный исполнитель";
        }
        if (src.startsWith("playlist:")) {
            const id = Number(src.slice("playlist:".length));
            const pl = playlists.value.find(p => p.id === id);
            return pl ? pl.name : "Без названия";
        }
        if (src.startsWith("search:")) return "Поиска по каталогу";
        return "Моей каталог";
    });

    // Подпись источника для Discord — как queueSourceName, но в именительном
    // падеже (queueSourceName согласован с префиксом «Сейчас играет из…»).
    const discordSourceLabel = computed(() => {
        const src = activePlaylistSource.value;
        if (src?.startsWith("search:")) return "Поиск по каталогу";
        return queueSourceName.value;
    });

    watch(
        () => activePlaylistSource.value,
        (src) => {
            if (src && src.startsWith("playlist:") && playlists.value.length === 0) {
                loadPlaylists();
            }
        },
        { immediate: true },
    );

    // ── Fullscreen mode ─────────────────────────────────────────────────────
    const fsMode = ref<FsMode>(null);
    const showOverlay = computed(() => fsMode.value !== null);
    const isQueueMode = computed(() => fsMode.value === "queue");

    function openFullscreen() {
        if (currentTrack.value) fsMode.value = "player";
    }

    function closeFullscreen() {
        fsMode.value = null;
    }

    function openQueueFullscreen() {
        if (currentTrack.value) fsMode.value = "queue";
    }

    function toggleFsMode() {
        if (!currentTrack.value) return;
        fsMode.value = fsMode.value === "queue" ? "player" : "queue";
    }

    // ── Fullscreen cover crossfade ──────────────────────────────────────────
    const fsCoverA = ref<string | null>(null);
    const fsCoverB = ref<string | null>(null);
    const fsActiveBuffer = ref<"a" | "b">("a");

    // Размытый фон полноэкранного плеера декодирует обложку на весь вьюпорт — для
    // него берём миниатюру низкого разрешения (size=blur ≈ 96px) вместо 512px:
    // картинка всё равно размывается на 80px, а декод и память кратно меньше.
    function toBlurCover(url: string | null): string {
        if (!url) return "";
        return url.replace(/size=\d+/, "size=blur");
    }

    watch(currentCoverArt, (newCover) => {
        if (!showOverlay.value) {
            fsCoverA.value = newCover;
            fsCoverB.value = newCover;
            fsActiveBuffer.value = "a";
            return;
        }
        if (fsActiveBuffer.value === "a") {
            fsCoverB.value = newCover;
            fsActiveBuffer.value = "b";
        } else {
            fsCoverA.value = newCover;
            fsActiveBuffer.value = "a";
        }
    }, { immediate: true });

    watch(showOverlay, (val) => {
        if (val) {
            fsCoverA.value = currentCoverArt.value;
            fsCoverB.value = currentCoverArt.value;
            fsActiveBuffer.value = "a";
        }
    });

    // ── Источник очереди / перенос «в начало» ───────────────────────────────
    const isMixSource = computed(() => activePlaylistSource.value === "mix");
    const isCatalogSource = computed(() => (activePlaylistSource.value ?? "").startsWith("catalog"));
    const isAlbumSource = computed(() => (activePlaylistSource.value ?? "").startsWith("album:"));
    const isPlaylistSource = computed(() => (activePlaylistSource.value ?? "").startsWith("playlist:"));
    const canMoveToTop = computed(() => isCatalogSource.value || isAlbumSource.value || isPlaylistSource.value);

    // Пути предстоящих треков (playbackOrder теперь хранит file_path).
    const queueUpcomingPaths = computed<string[]>(() => {
        const order = playbackOrder.value;
        const idx = currentTrackIndex.value;
        if (idx < 0 || order.length === 0) return [];

        if (isMixSource.value) {
            // «Мой микс» (Моя волна): показываем только следующий трек.
            return order.slice(idx + 1, idx + 2);
        }

        if (shuffled.value) {
            // Шаффл: хвост playbackOrder уже содержит заранее сгенерированные
            // решафл-проходы (ensureLookahead держит минимум QUEUE_DISPLAY_COUNT треков).
            return order.slice(idx + 1, idx + 1 + QUEUE_DISPLAY_COUNT);
        }

        // Без шаффла: зацикливаем очередь
        const result: string[] = [];
        for (let i = 1; i <= QUEUE_DISPLAY_COUNT; i++) {
            result.push(order[(idx + i) % order.length]);
        }
        return result;
    });

    // Резолв путей очереди в объекты через кэш (промахи дозагружаются фоном,
    // computed пересчитается, когда метаданные приедут).
    const queueUpcoming = computed<TrackMetadata[]>(() =>
        queueUpcomingPaths.value
            .map(p => getTrackByPath(p))
            .filter((t): t is TrackMetadata => !!t),
    );

    // Префетч метаданных всех предстоящих путей (чтобы очередь не была неполной).
    watch(queueUpcomingPaths, (paths) => {
        if (paths.length) void fetchTracksByPaths(paths);
    }, { immediate: true });

    const nextTrack = computed(() => queueUpcoming.value[0] ?? null);

    const nextTrackTitle = computed(() => {
        const track = nextTrack.value;
        if (!track) return t("player.noTrack");
        return track.title || track.file_path.split(/[/\\]/).pop()?.replace(/\.[^/.]+$/, "") || t("player.untitled");
    });

    // ── Отложенная замена очереди (прогрузка обложек перед показом) ──────────
    // Чтобы при включении shuffle очередь не моргала чёрными обложками, сначала
    // прогружаем обложки новых треков и только потом визуально заменяем список.
    // `displayQueue` — то, что реально рендерится; оно отстаёт от `queueUpcoming`,
    // пока обложки не будут готовы (с предохранительным таймаутом).
    const displayQueue = ref<TrackMetadata[]>([]);
    const pendingQueue = ref<TrackMetadata[] | null>(null);
    let queueSwapTimeout: ReturnType<typeof setTimeout> | null = null;

    function clearQueueSwapTimeout() {
        if (queueSwapTimeout) {
            clearTimeout(queueSwapTimeout);
            queueSwapTimeout = null;
        }
    }

    function commitQueue(tracks: TrackMetadata[]) {
        displayQueue.value = tracks;
        pendingQueue.value = null;
        clearQueueSwapTimeout();
    }

    watch([isQueueMode, queueUpcoming], ([show, tracks]) => {
        if (!show || tracks.length === 0) {
            commitQueue(tracks);
            return;
        }

        loadCoversForTracks(tracks);

        // Если обложки уже известны (загружены или подтверждённо отсутствуют) —
        // меняем очередь мгновенно. Иначе откладываем замену до прогрузки.
        if (tracks.every(isCoverResolved)) {
            commitQueue(tracks);
            return;
        }

        pendingQueue.value = tracks;
        clearQueueSwapTimeout();
        queueSwapTimeout = setTimeout(() => {
            if (pendingQueue.value) commitQueue(pendingQueue.value);
        }, 700);
    }, { immediate: true });

    // Как только обложки отложенной очереди прогрузились — заменяем список.
    watchEffect(() => {
        const pending = pendingQueue.value;
        if (pending && pending.every(isCoverResolved)) {
            commitQueue(pending);
        }
    });

    // Источник воспроизведения хранит порядок в sourcePaths; синхронизируем его
    // с обновлённым списком объектов источника.
    function applySourceTracks(tracks: TrackMetadata[]) {
        sourceTrackList.value = tracks;
        sourcePaths.value = tracks.map(t => t.file_path);
    }

    async function moveTrackToTop(track: TrackMetadata) {
        if (isAlbumSource.value) {
            const albumName = (activePlaylistSource.value ?? "").slice("album:".length);
            const { loadOrder, applyOrder, setOrder } = useAlbumTrackOrder();
            await loadOrder(albumName);
            // Треки альбома уже загружены как активный источник.
            const allAlbumTracks = sourceTrackList.value;
            const paths = applyOrder(albumName, allAlbumTracks).map(t => t.file_path);
            const idx = paths.indexOf(track.file_path);
            if (idx <= 0) return;
            paths.splice(idx, 1);
            paths.unshift(track.file_path);
            await setOrder(albumName, paths);
            applySourceTracks(
                paths.map(p => allAlbumTracks.find(t => t.file_path === p)!).filter(Boolean),
            );
            syncPlaybackOrder();
            return;
        }
        if (isPlaylistSource.value) {
            const playlistId = parseInt((activePlaylistSource.value ?? "").slice("playlist:".length), 10);
            const idx = sourceTrackList.value.findIndex(t => t.file_path === track.file_path);
            if (idx <= 0) return;
            const next = sourceTrackList.value.slice();
            const [item] = next.splice(idx, 1);
            next.unshift(item);
            applySourceTracks(next);
            await invoke("playlist_move_track_to_top", { playlistId, filePath: track.file_path });
            window.dispatchEvent(new CustomEvent("lfmusic:playlist-track-moved-to-top", { detail: { playlistId } }));
            syncPlaybackOrder();
            return;
        }
        // Каталог: путь переходит в начало канонического порядка (общий помощник).
        await moveCatalogTrackToTop(track.file_path);
    }

    // ── Громкость (общая для мини-бара и оверлея) ───────────────────────────
    const prevVolume = ref(volume.value || 0.66);

    function toggleMute() {
        if (volume.value === 0) {
            setVolume(prevVolume.value || 0.66);
        } else {
            prevVolume.value = volume.value;
            setVolume(0);
        }
    }

    function onVolumeScroll(event: WheelEvent) {
        event.preventDefault();
        const delta = event.deltaY < 0 ? 0.01 : -0.01;
        const newVol = Math.min(1, Math.max(0, volume.value + delta));
        setVolume(newVol);
    }

    function onVolumeInput(event: Event) {
        const val = parseFloat((event.target as HTMLInputElement).value);
        setVolume(val);
    }

    return {
        QUEUE_DISPLAY_COUNT,
        // подписи источника
        queueSourcePrefix,
        queueSourceName,
        discordSourceLabel,
        // режим оверлея
        fsMode,
        showOverlay,
        isQueueMode,
        openFullscreen,
        closeFullscreen,
        openQueueFullscreen,
        toggleFsMode,
        // кроссфейд обложек
        fsCoverA,
        fsCoverB,
        fsActiveBuffer,
        toBlurCover,
        // очередь
        isMixSource,
        isCatalogSource,
        isAlbumSource,
        isPlaylistSource,
        canMoveToTop,
        queueUpcoming,
        nextTrack,
        nextTrackTitle,
        displayQueue,
        moveTrackToTop,
        // громкость
        prevVolume,
        toggleMute,
        onVolumeScroll,
        onVolumeInput,
    };
}
