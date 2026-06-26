import { onMounted, onUnmounted, type Ref } from 'vue'

/**
 * Автофокус на поле поиска при наборе: если на странице ничего не сфокусировано
 * (или фокус не на поле ввода) и пользователь печатает букву/цифру — фокусируем
 * поле поиска и подставляем туда этот символ. Так не нужно сначала кликать в поле.
 *
 * Слушатель вешается per-страница (on mount → off unmount); т.к. одновременно
 * активна одна страница роутера, конфликтов между полями нет.
 *
 * Реализация: сами дописываем символ в `input.value` и шлём нативный `input`-
 * эвент — тогда отрабатывают и v-model, и `@input`-обработчики страницы (включая
 * дебаунс). Это же гарантирует, что ПЕРВАЯ буква не теряется (если бы мы просто
 * сфокусировали поле на keydown, текущий символ ушёл бы в «пустоту»).
 *
 * @param inputRef  ссылка на `<input>` поля поиска страницы
 * @param enabled   доп. условие активности (напр., поиск заблокирован) — по
 *                  умолчанию всегда включено; отключённый `input.disabled`
 *                  учитывается автоматически.
 */
export function useTypeToSearch(
    inputRef: Ref<HTMLInputElement | null>,
    enabled: () => boolean = () => true,
) {
    function onKeydown(e: KeyboardEvent) {
        // Только «чистый» набор: без модификаторов и не во время IME-композиции.
        if (e.ctrlKey || e.altKey || e.metaKey || e.isComposing) return
        // Одиночная буква/цифра (Enter/Tab/стрелки/F-клавиши/пробел/пунктуация — мимо).
        if (e.key.length !== 1 || !/[\p{L}\p{N}]/u.test(e.key)) return

        // Уже печатают в каком-то поле ввода — не вмешиваемся.
        const ae = document.activeElement as HTMLElement | null
        if (ae && (ae.tagName === 'INPUT' || ae.tagName === 'TEXTAREA' || ae.isContentEditable)) {
            return
        }

        const input = inputRef.value
        if (!input || input.disabled || !enabled()) return

        e.preventDefault()
        input.focus()
        // Дописываем символ в конец и эмитим input — отрабатывают v-model/@input.
        input.value = input.value + e.key
        input.dispatchEvent(new Event('input', { bubbles: true }))
        const len = input.value.length
        try {
            input.setSelectionRange(len, len)
        } catch {
            /* у некоторых типов input нет setSelectionRange — не критично */
        }
    }

    onMounted(() => window.addEventListener('keydown', onKeydown))
    onUnmounted(() => window.removeEventListener('keydown', onKeydown))
}
