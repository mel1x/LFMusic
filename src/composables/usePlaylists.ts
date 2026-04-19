import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { TrackMetadata } from "./usePlayer";

// ──────────────────────────────────────────────────────────────────────────────
// Типы
// ──────────────────────────────────────────────────────────────────────────────

export type PlaylistKind = "Album" | "User";

export interface PlaylistInfo {
  id: number;
  name: string;
  kind: PlaylistKind;
  track_count: number;
}

// ──────────────────────────────────────────────────────────────────────────────
// Singleton state
// ──────────────────────────────────────────────────────────────────────────────

const playlists = ref<PlaylistInfo[]>([]);
const loading = ref(false);
const topOrder = ref<number[]>([]);

// ──────────────────────────────────────────────────────────────────────────────
// Composable
// ──────────────────────────────────────────────────────────────────────────────

export function usePlaylists() {
  const albums = computed(() => playlists.value.filter((p) => p.kind === "Album"));
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

  function movePlaylistToTop(playlistId: number) {
    const idx = topOrder.value.indexOf(playlistId);
    if (idx >= 0) topOrder.value.splice(idx, 1);
    topOrder.value.unshift(playlistId);
  }

  async function loadPlaylists() {
    try {
      loading.value = true;
      playlists.value = await invoke<PlaylistInfo[]>("playlist_get_all");
    } catch (e) {
      console.error("Ошибка загрузки плейлистов:", e);
    } finally {
      loading.value = false;
    }
  }

  async function getPlaylistTracks(playlistId: number): Promise<TrackMetadata[]> {
    return invoke<TrackMetadata[]>("playlist_get_tracks", { playlistId });
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
      await invoke("playlist_delete", { playlistId });
      playlists.value = playlists.value.filter((p) => p.id !== playlistId);
      const idx = topOrder.value.indexOf(playlistId);
      if (idx >= 0) topOrder.value.splice(idx, 1);
    } catch (e) {
      console.error("Ошибка удаления плейлиста:", e);
    }
  }

  async function addTrackToPlaylist(playlistId: number, filePath: string) {
    try {
      await invoke("playlist_add_track", { playlistId, filePath });
      const idx = playlists.value.findIndex((p) => p.id === playlistId);
      if (idx !== -1) playlists.value[idx].track_count++;
    } catch (e) {
      console.error("Ошибка добавления трека:", e);
    }
  }

  async function removeTrackFromPlaylist(playlistId: number, filePath: string) {
    try {
      await invoke("playlist_remove_track", { playlistId, filePath });
      const idx = playlists.value.findIndex((p) => p.id === playlistId);
      if (idx !== -1) playlists.value[idx].track_count--;
    } catch (e) {
      console.error("Ошибка удаления трека:", e);
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

  return {
    playlists,
    albums,
    userPlaylists,
    loading,

    loadPlaylists,
    getPlaylistTracks,
    createPlaylist,
    renamePlaylist,
    deletePlaylist,
    addTrackToPlaylist,
    removeTrackFromPlaylist,
    movePlaylistTrackToTop,
    movePlaylistToTop,
    getPlaylistIdsForTrack,
  };
}
