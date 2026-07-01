import { ref, computed, effectScope } from "vue";
import { isAndroidRuntime } from "./usePlatform";

// Android shell: runtime permission status (media/notifications) and safe-area
// insets (exposed as CSS vars). Listens to the window.LFMusicAndroid bridge and
// focus/visibility events. Singleton shared by App (init) and
// AndroidPermissionWarning (reads state).

interface AndroidPermissionStatus {
    mediaGranted: boolean;
    notificationsGranted: boolean;
    notificationsRequired: boolean;
}

interface AndroidSafeInsets {
    top: number;
    right: number;
    bottom: number;
    left: number;
}

let androidShellInstance: ReturnType<typeof createAndroidShell> | null = null;

export function useAndroidShell() {
    if (!androidShellInstance) {
        androidShellInstance = effectScope(true).run(createAndroidShell)!;
    }
    return androidShellInstance;
}

function createAndroidShell() {
    const androidPermissionStatus = ref<AndroidPermissionStatus | null>(null);
    let androidInsetsRetryTimer: ReturnType<typeof setTimeout> | null = null;
    let androidInsetsRetryCount = 0;

    const missingAndroidPermissions = computed(() => {
        const status = androidPermissionStatus.value;
        if (!isAndroidRuntime() || !status) return [];

        const missing: string[] = [];
        if (!status.mediaGranted) missing.push("доступ к медиафайлам");
        if (status.notificationsRequired && !status.notificationsGranted) missing.push("уведомления");
        return missing;
    });

    const showAndroidPermissionWarning = computed(() => missingAndroidPermissions.value.length > 0);

    const androidPermissionWarningText = computed(() => {
        const missing = missingAndroidPermissions.value;
        if (missing.length === 0) return "";
        return `Не выданы разрешения: ${missing.join(", ")}. Без них LFMusic не сможет полноценно читать локальную музыку и показывать управление плеером в уведомлениях.`;
    });

    function applyAndroidPermissionStatus(status: AndroidPermissionStatus) {
        androidPermissionStatus.value = {
            mediaGranted: Boolean(status.mediaGranted),
            notificationsGranted: Boolean(status.notificationsGranted),
            notificationsRequired: Boolean(status.notificationsRequired),
        };
    }

    function refreshAndroidPermissions() {
        if (!isAndroidRuntime()) return;
        const raw = window.LFMusicAndroid?.getPermissionStatus?.();
        if (!raw) return;

        try {
            applyAndroidPermissionStatus(JSON.parse(raw) as AndroidPermissionStatus);
        } catch {
            androidPermissionStatus.value = null;
        }
    }

    function requestAndroidPermissions() {
        window.LFMusicAndroid?.requestRequiredPermissionsFromJs?.();
        setTimeout(refreshAndroidPermissions, 500);
    }

    function applyAndroidInsets(insets: AndroidSafeInsets) {
        const root = document.documentElement.style;
        root.setProperty("--android-safe-top", `${Math.max(0, insets.top || 0)}px`);
        root.setProperty("--android-safe-right", `${Math.max(0, insets.right || 0)}px`);
        root.setProperty("--android-safe-bottom", `${Math.max(0, insets.bottom || 0)}px`);
        root.setProperty("--android-safe-left", `${Math.max(0, insets.left || 0)}px`);
    }

    function refreshAndroidInsets() {
        if (!isAndroidRuntime()) return;
        const raw = window.LFMusicAndroid?.getSystemInsets?.();
        if (!raw) return;

        try {
            applyAndroidInsets(JSON.parse(raw) as AndroidSafeInsets);
        } catch {
            // Best-effort: CSS env() and fallback values stay in place.
        }
    }

    function scheduleAndroidInsetsRefresh() {
        if (!isAndroidRuntime()) return;
        refreshAndroidInsets();
        if (androidInsetsRetryCount >= 12) return;
        androidInsetsRetryCount += 1;
        if (androidInsetsRetryTimer) clearTimeout(androidInsetsRetryTimer);
        androidInsetsRetryTimer = setTimeout(scheduleAndroidInsetsRefresh, androidInsetsRetryCount < 5 ? 120 : 350);
    }

    function onAndroidPermissionsEvent(event: Event) {
        const detail = (event as CustomEvent<AndroidPermissionStatus>).detail;
        if (!detail) return;
        applyAndroidPermissionStatus(detail);
    }

    function onAndroidInsetsEvent(event: Event) {
        const detail = (event as CustomEvent<AndroidSafeInsets>).detail;
        if (detail) applyAndroidInsets(detail);
        refreshAndroidInsets();
    }

    function onVisibilityChange() {
        if (document.visibilityState === "visible") {
            refreshAndroidPermissions();
            scheduleAndroidInsetsRefresh();
        }
    }

    function onWindowFocus() {
        refreshAndroidPermissions();
        scheduleAndroidInsetsRefresh();
    }

    // Listeners live for the app's whole lifetime (root never unmounts).
    window.addEventListener("focus", onWindowFocus);
    window.addEventListener("lfmusic:android-permissions", onAndroidPermissionsEvent as EventListener);
    window.addEventListener("lfmusic:android-insets", onAndroidInsetsEvent as EventListener);
    window.visualViewport?.addEventListener("resize", scheduleAndroidInsetsRefresh);
    document.addEventListener("visibilitychange", onVisibilityChange);

    // Initial read (bridge may already be available).
    refreshAndroidPermissions();
    scheduleAndroidInsetsRefresh();

    return {
        showAndroidPermissionWarning,
        androidPermissionWarningText,
        requestAndroidPermissions,
        refreshAndroidPermissions,
    };
}
