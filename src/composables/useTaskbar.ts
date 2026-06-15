import { computed, watch, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { toast } from "vue-sonner";
import { usePlayer } from "./usePlayer";

// ──────────────────────────────────────────────────────────────────────────────
// Интеграция с панелью задач Windows: иконки play/shuffle/repeat, превью трёх
// обложек (prev/current/next) и приём нажатий кнопок-миниатюр (taskbar_media).
// Вынесено из App.vue без изменения поведения; вызывается один раз в корне.
// ──────────────────────────────────────────────────────────────────────────────

export function useTaskbar() {
    const {
        currentTrack,
        isPlaying,
        shuffled,
        repeatMode,
        playbackOrder,
        currentTrackIndex,
        togglePlayPause,
        playNext,
        playPrev,
        toggleShuffle,
        toggleRepeat,
    } = usePlayer();

    let unlistenTaskbarMedia: UnlistenFn | null = null;

    // Соседние треки в очереди (свежие объекты из trackMap — учёт ленивой подгрузки).
    // Очередь закольцована: кнопки prev/next на краях оборачиваются (см. playPrev/playNext),
    // поэтому превью тоже берёт соседей по кругу, а не оставляет пустой слот на стыке —
    // иначе в shuffle (где текущий трек стоит в начале playbackOrder) обложка предыдущего
    // не показывалась, пока не переключишься назад.
    // Очередь хранит пути (file_path); бэкенд таскбара сам читает обложки по пути.
    function queuePath(offset: number): string | null {
        const len = playbackOrder.value.length;
        const i = currentTrackIndex.value;
        if (i < 0 || len === 0) return null;
        const wrapped = (((i + offset) % len) + len) % len;
        if (wrapped === i) return null; // очередь из одного трека — соседей нет
        return playbackOrder.value[wrapped] ?? null;
    }
    const previewTracks = computed(() => ({
        prev: queuePath(-1),
        current: currentTrack.value?.file_path ?? null,
        next: queuePath(1),
    }));

    // Состояние воспроизведения → иконка кнопки play/pause в панели задач.
    watch(isPlaying, (playing) => {
        invoke("taskbar_set_playing", { playing }).catch(() => {});
    }, { immediate: true });

    // Состояния shuffle/repeat → иконки кнопок в панели задач.
    watch([shuffled, repeatMode], ([sh, rep]) => {
        invoke("taskbar_set_modes", { shuffled: sh, repeatTrack: rep === "track" }).catch(() => {});
    }, { immediate: true });

    // Превью в панели задач: бэкенд сам читает обложки из кэша WebP-миниатюр по
    // file_path (без base64) и генерирует недостающие на лету, поэтому ждать
    // загрузки cover_art во фронтенде не нужно. Раньше превью держали, пока все 3
    // обложки не «готовы» во фронтенде — но обложка соседа (напр. последнего трека
    // как «предыдущего» для первого, по закольцованной очереди) могла никогда не
    // подгрузиться, если он за пределами видимой части каталога (его запрос
    // вытеснялся из общей очереди обложек), и слот «предыдущего» оставался пустым.
    // Шлём тройку сразу при любой смене — бэкенд отрисует все доступные обложки.
    let lastPreviewPaths: { prev: string | null; current: string | null; next: string | null } = {
        prev: null, current: null, next: null,
    };
    watch(previewTracks, ({ prev, current, next }) => {
        const paths = { prev, current, next };

        // Определяем тип анимации: вперёд (next стал центром), назад (prev стал центром), иначе fade.
        let transition: "forward" | "backward" | "fade" | "none" = "none";
        if (lastPreviewPaths.current && paths.current) {
            if (paths.current === lastPreviewPaths.next) transition = "forward";
            else if (paths.current === lastPreviewPaths.prev) transition = "backward";
            else transition = "fade";
        }
        lastPreviewPaths = paths;

        // Бэкенд сам читает обложки из кэша WebP-миниатюр по file_path (без base64).
        invoke("taskbar_set_covers", {
            prev,
            current,
            next,
            transition,
        }).catch(() => {});
    }, { immediate: true });

    onMounted(() => {
        // Кнопки-миниатюры панели задач (Windows) → действия плеера.
        // Для shuffle/repeat показываем те же тосты, что и кнопки в плеере.
        void listen<string>("taskbar_media", (event) => {
            switch (event.payload) {
                case "shuffle":
                    toggleShuffle();
                    toast.info(shuffled.value ? "Случайный порядок" : "Воспроизведение подряд");
                    break;
                case "prev": playPrev(); break;
                case "playpause": togglePlayPause(); break;
                case "next": playNext(); break;
                case "repeat":
                    toggleRepeat();
                    toast.warning(repeatMode.value === "track" ? "Повтор трека" : "Повтор плейлиста");
                    break;
            }
        }).then((unlisten) => { unlistenTaskbarMedia = unlisten; }).catch(() => {});
    });

    onUnmounted(() => {
        if (unlistenTaskbarMedia) {
            unlistenTaskbarMedia();
            unlistenTaskbarMedia = null;
        }
    });
}
