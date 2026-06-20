import { reactive, ref, watch, effectScope } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useSettings } from "./useSettings";
import type { TrackMetadata } from "./usePlayer";

// ──────────────────────────────────────────────────────────────────────────────
// Типы (зеркало serde-структур из src-tauri/src/external.rs)
// ──────────────────────────────────────────────────────────────────────────────

export interface AlbumLayoutEntry {
  position: number;
  disc: number;
  title: string;
  artist: string;
  duration_secs: number | null;
  /** Путь к локальному файлу или null — «фантомный» (не скачанный) трек. */
  file_path: string | null;
}

export interface AlbumLayout {
  status: "ok" | "none";
  source: string | null;
  entries: AlbumLayoutEntry[];
}

interface ArtistImageResult {
  status: "ok" | "none";
  source: string | null;
  /** URL кастомного протокола lfmusic-external:// (картинку тянет вебвью). */
  url: string | null;
}

interface CachedArtistImage {
  artist_key: string;
  url: string;
}

// ──────────────────────────────────────────────────────────────────────────────
// Singleton state
// ──────────────────────────────────────────────────────────────────────────────

// Аватарки исполнителей: нормализованный ключ -> data-URL (или null = «нет»).
const avatars = reactive<Record<string, string | null>>({});

// Макеты альбомов: имя альбома -> layout (кэш сессии, чтобы не дёргать IPC при
// каждом ререндере).
const albumLayouts = reactive<Record<string, AlbumLayout>>({});
const inFlightAlbums = new Set<string>();

let cachedAvatarsLoaded = false;
const cachedAvatarsReady = ref(false);

/**
 * Планировщик внешних запросов с приоритетом по «свежести»: последний
 * запрошенный ключ выполняется первым, повторный запрос поднимает приоритет.
 * Так загрузка следует за текущим экраном, а не идёт строго с начала списка.
 * Очередь ограничена — самые старые (вне экрана) запросы отбрасываются и при
 * необходимости запрашиваются заново при возврате в зону видимости.
 */
function createScheduler(concurrency: number, maxPending: number) {
  let active = 0;
  const order: string[] = []; // ожидающие ключи, конец массива = высший приоритет
  const tasks = new Map<string, () => Promise<void>>();
  const running = new Set<string>();

  function pump(): void {
    while (active < concurrency && order.length > 0) {
      const key = order.pop()!;
      const run = tasks.get(key)!;
      tasks.delete(key);
      running.add(key);
      active++;
      void run().finally(() => {
        active--;
        running.delete(key);
        pump();
      });
    }
  }

  function schedule(key: string, run: () => Promise<void>): void {
    if (running.has(key)) return;
    if (tasks.has(key)) {
      // Уже в очереди — поднимаем приоритет (текущий экран вперёд).
      const i = order.indexOf(key);
      if (i !== -1) order.splice(i, 1);
      order.push(key);
      return;
    }
    tasks.set(key, run);
    order.push(key);
    while (order.length > maxPending) {
      const dropped = order.shift()!;
      tasks.delete(dropped);
    }
    pump();
  }

  return { schedule };
}

// Внешние API имеют rate-limit, а список может запросить десятки картинок разом.
const avatarScheduler = createScheduler(4, 60);
const coverScheduler = createScheduler(4, 80);

// Обложки треков без встроенной обложки: ключ кэша (артист+альбом) -> data-URL
// (или null = «не нашлось»). Несколько треков одного альбома делят одну обложку.
const trackCovers = reactive<Record<string, string | null>>({});
const coverWaiters = new Map<string, Set<TrackMetadata>>();

// Живые объекты треков, которым проставлена внешняя обложка (file_path -> track).
// Нужно для отката при выключении настройки и восстановления при включении.
// Сами data-URL остаются в trackCovers (и в бессрочном кэше бэкенда), поэтому
// повторное включение не ходит в сеть.
const externallyCovered = new Map<string, TrackMetadata>();

/** Применить внешнюю обложку к треку и запомнить его для отката/восстановления. */
function applyExternalCover(track: TrackMetadata, url: string): void {
  track.cover_art = url;
  track.cover_external = true;
  externallyCovered.set(track.file_path, track);
}

// Откат/восстановление внешних обложек при переключении настройки. Watch в
// отвязанном scope живёт всё время работы приложения (синглтон-композабл).
effectScope(true).run(() => {
  const { settings } = useSettings();
  watch(
    () => settings.value.external_track_covers,
    (enabled) => {
      if (enabled) {
        // Восстанавливаем из сессионного кэша (сеть не дёргаем).
        for (const track of externallyCovered.values()) {
          if (track.cover_art) continue;
          const key = coverCacheKey(track.artist || "", track.album || "", track.title || "");
          const url = trackCovers[key];
          if (url) applyExternalCover(track, url);
        }
      } else {
        // Прячем внешние обложки → показываются локальные (плейсхолдер). Кэш и
        // флаг cover_external сохраняем, чтобы восстановить при включении.
        for (const track of externallyCovered.values()) {
          if (track.cover_external) track.cover_art = null;
        }
      }
    },
  );
});

/** Сбросить пометку внешней обложки (когда у трека появилась локальная). */
export function clearExternalCoverFlag(track: TrackMetadata): void {
  if (track.cover_external) {
    track.cover_external = false;
    externallyCovered.delete(track.file_path);
  }
}

/** Ключ кэша обложки — должен совпадать с `cover_cache_key` в external.rs. */
function coverCacheKey(artist: string, album: string, title: string): string {
  const alb = album.trim();
  return alb
    ? normalizeKey(`${artist} ${alb}`)
    : normalizeKey(`${artist} ${title} t`);
}

/**
 * Нормализация имени исполнителя — должна совпадать с `normalize` в
 * src-tauri/src/external.rs: нижний регистр, только буквы/цифры, прочее →
 * одиночный пробел, обрезка краёв.
 */
function normalizeKey(s: string): string {
  let out = "";
  let prevSpace = true;
  for (const ch of s.toLowerCase()) {
    if (/[\p{L}\p{N}]/u.test(ch)) {
      out += ch;
      prevSpace = false;
    } else if (!prevSpace) {
      out += " ";
      prevSpace = true;
    }
  }
  return out.replace(/\s+$/u, "");
}

export function useExternalSources() {
  const { settings } = useSettings();

  /** Однократно подгружает уже закэшированные аватарки (быстрый старт). */
  async function loadCachedAvatars(): Promise<void> {
    if (cachedAvatarsLoaded) return;
    cachedAvatarsLoaded = true;
    try {
      const list = await invoke<CachedArtistImage[]>("external_load_cached_artist_images");
      for (const e of list) {
        if (avatars[e.artist_key] === undefined) avatars[e.artist_key] = e.url;
      }
    } catch (e) {
      console.error("Ошибка загрузки кэша аватарок:", e);
    } finally {
      cachedAvatarsReady.value = true;
    }
  }

  /**
   * Аватарка исполнителя из внешних источников (или null). Реактивно: пока идёт
   * загрузка возвращает то, что уже известно, и обновляется по готовности.
   * Сетевой запрос выполняется только если настройка включена.
   */
  function avatarFor(artistName: string): string | null {
    if (!artistName) return null;
    // Настройка скрывает внешние аватарки даже при наличии их в кэше.
    if (!settings.value.external_artist_avatars) return null;
    const key = normalizeKey(artistName);
    if (key in avatars) return avatars[key];

    // Запрос (или подъём приоритета, если уже в очереди) — текущий экран вперёд.
    avatarScheduler.schedule(key, () =>
      invoke<ArtistImageResult>("external_artist_image", { artistName, refresh: false })
        .then((res) => {
          avatars[key] = res.status === "ok" ? res.url : null;
        })
        .catch((e) => {
          console.error("Ошибка получения аватарки исполнителя:", e);
          avatars[key] = null;
        }),
    );
    return avatars[key] ?? null;
  }

  /** Принудительно перезапросить аватарку (минуя кэш). */
  async function refreshAvatar(artistName: string): Promise<string | null> {
    const key = normalizeKey(artistName);
    try {
      const res = await invoke<ArtistImageResult>("external_artist_image", {
        artistName,
        refresh: true,
      });
      avatars[key] = res.status === "ok" ? res.url : null;
      return avatars[key];
    } catch (e) {
      console.error("Ошибка обновления аватарки исполнителя:", e);
      return null;
    }
  }

  /**
   * Макет альбома (канонический порядок + фантомные треки). Кэшируется в памяти
   * сессии. Запрос идёт только если настройка включена.
   */
  async function getAlbumLayout(albumName: string, refresh = false): Promise<AlbumLayout | null> {
    if (!albumName) return null;
    if (!refresh && albumName in albumLayouts) return albumLayouts[albumName];
    // Макет нужен, если включён внешний порядок ИЛИ дополнение недостающими.
    if (!settings.value.external_album_order && !settings.value.external_supplement_missing && !refresh) return null;
    if (inFlightAlbums.has(albumName) && !refresh) return albumLayouts[albumName] ?? null;

    inFlightAlbums.add(albumName);
    try {
      const layout = await invoke<AlbumLayout>("external_album_layout", { albumName, refresh });
      albumLayouts[albumName] = layout;
      return layout;
    } catch (e) {
      console.error("Ошибка получения макета альбома:", e);
      return null;
    } finally {
      inFlightAlbums.delete(albumName);
    }
  }

  /**
   * Для трека без встроенной обложки лениво подгружает обложку альбома из
   * внешних источников и реактивно проставляет её в `track.cover_art`.
   * Запрос идёт только если настройка включена. Безопасно вызывать многократно.
   */
  function fetchTrackCover(track: TrackMetadata): void {
    if (!settings.value.external_track_covers) return;
    if (track.cover_art) return;
    const artist = track.artist || "";
    const album = track.album || "";
    const title = track.title || "";
    if (!artist && !album && !title) return;

    const key = coverCacheKey(artist, album, title);
    if (key in trackCovers) {
      const url = trackCovers[key];
      if (url && !track.cover_art) applyExternalCover(track, url);
      return;
    }

    // Регистрируем трек как ожидающего эту обложку (несколько треков альбома).
    let waiters = coverWaiters.get(key);
    if (!waiters) {
      waiters = new Set();
      coverWaiters.set(key, waiters);
    }
    waiters.add(track);

    // Запрос (или подъём приоритета) — текущий экран вперёд.
    coverScheduler.schedule(key, () =>
      invoke<ArtistImageResult>("external_track_cover", { artist, album, title, refresh: false })
        .then((res) => {
          const url = res.status === "ok" ? res.url : null;
          trackCovers[key] = url;
          const pending = coverWaiters.get(key);
          coverWaiters.delete(key);
          if (url && pending) {
            for (const t of pending) if (!t.cover_art) applyExternalCover(t, url);
          }
        })
        .catch((e) => {
          console.error("Ошибка получения обложки трека:", e);
          trackCovers[key] = null;
          coverWaiters.delete(key);
        }),
    );
  }

  return {
    avatars,
    cachedAvatarsReady,
    loadCachedAvatars,
    avatarFor,
    refreshAvatar,
    getAlbumLayout,
    fetchTrackCover,
  };
}
