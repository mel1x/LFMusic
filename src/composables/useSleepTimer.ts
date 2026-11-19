import { computed, ref } from 'vue'
import { setTrackEndPauseHook, usePlayer } from './usePlayer'

export type SleepMode = 'off' | 'time' | 'track'

export const SLEEP_PRESETS = [15, 30, 45, 60, 90]

const mode = ref<SleepMode>('off')
const minutes = ref(0)
const endsAt = ref(0)
const now = ref(Date.now())
let ticker: ReturnType<typeof setInterval> | null = null

function stopTicker() {
    if (ticker) clearInterval(ticker)
    ticker = null
}

function cancel() {
    mode.value = 'off'
    minutes.value = 0
    stopTicker()
    setTrackEndPauseHook(null)
}

function fire() {
    const { playbackState, togglePlayPause } = usePlayer()
    cancel()
    if (playbackState.value === 'Playing') void togglePlayPause()
}

function startTime(min: number) {
    cancel()
    mode.value = 'time'
    minutes.value = min
    endsAt.value = Date.now() + min * 60_000
    now.value = Date.now()
    ticker = setInterval(() => {
        now.value = Date.now()
        if (now.value >= endsAt.value) fire()
    }, 1000)
}

/** Lets the current track finish, then the next one loads paused. */
function startAfterTrack() {
    cancel()
    mode.value = 'track'
    setTrackEndPauseHook(() => {
        cancel()
        return true
    })
}

const remaining = computed(() => {
    if (mode.value !== 'time') return ''
    const secs = Math.max(0, Math.ceil((endsAt.value - now.value) / 1000))
    const m = Math.floor(secs / 60)
    const s = secs % 60
    return `${m}:${s.toString().padStart(2, '0')}`
})

export function useSleepTimer() {
    return { mode, minutes, remaining, startTime, startAfterTrack, cancel }
}
