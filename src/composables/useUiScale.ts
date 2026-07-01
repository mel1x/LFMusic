import { onMounted, onUnmounted } from "vue";
import { isPhoneViewport, updateRuntimeClasses } from "./usePlatform";

// Scales the UI to window size (base 1460x860) via body { zoom }; call once at the app root.
// Full-viewport fixed overlays scale separately via --overlay-scale.

const BASE_W = 1460;
const BASE_H = 860;
const MIN_SCALE = 0.78;
const MAX_SCALE = 2.0;

export function useUiScale() {
    let resizeRaf: number | null = null;

    function resetUiScale() {
        const body = document.body;
        (body.style as any).zoom = "";
        body.style.width = "100vw";
        body.style.height = "100dvh";
        document.documentElement.style.setProperty("--ui-scale", "1");
        document.documentElement.style.setProperty("--overlay-scale", "1");
        document.documentElement.style.setProperty("--overlay-scale", "1");
        (window as any).__uiScale = 1;
    }

    // In Chromium (WebView2), `zoom` doesn't affect `vw`, so a 100vw overlay inside a
    // zoomed body overflows the viewport by the scale factor — measure the actual
    // divisor instead of assuming it equals `scale`. WebKitGTK doesn't use zoom at all
    // (goes through resetUiScale instead).
    function measureOverlayScale(fallback: number) {
        let divisor = fallback;
        try {
            const probe = document.createElement("div");
            probe.style.cssText =
                "position:fixed;left:0;top:0;width:100vw;height:0;visibility:hidden;pointer-events:none;margin:0;padding:0;border:0";
            document.body.appendChild(probe);
            const measured = probe.getBoundingClientRect().width;
            document.body.removeChild(probe);
            const vw = window.innerWidth;
            if (measured > 0 && vw > 0) divisor = measured / vw;
        } catch {
            // Fall back to `scale` on measurement failure.
        }
        if (!isFinite(divisor) || divisor <= 0) divisor = fallback;
        document.documentElement.style.setProperty("--overlay-scale", String(divisor));
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
        (window as any).__uiScale = scale;
        measureOverlayScale(scale);
        measureOverlayScale(scale);
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
