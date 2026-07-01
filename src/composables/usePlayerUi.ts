import { ref, computed, watch, watchEffect, effectScope } from "vue";
import { invoke } from "@tauri-apps/api/core";

import { usePlayer, type TrackMetadata } from "./usePlayer";
import { usePlaylists } from "./usePlaylists";
import { useAlbumTrackOrder } from "./useAlbumTrackOrder";
import { useI18n } from "./useI18n";
import { isCoverResolved } from "./useCoverLoader";

// Shared player UI state between the mini-bar (PlaybackControls) and the
// fullscreen overlay (PlayerFullscreenOverlay): overlay mode, cover crossfade,
// queue, source labels, move-to-top, volume.
//
// Used to live as local refs inside monolithic Player.vue; after splitting into
// sub-components the state must be shared, hence this composable singleton
// (same effectScope pattern as usePlayer/usePins).

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

    // Queue source labels
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

    // Discord source label: like queueSourceName but in nominative case
    // (queueSourceName is grammatically matched to the "playing from..." prefix).
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

    // Fullscreen mode
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

    // Fullscreen cover crossfade
    const fsCoverA = ref<string | null>(null);
    const fsCoverB = ref<string | null>(null);
    const fsActiveBuffer = ref<"a" | "b">("a");

    // Blurred fullscreen background uses a low-res thumbnail (size=blur ~96px)
    // instead of 512px: it's blurred by 80px anyway, so decode/memory cost drops a lot.
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

    // Queue source / move-to-top
    const isMixSource = computed(() => activePlaylistSource.value === "mix");
    const isCatalogSource = computed(() => (activePlaylistSource.value ?? "").startsWith("catalog"));
    const isAlbumSource = computed(() => (activePlaylistSource.value ?? "").startsWith("album:"));
    const isPlaylistSource = computed(() => (activePlaylistSource.value ?? "").startsWith("playlist:"));
    const canMoveToTop = computed(() => isCatalogSource.value || isAlbumSource.value || isPlaylistSource.value);

    const queueUpcomingPaths = computed<string[]>(() => {
        const order = playbackOrder.value;
        const idx = currentTrackIndex.value;
        if (idx < 0 || order.length === 0) return [];

        if (isMixSource.value) {
            // Mix ("wave"): only show the next track.
            return order.slice(idx + 1, idx + 2);
        }

        if (shuffled.value) {
            // Shuffle: playbackOrder's tail already has pre-generated reshuffle
            // passes (ensureLookahead keeps at least QUEUE_DISPLAY_COUNT tracks).
            return order.slice(idx + 1, idx + 1 + QUEUE_DISPLAY_COUNT);
        }

        // No shuffle: loop the queue
        const result: string[] = [];
        for (let i = 1; i <= QUEUE_DISPLAY_COUNT; i++) {
            result.push(order[(idx + i) % order.length]);
        }
        return result;
    });

    // Resolve queue paths to objects via cache (misses are fetched in the
    // background; computed recalculates once metadata arrives).
    const queueUpcoming = computed<TrackMetadata[]>(() =>
        queueUpcomingPaths.value
            .map(p => getTrackByPath(p))
            .filter((t): t is TrackMetadata => !!t),
    );

    // Prefetch metadata for all upcoming paths so the queue isn't incomplete.
    watch(queueUpcomingPaths, (paths) => {
        if (paths.length) void fetchTracksByPaths(paths);
    }, { immediate: true });

    const nextTrack = computed(() => queueUpcoming.value[0] ?? null);

    const nextTrackTitle = computed(() => {
        const track = nextTrack.value;
        if (!track) return t("player.noTrack");
        return track.title || track.file_path.split(/[/\\]/).pop()?.replace(/\.[^/.]+$/, "") || t("player.untitled");
    });

    // Deferred queue swap: preload covers before showing the new list, so
    // enabling shuffle doesn't flash blank covers. `displayQueue` is what's
    // actually rendered; it lags `queueUpcoming` until covers are ready
    // (with a safety timeout).
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

        // If covers are already resolved (loaded or confirmed absent), swap the
        // queue instantly. Otherwise defer the swap until they're ready.
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

    // Once the pending queue's covers are ready, swap the list in.
    watchEffect(() => {
        const pending = pendingQueue.value;
        if (pending && pending.every(isCoverResolved)) {
            commitQueue(pending);
        }
    });

    // Playback source keeps its order in sourcePaths; sync it whenever the
    // source track list is updated.
    function applySourceTracks(tracks: TrackMetadata[]) {
        sourceTrackList.value = tracks;
        sourcePaths.value = tracks.map(t => t.file_path);
    }

    async function moveTrackToTop(track: TrackMetadata) {
        if (isAlbumSource.value) {
            const albumName = (activePlaylistSource.value ?? "").slice("album:".length);
            const { loadOrder, applyOrder, setOrder } = useAlbumTrackOrder();
            await loadOrder(albumName);
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
        // Catalog: move path to the front of the canonical order (shared helper).
        await moveCatalogTrackToTop(track.file_path);
    }

    // Volume (shared between mini-bar and overlay)
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
        // source labels
        queueSourcePrefix,
        queueSourceName,
        discordSourceLabel,
        // overlay mode
        fsMode,
        showOverlay,
        isQueueMode,
        openFullscreen,
        closeFullscreen,
        openQueueFullscreen,
        toggleFsMode,
        // cover crossfade
        fsCoverA,
        fsCoverB,
        fsActiveBuffer,
        toBlurCover,
        // queue
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
        // volume
        prevVolume,
        toggleMute,
        onVolumeScroll,
        onVolumeInput,
    };
}
