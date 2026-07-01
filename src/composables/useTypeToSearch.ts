import { onMounted, onUnmounted, type Ref } from 'vue'

/**
 * Auto-focuses the search input on typing: if nothing (or a non-input) is
 * focused and the user types a letter/digit, focus the search field and
 * insert that character, so users don't have to click the field first.
 *
 * Listener is attached per-page (on mount / off unmount); since only one
 * router page is active at a time, there's no conflict between fields.
 *
 * We append the char to `input.value` ourselves and dispatch a native `input`
 * event, so both v-model and the page's `@input` handlers (including debounce)
 * fire. This also ensures the FIRST character isn't lost — just focusing on
 * keydown would drop the current char into the void.
 *
 * @param inputRef  ref to the page's search `<input>`
 * @param enabled   extra activation condition (e.g. search is locked);
 *                  defaults to always-on. A disabled `input.disabled` is
 *                  handled automatically.
 */
export function useTypeToSearch(
    inputRef: Ref<HTMLInputElement | null>,
    enabled: () => boolean = () => true,
) {
    function onKeydown(e: KeyboardEvent) {
        // Plain typing only: no modifiers, not during IME composition.
        if (e.ctrlKey || e.altKey || e.metaKey || e.isComposing) return
        // Single letter/digit only (Enter/Tab/arrows/F-keys/space/punctuation excluded).
        if (e.key.length !== 1 || !/[\p{L}\p{N}]/u.test(e.key)) return

        // Already typing in some input, don't interfere.
        const ae = document.activeElement as HTMLElement | null
        if (ae && (ae.tagName === 'INPUT' || ae.tagName === 'TEXTAREA' || ae.isContentEditable)) {
            return
        }

        const input = inputRef.value
        if (!input || input.disabled || !enabled()) return

        e.preventDefault()
        input.focus()
        // Append the char and emit input, so v-model/@input handlers fire.
        input.value = input.value + e.key
        input.dispatchEvent(new Event('input', { bubbles: true }))
        const len = input.value.length
        try {
            input.setSelectionRange(len, len)
        } catch {
            // Some input types don't support setSelectionRange; not critical.
        }
    }

    onMounted(() => window.addEventListener('keydown', onKeydown))
    onUnmounted(() => window.removeEventListener('keydown', onKeydown))
}
