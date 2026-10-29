import type { Directive } from 'vue'
import { currentUiScale } from '../composables/useUiScale'

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
    background-color: hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0.15);
    border-radius: 12px;
    padding: 7px 12px;
    font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 12px;
    font-weight: 500;
    line-height: 1.35;
    color: hsla(0, 0%, 100%, 0.9);
    max-width: min(480px, calc(100vw - 24px));
    width: max-content;
    white-space: normal;
    word-break: break-word;
    backdrop-filter: blur(17px);
    z-index: 1000001;
    transition: opacity 0.18s ease, visibility 0.18s ease, transform 0.18s ease;
    box-shadow: 0 0px 16px rgba(0, 0, 0, 0.05);
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

// Text can be clipped by width (single-line ellipsis) or height (-webkit-line-clamp);
// check both so the directive works for single- and multi-line truncation.
function isOverflowing(el: HTMLElement): boolean {
    return el.scrollWidth - el.clientWidth > 1 || el.scrollHeight - el.clientHeight > 1
}

type Placement = 'auto' | 'top' | 'bottom'

const placementMap = new WeakMap<HTMLElement, Placement>()

// In Chromium (WebView2), body is scaled via `zoom` and getBoundingClientRect returns
// visual (zoomed) coordinates. The tooltip lives in that zoomed body, so its left/top
// (CSS px) must be divided by scale or it drifts under zoom (e.g. downward when forced to top).
function position(el: HTMLElement, tt: HTMLElement) {
    const rect = el.getBoundingClientRect()
    const ttRect = tt.getBoundingClientRect()
    const margin = 8
    let left = rect.left + rect.width / 2 - ttRect.width / 2
    if (left < margin) left = margin
    const maxLeft = window.innerWidth - ttRect.width - margin
    if (left > maxLeft) left = Math.max(margin, maxLeft)

    const placement = placementMap.get(el) ?? 'auto'
    // Forced placement (player bar) keeps the tooltip closer to the element, otherwise
    // it hangs too high above long titles/artists.
    const gap = placement === 'auto' ? margin : 3
    const above = rect.top - ttRect.height - gap
    const below = rect.bottom + gap
    let top: number
    if (placement === 'top') {
        top = Math.max(margin, above)
    } else if (placement === 'bottom') {
        top = below
    } else {
        top = above
        if (top < margin) top = below
    }
    const scale = currentUiScale()
    tt.style.left = (left / scale) + 'px'
    tt.style.top = (top / scale) + 'px'
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
    mounted(el, binding) {
        installGlobalListeners()
        const arg = binding.arg
        if (arg === 'top' || arg === 'bottom') placementMap.set(el, arg)
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
        placementMap.delete(el)
        hide(el)
    },
}
