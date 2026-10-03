import { ref, computed, watch, onUnmounted, type Ref, type ComputedRef } from 'vue'

export interface VirtualScrollOptions {
    itemHeight: number
    buffer?: number
    scrollOffset?: Ref<number> | ComputedRef<number>
}

export function useVirtualScroll<T>(
    items: Ref<T[]> | ComputedRef<T[]>,
    options: VirtualScrollOptions,
) {
    const { itemHeight, buffer = 5, scrollOffset } = options

    const containerEl = ref<HTMLElement | null>(null)
    const scrollTop = ref(0)
    const containerHeight = ref(0)

    let resizeObserver: ResizeObserver | null = null

    watch(containerEl, (el) => {
        if (resizeObserver) resizeObserver.disconnect()
        if (el) {
            containerHeight.value = el.clientHeight
            resizeObserver = new ResizeObserver(() => {
                if (containerEl.value) {
                    containerHeight.value = containerEl.value.clientHeight
                }
            })
            resizeObserver.observe(el)
        }
    })

    onUnmounted(() => {
        if (resizeObserver) resizeObserver.disconnect()
    })

    function onScroll(e: Event) {
        scrollTop.value = (e.target as HTMLElement).scrollTop
    }

    // Returns the previous object while start/end are unchanged, so scroll events
    // within the same row don't re-slice the list and re-render every visible row.
    const range = computed<{ start: number; end: number }>((prev) => {
        const total = items.value.length
        let start = 0
        let end = Math.min(total, 20)
        if (containerHeight.value) {
            const offset = scrollOffset?.value ?? 0
            const adjustedScroll = Math.max(0, scrollTop.value - offset)
            start = Math.max(0, Math.floor(adjustedScroll / itemHeight) - buffer)
            end = Math.min(total, Math.ceil((adjustedScroll + containerHeight.value) / itemHeight) + buffer)
        }
        if (prev && prev.start === start && prev.end === end) return prev
        return { start, end }
    })

    const visibleItems = computed(() => items.value.slice(range.value.start, range.value.end))
    const paddingTop = computed(() => range.value.start * itemHeight)
    const paddingBottom = computed(() => Math.max(0, (items.value.length - range.value.end) * itemHeight))

    function resetScroll() {
        scrollTop.value = 0
        if (containerEl.value) containerEl.value.scrollTop = 0
    }

    return {
        containerEl,
        scrollTop,
        containerHeight,
        onScroll,
        visibleItems,
        paddingTop,
        paddingBottom,
        resetScroll,
    }
}
