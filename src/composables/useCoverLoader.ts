import { reactive, watch, type ComputedRef } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { TrackMetadata } from "./usePlayer";

const MAX_CONCURRENT_COVERS = 3;
const loadingCovers = new Set<string>();
let activeCoverRequests = 0;

/** Резолвер живого объекта трека по пути (обычно `getTrackByPath` из usePlayer). */
type TrackResolver = (filePath: string) => TrackMetadata | undefined;

/**
 * Элемент очереди несёт собственный резолвер, захваченный в момент постановки.
 * Раньше резолвер хранился в одной глобальной переменной `opts`, которую
 * перезаписывал каждый новый экран; асинхронные ответы `get_cover_art`,
 * запущенные старым экраном, по завершении обращались к чужому резолверу.
 * Привязка резолвера к элементу очереди устраняет эту утечку состояния:
 * каждый запрос разрешается ровно тем экраном, который его поставил.
 */
interface CoverQueueItem {
    filePath: string;
    resolve: TrackResolver;
}

let coverQueue: CoverQueueItem[] = [];

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
    getTrackByPath: TrackResolver;
}

function processCoverQueue() {
    while (activeCoverRequests < MAX_CONCURRENT_COVERS && coverQueue.length > 0) {
        const { filePath, resolve } = coverQueue.shift()!;
        const track = resolve(filePath);
        if (!track || track.cover_art) {
            loadingCovers.delete(filePath);
            markCoverResolved(filePath);
            continue;
        }
        activeCoverRequests++;
        invoke<string | null>("get_cover_art", { filePath })
            .then(art => {
                if (art) {
                    // Записываем обложку в живой объект из кэша того же экрана (реактивно).
                    const cached = resolve(filePath);
                    if (cached) cached.cover_art = art;
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
    // Захватываем резолвер этого вызова локально — никаких общих мутируемых
    // переменных модуля, которые мог бы затереть следующий экран.
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
                const all = tracks.map(t => t.file_path);
                const preloadOnly = all.filter(p => !visibleSet.has(p));
                const ordered = [...visible, ...preloadOnly];

                for (const item of coverQueue) {
                    loadingCovers.delete(item.filePath);
                }
                coverQueue = [];
                for (const filePath of ordered) {
                    const track = resolve(filePath);
                    if (!track || track.cover_art || loadingCovers.has(filePath)) continue;
                    loadingCovers.add(filePath);
                    coverQueue.push({ filePath, resolve });
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
            coverQueue.push({ filePath: track.file_path, resolve });
        }
        processCoverQueue();
    }

    function clearQueue() {
        for (const item of coverQueue) {
            loadingCovers.delete(item.filePath);
        }
        coverQueue = [];
    }

    return { setupPreloadWatch, loadCoversForTracks, clearQueue };
}
