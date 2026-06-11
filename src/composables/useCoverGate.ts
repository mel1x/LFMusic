import { computed, ref, watch, onUnmounted } from "vue";
import { usePlayer, type TrackMetadata } from "./usePlayer";
import { isCoverResolved } from "./useCoverLoader";

/**
 * Holds back first paint of a list until the covers of its first items have
 * resolved (loaded or confirmed absent), so the user never sees a flash of
 * empty/black cover squares.
 *
 * `targetTracks` should return the exact set of track objects whose covers
 * must be ready before reveal (e.g. the first 20 tracks of a list). The gate
 * proactively requests those covers, so it does not depend on the list being
 * rendered/visible. A safety `timeout` guarantees reveal even if loading
 * stalls.
 */
export function useCoverGate(
    targetTracks: () => TrackMetadata[],
    options: { timeout?: number } = {},
) {
    const timeout = options.timeout ?? 3000;
    const { loadCoversForTracks } = usePlayer();

    const ready = ref(false);
    let timer: ReturnType<typeof setTimeout> | null = null;

    const targets = computed(() => targetTracks());
    const targetKey = computed(() => targets.value.map(t => t.file_path).join("|"));
    const allResolved = computed(() =>
        targets.value.every(t => isCoverResolved(t)),
    );

    function clearTimer() {
        if (timer) { clearTimeout(timer); timer = null; }
    }

    // Re-gate whenever the target set changes (new list / new search query).
    watch(targetKey, () => {
        ready.value = false;
        clearTimer();
        const list = targets.value;
        if (list.length) loadCoversForTracks(list);
        if (allResolved.value) {
            ready.value = true;
        } else {
            timer = setTimeout(() => { ready.value = true; timer = null; }, timeout);
        }
    }, { immediate: true });

    // Reveal as soon as the first covers finish loading.
    watch(allResolved, (v) => {
        if (v && !ready.value) {
            ready.value = true;
            clearTimer();
        }
    });

    onUnmounted(clearTimer);

    return { ready };
}
