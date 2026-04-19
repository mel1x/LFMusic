<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick } from 'vue'

let scrollLockCount = 0

const props = defineProps<{
    x: number
    y: number
    alignRight?: boolean
    anchorBottom?: boolean
    centerX?: boolean
    centerY?: boolean
}>()

const emit = defineEmits<{
    close: []
}>()

const menuEl = ref<HTMLElement | null>(null)
const positioned = ref(false)

function adjustPosition() {
    const el = menuEl.value
    if (!el) return
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

    el.style.left = left + 'px'
    el.style.top = top + 'px'
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

onMounted(async () => {
    lockScroll()
    await nextTick()
    adjustPosition()
    setTimeout(() => {
        document.addEventListener('mousedown', onClickOutside)
    }, 0)
})

onUnmounted(() => {
    unlockScroll()
    document.removeEventListener('mousedown', onClickOutside)
})
</script>

<template>
    <Teleport to="body">
        <div
            ref="menuEl"
            class="ctx-menu"
            :class="{ visible: positioned }"
            @mousedown.stop
        >
            <slot />
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
</style>
