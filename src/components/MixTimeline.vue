<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from '../composables/useI18n'
import type { MixTransition, MixEffect, MixPoint } from '../composables/usePlaylistMix'

const props = defineProps<{
    overviewOnly?: boolean, peaks?: Uint8Array, duration: number, cue: number, incoming: boolean,
    overlap: number, context: number, draft: MixTransition, playing: boolean, selected?: MixEffect
}>()
const emit = defineEmits<{ cue: [value: number], select: [kind: MixEffect], edit: [kind: MixEffect, points: MixPoint[]] }>()
const { t } = useI18n()
const drag = ref<{ x: number, cue: number, overview: boolean, rect: DOMRect }>()
const span = computed(() => props.overlap + 2 * props.context)
const start = computed(() => props.cue - props.context)
const cueLabel = computed(() => t(props.incoming ? 'playlistMix.entry' : 'playlistMix.exit'))
const clamp = (x: number) => Math.max(0, Math.min(1, x))

function wave(startTime: number, length: number, band: number, overview = false) {
    const peaks = props.peaks
    if (!peaks?.length || !props.duration) return ''
    const bins = Math.floor(peaks.length / 3)
    const upper: string[] = [], lower: string[] = []
    const points = overview ? 600 : 800
    for (let x = 0; x <= points; x++) {
        const time = startTime + x * length / points
        const bin = Math.floor(time / props.duration * bins)
        const end = Math.min(bins, Math.max(bin + 1, Math.ceil((time + length / points) / props.duration * bins)))
        let peak = 0
        for (let i = Math.max(0, bin); i < end; i++) {
            const high = peaks[i * 3 + 2] * 0.42
            const mid = band <= 1 ? peaks[i * 3 + 1] * 0.65 : 0
            const low = band === 0 ? peaks[i * 3] * 0.8 : 0
            peak = Math.max(peak, Math.min(255, Math.hypot(high, mid, low)))
        }
        const height = (peak / 255) ** (band === 2 ? 1.9 : 1.15) * (overview ? 94 : 40)
        const px = (1000 * x / points).toFixed(1)
        upper.push(px + ',' + ((overview ? 100 : 50) - height).toFixed(1))
        lower.push(px + ',' + (overview ? 100 : 50 + height).toFixed(1))
    }
    return 'M' + upper.join(' L') + ' L' + lower.reverse().join(' L') + 'Z'
}
const detail = computed(() => [0, 1, 2].map(b => wave(start.value, span.value, b)))
const overview = computed(() => [0, 1, 2].map(b => wave(0, props.duration, b, true)))
const left = computed(() => props.context / span.value * 100)
const right = computed(() => 100 - left.value)
const detailEl = ref<HTMLElement>()
const draggingPoint = ref<number | null>(null)
const effects = computed<MixEffect[]>(() => {
    if (!props.overlap) return []
    const kinds: MixEffect[] = ['volume']
    if (props.draft.eqBand !== 'off') kinds.push('eq')
    if (props.draft.filter !== 'off') kinds.push('filter')
    return kinds
})
const active = computed(() => props.selected && effects.value.includes(props.selected) ? props.selected : 'volume')
const lane = computed(() => props.draft.curves?.[props.incoming ? 1 : 0])
function baseLevel(kind: MixEffect, fraction: number) {
    const w = props.incoming ? fraction : 1 - fraction
    let level = w
    if (kind === 'volume') {
        switch (props.draft.volumeCurve ?? (props.draft.equalPower ? 'smooth' : 'linear')) {
            case 'smooth': level = Math.sin(w * Math.PI / 2); break
            case 'rise': level = w * w; break
            case 'overlap': level = Math.min(1, w * 8); break
            case 'cut': level = Number(w >= 0.5); break
        }
    } else if (kind === 'eq' && props.draft.eqBand === 'centerBass') level = clamp(w * 10 - 4.5)
    else if (kind === 'filter') level = props.draft.filter === 'highPass' ? 1 - w : w
    return level
}
const ranges = { volume: [92, 84], eq: [86, 40], filter: [65, 48] }
function xPercent(x: number) { return (props.context + x * props.overlap) / span.value * 100 }
function yPercent(kind: MixEffect, y: number) { return ranges[kind][0] - y * ranges[kind][1] }
const points = computed<MixPoint[]>(() => {
    const kind = active.value
    if (lane.value?.[kind]) return lane.value[kind]!
    let times = [0, 0.5, 1]
    if (kind === 'eq' && props.draft.eqBand === 'centerBass') times = [0.45, 0.5, 0.55]
    if (kind === 'volume' && props.draft.volumeCurve === 'overlap') {
        times = props.incoming ? [0, 0.125, 1] : [0, 0.875, 1]
    }
    if (kind === 'volume' && props.draft.volumeCurve === 'cut') times = [0.49, 0.5, 0.51]
    return times.map(x => [x, baseLevel(kind, x)] as MixPoint)
})
const lines = computed(() => Object.fromEntries(effects.value.map(kind => {
    const edited = lane.value?.[kind]
    const vertices: MixPoint[] = edited
        ? [[0, edited[0][1]], ...edited, [1, edited[2][1]]]
        : Array.from({ length: 121 }, (_, i) => [i / 120, baseLevel(kind, i / 120)])
    const coords = vertices.map(([x, y]) => (10 * xPercent(x)).toFixed(3) + ',' + yPercent(kind, y).toFixed(3))
    return [kind, 'M' + coords.join(' L')]
})))
function editPoint(i: number, x: number, y: number) {
    const next = points.value.slice()
    const min = i ? next[i - 1][0] : 0
    const max = i < 2 ? next[i + 1][0] : 1
    next[i] = [Math.max(min, Math.min(max, x)), clamp(y)]
    emit('edit', active.value, next)
}
function pointMove(e: PointerEvent, i: number) {
    if (draggingPoint.value !== i || !detailEl.value) return
    const rect = detailEl.value.getBoundingClientRect()
    const x = ((e.clientX - rect.left) / rect.width * span.value - props.context) / props.overlap
    const y = (ranges[active.value][0] - (e.clientY - rect.top) / rect.height * 100) / ranges[active.value][1]
    const others = points.value.filter((_, index) => index !== i)
    const snapX = others.find(p => Math.abs(p[0] - x) * rect.width * props.overlap / span.value <= 4)
    const time = snapX?.[0] ?? x
    const heights = others.map(p => p[1])
    if (i === 1 && others[1][0] > others[0][0]) {
        const t = clamp((time - others[0][0]) / (others[1][0] - others[0][0]))
        heights.push(others[0][1] + t * (others[1][1] - others[0][1]))
    }
    const nearest = heights.reduce((a, b) => Math.abs(a - y) < Math.abs(b - y) ? a : b)
    const snapY = Math.abs(nearest - y) * rect.height * ranges[active.value][1] / 100 <= 4
    editPoint(i, time, snapY ? nearest : y)
}
function straighten() {
    const [a, m, b] = points.value
    const fraction = b[0] > a[0] ? (m[0] - a[0]) / (b[0] - a[0]) : 0.5
    editPoint(1, m[0], a[1] + fraction * (b[1] - a[1]))
}
function move(e: PointerEvent) {
    if (!drag.value) return
    const rect = drag.value.rect
    const delta = (e.clientX - drag.value.x) / rect.width
    emit('cue', drag.value.overview ? (e.clientX - rect.left) / rect.width * props.duration : drag.value.cue - delta * span.value)
}
function down(e: PointerEvent, overview: boolean) {
    if (e.button !== 0) return
    drag.value = { x: e.clientX, cue: props.cue, overview, rect: (e.currentTarget as HTMLElement).getBoundingClientRect() }
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
    if (overview) move(e)
}
</script>

<template>
    <div class="lane" :class="{ incoming }">
        <div v-if="overviewOnly" class="overview" @pointerdown="down($event, true)" @pointermove="move" @lostpointercapture="drag = undefined" :title="cueLabel">
            <svg viewBox="0 0 1000 100" preserveAspectRatio="none"><path v-for="(shape, band) in overview" :key="band" :class="'band' + band" :d="shape" /></svg>
        </div>
        <div v-else ref="detailEl" class="detail" role="group" tabindex="0" :aria-label="cueLabel"
            @keydown.left.prevent="emit('cue', cue - ($event.shiftKey ? 1 : 0.1))" @keydown.right.prevent="emit('cue', cue + ($event.shiftKey ? 1 : 0.1))"
            @pointerdown="down($event, false)" @pointermove="move" @lostpointercapture="drag = undefined">
            <svg viewBox="0 0 1000 100" preserveAspectRatio="none">
                <path v-for="(shape, band) in detail" :key="band" class="wave" :class="'band' + band" :d="shape" />
                <path v-for="kind in effects" :key="kind" class="line" :class="kind" :d="lines[kind]" />
                <path v-for="kind in effects" :key="'hit' + kind" class="hit" :d="lines[kind]"
                    @pointerdown.stop="emit('select', kind)" />
            </svg>
            <template v-if="effects.length">
                <button v-for="(pt, i) in points" :key="i" class="point" :class="active"
                    :style="{ left: xPercent(pt[0]) + '%', top: yPercent(active, pt[1]) + '%' }"
                    :aria-label="cueLabel + ', ' + t('playlistMix.' + active) + ', ' + (i + 1)"
                    :title="i === 1 ? t('playlistMix.straighten') : undefined"
                    @dblclick.stop="i === 1 && straighten()" @keydown.enter.prevent="i === 1 && straighten()"
                    @pointerdown.stop="draggingPoint = i; ($event.currentTarget as HTMLElement).setPointerCapture($event.pointerId)"
                    @pointermove.stop="pointMove($event, i)" @lostpointercapture="draggingPoint = null"
                    @keydown.left.stop.prevent="editPoint(i, pt[0] - .01, pt[1])" @keydown.right.stop.prevent="editPoint(i, pt[0] + .01, pt[1])"
                    @keydown.up.stop.prevent="editPoint(i, pt[0], pt[1] + .02)" @keydown.down.stop.prevent="editPoint(i, pt[0], pt[1] - .02)" />
            </template>
            <i v-show="playing" class="cursor" />
            <span class="mark" :style="{ left: (incoming ? left : right) + '%' }" />
        </div>
    </div>
</template>

<style scoped>
.lane { --wave: #83aec4; }
.incoming { --wave: #c6a894; }
.overview, .detail { position: relative; overflow: hidden; touch-action: none; cursor: grab; }
.overview:active, .detail:active { cursor: grabbing; }
.overview { height: 24px; }
.overview svg { opacity:.9; }
.lane svg { width: 100%; height: 100%; position: absolute; inset: 0; fill: var(--wave); pointer-events: none; }
.detail { height: clamp(72px, 12vh, 124px); }
.band0 { fill:#283faa; } .band1 { fill:#a2642c; } .band2 { fill:#ffe2ad; }
.hit { stroke:transparent; stroke-width:11; fill:none; pointer-events:stroke; vector-effect:non-scaling-stroke; cursor:pointer; }
.point { position:absolute; z-index:1; width:20px; height:20px; padding:0; border:none; background:transparent; transform:translate(-50%,-50%); cursor:grab; touch-action:none; }
.point::after { content:''; display:block; margin:auto; width:8px; height:8px; border-radius:50%; background:currentColor; box-shadow:0 1px 4px #0009; }
.point.volume { color:var(--mix-volume); } .point.eq { color:var(--mix-eq); } .point.filter { color:var(--mix-filter); }
.line { fill: none; stroke-linejoin:miter; stroke-width: 2; vector-effect: non-scaling-stroke; }
.volume { stroke: var(--mix-volume); }
.eq { stroke: var(--mix-eq); }
.filter { stroke: var(--mix-filter); }
.cursor, .mark { position: absolute; inset-block: 0; width: 2px; background: white; pointer-events: none; }
.cursor { width:100%; left:0; background:none; border-left:2px solid white; }
.mark { width: 1px; background: #fff5; }
@media (max-width: 600px) { .overview { height: 18px; } .detail { height: clamp(80px, 14dvh, 145px); } }
</style>
