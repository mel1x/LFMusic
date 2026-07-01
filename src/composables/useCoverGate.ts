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
    options: { timeout?: number; armed?: () => boolean } = {},
) {
    const timeout = options.timeout ?? 3000;
    const armedFn = options.armed ?? (() => true);
    const { loadCoversForTracks } = usePlayer();

    const ready = ref(false);
    let timer: ReturnType<typeof setTimeout> | null = null;

    const targets = computed(() => targetTracks());
    const targetKey = computed(() => targets.value.map(t => t.file_path).join("|"));
    // `armed` gates the reveal: while the data source isn't ready yet (e.g.
    // playlists still loading), an empty target list must NOT count as "all
    // loaded" — otherwise the gate would open before data arrives, flash empty
    // content, then show the loader only after covers start loading (reversed order).
    const armed = computed(() => armedFn());
    const resolved = computed(() =>
        armed.value && targets.value.every(t => isCoverResolved(t)),
    );

    function clearTimer() {
        if (timer) { clearTimeout(timer); timer = null; }
    }

    // Re-gate whenever the target set changes (new list / new search query) or
    // the source becomes armed (data finished loading).
    watch([targetKey, armed], () => {
        ready.value = false;
        clearTimer();
        const list = targets.value;
        if (list.length) loadCoversForTracks(list);
        if (resolved.value) {
            ready.value = true;
        } else if (armed.value) {
            // Safety timer only starts once armed — otherwise on slow data loads it
            // would fire before data arrives (the same "reversed order" flash this
            // gate exists to prevent). Once armed flips true this watch re-runs
            // (armed is a dependency) and starts the timer correctly.
            timer = setTimeout(() => { ready.value = true; timer = null; }, timeout);
        }
    }, { immediate: true });

    // Reveal as soon as the first covers finish loading (once armed).
    watch(resolved, (v) => {
        if (v && !ready.value) {
            ready.value = true;
            clearTimer();
        }
    });

    onUnmounted(clearTimer);

    return { ready };
}
