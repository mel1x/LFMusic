import { onUnmounted, type Ref } from 'vue'
import { isPhoneViewport } from './usePlatform'

/** Drag distance (px) past which releasing the sheet handle closes the modal. */
const CLOSE_THRESHOLD = 76

/**
 * Shared modal behavior: on desktop a click on the backdrop closes it, on phones
 * the card is a bottom sheet that closes when dragged down by its handle.
 */
export function useSheetModal(cardEl: Ref<HTMLElement | null>, close: () => void) {
    let dragStartY = 0
    let dragging = false

    function onBackdropMousedown(e: MouseEvent) {
        if (isPhoneViewport()) return
        if (e.target === e.currentTarget) close()
    }

    function removeListeners() {
        window.removeEventListener('pointermove', onSheetDragMove)
        window.removeEventListener('pointerup', onSheetDragEnd)
        window.removeEventListener('pointercancel', onSheetDragEnd)
    }

    function onSheetDragStart(e: PointerEvent) {
        if (!isPhoneViewport()) return
        e.preventDefault()
        dragging = true
        dragStartY = e.clientY
        if (cardEl.value) cardEl.value.style.transition = 'none'
        ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
        window.addEventListener('pointermove', onSheetDragMove)
        window.addEventListener('pointerup', onSheetDragEnd)
        window.addEventListener('pointercancel', onSheetDragEnd)
    }

    function onSheetDragMove(e: PointerEvent) {
        if (!dragging || !cardEl.value) return
        e.preventDefault()
        const delta = Math.max(0, e.clientY - dragStartY)
        cardEl.value.style.transform = `translateY(${delta}px)`
    }

    function onSheetDragEnd(e: PointerEvent) {
        if (!dragging) return
        dragging = false
        removeListeners()
        const delta = Math.max(0, e.clientY - dragStartY)
        if (delta > CLOSE_THRESHOLD) {
            close()
            return
        }
        if (cardEl.value) {
            cardEl.value.style.transition = ''
            cardEl.value.style.transform = ''
        }
    }

    onUnmounted(removeListeners)

    return { onBackdropMousedown, onSheetDragStart }
}
