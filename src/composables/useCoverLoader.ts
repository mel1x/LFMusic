import { reactive, watch, type ComputedRef } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { TrackMetadata } from "./usePlayer";
import { useExternalSources, clearExternalCoverFlag } from "./useExternalSources";

const MAX_CONCURRENT_COVERS = 3;
// Cap on the waiting pool: current screen always wins priority; oldest
// off-screen requests get dropped and re-requested if scrolled back to.
const MAX_PENDING_COVERS = 150;
// Track metadata cache miss: not materialized yet, retry shortly after.
const COVER_MISS_RETRIES = 6;
const COVER_MISS_RETRY_MS = 150;

/** Resolves a live track object by path (usually `getTrackByPath` from usePlayer). */
type TrackResolver = (filePath: string) => TrackMetadata | undefined;

// Priority queue by recency: the most recently requested path loads first;
// re-requesting bumps priority, so loading follows the current screen
// instead of strictly draining from the start of the list.
const order: string[] = []; // pending paths, end of array = highest priority
const items = new Map<string, TrackResolver>();
const running = new Set<string>();
const missRetries = new Map<string, number>();
let active = 0;

// Paths whose cover is already resolved (loaded or confirmed absent).
// Reactive — screens gate first paint until the first N resolve.
const resolvedCovers = reactive(new Set<string>());

function markCoverResolved(filePath: string) {
    resolvedCovers.add(filePath);
}

/** True once a track's cover is known — either loaded, or confirmed absent. */
export function isCoverResolved(track: Pick<TrackMetadata, "file_path" | "cover_art">): boolean {
    return !!track.cover_art || resolvedCovers.has(track.file_path);
}

interface CoverLoaderOptions {
    getTrackByPath: TrackResolver;
}

function pump(): void {
    while (active < MAX_CONCURRENT_COVERS && order.length > 0) {
        const filePath = order.pop()!;
        const resolve = items.get(filePath)!;
        items.delete(filePath);

        const track = resolve(filePath);

        if (track && track.cover_art) {
            missRetries.delete(filePath);
            markCoverResolved(filePath);
            continue;
        }

        if (!track) {
            // Metadata hasn't landed in the cache yet (track materializes via
            // background loading) — retry shortly, bounded by retry count.
            const tries = missRetries.get(filePath) ?? 0;
            if (tries < COVER_MISS_RETRIES) {
                missRetries.set(filePath, tries + 1);
                setTimeout(() => schedule(filePath, resolve), COVER_MISS_RETRY_MS);
            } else {
                missRetries.delete(filePath);
                markCoverResolved(filePath);
            }
            continue;
        }

        // Track exists, no cover yet — read the embedded one.
        missRetries.delete(filePath);
        running.add(filePath);
        active++;
        invoke<string | null>("get_cover_art", { filePath })
            .then(art => {
                const cached = resolve(filePath);
                if (art) {
                    // Write into the live object from the same screen's cache (reactive).
                    // This is a local (embedded/override) cover — clear the external
                    // flag so disabling external covers won't revert it.
                    if (cached) {
                        cached.cover_art = art;
                        clearExternalCoverFlag(cached);
                    }
                } else if (cached) {
                    // No embedded cover — try an external source (if enabled).
                    useExternalSources().fetchTrackCover(cached);
                }
            })
            .catch(() => {})
            .finally(() => {
                active--;
                running.delete(filePath);
                markCoverResolved(filePath);
                pump();
            });
    }
}

/** Queues a path (or bumps its priority if already waiting). */
function schedule(filePath: string, resolve: TrackResolver): void {
    if (running.has(filePath)) return;
    if (items.has(filePath)) {
        // Already queued — bump priority (current screen first) and refresh
        // the resolver to the current screen.
        const i = order.indexOf(filePath);
        if (i !== -1) order.splice(i, 1);
        order.push(filePath);
        items.set(filePath, resolve);
        return;
    }
    items.set(filePath, resolve);
    order.push(filePath);
    while (order.length > MAX_PENDING_COVERS) {
        const dropped = order.shift()!;
        items.delete(dropped);
    }
    pump();
}

export function useCoverLoader(options: CoverLoaderOptions) {
    // Captures this call's resolver locally — no shared mutable module state
    // that a subsequent screen could overwrite.
    const resolve = options.getTrackByPath;

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
                const preloadOnly = tracks.map(t => t.file_path).filter(p => !visibleSet.has(p));
                // Preload is lower priority; visible window is higher (scheduled last).
                for (const filePath of preloadOnly) {
                    const t = resolve(filePath);
                    if (!t || !t.cover_art) schedule(filePath, resolve);
                }
                for (const filePath of visible) {
                    const t = resolve(filePath);
                    if (!t || !t.cover_art) schedule(filePath, resolve);
                }
            }, 50);
        }, { immediate: true });
    }

    function loadCoversForTracks(tracks: TrackMetadata[]) {
        const seen = new Set<string>();
        for (const track of tracks) {
            if (seen.has(track.file_path)) continue;
            seen.add(track.file_path);
            if (track.cover_art) continue;
            schedule(track.file_path, resolve);
        }
    }

    function clearQueue() {
        order.length = 0;
        items.clear();
    }

    return { setupPreloadWatch, loadCoversForTracks, clearQueue };
}
