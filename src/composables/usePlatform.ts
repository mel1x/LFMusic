export function isAndroidRuntime(): boolean {
    if (typeof navigator === 'undefined') return false
    return /android/i.test(navigator.userAgent)
}

// WebKit без Chromium — это WebKitGTK (десктоп Linux) и WKWebView (macOS).
// У этого движка иначе ведут себя вертикальные range-инпуты (writing-mode)
// и CSS `zoom` относительно vw-единиц, поэтому такие места правим адресно.
export function isWebKitRuntime(): boolean {
    if (typeof navigator === 'undefined') return false
    const ua = navigator.userAgent
    return /AppleWebKit/.test(ua) && !/Chrome|Chromium|Edg\//.test(ua)
}

// Десктопный Linux (WebKitGTK), исключая Android.
export function isLinuxDesktop(): boolean {
    if (typeof navigator === 'undefined') return false
    const ua = navigator.userAgent
    return /Linux/.test(ua) && !/android/i.test(ua)
}

export function isPhoneViewport(): boolean {
    if (typeof window === 'undefined') return false
    return window.matchMedia(
        '(max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px)'
    ).matches
}

export function updateRuntimeClasses() {
    if (typeof document === 'undefined') return

    const phone = isPhoneViewport()
    const android = isAndroidRuntime()
    const mobile = phone || android
    const webkit = isWebKitRuntime()

    for (const root of [document.documentElement, document.body]) {
        root.classList.toggle('lfmusic-phone', phone)
        root.classList.toggle('lfmusic-android', android)
        root.classList.toggle('lfmusic-mobile', mobile)
        root.classList.toggle('lfmusic-webkit', webkit)
    }
}
