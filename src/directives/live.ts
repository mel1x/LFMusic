import { watchEffect, type Directive, type WatchStopHandle } from 'vue'

// Per-frame values (playback position) written straight to the DOM. The binding is a
// getter, so the owning component's render never reads the reactive source and the
// whole template is not re-rendered on every animation frame.
type Getter = () => number

function liveDirective(apply: (el: HTMLElement, v: number) => void): Directive<HTMLElement, Getter> {
    const stops = new WeakMap<HTMLElement, WatchStopHandle>()
    const getters = new WeakMap<HTMLElement, Getter>()
    return {
        mounted(el, binding) {
            getters.set(el, binding.value)
            // 'post': runs after the component patch, so e.g. a new `max` on a range
            // input is already applied before the value is written.
            stops.set(el, watchEffect(() => {
                const get = getters.get(el)
                if (get) apply(el, get())
            }, { flush: 'post' }))
        },
        updated(el, binding) {
            getters.set(el, binding.value)
        },
        unmounted(el) {
            stops.get(el)?.()
            stops.delete(el)
            getters.delete(el)
        },
    }
}

/** `v-live-width="() => percent"` -> `style.width = percent%`. */
export const vLiveWidth = liveDirective((el, v) => {
    el.style.width = `${Number.isFinite(v) ? v : 0}%`
})

/** `v-live-value="() => value"` -> input `value`. */
export const vLiveValue = liveDirective((el, v) => {
    const s = String(Number.isFinite(v) ? v : 0)
    const input = el as HTMLInputElement
    if (input.value !== s) input.value = s
})
