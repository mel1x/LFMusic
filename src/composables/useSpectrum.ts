import { watch, effectScope } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { usePlayer } from './usePlayer';
import { frameLoop } from '../utils/frameLoop';

export const SPECTRUM_BARS = 16;

export type SpectrumListener = (levels: Float32Array) => void;

const FETCH_MS = 60;
const FRAME_MS = 33;
const ATTACK_60 = 0.5;
const DECAY_60 = 0.16;
const EPSILON = 0.002;

const levels = new Float32Array(SPECTRUM_BARS);
const listeners = new Set<SpectrumListener>();

let targets: number[] = new Array(SPECTRUM_BARS).fill(0);
let lastFetch = 0;
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
    } finally {
        fetching = false;
    }
}

function notify() {
    for (const fn of listeners) fn(levels);
}

const loop = frameLoop(frame, FRAME_MS);

function frame(now: number, dt: number): boolean {
    if (listeners.size === 0) return false;
    const playing = usePlayer().isPlaying.value;
    const steps = Math.min(100, dt || FRAME_MS) / (1000 / 60);
    const attack = 1 - Math.pow(1 - ATTACK_60, steps);
    const decay = 1 - Math.pow(1 - DECAY_60, steps);

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
        const k = target > c ? attack : decay;
        let next = c + (target - c) * k;
        if (target === 0 && next < 0.001) next = 0;
        if (Math.abs(next - c) > EPSILON || (next === 0 && c !== 0)) changed = true;
        levels[i] = next;
        if (next > 0.004 || target > 0.004) moving = true;
    }
    if (changed) notify();

    return playing || moving;
}

function start() {
    if (!loop.running) {
        lastFetch = 0;
        loop.start();
    }
}

function stop() {
    loop.stop();
}

function subscribe(fn: SpectrumListener) {
    listeners.add(fn);
    fn(levels);
    if (listeners.size > 1) return;
    if (!playingWatchSet) {
        playingWatchSet = true;
        // Detached scope: the first subscriber's unmount must not stop this watcher.
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

/** Holds the spectrum while `start(listener)` is active; `stop()` releases it. */
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
            fn(new Float32Array(SPECTRUM_BARS));
        },
    };
}
