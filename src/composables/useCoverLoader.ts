import { watch, type Ref, type ComputedRef } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { TrackMetadata } from "./usePlayer";

const MAX_CONCURRENT_COVERS = 3;
const loadingCovers = new Set<string>();
let activeCoverRequests = 0;
let coverQueue: string[] = [];

interface CoverLoaderOptions {
    trackList: Ref<TrackMetadata[]>;
    getTrackByPath: (filePath: string) => TrackMetadata | undefined;
    getTrackIndex: (filePath: string) => number;
}

let opts: CoverLoaderOptions | null = null;

function processCoverQueue() {
    if (!opts) return;
    while (activeCoverRequests < MAX_CONCURRENT_COVERS && coverQueue.length > 0) {
        const filePath = coverQueue.shift()!;
        const track = opts.getTrackByPath(filePath);
        if (!track || track.cover_art) {
            loadingCovers.delete(filePath);
            continue;
        }
        activeCoverRequests++;
        invoke<string | null>("get_cover_art", { filePath })
            .then(art => {
                if (art && opts) {
                    const idx = opts.getTrackIndex(filePath);
                    if (idx !== -1) opts.trackList.value[idx].cover_art = art;
                }
            })
            .catch(() => {})
            .finally(() => {
                loadingCovers.delete(filePath);
                activeCoverRequests--;
                processCoverQueue();
            });
    }
}

export function useCoverLoader(options: CoverLoaderOptions) {
    opts = options;

    function setupPreloadWatch(
        coverPreloadTracks: ComputedRef<TrackMetadata[]>,
        visibleTracks: ComputedRef<TrackMetadata[]>,
    ) {
        let coverDebounceTimer: ReturnType<typeof setTimeout> | null = null;

        watch(coverPreloadTracks, (tracks) => {
            if (coverDebounceTimer) clearTimeout(coverDebounceTimer);
            coverDebounceTimer = setTimeout(() => {
                const visible = visibleTracks.value.map(t => t.file_path);
                const all = tracks.map(t => t.file_path);
                const preloadOnly = all.filter(p => !visible.includes(p));
                const ordered = [...visible, ...preloadOnly];

                for (const filePath of coverQueue) {
                    loadingCovers.delete(filePath);
                }
                coverQueue = [];
                for (const filePath of ordered) {
                    const track = options.getTrackByPath(filePath);
                    if (!track || track.cover_art || loadingCovers.has(filePath)) continue;
                    loadingCovers.add(filePath);
                    coverQueue.push(filePath);
                }
                processCoverQueue();
            }, 50);
        }, { immediate: true });
    }

    function loadCoversForTracks(tracks: TrackMetadata[]) {
        for (const track of tracks) {
            if (track.cover_art || loadingCovers.has(track.file_path)) continue;
            loadingCovers.add(track.file_path);
            coverQueue.push(track.file_path);
        }
        processCoverQueue();
    }

    function clearQueue() {
        for (const filePath of coverQueue) {
            loadingCovers.delete(filePath);
        }
        coverQueue = [];
    }

    return { setupPreloadWatch, loadCoversForTracks, clearQueue };
}
