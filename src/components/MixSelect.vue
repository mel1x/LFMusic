<script setup lang="ts" generic="T extends string">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'

const props = defineProps<{ label: string, value: T, options: { value: T, label: string }[], disabled?: boolean, compact?: boolean }>()
const emit = defineEmits<{ change: [value: T] }>()
const root = ref<HTMLElement>()
const trigger = ref<HTMLButtonElement>()
const list = ref<HTMLElement>()
const open = ref(false)
const selected = computed(() => props.options.find(option => option.value === props.value))
function focusOption(index: number) {
    const buttons = list.value?.querySelectorAll<HTMLButtonElement>('button')
    if (buttons?.length) buttons[(index + buttons.length) % buttons.length]?.focus()
}
async function show() {
    open.value = true
    await nextTick()
    focusOption(Math.max(0, props.options.findIndex(option => option.value === props.value)))
}
function close() {
    open.value = false
    trigger.value?.focus()
}
function pick(value: T) {
    close()
    emit('change', value)
}
function outside(event: PointerEvent) {
    if (!root.value?.contains(event.target as Node)) open.value = false
}
onMounted(() => document.addEventListener('pointerdown', outside))
onBeforeUnmount(() => document.removeEventListener('pointerdown', outside))
</script>
<template>
    <div ref="root" class="picker" :class="{ compact }" @focusout="!root?.contains($event.relatedTarget as Node) && (open = false)">
        <button ref="trigger" class="trigger" type="button" aria-haspopup="listbox" :aria-expanded="open" :disabled="disabled"
            :aria-label="label + ': ' + selected?.label" @click="open ? close() : show()" @keydown.down.prevent="show" @keydown.up.prevent="show">
            <span v-if="!compact" class="caption">{{ label }}</span>
            <span class="value">{{ selected?.label }}</span>
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" :class="{ up: open }"><path d="m2 4 4 4 4-4" stroke="currentColor" stroke-width="1.5" /></svg>
        </button>
        <Transition name="mix-menu">
        <div v-if="open" ref="list" class="menu" role="listbox" :aria-label="label" @keydown.esc.stop.prevent="close">
            <button v-for="(option, i) in options" :key="option.value" tabindex="-1" type="button" role="option" :aria-selected="option.value === value"
                @keydown.down.prevent="focusOption(i + 1)" @keydown.up.prevent="focusOption(i - 1)" @keydown.home.prevent="focusOption(0)"
                @keydown.end.prevent="focusOption(options.length - 1)" @click="pick(option.value)">{{ option.label }}</button>
        </div>
        </Transition>
    </div>
</template>
<style scoped>
.picker { position:relative; min-width:0; flex:1; }
button { font:inherit; border:0; color:inherit; cursor:pointer; }
.trigger { display:grid; grid-template-columns:minmax(0,1fr) auto; align-items:center; gap:4px 8px; width:100%; padding:9px 12px; border-radius:12px; background:var(--dark-bg); text-align:left; color:white; }
.trigger:hover,.trigger[aria-expanded=true] { background:hsl(var(--accent-h),min(var(--accent-s),45%),19%); }
.trigger:disabled { opacity:.45; cursor:default; }
.caption { grid-column:1/-1; font-size:11px; color:var(--picker-color,#fffa); }
.value { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:13px; }
svg { opacity:.7; } .up { transform:rotate(180deg); }
.compact { flex:none; }
.compact .trigger { border-radius:16px; padding:7px 12px; background:hsl(var(--accent-h),min(var(--accent-s),35%),22%); }
.menu { position:absolute; bottom:calc(100% + 5px); left:0; z-index:5; width:max-content; min-width:100%; max-width:calc(100vw - 32px); max-height:35dvh; overflow:auto; box-sizing:border-box; padding:5px; border-radius:16px; background:hsl(var(--accent-h),min(var(--accent-s),40%),15%); box-shadow:0 8px 24px #0006; }
.picker:not(.compact):last-child .menu { left:auto; right:0; }
.compact .menu { left:50%; right:auto; transform:translateX(-50%); }
.menu button { display:block; width:100%; text-align:left; padding:8px 11px; border-radius:11px; background:transparent; font-size:13px; }
.menu button:is(:hover,:focus-visible,[aria-selected=true]) { background:#ffffff10; }
.menu button[aria-selected=true] { font-weight:700; color:var(--picker-color,white); }
.menu { transform-origin:bottom center; }
.mix-menu-enter-active,.mix-menu-leave-active { transition:opacity .14s ease,scale .14s ease,translate .14s ease; }
.mix-menu-enter-from,.mix-menu-leave-to { opacity:0; scale:.97; translate:0 4px; }
.mix-menu-leave-active { pointer-events:none; }
@media(prefers-reduced-motion:reduce) { .mix-menu-enter-active,.mix-menu-leave-active { transition:none; } }
</style>
