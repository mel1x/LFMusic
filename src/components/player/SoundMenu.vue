<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import SoundFxControls, { type SoundSection } from '../SoundFxControls.vue'
import { useSoundFx, formatSpeed } from '../../composables/useSoundFx'
import { useSleepTimer } from '../../composables/useSleepTimer'
import { useEqualizer } from '../../composables/useEqualizer'
import { currentUiScale } from '../../composables/useUiScale'

const props = defineProps<{ anchor: DOMRect }>()
const emit = defineEmits<{ (e: 'close'): void }>()

const { fx, load: loadFx } = useSoundFx()
const sleep = useSleepTimer()
const { eqEnabled, activePreset, load: loadEq } = useEqualizer()

const rootEl = ref<HTMLElement | null>(null)
const page = ref<SoundSection | null>(null)
const direction = ref<'forward' | 'back'>('forward')

const ICONS: Record<SoundSection, string> = {
    eq: '<path d="M2 10v3"/><path d="M6 6v11"/><path d="M10 3v18"/><path d="M14 8v7"/><path d="M18 5v13"/><path d="M22 10v3"/>',
    space: '<circle cx="12" cy="12" r="2"/><path d="M16.24 7.76a6 6 0 0 1 0 8.49"/><path d="M7.76 16.24a6 6 0 0 1 0-8.49"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/><path d="M4.93 19.07a10 10 0 0 1 0-14.14"/>',
    crusher: '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M3 9h18"/><path d="M3 15h18"/><path d="M9 3v18"/><path d="M15 3v18"/>',
    vinyl: '<circle cx="12" cy="12" r="10"/><path d="M6 12c0-1.7.7-3.2 1.8-4.2"/><circle cx="12" cy="12" r="2"/><path d="M18 12c0 1.7-.7 3.2-1.8 4.2"/>',
    speed: '<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>',
    sleep: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
}

const TITLES: Record<SoundSection, string> = {
    eq: 'Эквалайзер',
    space: 'Пространство',
    crusher: 'Биткрашер',
    vinyl: 'Винил',
    speed: 'Скорость',
    sleep: 'Таймер сна',
}

const rows = computed(() => {
    const f = fx.value
    return ([
        { id: 'eq', active: eqEnabled.value, value: eqEnabled.value ? (activePreset.value ?? '') : '' },
        { id: 'space', active: f.space.enabled, value: '' },
        { id: 'crusher', active: f.crusher.enabled, value: '' },
        { id: 'vinyl', active: f.vinyl.enabled, value: '' },
        { id: 'speed', active: f.speed !== 1, value: formatSpeed(f.speed) },
        {
            id: 'sleep',
            active: sleep.mode.value !== 'off',
            value: sleep.mode.value === 'time' ? sleep.remaining.value : sleep.mode.value === 'track' ? 'После трека' : '',
        },
    ] as { id: SoundSection; active: boolean; value: string }[])
})

function open(id: SoundSection) {
    direction.value = 'forward'
    page.value = id
}
function back() {
    direction.value = 'back'
    page.value = null
}

const style = computed(() => {
    const scale = currentUiScale() || 1
    const r = props.anchor
    return {
        right: `${Math.max(8, (window.innerWidth - r.right) / scale - 8)}px`,
        bottom: `${(window.innerHeight - r.top) / scale + 10}px`,
    }
})

function onDocMouseDown(e: MouseEvent) {
    if (!rootEl.value?.contains(e.target as Node)) emit('close')
}
function onKey(e: KeyboardEvent) {
    if (e.key !== 'Escape') return
    e.stopPropagation()
    if (page.value) back()
    else emit('close')
}

onMounted(() => {
    void loadFx()
    void loadEq()
    setTimeout(() => document.addEventListener('mousedown', onDocMouseDown), 0)
    document.addEventListener('keydown', onKey, true)
})
onUnmounted(() => {
    document.removeEventListener('mousedown', onDocMouseDown)
    document.removeEventListener('keydown', onKey, true)
})
</script>

<template>
    <Teleport to="body">
        <div ref="rootEl" class="soundMenu" :style="style" @mousedown.stop>
            <Transition :name="`sm-${direction}`" mode="out-in">
                <div v-if="!page" key="main" class="smPage">
                    <button v-for="row in rows" :key="row.id" class="smRow" @click="open(row.id)">
                        <svg class="smIcon" :class="{ active: row.active }" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" v-html="ICONS[row.id]"></svg>
                        <span class="smLabel">{{ TITLES[row.id] }}</span>
                        <span v-if="row.value" class="smValue" :class="{ active: row.active }">{{ row.value }}</span>
                        <span v-else-if="row.active" class="smDot"></span>
                        <svg class="smChevron" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6" /></svg>
                    </button>
                </div>
                <div v-else :key="page" class="smPage smSub">
                    <div class="smHeader">
                        <button class="smBack" aria-label="Назад" @click="back">
                            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6" /></svg>
                        </button>
                        <span class="smTitle">{{ TITLES[page] }}</span>
                    </div>
                    <SoundFxControls :section="page" compact />
                </div>
            </Transition>
        </div>
    </Teleport>
</template>

<style scoped>
.soundMenu {
    position: fixed;
    z-index: 100000;
    width: 340px;
    max-height: calc(100vh - 140px);
    overflow-y: auto;
    overflow-x: hidden;
    padding: 8px;
    border-radius: 22px;
    background-color: var(--light-bg);
    box-shadow: 0 18px 48px rgba(0, 0, 0, 0.4);
    transform-origin: bottom right;
    animation: sm-in .2s var(--ease-smooth);
    scrollbar-width: none;
}
.soundMenu::-webkit-scrollbar {
    display: none;
}
@keyframes sm-in {
    from {
        opacity: 0;
        transform: translateY(6px) scale(.97);
    }
}

.smPage {
    display: flex;
    flex-direction: column;
}
.smSub {
    padding: 2px 8px 10px;
}

.smRow {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    height: 46px;
    padding: 0 12px;
    border: none;
    border-radius: 14px;
    background: transparent;
    color: #fff;
    font-family: inherit;
    text-align: left;
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth);
}
.smRow:hover {
    background-color: hsla(0, 0%, 100%, 0.06);
}
.smIcon {
    flex-shrink: 0;
    color: hsla(0, 0%, 100%, 0.85);
    transition: color var(--duration-hover) var(--ease-smooth);
}
.smIcon.active {
    color: hsl(var(--accent-h), max(var(--accent-s), 55%), 68%);
}
.smLabel {
    flex: 1;
    min-width: 0;
    font-size: 15px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.smValue {
    flex-shrink: 0;
    max-width: 110px;
    font-size: 14px;
    color: hsla(0, 0%, 100%, 0.45);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}
.smValue.active {
    color: hsl(var(--accent-h), max(var(--accent-s), 55%), 72%);
}
.smDot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background-color: hsl(var(--accent-h), max(var(--accent-s), 55%), 62%);
}
.smChevron {
    flex-shrink: 0;
    color: hsla(0, 0%, 100%, 0.35);
}

.smHeader {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 44px;
    margin: 0 -6px 6px;
}
.smBack {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: hsla(0, 0%, 100%, 0.7);
    cursor: pointer;
    transition: background-color var(--duration-hover) var(--ease-smooth), color var(--duration-hover) var(--ease-smooth);
}
.smBack:hover {
    background-color: hsla(0, 0%, 100%, 0.08);
    color: #fff;
}
.smTitle {
    font-family: "Montserrat", "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 16px;
    font-weight: 800;
    color: #fff;
}

.sm-forward-enter-active,
.sm-back-enter-active {
    transition: opacity .22s var(--ease-smooth), transform .26s var(--ease-smooth);
}
.sm-forward-leave-active,
.sm-back-leave-active {
    transition: opacity .1s var(--ease-smooth);
}
.sm-forward-enter-from {
    opacity: 0;
    transform: translateX(18px);
}
.sm-back-enter-from {
    opacity: 0;
    transform: translateX(-18px);
}
.sm-forward-leave-to,
.sm-back-leave-to {
    opacity: 0;
}
</style>
