import { ref, computed, watch, onUnmounted, type Ref, type ComputedRef } from 'vue'

export interface VirtualScrollOptions {
    itemHeight: number
    buffer?: number
}

export function useVirtualScroll<T>(
    items: Ref<T[]> | ComputedRef<T[]>,
    options: VirtualScrollOptions,
) {
    const { itemHeight, buffer = 5 } = options

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

    const range = computed(() => {
        const total = items.value.length
        if (!containerHeight.value) return { start: 0, end: Math.min(total, 20) }
        const start = Math.max(0, Math.floor(scrollTop.value / itemHeight) - buffer)
        const end = Math.min(total, Math.ceil((scrollTop.value + containerHeight.value) / itemHeight) + buffer)
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
