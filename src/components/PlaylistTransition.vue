<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useI18n } from '../composables/useI18n'
import { applyMixPreset, mixBarSeconds, mixOverlap, mixKey, mixPresets, resolveMixTransition, saveTransition, type MixPreset, type MixEffect, type MixPoint, type LaneCurves } from '../composables/usePlaylistMix'
import { usePlayer, type TrackMetadata } from '../composables/usePlayer'
import defaultCover from '../assets/trackCover.webp'
import MixSelect from './MixSelect.vue'
import MixTimeline from './MixTimeline.vue'
import LoadingSpinner from './LoadingSpinner.vue'
import PlayIcon from './Icons/PlayIcon.vue'
import CloseIcon from './Icons/CloseIcon.vue'

const props = defineProps<{ source: string, from: TrackMetadata, to: TrackMetadata }>()
const emit = defineEmits<{ close: [] }>()
const { t } = useI18n()
const { currentTrack, playbackState, togglePlayPause, formatTime, volume } = usePlayer()
const dialog = ref<HTMLDialogElement>()
const visible = ref(true)
const expanded = ref(false)
const selectedEffect = ref<MixEffect>('volume')
const initial = resolveMixTransition(props.source, props.from, props.to.file_path, props.to)
const draft = ref({ ...initial, volumeCurve: initial.volumeCurve ?? (initial.equalPower ? 'smooth' : 'linear') })
const preset = computed(() => draft.value.preset ?? 'custom')
const peaks = shallowRef<(Uint8Array | undefined)[]>([])
const waveFailed = ref(false)
const pendingWaves = ref(2)
const previewFailed = ref(false)
const busy = ref(false)
const playing = ref(false)
let ctx: AudioContext | undefined
let node: AudioBufferSourceNode | undefined
let gain: GainNode | undefined
let resumePath: string | undefined
let generation = 0
let frame = 0
let closed = false
const tracks = [props.from, props.to]
const end = computed(() => Math.max(0, (props.from.duration_secs ?? 0) - draft.value.skipOutro))
const overlap = computed(() => mixOverlap(draft.value.seconds, end.value, (props.to.duration_secs ?? 0) - draft.value.skipIntro))
const cues = computed(() => [end.value - overlap.value, draft.value.skipIntro])
const overview = computed(() => {
    const durations = tracks.map(track => track.duration_secs || 1)
    const offset = cues.value[0] - cues.value[1]
    const start = Math.min(0, offset)
    const total = Math.max(durations[0], offset + durations[1]) - start
    const percent = (time: number) => 100 * time / total + '%'
    return {
        lanes: [0, offset].map((t, i) => ({ marginLeft: percent(t - start), width: percent(durations[i]) })),
        window: { left: percent(cues.value[0] - start), width: percent(overlap.value) },
    }
})
const context = computed(() => Math.min(3, Math.max(1, overlap.value / 4)))
const bars = computed(() => mixBarSeconds(props.from, props.to))
const lengthOptions = computed(() => {
    const options = [1, 2, 4, 8, 16].map(n => ({ value: String(n * bars.value), label: n + ' ' + t('playlistMix.bars') }))
        .filter(n => Number(n.value) <= 32)
    if (!options.some(n => Number(n.value) === draft.value.seconds)) {
        options.unshift({ value: String(draft.value.seconds), label: Number((draft.value.seconds / bars.value).toFixed(2)) + ' ' + t('playlistMix.bars') })
    }
    return options
})
function setPoints(i: number, effect: MixEffect, points: MixPoint[]) {
    const lanes: [LaneCurves, LaneCurves] = [...(draft.value.curves ?? [{}, {}])]
    lanes[i] = { ...lanes[i], [effect]: points }
    draft.value.curves = lanes
    draft.value.preset = 'custom'
}
function changeEffect(effect: MixEffect) {
    selectedEffect.value = effect
    if (draft.value.curves) {
        const lanes: [LaneCurves, LaneCurves] = [{ ...draft.value.curves[0] }, { ...draft.value.curves[1] }]
        for (const lane of lanes) delete lane[effect]
        draft.value.curves = lanes
    }
    draft.value.preset = 'custom'
}
function options<T extends string>(values: readonly T[]) {
    return values.map(value => ({ value, label: t('playlistMix.' + value) }))
}
const gridLines = computed(() => {
    if (!bars.value || !overlap.value) return []
    const count = Math.ceil(overlap.value / bars.value)
    return Array.from({ length: Math.min(64, count - 1) }, (_, i) => (i + 1) * bars.value / overlap.value * 100)
})
const dirty = computed(() => JSON.stringify(draft.value) !== JSON.stringify({ ...initial, volumeCurve: initial.volumeCurve ?? (initial.equalPower ? 'smooth' : 'linear') }))
onMounted(() => {
    dialog.value?.showModal()
    tracks.forEach(async (track, i) => {
        try {
            const result = await invoke<ArrayBuffer>('playlist_mix_waveform', { filePath: track.file_path, duration: track.duration_secs ?? 0 })
            const next = [...peaks.value]
            next[i] = new Uint8Array(result)
            peaks.value = next
        } catch { waveFailed.value = true }
        finally { pendingWaves.value-- }
    })
})
function restorePlayer() {
    const path = resumePath
    resumePath = undefined
    if (path && currentTrack.value?.file_path === path && playbackState.value === 'Paused') void togglePlayPause()
}
function stop() {
    generation++
    cancelAnimationFrame(frame)
    if (node) { node.onended = null; node.stop(); node.disconnect(); node = undefined }
    gain?.disconnect()
    gain = undefined
    playing.value = false
    busy.value = false
    restorePlayer()
}
watch(draft, stop, { deep: true, flush: 'sync' })
onBeforeUnmount(() => { closed = true; stop(); void ctx?.close() })
async function preview() {
    if (busy.value || node) { stop(); return }
    const token = ++generation
    previewFailed.value = false
    busy.value = true
    try {
        ctx ??= new AudioContext()
        await ctx.resume()
        const lead = Math.min(context.value, cues.value[0])
        const tail = Math.min(context.value, Math.max(0, (props.to.duration_secs ?? 0) - cues.value[1] - overlap.value))
        const raw = await invoke<ArrayBuffer>('playlist_mix_preview', { region: {
            fromPath: props.from.file_path, toPath: props.to.file_path,
            fromStart: cues.value[0] - lead, toStart: cues.value[1],
            lead, overlap: overlap.value, tail, profile: { ...draft.value }
        } })
        if (closed || token !== generation) return
        const pcm = new Int16Array(raw)
        const audio = ctx.createBuffer(2, pcm.length / 2, 32000)
        for (let c = 0; c < 2; c++) {
            const samples = audio.getChannelData(c)
            for (let j = 0; j < samples.length; j++) samples[j] = pcm[j * 2 + c] / 32768
        }
        if (playbackState.value === 'Playing') {
            const path = currentTrack.value?.file_path
            await togglePlayPause()
            if (closed || token !== generation) {
                if (path && currentTrack.value?.file_path === path && playbackState.value as string === 'Paused') await togglePlayPause()
                return
            }
            resumePath = path
        }
        node = ctx.createBufferSource()
        node.buffer = audio
        gain = ctx.createGain()
        gain.gain.value = volume.value
        node.connect(gain).connect(ctx.destination)
        node.onended = stop
        const start = ctx.currentTime
        node.start()
        busy.value = false
        playing.value = true
        const heads = dialog.value?.querySelectorAll<HTMLElement>('.cursor')
        const span = context.value * 2 + overlap.value
        let last = -100
        function tick(now: number) {
            if (now - last >= 32) {
                const x = Math.max(0, Math.min(1, (context.value + ctx!.currentTime - start - lead) / span))
                heads?.forEach(el => { el.style.transform = `translateX(${100 * x}%)` })
                last = now
            }
            frame = requestAnimationFrame(tick)
        }
        tick(performance.now())
    } catch {
        if (!closed && token === generation) { stop(); previewFailed.value = true }
    }
}
function choose(p: MixPreset) {
    selectedEffect.value = 'volume'
    const next = applyMixPreset(p, props.from, draft.value, props.to)
    draft.value = { ...next, volumeCurve: next.volumeCurve ?? 'smooth' }
}
function setCue(i: number, value: number) {
    if (preset.value === 'none') return
    const duration = tracks[i].duration_secs ?? 0
    const bounded = Math.max(0, Math.min(duration - overlap.value, value))
    if (i === 1) draft.value.skipIntro = bounded
    else draft.value.skipOutro = Math.max(0, duration - bounded - overlap.value)
    draft.value.preset = 'custom'
}
function changeLength(value: string) {
    const seconds = Number(value)
    if (!Number.isFinite(seconds) || seconds <= 0) return
    draft.value.seconds = Math.min(32, Math.max(0.1, seconds))
    draft.value.preset = 'custom'
}
function requestClose() {
    if (!visible.value) return
    visible.value = false
    stop()
}
function finishClose() {
    dialog.value?.close()
    emit('close')
}
function save() {
    saveTransition(props.source, props.from.file_path, props.to.file_path, preset.value === 'auto' ? null : draft.value)
    requestClose()
}
</script>

<template>
    <Transition name="mix-modal" appear @after-leave="finishClose">
    <dialog v-show="visible" ref="dialog" :class="{ expanded }" aria-labelledby="mix-heading" @cancel.prevent="requestClose" @click.self="requestClose">
        <article :inert="!visible">
            <header><h2 id="mix-heading">{{ t('playlistMix.edit') }}</h2>
                <button class="save" :disabled="!dirty" @click="save">{{ t('playlistMix.save') }}</button>
                <button class="close expand" :aria-label="t(expanded ? 'playlistMix.collapse' : 'playlistMix.expand')" :aria-pressed="expanded" @click="expanded = !expanded">
                    <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 6V2h4m4 0h4v4m0 4v4h-4m-4 0H2v-4" /></svg>
                </button>
                <button class="close" :aria-label="t('playlistMix.close')" @click="requestClose"><CloseIcon width="16" height="16" /></button>
            </header>
            <main :aria-busy="pendingWaves > 0">
                <LoadingSpinner v-if="pendingWaves" class="waveLoading" role="status" :aria-label="t('playlistMix.loading')" />
                <div :class="{ waiting: pendingWaves }" :inert="pendingWaves > 0">
                <div class="overviews">
                    <MixTimeline v-for="(track, i) in tracks" :key="i" overview-only :style="overview.lanes[i]" :peaks="peaks[i]" :duration="track.duration_secs || 1" :cue="cues[i]" :incoming="!!i"
                        :overlap="overlap" :context="context" :draft="draft" :playing="playing" @cue="setCue(i, $event)" />
                    <i class="overviewRange" :style="overview.window" />
                </div>
                <template v-for="(track, index) in tracks" :key="track.file_path">
                    <div class="track">
                        <img :src="track.cover_art || defaultCover" alt="" />
                        <div class="name"><strong>{{ track.title || track.file_path }}</strong><small>{{ track.artist }}</small></div>
                        <div class="data"><b v-if="track.bpm">{{ Math.round(track.bpm) }} BPM</b><small><em v-if="track.key" :title="track.key">{{ mixKey(track.key) }}</em>{{ formatTime(track.duration_secs ?? 0) }}</small></div>
                    </div>

                    <div v-if="index === 0" class="workstation">
                        <div class="workZone" :style="{ insetInline: context / (overlap + 2 * context) * 100 + '%' }">
                            <i v-for="line in gridLines" :key="line" :style="{ left: line + '%' }" />
                        </div>
                        <MixTimeline v-for="(lane, i) in tracks" :key="i" :peaks="peaks[i]" :selected="selectedEffect" @select="selectedEffect = $event" @edit="(effect, points) => setPoints(i, effect, points)" :duration="lane.duration_secs || 1" :cue="cues[i]" :incoming="!!i"
                        :overlap="overlap" :context="context" :draft="draft" :playing="playing" @cue="setCue(i, $event)" />
                        <button class="listen" :aria-label="t(playing || busy ? 'playlistMix.stop' : 'playlistMix.listen')" :aria-busy="busy" @click="preview">
                            <i v-if="playing || busy" class="stop" /><PlayIcon v-else width="15" height="15" />
                        </button>
                        <div class="lengthControl">
                            <MixSelect compact :label="t('playlistMix.duration')" :value="String(draft.seconds)" :options="lengthOptions"
                                :disabled="preset === 'none'" @change="changeLength" />
                        </div>
                    </div>
                </template>
                </div>
            </main>
            <p v-if="waveFailed || previewFailed" role="status">{{ t(previewFailed ? 'playlistMix.previewError' : 'playlistMix.waveError') }}</p>
            <footer>
                <div class="presets" :aria-label="t('playlistMix.presets')"><button v-for="p in mixPresets" :key="p" :aria-pressed="preset === p" @click="choose(p)">{{ t('playlistMix.' + p) }}</button></div>
                <div class="parameters">
                    <MixSelect :label="t('playlistMix.volume')" :value="draft.volumeCurve" @focusin="selectedEffect = 'volume'" :disabled="preset === 'none'"
                        :options="options(['smooth','linear','overlap','rise','cut'] as const)" @change="draft.volumeCurve = $event; changeEffect('volume')" />
                    <MixSelect :label="'EQ'" :value="draft.eqBand" @focusin="selectedEffect = 'eq'" :disabled="preset === 'none'"
                        :options="options(['off','centerBass','bass','mids','highs'] as const)" @change="draft.eqBand = $event; changeEffect('eq')" />
                    <MixSelect :label="t('playlistMix.filter')" :value="draft.filter" @focusin="selectedEffect = 'filter'" :disabled="preset === 'none'"
                        :options="options(['off','lowPass','highPass'] as const)" @change="draft.filter = $event; changeEffect('filter')" />
                </div>
            </footer>
        </article>
    </dialog>
    </Transition>
</template>

<style scoped>
dialog { --mix-volume:#51c6ef; --mix-eq:#ffcd28; --mix-filter:#dc8eef; margin:auto; width:min(1240px,calc(94vw / var(--overlay-scale, 1))); max-height:94dvh; border:0; padding:0; border-radius:28px; background:var(--light-bg); color:white; color-scheme:dark; box-shadow:0 20px 80px #0006; }
.expanded { width:calc(98vw / var(--overlay-scale, 1)); max-width:none; }
.expanded :deep(.detail) { height:clamp(90px,21vh,220px); }
dialog::backdrop { background:#000a; }
.mix-modal-enter-active, .mix-modal-leave-active {
    transition: opacity var(--duration-modal) var(--ease-smooth), transform var(--duration-modal) var(--ease-smooth);
}
.mix-modal-enter-active::backdrop, .mix-modal-leave-active::backdrop {
    transition: opacity var(--duration-modal) var(--ease-smooth);
}
.mix-modal-enter-from, .mix-modal-leave-to {
    opacity: 0;
    transform: translateY(8px) scale(.98);
}
.mix-modal-enter-from::backdrop, .mix-modal-leave-to::backdrop {
    opacity: 0;
}
article { padding:22px 26px; }
header,.track,.presets,.parameters { display:flex; align-items:center; }
header { gap:10px; margin-bottom:16px; }
h2 { flex:1; margin:0; font:800 22px Montserrat,sans-serif; }
button { font:inherit; color:inherit; cursor:pointer; border:0; }
button:disabled { opacity:.45; cursor:default; }
.save { background:#f1f1f1; color:#191919; border-radius:18px; padding:8px 15px; font-size:13px; font-weight:700; }
.close { background:transparent; width:34px; height:34px; display:grid; place-items:center; }
main { display:flex; flex-direction:column; }
.waiting { visibility:hidden; }
main { position:relative; }
.waveLoading { position:absolute; inset:0; padding:0; }
.overviewRange { position:absolute; inset-block:0; min-width:1px; border-radius:3px; background:#ffffff16; pointer-events:none; }
.overviews { position:relative; background:var(--dark-bg); overflow:hidden; border-radius:8px; margin:0 0 10px; }
.workstation { position:relative; isolation:isolate; background:transparent; border-radius:14px; padding-block:14px; margin:10px 0 20px; }
.workZone { position:absolute; inset-block:0; z-index:-1; border-radius:14px; background:hsl(var(--accent-h),min(var(--accent-s),12%),18%); overflow:hidden; }
.workZone i { position:absolute; inset-block:0; width:1px; background:#ffffff10; }
.lengthControl { position:absolute; left:50%; bottom:0; transform:translate(-50%,50%); z-index:2; }
.track { gap:11px; padding:3px 0; }
.track img { width:40px; height:40px; border-radius:7px; object-fit:cover; }
.name { flex:1; min-width:0; }
.track strong,.track small { display:block; overflow:hidden; white-space:nowrap; text-overflow:ellipsis; }
.track strong { font-size:15px; }
small { color:#fffa; font-size:12px; }
.data { text-align:right; font-size:12px; font-variant-numeric:tabular-nums; }
.data b { font-weight:600; }
.data em { display:inline-block; font-style:normal; font-weight:700; color:var(--mix-filter); background:var(--dark-bg); padding:1px 5px; border-radius:4px; margin-right:7px; }
.listen { position:absolute; left:50%; top:50%; transform:translate(-50%,-50%); width:46px; height:30px; display:grid; place-items:center; border-radius:14px; background:hsl(var(--accent-h),min(var(--accent-s),30%),25%); color:white; }
.stop { width:10px; height:10px; border-radius:2px; background:currentColor; }
footer { margin-top:16px; }
.presets { gap:5px; overflow-x:auto; padding:2px; scrollbar-width:thin; }
.presets button { flex:1 0 auto; white-space:nowrap; border-radius:10px; padding:9px 14px; background:var(--dark-bg); color:#fffb; font-size:13px; }
.presets button[aria-pressed=true] { background:#eee; color:#191919; }
.parameters { gap:8px; margin-top:10px; }
.parameters > :nth-child(1) { --picker-color:var(--mix-volume); }
.parameters > :nth-child(2) { --picker-color:var(--mix-eq); }
.parameters > :nth-child(3) { --picker-color:var(--mix-filter); }
p { color:#fffb; font-size:12px; }
@media(max-width:600px) {
    dialog { width:100%; max-width:100%; max-height:100dvh; margin:auto 0 0; border-radius:22px 22px 0 0; }
    .mix-modal-enter-from, .mix-modal-leave-to { opacity:1; transform:translateY(100%); }
    article { padding:16px 14px max(16px,env(safe-area-inset-bottom)); }
    .expand { display:none; }
    main :deep(.detail) { height:clamp(94px, 20dvh, 185px); }
    h2 { font-size:18px; } header { gap:4px; margin-bottom:12px; }
    footer { display:flex; flex-direction:column-reverse; gap:8px; margin-top:8px; }
    .parameters { gap:4px; margin:0; }
    .presets button { min-height:38px; }
}
@media (prefers-reduced-motion: reduce) {
    .mix-modal-enter-active, .mix-modal-leave-active,
    .mix-modal-enter-active::backdrop, .mix-modal-leave-active::backdrop {
        transition: none;
    }
}
</style>
