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
    // `armed` гейтит открытие: пока источник данных не готов (например, плейлисты
    // ещё грузятся), пустой список целей НЕ должен считаться «всё загружено» —
    // иначе гейт откроется до данных, мелькнёт пустой контент, и лоадер появится
    // уже после подгрузки обложек (обратный порядок).
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
            // Таймер-страховку запускаем ТОЛЬКО когда источник готов (armed).
            // Иначе при медленной загрузке данных гейт открылся бы по таймауту до
            // появления данных, мелькнул бы пустой контент, а затем закрылся снова
            // при armed=true (тот самый «обратный порядок»). Когда armed станет
            // true, этот watch перезапустится (armed — его зависимость) и заведёт
            // таймер уже корректно.
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
