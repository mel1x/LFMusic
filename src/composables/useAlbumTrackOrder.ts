import { invoke } from "@tauri-apps/api/core";
import { ref } from "vue";
import type { TrackMetadata } from "./usePlayer";

// Порядок треков внутри альбома. Хранится в SQLite (album_track_order),
// в сессии кэшируется в памяти. null = «не загружено», undefined-ключ = загружено,
// но кастомного порядка нет (сортируем по file_path по умолчанию).
const orderCache = ref<Record<string, string[] | null>>({});

function byFilePath(a: TrackMetadata, b: TrackMetadata): number {
  return a.file_path.localeCompare(b.file_path, undefined, {
    numeric: true,
    sensitivity: "base",
  });
}

export function useAlbumTrackOrder() {
  /** Загрузить порядок из БД (один раз за сессию для каждого альбома). */
  async function loadOrder(albumName: string): Promise<void> {
    if (albumName in orderCache.value) return;
    try {
      const paths = await invoke<string[] | null>("album_get_track_order", {
        albumName,
      });
      orderCache.value = { ...orderCache.value, [albumName]: paths ?? null };
    } catch (e) {
      console.error("Ошибка загрузки порядка треков альбома:", e);
      orderCache.value = { ...orderCache.value, [albumName]: null };
    }
  }

  /**
   * Применяет сохранённый порядок к трекам альбома. Треки из сохранённого
   * порядка идут первыми, новые (ещё не переставлявшиеся) — в конце по
   * file_path. Без сохранённого порядка — обычная сортировка по file_path.
   */
  function applyOrder(albumName: string, tracks: TrackMetadata[]): TrackMetadata[] {
    const order = orderCache.value[albumName];
    if (!order || order.length === 0) {
      return [...tracks].sort(byFilePath);
    }
    const rank = new Map(order.map((path, i) => [path, i]));
    const known: TrackMetadata[] = [];
    const unknown: TrackMetadata[] = [];
    for (const t of tracks) {
      (rank.has(t.file_path) ? known : unknown).push(t);
    }
    known.sort((a, b) => rank.get(a.file_path)! - rank.get(b.file_path)!);
    unknown.sort(byFilePath);
    return [...known, ...unknown];
  }

  /** Сохранить полный порядок file_path для альбома (и обновить кэш). */
  async function setOrder(albumName: string, paths: string[]): Promise<void> {
    orderCache.value = { ...orderCache.value, [albumName]: paths.slice() };
    try {
      await invoke("album_set_track_order", {
        albumName,
        trackPaths: paths,
      });
    } catch (e) {
      console.error("Ошибка сохранения порядка треков альбома:", e);
    }
  }

  /**
   * Переместить трек fromPath на позицию toPath в рамках текущего порядка
   * (currentPaths — порядок треков как он отображается сейчас).
   */
  async function moveTrack(
    albumName: string,
    currentPaths: string[],
    fromPath: string,
    toPath: string,
  ): Promise<void> {
    const paths = currentPaths.slice();
    const from = paths.indexOf(fromPath);
    const to = paths.indexOf(toPath);
    if (from === -1 || to === -1 || from === to) return;
    const [moved] = paths.splice(from, 1);
    paths.splice(to, 0, moved);
    await setOrder(albumName, paths);
  }

  return { loadOrder, applyOrder, setOrder, moveTrack };
}
