import { reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export type CoverKind = "catalog" | "playlist" | "album" | "artist";

interface BackendEntry {
  kind: string;
  key: string;
  data_url: string;
}

// Reactive object so per-key updates are tracked by consumers (template, computed)
// without rebuilding the whole map.
const customCovers = reactive<Record<string, string>>({});
let initialized = false;
let loadingPromise: Promise<void> | null = null;

function makeId(kind: CoverKind, key: string | number): string {
  return `${kind}:${String(key)}`;
}

async function loadAll(): Promise<void> {
  if (initialized) return;
  if (loadingPromise) return loadingPromise;
  loadingPromise = (async () => {
    try {
      const list = await invoke<BackendEntry[]>("cover_load_all");
      for (const e of list) {
        customCovers[`${e.kind}:${e.key}`] = e.data_url;
      }
      initialized = true;
    } catch (e) {
      console.error("Не удалось загрузить кастомные обложки:", e);
      initialized = true;
    } finally {
      loadingPromise = null;
    }
  })();
  return loadingPromise;
}

export function useCustomCovers() {
  function getCustomCover(kind: CoverKind, key: string | number): string | null {
    return customCovers[makeId(kind, key)] ?? null;
  }

  function hasCustomCover(kind: CoverKind, key: string | number): boolean {
    return makeId(kind, key) in customCovers;
  }

  async function pickAndSetCustomCover(
    kind: CoverKind,
    key: string | number,
  ): Promise<string | null> {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        filters: [
          {
            name: "Изображения",
            extensions: ["jpg", "jpeg", "png", "webp", "gif", "bmp"],
          },
        ],
      });
      if (!selected || typeof selected !== "string") return null;
      const dataUrl = await invoke<string>("cover_set_custom", {
        kind,
        key: String(key),
        sourcePath: selected,
      });
      customCovers[makeId(kind, key)] = dataUrl;
      return dataUrl;
    } catch (e) {
      console.error("Не удалось установить обложку:", e);
      return null;
    }
  }

  async function removeCustomCover(kind: CoverKind, key: string | number): Promise<void> {
    try {
      await invoke("cover_remove_custom", { kind, key: String(key) });
      delete customCovers[makeId(kind, key)];
    } catch (e) {
      console.error("Не удалось удалить обложку:", e);
    }
  }

  return {
    customCovers,
    loadCustomCovers: loadAll,
    getCustomCover,
    hasCustomCover,
    pickAndSetCustomCover,
    removeCustomCover,
  };
}
