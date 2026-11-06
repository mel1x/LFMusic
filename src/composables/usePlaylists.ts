import { ref, computed, watch, effectScope } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { TrackMetadata } from "./usePlayer";
import { useAlbumTrackOrder } from "./useAlbumTrackOrder";

export type PlaylistKind = "Album" | "Artist" | "User" | "Favorites";

export interface PlaylistInfo {
  id: number;
  name: string;
  kind: PlaylistKind;
  track_count: number;
  first_file_path: string | null;
}

const playlists = ref<PlaylistInfo[]>([]);
const loading = ref(false);
// Reactive membership of the "Мне нравится" playlist, keyed by file_path. Shared
// so every heart button (context menu + all track rows) reflects the same state
// and updates live on toggle. Vue 3 makes Set add/delete/has reactive.
const favoritePaths = ref<Set<string>>(new Set());
let favoritesLoadedOnce = false;
const topOrder = ref<number[]>([]);
const albumTopOrder = ref<string[]>([]);
const artistTopOrder = ref<string[]>([]);

const TOP_ORDER_STORAGE_KEY = "lfmusic.topOrder.v1";

try {
  const raw = localStorage.getItem(TOP_ORDER_STORAGE_KEY);
  if (raw) {
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed?.playlists)) {
      topOrder.value = parsed.playlists.filter((v: unknown) => typeof v === "number");
    }
    if (Array.isArray(parsed?.albums)) {
      albumTopOrder.value = parsed.albums.filter((v: unknown) => typeof v === "string");
    }
    if (Array.isArray(parsed?.artists)) {
      artistTopOrder.value = parsed.artists.filter((v: unknown) => typeof v === "string");
    }
  }
} catch (e) {
  console.error("Ошибка загрузки top-order из localStorage:", e);
}

watch(
  [topOrder, albumTopOrder, artistTopOrder],
  () => {
    try {
      localStorage.setItem(
        TOP_ORDER_STORAGE_KEY,
        JSON.stringify({
          playlists: topOrder.value,
          albums: albumTopOrder.value,
          artists: artistTopOrder.value,
        }),
      );
    } catch (e) {
      console.error("Ошибка сохранения top-order в localStorage:", e);
    }
  },
  { deep: true },
);

// Singleton: computed filters are created once per app, not per calling component.
let playlistsInstance: ReturnType<typeof createPlaylists> | null = null;

export function usePlaylists() {
  if (!playlistsInstance) {
    playlistsInstance = effectScope(true).run(createPlaylists)!;
  }
  return playlistsInstance;
}

function createPlaylists() {
  const albums = computed(() => playlists.value.filter((p) => p.kind === "Album"));
  const artists = computed(() => playlists.value.filter((p) => p.kind === "Artist"));
  // The built-in "Мне нравится" playlist (created on first launch by the backend).
  const favoritesPlaylist = computed(() => playlists.value.find((p) => p.kind === "Favorites") ?? null);
  const userPlaylists = computed(() => {
    const all = playlists.value.filter((p) => p.kind === "User");
    const topSet = new Set(topOrder.value);
    const top: PlaylistInfo[] = [];
    for (const id of topOrder.value) {
      const pl = all.find((p) => p.id === id);
      if (pl) top.push(pl);
    }
    const rest = all.filter((p) => !topSet.has(p.id)).sort((a, b) => b.id - a.id);
    return [...top, ...rest];
  });

  /** First free "Новый плейлист N" name. */
  function nextPlaylistName(): string {
    const re = /^Новый плейлист\s+(\d+)$/;
    let maxN = 0;
    for (const pl of userPlaylists.value) {
      const m = pl.name.match(re);
      if (m) maxN = Math.max(maxN, parseInt(m[1], 10));
    }
    return `Новый плейлист ${maxN + 1}`;
  }

  function movePlaylistToTop(playlistId: number) {
    const idx = topOrder.value.indexOf(playlistId);
    if (idx >= 0) topOrder.value.splice(idx, 1);
    topOrder.value.unshift(playlistId);
  }

  function moveAlbumToTop(name: string) {
    const idx = albumTopOrder.value.indexOf(name);
    if (idx >= 0) albumTopOrder.value.splice(idx, 1);
    albumTopOrder.value.unshift(name);
  }

  function moveArtistToTop(name: string) {
    const idx = artistTopOrder.value.indexOf(name);
    if (idx >= 0) artistTopOrder.value.splice(idx, 1);
    artistTopOrder.value.unshift(name);
  }

  async function loadPlaylists() {
    try {
      loading.value = true;
      playlists.value = await invoke<PlaylistInfo[]>("playlist_get_all");
      await loadFavorites();
    } catch (e) {
      console.error("Ошибка загрузки плейлистов:", e);
    } finally {
      loading.value = false;
    }
  }

  async function loadFavorites() {
    favoritesLoadedOnce = true;
    const fav = favoritesPlaylist.value;
    if (!fav) {
      favoritePaths.value = new Set();
      return;
    }
    try {
      const paths = await invoke<string[]>("playlist_get_track_paths", { playlistId: fav.id });
      favoritePaths.value = new Set(paths);
    } catch (e) {
      console.error("Ошибка загрузки избранного:", e);
    }
  }

  // Guarantees favorites membership is hydrated for views that render hearts but
  // don't otherwise call loadPlaylists (catalog, album, artist, search).
  async function ensureFavoritesLoaded() {
    if (favoritesLoadedOnce) return;
    if (!playlists.value.length) await loadPlaylists();
    else await loadFavorites();
  }

  function isFavorite(filePath: string): boolean {
    return favoritePaths.value.has(filePath);
  }

  // Add/remove the track from the favorites playlist. Optimistic: the heart
  // flips at once and flips back if the backend call fails.
  async function toggleFavorite(filePath: string) {
    await ensureFavoritesLoaded();
    const fav = favoritesPlaylist.value;
    if (!fav) return;
    if (favoritePaths.value.has(filePath)) {
      favoritePaths.value.delete(filePath);
      if (!(await removeTrackFromPlaylist(fav.id, filePath))) favoritePaths.value.add(filePath);
    } else {
      favoritePaths.value.add(filePath);
      // New likes go to the top of the favorites playlist.
      if (!(await addTrackToPlaylist(fav.id, filePath, true))) favoritePaths.value.delete(filePath);
    }
  }

  async function getPlaylistTracks(playlistId: number): Promise<TrackMetadata[]> {
    const tracks = await invoke<TrackMetadata[]>("playlist_get_tracks", { playlistId });
    const playlist = playlists.value.find((p) => p.id === playlistId);
    if (playlist && playlist.kind === "Album") {
      // Same order the album page shows (user's saved order, else by file path),
      // so a card's play button and a restored queue match the page.
      const { loadOrder, applyOrder } = useAlbumTrackOrder();
      await loadOrder(playlist.name);
      return applyOrder(playlist.name, tracks);
    }
    return tracks;
  }

  async function createPlaylist(name: string): Promise<PlaylistInfo | null> {
    try {
      const playlist = await invoke<PlaylistInfo>("playlist_create", { name });
      playlists.value.push(playlist);
      return playlist;
    } catch (e) {
      console.error("Ошибка создания плейлиста:", e);
      return null;
    }
  }

  async function renamePlaylist(playlistId: number, name: string) {
    try {
      await invoke("playlist_rename", { playlistId, name });
      const idx = playlists.value.findIndex((p) => p.id === playlistId);
      if (idx !== -1) playlists.value[idx].name = name;
    } catch (e) {
      console.error("Ошибка переименования плейлиста:", e);
    }
  }

  async function deletePlaylist(playlistId: number) {
    try {
      // Backend's delete_playlist decrements playlist_add_count for each track, symmetric to remove_track.
      await invoke("playlist_delete", { playlistId });
      playlists.value = playlists.value.filter((p) => p.id !== playlistId);
      const idx = topOrder.value.indexOf(playlistId);
      if (idx >= 0) topOrder.value.splice(idx, 1);
    } catch (e) {
      console.error("Ошибка удаления плейлиста:", e);
    }
  }

  /** Resolves to false when the backend call failed (callers roll back their UI). */
  async function addTrackToPlaylist(playlistId: number, filePath: string, prepend = false): Promise<boolean> {
    try {
      // false = the track was already in the playlist, so the count stays.
      const added = await invoke<boolean>("playlist_add_track", { playlistId, filePath, prepend });
      const idx = playlists.value.findIndex((p) => p.id === playlistId);
      if (added && idx !== -1) playlists.value[idx].track_count++;
      return true;
    } catch (e) {
      console.error("Ошибка добавления трека:", e);
      return false;
    }
  }

  /** Resolves to false when the backend call failed (callers roll back their UI). */
  async function removeTrackFromPlaylist(playlistId: number, filePath: string): Promise<boolean> {
    try {
      const removed = await invoke<boolean>("playlist_remove_track", { playlistId, filePath });
      const idx = playlists.value.findIndex((p) => p.id === playlistId);
      if (removed && idx !== -1) playlists.value[idx].track_count--;
      return true;
    } catch (e) {
      console.error("Ошибка удаления трека:", e);
      return false;
    }
  }

  async function movePlaylistTrackToTop(playlistId: number, filePath: string) {
    try {
      await invoke("playlist_move_track_to_top", { playlistId, filePath });
    } catch (e) {
      console.error("Ошибка перемещения трека:", e);
    }
  }

  async function getPlaylistIdsForTrack(filePath: string): Promise<number[]> {
    try {
      return await invoke<number[]>("playlist_get_ids_for_track", { filePath });
    } catch (e) {
      console.error("Ошибка получения плейлистов трека:", e);
      return [];
    }
  }

  async function rebuildArtists() {
    try {
      await invoke("library_rebuild_artists");
      await loadPlaylists();
    } catch (e) {
      console.error("Ошибка пересборки исполнителей:", e);
    }
  }

  return {
    playlists,
    albums,
    artists,
    userPlaylists,
    favoritesPlaylist,
    loading,
    rebuildArtists,

    albumTopOrder,
    artistTopOrder,

    favoritePaths,
    loadPlaylists,
    loadFavorites,
    ensureFavoritesLoaded,
    isFavorite,
    toggleFavorite,
    getPlaylistTracks,
    createPlaylist,
    nextPlaylistName,
    renamePlaylist,
    deletePlaylist,
    addTrackToPlaylist,
    removeTrackFromPlaylist,
    movePlaylistTrackToTop,
    movePlaylistToTop,
    moveAlbumToTop,
    moveArtistToTop,
    getPlaylistIdsForTrack,
  };
}
