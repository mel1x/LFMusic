import { reactive, watch, type ComputedRef } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { TrackMetadata } from "./usePlayer";
import { useExternalSources } from "./useExternalSources";

const MAX_CONCURRENT_COVERS = 3;
// Ограничение пула ожидающих: текущий экран всегда в приоритете, самые старые
// (вне экрана) запросы отбрасываются и запрашиваются заново при возврате.
const MAX_PENDING_COVERS = 150;
// Промах кэша метаданных: трек ещё не материализован — повторяем чуть позже.
const COVER_MISS_RETRIES = 6;
const COVER_MISS_RETRY_MS = 150;

/** Резолвер живого объекта трека по пути (обычно `getTrackByPath` из usePlayer). */
type TrackResolver = (filePath: string) => TrackMetadata | undefined;

// Очередь с приоритетом по «свежести»: последний запрошенный путь грузится
// первым, повторный запрос поднимает приоритет — загрузка следует за текущим
// экраном, а не идёт строго с начала списка.
const order: string[] = []; // ожидающие пути, конец массива = высший приоритет
const items = new Map<string, TrackResolver>();
const running = new Set<string>();
const missRetries = new Map<string, number>();
let active = 0;

// Пути, для которых обложка уже определена (загружена либо подтверждено
// отсутствие). Реактивно — экраны гейтят первую отрисовку до резолва первых N.
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
            // Метаданные ещё не подъехали в кэш — повторим чуть позже (трек
            // материализуется фоновой дозагрузкой). Ограничиваем число попыток.
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

        // Трек есть, обложки нет — читаем встроенную.
        missRetries.delete(filePath);
        running.add(filePath);
        active++;
        invoke<string | null>("get_cover_art", { filePath })
            .then(art => {
                const cached = resolve(filePath);
                if (art) {
                    // Записываем обложку в живой объект из кэша того же экрана (реактивно).
                    if (cached) cached.cover_art = art;
                } else if (cached) {
                    // Встроенной обложки нет — пробуем внешний источник (если включено).
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

/** Поставить путь в очередь (или поднять приоритет, если уже ждёт). */
function schedule(filePath: string, resolve: TrackResolver): void {
    if (running.has(filePath)) return;
    if (items.has(filePath)) {
        // Уже в очереди — поднимаем приоритет (текущий экран вперёд) и обновляем
        // резолвер на актуальный экран.
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
                const preloadOnly = tracks.map(t => t.file_path).filter(p => !visibleSet.has(p));
                // Предзагрузка — ниже приоритетом, видимое окно — выше (ставим последним).
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
