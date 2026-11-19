import { reactive, ref, watch, effectScope } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useSettings } from "./useSettings";
import { isAndroidRuntime } from "./usePlatform";
import type { TrackMetadata } from "./usePlayer";

export interface AlbumLayoutEntry {
  position: number;
  disc: number;
  title: string;
  artist: string;
  duration_secs: number | null;
  /** Local file path, or null for a "phantom" (not downloaded) track. */
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
  /** lfmusic-external:// custom protocol URL (webview fetches the image). */
  url: string | null;
}

interface CachedArtistImage {
  artist_key: string;
  url: string;
}

const avatars = reactive<Record<string, string | null>>({});

const albumLayouts = reactive<Record<string, AlbumLayout>>({});
const inFlightAlbums = new Set<string>();

let cachedAvatarsLoaded = false;
const cachedAvatarsReady = ref(false);

/**
 * Recency-prioritized request queue: the current screen loads first; capped, old requests are
 * dropped.
 */
function createScheduler(concurrency: number, maxPending: number) {
  let active = 0;
  const order: string[] = [];
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

const avatarScheduler = createScheduler(4, 60);
const coverScheduler = createScheduler(4, 80);

const trackCovers = reactive<Record<string, string | null>>({});
const coverWaiters = new Map<string, Set<TrackMetadata>>();

const externallyCovered = new Map<string, TrackMetadata>();

/** Apply an external cover to a track and remember it for rollback/restore. */
function applyExternalCover(track: TrackMetadata, url: string): void {
  track.cover_art = url;
  track.cover_external = true;
  externallyCovered.set(track.file_path, track);
}

effectScope(true).run(() => {
  const { settings } = useSettings();
  watch(
    () => settings.value.external_track_covers,
    (enabled) => {
      if (enabled) {
        for (const track of externallyCovered.values()) {
          if (track.cover_art) continue;
          const key = coverCacheKey(track.artist || "", track.album || "", track.title || "");
          const url = trackCovers[key];
          if (url) applyExternalCover(track, url);
        }
      } else {
        for (const track of externallyCovered.values()) {
          if (track.cover_external) track.cover_art = null;
        }
      }
    },
  );
});

/** Clear the external-cover flag once a track has a local cover. */
export function clearExternalCoverFlag(track: TrackMetadata): void {
  if (track.cover_external) {
    track.cover_external = false;
    externallyCovered.delete(track.file_path);
  }
}

/** Cover cache key - must match `cover_cache_key` in external.rs. */
function coverCacheKey(artist: string, album: string, title: string): string {
  const alb = album.trim();
  return alb
    ? normalizeKey(`${artist} ${alb}`)
    : normalizeKey(`${artist} ${title} t`);
}

/** Must match `normalize` in src-tauri/src/external.rs. */
function normalizeKey(s: string): string {
  let out = "";
  let prevSpace = true;
  // Per-char lowercase and Rust's alphanumeric set, like external::normalize.
  for (const orig of s) {
    for (const ch of orig.toLowerCase()) {
      if (/[\p{Alphabetic}\p{N}]/u.test(ch)) {
        out += ch;
        prevSpace = false;
      } else if (!prevSpace) {
        out += " ";
        prevSpace = true;
      }
    }
  }
  return out.replace(/\s+$/u, "");
}

export function useExternalSources() {
  const { settings } = useSettings();

  /** Load already-cached avatars once, for a fast start. */
  async function loadCachedAvatars(): Promise<void> {
    if (cachedAvatarsLoaded) return;
    cachedAvatarsLoaded = true;
    if (isAndroidRuntime()) {
      cachedAvatarsReady.value = true;
      return;
    }
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

  /** Artist avatar from external sources (or null). */
  function avatarFor(artistName: string): string | null {
    if (!artistName) return null;
    if (isAndroidRuntime()) return null;
    if (!settings.value.external_artist_avatars) return null;
    const key = normalizeKey(artistName);
    if (key in avatars) return avatars[key];

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

  /** Force a re-request of the avatar, bypassing the cache. */
  async function refreshAvatar(artistName: string): Promise<string | null> {
    if (isAndroidRuntime()) return null;
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

  /** Album layout (canonical order + phantom tracks). */
  async function getAlbumLayout(albumName: string, refresh = false): Promise<AlbumLayout | null> {
    if (!albumName) return null;
    if (isAndroidRuntime()) return null;
    if (!refresh && albumName in albumLayouts) return albumLayouts[albumName];
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

  function fetchTrackCover(track: TrackMetadata): void {
    if (isAndroidRuntime()) return;
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

    let waiters = coverWaiters.get(key);
    if (!waiters) {
      waiters = new Set();
      coverWaiters.set(key, waiters);
    }
    waiters.add(track);

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
