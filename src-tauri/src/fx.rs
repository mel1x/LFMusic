//! Creative effects: speed, space (reverb), bitcrusher and vinyl. Sources poll a version counter,
//! so there is no per-sample locking.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use rodio::{ChannelCount, SampleRate, Source};
use serde::{Deserialize, Serialize};

use crate::audio_player::{nz_channels, nz_rate, soft_clip};

pub const SPEED_MIN: f32 = 0.5;
pub const SPEED_MAX: f32 = 2.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpaceFx {
    pub enabled: bool,
    /// Wet amount, 0..1.
    pub mix: f32,
    /// Room size, 0..1.
    pub size: f32,
}

impl Default for SpaceFx {
    fn default() -> Self {
        Self { enabled: false, mix: 0.35, size: 0.6 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrusherFx {
    pub enabled: bool,
    /// Bit depth, 2..16.
    pub bits: u32,
    /// Sample-and-hold factor, 1..8 (1 = no rate reduction).
    pub downsample: f32,
}

pub const DOWNSAMPLE_MAX: f32 = 8.0;

impl Default for CrusherFx {
    fn default() -> Self {
        Self { enabled: false, bits: 8, downsample: 3.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VinylFx {
    pub enabled: bool,
    /// Crackle and hiss, 0..1.
    pub crackle: f32,
    /// Wear: muffled highs, thin lows and pitch wobble, 0..1.
    pub wear: f32,
}

impl Default for VinylFx {
    fn default() -> Self {
        Self { enabled: false, crackle: 0.5, wear: 0.4 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FxSettings {
    pub space: SpaceFx,
    pub crusher: CrusherFx,
    pub vinyl: VinylFx,
    /// Playback rate; pitch follows it, like a turntable.
    pub speed: f32,
}

impl Default for FxSettings {
    fn default() -> Self {
        Self {
            space: SpaceFx::default(),
            crusher: CrusherFx::default(),
            vinyl: VinylFx::default(),
            speed: 1.0,
        }
    }
}

impl FxSettings {
    /// Clamps every field into its valid range (values come from the UI / settings.json).
    pub fn sanitized(mut self) -> Self {
        let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        self.space.mix = unit(self.space.mix);
        self.space.size = unit(self.space.size);
        self.crusher.bits = self.crusher.bits.clamp(2, 16);
        self.crusher.downsample = if self.crusher.downsample.is_finite() {
            self.crusher.downsample.clamp(1.0, DOWNSAMPLE_MAX)
        } else {
            1.0
        };
        self.vinyl.crackle = unit(self.vinyl.crackle);
        self.vinyl.wear = unit(self.vinyl.wear);
        self.speed = if self.speed.is_finite() { self.speed.clamp(SPEED_MIN, SPEED_MAX) } else { 1.0 };
        self
    }
}

pub struct FxShared {
    settings: RwLock<FxSettings>,
    version: AtomicU64,
    speed_bits: AtomicU32,
}

impl FxShared {
    pub fn new() -> Self {
        Self {
            settings: RwLock::new(FxSettings::default()),
            version: AtomicU64::new(0),
            speed_bits: AtomicU32::new(1.0f32.to_bits()),
        }
    }

    pub fn settings(&self) -> FxSettings {
        self.settings.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn apply(&self, settings: FxSettings) {
        let settings = settings.sanitized();
        self.speed_bits.store(settings.speed.to_bits(), Ordering::Relaxed);
        if let Ok(mut s) = self.settings.write() {
            *s = settings;
        }
        self.version.fetch_add(1, Ordering::Release);
    }

    pub fn speed(&self) -> f32 {
        f32::from_bits(self.speed_bits.load(Ordering::Relaxed))
    }

    fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }
}

impl Default for FxShared {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SpeedSource<S: Source<Item = f32>> {
    source: S,
    shared: Arc<FxShared>,
    channels: usize,
    sample_rate: u32,
    prev: Vec<f32>,
    next: Vec<f32>,
    out: Vec<f32>,
    out_idx: usize,
    /// Position between `prev` and `next`, 0..1.
    frac: f32,
    primed: bool,
    ended: bool,
}

impl<S: Source<Item = f32>> SpeedSource<S> {
    pub fn new(source: S, shared: Arc<FxShared>) -> Self {
        let channels = source.channels().get() as usize;
        let sample_rate = source.sample_rate().get();
        Self {
            source,
            shared,
            channels,
            sample_rate,
            prev: vec![0.0; channels],
            next: vec![0.0; channels],
            out: vec![0.0; channels],
            out_idx: channels,
            frac: 0.0,
            primed: false,
            ended: false,
        }
    }

    fn read_frame(&mut self) -> bool {
        std::mem::swap(&mut self.prev, &mut self.next);
        for c in 0..self.channels {
            match self.source.next() {
                Some(s) => self.next[c] = s,
                None => return false,
            }
        }
        true
    }

    fn produce_frame(&mut self) -> bool {
        if self.ended {
            return false;
        }
        if !self.primed {
            self.primed = true;
            if !self.read_frame() || !self.read_frame() {
                self.ended = true;
                return false;
            }
        }
        let t = self.frac;
        for c in 0..self.channels {
            self.out[c] = self.prev[c] + (self.next[c] - self.prev[c]) * t;
        }
        self.frac += self.shared.speed();
        while self.frac >= 1.0 {
            self.frac -= 1.0;
            if !self.read_frame() {
                self.ended = true;
                break;
            }
        }
        true
    }
}

impl<S: Source<Item = f32>> Iterator for SpeedSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.out_idx >= self.channels {
            if !self.produce_frame() {
                return None;
            }
            self.out_idx = 0;
        }
        let s = self.out[self.out_idx];
        self.out_idx += 1;
        Some(s)
    }
}

impl<S: Source<Item = f32>> Source for SpeedSource<S> {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> ChannelCount { nz_channels(self.channels as u16) }
    fn sample_rate(&self) -> SampleRate { nz_rate(self.sample_rate) }
    fn total_duration(&self) -> Option<Duration> {
        let speed = self.shared.speed().max(SPEED_MIN);
        self.source.total_duration().map(|d| d.div_f32(speed))
    }
}

/// xorshift32: cheap noise for crackle and hiss.
struct Rng(u32);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
    fn chance(&mut self) -> f32 {
        self.next_f32() * 0.5 + 0.5
    }
}

/// Freeverb comb filter with a one-pole damping in the feedback path.
struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0, store: 0.0 }
    }
    #[inline]
    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.idx] = input + self.store * feedback;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(1)], idx: 0 }
    }
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buf[self.idx];
        let out = buffered - input;
        self.buf[self.idx] = input + buffered * 0.5;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

/// One Freeverb channel (8 parallel combs into 4 series allpasses).
struct ReverbChannel {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}

impl ReverbChannel {
    fn new(sample_rate: u32, spread: usize) -> Self {
        let scale = sample_rate as f32 / 44100.0;
        let len = |n: usize| ((n + spread) as f32 * scale) as usize;
        Self {
            combs: COMB_TUNING.iter().map(|&n| Comb::new(len(n))).collect(),
            allpasses: ALLPASS_TUNING.iter().map(|&n| Allpass::new(len(n))).collect(),
        }
    }
    #[inline]
    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let mut out = 0.0;
        for comb in &mut self.combs {
            out += comb.process(input, feedback, damp);
        }
        for ap in &mut self.allpasses {
            out = ap.process(out);
        }
        out
    }
}

/// Short modulated delay line for the vinyl wow/flutter.
struct WobbleLine {
    buf: Vec<f32>,
    write: usize,
}

impl WobbleLine {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len], write: 0 }
    }
    #[inline]
    fn process(&mut self, input: f32, delay: f32) -> f32 {
        let len = self.buf.len();
        self.buf[self.write] = input;
        let read = self.write as f32 - delay;
        let read = if read < 0.0 { read + len as f32 } else { read };
        let i0 = read.floor() as usize % len;
        let i1 = (i0 + 1) % len;
        let t = read - read.floor();
        self.write = (self.write + 1) % len;
        self.buf[i0] + (self.buf[i1] - self.buf[i0]) * t
    }
}

/// Bitcrusher, vinyl and space, in that order, after the user EQ.
pub struct FxSource<S: Source<Item = f32>> {
    source: S,
    shared: Arc<FxShared>,
    channels: usize,
    sample_rate: u32,
    settings: FxSettings,
    version: u64,
    frames_since_check: usize,
    frame: Vec<f32>,
    out_idx: usize,

    hold: Vec<f32>,
    /// Advances by 1/downsample per frame; a new sample is taken on each wrap.
    hold_phase: f32,

    rng: Rng,
    wobble: Vec<WobbleLine>,
    wobble_phase: f32,
    flutter_phase: f32,
    lp_state: Vec<f32>,
    hp_state: Vec<f32>,
    hp_prev: Vec<f32>,
    click: f32,
    hiss_state: f32,

    reverb: Vec<ReverbChannel>,
}

impl<S: Source<Item = f32>> FxSource<S> {
    pub fn new(source: S, shared: Arc<FxShared>) -> Self {
        let channels = source.channels().get() as usize;
        let sample_rate = source.sample_rate().get();
        let settings = shared.settings();
        let version = shared.version();
        let wobble_len = (sample_rate as f32 * 0.03) as usize + 4;
        Self {
            source,
            shared,
            channels,
            sample_rate,
            settings,
            version,
            frames_since_check: 0,
            frame: vec![0.0; channels],
            out_idx: channels,
            hold: vec![0.0; channels],
            hold_phase: 1.0,
            rng: Rng(0x9E37_79B9),
            wobble: (0..channels).map(|_| WobbleLine::new(wobble_len)).collect(),
            wobble_phase: 0.0,
            flutter_phase: 0.0,
            lp_state: vec![0.0; channels],
            hp_state: vec![0.0; channels],
            hp_prev: vec![0.0; channels],
            click: 0.0,
            hiss_state: 0.0,
            reverb: (0..channels.min(2)).map(|c| ReverbChannel::new(sample_rate, c * STEREO_SPREAD)).collect(),
        }
    }

    fn maybe_reload(&mut self) {
        self.frames_since_check += 1;
        if self.frames_since_check < 512 {
            return;
        }
        self.frames_since_check = 0;
        let v = self.shared.version();
        if v != self.version {
            self.version = v;
            self.settings = self.shared.settings();
        }
    }

    fn crush(&mut self) {
        let cfg = &self.settings.crusher;
        if self.hold_phase >= 1.0 {
            self.hold_phase -= 1.0;
            let levels = (1u32 << (cfg.bits - 1)) as f32;
            for c in 0..self.channels {
                self.hold[c] = (self.frame[c] * levels).round() / levels;
            }
        }
        self.hold_phase += 1.0 / cfg.downsample;
        self.frame.copy_from_slice(&self.hold);
    }

    fn vinyl(&mut self) {
        let crackle = self.settings.vinyl.crackle;
        let wear = self.settings.vinyl.wear;
        let sr = self.sample_rate as f32;
        let tau = std::f32::consts::TAU;

        self.wobble_phase = (self.wobble_phase + 0.55 / sr) % 1.0;
        self.flutter_phase = (self.flutter_phase + 6.0 / sr) % 1.0;
        let depth = sr * 0.0018 * wear;
        let delay = sr * 0.004
            + depth * (tau * self.wobble_phase).sin()
            + depth * 0.15 * (tau * self.flutter_phase).sin();

        let cutoff = 18000.0 - wear * 14500.0;
        let lp_a = (-tau * cutoff / sr).exp();
        let hp_cut = 20.0 + wear * 130.0;
        let hp_a = (-tau * hp_cut / sr).exp();

        if self.rng.chance() < crackle * crackle * 0.0009 {
            self.click = self.rng.next_f32() * (0.05 + 0.25 * crackle);
        }
        let click = self.click;
        self.click *= 0.55;
        self.hiss_state = self.hiss_state * 0.6 + self.rng.next_f32() * 0.4;
        let noise = click + self.hiss_state * 0.006 * crackle;

        for c in 0..self.channels {
            let x = self.wobble[c].process(self.frame[c], delay);
            self.lp_state[c] = x * (1.0 - lp_a) + self.lp_state[c] * lp_a;
            let lp = self.lp_state[c];
            let hp = hp_a * (self.hp_state[c] + lp - self.hp_prev[c]);
            self.hp_prev[c] = lp;
            self.hp_state[c] = hp;
            self.frame[c] = hp + noise;
        }
    }

    fn space(&mut self) {
        let mix = self.settings.space.mix;
        let size = self.settings.space.size;
        let feedback = 0.7 + size * 0.28;
        let damp = 0.45 - size * 0.2;
        let dry = 1.0 - mix * 0.5;
        let wet = mix * 3.0;

        let input = if self.channels >= 2 {
            (self.frame[0] + self.frame[1]) * 0.5
        } else {
            self.frame[0]
        } * 0.015;

        for (c, ch) in self.reverb.iter_mut().enumerate() {
            let w = ch.process(input, feedback, damp);
            self.frame[c] = self.frame[c] * dry + w * wet;
        }
    }

    fn process_frame(&mut self) {
        self.maybe_reload();
        let any = self.settings.crusher.enabled || self.settings.vinyl.enabled || self.settings.space.enabled;
        if !any {
            return;
        }
        if self.settings.crusher.enabled {
            self.crush();
        }
        if self.settings.vinyl.enabled {
            self.vinyl();
        }
        if self.settings.space.enabled {
            self.space();
        }
        for s in &mut self.frame {
            *s = soft_clip(*s);
        }
    }
}

impl<S: Source<Item = f32>> Iterator for FxSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.out_idx >= self.channels {
            for c in 0..self.channels {
                self.frame[c] = self.source.next()?;
            }
            self.process_frame();
            self.out_idx = 0;
        }
        let s = self.frame[self.out_idx];
        self.out_idx += 1;
        Some(s)
    }
}

impl<S: Source<Item = f32>> Source for FxSource<S> {
    fn current_span_len(&self) -> Option<usize> { self.source.current_span_len() }
    fn channels(&self) -> ChannelCount { nz_channels(self.channels as u16) }
    fn sample_rate(&self) -> SampleRate { nz_rate(self.sample_rate) }
    fn total_duration(&self) -> Option<Duration> { self.source.total_duration() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;

    fn sine(frames: usize, channels: u16) -> SamplesBuffer {
        let data: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let v = (i as f32 * 0.05).sin() * 0.5;
                std::iter::repeat_n(v, channels as usize)
            })
            .collect();
        SamplesBuffer::new(nz_channels(channels), nz_rate(44100), data)
    }

    fn shared_with(f: impl FnOnce(&mut FxSettings)) -> Arc<FxShared> {
        let shared = Arc::new(FxShared::new());
        let mut s = FxSettings::default();
        f(&mut s);
        shared.apply(s);
        shared
    }

    #[test]
    fn disabled_effects_pass_audio_through() {
        let input: Vec<f32> = sine(2000, 2).collect();
        let out: Vec<f32> = FxSource::new(sine(2000, 2), shared_with(|_| {})).collect();
        assert_eq!(input, out);
    }

    #[test]
    fn speed_changes_length_and_keeps_format() {
        let fast = SpeedSource::new(sine(10_000, 2), shared_with(|s| s.speed = 2.0));
        assert_eq!(fast.channels().get(), 2);
        let n = fast.count();
        assert!((9_000..=11_000).contains(&n), "2x speed should halve 20000 samples, got {n}");

        let slow = SpeedSource::new(sine(10_000, 2), shared_with(|s| s.speed = 0.5)).count();
        assert!((39_000..=41_000).contains(&slow), "0.5x speed should double, got {slow}");
    }

    #[test]
    fn normal_speed_keeps_samples() {
        let input: Vec<f32> = sine(500, 1).collect();
        let out: Vec<f32> = SpeedSource::new(sine(500, 1), shared_with(|_| {})).collect();
        assert_eq!(&input[..out.len()], &out[..]);
        assert!(out.len() >= input.len() - 1);
    }

    #[test]
    fn crusher_quantizes_to_bit_depth() {
        let shared = shared_with(|s| {
            s.crusher.enabled = true;
            s.crusher.bits = 2;
            s.crusher.downsample = 1.0;
        });
        let out: Vec<f32> = FxSource::new(sine(3000, 1), shared).collect();
        assert!(out.iter().all(|v| ((v * 2.0).round() - v * 2.0).abs() < 1e-4 || v.abs() > 0.97));
    }

    #[test]
    fn fractional_downsample_holds_between_whole_factors() {
        let distinct = |factor: f32| {
            let shared = shared_with(|s| {
                s.crusher.enabled = true;
                s.crusher.bits = 16;
                s.crusher.downsample = factor;
            });
            let out: Vec<f32> = FxSource::new(sine(4000, 1), shared).collect();
            out.windows(2).filter(|w| w[0] != w[1]).count()
        };
        let (two, two_half, three) = (distinct(2.0), distinct(2.5), distinct(3.0));
        assert!(two > two_half && two_half > three, "{two} {two_half} {three}");
    }

    #[test]
    fn old_integer_settings_still_load() {
        let s: FxSettings = serde_json::from_str(r#"{"crusher":{"enabled":true,"bits":6,"downsample":20}}"#).unwrap();
        assert_eq!(s.sanitized().crusher.downsample, DOWNSAMPLE_MAX);
    }

    #[test]
    fn all_effects_stay_finite_and_bounded() {
        let shared = shared_with(|s| {
            s.space.enabled = true;
            s.space.mix = 1.0;
            s.space.size = 1.0;
            s.crusher.enabled = true;
            s.vinyl.enabled = true;
            s.vinyl.crackle = 1.0;
            s.vinyl.wear = 1.0;
        });
        let out: Vec<f32> = FxSource::new(sine(20_000, 2), shared).collect();
        assert_eq!(out.len(), 40_000);
        assert!(out.iter().all(|v| v.is_finite() && v.abs() < 1.0));
    }

    #[test]
    fn reverb_leaves_a_tail() {
        let shared = shared_with(|s| {
            s.space.enabled = true;
            s.space.mix = 0.8;
        });
        let mut data = vec![0.0f32; 20_000];
        data[0] = 0.8;
        let src = SamplesBuffer::new(nz_channels(1), nz_rate(44100), data);
        let out: Vec<f32> = FxSource::new(src, shared).collect();
        let tail: f32 = out[5_000..].iter().map(|v| v.abs()).sum();
        assert!(tail > 0.01, "reverb should ring after the impulse");
    }

    #[test]
    fn settings_are_clamped() {
        let s = FxSettings {
            speed: 9.0,
            crusher: CrusherFx { enabled: true, bits: 40, downsample: 0.0 },
            space: SpaceFx { enabled: true, mix: 3.0, size: f32::NAN },
            ..FxSettings::default()
        }
        .sanitized();
        assert_eq!(s.speed, SPEED_MAX);
        assert_eq!(s.crusher.bits, 16);
        assert_eq!(s.crusher.downsample, 1.0);
        assert_eq!(s.space.mix, 1.0);
        assert_eq!(s.space.size, 0.0);
    }
}
