import { onMounted, onUnmounted } from "vue";
import { isPhoneViewport, updateRuntimeClasses } from "./usePlatform";

// Scales the UI to window size (base 1460x860) via body { zoom }; call once at the app root.
// Full-viewport fixed overlays use the same scale as the body so their compensated
// dimensions stay in sync throughout maximize/fullscreen resize sequences.

const BASE_W = 1460;
const BASE_H = 860;
const MIN_SCALE = 0.78;
const MAX_SCALE = 2.0;

/** Current body zoom set by `useUiScale` (1 on phones or before the first apply). */
export function currentUiScale(): number {
    const scale = Number((window as any).__uiScale);
    if (Number.isFinite(scale) && scale > 0) return scale;
    const cssScale = Number(getComputedStyle(document.documentElement)
        .getPropertyValue("--ui-scale")
        .trim());
    return Number.isFinite(cssScale) && cssScale > 0 ? cssScale : 1;
}

export function useUiScale() {
    let resizeRaf: number | null = null;

    function resetUiScale() {
        const body = document.body;
        (body.style as any).zoom = "";
        body.style.width = "100vw";
        body.style.height = "100dvh";
        document.documentElement.style.setProperty("--ui-scale", "1");
        document.documentElement.style.setProperty("--overlay-scale", "1");
        (window as any).__uiScale = 1;
    }

    function applyUiScale() {
        resizeRaf = null;
        updateRuntimeClasses();

        if (isPhoneViewport()) {
            resetUiScale();
            return;
        }

        const w = window.innerWidth;
        const h = window.innerHeight;
        const raw = Math.min(w / BASE_W, h / BASE_H);
        const scale = Math.max(MIN_SCALE, Math.min(MAX_SCALE, raw));
        const body = document.body;
        (body.style as any).zoom = String(scale);
        body.style.width = (w / scale) + "px";
        body.style.height = (h / scale) + "px";
        document.documentElement.style.setProperty("--ui-scale", String(scale));
        document.documentElement.style.setProperty("--overlay-scale", String(scale));
        (window as any).__uiScale = scale;
    }

    function scheduleUiScale() {
        if (resizeRaf !== null) return;
        resizeRaf = requestAnimationFrame(applyUiScale);
    }

    onMounted(() => {
        applyUiScale();
        window.addEventListener("resize", scheduleUiScale);
        window.visualViewport?.addEventListener("resize", scheduleUiScale);
    });

    onUnmounted(() => {
        window.removeEventListener("resize", scheduleUiScale);
        window.visualViewport?.removeEventListener("resize", scheduleUiScale);
        if (resizeRaf !== null) {
            cancelAnimationFrame(resizeRaf);
            resizeRaf = null;
        }
    });

    return { applyUiScale, scheduleUiScale };
}
