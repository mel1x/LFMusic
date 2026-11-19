/**
 * Animation loop capped at one frame per `frameMs`: waits on a timer between frames instead of
 * waking on every vsync. `tick` returning false stops it.
 */
export function frameLoop(
    tick: (now: number, dt: number) => boolean | void,
    frameMs: number | (() => number),
) {
    let timer: number | null = null
    let raf: number | null = null
    let last = 0

    const interval = () => (typeof frameMs === 'number' ? frameMs : frameMs())

    function frame(now: number) {
        raf = null
        const dt = last ? now - last : 0
        last = now
        if (tick(now, dt) === false) {
            last = 0
            return
        }
        const wait = Math.max(0, interval() - (performance.now() - now) - 8)
        timer = window.setTimeout(() => {
            timer = null
            raf = requestAnimationFrame(frame)
        }, wait)
    }

    return {
        start() {
            if (timer !== null || raf !== null) return
            last = 0
            raf = requestAnimationFrame(frame)
        },
        stop() {
            if (timer !== null) clearTimeout(timer)
            if (raf !== null) cancelAnimationFrame(raf)
            timer = null
            raf = null
            last = 0
        },
        get running() {
            return timer !== null || raf !== null
        },
    }
}
