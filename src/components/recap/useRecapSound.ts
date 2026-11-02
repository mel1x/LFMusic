import { ref, watch } from 'vue'

// Looping track fragments behind the recap cards (Web Audio: gapless loop and
// smooth crossfades between cards). Fragments come from the lfmusic-snippet
// protocol and are decoded once per recap session.

const DEFAULT_VOLUME = 0.85
const VOLUME_KEY = 'lfmusic_recap_volume'
const FADE_IN = 0.9
const FADE_OUT = 0.6
/** Short fade baked into each fragment's edges so the loop seam doesn't click. */
const EDGE_FADE_SECS = 0.06
/** Decoded fragments kept around (~5 MB of PCM each): current, next and a few to go back to. */
const MAX_BUFFERS = 6

interface Voice {
  url: string
  source: AudioBufferSourceNode
  gain: GainNode
}

function savedVolume(): number {
  try {
    const v = parseFloat(localStorage.getItem(VOLUME_KEY) ?? '')
    return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : DEFAULT_VOLUME
  } catch {
    return DEFAULT_VOLUME
  }
}

export function useRecapSound() {
  const muted = ref(false)
  /** 0..1, separate from the player's volume; remembered between recaps. */
  const volume = ref(savedVolume())
  const playingUrl = ref<string | null>(null)
  let ctx: AudioContext | null = null
  let master: GainNode | null = null
  let voice: Voice | null = null
  const buffers = new Map<string, Promise<AudioBuffer | null>>()

  /** Must run inside the click that opens the recap (autoplay policy). */
  function start() {
    if (ctx) return
    ctx = new AudioContext()
    master = ctx.createGain()
    master.gain.value = outputGain()
    master.connect(ctx.destination)
  }

  function softenEdges(buffer: AudioBuffer) {
    const n = Math.min(Math.floor(buffer.sampleRate * EDGE_FADE_SECS), Math.floor(buffer.length / 2))
    for (let c = 0; c < buffer.numberOfChannels; c++) {
      const data = buffer.getChannelData(c)
      for (let i = 0; i < n; i++) {
        const k = i / n
        data[i] *= k
        data[data.length - 1 - i] *= k
      }
    }
  }

  function load(url: string): Promise<AudioBuffer | null> {
    let pending = buffers.get(url)
    if (pending) {
      // Map order is the LRU order: move the hit to the end.
      buffers.delete(url)
      buffers.set(url, pending)
    } else {
      const audio = ctx!
      pending = fetch(url)
        .then(res => (res.ok ? res.arrayBuffer() : Promise.reject(res.status)))
        .then(data => audio.decodeAudioData(data))
        .then(buffer => {
          softenEdges(buffer)
          return buffer
        })
        .catch(() => null)
      buffers.set(url, pending)
      // A playing voice holds its own buffer, so evicting it here is safe.
      for (const key of buffers.keys()) {
        if (buffers.size <= MAX_BUFFERS) break
        if (key !== url && key !== playingUrl.value) buffers.delete(key)
      }
    }
    return pending
  }

  function fadeOut(v: Voice) {
    if (!ctx) return
    const now = ctx.currentTime
    v.gain.gain.cancelScheduledValues(now)
    v.gain.gain.setValueAtTime(v.gain.gain.value, now)
    v.gain.gain.linearRampToValueAtTime(0, now + FADE_OUT)
    v.source.stop(now + FADE_OUT + 0.05)
  }

  /** Crossfades to `url` (or to silence with null). Repeated calls for the same url are no-ops. */
  async function play(url: string | null) {
    if (!ctx || !master || playingUrl.value === url) return
    playingUrl.value = url
    if (voice) {
      fadeOut(voice)
      voice = null
    }
    if (!url) return
    const buffer = await load(url)
    // Another card may have taken over while the fragment was decoding.
    if (!buffer || !ctx || !master || playingUrl.value !== url) return

    const source = ctx.createBufferSource()
    source.buffer = buffer
    source.loop = true
    const gain = ctx.createGain()
    const now = ctx.currentTime
    gain.gain.setValueAtTime(0, now)
    gain.gain.linearRampToValueAtTime(1, now + FADE_IN)
    source.connect(gain).connect(master)
    source.start()
    voice = { url, source, gain }
  }

  /** Warms the decode cache so the next card starts without a gap. */
  function preload(url: string | null | undefined) {
    if (ctx && url) void load(url)
  }

  function setPaused(paused: boolean) {
    if (!ctx) return
    void (paused ? ctx.suspend() : ctx.resume())
  }

  async function stop() {
    if (voice) fadeOut(voice)
    voice = null
    playingUrl.value = null
    const closing = ctx
    ctx = null
    master = null
    buffers.clear()
    if (closing) {
      await new Promise(resolve => setTimeout(resolve, FADE_OUT * 1000))
      await closing.close().catch(() => {})
    }
  }

  /** Perceived loudness is closer to linear with a squared gain. */
  function outputGain(): number {
    return muted.value ? 0 : volume.value * volume.value
  }

  watch([muted, volume], () => {
    if (!ctx || !master) return
    const now = ctx.currentTime
    master.gain.cancelScheduledValues(now)
    master.gain.setValueAtTime(master.gain.value, now)
    master.gain.linearRampToValueAtTime(outputGain(), now + 0.08)
  })

  watch(volume, value => {
    try {
      localStorage.setItem(VOLUME_KEY, String(value))
    } catch {
      // Not saved; the default comes back next time.
    }
  })

  /** Sets the level; any change away from zero also unmutes. */
  function setVolume(value: number) {
    volume.value = Math.round(Math.min(1, Math.max(0, value)) * 100) / 100
    muted.value = volume.value === 0
  }

  function toggleMute() {
    if (muted.value && volume.value === 0) setVolume(0.5)
    else muted.value = !muted.value
  }

  return { muted, volume, playingUrl, start, play, preload, setPaused, stop, setVolume, toggleMute }
}
