import { computed, watch, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { toast } from "vue-sonner";
import { usePlayer } from "./usePlayer";

// Windows taskbar integration: play/shuffle/repeat icons, 3-cover preview
// (prev/current/next), and thumbnail button clicks (taskbar_media). Extracted
// from App.vue without behavior change; called once at the root.

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

    // Queue wraps around (prev/next buttons wrap at the edges, see playPrev/playNext),
    // so the preview also wraps instead of leaving an empty slot at the seam — otherwise
    // in shuffle (current track at the start of playbackOrder) the "previous" cover
    // wouldn't show until switching back once.
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

    // Backend reads covers from the WebP thumbnail cache by file_path (no base64)
    // and generates missing ones on the fly, so the frontend doesn't need to wait
    // for cover_art. Previously we held the preview until all 3 covers were "ready"
    // frontend-side, but a neighbor outside the visible catalog range (e.g. the
    // last track as "previous" for the first, via the wrapped queue) could never
    // resolve — its cover request got evicted from the shared load queue — leaving
    // the slot permanently empty. Now we send the triple on every change and let
    // the backend render whatever covers are actually available.
    let lastPreviewPaths: { prev: string | null; current: string | null; next: string | null } = {
        prev: null, current: null, next: null,
    };
    watch(previewTracks, ({ prev, current, next }) => {
        const paths = { prev, current, next };

        // forward when next becomes current, backward when prev becomes current, else fade
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
        // Taskbar thumbnail buttons -> player actions; shuffle/repeat show the
        // same toasts as the in-player buttons.
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
