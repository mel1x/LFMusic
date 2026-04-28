import { ref, computed, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { usePlayer } from "./usePlayer";
import { usePlaylists } from "./usePlaylists";
import { useCustomCovers } from "./useCustomCovers";

export type PinKind = "catalog" | "playlist" | "album" | "artist";

export interface PinEntry {
  kind: PinKind;
  key: string | number;
  name: string;
}

export interface ResolvedPin {
  kind: PinKind;
  key: string;
  name: string;
  cover: string | null;
  trackCount: number;
  route: string;
  exists: boolean;
  firstPath: string | null;
}

interface BackendPinEntry {
  kind: string;
  key: string;
  name: string;
}

interface BackendPinsState {
  pins: BackendPinEntry[];
}

const pins = ref<PinEntry[]>([]);
let initialized = false;
let saveTimer: ReturnType<typeof setTimeout> | null = null;
let suppressPersist = false;

async function loadFromBackend() {
  if (initialized) return;
  try {
    const state = await invoke<BackendPinsState>("pins_load");
    suppressPersist = true;
    pins.value = (state.pins || [])
      .filter(
        (p) =>
          p &&
          typeof p.key === "string" &&
          typeof p.name === "string" &&
          ["catalog", "playlist", "album", "artist"].includes(p.kind),
      )
      .map((p) => ({ kind: p.kind as PinKind, key: p.key, name: p.name }));
    initialized = true;
    setTimeout(() => {
      suppressPersist = false;
    }, 0);
  } catch (e) {
    console.error("Ошибка загрузки закреплений:", e);
    initialized = true;
  }
}

function persistPins() {
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      const payload: BackendPinsState = {
        pins: pins.value.map((p) => ({ kind: p.kind, key: String(p.key), name: p.name })),
      };
      await invoke("pins_save", { pinsState: payload });
    } catch (e) {
      console.error("Ошибка сохранения закреплений:", e);
    }
  }, 200);
}

watch(
  pins,
  () => {
    if (suppressPersist) return;
    persistPins();
  },
  { deep: true },
);

export function usePins() {
  const { trackList, getTrackByPath } = usePlayer();
  const { playlists, albums, artists } = usePlaylists();
  const { getCustomCover } = useCustomCovers();

  function isPinned(kind: PinKind, key: string | number): boolean {
    const k = String(key);
    return pins.value.some((p) => p.kind === kind && p.key === k);
  }

  function pin(entry: PinEntry) {
    if (isPinned(entry.kind, entry.key)) return;
    pins.value.unshift({ ...entry, key: String(entry.key) });
  }

  function unpin(kind: PinKind, key: string | number) {
    const k = String(key);
    const i = pins.value.findIndex((p) => p.kind === kind && p.key === k);
    if (i >= 0) pins.value.splice(i, 1);
  }

  function togglePin(entry: PinEntry) {
    if (isPinned(entry.kind, entry.key)) unpin(entry.kind, entry.key);
    else pin(entry);
  }

  function updatePinName(kind: PinKind, key: string | number, name: string) {
    const k = String(key);
    const found = pins.value.find((p) => p.kind === kind && p.key === k);
    if (found && found.name !== name) found.name = name;
  }

  function movePinToTop(kind: PinKind, key: string | number) {
    const k = String(key);
    const i = pins.value.findIndex((p) => p.kind === kind && p.key === k);
    if (i <= 0) return;
    const [item] = pins.value.splice(i, 1);
    pins.value.unshift(item);
  }

  const resolvedPins = computed<ResolvedPin[]>(() => {
    return pins.value.map((p) => resolvePin(p));
  });

  function resolvePin(p: PinEntry): ResolvedPin {
    const key = String(p.key);
    const customCover = getCustomCover(p.kind, key);
    if (p.kind === "catalog") {
      const first = trackList.value.find((t) => !!t.cover_art) || trackList.value[0];
      return {
        kind: p.kind,
        key,
        name: p.name,
        cover: customCover ?? first?.cover_art ?? null,
        trackCount: trackList.value.length,
        route: "/catalog",
        exists: true,
        firstPath: first?.file_path ?? null,
      };
    }
    if (p.kind === "playlist") {
      const id = Number(key);
      const pl = playlists.value.find((x) => x.id === id);
      if (!pl) {
        return {
          kind: p.kind,
          key,
          name: p.name,
          cover: null,
          trackCount: 0,
          route: `/playlist/${id}`,
          exists: false,
          firstPath: null,
        };
      }
      const first = pl.first_file_path ? getTrackByPath(pl.first_file_path) : undefined;
      return {
        kind: p.kind,
        key,
        name: pl.name,
        cover: customCover ?? first?.cover_art ?? null,
        trackCount: pl.track_count,
        route: `/playlist/${id}`,
        exists: true,
        firstPath: pl.first_file_path,
      };
    }
    if (p.kind === "album") {
      const al = albums.value.find((x) => x.name === key);
      const first = al?.first_file_path ? getTrackByPath(al.first_file_path) : undefined;
      return {
        kind: p.kind,
        key,
        name: p.name,
        cover: customCover ?? first?.cover_art ?? null,
        trackCount: al?.track_count ?? 0,
        route: `/album/${encodeURIComponent(key)}`,
        exists: !!al,
        firstPath: al?.first_file_path ?? null,
      };
    }
    // artist
    const ar = artists.value.find((x) => x.name === key);
    const first = ar?.first_file_path ? getTrackByPath(ar.first_file_path) : undefined;
    return {
      kind: p.kind,
      key,
      name: p.name,
      cover: customCover ?? first?.cover_art ?? null,
      trackCount: ar?.track_count ?? 0,
      route: `/artist/${encodeURIComponent(key)}`,
      exists: !!ar,
      firstPath: ar?.first_file_path ?? null,
    };
  }

  return {
    pins,
    resolvedPins,
    isPinned,
    pin,
    unpin,
    togglePin,
    movePinToTop,
    updatePinName,
    loadPins: loadFromBackend,
  };
}
