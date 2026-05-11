export function isAndroidRuntime(): boolean {
    if (typeof navigator === 'undefined') return false
    return /android/i.test(navigator.userAgent)
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

    for (const root of [document.documentElement, document.body]) {
        root.classList.toggle('lfmusic-phone', phone)
        root.classList.toggle('lfmusic-android', android)
        root.classList.toggle('lfmusic-mobile', mobile)
    }
}
