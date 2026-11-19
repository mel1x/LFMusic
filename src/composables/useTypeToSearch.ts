import { onMounted, onUnmounted, type Ref } from 'vue'

/**
 * Typing a letter or digit with nothing focused focuses the search input and inserts the char (an
 * `input` event is dispatched, so the first char is not lost).
 */
export function useTypeToSearch(
    inputRef: Ref<HTMLInputElement | null>,
    enabled: () => boolean = () => true,
) {
    function onKeydown(e: KeyboardEvent) {
        if (e.ctrlKey || e.altKey || e.metaKey || e.isComposing) return
        if (e.key.length !== 1 || !/[\p{L}\p{N}]/u.test(e.key)) return

        const ae = document.activeElement as HTMLElement | null
        if (ae && (ae.tagName === 'INPUT' || ae.tagName === 'TEXTAREA' || ae.isContentEditable)) {
            return
        }

        const input = inputRef.value
        if (!input || input.disabled || !enabled()) return

        e.preventDefault()
        input.focus()
        input.value = input.value + e.key
        input.dispatchEvent(new Event('input', { bubbles: true }))
        const len = input.value.length
        try {
            input.setSelectionRange(len, len)
        } catch {
        }
    }

    onMounted(() => window.addEventListener('keydown', onKeydown))
    onUnmounted(() => window.removeEventListener('keydown', onKeydown))
}
