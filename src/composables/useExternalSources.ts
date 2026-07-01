import { reactive, ref, watch, effectScope } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useSettings } from "./useSettings";
import type { TrackMetadata } from "./usePlayer";

// Types mirror the serde structs in src-tauri/src/external.rs

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

// Singleton state

// Artist avatars: normalized key -> data URL (or null = "none").
const avatars = reactive<Record<string, string | null>>({});

// Album layouts: album name -> layout, cached for the session to avoid
// re-hitting IPC on every re-render.
const albumLayouts = reactive<Record<string, AlbumLayout>>({});
const inFlightAlbums = new Set<string>();

let cachedAvatarsLoaded = false;
const cachedAvatarsReady = ref(false);

/**
 * External request scheduler prioritized by recency: the most recently
 * requested key runs first, and re-requesting bumps priority. This makes
 * loading follow the current screen instead of the start of the list.
 * The queue is capped — oldest (off-screen) requests get dropped and
 * re-requested if they scroll back into view.
 */
function createScheduler(concurrency: number, maxPending: number) {
  let active = 0;
  const order: string[] = []; // pending keys, end of array = highest priority
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
      // Already queued — bump priority (current screen first).
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

// External APIs are rate-limited, and a list can request dozens of images at once.
const avatarScheduler = createScheduler(4, 60);
const coverScheduler = createScheduler(4, 80);

// Covers for tracks without embedded art: cache key (artist+album) -> data URL
// (or null = "not found"). Tracks from the same album share one cover.
const trackCovers = reactive<Record<string, string | null>>({});
const coverWaiters = new Map<string, Set<TrackMetadata>>();

// Live track objects that got an external cover applied (file_path -> track).
// Needed to roll back when the setting is disabled and restore when re-enabled.
// The data URLs themselves stay in trackCovers (and the backend's permanent
// cache), so re-enabling doesn't hit the network again.
const externallyCovered = new Map<string, TrackMetadata>();

/** Apply an external cover to a track and remember it for rollback/restore. */
function applyExternalCover(track: TrackMetadata, url: string): void {
  track.cover_art = url;
  track.cover_external = true;
  externallyCovered.set(track.file_path, track);
}

// Roll back/restore external covers when the setting is toggled. The watch
// runs in a detached scope that lives for the app's lifetime (singleton composable).
effectScope(true).run(() => {
  const { settings } = useSettings();
  watch(
    () => settings.value.external_track_covers,
    (enabled) => {
      if (enabled) {
        // Restore from the session cache (no network hit).
        for (const track of externallyCovered.values()) {
          if (track.cover_art) continue;
          const key = coverCacheKey(track.artist || "", track.album || "", track.title || "");
          const url = trackCovers[key];
          if (url) applyExternalCover(track, url);
        }
      } else {
        // Hide external covers -> local placeholder shows. Keep the cache and
        // cover_external flag so we can restore on re-enable.
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

/** Cover cache key — must match `cover_cache_key` in external.rs. */
function coverCacheKey(artist: string, album: string, title: string): string {
  const alb = album.trim();
  return alb
    ? normalizeKey(`${artist} ${alb}`)
    : normalizeKey(`${artist} ${title} t`);
}

/**
 * Artist name normalization — must match `normalize` in
 * src-tauri/src/external.rs: lowercase, letters/digits only, everything
 * else collapses to a single space, trimmed.
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

  /** Load already-cached avatars once, for a fast start. */
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
   * Artist avatar from external sources (or null). Reactive: returns what's
   * already known while loading, then updates when ready. Network request
   * only fires if the setting is enabled.
   */
  function avatarFor(artistName: string): string | null {
    if (!artistName) return null;
    // The setting hides external avatars even if already cached.
    if (!settings.value.external_artist_avatars) return null;
    const key = normalizeKey(artistName);
    if (key in avatars) return avatars[key];

    // Request (or bump priority if already queued) — current screen first.
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
   * Album layout (canonical order + phantom tracks). Cached in session memory.
   * Only requested if the setting is enabled.
   */
  async function getAlbumLayout(albumName: string, refresh = false): Promise<AlbumLayout | null> {
    if (!albumName) return null;
    if (!refresh && albumName in albumLayouts) return albumLayouts[albumName];
    // Layout is needed if external order OR supplement-missing is enabled.
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
   * For a track without an embedded cover, lazily fetches an album cover from
   * external sources and reactively sets it on `track.cover_art`. Only
   * requests if the setting is enabled. Safe to call repeatedly.
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

    // Register the track as waiting for this cover (multiple tracks per album).
    let waiters = coverWaiters.get(key);
    if (!waiters) {
      waiters = new Set();
      coverWaiters.set(key, waiters);
    }
    waiters.add(track);

    // Request (or bump priority) — current screen first.
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
