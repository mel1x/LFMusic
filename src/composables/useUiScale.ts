import { onMounted, onUnmounted } from "vue";
import { isPhoneViewport, updateRuntimeClasses } from "./usePlatform";

// ──────────────────────────────────────────────────────────────────────────────
// Масштабирование интерфейса под размер окна (base 1460×860) через body { zoom }.
// Полноэкранные fixed-оверлеи масштабируются отдельной переменной --overlay-scale.
// Вынесено из App.vue без изменения поведения; вызывается один раз в корне.
// ──────────────────────────────────────────────────────────────────────────────

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

    // Полноэкранные fixed-оверлеи (модалки, затемнённые фоны) масштабируем через
    // отдельную переменную --overlay-scale. В Chromium (WebView2) `zoom` не влияет на
    // vw, поэтому 100vw-оверлей внутри отмасштабированного body перекрывает вьюпорт в
    // `scale` раз — делим на это. Замеряем коэффициент напрямую, а не берём scale
    // вслепую. (На WebKitGTK zoom не используется вовсе — там путь resetUiScale.)
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
            // На ошибке оставляем fallback (scale) — поведение как раньше.
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
