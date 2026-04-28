<script setup lang="ts">
import { computed, ref, nextTick, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { useSettings } from '../composables/useSettings'
import { splitArtistsWithSeparators } from '../utils/format'

const props = withDefaults(defineProps<{
    artist: string | null | undefined
    fallback?: string
}>(), {
    fallback: 'Неизвестный исполнитель',
})

const router = useRouter()
const { settings } = useSettings()

const parts = computed(() => splitArtistsWithSeparators(props.artist || '', settings.value.artist_separators))

const rootEl = ref<HTMLElement | null>(null)
const tooltipEl = ref<HTMLElement | null>(null)
const showTooltip = ref(false)
const tooltipLeft = ref(0)
const tooltipTop = ref(0)
let hideTimer: ReturnType<typeof setTimeout> | null = null
let scrollHandler: (() => void) | null = null

function findClippingAncestor(el: HTMLElement | null): HTMLElement | null {
    let cur: HTMLElement | null = el
    while (cur && cur !== document.body) {
        const cs = getComputedStyle(cur)
        if (cs.textOverflow === 'ellipsis' || cs.overflow === 'hidden' || cs.overflowX === 'hidden') {
            return cur
        }
        cur = cur.parentElement
    }
    return el
}

function isOverflowing(el: HTMLElement | null): boolean {
    if (!el) return false
    return el.scrollWidth - el.clientWidth > 1
}

function clearHide() {
    if (hideTimer) {
        clearTimeout(hideTimer)
        hideTimer = null
    }
}

function scheduleHide() {
    clearHide()
    hideTimer = setTimeout(() => {
        showTooltip.value = false
        teardownScrollHandler()
    }, 180)
}

function setupScrollHandler() {
    if (scrollHandler) return
    scrollHandler = () => {
        showTooltip.value = false
        teardownScrollHandler()
    }
    document.addEventListener('scroll', scrollHandler, true)
    window.addEventListener('resize', scrollHandler)
    window.addEventListener('blur', scrollHandler)
}

function teardownScrollHandler() {
    if (!scrollHandler) return
    document.removeEventListener('scroll', scrollHandler, true)
    window.removeEventListener('resize', scrollHandler)
    window.removeEventListener('blur', scrollHandler)
    scrollHandler = null
}

async function tryShowTooltip() {
    clearHide()
    if (parts.value.length === 0) return
    const root = rootEl.value
    if (!root) return
    const clipper = findClippingAncestor(root.parentElement)
    if (!isOverflowing(clipper)) return
    const rect = (clipper || root).getBoundingClientRect()
    showTooltip.value = true
    setupScrollHandler()
    await nextTick()
    const tt = tooltipEl.value
    if (!tt) return
    const ttW = tt.offsetWidth
    const ttH = tt.offsetHeight
    const margin = 8
    let left = rect.left + rect.width / 2 - ttW / 2
    let top = rect.top - ttH - margin
    if (left < margin) left = margin
    const maxLeft = window.innerWidth - ttW - margin
    if (left > maxLeft) left = Math.max(margin, maxLeft)
    if (top < margin) top = rect.bottom + margin
    tooltipLeft.value = left
    tooltipTop.value = top
}

function onTriggerLeave() { scheduleHide() }
function onTooltipEnter() { clearHide() }
function onTooltipLeave() { scheduleHide() }

function openArtist(name: string, e: MouseEvent) {
    e.stopPropagation()
    showTooltip.value = false
    teardownScrollHandler()
    clearHide()
    router.push(`/artist/${encodeURIComponent(name)}`)
}

onUnmounted(() => {
    clearHide()
    teardownScrollHandler()
    showTooltip.value = false
})
</script>

<template>
    <span ref="rootEl" class="ta-root" @mouseenter="tryShowTooltip" @mouseleave="onTriggerLeave">
        <template v-if="parts.length > 0">
            <template v-for="(p, i) in parts" :key="i">
                <span
                    v-if="p.type === 'artist'"
                    class="ta-link"
                    @click="openArtist(p.text, $event)"
                    @mousedown.stop
                >{{ p.text }}</span><span
                    v-else
                    class="ta-sep"
                >, </span>
            </template>
        </template>
        <template v-else>{{ fallback }}</template>
    </span>
    <Teleport to="body">
        <Transition name="ta-tt">
            <div
                v-if="showTooltip"
                ref="tooltipEl"
                class="ta-tooltip"
                :style="{ left: tooltipLeft + 'px', top: tooltipTop + 'px' }"
                @mouseenter="onTooltipEnter"
                @mouseleave="onTooltipLeave"
                @click.stop
                @mousedown.stop
            >
                <template v-for="(p, i) in parts" :key="i">
                    <span
                        v-if="p.type === 'artist'"
                        class="ta-tt-link"
                        @click="openArtist(p.text, $event)"
                    >{{ p.text }}</span><span
                        v-else
                        class="ta-tt-sep"
                    >, </span>
                </template>
            </div>
        </Transition>
    </Teleport>
</template>

<style scoped>
.ta-root {
    color: inherit;
}
.ta-link {
    cursor: pointer;
    color: inherit;
    transition: color .2s ease;
}
.ta-link:hover {
    color: hsla(0, 0%, 100%, 1);
}
.ta-sep {
    cursor: default;
    color: inherit;
}

.ta-tooltip {
    position: fixed;
    left: 0;
    top: 0;
    z-index: 1000001;
    background: var(--dark-bg, #1a1a1a);
    border: 1px solid hsla(0, 0%, 100%, 0.1);
    border-radius: 8px;
    padding: 6px 10px;
    font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 12px;
    font-weight: 500;
    line-height: 1.45;
    color: hsla(0, 0%, 100%, 0.85);
    max-width: min(480px, calc(100vw - 24px));
    white-space: normal;
    word-break: break-word;
    backdrop-filter: blur(12px);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
    user-select: none;
}
.ta-tt-link {
    cursor: pointer;
    color: hsla(0, 0%, 100%, 0.85);
    transition: color .18s ease;
}
.ta-tt-link:hover {
    color: #fff;
}
.ta-tt-sep {
    cursor: default;
    color: hsla(0, 0%, 100%, 0.4);
}

.ta-tt-enter-active,
.ta-tt-leave-active {
    transition: opacity .18s ease, transform .18s ease;
}
.ta-tt-enter-from,
.ta-tt-leave-to {
    opacity: 0;
    transform: translateY(2px);
}
</style>
