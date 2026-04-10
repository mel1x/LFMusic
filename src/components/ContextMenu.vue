<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick } from 'vue'

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
    if (menuEl.value && !menuEl.value.contains(e.target as Node)) {
        emit('close')
    }
}

onMounted(async () => {
    await nextTick()
    adjustPosition()
    setTimeout(() => {
        document.addEventListener('mousedown', onClickOutside)
    }, 0)
})

onUnmounted(() => {
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
    z-index: 9999;
    min-width: 200px;
    padding: 6px;
    border-radius: 12px;
    background-color: hsla(0, 0%, 12%, 0.95);
    backdrop-filter: blur(20px);
    border: 1px solid hsla(0, 0%, 100%, 0.08);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5), 0 2px 8px rgba(0, 0, 0, 0.3);
    opacity: 0;
    left: -9999px;
    top: -9999px;
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
