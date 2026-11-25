use base64::Engine;

use crate::enhance::{EnhanceSettings, EnhanceShared, EnhanceSource, EnhanceStatus};
use crate::fx::{FxSettings, FxShared, FxSource, SpeedSource};
use crate::spectrum::SpectrumAnalyzer;
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::probe::Probe;
use rodio::{ChannelCount, MixerDeviceSink, Player, SampleRate, Source};

/// rodio wants non-zero channel counts and sample rates; our sources keep plain ints.
pub(crate) fn nz_channels(c: u16) -> ChannelCount {
    ChannelCount::new(c).unwrap_or(ChannelCount::MIN)
}
pub(crate) fn nz_rate(r: u32) -> SampleRate {
    SampleRate::new(r).unwrap_or(SampleRate::MIN)
}
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_ALAC, CODEC_ID_FLAC, CODEC_ID_PCM_F32LE, CODEC_ID_PCM_S16BE, CODEC_ID_PCM_S16LE,
    CODEC_ID_PCM_S24LE, CODEC_ID_PCM_S32LE,
};
use symphonia::core::codecs::audio::{
    AudioCodecId, AudioCodecParameters, AudioDecoder, AudioDecoderOptions, CODEC_ID_NULL_AUDIO,
};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::packet::Packet;
use symphonia::core::units::Time;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub duration_secs: Option<f64>,
    pub format: String,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub bitrate: Option<u32>,
    pub is_lossless: bool,
    pub file_path: String,
    /// Data URL (data:image/jpeg;base64,...)
    pub cover_art: Option<String>,
    /// Hex (#RRGGBB)
    pub accent_color: Option<String>,
    pub bpm: Option<f32>,
    pub loudness: Option<f32>,
    pub energy: Option<f32>,
    pub danceability: Option<f32>,
    pub spectral_centroid: Option<f32>,
    pub key: Option<String>,
    pub audio_fingerprint: Option<String>,
    /// Embedded cover present; cached in `tracks.has_cover`. Internal only.
    #[serde(skip)]
    pub has_cover: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Lightweight status for frequent polling (no full track metadata)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStatus {
    pub state: PlaybackState,
    pub volume: f32,
    pub position_secs: f64,
    pub current_file_path: Option<String>,
}

/// 10-band EQ center frequencies (Hz)
pub const EQ_BANDS: [f32; 10] = [60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0];

/// Shared between AudioPlayer and EqSource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqSettings {
    pub enabled: bool,
    /// Per-band gain in dB (-12..+12)
    pub bands: Vec<f32>,
    /// Preamp in dB (-12..+12)
    pub gain: f32,
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bands: vec![0.0; 10],
            gain: 0.0,
        }
    }
}

/// Biquad coefficients (Direct Form I)
#[derive(Debug, Clone, Copy)]
pub(crate) struct BiquadCoeffs {
    pub(crate) b0: f32, pub(crate) b1: f32, pub(crate) b2: f32,
    pub(crate) a1: f32, pub(crate) a2: f32,
}

impl BiquadCoeffs {
    pub(crate) fn peaking_eq(sample_rate: f32, center_freq: f32, gain_db: f32, q: f32) -> Self {
        if gain_db.abs() < 0.01 {
            return Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };
        }

        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * center_freq / sample_rate;
        let sin_w0 = w0.sin();
        let cos_w0 = w0.cos();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BiquadState {
    x1: f32, x2: f32,
    y1: f32, y2: f32,
}

/// Soft limiter: passthrough below 0.97, tanh toward CEILING above. CEILING < 1.0, so a sample
/// never hits +-1.0.
#[inline(always)]
pub(crate) fn soft_clip(x: f32) -> f32 {
    const T: f32 = 0.97;
    const CEILING: f32 = 0.999;
    const KNEE: f32 = 0.5;
    let a = x.abs();
    if a <= T {
        x
    } else {
        let sign = if x >= 0.0 { 1.0 } else { -1.0 };
        let over = (a - T) / KNEE;
        sign * (T + (CEILING - T) * over.tanh())
    }
}

impl BiquadState {
    pub(crate) fn process(&mut self, coeffs: &BiquadCoeffs, input: f32) -> f32 {
        let output = coeffs.b0 * input + coeffs.b1 * self.x1 + coeffs.b2 * self.x2
            - coeffs.a1 * self.y1 - coeffs.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

/// Applies the 10-band parametric EQ in real time, reading settings from a shared lock.
struct EqSource<S: Source<Item = f32>> {
    source: S,
    eq_settings: Arc<RwLock<EqSettings>>,
    coeffs: Vec<BiquadCoeffs>,
    /// [band][channel]
    states: Vec<Vec<BiquadState>>,
    /// Cached to detect settings changes
    cached_bands: Vec<f32>,
    cached_enabled: bool,
    cached_gain_db: f32,
    /// Linear multiplier
    cached_gain_linear: f32,
    /// Counts down to the next settings-change check
    samples_since_check: usize,
    /// Interleaved channel counter: 0, 1, 0, 1, ...
    current_channel: usize,
    channels: u16,
    sample_rate: u32,
}

impl<S: Source<Item = f32>> EqSource<S> {
    fn new(source: S, eq_settings: Arc<RwLock<EqSettings>>) -> Self {
        let channels = source.channels().get();
        let sample_rate = source.sample_rate().get();
        let num_bands = EQ_BANDS.len();

        let settings = eq_settings.read().unwrap();
        let bands = settings.bands.clone();
        let enabled = settings.enabled;
        let gain_db = settings.gain;
        drop(settings);

        let coeffs: Vec<BiquadCoeffs> = EQ_BANDS.iter().enumerate().map(|(i, &freq)| {
            let gain = bands.get(i).copied().unwrap_or(0.0);
            BiquadCoeffs::peaking_eq(sample_rate as f32, freq, gain, 1.4)
        }).collect();

        let states = vec![vec![BiquadState::default(); channels as usize]; num_bands];

        Self {
            source,
            eq_settings,
            coeffs,
            states,
            cached_bands: bands,
            cached_enabled: enabled,
            cached_gain_db: gain_db,
            cached_gain_linear: 10.0_f32.powf(gain_db / 20.0),
            samples_since_check: 0,
            current_channel: 0,
            channels,
            sample_rate,
        }
    }

    fn maybe_update_coeffs(&mut self) {
        let settings = match self.eq_settings.try_read() {
            Ok(s) => s,
            Err(_) => return,
        };

        if settings.enabled == self.cached_enabled && settings.bands == self.cached_bands && settings.gain == self.cached_gain_db {
            return;
        }

        self.cached_enabled = settings.enabled;
        self.cached_bands = settings.bands.clone();
        self.cached_gain_db = settings.gain;
        self.cached_gain_linear = 10.0_f32.powf(settings.gain / 20.0);
        drop(settings);

        for (i, &freq) in EQ_BANDS.iter().enumerate() {
            let gain = self.cached_bands.get(i).copied().unwrap_or(0.0);
            self.coeffs[i] = BiquadCoeffs::peaking_eq(self.sample_rate as f32, freq, gain, 1.4);
        }
    }
}

impl<S: Source<Item = f32>> Iterator for EqSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.source.next()?;

        let ch = self.current_channel;
        self.current_channel += 1;
        if self.current_channel >= self.channels as usize {
            self.current_channel = 0;
        }

        self.samples_since_check += 1;
        if self.samples_since_check >= 1024 {
            self.samples_since_check = 0;
            self.maybe_update_coeffs();
        }

        if !self.cached_enabled {
            return Some(sample);
        }

        let mut out = sample * self.cached_gain_linear;
        for (band_idx, coeffs) in self.coeffs.iter().enumerate() {
            out = self.states[band_idx][ch].process(coeffs, out);
        }

        Some(soft_clip(out))
    }
}

impl<S: Source<Item = f32>> Source for EqSource<S> {
    fn current_span_len(&self) -> Option<usize> { self.source.current_span_len() }
    fn channels(&self) -> ChannelCount { nz_channels(self.channels) }
    fn sample_rate(&self) -> SampleRate { nz_rate(self.sample_rate) }
    fn total_duration(&self) -> Option<Duration> { self.source.total_duration() }
}

/// Speed, user EQ, creative effects (crusher / vinyl / space), then auto enhancement.
fn effects<S: Source<Item = f32>>(
    source: S,
    eq_settings: &Arc<RwLock<EqSettings>>,
    enhance: &Arc<EnhanceShared>,
    fx: &Arc<FxShared>,
) -> EnhanceSource<FxSource<EqSource<SpeedSource<S>>>> {
    let sped = SpeedSource::new(source, fx.clone());
    let eq = EqSource::new(sped, eq_settings.clone());
    EnhanceSource::new(FxSource::new(eq, fx.clone()), enhance.clone())
}

type Resp = mpsc::SyncSender<Result<(), String>>;

enum AudioCmd {
    PlayFrom {
        file_path: String,
        seek_secs: f64,
        volume: f32,
        resp: Resp,
    },
    Pause,
    Resume {
        file_path: Option<String>,
        position_secs: f64,
        volume: f32,
    },
    Stop,
    SetVolume(f32),
    Seek {
        file_path: String,
        position_secs: f64,
        volume: f32,
        was_paused: bool,
        resp: Resp,
    },
    SeekCrossfade {
        file_path: String,
        position_secs: f64,
        current_pos: f64,
        volume: f32,
        was_paused: bool,
        crossfade_secs: f64,
        resp: Resp,
    },
    PlayCrossfade {
        mix_profile: Option<MixProfile>,
        file_path: String,
        seek_secs: f64,
        volume: f32,
        crossfade_secs: f64,
        resp: Resp,
    },
}

/// Opens `file_path` and returns symphonia's reader for its container.
pub(crate) fn probe_reader(file_path: &str) -> Result<Box<dyn FormatReader>, String> {
    let file = File::open(file_path).map_err(|e| format!("Не удалось открыть файл: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(file_path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("Не удалось определить формат: {}", e))
}

/// Audio tracks of a container, in file order (video, subtitle and unknown tracks skipped).
fn audio_tracks(reader: &dyn FormatReader) -> Vec<(u32, AudioCodecParameters, Option<u64>)> {
    reader
        .tracks()
        .iter()
        .filter_map(|t| {
            let params = t.codec_params.as_ref()?.audio()?;
            (params.codec != CODEC_ID_NULL_AUDIO).then(|| (t.id, params.clone(), t.num_frames))
        })
        .collect()
}

/// A file opened for decoding: its reader, the chosen audio track and a decoder for it.
pub(crate) struct OpenedAudio {
    pub reader: Box<dyn FormatReader>,
    pub decoder: Box<dyn AudioDecoder>,
    pub track_id: u32,
    pub sample_rate: u32,
    pub channels: u16,
    /// Playable frames of the track, when the container states it.
    pub num_frames: Option<u64>,
}

impl OpenedAudio {
    pub(crate) fn open(file_path: &str) -> Result<Self, String> {
        let reader = probe_reader(file_path)?;
        let mut last_err = "Не найден аудио трек".to_string();
        for (track_id, params, num_frames) in audio_tracks(&*reader) {
            match symphonia::default::get_codecs()
                .make_audio_decoder(&params, &AudioDecoderOptions::default())
            {
                Ok(decoder) => {
                    return Ok(Self {
                        reader,
                        decoder,
                        track_id,
                        sample_rate: params.sample_rate.unwrap_or(44_100).max(1),
                        channels: params.channels.as_ref().map(|c| c.count() as u16).unwrap_or(2).max(1),
                        num_frames,
                    });
                }
                Err(e) => last_err = format!("Не удалось создать декодер: {}", e),
            }
        }
        Err(last_err)
    }

    /// Seeks the track to `secs` and resets the decoder.
    pub(crate) fn seek(&mut self, secs: f64, mode: SeekMode) -> Result<(), String> {
        let time = Time::try_from_secs_f64(secs.max(0.0)).unwrap_or(Time::ZERO);
        self.reader
            .seek(mode, SeekTo::Time { time, track_id: Some(self.track_id) })
            .map_err(|e| format!("Ошибка seek: {}", e))?;
        self.decoder.reset();
        Ok(())
    }

    /// Next packet of the chosen track; `None` at the end of the stream or on a read error.
    pub(crate) fn next_packet(&mut self) -> Option<Packet> {
        loop {
            match self.reader.next_packet() {
                Ok(Some(packet)) if packet.track_id == self.track_id => return Some(packet),
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => return None,
            }
        }
    }

    /// Decodes the next packet into `out` as interleaved f32 (replacing its content).
    pub(crate) fn decode_next(&mut self, out: &mut Vec<f32>) -> bool {
        while let Some(packet) = self.next_packet() {
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    decoded.copy_to_vec_interleaved(out);
                    if !out.is_empty() {
                        return true;
                    }
                }
                Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                Err(_) => return false,
            }
        }
        false
    }
}

struct SymphoniaSource {
    audio: OpenedAudio,
    sample_rate: u32,
    channels: u16,
    current_samples: Vec<f32>,
    current_pos: usize,
}

impl SymphoniaSource {
    fn new(file_path: &str) -> Result<Self, String> {
        let audio = OpenedAudio::open(file_path)?;
        Ok(Self {
            sample_rate: audio.sample_rate,
            channels: audio.channels,
            audio,
            current_samples: Vec::new(),
            current_pos: 0,
        })
    }

    fn new_with_seek(file_path: &str, seek_secs: f64) -> Result<Self, String> {
        let mut source = Self::new(file_path)?;
        if seek_secs > 0.0 {
            source.audio.seek(seek_secs, SeekMode::Accurate)?;
        }
        Ok(source)
    }

    fn decode_next_packet(&mut self) -> bool {
        self.current_pos = 0;
        self.audio.decode_next(&mut self.current_samples)
    }
}

impl Iterator for SymphoniaSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.current_pos >= self.current_samples.len()
            && !self.decode_next_packet() {
                return None;
            }
        if self.current_pos < self.current_samples.len() {
            let sample = self.current_samples[self.current_pos];
            self.current_pos += 1;
            Some(sample)
        } else {
            None
        }
    }
}

impl Source for SymphoniaSource {
    fn current_span_len(&self) -> Option<usize> {
        let remaining = self.current_samples.len() - self.current_pos;
        if remaining > 0 { Some(remaining) } else { None }
    }
    fn channels(&self) -> ChannelCount { nz_channels(self.channels) }
    fn sample_rate(&self) -> SampleRate { nz_rate(self.sample_rate) }
    fn total_duration(&self) -> Option<Duration> { None }
}

/// Ring-buffer depth in decoded packet-blocks (FLAC ~4096 frames, MP3 ~1152, Opus ~960 per block).
const DECODE_QUEUE_BLOCKS: usize = 64;

/// Pipelined source: decoding runs on an `audio-decode` thread into a bounded queue. Dropping the
/// source closes `rx`, so the thread exits on its own.
struct BufferedSymphoniaSource {
    sample_rate: u32,
    channels: u16,
    rx: mpsc::Receiver<Vec<f32>>,
    recycle: mpsc::SyncSender<Vec<f32>>,
    current: Vec<f32>,
    pos: usize,
}

impl BufferedSymphoniaSource {
    fn new_with_seek(file_path: &str, seek_secs: f64) -> Result<Self, String> {
        let mut inner = SymphoniaSource::new_with_seek(file_path, seek_secs)?;
        let sample_rate = inner.sample_rate;
        let channels = inner.channels;

        let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(DECODE_QUEUE_BLOCKS);
        let (recycle, recycled) = mpsc::sync_channel::<Vec<f32>>(DECODE_QUEUE_BLOCKS);

        std::thread::Builder::new()
            .name("audio-decode".into())
            .spawn(move || {
                loop {
                    if let Ok(buf) = recycled.try_recv() {
                        inner.current_samples = buf;
                    }
                    if !inner.decode_next_packet() {
                        break;
                    }
                    let block = std::mem::take(&mut inner.current_samples);
                    if block.is_empty() {
                        continue;
                    }
                    if tx.send(block).is_err() {
                        return;
                    }
                }
            })
            .map_err(|e| format!("Не удалось запустить поток декодирования: {}", e))?;

        Ok(Self {
            sample_rate,
            channels,
            rx,
            recycle,
            current: Vec::new(),
            pos: 0,
        })
    }
}

impl Iterator for BufferedSymphoniaSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.pos >= self.current.len() {
            match self.rx.recv() {
                Ok(block) => {
                    let spent = std::mem::replace(&mut self.current, block);
                    if spent.capacity() > 0 {
                        let _ = self.recycle.try_send(spent);
                    }
                    self.pos = 0;
                }
                Err(_) => return None,
            }
        }
        let sample = self.current[self.pos];
        self.pos += 1;
        Some(sample)
    }
}

impl Source for BufferedSymphoniaSource {
    fn current_span_len(&self) -> Option<usize> {
        let remaining = self.current.len() - self.pos;
        if remaining > 0 { Some(remaining) } else { None }
    }
    fn channels(&self) -> ChannelCount { nz_channels(self.channels) }
    fn sample_rate(&self) -> SampleRate { nz_rate(self.sample_rate) }
    fn total_duration(&self) -> Option<Duration> { None }
}

struct FadeOut<S: Source<Item = f32>> {
    source: S,
    total_samples: usize,
    elapsed: usize,
}

impl<S: Source<Item = f32>> FadeOut<S> {
    fn new(source: S, duration: Duration) -> Self {
        let total_samples = (source.sample_rate().get() as f64 * duration.as_secs_f64()) as usize
            * source.channels().get() as usize;
        Self { source, total_samples, elapsed: 0 }
    }
}

impl<S: Source<Item = f32>> Iterator for FadeOut<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.elapsed >= self.total_samples {
            return None;
        }
        let gain = 1.0 - self.elapsed as f32 / self.total_samples as f32;
        let sample = self.source.next()? * gain;
        self.elapsed += 1;
        Some(sample)
    }
}

impl<S: Source<Item = f32>> Source for FadeOut<S> {
    fn current_span_len(&self) -> Option<usize> { self.source.current_span_len() }
    fn channels(&self) -> ChannelCount { self.source.channels() }
    fn sample_rate(&self) -> SampleRate { self.source.sample_rate() }
    fn total_duration(&self) -> Option<Duration> { None }
}

type MixPoints = [[f32; 2]; 3];

#[derive(Clone, Copy, Deserialize, Default)]
struct LaneCurves {
    volume: Option<MixPoints>,
    eq: Option<MixPoints>,
    filter: Option<MixPoints>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MixProfile {
    #[serde(default)]
    volume_curve: Option<MixVolume>,
    curves: Option<Box<[LaneCurves; 2]>>,
    equal_power: bool,
    eq_band: MixBand,
    filter: MixFilter,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum MixFilter { Off, LowPass, HighPass }

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum MixBand { Off, Bass, CenterBass, Mids, Highs }

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum MixVolume { Linear, Smooth, Overlap, Rise, Cut }

impl MixProfile {
    fn gain(&self, weight: f32) -> f32 {
        match self.volume_curve {
            Some(MixVolume::Overlap) => (weight * 8.0).min(1.0),
            Some(MixVolume::Rise) => weight * weight,
            Some(MixVolume::Cut) => if weight >= 0.5 { 1.0 } else { 0.0 },
            Some(MixVolume::Linear) => weight,
            None if !self.equal_power => weight,
            _ => (weight * std::f32::consts::FRAC_PI_2).sin(),
        }
    }
}

fn curve_value(points: &Option<MixPoints>, time: f32, fallback: f32) -> f32 {
    let Some(points) = points else { return fallback };
    let [a, b, c] = points;
    if time < a[0] { return a[1].clamp(0.0, 1.0); }
    if time >= c[0] { return c[1].clamp(0.0, 1.0); }
    // Equal timestamps make a step. Interpolate only across a non-empty interval.
    let (left, right) = if time < b[0] { (a, b) } else { (b, c) };
    let t = (time - left[0]) / (right[0] - left[0]);
    (left[1] + (right[1] - left[1]) * t).clamp(0.0, 1.0)
}

struct MixEnvelope {
    curves: LaneCurves,
    eq_amount: f32,
    profile: MixProfile,
    incoming: bool,
    frames: usize,
    frame: usize,
    channel: usize,
    low: Vec<f32>,
    mid_lowpass: Vec<f32>,
    mid_alpha: f32,
    filtered: Vec<f32>,
    bass_alpha: f32,
    rate: f32,
    gain: f32,
    weight: f32,
    alpha: f32,
}

impl MixEnvelope {
    fn new(profile: MixProfile, incoming: bool, seconds: f64, channels: u16, rate: u32) -> Self {
        let rate = rate as f32;
        Self {
            curves: profile.curves.as_ref().map(|lanes| lanes[incoming as usize]).unwrap_or_default(),
            eq_amount: 0.0, profile, incoming, frames: ((seconds * rate as f64) as usize).max(1),
            frame: 0, channel: 0, low: vec![0.0; channels as usize],
            mid_lowpass: vec![0.0; channels as usize],
            mid_alpha: 1.0 - (-std::f32::consts::TAU * 2500.0 / rate).exp(),
            filtered: vec![0.0; channels as usize], rate,
            bass_alpha: 1.0 - (-std::f32::consts::TAU * 180.0 / rate).exp(),
            gain: 0.0, weight: 0.0, alpha: 0.0,
        }
    }

    fn sample(&mut self, mut x: f32) -> f32 {
        let ch = self.channel;
        if ch == 0 {
            let progress = self.frame as f32 / self.frames as f32;
            self.weight = if self.incoming { progress } else { 1.0 - progress };
            self.gain = curve_value(&self.curves.volume, progress, self.profile.gain(self.weight));
            let band_level = if matches!(self.profile.eq_band, MixBand::CenterBass) {
                (self.weight * 10.0 - 4.5).clamp(0.0, 1.0)
            } else { self.weight };
            self.eq_amount = 1.0 - curve_value(&self.curves.eq, progress, band_level);
            if !matches!(self.profile.filter, MixFilter::Off) {
                let high_pass = matches!(self.profile.filter, MixFilter::HighPass);
                let level = if high_pass { 1.0 - self.weight } else { self.weight };
                let level = curve_value(&self.curves.filter, progress, level);
                self.weight = if high_pass { 1.0 - level } else { level };
                let cutoff = match self.profile.filter {
                    MixFilter::LowPass => 250.0 * 64.0_f32.powf(self.weight),
                    _ => 30.0 * 64.0_f32.powf(1.0 - self.weight),
                };
                self.alpha = 1.0 - (-std::f32::consts::TAU * cutoff / self.rate).exp();
            }
        }
        if !matches!(self.profile.eq_band, MixBand::Off) {
            self.low[ch] += self.bass_alpha * (x - self.low[ch]);
            self.mid_lowpass[ch] += self.mid_alpha * (x - self.mid_lowpass[ch]);
            let band = match self.profile.eq_band {
                MixBand::Bass | MixBand::CenterBass => self.low[ch],
                MixBand::Mids => self.mid_lowpass[ch] - self.low[ch],
                MixBand::Highs => x - self.mid_lowpass[ch],
                MixBand::Off => 0.0,
            };
            x -= band * self.eq_amount;
        }
        if !matches!(self.profile.filter, MixFilter::Off) {
            self.filtered[ch] += self.alpha * (x - self.filtered[ch]);
            let wet = match self.profile.filter {
                MixFilter::LowPass => self.filtered[ch],
                _ => x - self.filtered[ch],
            };
            x += (wet - x) * (1.0 - self.weight);
        }
        self.channel += 1;
        if self.channel == self.low.len() {
            self.channel = 0;
            self.frame += 1;
        }
        x * self.gain
    }
}

pub(crate) fn track_waveform(path: &str, duration: f64) -> Result<Vec<u8>, String> {
    let mut audio = OpenedAudio::open(path)?;
    let frames = audio.num_frames.filter(|n| *n > 0)
        .unwrap_or((duration * audio.sample_rate as f64) as u64).max(1);
    // Interleaved low, mid and high peak envelopes for each time bin.
    let mut peaks = vec![0u8; 8192 * 3];
    let mut low = vec![0.0; audio.channels as usize];
    let mut mid = low.clone();
    let alpha = |hz: f32| 1.0 - (-std::f32::consts::TAU * hz / audio.sample_rate as f32).exp();
    let bass_alpha = alpha(180.0);
    let mid_alpha = alpha(2500.0);
    let mut frame = 0u64;
    let mut samples = Vec::new();
    while audio.decode_next(&mut samples) {
        for channels in samples.chunks(audio.channels as usize) {
            let bin = (frame * 8192 / frames).min(8191) as usize * 3;
            for (ch, &sample) in channels.iter().enumerate() {
                low[ch] += bass_alpha * (sample - low[ch]);
                mid[ch] += mid_alpha * (sample - mid[ch]);
                for (band, value) in [low[ch], mid[ch] - low[ch], sample - mid[ch]].iter().enumerate() {
                    peaks[bin + band] = peaks[bin + band].max((value.abs().sqrt() * 255.0) as u8);
                }
            }
            frame += 1;
        }
    }
    if frame == 0 { return Err("Empty audio stream".into()); }
    Ok(peaks)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviewRegion {
    from_path: String,
    to_path: String,
    from_start: f64,
    to_start: f64,
    lead: f64,
    overlap: f64,
    tail: f64,
    profile: MixProfile,
}

pub(crate) fn render_mix_preview(region: PreviewRegion) -> Result<Vec<u8>, String> {
    use rodio::source::UniformSourceIterator;
    const RATE: u32 = 32000;
    let open = |path: &str, start: f64| -> Result<_, String> {
        let source = SymphoniaSource::new_with_seek(path, start)?;
        Ok(UniformSourceIterator::new(source, nz_channels(2), nz_rate(RATE)))
    };
    let mut outgoing = open(&region.from_path, region.from_start)?;
    let mut incoming = open(&region.to_path, region.to_start)?;
    let overlap = region.overlap.clamp(0.0, 32.0);
    let frames = |secs: f64| (secs * RATE as f64).round() as usize;
    let start = frames(region.lead.clamp(0.0, 3.0));
    let end = start + frames(overlap);
    let total = end + frames(region.tail.clamp(0.0, 3.0));
    let mut fade_out = MixEnvelope::new(region.profile.clone(), false, overlap, 2, RATE);
    let mut fade_in = MixEnvelope::new(region.profile, true, overlap, 2, RATE);
    let mut pcm = Vec::with_capacity(total * 4);
    for frame in 0..total {
        for _ in 0..2 {
            let sample = if frame < start { outgoing.next().unwrap_or(0.0) }
                else if frame >= end { incoming.next().unwrap_or(0.0) }
                else { fade_out.sample(outgoing.next().unwrap_or(0.0)) +
                       fade_in.sample(incoming.next().unwrap_or(0.0)) };
            pcm.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    Ok(pcm)
}

type FadeRequest = Arc<OnceLock<(MixProfile, f64)>>;

/// Atomic "sink empty" flag for lock-free status reads, plus a condvar for the track-end watcher.
pub struct SinkEndSignal {
    empty: AtomicBool,
    lock: Mutex<()>,
    cvar: Condvar,
}

impl SinkEndSignal {
    fn new(initially_empty: bool) -> Self {
        Self {
            empty: AtomicBool::new(initially_empty),
            lock: Mutex::new(()),
            cvar: Condvar::new(),
        }
    }

    fn set_playing(&self) {
        self.empty.store(false, Ordering::Relaxed);
    }

    /// Marks the sink empty and wakes anyone blocked in `wait`.
    fn set_finished(&self) {
        self.empty.store(true, Ordering::Relaxed);
        let guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.cvar.notify_all();
        drop(guard);
    }

    fn is_finished(&self) -> bool {
        self.empty.load(Ordering::Relaxed)
    }

    /// Blocks until `set_finished` or `timeout` (safety net against a lost wakeup).
    pub fn wait(&self, timeout: Duration) {
        let guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.cvar.wait_timeout(guard, timeout);
    }
}

/// Transparent tap feeding mono blocks to the spectrum analyzer; placed above EQ so the spectrum
/// shows what is audible.
struct AnalyzerSource<S: Source<Item = f32>> {
    source: S,
    analyzer: Arc<SpectrumAnalyzer>,
    finished: Arc<SinkEndSignal>,
    channels: u16,
    sample_rate: u32,
    ch_idx: u16,
    outgoing: FadeRequest,
    envelope: Option<MixEnvelope>,
    frame_sum: f32,
    block: Vec<f32>,
}

impl<S: Source<Item = f32>> AnalyzerSource<S> {
    const BLOCK: usize = 256;

    fn new(source: S, analyzer: Arc<SpectrumAnalyzer>, finished: Arc<SinkEndSignal>, outgoing: FadeRequest) -> Self {
        let channels = source.channels().get();
        let sample_rate = source.sample_rate().get();
        Self {
            source,
            analyzer,
            finished,
            channels,
            sample_rate,
            outgoing,
            envelope: None,
            ch_idx: 0,
            frame_sum: 0.0,
            block: Vec::with_capacity(Self::BLOCK),
        }
    }
}

impl<S: Source<Item = f32>> Iterator for AnalyzerSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if let Some((profile, seconds)) = self.outgoing.get()
            && self.envelope.as_ref().is_none_or(|e| e.incoming) {
            self.envelope = Some(MixEnvelope::new(profile.clone(), false, *seconds, self.channels, self.sample_rate));
        }
        if self.envelope.as_ref().is_some_and(|e| !e.incoming && e.frame >= e.frames) {
            return None;
        }
        let mut sample = match self.source.next() {
            Some(s) => s,
            None => {
                if self.outgoing.get().is_none() { self.finished.set_finished(); }
                return None;
            }
        };

        if let Some(envelope) = &mut self.envelope {
            if envelope.frame < envelope.frames { sample = envelope.sample(sample); }
            else { self.envelope = None; }
        }
        if self.outgoing.get().is_some() || !self.analyzer.is_enabled() {
            return Some(sample);
        }

        self.frame_sum += sample;
        self.ch_idx += 1;
        if self.ch_idx >= self.channels {
            let mono = self.frame_sum / self.channels as f32;
            self.frame_sum = 0.0;
            self.ch_idx = 0;
            self.block.push(mono);
            if self.block.len() >= Self::BLOCK {
                self.analyzer.push_mono(&self.block, self.sample_rate);
                self.block.clear();
            }
        }

        Some(sample)
    }
}

impl<S: Source<Item = f32>> Source for AnalyzerSource<S> {
    fn current_span_len(&self) -> Option<usize> { self.source.current_span_len() }
    fn channels(&self) -> ChannelCount { nz_channels(self.channels) }
    fn sample_rate(&self) -> SampleRate { nz_rate(self.sample_rate) }
    fn total_duration(&self) -> Option<Duration> { self.source.total_duration() }
}

fn cover_art_from_tag(tag: &lofty::tag::Tag) -> Option<String> {
    let picture = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())?;

    let mime = picture
        .mime_type()
        .map(|m| m.to_string())
        .unwrap_or_else(|| "image/jpeg".to_string());

    let b64 = base64::engine::general_purpose::STANDARD.encode(picture.data());
    Some(format!("data:{};base64,{}", mime, b64))
}

pub fn extract_accent_color(image_data: &[u8]) -> Option<String> {
    let img = image::load_from_memory(image_data).ok()?;
    let thumb = img.thumbnail(96, 96);
    extract_via_hue_histogram(&thumb)
}

const HUE_BINS: usize = 36;
const HUE_BIN_WIDTH: f64 = 360.0 / HUE_BINS as f64;

/// Yellow zone, disallowed as a UI accent.
const YELLOW_FROM: f64 = 45.0;
const YELLOW_TO: f64 = 78.0;

fn is_yellow_hue(hue: f64) -> bool {
    (YELLOW_FROM..YELLOW_TO).contains(&hue)
}

#[derive(Clone, Copy, Default)]
struct HueBin {
    weight: f64,
    sin: f64,
    cos: f64,
    sat: f64,
    light: f64,
}

/// Smoothed window over 3 neighboring bins: mass and weighted-average color.
#[derive(Clone, Copy, Default)]
struct HueWindow {
    weight: f64,
    hue: f64,
    sat: f64,
    light: f64,
}

fn extract_via_hue_histogram(thumb: &image::DynamicImage) -> Option<String> {
    let rgb = thumb.to_rgb8();
    let total_px = (rgb.width() as f64) * (rgb.height() as f64);
    if total_px < 1.0 {
        return None;
    }

    let mut bins = [HueBin::default(); HUE_BINS];
    let mut chroma_weight = 0.0f64;
    let mut light_sum = 0.0f64;

    for px in rgb.pixels() {
        let (h, s, l) = rgb_to_hsl(px[0], px[1], px[2]);
        light_sum += l;
        if s < 0.12 || !(0.06..=0.94).contains(&l) {
            continue;
        }
        let w = s * (1.0 - (2.0 * l - 1.0).abs()).sqrt();
        let bin = &mut bins[((h / HUE_BIN_WIDTH) as usize).min(HUE_BINS - 1)];
        let rad = h.to_radians();
        bin.weight += w;
        bin.sin += w * rad.sin();
        bin.cos += w * rad.cos();
        bin.sat += w * s;
        bin.light += w * l;
        chroma_weight += w;
    }

    if chroma_weight < total_px * 0.015 {
        let l = (light_sum / total_px).clamp(0.30, 0.46);
        let (r, g, b) = hsl_to_rgb(0.0, 0.0, l);
        return Some(format!("#{:02X}{:02X}{:02X}", r, g, b));
    }

    let window_at = |i: usize| -> HueWindow {
        let prev = &bins[(i + HUE_BINS - 1) % HUE_BINS];
        let cur = &bins[i];
        let next = &bins[(i + 1) % HUE_BINS];
        let weight = cur.weight + 0.5 * (prev.weight + next.weight);
        if weight <= f64::EPSILON {
            return HueWindow::default();
        }
        let sin = cur.sin + 0.5 * (prev.sin + next.sin);
        let cos = cur.cos + 0.5 * (prev.cos + next.cos);
        HueWindow {
            weight,
            hue: sin.atan2(cos).to_degrees().rem_euclid(360.0),
            sat: (cur.sat + 0.5 * (prev.sat + next.sat)) / weight,
            light: (cur.light + 0.5 * (prev.light + next.light)) / weight,
        }
    };
    let windows: Vec<HueWindow> = (0..HUE_BINS).map(window_at).collect();
    let max_weight = windows.iter().map(|w| w.weight).fold(0.0f64, f64::max);
    if max_weight <= f64::EPSILON {
        return None;
    }

    let mut pool: Vec<usize> = (0..HUE_BINS)
        .filter(|&i| windows[i].weight >= chroma_weight * 0.06)
        .collect();
    if pool.is_empty() {
        pool = (0..HUE_BINS).filter(|&i| windows[i].weight > 0.0).collect();
    }

    let non_yellow: Vec<usize> = pool
        .iter()
        .copied()
        .filter(|&i| !is_yellow_hue(windows[i].hue))
        .collect();
    if !non_yellow.is_empty() {
        pool = non_yellow;
    }

    let score = |w: &HueWindow| -> f64 {
        let pop = w.weight / max_weight;
        let sat_fit = (w.sat / 0.55).min(1.0);
        let light_fit = (1.0 - (w.light - 0.45).abs() * 1.7).clamp(0.0, 1.0);
        pop * 0.62 + sat_fit * 0.16 + light_fit * 0.22
    };

    let best = pool.into_iter().max_by(|&a, &b| {
        score(&windows[a])
            .partial_cmp(&score(&windows[b]))
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;

    let w = &windows[best];
    let (r, g, b) = normalize_accent_hsl(w.hue, w.sat, w.light);
    Some(format!("#{:02X}{:02X}{:02X}", r, g, b))
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let rf = r as f64 / 255.0;
    let gf = g as f64 / 255.0;
    let bf = b as f64 / 255.0;

    let max_c = rf.max(gf).max(bf);
    let min_c = rf.min(gf).min(bf);
    let delta = max_c - min_c;
    let lightness = (max_c + min_c) / 2.0;

    let hue = if delta == 0.0 {
        0.0
    } else if max_c == rf {
        60.0 * (((gf - bf) / delta) % 6.0)
    } else if max_c == gf {
        60.0 * ((bf - rf) / delta + 2.0)
    } else {
        60.0 * ((rf - gf) / delta + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };

    let sat = if delta == 0.0 {
        0.0
    } else {
        delta / (1.0 - (2.0 * lightness - 1.0).abs())
    };

    (hue, sat, lightness)
}

fn hsl_to_rgb(hue: f64, sat: f64, lightness: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * lightness - 1.0).abs()) * sat.min(1.0);
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = lightness - c / 2.0;

    let (r1, g1, b1) = match (hue as u32) / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

fn saturation_ceiling(hue: f64) -> f64 {
    const POINTS: [f64; 13] = [
        0.60,
        0.58,
        0.45,
        0.42,
        0.45,
        0.48,
        0.52,
        0.58,
        0.62,
        0.60,
        0.52,
        0.56,
        0.60,
    ];
    let h = hue.rem_euclid(360.0);
    let seg = ((h / 30.0) as usize).min(11);
    let t = (h - seg as f64 * 30.0) / 30.0;
    POINTS[seg] + (POINTS[seg + 1] - POINTS[seg]) * t
}

fn normalize_accent_hsl(hue: f64, sat: f64, light: f64) -> (u8, u8, u8) {
    let mut hue = hue.rem_euclid(360.0);
    let mut sat = sat.clamp(0.0, 1.0);

    if (YELLOW_FROM..62.0).contains(&hue) {
        hue = 36.0 + (hue - YELLOW_FROM) * (6.0 / (62.0 - YELLOW_FROM));
    } else if (62.0..YELLOW_TO).contains(&hue) {
        hue = 88.0 + (hue - 62.0) * (7.0 / (YELLOW_TO - 62.0));
    }

    let light = light.clamp(0.32, 0.48);

    let ceiling = saturation_ceiling(hue);
    let knee = ceiling * 0.8;
    if sat > knee {
        sat = knee + (sat - knee) * 0.35;
    }
    sat = sat.min(ceiling);

    hsl_to_rgb(hue, sat, light)
}

/// Whether a cover is embedded + its accent color (computed once, cached in the DB).
fn cover_meta_from_tag(tag: &lofty::tag::Tag) -> (bool, Option<String>) {
    let picture = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first());

    let Some(picture) = picture else {
        return (false, None);
    };

    let accent = extract_accent_color(picture.data());
    (true, accent)
}

/// Caches probe_actual_codec by (path, mtime): only M4A/MP4 needs a second open (AAC vs ALAC).
fn probe_cache() -> &'static Mutex<HashMap<(String, u64), AudioCodecId>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, u64), AudioCodecId>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn file_mtime_secs(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| {
            t.duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        })
        .unwrap_or(0)
}

/// Only needed for M4A/MP4 containers
fn probe_actual_codec(file_path: &str) -> Option<AudioCodecId> {
    let mtime = file_mtime_secs(std::path::Path::new(file_path));
    let key = (file_path.to_string(), mtime);

    if let Ok(cache) = probe_cache().lock()
        && let Some(&codec) = cache.get(&key) {
            return Some(codec);
        }

    let reader = probe_reader(file_path).ok()?;
    let codec = audio_tracks(&*reader).first().map(|(_, params, _)| params.codec)?;

    if let Ok(mut cache) = probe_cache().lock() {
        cache.insert(key, codec);
    }
    Some(codec)
}

fn is_lossless_codec(codec: AudioCodecId) -> bool {
    matches!(
        codec,
        CODEC_ID_FLAC
            | CODEC_ID_ALAC
            | CODEC_ID_PCM_S16LE
            | CODEC_ID_PCM_S16BE
            | CODEC_ID_PCM_S24LE
            | CODEC_ID_PCM_S32LE
            | CODEC_ID_PCM_F32LE
    )
}

fn format_name_from_ext(ext: &str) -> String {
    match ext {
        "flac" => "FLAC",
        "wav" => "WAV",
        "mp3" => "MP3",
        "ogg" => "OGG Vorbis",
        "aac" => "AAC",
        "alac" => "ALAC",
        "ape" => "APE",
        "wv" => "WavPack",
        "aiff" | "aif" => "AIFF",
        "opus" => "Opus",
        other => other,
    }
    .to_string()
}

/// A failed open leaves nothing playing: drop the sinks and mark the end signal, or the player
/// stays "Playing".
fn stop_after_failed_open(
    sink: &mut Option<Player>,
    fadeout_sink: &mut Option<Player>,
    spectrum: &SpectrumAnalyzer,
    sink_empty: &SinkEndSignal,
) {
    if let Some(s) = sink.take() { s.stop(); }
    if let Some(s) = fadeout_sink.take() { s.stop(); }
    spectrum.set_active(false);
    sink_empty.set_finished();
}

/// An open stream keeps the device rendering silence: paused or stopped this long, the device is
/// closed. Play/resume reopens it.
const IDLE_RELEASE: Duration = Duration::from_secs(30);

fn open_output(enhance: &EnhanceShared) -> Result<MixerDeviceSink, String> {
    let mut stream = rodio::DeviceSinkBuilder::open_default_sink()
        .map_err(|e| format!("Не удалось инициализировать аудио выход: {}", e))?;
    stream.log_on_drop(false);
    let (device_name, profile) = crate::enhance::detect_output_device();
    enhance.set_detected(device_name, profile);
    Ok(stream)
}

/// The open output stream, opening the default device first when it was released.
fn ensure_output<'a>(
    stream: &'a mut Option<MixerDeviceSink>,
    enhance: &EnhanceShared,
) -> Result<&'a MixerDeviceSink, String> {
    if stream.is_none() {
        *stream = Some(open_output(enhance)?);
    }
    stream.as_ref().ok_or_else(|| "Аудио выход недоступен".to_string())
}

fn audio_thread_loop(
    rx: mpsc::Receiver<AudioCmd>,
    eq_settings: Arc<RwLock<EqSettings>>,
    enhance: Arc<EnhanceShared>,
    fx: Arc<FxShared>,
    spectrum: Arc<SpectrumAnalyzer>,
    sink_empty: Arc<SinkEndSignal>,
) {
    let mut stream: Option<MixerDeviceSink> = None;
    let mut sink: Option<Player> = None;
    let mut fadeout_sink: Option<Player> = None;
    let mut outgoing = FadeRequest::default();
    let mut idle_since: Option<Instant> = None;

    loop {
        let idle = stream.is_some()
            && sink.as_ref().is_none_or(|s| s.is_paused() || sink_empty.is_finished());
        if !idle {
            idle_since = None;
        } else if idle_since.is_none() {
            idle_since = Some(Instant::now());
        }

        let cmd = if stream.is_none() {
            match rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            }
        } else {
            let timeout = idle_since.map_or(IDLE_RELEASE, |t| IDLE_RELEASE.saturating_sub(t.elapsed()));
            match rx.recv_timeout(timeout) {
                Ok(cmd) => cmd,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if idle_since.is_some_and(|t| t.elapsed() >= IDLE_RELEASE) {
                        // The paused sink goes too: Resume reopens the file at the position.
                        if let Some(s) = fadeout_sink.take() { s.stop(); }
                        if let Some(s) = sink.take() { s.stop(); }
                        spectrum.set_active(false);
                        stream = None;
                        idle_since = None;
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };

        match cmd {
            AudioCmd::PlayFrom { file_path, seek_secs, volume, resp } => {
                if let Some(old) = fadeout_sink.take() { old.stop(); }
                if let Some(ref s) = sink { s.stop(); }
                let result = (|| -> Result<(), String> {
                    let output = ensure_output(&mut stream, &enhance)?;
                    let new_sink = Player::connect_new(output.mixer());
                    new_sink.set_volume(volume);
                    let source = BufferedSymphoniaSource::new_with_seek(&file_path, seek_secs)?;
                    outgoing = FadeRequest::default();
                    sink_empty.set_playing();
                    new_sink.append(AnalyzerSource::new(
                        effects(source, &eq_settings, &enhance, &fx),
                        spectrum.clone(),
                        sink_empty.clone(), outgoing.clone(),
                    ));
                    sink = Some(new_sink);
                    Ok(())
                })();
                if result.is_err() {
                    stop_after_failed_open(&mut sink, &mut fadeout_sink, &spectrum, &sink_empty);
                }
                let _ = resp.send(result);
            }

            AudioCmd::Pause => {
                if let Some(old) = &fadeout_sink { old.pause(); }
                if let Some(ref s) = sink
                    && !s.is_paused() { s.pause(); }
            }

            AudioCmd::Resume { file_path, position_secs, volume } => {
                if let Some(old) = &fadeout_sink { old.play(); }
                if let Some(ref s) = sink {
                    if s.is_paused() { s.play(); }
                } else if let Some(file_path) = file_path {
                    let result = (|| -> Result<(), String> {
                        let output = ensure_output(&mut stream, &enhance)?;
                        let new_sink = Player::connect_new(output.mixer());
                        new_sink.set_volume(volume);
                        let source = BufferedSymphoniaSource::new_with_seek(&file_path, position_secs)?;
                        outgoing = FadeRequest::default();
                        sink_empty.set_playing();
                        new_sink.append(AnalyzerSource::new(
                            effects(source, &eq_settings, &enhance, &fx),
                            spectrum.clone(),
                            sink_empty.clone(), outgoing.clone(),
                        ));
                        sink = Some(new_sink);
                        Ok(())
                    })();
                    if result.is_err() {
                        stop_after_failed_open(&mut sink, &mut fadeout_sink, &spectrum, &sink_empty);
                    }
                }
            }

            AudioCmd::Stop => {
                if let Some(ref s) = sink { s.stop(); }
                sink = None;
                if let Some(old_fade) = fadeout_sink.take() { old_fade.stop(); }
                spectrum.set_active(false);
                sink_empty.set_finished();
            }

            AudioCmd::SetVolume(vol) => {
                if let Some(old) = &fadeout_sink { old.set_volume(vol); }
                if let Some(ref s) = sink { s.set_volume(vol); }
            }

            AudioCmd::Seek { file_path, position_secs, volume, was_paused, resp } => {
                if let Some(old) = fadeout_sink.take() { old.stop(); }
                if let Some(ref s) = sink { s.stop(); }
                let result = (|| -> Result<(), String> {
                    let output = ensure_output(&mut stream, &enhance)?;
                    let new_sink = Player::connect_new(output.mixer());
                    new_sink.set_volume(volume);
                    let source = BufferedSymphoniaSource::new_with_seek(&file_path, position_secs)?;
                    outgoing = FadeRequest::default();
                    sink_empty.set_playing();
                    new_sink.append(AnalyzerSource::new(
                        effects(source, &eq_settings, &enhance, &fx),
                        spectrum.clone(),
                        sink_empty.clone(), outgoing.clone(),
                    ));
                    if was_paused { new_sink.pause(); }
                    sink = Some(new_sink);
                    Ok(())
                })();
                if result.is_err() {
                    stop_after_failed_open(&mut sink, &mut fadeout_sink, &spectrum, &sink_empty);
                }
                let _ = resp.send(result);
            }

            AudioCmd::SeekCrossfade {
                file_path, position_secs, current_pos, volume,
                was_paused, crossfade_secs, resp,
            } => {
                let result = (|| -> Result<(), String> {
                    let dur = Duration::from_secs_f64(crossfade_secs);

                    if let Some(old_fade) = fadeout_sink.take() { old_fade.stop(); }

                    let output = ensure_output(&mut stream, &enhance)?;
                    let fade_sink = Player::connect_new(output.mixer());
                    fade_sink.set_volume(volume);
                    let fade_source = BufferedSymphoniaSource::new_with_seek(&file_path, current_pos)?;
                    fade_sink.append(FadeOut::new(
                        effects(fade_source, &eq_settings, &enhance, &fx),
                        dur,
                    ));

                    if let Some(ref old) = sink { old.stop(); }
                    fadeout_sink = Some(fade_sink);

                    let new_sink = Player::connect_new(output.mixer());
                    new_sink.set_volume(volume);
                    let source = BufferedSymphoniaSource::new_with_seek(&file_path, position_secs)?;
                    outgoing = FadeRequest::default();
                    sink_empty.set_playing();
                    new_sink.append(AnalyzerSource::new(
                        effects(source, &eq_settings, &enhance, &fx).fade_in(dur),
                        spectrum.clone(),
                        sink_empty.clone(), outgoing.clone(),
                    ));
                    if was_paused { new_sink.pause(); }
                    sink = Some(new_sink);
                    Ok(())
                })();
                if result.is_err() {
                    stop_after_failed_open(&mut sink, &mut fadeout_sink, &spectrum, &sink_empty);
                }
                let _ = resp.send(result);
            }

            AudioCmd::PlayCrossfade {
                file_path, seek_secs, volume, crossfade_secs, mix_profile, resp,
            } => {
                let result = (|| -> Result<(), String> {
                    let dur = Duration::from_secs_f64(crossfade_secs);

                    if let Some(old_fade) = fadeout_sink.take() { old_fade.stop(); }

                    let output = ensure_output(&mut stream, &enhance)?;
                    let new_sink = Player::connect_new(output.mixer());
                    new_sink.set_volume(volume);
                    let source = BufferedSymphoniaSource::new_with_seek(&file_path, seek_secs)?;
                    if let Some(old) = sink.take() {
                        if let Some(profile) = &mix_profile {
                            let _ = outgoing.set((profile.clone(), crossfade_secs));
                            fadeout_sink = Some(old);
                        } else { old.stop(); }
                    }
                    outgoing = FadeRequest::default();
                    sink_empty.set_playing();
                    let mut input = AnalyzerSource::new(
                        effects(source, &eq_settings, &enhance, &fx),
                        spectrum.clone(),
                        sink_empty.clone(), outgoing.clone(),
                    );
                    if let Some(profile) = mix_profile {
                        input.envelope = Some(MixEnvelope::new(profile, true, crossfade_secs, input.channels, input.sample_rate));
                        new_sink.append(input);
                    } else { new_sink.append(input.fade_in(dur)); }
                    sink = Some(new_sink);
                    Ok(())
                })();
                if result.is_err() {
                    stop_after_failed_open(&mut sink, &mut fadeout_sink, &spectrum, &sink_empty);
                }
                let _ = resp.send(result);
            }
        }
    }
}

/// The output stream (cpal, COM on Windows) lives on the `audio-output` thread; sink operations go
/// through a channel.
pub struct AudioPlayer {
    cmd_tx: mpsc::Sender<AudioCmd>,
    current_track: Option<TrackMetadata>,
    state: PlaybackState,
    volume: f32,
    playback_started_at: Option<Instant>,
    accumulated_position: f64,
    eq_settings: Arc<RwLock<EqSettings>>,
    enhance: Arc<EnhanceShared>,
    fx: Arc<FxShared>,
    spectrum: Arc<SpectrumAnalyzer>,
    /// Whether the current sink is empty (track finished/stopped).
    sink_empty: Arc<SinkEndSignal>,
}

impl AudioPlayer {
    /// Spawns the audio thread.
    pub fn new() -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (init_tx, init_rx) = mpsc::sync_channel::<Result<(), String>>(0);
        let eq_settings = Arc::new(RwLock::new(EqSettings::default()));
        let eq_for_thread = eq_settings.clone();
        let enhance = Arc::new(EnhanceShared::new());
        let enhance_for_thread = enhance.clone();
        let fx = Arc::new(FxShared::new());
        let fx_for_thread = fx.clone();
        let spectrum = Arc::new(SpectrumAnalyzer::new());
        let spectrum_for_thread = spectrum.clone();
        let sink_empty = Arc::new(SinkEndSignal::new(true));
        let sink_empty_for_thread = sink_empty.clone();

        std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || {
                let (device_name, profile) = crate::enhance::detect_output_device();
                enhance_for_thread.set_detected(device_name, profile);
                let _ = init_tx.send(Ok(()));
                audio_thread_loop(
                    cmd_rx,
                    eq_for_thread,
                    enhance_for_thread,
                    fx_for_thread,
                    spectrum_for_thread,
                    sink_empty_for_thread,
                );
            })
            .map_err(|e| format!("Не удалось запустить аудио поток: {}", e))?;

        init_rx
            .recv()
            .map_err(|_| "Аудио поток завершился преждевременно".to_string())??;

        Ok(Self {
            cmd_tx,
            current_track: None,
            state: PlaybackState::Stopped,
            volume: 1.0,
            playback_started_at: None,
            accumulated_position: 0.0,
            eq_settings,
            enhance,
            fx,
            spectrum,
            sink_empty,
        })
    }

    pub fn spectrum_bars(&self, bars: usize) -> Vec<f32> {
        self.spectrum.compute_bars(bars)
    }

    pub fn set_spectrum_enabled(&self, enabled: bool) {
        self.spectrum.set_enabled(enabled);
    }

    fn send(&self, make: impl FnOnce(Resp) -> AudioCmd) -> Result<(), String> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.cmd_tx
            .send(make(tx))
            .map_err(|_| "Аудио поток недоступен".to_string())?;
        rx.recv()
            .map_err(|_| "Аудио поток недоступен".to_string())?
    }

    fn send_play(&mut self, make: impl FnOnce(Resp) -> AudioCmd) -> Result<(), String> {
        let result = self.send(make);
        if result.is_err() {
            self.sink_stopped();
        }
        result
    }

    /// Fire-and-forget, no response wait
    fn fire(&self, cmd: AudioCmd) {
        let _ = self.cmd_tx.send(cmd);
    }

    fn is_sink_empty(&self) -> bool {
        self.sink_empty.is_finished()
    }

    pub fn sink_end_signal(&self) -> Arc<SinkEndSignal> {
        self.sink_empty.clone()
    }

    /// Detects the file type by content: yt-dlp writes Opus into `.ogg` files, which
    /// extension-based probing reads as Vorbis and fails.
    pub(crate) fn read_tagged_file(path: &std::path::Path) -> Option<lofty::file::TaggedFile> {
        Probe::open(path).ok()?.guess_file_type().ok()?.read().ok()
    }

    /// Full-size cover as a data URL.
    pub fn extract_cover_art(file_path: &str) -> Option<String> {
        let tagged_file = Self::read_tagged_file(&PathBuf::from(file_path))?;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())?;
        cover_art_from_tag(tag)
    }

    /// Cheap check - doesn't extract picture bytes.
    pub fn has_embedded_cover(file_path: &str) -> bool {
        let Some(tagged_file) = Self::read_tagged_file(&PathBuf::from(file_path)) else {
            return false;
        };
        match tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) {
            Some(tag) => !tag.pictures().is_empty(),
            None => false,
        }
    }

    /// Raw embedded cover bytes, for generating a WebP thumbnail.
    pub fn extract_cover_bytes(file_path: &str) -> Option<Vec<u8>> {
        let tagged_file = Self::read_tagged_file(&PathBuf::from(file_path))?;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())?;
        let picture = tag
            .pictures()
            .iter()
            .find(|p| p.pic_type() == PictureType::CoverFront)
            .or_else(|| tag.pictures().first())?;
        Some(picture.data().to_vec())
    }

    /// No cover/accent color - used for library scanning.
    pub fn get_metadata_light(file_path: &str) -> Result<TrackMetadata, String> {
        Self::get_metadata_inner(file_path, false)
    }

    /// Symphonia probing only for M4A/MP4, where the codec is ambiguous (AAC vs ALAC).
    pub fn get_metadata(file_path: &str) -> Result<TrackMetadata, String> {
        Self::get_metadata_inner(file_path, true)
    }

    fn get_metadata_inner(file_path: &str, with_cover: bool) -> Result<TrackMetadata, String> {
        let path = PathBuf::from(file_path);
        if !path.exists() {
            return Err(format!("Файл не найден: {}", file_path));
        }

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let needs_probe = matches!(extension.as_str(), "m4a" | "mp4");

        let (is_lossless, format) = if needs_probe {
            let lossless = probe_actual_codec(file_path)
                .map(is_lossless_codec)
                .unwrap_or(false);
            let fmt = if lossless {
                match extension.as_str() {
                    "m4a" => "M4A (ALAC)".to_string(),
                    _ => "ALAC".to_string(),
                }
            } else {
                match extension.as_str() {
                    "m4a" => "M4A (AAC)".to_string(),
                    "mp4" => "MP4".to_string(),
                    _ => extension.to_uppercase(),
                }
            };
            (lossless, fmt)
        } else {
            let lossless = matches!(
                extension.as_str(),
                "flac" | "wav" | "alac" | "ape" | "wv" | "aiff" | "aif"
            );
            (lossless, format_name_from_ext(&extension))
        };

        let mut title = None;
        let mut artist = None;
        let mut album = None;
        let mut genre = None;
        let mut duration_secs = None;
        let mut sample_rate = None;
        let mut bit_depth = None;
        let mut channels = None;
        let mut bitrate = None;
        let mut cover_art: Option<String> = None;
        let mut accent_color: Option<String> = None;
        let mut has_cover = false;

        if let Some(tagged_file) = Self::read_tagged_file(&path) {
            {
                if let Some(tag) = tagged_file
                    .primary_tag()
                    .or_else(|| tagged_file.first_tag())
                {
                    title = tag.title().map(|s| s.to_string());
                    artist = tag.artist().map(|s| s.to_string());
                    album = tag.album().map(|s| s.to_string());
                    genre = tag.genre().map(|s| s.to_string());
                    has_cover = !tag.pictures().is_empty();
                    if with_cover {
                        let (_, accent) = cover_meta_from_tag(tag);
                        accent_color = accent;
                        if has_cover {
                            cover_art = Some(crate::thumbnails::cover_protocol_url(
                                file_path,
                                &crate::thumbnails::embedded_sig(file_path),
                            ));
                        }
                    }
                }

                let properties = tagged_file.properties();
                duration_secs = Some(properties.duration().as_secs_f64());
                sample_rate = properties.sample_rate();
                bit_depth = properties.bit_depth();
                channels = properties.channels();
                bitrate = properties.audio_bitrate();
            }
        }

        Ok(TrackMetadata {
            title,
            artist,
            album,
            genre,
            duration_secs,
            format,
            sample_rate,
            bit_depth,
            channels,
            bitrate,
            is_lossless,
            file_path: file_path.to_string(),
            cover_art,
            accent_color,
            bpm: None,
            loudness: None,
            energy: None,
            danceability: None,
            spectral_centroid: None,
            key: None,
            audio_fingerprint: None,
            has_cover,
        })
    }

    pub fn supported_formats() -> &'static [&'static str] {
        &["flac", "wav", "mp3", "ogg", "aac", "m4a", "mp4", "aiff", "aif", "opus"]
    }

    pub fn play(&mut self, file_path: &str) -> Result<TrackMetadata, String> {
        let metadata = Self::get_metadata(file_path)?;
        self.play_from(file_path, 0.0, metadata)
    }

    /// Like [`Self::play`], with `metadata` already read via [`Self::get_metadata`].
    pub fn play_preloaded(&mut self, file_path: &str, metadata: TrackMetadata) -> Result<TrackMetadata, String> {
        self.play_from(file_path, 0.0, metadata)
    }

    /// Loads, seeks, and immediately pauses - used to restore a saved session.
    pub fn play_paused_preloaded(
        &mut self,
        file_path: &str,
        position_secs: f64,
        metadata: TrackMetadata,
    ) -> Result<TrackMetadata, String> {
        let metadata = self.play_from(file_path, position_secs, metadata)?;
        self.pause();
        Ok(metadata)
    }

    pub fn play_crossfade_preloaded(
        &mut self,
        file_path: &str,
        crossfade_secs: f64,
        mix_profile: Option<MixProfile>,
        position_secs: f64,
        metadata: TrackMetadata,
    ) -> Result<TrackMetadata, String> {
        let seek_secs = position_secs.max(0.0).min(metadata.duration_secs.unwrap_or(f64::MAX));
        let crossfade_secs = crossfade_secs.clamp(0.0, 32.0);
        if !crossfade_secs.is_finite() || crossfade_secs <= 0.0 || self.state == PlaybackState::Stopped {
            return self.play_from(file_path, seek_secs, metadata);
        }

        let volume = self.volume;
        self.send_play(|resp| AudioCmd::PlayCrossfade {
            mix_profile,
            file_path: file_path.to_string(),
            seek_secs,
            volume,
            crossfade_secs,
            resp,
        })?;

        self.current_track = Some(metadata.clone());
        self.state = PlaybackState::Playing;
        self.playback_started_at = Some(Instant::now());
        self.accumulated_position = seek_secs;

        Ok(metadata)
    }

    fn play_from(
        &mut self,
        file_path: &str,
        seek_secs: f64,
        metadata: TrackMetadata,
    ) -> Result<TrackMetadata, String> {
        let volume = self.volume;
        self.send_play(|resp| AudioCmd::PlayFrom {
            file_path: file_path.to_string(),
            seek_secs,
            volume,
            resp,
        })?;

        self.current_track = Some(metadata.clone());
        self.state = PlaybackState::Playing;
        self.playback_started_at = Some(Instant::now());
        self.accumulated_position = seek_secs;

        Ok(metadata)
    }

    pub fn pause(&mut self) {
        if self.state == PlaybackState::Playing {
            self.fire(AudioCmd::Pause);
            if let Some(started) = self.playback_started_at.take() {
                self.accumulated_position += self.track_secs_since(started);
            }
            self.state = PlaybackState::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.state == PlaybackState::Paused {
            self.fire(AudioCmd::Resume {
                file_path: self.current_file_path(),
                position_secs: self.get_position(),
                volume: self.volume,
            });
            self.playback_started_at = Some(Instant::now());
            self.state = PlaybackState::Playing;
        }
    }

    pub fn stop(&mut self) {
        self.fire(AudioCmd::Stop);
        self.sink_stopped();
    }

    fn sink_stopped(&mut self) {
        self.state = PlaybackState::Stopped;
        self.current_track = None;
        self.playback_started_at = None;
        self.accumulated_position = 0.0;
    }

    pub fn set_volume(&mut self, volume: f32) {
        let vol = volume.clamp(0.0, 1.0);
        self.fire(AudioCmd::SetVolume(vol));
        self.volume = vol;
    }

    pub fn seek(&mut self, position_secs: f64) -> Result<(), String> {
        let file_path = self.current_track
            .as_ref()
            .map(|t| t.file_path.clone())
            .ok_or_else(|| "Нет активного трека".to_string())?;

        let was_paused = self.state == PlaybackState::Paused;

        let volume = self.volume;
        self.send_play(|resp| AudioCmd::Seek {
            file_path,
            position_secs,
            volume,
            was_paused,
            resp,
        })?;

        self.accumulated_position = position_secs;
        if self.state == PlaybackState::Playing {
            self.playback_started_at = Some(Instant::now());
        }

        Ok(())
    }

    pub fn seek_crossfade(&mut self, position_secs: f64, crossfade_secs: f64) -> Result<(), String> {
        if crossfade_secs <= 0.0 {
            return self.seek(position_secs);
        }

        let file_path = self.current_track
            .as_ref()
            .map(|t| t.file_path.clone())
            .ok_or_else(|| "Нет активного трека".to_string())?;

        let current_pos = self.get_position();
        let was_paused = self.state == PlaybackState::Paused;

        let volume = self.volume;
        self.send_play(|resp| AudioCmd::SeekCrossfade {
            file_path,
            position_secs,
            current_pos,
            volume,
            was_paused,
            crossfade_secs,
            resp,
        })?;

        self.accumulated_position = position_secs;
        if self.state == PlaybackState::Playing {
            self.playback_started_at = Some(Instant::now());
        }

        Ok(())
    }

    pub fn get_position(&self) -> f64 {
        let elapsed = match self.playback_started_at {
            Some(started) if self.state == PlaybackState::Playing => {
                self.track_secs_since(started)
            }
            _ => 0.0,
        };
        let pos = self.accumulated_position + elapsed;
        if let Some(ref track) = self.current_track
            && let Some(duration) = track.duration_secs {
                return pos.min(duration);
            }
        pos
    }

    pub fn current_file_path(&self) -> Option<String> {
        self.current_track.as_ref().map(|t| t.file_path.clone())
    }

    /// Detects a naturally finished sink and returns its path once (repeat calls are no-ops).
    pub fn poll_finished(&mut self) -> Option<String> {
        if self.state == PlaybackState::Playing && self.is_sink_empty() {
            let path = self.current_file_path();
            self.sink_stopped();
            path
        } else {
            None
        }
    }

    pub fn get_status(&mut self) -> PlayerStatus {
        self.poll_finished();

        PlayerStatus {
            state: self.state,
            volume: self.volume,
            position_secs: self.get_position(),
            current_file_path: self.current_track.as_ref().map(|t| t.file_path.clone()),
        }
    }

    pub fn set_eq_bands(&self, bands: Vec<f32>) {
        if let Ok(mut settings) = self.eq_settings.write() {
            settings.bands = bands.into_iter().map(|b| b.clamp(-12.0, 12.0)).collect();
        }
    }

    pub fn set_eq_gain(&self, gain: f32) {
        if let Ok(mut settings) = self.eq_settings.write() {
            settings.gain = gain.clamp(-12.0, 12.0);
        }
    }

    pub fn set_eq_enabled(&self, enabled: bool) {
        if let Ok(mut settings) = self.eq_settings.write() {
            settings.enabled = enabled;
        }
    }

    pub fn get_eq_settings(&self) -> EqSettings {
        self.eq_settings.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_eq_settings(&self, settings: EqSettings) {
        if let Ok(mut current) = self.eq_settings.write() {
            *current = settings;
        }
    }

    pub fn set_enhance_settings(&self, settings: &EnhanceSettings) {
        self.enhance.apply(settings);
    }

    pub fn enhance_settings(&self) -> EnhanceSettings {
        self.enhance.settings()
    }

    pub fn enhance_status(&self) -> EnhanceStatus {
        self.enhance.status()
    }

    /// Track time that passed since `started`: wall time scaled by playback speed.
    fn track_secs_since(&self, started: Instant) -> f64 {
        started.elapsed().as_secs_f64() * self.fx.speed() as f64
    }

    pub fn fx_settings(&self) -> FxSettings {
        self.fx.settings()
    }

    pub fn set_fx_settings(&mut self, settings: FxSettings) {
        if self.state == PlaybackState::Playing
            && let Some(started) = self.playback_started_at {
                self.accumulated_position += self.track_secs_since(started);
                self.playback_started_at = Some(Instant::now());
            }
        self.fx.apply(settings);
    }
}

#[cfg(test)]
mod accent_color_tests {
    use super::*;

    fn img_from_fn(f: impl Fn(u32, u32) -> (u8, u8, u8)) -> image::DynamicImage {
        let buf = image::RgbImage::from_fn(96, 96, |x, y| {
            let (r, g, b) = f(x, y);
            image::Rgb([r, g, b])
        });
        image::DynamicImage::ImageRgb8(buf)
    }

    fn accent_hsl(img: &image::DynamicImage) -> (f64, f64, f64) {
        let hex = extract_via_hue_histogram(img).expect("цвет должен извлекаться");
        let r = u8::from_str_radix(&hex[1..3], 16).unwrap();
        let g = u8::from_str_radix(&hex[3..5], 16).unwrap();
        let b = u8::from_str_radix(&hex[5..7], 16).unwrap();
        rgb_to_hsl(r, g, b)
    }

    #[test]
    fn dominant_muted_color_beats_small_vivid_spot() {
        let blue = hsl_to_rgb(220.0, 0.35, 0.40);
        let red = hsl_to_rgb(0.0, 1.0, 0.50);
        let img = img_from_fn(|x, _| if x < 86 { blue } else { red });
        let (h, _, _) = accent_hsl(&img);
        assert!((190.0..260.0).contains(&h), "ожидался синий, получен hue={h}");
    }

    #[test]
    fn yellow_skipped_when_alternative_exists() {
        let yellow = hsl_to_rgb(55.0, 0.80, 0.50);
        let blue = hsl_to_rgb(230.0, 0.60, 0.45);
        let img = img_from_fn(|x, _| if x < 58 { yellow } else { blue });
        let (h, _, _) = accent_hsl(&img);
        assert!((200.0..260.0).contains(&h), "ожидался синий, получен hue={h}");
    }

    #[test]
    fn fully_yellow_cover_shifts_out_of_yellow_zone() {
        let yellow = hsl_to_rgb(55.0, 0.90, 0.50);
        let img = img_from_fn(|_, _| yellow);
        let (h, _, _) = accent_hsl(&img);
        assert!(
            !is_yellow_hue(h),
            "жёлтый не должен попадать в UI, получен hue={h}"
        );
    }

    #[test]
    fn neon_green_is_tamed() {
        let neon = hsl_to_rgb(120.0, 1.0, 0.50);
        let img = img_from_fn(|_, _| neon);
        let (h, s, l) = accent_hsl(&img);
        assert!((110.0..130.0).contains(&h), "оттенок должен сохраниться, hue={h}");
        assert!(s <= 0.47, "насыщенность должна быть приглушена, s={s}");
        assert!((0.31..=0.49).contains(&l), "светлота вне диапазона, l={l}");
    }

    #[test]
    fn monochrome_cover_gives_neutral_gray() {
        let img = img_from_fn(|_, _| (128, 128, 128));
        let (_, s, _) = accent_hsl(&img);
        assert!(s < 0.02, "ч/б обложка должна давать нейтральный серый, s={s}");
    }

    #[test]
    fn saturation_ceiling_is_continuous() {
        for i in 0..=12 {
            let h = i as f64 * 30.0;
            let before = saturation_ceiling(h - 0.01);
            let after = saturation_ceiling(h + 0.01);
            assert!(
                (before - after).abs() < 0.01,
                "разрыв потолка насыщенности на {h}°"
            );
        }
    }

    #[test]
    fn extract_accent_color_returns_none_for_invalid_image_bytes() {
        assert_eq!(extract_accent_color(&[]), None);
        assert_eq!(extract_accent_color(b"not an image"), None);
    }

    #[test]
    fn rgb_hsl_round_trip_is_stable() {
        for (r, g, b) in [(255u8, 0u8, 0u8), (0, 255, 0), (0, 0, 255), (30, 144, 255), (128, 128, 128)] {
            let (h, s, l) = rgb_to_hsl(r, g, b);
            let (r2, g2, b2) = hsl_to_rgb(h, s, l);
            assert!((r as i16 - r2 as i16).abs() <= 1, "r drift: {r} vs {r2}");
            assert!((g as i16 - g2 as i16).abs() <= 1, "g drift: {g} vs {g2}");
            assert!((b as i16 - b2 as i16).abs() <= 1, "b drift: {b} vs {b2}");
        }
    }
}

#[cfg(test)]
mod eq_math_tests {
    use super::*;

    #[test]
    fn peaking_eq_is_exact_passthrough_for_near_zero_gain() {
        let c = BiquadCoeffs::peaking_eq(44_100.0, 1000.0, 0.0, 1.4);
        assert_eq!(c.b0, 1.0);
        assert_eq!(c.b1, 0.0);
        assert_eq!(c.b2, 0.0);
        assert_eq!(c.a1, 0.0);
        assert_eq!(c.a2, 0.0);

        let mut state = BiquadState::default();
        for i in 0..32 {
            let x = (i as f32 * 0.37).sin();
            assert_eq!(state.process(&c, x), x, "нейтральный фильтр не должен менять сигнал");
        }
    }

    fn steady_state_rms(coeffs: &BiquadCoeffs, freq: f32, sample_rate: f32, cycles: usize) -> f32 {
        let mut state = BiquadState::default();
        let samples_per_cycle = (sample_rate / freq).round() as usize;
        let total = samples_per_cycle * cycles;
        let warmup = total / 4;
        let mut sum_sq = 0.0f64;
        let mut counted = 0usize;
        for i in 0..total {
            let t = i as f32 / sample_rate;
            let x = (2.0 * std::f32::consts::PI * freq * t).sin();
            let y = state.process(coeffs, x);
            if i >= warmup {
                sum_sq += (y as f64) * (y as f64);
                counted += 1;
            }
        }
        ((sum_sq / counted.max(1) as f64).sqrt()) as f32
    }

    #[test]
    fn peaking_eq_boosts_signal_at_center_frequency() {
        let sample_rate = 44_100.0;
        let freq = 1000.0;
        let flat = BiquadCoeffs::peaking_eq(sample_rate, freq, 0.0, 1.4);
        let boosted = BiquadCoeffs::peaking_eq(sample_rate, freq, 6.0, 1.4);

        let flat_rms = steady_state_rms(&flat, freq, sample_rate, 40);
        let boosted_rms = steady_state_rms(&boosted, freq, sample_rate, 40);

        let ratio = boosted_rms / flat_rms;
        assert!(
            (1.7..2.3).contains(&ratio),
            "ожидали усиление ~x2.0 на центральной частоте, получили x{ratio}"
        );
    }

    #[test]
    fn peaking_eq_cut_attenuates_signal_at_center_frequency() {
        let sample_rate = 44_100.0;
        let freq = 1000.0;
        let flat = BiquadCoeffs::peaking_eq(sample_rate, freq, 0.0, 1.4);
        let cut = BiquadCoeffs::peaking_eq(sample_rate, freq, -6.0, 1.4);

        let flat_rms = steady_state_rms(&flat, freq, sample_rate, 40);
        let cut_rms = steady_state_rms(&cut, freq, sample_rate, 40);

        assert!(cut_rms < flat_rms, "срез на центральной частоте должен ослаблять сигнал");
    }

    #[test]
    fn peaking_eq_barely_affects_far_off_center_frequency() {
        let sample_rate = 44_100.0;
        let coeffs = BiquadCoeffs::peaking_eq(sample_rate, 60.0, 12.0, 1.4);
        let flat = BiquadCoeffs::peaking_eq(sample_rate, 60.0, 0.0, 1.4);

        let far_rms = steady_state_rms(&coeffs, 12_000.0, sample_rate, 40);
        let far_flat_rms = steady_state_rms(&flat, 12_000.0, sample_rate, 40);

        let ratio = far_rms / far_flat_rms;
        assert!(
            (0.9..1.1).contains(&ratio),
            "усиление баса не должно заметно влиять на 12кГц, x{ratio}"
        );
    }

    #[test]
    fn soft_clip_passes_through_within_threshold() {
        for x in [0.0f32, 0.5, -0.5, 0.97, -0.97] {
            assert_eq!(soft_clip(x), x);
        }
    }

    #[test]
    fn soft_clip_never_reaches_unity() {
        for x in [0.98f32, 1.0, 2.0, 10.0, 1000.0] {
            let y = soft_clip(x);
            assert!(y < 1.0, "soft_clip({x}) = {y} должен быть строго меньше 1.0");
        }
        for x in [-0.98f32, -1.0, -2.0, -10.0] {
            let y = soft_clip(x);
            assert!(y > -1.0, "soft_clip({x}) = {y} должен быть строго больше -1.0");
        }
    }

    #[test]
    fn soft_clip_is_smooth_just_above_threshold() {
        let y = soft_clip(0.98);
        assert!(y > 0.97 && y < 1.0, "y={y}");
    }

    #[test]
    fn soft_clip_is_gradual_over_wide_overdrive_range() {
        let moderate_overdrive = soft_clip(1.2);
        assert!(
            moderate_overdrive < 0.99,
            "ожидали плавное насыщение при x=1.2 (значение заметно ниже потолка), получили {moderate_overdrive}"
        );
    }

    #[test]
    fn soft_clip_is_odd_symmetric() {
        for x in [0.5f32, 0.97, 1.5, 5.0] {
            assert!((soft_clip(x) + soft_clip(-x)).abs() < 1e-6);
        }
    }

    #[test]
    fn soft_clip_is_monotonically_increasing() {
        let xs: Vec<f32> = (0..200).map(|i| -5.0 + i as f32 * 0.05).collect();
        let mut prev = soft_clip(xs[0]);
        for &x in &xs[1..] {
            let y = soft_clip(x);
            assert!(y >= prev, "soft_clip должен быть монотонным: {prev} -> {y} at x={x}");
            prev = y;
        }
    }

    #[test]
    fn eq_settings_default_is_flat_and_disabled() {
        let s = EqSettings::default();
        assert!(!s.enabled);
        assert_eq!(s.gain, 0.0);
        assert_eq!(s.bands.len(), 10);
        assert!(s.bands.iter().all(|&b| b == 0.0));
    }

    #[test]
    fn is_lossless_codec_classifies_known_codecs() {
        use symphonia::core::codecs::audio::well_known::*;
        assert!(is_lossless_codec(CODEC_ID_FLAC));
        assert!(is_lossless_codec(CODEC_ID_ALAC));
        assert!(is_lossless_codec(CODEC_ID_PCM_S16LE));
        assert!(!is_lossless_codec(CODEC_ID_MP3));
        assert!(!is_lossless_codec(CODEC_ID_AAC));
    }

    #[test]
    fn format_name_from_ext_maps_known_and_unknown_extensions() {
        assert_eq!(format_name_from_ext("flac"), "FLAC");
        assert_eq!(format_name_from_ext("mp3"), "MP3");
        assert_eq!(format_name_from_ext("opus"), "Opus");
        assert_eq!(format_name_from_ext("xyz"), "xyz");
    }

    #[test]
    fn supported_formats_includes_common_audio_types() {
        let formats = AudioPlayer::supported_formats();
        for f in ["flac", "mp3", "wav", "ogg", "m4a", "opus"] {
            assert!(formats.contains(&f), "{f} должен быть в списке поддерживаемых форматов");
        }
        assert!(!formats.contains(&"wma"), "wma не поддерживается symphonia/rodio в этом стеке");
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    use std::path::PathBuf as StdPathBuf;

    fn temp_path(name: &str) -> StdPathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("lfmusic_audio_player_{unique}_{name}"))
    }

    fn write_test_wav(path: &std::path::Path, sample_rate: u32, channels: u16, num_frames: u32) {
        let bits_per_sample: u16 = 16;
        let block_align: u16 = channels * (bits_per_sample / 8);
        let byte_rate: u32 = sample_rate * block_align as u32;
        let data_size: u32 = num_frames * block_align as u32;

        let mut buf = Vec::with_capacity(44 + data_size as usize);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_size).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits_per_sample.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        for i in 0..(num_frames as usize * channels as usize) {
            let sample = (((i % 200) as i32 - 100) * 300) as i16;
            buf.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, buf).expect("write test wav");
    }

    /// 24-bit PCM WAV with a full-scale-ish square wave.
    fn write_test_wav_24(path: &std::path::Path, sample_rate: u32, channels: u16, num_frames: u32) {
        let block_align: u16 = channels * 3;
        let data_size: u32 = num_frames * block_align as u32;
        let mut buf = Vec::with_capacity(44 + data_size as usize);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_size).to_le_bytes());
        buf.extend_from_slice(b"WAVEfmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&24u16.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        for i in 0..(num_frames as usize * channels as usize) {
            let v: i32 = if (i / 50) % 2 == 0 { 4_000_000 } else { -4_000_000 };
            buf.extend_from_slice(&v.to_le_bytes()[..3]);
        }
        std::fs::write(path, buf).expect("write test wav");
    }

    #[test]
    fn mix_editor_decodes_waveform_and_renders_stereo_preview() {
        let path = temp_path("mix-preview.wav");
        write_test_wav(&path, 8_000, 1, 16_000);
        let file = path.to_str().unwrap();
        let peaks = track_waveform(file, 2.0).unwrap();
        assert_eq!(peaks.len(), 8192 * 3);
        assert!(peaks.iter().any(|&peak| peak > 127));
        let region = PreviewRegion {
            from_path: file.into(), to_path: file.into(), from_start: 0.25, to_start: 0.5,
            lead: 0.25, overlap: 0.5, tail: 0.25,
            profile: MixProfile { curves: None, volume_curve: Some(MixVolume::Smooth), equal_power: true,
                eq_band: MixBand::CenterBass, filter: MixFilter::LowPass },
        };
        let pcm = render_mix_preview(region).unwrap();
        assert_eq!(pcm.len(), 32_000 * 4);
        for part in pcm.chunks(32_000) {
            assert!(part.as_chunks::<2>().0.iter().any(|s| i16::from_le_bytes(*s).unsigned_abs() > 100));
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn symphonia_source_decodes_24_bit_wav_as_audible_samples() {
        let path = temp_path("clip24.wav");
        write_test_wav_24(&path, 8_000, 2, 4_000);

        let samples: Vec<f32> = SymphoniaSource::new(path.to_str().unwrap())
            .expect("open 24-bit wav")
            .take(2_000)
            .collect();
        assert_eq!(samples.len(), 2_000);
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 4_000_000.0 / 8_388_607.0).abs() < 0.01, "peak={peak}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn get_metadata_for_missing_file_is_an_error() {
        let path = temp_path("missing.flac");
        let err = AudioPlayer::get_metadata(path.to_str().unwrap()).unwrap_err();
        assert!(err.contains("не найден") || err.contains("not found"), "err={err}");
    }

    #[test]
    fn get_metadata_light_reads_wav_properties_without_real_audio_fixture() {
        let path = temp_path("clip.wav");
        write_test_wav(&path, 8_000, 1, 8_000 * 2);

        let m = AudioPlayer::get_metadata_light(path.to_str().unwrap()).expect("metadata read");
        assert_eq!(m.format, "WAV");
        assert!(m.is_lossless, "WAV должен определяться как lossless");
        assert_eq!(m.sample_rate, Some(8_000));
        assert_eq!(m.channels, Some(1));
        let duration = m.duration_secs.expect("duration");
        assert!((duration - 2.0).abs() < 0.05, "duration={duration}");
        assert_eq!(m.cover_art, None, "get_metadata_light не должен читать обложку");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn get_metadata_for_untagged_wav_has_no_cover_or_accent() {
        let path = temp_path("untagged.wav");
        write_test_wav(&path, 44_100, 2, 44_100);

        let m = AudioPlayer::get_metadata(path.to_str().unwrap()).expect("metadata read");
        assert_eq!(m.channels, Some(2));
        assert_eq!(m.sample_rate, Some(44_100));
        assert_eq!(m.cover_art, None);
        assert_eq!(m.accent_color, None);
        assert!(!AudioPlayer::has_embedded_cover(path.to_str().unwrap()));
        assert_eq!(AudioPlayer::extract_cover_art(path.to_str().unwrap()), None);

        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod player_instance_tests {
    use super::*;

    // Real AudioPlayer: never play() an existing file here, it would play out loud during `cargo
    // test`.

    #[test]
    fn new_player_starts_stopped_with_full_volume() {
        let mut player = AudioPlayer::new().expect("audio output should initialize");
        let status = player.get_status();
        assert_eq!(status.state, PlaybackState::Stopped);
        assert_eq!(status.volume, 1.0);
        assert_eq!(status.current_file_path, None);
        assert_eq!(player.current_file_path(), None);
    }

    #[test]
    fn set_volume_clamps_to_unit_range() {
        let mut player = AudioPlayer::new().expect("audio output should initialize");
        player.set_volume(1.5);
        assert_eq!(player.get_status().volume, 1.0);
        player.set_volume(-0.5);
        assert_eq!(player.get_status().volume, 0.0);
        player.set_volume(0.42);
        assert_eq!(player.get_status().volume, 0.42);
    }

    #[test]
    fn play_missing_file_returns_error_and_does_not_change_state() {
        let mut player = AudioPlayer::new().expect("audio output should initialize");
        let result = player.play("Z:/definitely/not/a/real/path.flac");
        assert!(result.is_err());
        let status = player.get_status();
        assert_eq!(status.state, PlaybackState::Stopped);
        assert_eq!(status.current_file_path, None);
    }

    #[test]
    fn eq_settings_round_trip_through_setters_and_getters() {
        let player = AudioPlayer::new().expect("audio output should initialize");
        player.set_eq_enabled(true);
        player.set_eq_gain(4.0);
        player.set_eq_bands(vec![1.0, -2.0, 3.0, -4.0, 5.0, -6.0, 7.0, -8.0, 9.0, -10.0]);

        let settings = player.get_eq_settings();
        assert!(settings.enabled);
        assert_eq!(settings.gain, 4.0);
        assert_eq!(settings.bands, vec![1.0, -2.0, 3.0, -4.0, 5.0, -6.0, 7.0, -8.0, 9.0, -10.0]);
    }

    #[test]
    fn eq_bands_and_gain_are_clamped_to_plus_minus_12_db() {
        let player = AudioPlayer::new().expect("audio output should initialize");
        player.set_eq_bands(vec![100.0, -100.0]);
        player.set_eq_gain(50.0);

        let settings = player.get_eq_settings();
        assert_eq!(settings.bands[0], 12.0);
        assert_eq!(settings.bands[1], -12.0);
        assert_eq!(settings.gain, 12.0);

        player.set_eq_gain(-50.0);
        assert_eq!(player.get_eq_settings().gain, -12.0);
    }

    #[test]
    fn set_eq_settings_replaces_whole_struct() {
        let player = AudioPlayer::new().expect("audio output should initialize");
        let custom = EqSettings {
            enabled: true,
            bands: vec![2.0; 10],
            gain: -3.0,
        };
        player.set_eq_settings(custom.clone());
        let read_back = player.get_eq_settings();
        assert_eq!(read_back.enabled, custom.enabled);
        assert_eq!(read_back.bands, custom.bands);
        assert_eq!(read_back.gain, custom.gain);
    }
}

#[cfg(test)]
mod transition_tests {
    use super::*;

    #[test]
    fn outgoing_audio_ends_without_advancing_the_new_track() {
        let source = rodio::buffer::SamplesBuffer::new(nz_channels(1), nz_rate(100), vec![1.0; 200]);
        let signal = Arc::new(SinkEndSignal::new(false));
        let request = FadeRequest::default();
        let mut audio = AnalyzerSource::new(source, Arc::new(SpectrumAnalyzer::new()), signal.clone(), request.clone());
        assert_eq!(audio.next(), Some(1.0));
        let profile = MixProfile { curves: None, volume_curve: None, equal_power: false, eq_band: MixBand::Off, filter: MixFilter::Off };
        assert!(request.set((profile, 0.5)).is_ok());
        let tail: Vec<_> = audio.collect();
        assert_eq!(tail.len(), 50);
        assert!(tail[0] > 0.99 && tail[49] < 0.03);
        assert!(!signal.is_finished());
    }

    #[test]
    fn aligned_points_hold_then_step_on_the_selected_frame() {
        let points = Some([[0.0, 1.0], [0.5, 1.0], [0.5, 0.0]]);
        assert_eq!(curve_value(&points, 0.49999, 0.0), 1.0);
        assert_eq!(curve_value(&points, 0.5, 1.0), 0.0);
        assert_eq!(curve_value(&points, 1.0, 1.0), 0.0);
    }

    #[test]
    fn custom_points_control_gain_eq_and_filter_on_the_selected_lane() {
        let points = Some([[0.1, 0.2], [0.4, 0.8], [0.9, 0.4]]);
        let lane = LaneCurves { volume: points, eq: points, filter: points };
        let profile = MixProfile {
            curves: Some(Box::new([LaneCurves::default(), lane])),
            volume_curve: None, equal_power: false, eq_band: MixBand::Bass, filter: MixFilter::LowPass,
        };
        let mut input = MixEnvelope::new(profile.clone(), true, 1.0, 1, 100);
        let mut output = MixEnvelope::new(profile, false, 1.0, 1, 100);
        for _ in 0..=40 { input.sample(1.0); output.sample(1.0); }
        assert!((input.gain - 0.8).abs() < 1e-5);
        assert!((input.eq_amount - 0.2).abs() < 1e-5);
        assert!((input.weight - 0.8).abs() < 1e-5);
        assert!((output.gain - 0.6).abs() < 1e-5);
    }

    #[test]
    fn linear_mix_preserves_level_and_stereo_alignment() {
        let profile = MixProfile { curves: None, volume_curve: None, equal_power: false, eq_band: MixBand::Off, filter: MixFilter::Off };
        let mut incoming = MixEnvelope::new(profile.clone(), true, 1.0, 2, 100);
        let mut outgoing = MixEnvelope::new(profile, false, 1.0, 2, 100);
        for frame in 0..100 {
            for _ in 0..2 {
                let a = incoming.sample(1.0);
                let b = outgoing.sample(1.0);
                assert!((a - frame as f32 / 100.0).abs() < 1e-6);
                assert!((a + b - 1.0).abs() < 1e-6);
            }
        }
        assert_eq!(outgoing.frame, outgoing.frames);
    }
}
