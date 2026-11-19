import { ref, computed, watch, effectScope } from "vue";
import { invoke } from "@tauri-apps/api/core";

import { usePlayer } from "./usePlayer";

export interface LyricLine {
    time: number | null;
    text: string;
}

const TIME_TAG = /\[(\d{1,3}):(\d{1,2}(?:[.:]\d{1,3})?)\]/g;
const META_TAG = /^\[[a-z]+:.*\]$/i;

export function parseLyrics(raw: string): { synced: boolean; lines: LyricLine[] } {
    const offsetMatch = raw.match(/^\[offset:\s*([+-]?\d+)\s*\]/im);
    const offset = offsetMatch ? Number(offsetMatch[1]) / 1000 : 0;
    const timed: LyricLine[] = [];
    const plain: LyricLine[] = [];

    for (const rawLine of raw.split(/\r?\n/)) {
        const line = rawLine.trim();
        const times: number[] = [];
        for (const m of line.matchAll(TIME_TAG)) {
            times.push(Number(m[1]) * 60 + Number(m[2].replace(":", ".")) - offset);
        }
        const text = line.replace(TIME_TAG, "").replace(/<\d+:\d+(?:\.\d+)?>/g, "").trim();
        if (times.length) {
            for (const time of times) timed.push({ time: Math.max(0, time), text });
        } else if (!META_TAG.test(line)) {
            plain.push({ time: null, text });
        }
    }

    if (timed.length) {
        timed.sort((a, b) => (a.time ?? 0) - (b.time ?? 0));
        while (timed.length && !timed[0].text) timed.shift();
        return { synced: true, lines: timed };
    }
    const lines = plain.filter((l, i) => l.text || (i > 0 && plain[i - 1].text));
    while (lines.length && !lines[lines.length - 1].text) lines.pop();
    return { synced: false, lines };
}

let instance: ReturnType<typeof createLyrics> | null = null;

export function useLyrics() {
    if (!instance) instance = effectScope(true).run(createLyrics)!;
    return instance;
}

function createLyrics() {
    const { currentTrack, positionSecs } = usePlayer();

    const lyricsPath = ref<string | null>(null);
    const lines = ref<LyricLine[]>([]);
    const synced = ref(false);
    const hasLyrics = computed(() => lines.value.some(l => l.text));

    let requestId = 0;
    watch(() => currentTrack.value?.file_path ?? null, async (path) => {
        const id = ++requestId;
        if (!path) {
            lyricsPath.value = null;
            lines.value = [];
            return;
        }
        let raw: string | null = null;
        try {
            raw = await invoke<string | null>("get_track_lyrics", { filePath: path });
        } catch {
            raw = null;
        }
        if (id !== requestId) return;
        const parsed = raw ? parseLyrics(raw) : { synced: false, lines: [] };
        lyricsPath.value = path;
        synced.value = parsed.synced;
        lines.value = parsed.lines;
    }, { immediate: true });

    const activeIndex = computed(() => {
        if (!synced.value) return -1;
        const pos = positionSecs.value + 0.15;
        const list = lines.value;
        let lo = 0;
        let hi = list.length - 1;
        let found = -1;
        while (lo <= hi) {
            const mid = (lo + hi) >> 1;
            if ((list[mid].time ?? 0) <= pos) {
                found = mid;
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        return found;
    });

    return { lyricsPath, lines, synced, hasLyrics, activeIndex };
}
