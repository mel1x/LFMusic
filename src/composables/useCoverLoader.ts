import { reactive, watch, type Ref, type ComputedRef } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { TrackMetadata } from "./usePlayer";

const MAX_CONCURRENT_COVERS = 3;
const loadingCovers = new Set<string>();
let activeCoverRequests = 0;
let coverQueue: string[] = [];

// Paths whose cover request has finished (whether it yielded art or not).
// Reactive so views can gate first paint until the first N covers resolve.
const resolvedCovers = reactive(new Set<string>());

function markCoverResolved(filePath: string) {
    resolvedCovers.add(filePath);
}

/** True once a track's cover is known — either loaded, or confirmed absent. */
export function isCoverResolved(track: Pick<TrackMetadata, "file_path" | "cover_art">): boolean {
    return !!track.cover_art || resolvedCovers.has(track.file_path);
}

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
            markCoverResolved(filePath);
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
                markCoverResolved(filePath);
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
                const visibleSet = new Set(visible);
                const all = tracks.map(t => t.file_path);
                const preloadOnly = all.filter(p => !visibleSet.has(p));
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
        const seen = new Set<string>();
        for (const track of tracks) {
            if (seen.has(track.file_path)) continue;
            seen.add(track.file_path);
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
