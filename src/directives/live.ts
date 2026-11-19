import { watchEffect, type Directive, type WatchStopHandle } from 'vue'

// Per-frame values written straight to the DOM: the binding is a getter, so the owner's template
// never re-renders per frame.
type Getter = () => number

function liveDirective(apply: (el: HTMLElement, v: number) => void): Directive<HTMLElement, Getter> {
    const stops = new WeakMap<HTMLElement, WatchStopHandle>()
    const getters = new WeakMap<HTMLElement, Getter>()
    return {
        mounted(el, binding) {
            getters.set(el, binding.value)
            // 'post': runs after the patch, so a new `max` is applied before the value.
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
