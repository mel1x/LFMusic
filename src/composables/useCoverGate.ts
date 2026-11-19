import { computed, ref, watch, onUnmounted } from "vue";
import { usePlayer, type TrackMetadata } from "./usePlayer";
import { isCoverResolved } from "./useCoverLoader";

/**
 * Holds back a list's first paint until the covers of its first items resolve; `timeout`
 * guarantees the reveal.
 */
export function useCoverGate(
    targetTracks: () => TrackMetadata[],
    options: { timeout?: number; armed?: () => boolean } = {},
) {
    const timeout = options.timeout ?? 3000;
    const armedFn = options.armed ?? (() => true);
    const { loadCoversForTracks } = usePlayer();

    const ready = ref(false);
    let timer: ReturnType<typeof setTimeout> | null = null;

    const targets = computed(() => targetTracks());
    const targetKey = computed(() => targets.value.map(t => t.file_path).join("|"));
    // `armed`: until the data is ready an empty list must not count as loaded, or content flashes
    // before the loader.
    const armed = computed(() => armedFn());
    const resolved = computed(() =>
        armed.value && targets.value.every(t => isCoverResolved(t)),
    );

    function clearTimer() {
        if (timer) { clearTimeout(timer); timer = null; }
    }

    watch([targetKey, armed], () => {
        ready.value = false;
        clearTimer();
        const list = targets.value;
        if (list.length) loadCoversForTracks(list);
        if (resolved.value) {
            ready.value = true;
        } else if (armed.value) {
            timer = setTimeout(() => { ready.value = true; timer = null; }, timeout);
        }
    }, { immediate: true });

    watch(resolved, (v) => {
        if (v && !ready.value) {
            ready.value = true;
            clearTimer();
        }
    });

    onUnmounted(clearTimer);

    return { ready };
}
