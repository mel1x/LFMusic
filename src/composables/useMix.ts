import { ref, watch } from "vue";

// "My Mix" settings and queue: module-level singleton persisted to localStorage
// so it survives page navigation and restart. The mix queue only exists on the
// frontend, so it's persisted here too (as file_path list) and restored by
// usePlayer.restoreState().

export interface Mood {
  id: string;
  label: string;
  desc: string;
  short_desc: string;
}

export const moods: Mood[] = [
  { id: "mix", label: "Любое", desc: "Бесконечный поток из вашей библиотеки, собранный под ваш вкус.", short_desc: "Поток под ваш вкус" },
  { id: "energetic", label: "Бодрое", desc: "Заряжающие энергией треки, чтобы держать высокий темп.", short_desc: "Заряд энергии и темп" },
  { id: "relax", label: "Спокойное", desc: "Расслабряющая музыка для отдыха и концентрации.", short_desc: "Для отдыха и концентрации" },
  { id: "dance", label: "Танцевальное", desc: "Ритмичный танцевальный поток с высокой подачей.", short_desc: "Ритмичный бит и движение" },
  { id: "melancholic", label: "Грустное", desc: "Меланхоличные треки для задумчивого настроения.", short_desc: "Задумчивые и мягкие треки" },
];

export type Familiarity = "fresh" | "balanced" | "favorite";

const SETTINGS_STORAGE_KEY = "lfmusic:mix-settings";
const QUEUE_STORAGE_KEY = "lfmusic:mix-queue";

const selectedMood = ref("mix");
const familiarity = ref<Familiarity>("balanced");
const losslessOnly = ref(false);

interface PersistedMixSettings {
  mood?: string;
  familiarity?: Familiarity;
  losslessOnly?: boolean;
}

(function loadMixSettings() {
  try {
    const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
    if (!raw) return;
    const parsed = JSON.parse(raw) as PersistedMixSettings;
    if (typeof parsed.mood === "string" && moods.some(m => m.id === parsed.mood)) {
      selectedMood.value = parsed.mood;
    }
    if (parsed.familiarity === "fresh" || parsed.familiarity === "balanced" || parsed.familiarity === "favorite") {
      familiarity.value = parsed.familiarity;
    }
    losslessOnly.value = parsed.losslessOnly === true;
  } catch (e) {
    console.error("Ошибка загрузки настроек микса:", e);
  }
})();

watch([selectedMood, familiarity, losslessOnly], () => {
  try {
    localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify({
      mood: selectedMood.value,
      familiarity: familiarity.value,
      losslessOnly: losslessOnly.value,
    } satisfies PersistedMixSettings));
  } catch (e) {
    console.error("Ошибка сохранения настроек микса:", e);
  }
});

export function useMixSettings() {
  return { selectedMood, familiarity, losslessOnly };
}

// Mix queue

export function saveMixQueuePaths(paths: string[]) {
  try {
    localStorage.setItem(QUEUE_STORAGE_KEY, JSON.stringify(paths));
  } catch (e) {
    console.error("Ошибка сохранения очереди микса:", e);
  }
}

export function loadMixQueuePaths(): string[] {
  try {
    const raw = localStorage.getItem(QUEUE_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter((p): p is string => typeof p === "string") : [];
  } catch (e) {
    console.error("Ошибка загрузки очереди микса:", e);
    return [];
  }
}
