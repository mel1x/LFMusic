use base64::Engine;

use crate::spectrum::SpectrumAnalyzer;
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::probe::Probe;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{CodecType, DecoderOptions};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

// ── Публичные типы ──────────────────────────────────────────────────────────

/// Метаданные трека
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
    /// Обложка трека в виде data URL (data:image/jpeg;base64,...)
    pub cover_art: Option<String>,
    /// Акцентный цвет обложки в формате hex (#RRGGBB)
    pub accent_color: Option<String>,
    pub bpm: Option<f32>,
    pub loudness: Option<f32>,
    pub energy: Option<f32>,
    pub danceability: Option<f32>,
    pub spectral_centroid: Option<f32>,
    pub key: Option<String>,
    pub audio_fingerprint: Option<String>,
}

/// Состояние воспроизведения
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Лёгкий статус для частого поллинга (без полных метаданных трека)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStatus {
    pub state: PlaybackState,
    pub volume: f32,
    pub position_secs: f64,
    /// Путь текущего трека — для определения смены трека на фронте
    pub current_file_path: Option<String>,
}

// ── Эквалайзер ─────────────────────────────────────────────────────────────

/// Центральные частоты 10-полосного эквалайзера (Гц)
pub const EQ_BANDS: [f32; 10] = [60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0];

/// Настройки эквалайзера, разделяемые между AudioPlayer и EqSource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqSettings {
    /// Включён ли эквалайзер
    pub enabled: bool,
    /// Усиление каждой полосы в dB (от -12 до +12)
    pub bands: Vec<f32>,
    /// Общий уровень (preamp) в dB (от -12 до +12)
    #[serde(default)]
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

/// Коэффициенты биквадратного фильтра (Direct Form I)
#[derive(Debug, Clone, Copy)]
struct BiquadCoeffs {
    b0: f32, b1: f32, b2: f32,
    a1: f32, a2: f32,
}

impl BiquadCoeffs {
    /// Рассчитать коэффициенты peaking EQ фильтра
    fn peaking_eq(sample_rate: f32, center_freq: f32, gain_db: f32, q: f32) -> Self {
        if gain_db.abs() < 0.01 {
            // Нейтральный фильтр — пропускает без изменений
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

/// Состояние одного канала биквадратного фильтра
#[derive(Debug, Clone, Copy, Default)]
struct BiquadState {
    x1: f32, x2: f32,
    y1: f32, y2: f32,
}

/// Мягкий лимитер: линейный при |x| <= T, ниже — плавное насыщение через tanh.
/// При |x| <= 0.97 эффекта нет; выше — плавно сжимается к ±1, никогда не превышая.
#[inline(always)]
fn soft_clip(x: f32) -> f32 {
    const T: f32 = 0.97;
    let a = x.abs();
    if a <= T {
        x
    } else {
        let sign = if x >= 0.0 { 1.0 } else { -1.0 };
        let over = (a - T) / (1.0 - T);
        sign * (T + (1.0 - T) * over.tanh())
    }
}

impl BiquadState {
    fn process(&mut self, coeffs: &BiquadCoeffs, input: f32) -> f32 {
        let output = coeffs.b0 * input + coeffs.b1 * self.x1 + coeffs.b2 * self.x2
            - coeffs.a1 * self.y1 - coeffs.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

/// Source-обёртка, применяющая 10-полосный параметрический EQ в реальном времени.
/// Читает настройки из разделяемого Arc<RwLock<EqSettings>>.
struct EqSource<S: Source<Item = f32>> {
    source: S,
    eq_settings: Arc<RwLock<EqSettings>>,
    /// Текущие коэффициенты — пересчитываются при изменении настроек
    coeffs: Vec<BiquadCoeffs>,
    /// Состояние фильтров: [band][channel]
    states: Vec<Vec<BiquadState>>,
    /// Кэш последних значений bands для детекции изменений
    cached_bands: Vec<f32>,
    cached_enabled: bool,
    /// Кэш общего уровня в dB
    cached_gain_db: f32,
    /// Общий уровень (линейный множитель)
    cached_gain_linear: f32,
    /// Счётчик сэмплов для периодической проверки настроек
    samples_since_check: usize,
    /// Счётчик текущего канала (interleaved: 0, 1, 0, 1, ...)
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

    /// Проверить и обновить коэффициенты при изменении настроек
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

        // Пересчитать коэффициенты
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

        // Проверяем настройки каждые ~1024 сэмпла
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

// ── Команды аудио-потока ────────────────────────────────────────────────────

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
    IsEmpty {
        resp: mpsc::SyncSender<bool>,
    },
}

// ── SymphoniaSource ─────────────────────────────────────────────────────────

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

        let track = format_reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .ok_or_else(|| "Не найден аудио трек".to_string())?;

        let track_id = track.id;
        let codec_params = &track.codec_params;
        let sample_rate = codec_params.sample_rate.unwrap_or(44100);
        let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);

        let decoder = symphonia::default::get_codecs()
            .make(codec_params, &DecoderOptions::default())
            .map_err(|e| format!("Не удалось создать декодер: {}", e))?;

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
        if self.current_pos >= self.current_samples.len() {
            if !self.decode_next_packet() {
                return None;
            }
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

// ── FadeOut ─────────────────────────────────────────────────────────────────

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

// ── AnalyzerSource ────────────────────────────────────────────────────────────

/// Прозрачная обёртка-«отвод»: пропускает сэмплы без изменений, попутно сводя
/// кадры в моно и подавая блоки в общий `SpectrumAnalyzer` для БПФ-визуализации.
/// Ставится поверх EQ (и fade-in), чтобы спектр отражал реально слышимый сигнал.
struct AnalyzerSource<S: Source<Item = f32>> {
    source: S,
    analyzer: Arc<SpectrumAnalyzer>,
    channels: u16,
    sample_rate: u32,
    ch_idx: u16,
    frame_sum: f32,
    block: Vec<f32>,
}

impl<S: Source<Item = f32>> AnalyzerSource<S> {
    /// Размер блока моно-сэмплов между подачами в анализатор: реже берём
    /// мьютекс (~раз в 256 кадров), не теряя плавности на 60 fps.
    const BLOCK: usize = 256;

    fn new(source: S, analyzer: Arc<SpectrumAnalyzer>) -> Self {
        let channels = source.channels().max(1);
        let sample_rate = source.sample_rate();
        Self {
            source,
            analyzer,
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
        let sample = self.source.next()?;

        // Визуализатор выключен — пропускаем звук как есть, без снятия спектра.
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

// ── Вспомогательные функции ─────────────────────────────────────────────────

/// Извлечь обложку из lofty тега (CoverFront или первая доступная)
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

/// Извлечь акцентный цвет из сырых байтов изображения.
/// Строит взвешенную по насыщенности гистограмму оттенков: побеждает оттенок,
/// который занимает наибольшую «видимую» цветовую массу обложки (площадь ×
/// насыщенность), а не редкие сочные пиксели. Жёлтые и кислотные тона
/// исключаются или приглушаются — они плохо работают как цвет UI.
pub fn extract_accent_color(image_data: &[u8]) -> Option<String> {
    let img = image::load_from_memory(image_data).ok()?;
    let thumb = img.thumbnail(96, 96);
    extract_via_hue_histogram(&thumb)
}

const HUE_BINS: usize = 36;
const HUE_BIN_WIDTH: f64 = 360.0 / HUE_BINS as f64;

/// Зона жёлтого, запрещённая для UI-акцента.
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

/// Сглаженное окно из трёх соседних бинов: масса и средневзвешенный цвет.
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
        // Почти серые и экстремально тёмные/светлые пиксели оттенка не несут
        if s < 0.12 || !(0.06..=0.94).contains(&l) {
            continue;
        }
        // Видимая цветовая масса пикселя: чем насыщеннее и ближе к средней
        // светлоте, тем заметнее его оттенок на обложке.
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

    // Почти монохромная обложка: нейтральный серый по её средней светлоте,
    // чтобы случайный цветной пиксель-шум не красил весь интерфейс.
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

    // Кандидаты — только реально заметные оттенки (≥6% цветовой массы):
    // мелкое сочное пятно не должно красить интерфейс.
    let mut pool: Vec<usize> = (0..HUE_BINS)
        .filter(|&i| windows[i].weight >= chroma_weight * 0.06)
        .collect();
    if pool.is_empty() {
        pool = (0..HUE_BINS).filter(|&i| windows[i].weight > 0.0).collect();
    }

    // Жёлтые оттенки выкидываем, если есть хоть одна заметная альтернатива.
    // Если вся обложка жёлтая — оставляем, normalize_accent_hsl уведёт оттенок.
    let non_yellow: Vec<usize> = pool
        .iter()
        .copied()
        .filter(|&i| !is_yellow_hue(windows[i].hue))
        .collect();
    if !non_yellow.is_empty() {
        pool = non_yellow;
    }

    // Доминантность решает в первую очередь, качество цвета — во вторую:
    // редкий, но сочный цвет не должен обгонять основной тон обложки.
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

/// Конвертировать RGB -> HSL
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

/// Конвертировать HSL -> RGB
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

/// Кусочно-линейный потолок насыщенности по оттенку: «токсичные» зоны
/// (лайм, неоновая зелень, фуксия) приглушаются сильнее спокойных синих.
fn saturation_ceiling(hue: f64) -> f64 {
    // Контрольные точки каждые 30°: 0° красный → 360° красный.
    const POINTS: [f64; 13] = [
        0.60, // 0°   красный
        0.58, // 30°  оранжевый
        0.45, // 60°  жёлтый (после сдвига сюда почти не попадаем)
        0.42, // 90°  лайм/олива
        0.45, // 120° зелёный
        0.48, // 150° изумруд
        0.52, // 180° циан
        0.58, // 210° голубой
        0.62, // 240° синий
        0.60, // 270° фиолетовый
        0.52, // 300° маджента
        0.56, // 330° розовый
        0.60, // 360° = 0°
    ];
    let h = hue.rem_euclid(360.0);
    let seg = ((h / 30.0) as usize).min(11);
    let t = (h - seg as f64 * 30.0) / 30.0;
    POINTS[seg] + (POINTS[seg + 1] - POINTS[seg]) * t
}

/// Подогнать цвет под фон UI: исключить жёлтый, зажать светлоту в узкий
/// диапазон и ограничить насыщенность с учётом «токсичности» оттенка,
/// сохранив характер цвета обложки.
fn normalize_accent_hsl(hue: f64, sat: f64, light: f64) -> (u8, u8, u8) {
    let mut hue = hue.rem_euclid(360.0);
    let mut sat = sat.clamp(0.0, 1.0);

    // Жёлтый как цвет UI не используем: тёплую половину зоны уводим
    // в янтарь, холодную (лаймовую) — в приглушённую оливу.
    if (YELLOW_FROM..62.0).contains(&hue) {
        hue = 36.0 + (hue - YELLOW_FROM) * (6.0 / (62.0 - YELLOW_FROM));
    } else if (62.0..YELLOW_TO).contains(&hue) {
        hue = 88.0 + (hue - 62.0) * (7.0 / (YELLOW_TO - 62.0));
    }

    // Единый узкий диапазон светлоты — чтобы акцент и производные фоны
    // (--dark-bg/--light-bg) выглядели одинаково плотно на любой обложке.
    let light = light.clamp(0.32, 0.48);

    // Мягкое сжатие насыщенности к потолку оттенка: убирает кислотность,
    // сохраняя различие между умеренными и сочными обложками.
    let ceiling = saturation_ceiling(hue);
    let knee = ceiling * 0.8;
    if sat > knee {
        sat = knee + (sat - knee) * 0.35;
    }
    sat = sat.min(ceiling);

    hsl_to_rgb(hue, sat, light)
}

/// Извлечь обложку и акцентный цвет из lofty тега
/// Из тега: есть ли встроенная обложка + акцентный цвет (вычисляется по её
/// байтам, нужен один раз и кэшируется в БД). Полную обложку в base64 больше
/// не формируем — для показа используется уменьшенная WebP-миниатюра через
/// протокол `lfmusic-cover` (см. модуль `thumbnails`).
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

/// Извлечь сэмплы из AudioBufferRef в вектор f32
fn extract_samples_into(buf: &AudioBufferRef, samples: &mut Vec<f32>) {
    macro_rules! extract {
        ($b:expr, $conv:expr) => {{
            let b = $b;
            samples.reserve(b.frames() * b.spec().channels.count());
            for frame in 0..b.frames() {
                for ch in 0..b.spec().channels.count() {
                    samples.push($conv(b.chan(ch)[frame]));
                }
            }
        }};
    }
    match buf {
        AudioBufferRef::F32(b) => extract!(b, |s: f32| s),
        AudioBufferRef::S32(b) => extract!(b, |s: i32| s as f32 / i32::MAX as f32),
        AudioBufferRef::S16(b) => extract!(b, |s: i16| s as f32 / i16::MAX as f32),
        AudioBufferRef::U8(b)  => extract!(b, |s: u8| (s as f32 - 128.0) / 128.0),
        _ => {}
    }
}

/// Кэш результата probe_actual_codec по (path, mtime).
/// M4A/MP4 — единственный формат, где нужен второй открытый файл для определения
/// кодека (AAC vs ALAC). play() вызывается на один и тот же файл многократно,
/// поэтому кэш убирает повторный парсинг контейнера.
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

/// Определить кодек через symphonia (нужно только для M4A/MP4 контейнеров)
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

// ── Аудио-поток ─────────────────────────────────────────────────────────────

/// Главный цикл выделенного аудио-потока.
/// Владеет OutputStream (и COM-контекстом на Windows), OutputStreamHandle и текущим Sink.
fn audio_thread_loop(
    rx: mpsc::Receiver<AudioCmd>,
    _stream: OutputStream, // должен жить пока работает поток
    stream_handle: OutputStreamHandle,
    eq_settings: Arc<RwLock<EqSettings>>,
    spectrum: Arc<SpectrumAnalyzer>,
) {
    let mut sink: Option<Sink> = None;
    // Предыдущий fade-out sink — при новом crossfade старый немедленно останавливается,
    // что ограничивает количество одновременных fade-потоков до одного.
    let mut fadeout_sink: Option<Sink> = None;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            AudioCmd::PlayFrom { file_path, seek_secs, volume, resp } => {
                if let Some(ref s) = sink { s.stop(); }
                let result = (|| -> Result<(), String> {
                    let new_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Не удалось создать sink: {}", e))?;
                    new_sink.set_volume(volume);
                    let source = SymphoniaSource::new_with_seek(&file_path, seek_secs)?;
                    new_sink.append(AnalyzerSource::new(
                        EqSource::new(source, eq_settings.clone()),
                        spectrum.clone(),
                    ));
                    sink = Some(new_sink);
                    Ok(())
                })();
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
                    let source = SymphoniaSource::new_with_seek(&file_path, position_secs)?;
                    new_sink.append(AnalyzerSource::new(
                        EqSource::new(source, eq_settings.clone()),
                        spectrum.clone(),
                    ));
                    if was_paused { new_sink.pause(); }
                    sink = Some(new_sink);
                    Ok(())
                })();
                let _ = resp.send(result);
            }

            AudioCmd::SeekCrossfade {
                file_path, position_secs, current_pos, volume,
                was_paused, crossfade_secs, resp,
            } => {
                let result = (|| -> Result<(), String> {
                    let dur = Duration::from_secs_f64(crossfade_secs);

                    // Останавливаем предыдущий fade-out, если есть
                    if let Some(old_fade) = fadeout_sink.take() { old_fade.stop(); }

                    // Fade-out на отдельном sink
                    let fade_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Ошибка fade-out sink: {}", e))?;
                    fade_sink.set_volume(volume);
                    let fade_source = SymphoniaSource::new_with_seek(&file_path, current_pos)?;
                    fade_sink.append(FadeOut::new(
                        EqSource::new(fade_source, eq_settings.clone()),
                        dur,
                    ));

                    if let Some(ref old) = sink { old.stop(); }
                    // Сохраняем fade-out sink вместо создания потока — rodio проигрывает его в фоне
                    fadeout_sink = Some(fade_sink);

                    // Новый sink с fade-in
                    let new_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Ошибка нового sink: {}", e))?;
                    new_sink.set_volume(volume);
                    let source = SymphoniaSource::new_with_seek(&file_path, position_secs)?;
                    new_sink.append(AnalyzerSource::new(
                        EqSource::new(source, eq_settings.clone()).fade_in(dur),
                        spectrum.clone(),
                    ));
                    if was_paused { new_sink.pause(); }
                    sink = Some(new_sink);
                    Ok(())
                })();
                let _ = resp.send(result);
            }

            AudioCmd::PlayCrossfade {
                file_path, seek_secs, volume, crossfade_secs, resp,
            } => {
                let result = (|| -> Result<(), String> {
                    let dur = Duration::from_secs_f64(crossfade_secs);

                    // Останавливаем предыдущий fade-out, если есть
                    if let Some(old_fade) = fadeout_sink.take() { old_fade.stop(); }

                    // Останавливаем старый sink — fade-in на новом треке
                    // обеспечивает плавный переход
                    if let Some(old_sink) = sink.take() { old_sink.stop(); }

                    // Новый sink с fade-in
                    let new_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Ошибка нового sink: {}", e))?;
                    new_sink.set_volume(volume);
                    let source = SymphoniaSource::new_with_seek(&file_path, seek_secs)?;
                    new_sink.append(AnalyzerSource::new(
                        EqSource::new(source, eq_settings.clone()).fade_in(dur),
                        spectrum.clone(),
                    ));
                    sink = Some(new_sink);
                    Ok(())
                })();
                let _ = resp.send(result);
            }

            AudioCmd::IsEmpty { resp } => {
                let empty = sink.as_ref().map(|s| s.empty()).unwrap_or(true);
                let _ = resp.send(empty);
            }
        }
    }
}

// ── AudioPlayer ─────────────────────────────────────────────────────────────

/// Основная структура аудиоплеера.
///
/// `OutputStream` (cpal, COM на Windows) живёт на выделенном потоке `audio-output`.
/// Все операции с Sink проксируются через `mpsc` канал — `unsafe impl Send/Sync` не нужен.
pub struct AudioPlayer {
    cmd_tx: mpsc::Sender<AudioCmd>,
    current_track: Option<TrackMetadata>,
    state: PlaybackState,
    volume: f32,
    playback_started_at: Option<Instant>,
    accumulated_position: f64,
    eq_settings: Arc<RwLock<EqSettings>>,
    spectrum: Arc<SpectrumAnalyzer>,
}

impl AudioPlayer {
    /// Создать новый экземпляр плеера (запускает аудио-поток)
    pub fn new() -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (init_tx, init_rx) = mpsc::sync_channel::<Result<(), String>>(0);
        let eq_settings = Arc::new(RwLock::new(EqSettings::default()));
        let eq_for_thread = eq_settings.clone();
        let spectrum = Arc::new(SpectrumAnalyzer::new());
        let spectrum_for_thread = spectrum.clone();

        std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || {
                match OutputStream::try_default() {
                    Ok((stream, handle)) => {
                        let _ = init_tx.send(Ok(()));
                        audio_thread_loop(cmd_rx, stream, handle, eq_for_thread, spectrum_for_thread);
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
            spectrum,
        })
    }

    /// Текущие частотные полосы спектра (0..1), посчитанные по реальному звуку.
    pub fn spectrum_bars(&self, bars: usize) -> Vec<f32> {
        self.spectrum.compute_bars(bars)
    }

    /// Включить/выключить снятие спектра (по состоянию блока-визуализатора).
    pub fn set_spectrum_enabled(&self, enabled: bool) {
        self.spectrum.set_enabled(enabled);
    }

    // ── Внутренние хелперы ──────────────────────────────────────────────────

    /// Отправить команду с ожиданием ответа
    fn send(&self, make: impl FnOnce(Resp) -> AudioCmd) -> Result<(), String> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.cmd_tx
            .send(make(tx))
            .map_err(|_| "Аудио поток недоступен".to_string())?;
        rx.recv()
            .map_err(|_| "Аудио поток недоступен".to_string())?
    }

    /// Отправить команду без ожидания ответа
    fn fire(&self, cmd: AudioCmd) {
        let _ = self.cmd_tx.send(cmd);
    }

    fn is_sink_empty(&self) -> bool {
        let (tx, rx) = mpsc::sync_channel(1);
        if self.cmd_tx.send(AudioCmd::IsEmpty { resp: tx }).is_err() {
            return true;
        }
        rx.recv().unwrap_or(true)
    }

    // ── Статические методы (метаданные) ─────────────────────────────────────

    /// Извлечь обложку трека как data URL (полного размера).
    /// Используется только для превью «сбросить обложку» в модалке редактирования
    /// (нужен оригинал из тегов в обход пользовательского переопределения).
    pub fn extract_cover_art(file_path: &str) -> Option<String> {
        let tagged_file = Probe::open(PathBuf::from(file_path)).ok()?.read().ok()?;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())?;
        cover_art_from_tag(tag)
    }

    /// Проверить наличие встроенной обложки (дёшево — без извлечения байтов).
    pub fn has_embedded_cover(file_path: &str) -> bool {
        let Ok(tagged_file) = Probe::open(PathBuf::from(file_path)).and_then(|p| p.read()) else {
            return false;
        };
        match tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) {
            Some(tag) => !tag.pictures().is_empty(),
            None => false,
        }
    }

    /// Сырые байты встроенной обложки (для генерации WebP-миниатюры).
    pub fn extract_cover_bytes(file_path: &str) -> Option<Vec<u8>> {
        let tagged_file = Probe::open(PathBuf::from(file_path)).ok()?.read().ok()?;
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

    /// Извлечь метаданные без обложки и акцентного цвета (для сканирования библиотеки)
    pub fn get_metadata_light(file_path: &str) -> Result<TrackMetadata, String> {
        Self::get_metadata_inner(file_path, false)
    }

    /// Извлечь метаданные из аудиофайла.
    ///
    /// Для однозначных форматов (FLAC, MP3, WAV, OGG, ...) открывает файл один раз (lofty).
    /// Symphonia-проба используется только для контейнеров M4A/MP4, где кодек неоднозначен
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

        // Для M4A/MP4 нужно пробировать кодек через symphonia
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

        // Теги и свойства через lofty (единственное открытие файла для не-MP4)
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

        if let Ok(probe) = Probe::open(&path) {
            if let Ok(tagged_file) = probe.read() {
                if let Some(tag) = tagged_file
                    .primary_tag()
                    .or_else(|| tagged_file.first_tag())
                {
                    title = tag.title().map(|s| s.to_string());
                    artist = tag.artist().map(|s| s.to_string());
                    album = tag.album().map(|s| s.to_string());
                    genre = tag.genre().map(|s| s.to_string());
                    if with_cover {
                        let (has_cover, accent) = cover_meta_from_tag(tag);
                        accent_color = accent;
                        if has_cover {
                            // Обложка отдаётся миниатюрой через протокол, без base64.
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
        })
    }

    /// Получить список поддерживаемых форматов (статический срез)
    pub fn supported_formats() -> &'static [&'static str] {
        &["flac", "wav", "mp3", "ogg", "aac", "m4a", "mp4", "aiff", "aif", "opus"]
    }

    // ── Управление воспроизведением ─────────────────────────────────────────

    /// Загрузить и воспроизвести файл
    pub fn play(&mut self, file_path: &str) -> Result<TrackMetadata, String> {
        self.play_from(file_path, 0.0)
    }

    /// Загрузить файл, перемотать на позицию и сразу поставить на паузу (для восстановления сессии)
    pub fn play_paused(&mut self, file_path: &str, position_secs: f64) -> Result<TrackMetadata, String> {
        let metadata = self.play_from(file_path, position_secs)?;
        self.pause();
        Ok(metadata)
    }

    pub fn play_crossfade(&mut self, file_path: &str, crossfade_secs: f64) -> Result<TrackMetadata, String> {
        if crossfade_secs <= 0.0 || self.state == PlaybackState::Stopped {
            return self.play(file_path);
        }

        let metadata = Self::get_metadata(file_path)?;

        self.send(|resp| AudioCmd::PlayCrossfade {
            file_path: file_path.to_string(),
            seek_secs: 0.0,
            volume: self.volume,
            crossfade_secs,
            resp,
        })?;

        self.current_track = Some(metadata.clone());
        self.state = PlaybackState::Playing;
        self.playback_started_at = Some(Instant::now());
        self.accumulated_position = 0.0;

        Ok(metadata)
    }

    fn play_from(&mut self, file_path: &str, seek_secs: f64) -> Result<TrackMetadata, String> {
        let metadata = Self::get_metadata(file_path)?;

        self.send(|resp| AudioCmd::PlayFrom {
            file_path: file_path.to_string(),
            seek_secs,
            volume: self.volume,
            resp,
        })?;

        self.current_track = Some(metadata.clone());
        self.state = PlaybackState::Playing;
        self.playback_started_at = Some(Instant::now());
        self.accumulated_position = seek_secs;

        Ok(metadata)
    }

    /// Поставить на паузу
    pub fn pause(&mut self) {
        if self.state == PlaybackState::Playing {
            self.fire(AudioCmd::Pause);
            if let Some(started) = self.playback_started_at.take() {
                self.accumulated_position += started.elapsed().as_secs_f64();
            }
            self.state = PlaybackState::Paused;
        }
    }

    /// Продолжить воспроизведение
    pub fn resume(&mut self) {
        if self.state == PlaybackState::Paused {
            self.fire(AudioCmd::Resume);
            self.playback_started_at = Some(Instant::now());
            self.state = PlaybackState::Playing;
        }
    }

    /// Остановить воспроизведение
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

    /// Установить громкость (0.0 – 1.0)
    pub fn set_volume(&mut self, volume: f32) {
        let vol = volume.clamp(0.0, 1.0);
        self.fire(AudioCmd::SetVolume(vol));
        self.volume = vol;
    }

    /// Получить текущую громкость
    pub fn get_volume(&self) -> f32 {
        self.volume
    }

    /// Перемотка на указанную позицию (в секундах)
    pub fn seek(&mut self, position_secs: f64) -> Result<(), String> {
        let file_path = self.current_track
            .as_ref()
            .map(|t| t.file_path.clone())
            .ok_or_else(|| "Нет активного трека".to_string())?;

        let was_paused = self.state == PlaybackState::Paused;

        self.send(|resp| AudioCmd::Seek {
            file_path,
            position_secs,
            volume: self.volume,
            was_paused,
            resp,
        })?;

        self.accumulated_position = position_secs;
        if self.state == PlaybackState::Playing {
            self.playback_started_at = Some(Instant::now());
        }

        Ok(())
    }

    /// Перемотка с кроссфейдом
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

        self.send(|resp| AudioCmd::SeekCrossfade {
            file_path,
            position_secs,
            current_pos,
            volume: self.volume,
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

    /// Получить текущую позицию воспроизведения в секундах
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

    /// Лёгкий статус для частого поллинга
    pub fn get_status(&mut self) -> PlayerStatus {
        // Проверяем, не закончился ли трек
        if self.state == PlaybackState::Playing && self.is_sink_empty() {
            self.sink_stopped();
        }

        PlayerStatus {
            state: self.state,
            volume: self.volume,
            position_secs: self.get_position(),
            current_file_path: self.current_track.as_ref().map(|t| t.file_path.clone()),
        }
    }

    // ── Управление эквалайзером ────────────────────────────────────────────

    /// Установить усиление полос эквалайзера (в dB, от -12 до +12)
    pub fn set_eq_bands(&self, bands: Vec<f32>) {
        if let Ok(mut settings) = self.eq_settings.write() {
            settings.bands = bands.into_iter().map(|b| b.clamp(-12.0, 12.0)).collect();
        }
    }

    /// Получить текущие значения полос эквалайзера
    pub fn get_eq_bands(&self) -> Vec<f32> {
        self.eq_settings.read().map(|s| s.bands.clone()).unwrap_or_else(|_| vec![0.0; 10])
    }

    /// Установить общий уровень эквалайзера (в dB, от -12 до +12)
    pub fn set_eq_gain(&self, gain: f32) {
        if let Ok(mut settings) = self.eq_settings.write() {
            settings.gain = gain.clamp(-12.0, 12.0);
        }
    }

    /// Получить общий уровень эквалайзера
    pub fn get_eq_gain(&self) -> f32 {
        self.eq_settings.read().map(|s| s.gain).unwrap_or(0.0)
    }

    /// Включить/выключить эквалайзер
    pub fn set_eq_enabled(&self, enabled: bool) {
        if let Ok(mut settings) = self.eq_settings.write() {
            settings.enabled = enabled;
        }
    }

    /// Получить состояние эквалайзера
    pub fn get_eq_enabled(&self) -> bool {
        self.eq_settings.read().map(|s| s.enabled).unwrap_or(false)
    }

    /// Получить полные настройки эквалайзера
    pub fn get_eq_settings(&self) -> EqSettings {
        self.eq_settings.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Установить полные настройки эквалайзера
    pub fn set_eq_settings(&self, settings: EqSettings) {
        if let Ok(mut current) = self.eq_settings.write() {
            *current = settings;
        }
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
        // 90% приглушённого синего фона + узкая неоново-красная полоса:
        // акцентом должен стать синий, а не заметный лишь вблизи красный.
        let blue = hsl_to_rgb(220.0, 0.35, 0.40);
        let red = hsl_to_rgb(0.0, 1.0, 0.50);
        let img = img_from_fn(|x, _| if x < 86 { blue } else { red });
        let (h, _, _) = accent_hsl(&img);
        assert!((190.0..260.0).contains(&h), "ожидался синий, получен hue={h}");
    }

    #[test]
    fn yellow_skipped_when_alternative_exists() {
        // Жёлтый занимает больше площади, но при наличии заметного синего
        // он не должен попадать в UI.
        let yellow = hsl_to_rgb(55.0, 0.80, 0.50);
        let blue = hsl_to_rgb(230.0, 0.60, 0.45);
        let img = img_from_fn(|x, _| if x < 58 { yellow } else { blue });
        let (h, _, _) = accent_hsl(&img);
        assert!((200.0..260.0).contains(&h), "ожидался синий, получен hue={h}");
    }

    #[test]
    fn fully_yellow_cover_shifts_out_of_yellow_zone() {
        // Полностью жёлтая обложка: оттенок уводится в янтарь,
        // жёлтым акцент быть не должен.
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
        // Кислотно-зелёная обложка: оттенок сохраняется, но насыщенность
        // зажимается потолком зоны, светлота — общим диапазоном.
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
        // Интерполяция между контрольными точками без разрывов на стыках.
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
}
