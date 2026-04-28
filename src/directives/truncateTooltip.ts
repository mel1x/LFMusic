import type { Directive } from 'vue'

let tooltipEl: HTMLElement | null = null
let styleInjected = false
let activeTarget: HTMLElement | null = null
let globalListenersInstalled = false

const STYLE = `
.truncate-tooltip {
    position: fixed;
    left: 0;
    top: 0;
    pointer-events: none;
    visibility: hidden;
    opacity: 0;
    background: var(--dark-bg, #1a1a1a);
    border: 1px solid hsla(0, 0%, 100%, 0.1);
    border-radius: 8px;
    padding: 6px 10px;
    font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 12px;
    font-weight: 500;
    line-height: 1.35;
    color: hsla(0, 0%, 100%, 0.9);
    max-width: min(480px, calc(100vw - 24px));
    white-space: normal;
    word-break: break-word;
    backdrop-filter: blur(12px);
    z-index: 1000001;
    transition: opacity 0.18s ease, visibility 0.18s ease, transform 0.18s ease;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
    transform: translateY(2px);
}
.truncate-tooltip.visible {
    visibility: visible;
    opacity: 1;
    transform: translateY(0);
}
`

function injectStyle() {
    if (styleInjected) return
    const style = document.createElement('style')
    style.setAttribute('data-truncate-tooltip', '')
    style.textContent = STYLE
    document.head.appendChild(style)
    styleInjected = true
}

function getTooltip(): HTMLElement {
    if (!tooltipEl) {
        injectStyle()
        tooltipEl = document.createElement('div')
        tooltipEl.className = 'truncate-tooltip'
        tooltipEl.setAttribute('role', 'tooltip')
        document.body.appendChild(tooltipEl)
    }
    return tooltipEl
}

function isOverflowing(el: HTMLElement): boolean {
    return el.scrollWidth - el.clientWidth > 1
}

function position(el: HTMLElement, tt: HTMLElement) {
    const rect = el.getBoundingClientRect()
    const ttRect = tt.getBoundingClientRect()
    const margin = 8
    let left = rect.left + rect.width / 2 - ttRect.width / 2
    let top = rect.top - ttRect.height - margin
    if (left < margin) left = margin
    const maxLeft = window.innerWidth - ttRect.width - margin
    if (left > maxLeft) left = Math.max(margin, maxLeft)
    if (top < margin) top = rect.bottom + margin
    tt.style.left = left + 'px'
    tt.style.top = top + 'px'
}

function show(el: HTMLElement) {
    if (!isOverflowing(el)) return
    const text = (el.textContent || '').trim()
    if (!text) return
    const tt = getTooltip()
    tt.textContent = text
    activeTarget = el
    tt.style.left = '-9999px'
    tt.style.top = '-9999px'
    tt.classList.add('visible')
    requestAnimationFrame(() => {
        if (activeTarget !== el) return
        position(el, tt)
    })
}

function hide(el: HTMLElement) {
    if (activeTarget !== el) return
    activeTarget = null
    if (tooltipEl) tooltipEl.classList.remove('visible')
}

function hideAny() {
    if (activeTarget) hide(activeTarget)
}

function installGlobalListeners() {
    if (globalListenersInstalled) return
    document.addEventListener('scroll', hideAny, true)
    window.addEventListener('blur', hideAny)
    window.addEventListener('resize', hideAny)
    globalListenersInstalled = true
}

interface Handlers {
    enter: () => void
    leave: () => void
}

const handlerMap = new WeakMap<HTMLElement, Handlers>()

export const vTruncate: Directive<HTMLElement> = {
    mounted(el) {
        installGlobalListeners()
        const enter = () => show(el)
        const leave = () => hide(el)
        el.addEventListener('mouseenter', enter)
        el.addEventListener('mouseleave', leave)
        el.addEventListener('mousedown', leave)
        handlerMap.set(el, { enter, leave })
    },
    unmounted(el) {
        const h = handlerMap.get(el)
        if (h) {
            el.removeEventListener('mouseenter', h.enter)
            el.removeEventListener('mouseleave', h.leave)
            el.removeEventListener('mousedown', h.leave)
            handlerMap.delete(el)
        }
        hide(el)
    },
}
