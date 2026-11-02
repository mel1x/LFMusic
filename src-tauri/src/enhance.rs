//! Auto sound enhancement: a device-aware stage that runs after the user EQ.
//!
//! - Detects what the default output device is (headphones, laptop speakers,
//!   external speakers, TV/HDMI) from its name and applies a gentle corrective
//!   tone curve for it (rumble filter, bass/presence/air shelves, stereo width,
//!   crossfeed on headphones).
//! - Keeps loudness even between tracks: slow AGC towards a target level,
//!   a soft-knee compressor for peaks and the shared soft limiter.
//! - Toggling is click-free: dry and wet signals are crossfaded.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use rodio::Source;
use serde::{Deserialize, Serialize};

use crate::audio_player::{soft_clip, BiquadCoeffs, BiquadState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceProfile {
    Headphones,
    Laptop,
    Speakers,
    Tv,
    Universal,
}

impl DeviceProfile {
    const ALL: [DeviceProfile; 5] = [
        DeviceProfile::Headphones,
        DeviceProfile::Laptop,
        DeviceProfile::Speakers,
        DeviceProfile::Tv,
        DeviceProfile::Universal,
    ];

    fn to_u8(self) -> u8 {
        Self::ALL.iter().position(|&p| p == self).unwrap_or(4) as u8 + 1
    }

    /// 0 means "none" (auto).
    fn from_u8(v: u8) -> Option<Self> {
        if v == 0 { None } else { Self::ALL.get(v as usize - 1).copied() }
    }
}

/// Persisted in `settings.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EnhanceSettings {
    pub enabled: bool,
    /// Forced device profile; `None` = detect automatically.
    pub device: Option<DeviceProfile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnhanceStatus {
    pub enabled: bool,
    pub device: Option<DeviceProfile>,
    pub detected: DeviceProfile,
    pub device_name: Option<String>,
}

/// Shared between `AudioPlayer` (settings) and every `EnhanceSource` (audio thread).
pub struct EnhanceShared {
    enabled: AtomicBool,
    forced: AtomicU8,
    detected: AtomicU8,
    device_name: RwLock<Option<String>>,
    /// AGC gain (dB, f32 bits) the previous source settled on; the next track
    /// starts from it so consecutive tracks don't jump in level.
    last_gain_db: AtomicU32,
}

impl EnhanceShared {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            forced: AtomicU8::new(0),
            detected: AtomicU8::new(DeviceProfile::Universal.to_u8()),
            device_name: RwLock::new(None),
            last_gain_db: AtomicU32::new(0.0_f32.to_bits()),
        }
    }

    pub fn apply(&self, settings: &EnhanceSettings) {
        self.enabled.store(settings.enabled, Ordering::Relaxed);
        self.forced.store(settings.device.map_or(0, |d| d.to_u8()), Ordering::Relaxed);
    }

    pub fn settings(&self) -> EnhanceSettings {
        EnhanceSettings {
            enabled: self.enabled.load(Ordering::Relaxed),
            device: DeviceProfile::from_u8(self.forced.load(Ordering::Relaxed)),
        }
    }

    pub fn set_detected(&self, name: Option<String>, profile: DeviceProfile) {
        self.detected.store(profile.to_u8(), Ordering::Relaxed);
        if let Ok(mut n) = self.device_name.write() {
            *n = name;
        }
    }

    fn detected(&self) -> DeviceProfile {
        DeviceProfile::from_u8(self.detected.load(Ordering::Relaxed)).unwrap_or(DeviceProfile::Universal)
    }

    fn active_profile(&self) -> DeviceProfile {
        DeviceProfile::from_u8(self.forced.load(Ordering::Relaxed)).unwrap_or_else(|| self.detected())
    }

    pub fn status(&self) -> EnhanceStatus {
        let s = self.settings();
        EnhanceStatus {
            enabled: s.enabled,
            device: s.device,
            detected: self.detected(),
            device_name: self.device_name.read().ok().and_then(|n| n.clone()),
        }
    }

    fn last_gain_db(&self) -> f32 {
        f32::from_bits(self.last_gain_db.load(Ordering::Relaxed))
    }

    fn store_last_gain_db(&self, db: f32) {
        self.last_gain_db.store(db.to_bits(), Ordering::Relaxed);
    }
}

// ── Device detection ─────────────────────────────────────────────────────────

/// Name of the default output device (the one rodio's `OutputStream::try_default` opens)
/// and its guessed profile. Call on the audio thread right after opening the stream.
pub fn detect_output_device() -> (Option<String>, DeviceProfile) {
    use rodio::cpal::traits::HostTrait;
    use rodio::DeviceTrait;

    let name = rodio::cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok());
    let profile = classify_device(name.as_deref().unwrap_or(""), has_battery());
    (name, profile)
}

fn classify_device(name: &str, has_battery: bool) -> DeviceProfile {
    let n = name.to_lowercase();
    let has = |keys: &[&str]| keys.iter().any(|k| n.contains(k));
    let words: Vec<&str> = n.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();

    if has(&[
        "headphone", "headset", "наушник", "гарнитур", "earphone", "earbud", "buds", "airpods",
        "hands-free", "hands free", "handsfree", "wh-", "wf-", "beats", "sennheiser", "jabra",
        "quietcomfort", "arctis", "hyperx", "kraken", "audio-technica", "ath-",
    ]) {
        return DeviceProfile::Headphones;
    }
    if words.contains(&"tv")
        || has(&["hdmi", "displayport", "display audio", "телевизор", "monitor", "монитор", "high definition audio device"])
    {
        return DeviceProfile::Tv;
    }
    if has(&["macbook", "built-in", "встроен", "internal speaker"]) {
        return DeviceProfile::Laptop;
    }
    if has(&["soundbar", "саундбар", "колонк", "jbl", "marshall", "sonos", "homepod", "edifier", "line out", "линейн"]) {
        return DeviceProfile::Speakers;
    }
    if has(&["speaker", "динамик"]) {
        // On a laptop "Speakers (Realtek...)" are almost always the built-in ones.
        return if has_battery { DeviceProfile::Laptop } else { DeviceProfile::Speakers };
    }
    DeviceProfile::Universal
}

#[cfg(windows)]
fn has_battery() -> bool {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: plain out-parameter struct.
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return false;
    }
    // 128 = no system battery, 255 = unknown.
    status.BatteryFlag != 128 && status.BatteryFlag != 255
}

#[cfg(target_os = "linux")]
fn has_battery() -> bool {
    std::fs::read_dir("/sys/class/power_supply")
        .map(|dir| {
            dir.flatten().any(|e| {
                std::fs::read_to_string(e.path().join("type"))
                    .map(|t| t.trim() == "Battery")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn has_battery() -> bool {
    false
}

// ── Profile tuning ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct Tuning {
    /// Rumble / excursion filter (Hz).
    highpass_hz: f32,
    low_shelf: (f32, f32),
    /// (freq, gain dB, Q)
    presence: (f32, f32, f32),
    high_shelf: (f32, f32),
    /// AGC target level of the weighted RMS (dBFS).
    target_db: f32,
    comp_threshold_db: f32,
    comp_ratio: f32,
    /// Side channel gain: >1 widens, <1 narrows.
    width: f32,
    /// Headphone crossfeed amount (0 = off).
    crossfeed: f32,
}

impl Tuning {
    fn for_profile(p: DeviceProfile) -> Self {
        match p {
            // Slight warmth, tamed 3 kHz glare, crossfeed against "inside the head" fatigue.
            DeviceProfile::Headphones => Self {
                highpass_hz: 20.0,
                low_shelf: (105.0, 1.5),
                presence: (3200.0, -1.0, 1.0),
                high_shelf: (10000.0, 1.0),
                target_db: -18.0,
                comp_threshold_db: -14.0,
                comp_ratio: 2.0,
                width: 1.0,
                crossfeed: 0.22,
            },
            // Small drivers: cut what they can't play (less distortion), lift upper bass
            // and presence so it sounds fuller and clearer, wider stage, tighter dynamics.
            DeviceProfile::Laptop => Self {
                highpass_hz: 110.0,
                low_shelf: (190.0, 3.0),
                presence: (2600.0, 2.0, 0.9),
                high_shelf: (9000.0, 1.0),
                target_db: -16.0,
                comp_threshold_db: -18.0,
                comp_ratio: 3.0,
                width: 1.25,
                crossfeed: 0.0,
            },
            DeviceProfile::Speakers => Self {
                highpass_hz: 28.0,
                low_shelf: (90.0, 1.0),
                presence: (3000.0, 0.5, 1.0),
                high_shelf: (11000.0, 1.0),
                target_db: -18.0,
                comp_threshold_db: -14.0,
                comp_ratio: 2.0,
                width: 1.1,
                crossfeed: 0.0,
            },
            // TV speakers: vocals forward, controlled bass, even level.
            DeviceProfile::Tv => Self {
                highpass_hz: 50.0,
                low_shelf: (120.0, 1.5),
                presence: (2500.0, 2.5, 0.9),
                high_shelf: (10000.0, 0.5),
                target_db: -17.0,
                comp_threshold_db: -16.0,
                comp_ratio: 2.5,
                width: 1.05,
                crossfeed: 0.0,
            },
            DeviceProfile::Universal => Self {
                highpass_hz: 25.0,
                low_shelf: (100.0, 1.0),
                presence: (3000.0, 0.5, 1.0),
                high_shelf: (10000.0, 1.0),
                target_db: -18.0,
                comp_threshold_db: -14.0,
                comp_ratio: 2.0,
                width: 1.0,
                crossfeed: 0.0,
            },
        }
    }
}

// ── Filters (RBJ cookbook) ───────────────────────────────────────────────────

fn clamp_freq(sample_rate: f32, f: f32) -> f32 {
    f.clamp(10.0, sample_rate * 0.45)
}

fn normalize(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> BiquadCoeffs {
    BiquadCoeffs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
}

fn highpass(sample_rate: f32, freq: f32, q: f32) -> BiquadCoeffs {
    let w0 = 2.0 * std::f32::consts::PI * clamp_freq(sample_rate, freq) / sample_rate;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    normalize((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
}

fn lowpass(sample_rate: f32, freq: f32, q: f32) -> BiquadCoeffs {
    let w0 = 2.0 * std::f32::consts::PI * clamp_freq(sample_rate, freq) / sample_rate;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    normalize((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
}

/// Shelf with slope S = 1.
fn shelf(sample_rate: f32, freq: f32, gain_db: f32, high: bool) -> BiquadCoeffs {
    let a = 10.0_f32.powf(gain_db / 40.0);
    let w0 = 2.0 * std::f32::consts::PI * clamp_freq(sample_rate, freq) / sample_rate;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / 2.0 * std::f32::consts::SQRT_2;
    let k = 2.0 * a.sqrt() * alpha;
    if high {
        normalize(
            a * ((a + 1.0) + (a - 1.0) * cos + k),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - k),
            (a + 1.0) - (a - 1.0) * cos + k,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - k,
        )
    } else {
        normalize(
            a * ((a + 1.0) - (a - 1.0) * cos + k),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - k),
            (a + 1.0) + (a - 1.0) * cos + k,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - k,
        )
    }
}

/// Static soft-knee compressor curve: gain change (dB, <= 0) for a level `x_db`.
fn compressor_gain_db(x_db: f32, threshold_db: f32, ratio: f32, knee_db: f32) -> f32 {
    let over = x_db - threshold_db;
    let slope = 1.0 / ratio - 1.0;
    if over <= -knee_db / 2.0 {
        0.0
    } else if over >= knee_db / 2.0 {
        slope * over
    } else {
        let t = over + knee_db / 2.0;
        slope * t * t / (2.0 * knee_db)
    }
}

fn db_to_lin(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

fn time_coeff(secs: f32, sample_rate: f32) -> f32 {
    1.0 - (-1.0 / (secs * sample_rate)).exp()
}

// ── Source ───────────────────────────────────────────────────────────────────

const TONE_STAGES: usize = 4;
const MAX_CHANNELS: usize = 8;
const AGC_BLOCK: u32 = 64;
const AGC_GATE_DB: f32 = -45.0;
const AGC_MAX_BOOST_DB: f32 = 6.0;
const AGC_MAX_CUT_DB: f32 = -10.0;
/// First seconds of a track adapt quickly, then the gain only drifts slowly.
const AGC_FAST_SECS: f32 = 2.0;
const COMP_KNEE_DB: f32 = 6.0;

pub struct EnhanceSource<S: Source<Item = f32>> {
    source: S,
    shared: Arc<EnhanceShared>,
    channels: usize,
    sample_rate: u32,
    profile: DeviceProfile,
    tuning: Tuning,
    tone: [BiquadCoeffs; TONE_STAGES],
    tone_states: [[BiquadState; MAX_CHANNELS]; TONE_STAGES],
    crossfeed_lp: BiquadCoeffs,
    crossfeed_states: [BiquadState; 2],
    /// Detector weighting (roughly K-weighting: rumble cut + upper-mid lift).
    detect: [BiquadCoeffs; 2],
    detect_states: [BiquadState; 2],
    mean_square: f32,
    /// Detector smoothing per frame during the fast start and afterwards
    /// (precomputed: an `exp()` per sample is the costliest op of the stage).
    ms_coeff_fast: f32,
    ms_coeff_slow: f32,
    fast_frames: u64,
    agc_gain_db: f32,
    agc_gain: f32,
    agc_counter: u32,
    frames_seen: u64,
    envelope: f32,
    /// Envelope below this (linear) is under the compressor knee: no gain change,
    /// so the per-sample log10 is skipped.
    comp_knee_lin: f32,
    attack: f32,
    release: f32,
    enabled: bool,
    /// Dry/wet mix, ramps towards `enabled` to avoid clicks.
    mix: f32,
    mix_step: f32,
    frame: [f32; MAX_CHANNELS],
    out_pos: usize,
    out_len: usize,
    frames_since_check: u32,
}

impl<S: Source<Item = f32>> EnhanceSource<S> {
    pub fn new(source: S, shared: Arc<EnhanceShared>) -> Self {
        let channels = source.channels() as usize;
        let sample_rate = source.sample_rate().max(1);
        let sr = sample_rate as f32;
        let profile = shared.active_profile();
        let tuning = Tuning::for_profile(profile);
        let enabled = shared.enabled.load(Ordering::Relaxed);
        let agc_gain_db = shared.last_gain_db().clamp(AGC_MAX_CUT_DB, AGC_MAX_BOOST_DB);

        let mut s = Self {
            source,
            channels,
            sample_rate,
            profile,
            tuning,
            tone: [BiquadCoeffs { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 }; TONE_STAGES],
            tone_states: [[BiquadState::default(); MAX_CHANNELS]; TONE_STAGES],
            crossfeed_lp: lowpass(sr, 700.0, 0.707),
            crossfeed_states: [BiquadState::default(); 2],
            detect: [highpass(sr, 60.0, 0.5), shelf(sr, 1500.0, 4.0, true)],
            detect_states: [BiquadState::default(); 2],
            // Start consistent with the initial gain so nothing moves until real data arrives.
            mean_square: 10.0_f32.powf((tuning.target_db - agc_gain_db) / 10.0),
            ms_coeff_fast: time_coeff(0.4, sr),
            ms_coeff_slow: time_coeff(3.0, sr),
            fast_frames: (AGC_FAST_SECS * sr) as u64,
            agc_gain_db,
            agc_gain: db_to_lin(agc_gain_db),
            agc_counter: 0,
            frames_seen: 0,
            envelope: 0.0,
            comp_knee_lin: 0.0,
            attack: time_coeff(0.005, sr),
            release: time_coeff(0.12, sr),
            enabled,
            mix: if enabled { 1.0 } else { 0.0 },
            mix_step: 1.0 / (0.15 * sr),
            frame: [0.0; MAX_CHANNELS],
            out_pos: 0,
            out_len: 0,
            frames_since_check: 0,
            shared,
        };
        s.rebuild_tone();
        s
    }

    fn rebuild_tone(&mut self) {
        let sr = self.sample_rate as f32;
        let t = self.tuning;
        self.tone = [
            highpass(sr, t.highpass_hz, 0.707),
            shelf(sr, t.low_shelf.0, t.low_shelf.1, false),
            BiquadCoeffs::peaking_eq(sr, t.presence.0, t.presence.1, t.presence.2),
            shelf(sr, t.high_shelf.0, t.high_shelf.1, true),
        ];
        self.comp_knee_lin = db_to_lin(t.comp_threshold_db - COMP_KNEE_DB / 2.0);
    }

    fn refresh(&mut self) {
        self.enabled = self.shared.enabled.load(Ordering::Relaxed);
        let profile = self.shared.active_profile();
        if profile != self.profile {
            self.profile = profile;
            self.tuning = Tuning::for_profile(profile);
            self.rebuild_tone();
        }
    }

    fn reset_state(&mut self) {
        self.tone_states = [[BiquadState::default(); MAX_CHANNELS]; TONE_STAGES];
        self.crossfeed_states = [BiquadState::default(); 2];
        self.envelope = 0.0;
    }

    fn update_agc(&mut self) {
        let sr = self.sample_rate as f32;
        let fast = self.frames_seen < self.fast_frames;
        let level_db = 10.0 * self.mean_square.max(1.0e-10).log10();
        if level_db < AGC_GATE_DB {
            // Silence / fades: hold the gain instead of pumping it up.
            return;
        }
        let desired = (self.tuning.target_db - level_db).clamp(AGC_MAX_CUT_DB, AGC_MAX_BOOST_DB);
        let diff = desired - self.agc_gain_db;
        // dB per second; turning down is quicker than turning up.
        let rate = match (fast, diff < 0.0) {
            (true, _) => 12.0,
            (false, true) => 4.0,
            (false, false) => 1.0,
        };
        let max_step = rate * AGC_BLOCK as f32 / sr;
        self.agc_gain_db += diff.clamp(-max_step, max_step);
        self.agc_gain = db_to_lin(self.agc_gain_db);
        self.shared.store_last_gain_db(self.agc_gain_db);
    }

    fn process_frame(&mut self) {
        self.frames_since_check += 1;
        if self.frames_since_check >= 1024 {
            self.frames_since_check = 0;
            self.refresh();
        }

        let target = if self.enabled { 1.0 } else { 0.0 };
        if self.mix == 0.0 && target == 0.0 {
            return;
        }
        if self.mix < target {
            self.mix = (self.mix + self.mix_step).min(1.0);
        } else if self.mix > target {
            self.mix = (self.mix - self.mix_step).max(0.0);
            if self.mix == 0.0 {
                self.reset_state();
            }
        }

        let ch = self.channels;
        let dry = self.frame;
        let mut wet = self.frame;
        let t = self.tuning;

        for (c, x) in wet.iter_mut().enumerate().take(ch) {
            for stage in 0..TONE_STAGES {
                *x = self.tone_states[stage][c].process(&self.tone[stage], *x);
            }
        }

        if ch == 2 {
            let (mut l, mut r) = (wet[0], wet[1]);
            if t.crossfeed > 0.0 {
                let from_r = self.crossfeed_states[0].process(&self.crossfeed_lp, r);
                let from_l = self.crossfeed_states[1].process(&self.crossfeed_lp, l);
                let norm = 1.0 / (1.0 + t.crossfeed);
                l = (l + t.crossfeed * from_r) * norm;
                r = (r + t.crossfeed * from_l) * norm;
            }
            if t.width != 1.0 {
                let mid = (l + r) * 0.5;
                let side = (l - r) * 0.5 * t.width;
                l = mid + side;
                r = mid - side;
            }
            wet[0] = l;
            wet[1] = r;
        }

        // Loudness detector + AGC
        let mono = wet[..ch].iter().sum::<f32>() / ch as f32;
        let d0 = self.detect_states[0].process(&self.detect[0], mono);
        let d = self.detect_states[1].process(&self.detect[1], d0);
        let ms_coeff = if self.frames_seen < self.fast_frames { self.ms_coeff_fast } else { self.ms_coeff_slow };
        self.mean_square += (d * d - self.mean_square) * ms_coeff;
        self.frames_seen += 1;
        self.agc_counter += 1;
        if self.agc_counter >= AGC_BLOCK {
            self.agc_counter = 0;
            self.update_agc();
        }

        // Compressor (stereo-linked peak envelope)
        let mut peak = 0.0_f32;
        for x in wet.iter_mut().take(ch) {
            *x *= self.agc_gain;
            peak = peak.max(x.abs());
        }
        let coeff = if peak > self.envelope { self.attack } else { self.release };
        self.envelope += (peak - self.envelope) * coeff;
        let gr = if self.envelope <= self.comp_knee_lin {
            1.0
        } else {
            let env_db = 20.0 * self.envelope.max(1.0e-6).log10();
            let gr_db = compressor_gain_db(env_db, t.comp_threshold_db, t.comp_ratio, COMP_KNEE_DB);
            if gr_db < -0.01 { db_to_lin(gr_db) } else { 1.0 }
        };

        let mix = self.mix;
        for c in 0..ch {
            let w = soft_clip(wet[c] * gr);
            self.frame[c] = dry[c] * (1.0 - mix) + w * mix;
        }
    }
}

impl<S: Source<Item = f32>> Iterator for EnhanceSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.out_pos < self.out_len {
            let s = self.frame[self.out_pos];
            self.out_pos += 1;
            return Some(s);
        }
        // Unusual layouts (>8 channels) are passed through untouched.
        if self.channels == 0 || self.channels > MAX_CHANNELS {
            return self.source.next();
        }

        let mut n = 0;
        while n < self.channels {
            match self.source.next() {
                Some(s) => {
                    self.frame[n] = s;
                    n += 1;
                }
                None => break,
            }
        }
        if n == 0 {
            return None;
        }
        if n == self.channels {
            self.process_frame();
        }
        self.out_len = n;
        self.out_pos = 1;
        Some(self.frame[0])
    }
}

impl<S: Source<Item = f32>> Source for EnhanceSource<S> {
    fn current_frame_len(&self) -> Option<usize> { self.source.current_frame_len() }
    fn channels(&self) -> u16 { self.channels as u16 }
    fn sample_rate(&self) -> u32 { self.sample_rate }
    fn total_duration(&self) -> Option<Duration> { self.source.total_duration() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;

    fn stereo_sine(amp: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (secs * sr as f32) as usize;
        (0..n)
            .flat_map(|i| {
                let v = amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin();
                [v, v]
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn run(enabled: bool, input: Vec<f32>) -> Vec<f32> {
        let shared = Arc::new(EnhanceShared::new());
        shared.apply(&EnhanceSettings { enabled, device: Some(DeviceProfile::Universal) });
        EnhanceSource::new(SamplesBuffer::new(2, 44_100, input), shared).collect()
    }

    #[test]
    fn disabled_is_exact_passthrough() {
        let input = stereo_sine(0.3, 0.5, 44_100);
        assert_eq!(run(false, input.clone()), input);
    }

    #[test]
    fn keeps_sample_count() {
        let mut input = stereo_sine(0.3, 0.2, 44_100);
        input.push(0.1); // dangling half frame
        assert_eq!(run(true, input.clone()).len(), input.len());
    }

    #[test]
    fn agc_brings_quiet_and_loud_tracks_closer() {
        let quiet = run(true, stereo_sine(0.03, 6.0, 44_100));
        let loud = run(true, stereo_sine(0.6, 6.0, 44_100));
        let tail = |v: &Vec<f32>| rms(&v[v.len() - 44_100..]);
        let before = 20.0 * (0.6_f32 / 0.03).log10();
        let after = 20.0 * (tail(&loud) / tail(&quiet)).log10();
        assert!(after < before - 8.0, "difference {before} dB -> {after} dB");
        assert!(loud.iter().all(|s| s.abs() < 1.0));
    }

    #[test]
    fn compressor_curve() {
        assert_eq!(compressor_gain_db(-30.0, -14.0, 2.0, 6.0), 0.0);
        assert!((compressor_gain_db(-4.0, -14.0, 2.0, 6.0) + 5.0).abs() < 1e-4);
        let knee = compressor_gain_db(-14.0, -14.0, 2.0, 6.0);
        assert!(knee < 0.0 && knee > -1.0);
    }

    #[test]
    fn classifies_common_device_names() {
        assert_eq!(classify_device("Headphones (Realtek(R) Audio)", true), DeviceProfile::Headphones);
        assert_eq!(classify_device("Наушники (WH-1000XM4)", false), DeviceProfile::Headphones);
        assert_eq!(classify_device("Speakers (Realtek(R) Audio)", true), DeviceProfile::Laptop);
        assert_eq!(classify_device("Динамики (Realtek(R) Audio)", false), DeviceProfile::Speakers);
        assert_eq!(classify_device("LG TV (NVIDIA High Definition Audio)", false), DeviceProfile::Tv);
        assert_eq!(classify_device("MacBook Pro Speakers", true), DeviceProfile::Laptop);
        assert_eq!(classify_device("USB DAC", false), DeviceProfile::Universal);
    }

    #[test]
    fn profile_roundtrip() {
        for p in DeviceProfile::ALL {
            assert_eq!(DeviceProfile::from_u8(p.to_u8()), Some(p));
        }
        assert_eq!(DeviceProfile::from_u8(0), None);
    }
}
