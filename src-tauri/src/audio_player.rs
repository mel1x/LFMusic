use base64::Engine;

use crate::enhance::{EnhanceSettings, EnhanceShared, EnhanceSource, EnhanceStatus};
use crate::spectrum::SpectrumAnalyzer;
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::probe::Probe;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{CodecType, DecoderOptions};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

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
    /// Whether the file has any embedded cover picture (not the picture itself) —
    /// cached in `tracks.has_cover` so cover-presence checks (`get_cover_art`,
    /// external-prep) don't need to re-open/re-parse tags each time. Internal
    /// only, never sent to the frontend.
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
            // Passthrough
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

/// Soft limiter: passthrough below T=0.97, tanh saturation above it toward CEILING.
/// KNEE is deliberately decoupled from (CEILING - T): dividing `over` by that gap
/// directly used to saturate tanh to exactly 1.0 already at x≈1.2, making the
/// limiter effectively hard-clip for most of the loud range. CEILING < 1.0 so a
/// sample never hits exactly ±1.0 (guards against overflow if ever converted to int).
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
        let channels = source.channels();
        let sample_rate = source.sample_rate();
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

        // Check settings every ~1024 samples
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
    fn current_frame_len(&self) -> Option<usize> { self.source.current_frame_len() }
    fn channels(&self) -> u16 { self.channels }
    fn sample_rate(&self) -> u32 { self.sample_rate }
    fn total_duration(&self) -> Option<Duration> { self.source.total_duration() }
}

/// User EQ followed by auto enhancement.
fn effects<S: Source<Item = f32>>(
    source: S,
    eq_settings: &Arc<RwLock<EqSettings>>,
    enhance: &Arc<EnhanceShared>,
) -> EnhanceSource<EqSource<S>> {
    EnhanceSource::new(EqSource::new(source, eq_settings.clone()), enhance.clone())
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
    Resume,
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
        file_path: String,
        seek_secs: f64,
        volume: f32,
        crossfade_secs: f64,
        resp: Resp,
    },
}

struct SymphoniaSource {
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    format_reader: Box<dyn symphonia::core::formats::FormatReader>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
    current_samples: Vec<f32>,
    current_pos: usize,
}

impl SymphoniaSource {
    fn new(file_path: &str) -> Result<Self, String> {
        let file = File::open(file_path)
            .map_err(|e| format!("Не удалось открыть файл: {}", e))?;

        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = PathBuf::from(file_path).extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(|e| format!("Не удалось определить формат: {}", e))?;

        let format_reader = probed.format;

        let candidates: Vec<(u32, symphonia::core::codecs::CodecParameters)> = format_reader
            .tracks()
            .iter()
            .filter(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .map(|t| (t.id, t.codec_params.clone()))
            .collect();

        // A container can hold more than one non-null stream — e.g. some tools embed
        // cover art in OGG/Opus as a legacy MJPEG "video" stream (OGM-style) instead of
        // METADATA_BLOCK_PICTURE. CodecParameters doesn't distinguish audio/video, and
        // that stream can be listed first with no matching symphonia decoder, which used
        // to break playback of the whole file. Try each non-null track in turn and use
        // the first one that actually builds a decoder.
        let mut last_err = "Не найден аудио трек".to_string();
        let mut resolved = None;
        for (id, params) in candidates {
            match symphonia::default::get_codecs().make(&params, &DecoderOptions::default()) {
                Ok(decoder) => {
                    resolved = Some((id, params, decoder));
                    break;
                }
                Err(e) => last_err = format!("Не удалось создать декодер: {}", e),
            }
        }
        let (track_id, codec_params, decoder) = resolved.ok_or(last_err)?;
        let sample_rate = codec_params.sample_rate.unwrap_or(44100);
        let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);

        Ok(Self {
            decoder,
            format_reader,
            track_id,
            sample_rate,
            channels,
            current_samples: Vec::new(),
            current_pos: 0,
        })
    }

    fn new_with_seek(file_path: &str, seek_secs: f64) -> Result<Self, String> {
        let mut source = Self::new(file_path)?;

        if seek_secs > 0.0 {
            use symphonia::core::formats::{SeekMode, SeekTo};
            use symphonia::core::units::Time;

            source
                .format_reader
                .seek(
                    SeekMode::Accurate,
                    SeekTo::Time {
                        time: Time::from(seek_secs),
                        track_id: Some(source.track_id),
                    },
                )
                .map_err(|e| format!("Ошибка seek: {}", e))?;

            source.decoder.reset();
            source.current_samples.clear();
            source.current_pos = 0;
        }

        Ok(source)
    }

    fn decode_next_packet(&mut self) -> bool {
        loop {
            match self.format_reader.next_packet() {
                Ok(packet) => {
                    if packet.track_id() != self.track_id {
                        continue;
                    }
                    match self.decoder.decode(&packet) {
                        Ok(decoded) => {
                            self.current_samples.clear();
                            self.current_pos = 0;
                            extract_samples_into(&decoded, &mut self.current_samples);
                            return true;
                        }
                        Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                        Err(_) => return false,
                    }
                }
                Err(_) => return false,
            }
        }
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
    fn current_frame_len(&self) -> Option<usize> {
        let remaining = self.current_samples.len() - self.current_pos;
        if remaining > 0 { Some(remaining) } else { None }
    }
    fn channels(&self) -> u16 { self.channels }
    fn sample_rate(&self) -> u32 { self.sample_rate }
    fn total_duration(&self) -> Option<Duration> { None }
}

/// Ring-buffer depth in decoded packet-blocks (FLAC ~4096 frames, MP3 ~1152, Opus ~960
/// per block). 64 blocks give ~1-3s of read-ahead to absorb slow disk I/O (USB/network
/// drives) while staying under ~2MB.
const DECODE_QUEUE_BLOCKS: usize = 64;

/// Pipelined source: packet decoding runs on a background `audio-decode` thread that
/// fills a bounded SPSC queue; `next()` only pulls ready blocks, so slow disk reads or
/// decoding high-bitrate FLAC/WAV no longer cause audio glitches.
///
/// Lifetime = one playback span. On track change/seek/stop, rodio drops the source,
/// closing `rx`; the background thread's blocking `send()` then errors out and the
/// thread exits on its own — no separate stop flag or `join` needed.
struct BufferedSymphoniaSource {
    sample_rate: u32,
    channels: u16,
    rx: mpsc::Receiver<Vec<f32>>,
    /// Spent blocks go back to the decode thread for reuse, so the real-time output
    /// callback never frees memory and the decoder doesn't allocate per packet.
    recycle: mpsc::SyncSender<Vec<f32>>,
    current: Vec<f32>,
    pos: usize,
}

impl BufferedSymphoniaSource {
    fn new_with_seek(file_path: &str, seek_secs: f64) -> Result<Self, String> {
        // Open/decode synchronously so sample rate and channel count are known
        // immediately (rodio needs them before the thread starts), and open/seek
        // errors still surface to the caller directly.
        let mut inner = SymphoniaSource::new_with_seek(file_path, seek_secs)?;
        let sample_rate = inner.sample_rate;
        let channels = inner.channels;

        // Bounded queue = backpressure: decoder blocks on `send()` when full,
        // consumer blocks on `recv()` when empty.
        let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(DECODE_QUEUE_BLOCKS);
        let (recycle, recycled) = mpsc::sync_channel::<Vec<f32>>(DECODE_QUEUE_BLOCKS);

        std::thread::Builder::new()
            .name("audio-decode".into())
            .spawn(move || {
                loop {
                    // Decode into a spent block when one is back (keeps its capacity).
                    if let Ok(buf) = recycled.try_recv() {
                        inner.current_samples = buf;
                    }
                    if !inner.decode_next_packet() {
                        break;
                    }
                    // Take without copying; the consumer sends the buffer back once played.
                    let block = std::mem::take(&mut inner.current_samples);
                    if block.is_empty() {
                        continue;
                    }
                    if tx.send(block).is_err() {
                        return; // consumer gone (track change/seek/stop)
                    }
                }
                // EOF/fatal decode error: dropping `tx` here lets the consumer see
                // `Err` from `recv()` once it drains the rest of the queue.
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
            // Blocks only happen under starvation; normally the buffer stays non-empty.
            match self.rx.recv() {
                Ok(block) => {
                    let spent = std::mem::replace(&mut self.current, block);
                    if spent.capacity() > 0 {
                        // Non-blocking; if the decoder is gone or the queue is full, the
                        // buffer is simply dropped.
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
    fn current_frame_len(&self) -> Option<usize> {
        let remaining = self.current.len() - self.pos;
        if remaining > 0 { Some(remaining) } else { None }
    }
    fn channels(&self) -> u16 { self.channels }
    fn sample_rate(&self) -> u32 { self.sample_rate }
    fn total_duration(&self) -> Option<Duration> { None }
}

struct FadeOut<S: Source<Item = f32>> {
    source: S,
    total_samples: usize,
    elapsed: usize,
}

impl<S: Source<Item = f32>> FadeOut<S> {
    fn new(source: S, duration: Duration) -> Self {
        let total_samples = (source.sample_rate() as f64 * duration.as_secs_f64()) as usize
            * source.channels() as usize;
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
    fn current_frame_len(&self) -> Option<usize> { self.source.current_frame_len() }
    fn channels(&self) -> u16 { self.source.channels() }
    fn sample_rate(&self) -> u32 { self.source.sample_rate() }
    fn total_duration(&self) -> Option<Duration> { None }
}

/// Shared between the audio thread and `AudioPlayer`: an atomic flag for cheap
/// lock-free reads (`get_status` polling) plus a condvar so the track-end watcher
/// thread can block instead of spin-polling, waking almost immediately when a
/// track's samples are exhausted (`set_finished`) rather than up to 500ms late.
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

    /// Marks the sink empty and wakes anyone blocked in `wait`. Called both on
    /// natural EOF (from the audio thread) and on an explicit `Stop`.
    fn set_finished(&self) {
        self.empty.store(true, Ordering::Relaxed);
        let guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.cvar.notify_all();
        drop(guard);
    }

    fn is_finished(&self) -> bool {
        self.empty.load(Ordering::Relaxed)
    }

    /// Blocks until `set_finished` notifies or `timeout` elapses (safety net against
    /// a wakeup lost to the classic check/wait race — worst case, same latency as
    /// the old fixed-interval poll).
    pub fn wait(&self, timeout: Duration) {
        let guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.cvar.wait_timeout(guard, timeout);
    }
}

/// Transparent tap: passes samples through unchanged while mixing frames to mono and
/// feeding blocks to the shared `SpectrumAnalyzer` for FFT visualization. Placed above
/// EQ (and fade-in) so the spectrum reflects what's actually audible.
struct AnalyzerSource<S: Source<Item = f32>> {
    source: S,
    analyzer: Arc<SpectrumAnalyzer>,
    /// Set on EOF; mirrored in `AudioPlayer::sink_empty` so the main thread can poll
    /// via a plain atomic load, and so the track-end watcher thread wakes promptly.
    finished: Arc<SinkEndSignal>,
    channels: u16,
    sample_rate: u32,
    ch_idx: u16,
    frame_sum: f32,
    block: Vec<f32>,
}

impl<S: Source<Item = f32>> AnalyzerSource<S> {
    /// Mono samples per analyzer push — batches the mutex lock (~once per 256 frames)
    /// while staying smooth at 60fps.
    const BLOCK: usize = 256;

    fn new(source: S, analyzer: Arc<SpectrumAnalyzer>, finished: Arc<SinkEndSignal>) -> Self {
        let channels = source.channels().max(1);
        let sample_rate = source.sample_rate();
        Self {
            source,
            analyzer,
            finished,
            channels,
            sample_rate,
            ch_idx: 0,
            frame_sum: 0.0,
            block: Vec::with_capacity(Self::BLOCK),
        }
    }
}

impl<S: Source<Item = f32>> Iterator for AnalyzerSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = match self.source.next() {
            Some(s) => s,
            None => {
                // Reset to `false` when a new sink is created in the audio thread.
                self.finished.set_finished();
                return None;
            }
        };

        if !self.analyzer.is_enabled() {
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
    fn current_frame_len(&self) -> Option<usize> { self.source.current_frame_len() }
    fn channels(&self) -> u16 { self.channels }
    fn sample_rate(&self) -> u32 { self.sample_rate }
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

/// Builds a saturation-weighted hue histogram: the winning hue is the one with the
/// largest "visible" color mass (area x saturation), not just the most vivid pixel.
/// Yellow/acid tones are excluded or muted since they make poor UI accent colors.
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
        // Near-gray and extreme dark/light pixels carry no meaningful hue
        if s < 0.12 || !(0.06..=0.94).contains(&l) {
            continue;
        }
        // Visible color mass: more saturated + closer to mid-lightness = more noticeable
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

    // Near-monochrome cover: fall back to neutral gray so stray color noise
    // doesn't tint the whole UI.
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

    // Only keep hues with >= 6% of the color mass: a tiny vivid spot shouldn't win.
    let mut pool: Vec<usize> = (0..HUE_BINS)
        .filter(|&i| windows[i].weight >= chroma_weight * 0.06)
        .collect();
    if pool.is_empty() {
        pool = (0..HUE_BINS).filter(|&i| windows[i].weight > 0.0).collect();
    }

    // Drop yellow if a non-yellow alternative exists; if the whole cover is yellow,
    // keep it — normalize_accent_hsl() will shift the hue out of the zone later.
    let non_yellow: Vec<usize> = pool
        .iter()
        .copied()
        .filter(|&i| !is_yellow_hue(windows[i].hue))
        .collect();
    if !non_yellow.is_empty() {
        pool = non_yellow;
    }

    // Dominance is weighted first, color quality second: a rare vivid hue
    // shouldn't outrank the cover's main tone.
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

/// Piecewise-linear saturation ceiling by hue: "toxic" zones (lime, neon green,
/// fuchsia) get clamped harder than calm blues.
fn saturation_ceiling(hue: f64) -> f64 {
    // Control points every 30°, starting at red (0°/360°).
    const POINTS: [f64; 13] = [
        0.60, // 0    red
        0.58, // 30   orange
        0.45, // 60   yellow (rarely hit after the hue shift)
        0.42, // 90   lime/olive
        0.45, // 120  green
        0.48, // 150  emerald
        0.52, // 180  cyan
        0.58, // 210  azure
        0.62, // 240  blue
        0.60, // 270  violet
        0.52, // 300  magenta
        0.56, // 330  pink
        0.60, // 360 = 0
    ];
    let h = hue.rem_euclid(360.0);
    let seg = ((h / 30.0) as usize).min(11);
    let t = (h - seg as f64 * 30.0) / 30.0;
    POINTS[seg] + (POINTS[seg + 1] - POINTS[seg]) * t
}

/// Fits a color to the UI background: excludes yellow, clamps lightness to a narrow
/// range, and caps saturation by hue "toxicity" while keeping the cover's character.
fn normalize_accent_hsl(hue: f64, sat: f64, light: f64) -> (u8, u8, u8) {
    let mut hue = hue.rem_euclid(360.0);
    let mut sat = sat.clamp(0.0, 1.0);

    // No yellow as a UI color: shift the warm half toward amber, the cool
    // (lime) half toward muted olive.
    if (YELLOW_FROM..62.0).contains(&hue) {
        hue = 36.0 + (hue - YELLOW_FROM) * (6.0 / (62.0 - YELLOW_FROM));
    } else if (62.0..YELLOW_TO).contains(&hue) {
        hue = 88.0 + (hue - 62.0) * (7.0 / (YELLOW_TO - 62.0));
    }

    // Narrow lightness range so the accent and derived backgrounds
    // (--dark-bg/--light-bg) look equally dense across any cover.
    let light = light.clamp(0.32, 0.48);

    // Soft-compress saturation toward the hue ceiling: removes acidity while
    // still distinguishing muted vs. vivid covers.
    let ceiling = saturation_ceiling(hue);
    let knee = ceiling * 0.8;
    if sat > knee {
        sat = knee + (sat - knee) * 0.35;
    }
    sat = sat.min(ceiling);

    hsl_to_rgb(hue, sat, light)
}

/// Whether a cover is embedded + its accent color (computed once, cached in the DB).
/// The full base64 cover is no longer produced here — display uses a WebP thumbnail
/// served via the `lfmusic-cover` protocol (see `thumbnails`).
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

/// Runs `$m!(buffer, conv)` for whichever sample format `$buf` holds, where `conv`
/// maps one sample to f32 in [-1, 1]. Covers every symphonia format: a missing arm
/// used to decode e.g. 24-bit WAV/AIFF or 64-bit float WAV as silence.
macro_rules! dispatch_f32 {
    ($buf:expr, $m:ident) => {{
        use symphonia::core::audio::AudioBufferRef as B;
        use symphonia::core::sample::{i24, u24};
        match $buf {
            B::F32(b) => $m!(b, |s: f32| s),
            B::F64(b) => $m!(b, |s: f64| s as f32),
            B::S32(b) => $m!(b, |s: i32| s as f32 / i32::MAX as f32),
            B::S24(b) => $m!(b, |s: i24| s.inner() as f32 / 8_388_607.0),
            B::S16(b) => $m!(b, |s: i16| s as f32 / i16::MAX as f32),
            B::S8(b) => $m!(b, |s: i8| s as f32 / i8::MAX as f32),
            B::U32(b) => $m!(b, |s: u32| ((s as f64 - 2_147_483_648.0) / 2_147_483_648.0) as f32),
            B::U24(b) => $m!(b, |s: u24| (s.inner() as f32 - 8_388_608.0) / 8_388_608.0),
            B::U16(b) => $m!(b, |s: u16| (s as f32 - 32_768.0) / 32_768.0),
            B::U8(b) => $m!(b, |s: u8| (s as f32 - 128.0) / 128.0),
        }
    }};
}
pub(crate) use dispatch_f32;

/// Appends `buf` to `samples` as interleaved f32.
fn extract_samples_into(buf: &AudioBufferRef, samples: &mut Vec<f32>) {
    macro_rules! extract {
        ($b:expr, $conv:expr) => {{
            let b = $b;
            let frames = b.frames();
            let channels = b.spec().channels.count();
            samples.reserve(frames * channels);
            // Mono/stereo fast paths: slices hoisted out of the loop, no per-sample
            // plane lookup or bounds check.
            match channels {
                1 => samples.extend(b.chan(0)[..frames].iter().map(|&s| $conv(s))),
                2 => {
                    let (l, r) = (&b.chan(0)[..frames], &b.chan(1)[..frames]);
                    for (&a, &c) in l.iter().zip(r) {
                        samples.push($conv(a));
                        samples.push($conv(c));
                    }
                }
                _ => {
                    for frame in 0..frames {
                        for ch in 0..channels {
                            samples.push($conv(b.chan(ch)[frame]));
                        }
                    }
                }
            }
        }};
    }
    dispatch_f32!(buf, extract)
}

/// Caches probe_actual_codec by (path, mtime). M4A/MP4 is the only format needing a
/// second file open to disambiguate the codec (AAC vs ALAC); play() re-probes the
/// same file often, so this avoids re-parsing the container each time.
fn probe_cache() -> &'static Mutex<HashMap<(String, u64), CodecType>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, u64), CodecType>>> = OnceLock::new();
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
fn probe_actual_codec(file_path: &str, extension: &str) -> Option<CodecType> {
    let mtime = file_mtime_secs(std::path::Path::new(file_path));
    let key = (file_path.to_string(), mtime);

    if let Ok(cache) = probe_cache().lock() {
        if let Some(&codec) = cache.get(&key) {
            return Some(codec);
        }
    }

    let file = File::open(file_path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(extension);

    let codec = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()
        .and_then(|probed| {
            probed
                .format
                .tracks()
                .iter()
                .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
                .map(|t| t.codec_params.codec)
        })?;

    if let Ok(mut cache) = probe_cache().lock() {
        cache.insert(key, codec);
    }
    Some(codec)
}

fn is_lossless_codec(codec: CodecType) -> bool {
    matches!(
        codec,
        symphonia::core::codecs::CODEC_TYPE_FLAC
            | symphonia::core::codecs::CODEC_TYPE_ALAC
            | symphonia::core::codecs::CODEC_TYPE_PCM_S16LE
            | symphonia::core::codecs::CODEC_TYPE_PCM_S16BE
            | symphonia::core::codecs::CODEC_TYPE_PCM_S24LE
            | symphonia::core::codecs::CODEC_TYPE_PCM_S32LE
            | symphonia::core::codecs::CODEC_TYPE_PCM_F32LE
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

/// Play/seek commands stop the old sink before opening the new source. When the
/// open fails, nothing is playing any more: drop the sinks and mark the end signal,
/// otherwise the stopped source never reports EOF and the player stays "Playing".
fn stop_after_failed_open(
    sink: &mut Option<Sink>,
    fadeout_sink: &mut Option<Sink>,
    spectrum: &SpectrumAnalyzer,
    sink_empty: &SinkEndSignal,
) {
    if let Some(s) = sink.take() { s.stop(); }
    if let Some(s) = fadeout_sink.take() { s.stop(); }
    spectrum.set_active(false);
    sink_empty.set_finished();
}

/// Main loop of the dedicated audio thread; owns OutputStream (and the Windows COM
/// context), OutputStreamHandle, and the current Sink.
fn audio_thread_loop(
    rx: mpsc::Receiver<AudioCmd>,
    _stream: OutputStream, // must stay alive for the thread's lifetime
    stream_handle: OutputStreamHandle,
    eq_settings: Arc<RwLock<EqSettings>>,
    enhance: Arc<EnhanceShared>,
    spectrum: Arc<SpectrumAnalyzer>,
    sink_empty: Arc<SinkEndSignal>,
) {
    let mut sink: Option<Sink> = None;
    // A new crossfade immediately stops the previous fade-out sink, capping
    // concurrent fade streams at one.
    let mut fadeout_sink: Option<Sink> = None;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            AudioCmd::PlayFrom { file_path, seek_secs, volume, resp } => {
                if let Some(ref s) = sink { s.stop(); }
                let result = (|| -> Result<(), String> {
                    let new_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Не удалось создать sink: {}", e))?;
                    new_sink.set_volume(volume);
                    let source = BufferedSymphoniaSource::new_with_seek(&file_path, seek_secs)?;
                    sink_empty.set_playing();
                    new_sink.append(AnalyzerSource::new(
                        effects(source, &eq_settings, &enhance),
                        spectrum.clone(),
                        sink_empty.clone(),
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
                if let Some(ref s) = sink {
                    if !s.is_paused() { s.pause(); }
                }
            }

            AudioCmd::Resume => {
                if let Some(ref s) = sink {
                    if s.is_paused() { s.play(); }
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
                if let Some(ref s) = sink { s.set_volume(vol); }
            }

            AudioCmd::Seek { file_path, position_secs, volume, was_paused, resp } => {
                if let Some(ref s) = sink { s.stop(); }
                let result = (|| -> Result<(), String> {
                    let new_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Не удалось создать sink: {}", e))?;
                    new_sink.set_volume(volume);
                    let source = BufferedSymphoniaSource::new_with_seek(&file_path, position_secs)?;
                    sink_empty.set_playing();
                    new_sink.append(AnalyzerSource::new(
                        effects(source, &eq_settings, &enhance),
                        spectrum.clone(),
                        sink_empty.clone(),
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

                    let fade_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Ошибка fade-out sink: {}", e))?;
                    fade_sink.set_volume(volume);
                    let fade_source = BufferedSymphoniaSource::new_with_seek(&file_path, current_pos)?;
                    fade_sink.append(FadeOut::new(
                        effects(fade_source, &eq_settings, &enhance),
                        dur,
                    ));

                    if let Some(ref old) = sink { old.stop(); }
                    // rodio keeps playing this sink in the background
                    fadeout_sink = Some(fade_sink);

                    let new_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Ошибка нового sink: {}", e))?;
                    new_sink.set_volume(volume);
                    let source = BufferedSymphoniaSource::new_with_seek(&file_path, position_secs)?;
                    sink_empty.set_playing();
                    new_sink.append(AnalyzerSource::new(
                        effects(source, &eq_settings, &enhance).fade_in(dur),
                        spectrum.clone(),
                        sink_empty.clone(),
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
                file_path, seek_secs, volume, crossfade_secs, resp,
            } => {
                let result = (|| -> Result<(), String> {
                    let dur = Duration::from_secs_f64(crossfade_secs);

                    if let Some(old_fade) = fadeout_sink.take() { old_fade.stop(); }

                    // fade-in on the new track carries the transition
                    if let Some(old_sink) = sink.take() { old_sink.stop(); }

                    let new_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Ошибка нового sink: {}", e))?;
                    new_sink.set_volume(volume);
                    let source = BufferedSymphoniaSource::new_with_seek(&file_path, seek_secs)?;
                    sink_empty.set_playing();
                    new_sink.append(AnalyzerSource::new(
                        effects(source, &eq_settings, &enhance).fade_in(dur),
                        spectrum.clone(),
                        sink_empty.clone(),
                    ));
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

/// `OutputStream` (cpal, COM on Windows) lives on a dedicated `audio-output` thread.
/// All Sink operations proxy through an `mpsc` channel, avoiding the need for
/// `unsafe impl Send/Sync`.
pub struct AudioPlayer {
    cmd_tx: mpsc::Sender<AudioCmd>,
    current_track: Option<TrackMetadata>,
    state: PlaybackState,
    volume: f32,
    playback_started_at: Option<Instant>,
    accumulated_position: f64,
    eq_settings: Arc<RwLock<EqSettings>>,
    enhance: Arc<EnhanceShared>,
    spectrum: Arc<SpectrumAnalyzer>,
    /// Whether the current sink is empty (track finished/stopped). Updated
    /// asynchronously by the audio thread so the main thread can poll status
    /// without a blocking channel round-trip; also wakes the track-end watcher.
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
        let spectrum = Arc::new(SpectrumAnalyzer::new());
        let spectrum_for_thread = spectrum.clone();
        // true = nothing playing; set false when a track starts.
        let sink_empty = Arc::new(SinkEndSignal::new(true));
        let sink_empty_for_thread = sink_empty.clone();

        std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || {
                match OutputStream::try_default() {
                    Ok((stream, handle)) => {
                        let (device_name, profile) = crate::enhance::detect_output_device();
                        enhance_for_thread.set_detected(device_name, profile);
                        let _ = init_tx.send(Ok(()));
                        audio_thread_loop(
                            cmd_rx,
                            stream,
                            handle,
                            eq_for_thread,
                            enhance_for_thread,
                            spectrum_for_thread,
                            sink_empty_for_thread,
                        );
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(format!(
                            "Не удалось инициализировать аудио выход: {}",
                            e
                        )));
                    }
                }
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

    /// Like `send`, for play/seek commands: on failure the audio thread has already
    /// stopped output (see `stop_after_failed_open`), so the state follows it.
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

    /// Arc handle for the track-end watcher thread (spawned once in `run()`), so it
    /// can block on the condvar without needing to lock `AudioPlayer` itself.
    pub fn sink_end_signal(&self) -> Arc<SinkEndSignal> {
        self.sink_empty.clone()
    }

    /// Opens a file for tag/property reading, detecting type by content.
    ///
    /// Uses `guess_file_type` (magic bytes) instead of extension-based
    /// `Probe::open(...).read()`: yt-dlp remuxes Opus into a `.ogg`-named container,
    /// so extension-based probing expects Vorbis, fails to find its signature, and
    /// errors with `FileDecoding(Vorbis: "File missing magic signature")` — losing
    /// tags/cover for those tracks. Content-based detection fixes this while still
    /// working normally when the extension matches (used as a fallback).
    pub(crate) fn read_tagged_file(path: &std::path::Path) -> Option<lofty::file::TaggedFile> {
        Probe::open(path).ok()?.guess_file_type().ok()?.read().ok()
    }

    /// Full-size cover as a data URL. Only used for the "reset cover" preview in the
    /// edit modal (needs the tag original, bypassing any user override).
    pub fn extract_cover_art(file_path: &str) -> Option<String> {
        let tagged_file = Self::read_tagged_file(&PathBuf::from(file_path))?;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())?;
        cover_art_from_tag(tag)
    }

    /// Cheap check — doesn't extract picture bytes.
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

    /// No cover/accent color — used for library scanning.
    pub fn get_metadata_light(file_path: &str) -> Result<TrackMetadata, String> {
        Self::get_metadata_inner(file_path, false)
    }

    /// For unambiguous formats (FLAC, MP3, WAV, OGG, ...) opens the file once via lofty.
    /// Symphonia probing only kicks in for M4A/MP4, where the codec is ambiguous
    /// (AAC vs ALAC).
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
            let lossless = probe_actual_codec(file_path, &extension)
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
                    // Presence check is a free byproduct of the tag already being
                    // parsed (no picture bytes touched) — computed even in "light"
                    // (with_cover=false) scan mode so it can be cached in the DB.
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
    /// Lets callers do the tag/cover parsing before taking the player lock.
    pub fn play_preloaded(&mut self, file_path: &str, metadata: TrackMetadata) -> Result<TrackMetadata, String> {
        self.play_from(file_path, 0.0, metadata)
    }

    /// Loads, seeks, and immediately pauses — used to restore a saved session.
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
        metadata: TrackMetadata,
    ) -> Result<TrackMetadata, String> {
        if crossfade_secs <= 0.0 || self.state == PlaybackState::Stopped {
            return self.play_from(file_path, 0.0, metadata);
        }

        let volume = self.volume;
        self.send_play(|resp| AudioCmd::PlayCrossfade {
            file_path: file_path.to_string(),
            seek_secs: 0.0,
            volume,
            crossfade_secs,
            resp,
        })?;

        self.current_track = Some(metadata.clone());
        self.state = PlaybackState::Playing;
        self.playback_started_at = Some(Instant::now());
        self.accumulated_position = 0.0;

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
                self.accumulated_position += started.elapsed().as_secs_f64();
            }
            self.state = PlaybackState::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.state == PlaybackState::Paused {
            self.fire(AudioCmd::Resume);
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
                started.elapsed().as_secs_f64()
            }
            _ => 0.0,
        };
        let pos = self.accumulated_position + elapsed;
        if let Some(ref track) = self.current_track {
            if let Some(duration) = track.duration_secs {
                return pos.min(duration);
            }
        }
        pos
    }

    /// Used e.g. to stop playback before deleting a downloaded file if it's the one
    /// currently playing — the decoder holds a file handle otherwise blocking deletion.
    pub fn current_file_path(&self) -> Option<String> {
        self.current_track.as_ref().map(|t| t.file_path.clone())
    }

    /// Detects a naturally-finished sink (EOF, not a user-initiated stop) and
    /// transitions state accordingly. Returns the path that just finished, once,
    /// so callers (status polling, the track-end watcher thread) don't double-fire —
    /// after the transition `current_track` is `None`, so a repeat call is a no-op.
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
        // 90% muted blue background + a thin neon-red stripe: blue should win.
        let blue = hsl_to_rgb(220.0, 0.35, 0.40);
        let red = hsl_to_rgb(0.0, 1.0, 0.50);
        let img = img_from_fn(|x, _| if x < 86 { blue } else { red });
        let (h, _, _) = accent_hsl(&img);
        assert!((190.0..260.0).contains(&h), "ожидался синий, получен hue={h}");
    }

    #[test]
    fn yellow_skipped_when_alternative_exists() {
        // Yellow covers more area, but a noticeable blue alternative should win.
        let yellow = hsl_to_rgb(55.0, 0.80, 0.50);
        let blue = hsl_to_rgb(230.0, 0.60, 0.45);
        let img = img_from_fn(|x, _| if x < 58 { yellow } else { blue });
        let (h, _, _) = accent_hsl(&img);
        assert!((200.0..260.0).contains(&h), "ожидался синий, получен hue={h}");
    }

    #[test]
    fn fully_yellow_cover_shifts_out_of_yellow_zone() {
        // All-yellow cover: hue shifts to amber, accent must not stay yellow.
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
        // Hue is preserved, but saturation gets clamped by the zone ceiling.
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
        // No jumps at control-point boundaries.
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

    /// Steady-state RMS response of the biquad filter to a sine at `freq`,
    /// measured after a warmup period to let the transient settle.
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

        // +6dB ~ x1.995 amplitude at the resonant frequency.
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
        // A boosted 60Hz band shouldn't noticeably affect 12kHz.
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
        // CEILING < 1.0 keeps output strictly inside (-1.0, 1.0) even when tanh
        // saturates to 1.0 in f32 — this is the overflow guard for int conversion.
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

    /// Regression test: `over = (a - T) / (1.0 - T)` used to divide by a narrow gap
    /// (0.03), saturating tanh to exactly 1.0 in f32 already at x≈1.2 — the limiter
    /// was effectively hard-clipping most of the loud range. KNEE is now decoupled
    /// from the ceiling gap, so moderate overdrive stays well below the ceiling.
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
        use symphonia::core::codecs::*;
        assert!(is_lossless_codec(CODEC_TYPE_FLAC));
        assert!(is_lossless_codec(CODEC_TYPE_ALAC));
        assert!(is_lossless_codec(CODEC_TYPE_PCM_S16LE));
        assert!(!is_lossless_codec(CODEC_TYPE_MP3));
        assert!(!is_lossless_codec(CODEC_TYPE_AAC));
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

    /// Minimal valid 16-bit PCM WAV (no tags) — enough for lofty/symphonia to read
    /// format/properties without external audio fixtures.
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
        write_test_wav(&path, 8_000, 1, 8_000 * 2); // 2s mono @ 8kHz

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
        write_test_wav(&path, 44_100, 2, 44_100); // 1s stereo

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

    // These tests spin up a real `AudioPlayer` (real audio device) but must never call
    // play()/play_paused()/play_crossfade() on an existing file — that would actually
    // play audio out loud during `cargo test`. Only pre-playback state/control is tested.

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
