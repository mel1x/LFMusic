import { computed, watch, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { toast } from "vue-sonner";
import { usePlayer } from "./usePlayer";

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

    function queuePath(offset: number): string | null {
        const len = playbackOrder.value.length;
        const i = currentTrackIndex.value;
        if (i < 0 || len === 0) return null;
        const wrapped = (((i + offset) % len) + len) % len;
        if (wrapped === i) return null;
        return playbackOrder.value[wrapped] ?? null;
    }
    const previewTracks = computed(() => ({
        prev: queuePath(-1),
        current: currentTrack.value?.file_path ?? null,
        next: queuePath(1),
    }));

    watch(isPlaying, (playing) => {
        invoke("taskbar_set_playing", { playing }).catch(() => {});
    }, { immediate: true });

    watch([shuffled, repeatMode], ([sh, rep]) => {
        invoke("taskbar_set_modes", { shuffled: sh, repeatTrack: rep === "track" }).catch(() => {});
    }, { immediate: true });

    // Send the triple on every change: the backend renders whatever covers exist (waiting for all
    // three could stall forever).
    let lastPreviewPaths: { prev: string | null; current: string | null; next: string | null } = {
        prev: null, current: null, next: null,
    };
    watch(previewTracks, ({ prev, current, next }) => {
        const paths = { prev, current, next };

        let transition: "forward" | "backward" | "fade" | "none" = "none";
        if (lastPreviewPaths.current && paths.current) {
            if (paths.current === lastPreviewPaths.next) transition = "forward";
            else if (paths.current === lastPreviewPaths.prev) transition = "backward";
            else transition = "fade";
        }
        lastPreviewPaths = paths;

        invoke("taskbar_set_covers", {
            prev,
            current,
            next,
            transition,
        }).catch(() => {});
    }, { immediate: true });

    onMounted(() => {
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
