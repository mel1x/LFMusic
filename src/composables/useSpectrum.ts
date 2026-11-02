import { watch, effectScope } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { usePlayer } from './usePlayer';

// Shared FFT spectrum from the Rust backend, SPECTRUM_BARS bars (0..1), for every
// visualizer at once: one backend tap and one polling loop, enabled while at
// least one consumer holds it. Data is polled at ~25Hz and smoothed toward the
// targets at up to ~60fps (fast attack, slow decay so it looks alive instead of
// jittery). Levels go to listeners as a plain array and consumers write the bar
// transforms straight to the DOM: no Vue re-render per frame.
export const SPECTRUM_BARS = 16;

export type SpectrumListener = (levels: Float32Array) => void;

const FETCH_MS = 40;
// Cap render rate so 120/144Hz screens don't do extra work (and decay speed stays the same).
const FRAME_MS = 15;
// Changes smaller than this are invisible; skip notifying listeners.
const EPSILON = 0.002;

const levels = new Float32Array(SPECTRUM_BARS);
const listeners = new Set<SpectrumListener>();

let raf: number | null = null;
let targets: number[] = new Array(SPECTRUM_BARS).fill(0);
let lastFetch = 0;
let lastFrame = 0;
let fetching = false;
let playingWatchSet = false;

async function fetchSpectrum() {
    if (fetching) return;
    fetching = true;
    try {
        const bars = await invoke<number[]>('player_get_spectrum', { bars: SPECTRUM_BARS });
        if (Array.isArray(bars) && bars.length === SPECTRUM_BARS) {
            targets = bars;
        }
    } catch {
        // Backend unavailable (e.g. plain Vite mode) - keep zeros.
    } finally {
        fetching = false;
    }
}

function notify() {
    for (const fn of listeners) fn(levels);
}

function frame(now: number) {
    raf = null;
    if (listeners.size === 0) return;
    const playing = usePlayer().isPlaying.value;

    if (now - lastFrame < FRAME_MS) {
        raf = requestAnimationFrame(frame);
        return;
    }
    lastFrame = now;

    if (playing && now - lastFetch >= FETCH_MS) {
        lastFetch = now;
        void fetchSpectrum();
    }
    if (!playing) {
        targets.fill(0);
    }

    let moving = false;
    let changed = false;
    for (let i = 0; i < SPECTRUM_BARS; i++) {
        const target = targets[i] ?? 0;
        const c = levels[i];
        // Attack faster than decay: peaks jump up, don't jitter down.
        const k = target > c ? 0.5 : 0.16;
        let next = c + (target - c) * k;
        if (target === 0 && next < 0.001) next = 0;
        if (Math.abs(next - c) > EPSILON || (next === 0 && c !== 0)) changed = true;
        levels[i] = next;
        if (next > 0.004 || target > 0.004) moving = true;
    }
    if (changed) notify();

    // Paused and settled: the loop sleeps until playback resumes.
    if (playing || moving) {
        raf = requestAnimationFrame(frame);
    }
}

function start() {
    if (raf == null) {
        lastFetch = 0;
        lastFrame = 0;
        raf = requestAnimationFrame(frame);
    }
}

function stop() {
    if (raf != null) {
        cancelAnimationFrame(raf);
        raf = null;
    }
}

function subscribe(fn: SpectrumListener) {
    listeners.add(fn);
    fn(levels);
    if (listeners.size > 1) return;
    if (!playingWatchSet) {
        playingWatchSet = true;
        // Play/pause restarts the loop; on pause the bars settle smoothly. Detached scope:
        // the first subscriber is a component, and its unmount must not stop this watcher.
        effectScope(true).run(() => {
            watch(usePlayer().isPlaying, () => { if (listeners.size > 0) start(); });
        });
    }
    void invoke('player_set_spectrum_enabled', { enabled: true }).catch(() => {});
    start();
}

function unsubscribe(fn: SpectrumListener) {
    if (!listeners.delete(fn) || listeners.size > 0) return;
    stop();
    targets = new Array(SPECTRUM_BARS).fill(0);
    levels.fill(0);
    void invoke('player_set_spectrum_enabled', { enabled: false }).catch(() => {});
}

/** Holds the spectrum while `start(listener)` is active; `stop()` releases it. Safe to call repeatedly. */
export function useSpectrum() {
    let current: SpectrumListener | null = null;
    return {
        start(fn: SpectrumListener) {
            if (current) return;
            current = fn;
            subscribe(fn);
        },
        stop() {
            if (!current) return;
            const fn = current;
            current = null;
            unsubscribe(fn);
            // Leave bars at rest.
            fn(new Float32Array(SPECTRUM_BARS));
        },
    };
}
