/**
 * Создаёт ripple-эффект на элементе по координатам клика.
 * Требует CSS-класс `.ripple` с анимацией `ripple-expand`.
 */
export function createRipple(e: MouseEvent) {
    const el = e.currentTarget as HTMLElement
    if (!el) return
    const rect = el.getBoundingClientRect()
    const ripple = document.createElement('span')
    ripple.className = 'ripple'
    const size = Math.max(rect.width, rect.height) * 2
    ripple.style.width = ripple.style.height = size + 'px'
    ripple.style.left = (e.clientX - rect.left - size / 2) + 'px'
    ripple.style.top = (e.clientY - rect.top - size / 2) + 'px'
    el.appendChild(ripple)
    ripple.addEventListener('animationend', () => ripple.remove())
}
