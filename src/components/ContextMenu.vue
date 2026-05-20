<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick } from 'vue'
import { isPhoneViewport } from '../composables/usePlatform'

let scrollLockCount = 0

const props = defineProps<{
    x: number
    y: number
    alignRight?: boolean
    anchorBottom?: boolean
    centerX?: boolean
    centerY?: boolean
    mobileSheet?: 'full' | 'content'
}>()

const emit = defineEmits<{
    close: []
}>()

const menuEl = ref<HTMLElement | null>(null)
const positioned = ref(false)
const compactMobileSheet = ref(false)
let dragStartY = 0
let dragging = false

function adjustPosition() {
    const el = menuEl.value
    if (!el) return

    if (isPhoneViewport()) {
        compactMobileSheet.value = false
        el.style.left = '8px'
        el.style.right = '8px'
        el.style.top = 'auto'
        el.style.bottom = 'calc(var(--safe-bottom) + 10px)'
        if (props.mobileSheet === 'content') {
            el.style.left = '0'
            el.style.right = '0'
            el.style.top = 'auto'
            el.style.bottom = '0'
        }
        if (props.mobileSheet === 'full') {
            el.style.left = '0'
            el.style.right = '0'
            el.style.top = 'calc(var(--safe-top) + 12px)'
            el.style.bottom = '0'
        }
        positioned.value = true
        requestAnimationFrame(() => {
            if (!menuEl.value || props.mobileSheet !== 'content' || !isPhoneViewport()) return
            const viewportHeight = window.visualViewport?.height ?? window.innerHeight
            compactMobileSheet.value = menuEl.value.getBoundingClientRect().height < viewportHeight * 0.5
        })
        return
    }

    compactMobileSheet.value = false
    const scale = (window as any).__uiScale || 1
    const rect = el.getBoundingClientRect()
    const pad = 8

    let left = props.alignRight ? props.x - rect.width
               : props.centerX ? props.x - rect.width / 2
               : props.x
    let top = props.anchorBottom ? props.y - rect.height
              : props.centerY ? props.y - rect.height / 2
              : props.y

    if (left + rect.width > window.innerWidth - pad) {
        left = window.innerWidth - rect.width - pad
    }
    if (left < pad) left = pad

    if (top + rect.height > window.innerHeight - pad) {
        top = window.innerHeight - rect.height - pad
    }
    if (top < pad) top = pad

    el.style.left = (left / scale) + 'px'
    el.style.top = (top / scale) + 'px'
    positioned.value = true
}

function onClickOutside(e: MouseEvent) {
    const target = e.target as Node
    if (menuEl.value && !menuEl.value.contains(target)) {
        const submenu = (target as Element).closest?.('.ctx-submenu')
        if (!submenu) emit('close')
    }
}

function blockScroll(e: WheelEvent) {
    const target = e.target as Element
    if (target.closest?.('.ctx-submenu')) return
    e.preventDefault()
}

function lockScroll() {
    if (scrollLockCount === 0) {
        document.addEventListener('wheel', blockScroll, { passive: false })
    }
    scrollLockCount++
}

function unlockScroll() {
    scrollLockCount--
    if (scrollLockCount <= 0) {
        scrollLockCount = 0
        document.removeEventListener('wheel', blockScroll)
    }
}

function onWindowResize() {
    adjustPosition()
}

function onDragStart(e: PointerEvent) {
    if (!props.mobileSheet || !isPhoneViewport()) return
    e.preventDefault()
    dragging = true
    dragStartY = e.clientY
    if (menuEl.value) menuEl.value.style.transition = 'none'
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
    window.addEventListener('pointermove', onDragMove)
    window.addEventListener('pointerup', onDragEnd)
    window.addEventListener('pointercancel', onDragEnd)
}

function onDragMove(e: PointerEvent) {
    if (!dragging || !menuEl.value) return
    e.preventDefault()
    const delta = Math.max(0, e.clientY - dragStartY)
    menuEl.value.style.transform = `translateY(${delta}px)`
}

function onDragEnd(e: PointerEvent) {
    if (!dragging) return
    dragging = false
    window.removeEventListener('pointermove', onDragMove)
    window.removeEventListener('pointerup', onDragEnd)
    window.removeEventListener('pointercancel', onDragEnd)
    const delta = Math.max(0, e.clientY - dragStartY)
    if (delta > 76) {
        emit('close')
        return
    }
    if (menuEl.value) {
        menuEl.value.style.transition = ''
        menuEl.value.style.transform = ''
    }
}

onMounted(async () => {
    lockScroll()
    await nextTick()
    adjustPosition()
    window.addEventListener('resize', onWindowResize)
    setTimeout(() => {
        document.addEventListener('mousedown', onClickOutside)
    }, 0)
})

onUnmounted(() => {
    unlockScroll()
    window.removeEventListener('resize', onWindowResize)
    document.removeEventListener('mousedown', onClickOutside)
    window.removeEventListener('pointermove', onDragMove)
    window.removeEventListener('pointerup', onDragEnd)
    window.removeEventListener('pointercancel', onDragEnd)
})
</script>

<template>
    <Teleport to="body">
        <div
            v-if="mobileSheet"
            class="ctx-mobile-backdrop"
            :class="{ visible: positioned }"
            @mousedown="emit('close')"
        ></div>
        <div
            ref="menuEl"
            class="ctx-menu"
            :class="{
                visible: positioned,
                'mobile-full-sheet': mobileSheet === 'full',
                'mobile-content-sheet': mobileSheet === 'content',
                'compact-mobile-sheet': compactMobileSheet,
            }"
            @mousedown.stop
            @pointerdown.self="onDragStart"
        >
            <div
                v-if="mobileSheet"
                class="ctx-drag-handle"
                @pointerdown="onDragStart"
            ></div>
            <slot />
            <div
                v-if="mobileSheet === 'content' && compactMobileSheet"
                class="ctx-mobile-spacer"
                @pointerdown="onDragStart"
            ></div>
        </div>
    </Teleport>
</template>

<style scoped>
.ctx-menu {
    position: fixed;
    z-index: 1000000;
    min-width: 200px;
    border-radius: 12px;
    background-color: hsla(var(--accent-h), var(--accent-s), var(--accent-l), 0.15);
    backdrop-filter: blur(17px);
    box-shadow: 0 0px 16px rgba(0, 0, 0, 0.05);
    opacity: 0;
    left: -9999px;
    top: -9999px;
    overflow: hidden;
}
.ctx-drag-handle {
    display: none;
}
.ctx-mobile-backdrop {
    display: none;
}
.ctx-mobile-spacer {
    display: none;
}
.ctx-menu.visible {
    opacity: 1;
    animation: ctx-appear 0.15s ease;
}

@keyframes ctx-appear {
    from {
        opacity: 0;
        transform: scale(0.95) translateY(-4px);
    }
    to {
        opacity: 1;
        transform: scale(1) translateY(0);
    }
}

@media (max-width: 768px), (pointer: coarse) and (max-width: 900px), (hover: none) and (max-width: 900px) {
    .ctx-mobile-backdrop {
        position: fixed;
        inset: 0;
        z-index: 999999;
        display: block;
        background: transparent;
        opacity: 0;
        pointer-events: auto;
        transition: opacity .18s ease;
    }

    .ctx-mobile-backdrop.visible {
        opacity: 1;
    }

    :global(body.lfmusic-phone .ctx-menu.mobile-full-sheet) {
        left: 0 !important;
        right: 0 !important;
        top: calc(var(--safe-top) + 12px) !important;
        bottom: 0 !important;
        width: 100vw !important;
        min-width: 0 !important;
        max-height: none !important;
        height: calc(100dvh - var(--safe-top) - 12px) !important;
        border-radius: 28px 28px 0 0 !important;
        background-color: hsl(var(--accent-h), min(var(--accent-s), 48%), 11%) !important;
        box-shadow: 0 -22px 60px rgba(0, 0, 0, 0.46) !important;
        overflow-y: auto !important;
        overscroll-behavior: contain;
        display: flex;
        flex-direction: column;
        touch-action: pan-y;
        transition: transform .2s ease;
        will-change: transform;
    }

    :global(body.lfmusic-phone .ctx-menu.mobile-content-sheet) {
        left: 0 !important;
        right: 0 !important;
        top: auto !important;
        bottom: 0 !important;
        width: 100vw !important;
        min-width: 0 !important;
        height: auto !important;
        max-height: min(72dvh, 520px) !important;
        padding-bottom: var(--safe-bottom) !important;
        border-radius: 24px 24px 0 0 !important;
        background-color: hsl(var(--accent-h), min(var(--accent-s), 48%), 11%) !important;
        box-shadow: 0 -18px 48px rgba(0, 0, 0, 0.44) !important;
        overflow-y: auto !important;
        overscroll-behavior: contain;
        display: flex;
        flex-direction: column;
        touch-action: pan-y;
        transition: transform .2s ease;
        will-change: transform;
    }

    .ctx-mobile-spacer {
        display: block;
        flex: 0 0 48px;
        touch-action: none;
    }

    .ctx-drag-handle {
        display: flex;
        flex: 0 0 auto;
        height: 34px;
        align-items: center;
        justify-content: center;
        cursor: grab;
        touch-action: none;
    }

    .ctx-drag-handle::before {
        content: "";
        width: 42px;
        height: 4px;
        border-radius: 999px;
        background-color: hsla(0, 0%, 100%, 0.28);
    }

    .ctx-menu.visible {
        animation: ctx-mobile-appear 0.18s ease;
    }

    @keyframes ctx-mobile-appear {
        from {
            opacity: 0;
            transform: translateY(12px);
        }
        to {
            opacity: 1;
            transform: translateY(0);
        }
    }
}
</style>
