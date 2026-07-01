use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

const TARGET_SAMPLE_RATE: u32 = 8_000;
const ANALYSIS_WINDOW_SECS: f64 = 8.0;
const FFT_SIZE: usize = 512;
const HOP_SIZE: usize = 256;
const SPECTRAL_FRAME_STEP: usize = 6;
const FINGERPRINT_BANDS: usize = 16;

pub const AUDIO_FEATURE_ANALYZER_VERSION: i32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioFeatures {
    pub bpm: Option<f32>,
    pub loudness: Option<f32>,
    pub energy: Option<f32>,
    pub danceability: Option<f32>,
    pub spectral_centroid: Option<f32>,
    pub key: Option<String>,
    pub audio_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    pub(crate) fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    pub(crate) fn magnitude(self) -> f32 {
        (self.re * self.re + self.im * self.im).sqrt()
    }
}

#[derive(Debug, Clone)]
struct SpectralStats {
    centroid_hz: Option<f32>,
    chroma: [f32; 12],
    band_energy: [f32; FINGERPRINT_BANDS],
}

#[derive(Debug, Clone, Default)]
struct TempoStats {
    bpm: Option<f32>,
    confidence: f32,
}

pub fn analyze_file(file_path: &str, duration_secs: Option<f64>) -> Result<AudioFeatures, String> {
    let (samples, sample_rate) = collect_analysis_samples(file_path, duration_secs)?;
    Ok(analyze_samples(&samples, sample_rate))
}

#[cfg(test)]
fn analyze_samples_for_tests(samples: &[f32], sample_rate: u32) -> AudioFeatures {
    analyze_samples(samples, sample_rate)
}

fn analyze_samples(samples: &[f32], sample_rate: u32) -> AudioFeatures {
    if samples.len() < FFT_SIZE || sample_rate == 0 {
        return AudioFeatures {
            bpm: None,
            loudness: None,
            energy: None,
            danceability: None,
            spectral_centroid: None,
            key: None,
            audio_fingerprint: None,
        };
    }

    let rms = global_rms(samples);
    let peak = samples.iter().fold(0.0_f32, |acc, &s| acc.max(s.abs()));
    let loudness = 20.0 * rms.max(1.0e-7).log10();
    let loudness = loudness.clamp(-90.0, 0.0);
    let frame_rms = frame_rms(samples);
    let tempo = estimate_tempo(&frame_rms, sample_rate as f32 / HOP_SIZE as f32);
    let spectral = spectral_stats(samples, sample_rate);
    let energy = estimate_energy(rms, peak, loudness);
    let key = detect_key(&spectral.chroma).map(|(name, _)| name);
    let danceability = estimate_danceability(tempo.bpm, tempo.confidence, energy);
    let audio_fingerprint = Some(audio_fingerprint(
        &spectral.band_energy,
        &spectral.chroma,
        tempo.bpm,
        spectral.centroid_hz,
        energy,
    ));

    AudioFeatures {
        bpm: tempo.bpm.map(round2),
        loudness: Some(round2(loudness)),
        energy: Some(round4(energy)),
        danceability: Some(round4(danceability)),
        spectral_centroid: spectral.centroid_hz.map(round2),
        key,
        audio_fingerprint,
    }
}

fn collect_analysis_samples(
    file_path: &str,
    duration_secs: Option<f64>,
) -> Result<(Vec<f32>, u32), String> {
    let file =
        File::open(file_path).map_err(|e| format!("Не удалось открыть файл для анализа: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = PathBuf::from(file_path)
        .extension()
        .and_then(|e| e.to_str())
    {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Не удалось определить формат для анализа: {}", e))?;
    let mut format_reader = probed.format;

    let track = format_reader
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "Не найден аудио-трек для анализа".to_string())?;

    let track_id = track.id;
    let source_sample_rate = track.codec_params.sample_rate.unwrap_or(44_100).max(1);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Не удалось создать декодер анализа: {}", e))?;

    let start_secs = choose_window_start(duration_secs);
    if start_secs > 0.0 {
        let _ = format_reader.seek(
            SeekMode::Coarse,
            SeekTo::Time {
                time: Time::from(start_secs),
                track_id: Some(track_id),
            },
        );
        decoder.reset();
    }

    let stride = (source_sample_rate as f32 / TARGET_SAMPLE_RATE as f32)
        .round()
        .max(1.0) as usize;
    let effective_sample_rate = (source_sample_rate as usize / stride).max(1) as u32;
    let max_samples = (effective_sample_rate as f64 * ANALYSIS_WINDOW_SECS) as usize;
    let mut samples = Vec::with_capacity(max_samples.min(400_000));
    let mut decimation_pos = 0usize;

    while samples.len() < max_samples {
        let packet = match format_reader.next_packet() {
            Ok(packet) => packet,
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => push_downsampled_mono(
                &decoded,
                &mut samples,
                stride,
                &mut decimation_pos,
                max_samples,
            ),
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }

    if samples.len() < FFT_SIZE {
        return Err("Недостаточно аудиоданных для анализа".to_string());
    }

    Ok((samples, effective_sample_rate))
}

fn choose_window_start(duration_secs: Option<f64>) -> f64 {
    let _ = duration_secs;
    0.0
}

fn push_downsampled_mono(
    buf: &AudioBufferRef,
    samples: &mut Vec<f32>,
    stride: usize,
    decimation_pos: &mut usize,
    max_samples: usize,
) {
    macro_rules! extract {
        ($b:expr, $conv:expr) => {{
            let b = $b;
            let channels = b.spec().channels.count().max(1);
            for frame in 0..b.frames() {
                if *decimation_pos == 0 && samples.len() < max_samples {
                    let mut sum = 0.0_f32;
                    for ch in 0..channels {
                        sum += $conv(b.chan(ch)[frame]);
                    }
                    samples.push((sum / channels as f32).clamp(-1.0, 1.0));
                }
                *decimation_pos += 1;
                if *decimation_pos >= stride {
                    *decimation_pos = 0;
                }
                if samples.len() >= max_samples {
                    break;
                }
            }
        }};
    }

    match buf {
        AudioBufferRef::F32(b) => extract!(b, |s: f32| s),
        AudioBufferRef::S32(b) => extract!(b, |s: i32| s as f32 / i32::MAX as f32),
        AudioBufferRef::S16(b) => extract!(b, |s: i16| s as f32 / i16::MAX as f32),
        AudioBufferRef::U8(b) => extract!(b, |s: u8| (s as f32 - 128.0) / 128.0),
        _ => {}
    }
}

fn global_rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

fn frame_rms(samples: &[f32]) -> Vec<f32> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + FFT_SIZE <= samples.len() {
        out.push(global_rms(&samples[pos..pos + FFT_SIZE]));
        pos += HOP_SIZE;
    }
    out
}

fn estimate_energy(rms: f32, peak: f32, loudness: f32) -> f32 {
    let loudness_energy = ((loudness + 55.0) / 55.0).clamp(0.0, 1.0);
    (rms * 1.8 + peak * 0.25 + loudness_energy * 0.35).clamp(0.0, 1.0)
}

fn estimate_tempo(frame_rms: &[f32], frame_rate: f32) -> TempoStats {
    if frame_rms.len() < 16 || frame_rate <= 0.0 {
        return TempoStats::default();
    }

    let mut envelope = Vec::with_capacity(frame_rms.len().saturating_sub(1));
    let mut prev = (frame_rms[0] + 1.0e-6).ln();
    for &rms in frame_rms.iter().skip(1) {
        let value = (rms + 1.0e-6).ln();
        envelope.push((value - prev).max(0.0));
        prev = value;
    }

    let mean = envelope.iter().sum::<f32>() / envelope.len() as f32;
    for value in &mut envelope {
        *value = (*value - mean).max(0.0);
    }

    let energy = envelope.iter().map(|v| v * v).sum::<f32>();
    if energy <= 1.0e-7 {
        return TempoStats::default();
    }

    let mut best_bpm = None;
    let mut best_score = 0.0_f32;
    for bpm in 60..=200 {
        let lag = frame_rate * 60.0 / bpm as f32;
        if lag < 1.0 || lag + 2.0 >= envelope.len() as f32 {
            continue;
        }

        let mut score = 0.0_f32;
        let mut norm = 0.0_f32;
        let max_i = envelope.len().saturating_sub(lag.ceil() as usize + 1);
        for i in 0..max_i {
            let b = interp(&envelope, i as f32 + lag);
            score += envelope[i] * b;
            norm += envelope[i] * envelope[i];
        }

        let score = if norm > 0.0 { score / norm } else { 0.0 };
        if score > best_score {
            best_score = score;
            best_bpm = Some(bpm as f32);
        }
    }

    TempoStats {
        bpm: best_bpm.map(normalize_bpm),
        confidence: best_score.clamp(0.0, 1.0),
    }
}

fn normalize_bpm(mut bpm: f32) -> f32 {
    while bpm < 90.0 {
        bpm *= 2.0;
    }
    while bpm > 180.0 {
        bpm *= 0.5;
    }
    bpm
}

fn interp(values: &[f32], index: f32) -> f32 {
    let lo = index.floor() as usize;
    let hi = (lo + 1).min(values.len().saturating_sub(1));
    let frac = index - lo as f32;
    values.get(lo).copied().unwrap_or(0.0) * (1.0 - frac)
        + values.get(hi).copied().unwrap_or(0.0) * frac
}

fn spectral_stats(samples: &[f32], sample_rate: u32) -> SpectralStats {
    let window = hann_window(FFT_SIZE);
    let mut fft_buffer = vec![Complex::default(); FFT_SIZE];
    let mut centroid_num = 0.0_f64;
    let mut centroid_den = 0.0_f64;
    let mut chroma = [0.0_f32; 12];
    let mut band_energy = [0.0_f32; FINGERPRINT_BANDS];

    let mut frame_index = 0usize;
    let mut pos = 0usize;
    while pos + FFT_SIZE <= samples.len() {
        if frame_index % SPECTRAL_FRAME_STEP == 0 {
            for i in 0..FFT_SIZE {
                fft_buffer[i] = Complex::new(samples[pos + i] * window[i], 0.0);
            }
            fft_radix2(&mut fft_buffer);

            let bin_hz = sample_rate as f32 / FFT_SIZE as f32;
            for (bin, value) in fft_buffer.iter().take(FFT_SIZE / 2).enumerate().skip(1) {
                let freq = bin as f32 * bin_hz;
                let mag = value.magnitude();
                if !mag.is_finite() || mag <= 1.0e-8 {
                    continue;
                }

                centroid_num += freq as f64 * mag as f64;
                centroid_den += mag as f64;

                if (27.5..=5000.0).contains(&freq) {
                    let midi = (69.0 + 12.0 * (freq / 440.0).log2()).round() as i32;
                    let pc = midi.rem_euclid(12) as usize;
                    chroma[pc] += mag;
                }

                if (50.0..=5500.0).contains(&freq) {
                    let ratio = (freq / 50.0).ln() / (5500.0_f32 / 50.0).ln();
                    let band = ((ratio * FINGERPRINT_BANDS as f32).floor() as usize)
                        .min(FINGERPRINT_BANDS - 1);
                    band_energy[band] += mag;
                }
            }
        }

        frame_index += 1;
        pos += HOP_SIZE;
    }

    let centroid_hz = if centroid_den > 0.0 {
        Some((centroid_num / centroid_den) as f32)
    } else {
        None
    };

    SpectralStats {
        centroid_hz,
        chroma,
        band_energy,
    }
}

thread_local! {
    /// Twiddle factors cached per FFT size, avoids recomputing cos/sin every call.
    static TWIDDLES: RefCell<HashMap<usize, Vec<Complex>>> = RefCell::new(HashMap::new());
    /// Hann window cache per size.
    static HANN: RefCell<HashMap<usize, Vec<f32>>> = RefCell::new(HashMap::new());
}

fn build_twiddles(n: usize) -> Vec<Complex> {
    (0..n / 2)
        .map(|k| {
            let angle = -2.0 * std::f32::consts::PI * k as f32 / n as f32;
            Complex::new(angle.cos(), angle.sin())
        })
        .collect()
}

pub(crate) fn hann_window(size: usize) -> Vec<f32> {
    HANN.with(|cache| {
        cache
            .borrow_mut()
            .entry(size)
            .or_insert_with(|| {
                (0..size)
                    .map(|i| {
                        let phase = 2.0 * std::f32::consts::PI * i as f32 / (size - 1) as f32;
                        0.5 - 0.5 * phase.cos()
                    })
                    .collect()
            })
            .clone()
    })
}

pub(crate) fn fft_radix2(buffer: &mut [Complex]) {
    let n = buffer.len();
    debug_assert!(n.is_power_of_two());
    if n < 2 {
        return;
    }

    // Bit-reversal permutation (in-place).
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            buffer.swap(i, j);
        }
    }

    // Butterfly stage using cached twiddles: for stage length `len`, the
    // factor exp(-2pi*i*k/len) = table[k*(n/len)].
    TWIDDLES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let table = cache.entry(n).or_insert_with(|| build_twiddles(n));
        let mut len = 2usize;
        while len <= n {
            let half = len / 2;
            let stride = n / len;
            for i in (0..n).step_by(len) {
                for k in 0..half {
                    let w = table[k * stride];
                    let u = buffer[i + k];
                    let v = multiply(buffer[i + k + half], w);
                    buffer[i + k] = Complex::new(u.re + v.re, u.im + v.im);
                    buffer[i + k + half] = Complex::new(u.re - v.re, u.im - v.im);
                }
            }
            len <<= 1;
        }
    });
}

fn multiply(a: Complex, b: Complex) -> Complex {
    Complex::new(a.re * b.re - a.im * b.im, a.re * b.im + a.im * b.re)
}

fn detect_key(chroma: &[f32; 12]) -> Option<(String, f32)> {
    let total = chroma.iter().sum::<f32>();
    if total <= 1.0e-6 {
        return None;
    }

    const NOTE_NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    const MAJOR: [f32; 12] = [
        6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
    ];
    const MINOR: [f32; 12] = [
        6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
    ];

    let chroma_norm = normalize_12(chroma);
    let major_norm = normalize_12(&MAJOR);
    let minor_norm = normalize_12(&MINOR);
    let mut best_score = f32::MIN;
    let mut best_root = 0usize;
    let mut best_is_major = true;

    for root in 0..12 {
        let major_score = profile_score(&chroma_norm, &major_norm, root);
        if major_score > best_score {
            best_score = major_score;
            best_root = root;
            best_is_major = true;
        }
        let minor_score = profile_score(&chroma_norm, &minor_norm, root);
        if minor_score > best_score {
            best_score = minor_score;
            best_root = root;
            best_is_major = false;
        }
    }

    let mode = if best_is_major { "major" } else { "minor" };
    let strength = ((best_score + 1.0) / 2.0).clamp(0.0, 1.0);
    Some((format!("{} {}", NOTE_NAMES[best_root], mode), strength))
}

fn normalize_12(values: &[f32; 12]) -> [f32; 12] {
    let mean = values.iter().sum::<f32>() / 12.0;
    let mut out = [0.0_f32; 12];
    let mut norm = 0.0_f32;
    for i in 0..12 {
        out[i] = values[i] - mean;
        norm += out[i] * out[i];
    }
    let norm = norm.sqrt().max(1.0e-6);
    for v in &mut out {
        *v /= norm;
    }
    out
}

fn profile_score(chroma: &[f32; 12], profile: &[f32; 12], root: usize) -> f32 {
    let mut score = 0.0_f32;
    for i in 0..12 {
        score += chroma[(i + root) % 12] * profile[i];
    }
    score
}

fn estimate_danceability(bpm: Option<f32>, beat_confidence: f32, energy: f32) -> f32 {
    let tempo_fit = bpm
        .map(|v| (1.0 - ((v - 120.0).abs() / 90.0)).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    (beat_confidence * 0.68 + tempo_fit * 0.22 + energy * 0.10).clamp(0.0, 1.0)
}

fn audio_fingerprint(
    bands: &[f32; FINGERPRINT_BANDS],
    chroma: &[f32; 12],
    bpm: Option<f32>,
    centroid: Option<f32>,
    energy: f32,
) -> String {
    let mut h1 = 0xcbf29ce484222325_u64;
    let mut h2 = 0x84222325cbf29ce4_u64;

    for (idx, &value) in bands.iter().enumerate() {
        let q = (value.max(1.0e-9).log10() * 128.0).round() as i64;
        mix_hash(&mut h1, idx as u64, q as u64);
        mix_hash(&mut h2, (idx as u64) ^ 0x55, q.unsigned_abs());
    }
    for (idx, &value) in chroma.iter().enumerate() {
        let q = (value.max(1.0e-9).log10() * 96.0).round() as i64;
        mix_hash(&mut h1, (idx + 31) as u64, q as u64);
        mix_hash(&mut h2, (idx + 79) as u64, q.unsigned_abs());
    }
    mix_hash(&mut h1, 101, bpm.unwrap_or(0.0).round() as u64);
    mix_hash(&mut h1, 102, centroid.unwrap_or(0.0).round() as u64);
    mix_hash(&mut h2, 103, (energy * 1000.0).round() as u64);

    format!("{:016x}{:016x}", h1, h2)
}

fn mix_hash(hash: &mut u64, index: u64, value: u64) {
    *hash ^= index.wrapping_mul(0x9e3779b185ebca87);
    *hash = hash.wrapping_mul(0x100000001b3);
    *hash ^= value.wrapping_mul(0xbf58476d1ce4e5b9);
    *hash = hash.rotate_left(13);
}

fn round2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

fn round4(value: f32) -> f32 {
    (value * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sample_rate: u32, seconds: f32) -> Vec<f32> {
        let len = (sample_rate as f32 * seconds) as usize;
        (0..len)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.5
            })
            .collect()
    }

    #[test]
    fn estimates_centroid_near_tone_frequency() {
        let samples = sine(440.0, 11_025, 4.0);
        let features = analyze_samples_for_tests(&samples, 11_025);
        let centroid = features.spectral_centroid.expect("centroid");

        assert!((centroid - 440.0).abs() < 90.0, "centroid={centroid}");
    }

    #[test]
    fn estimates_bpm_from_regular_pulses() {
        let sample_rate = 11_025;
        let mut samples = vec![0.0; sample_rate as usize * 12];
        for beat in 0..24 {
            let start = (beat as f32 * 0.5 * sample_rate as f32) as usize;
            for i in 0..350 {
                let idx = start + i;
                if idx < samples.len() {
                    samples[idx] += (1.0 - i as f32 / 350.0) * 0.8;
                }
            }
        }

        let features = analyze_samples_for_tests(&samples, sample_rate);
        let bpm = features.bpm.expect("bpm");

        assert!((bpm - 120.0).abs() < 4.0, "bpm={bpm}");
        assert!(features.danceability.unwrap_or(0.0) > 0.55);
    }

    #[test]
    fn detects_major_chord_key_root() {
        let sample_rate = 11_025;
        let len = sample_rate as usize * 4;
        let mut samples = vec![0.0; len];
        for freq in [440.0, 554.37, 659.25] {
            for (i, sample) in samples.iter_mut().enumerate() {
                let t = i as f32 / sample_rate as f32;
                *sample += (2.0 * std::f32::consts::PI * freq * t).sin() * 0.22;
            }
        }

        let features = analyze_samples_for_tests(&samples, sample_rate);
        let key = features.key.expect("key");

        assert!(key.starts_with('A'), "key={key}");
    }

    #[test]
    fn fingerprint_is_deterministic_and_content_sensitive() {
        let a = sine(330.0, 11_025, 4.0);
        let b = sine(880.0, 11_025, 4.0);

        let fa = analyze_samples_for_tests(&a, 11_025)
            .audio_fingerprint
            .expect("fingerprint a");
        let fa2 = analyze_samples_for_tests(&a, 11_025)
            .audio_fingerprint
            .expect("fingerprint a2");
        let fb = analyze_samples_for_tests(&b, 11_025)
            .audio_fingerprint
            .expect("fingerprint b");

        assert_eq!(fa, fa2);
        assert_ne!(fa, fb);
        assert!(fa.len() >= 16);
    }

    #[test]
    fn file_analysis_budget_stays_small_for_scanning() {
        assert!(ANALYSIS_WINDOW_SECS >= 8.0);
        assert!(ANALYSIS_WINDOW_SECS <= 12.0);
        assert!(TARGET_SAMPLE_RATE <= 8_000);
        assert!(FFT_SIZE <= 512);
    }

    #[test]
    fn analyze_samples_below_fft_size_returns_all_none() {
        let samples = vec![0.3f32; FFT_SIZE - 1];
        let features = analyze_samples_for_tests(&samples, 11_025);
        assert_eq!(features.bpm, None);
        assert_eq!(features.loudness, None);
        assert_eq!(features.energy, None);
        assert_eq!(features.danceability, None);
        assert_eq!(features.spectral_centroid, None);
        assert_eq!(features.key, None);
        assert_eq!(features.audio_fingerprint, None);
    }

    #[test]
    fn analyze_samples_with_zero_sample_rate_returns_all_none() {
        let samples = vec![0.3f32; FFT_SIZE * 4];
        let features = analyze_samples_for_tests(&samples, 0);
        assert_eq!(features.loudness, None);
        assert_eq!(features.audio_fingerprint, None);
    }

    #[test]
    fn silence_yields_minimum_loudness_and_zero_energy() {
        let samples = vec![0.0f32; FFT_SIZE * 4];
        let features = analyze_samples_for_tests(&samples, 11_025);
        // global_rms(0) -> log10(1e-7) clamped to range minimum -90.0.
        assert_eq!(features.loudness, Some(-90.0));
        assert_eq!(features.energy, Some(0.0));
        assert_eq!(features.key, None, "у тишины не должно быть определяемой тональности");
    }

    #[test]
    fn normalize_bpm_doubles_slow_tempo_into_target_range() {
        assert_eq!(normalize_bpm(45.0), 90.0);
        assert_eq!(normalize_bpm(50.0), 100.0);
    }

    #[test]
    fn normalize_bpm_halves_fast_tempo_into_target_range() {
        assert_eq!(normalize_bpm(200.0), 100.0);
        assert_eq!(normalize_bpm(300.0), 150.0);
    }

    #[test]
    fn normalize_bpm_leaves_in_range_tempo_untouched() {
        assert_eq!(normalize_bpm(120.0), 120.0);
        assert_eq!(normalize_bpm(90.0), 90.0);
        assert_eq!(normalize_bpm(180.0), 180.0);
    }

    #[test]
    fn estimate_danceability_is_maximal_for_ideal_dance_tempo() {
        let d = estimate_danceability(Some(120.0), 1.0, 1.0);
        assert!((d - 1.0).abs() < 1e-6, "d={d}");
    }

    #[test]
    fn estimate_danceability_without_bpm_ignores_tempo_term() {
        let with_bpm = estimate_danceability(Some(120.0), 0.5, 0.5);
        let without_bpm = estimate_danceability(None, 0.5, 0.5);
        assert!(without_bpm < with_bpm);
        // Without bpm, tempo_fit contributes nothing: 0.5*0.68 + 0.5*0.10.
        assert!((without_bpm - (0.5 * 0.68 + 0.5 * 0.10)).abs() < 1e-6);
    }

    #[test]
    fn estimate_danceability_clamps_to_unit_range() {
        assert!(estimate_danceability(Some(120.0), 2.0, 2.0) <= 1.0);
        assert!(estimate_danceability(Some(-1000.0), 0.0, 0.0) >= 0.0);
    }

    #[test]
    fn round2_and_round4_truncate_to_expected_precision() {
        assert_eq!(round2(1.23456), 1.23);
        assert_eq!(round2(-1.236), -1.24);
        assert_eq!(round4(1.234567), 1.2346);
    }

    #[test]
    fn detect_key_returns_none_for_silent_chroma() {
        assert_eq!(detect_key(&[0.0; 12]), None);
    }

    #[test]
    fn fft_radix2_matches_naive_dft_for_random_signal() {
        // Cross-check against a naive DFT: catches permutation/butterfly bugs
        // that the sine-wave tests above could miss (right magnitude, wrong phase/bin).
        let n = 64usize;
        let mut input: Vec<Complex> = (0..n)
            .map(|i| Complex::new(((i * 37 % 13) as f32) - 6.0, ((i * 19 % 11) as f32) - 5.0))
            .collect();

        let mut expected = vec![Complex::new(0.0, 0.0); n];
        for (k, slot) in expected.iter_mut().enumerate() {
            let mut re = 0.0f32;
            let mut im = 0.0f32;
            for (t, sample) in input.iter().enumerate() {
                let angle = -2.0 * std::f32::consts::PI * (k as f32) * (t as f32) / n as f32;
                let (s, c) = angle.sin_cos();
                re += sample.re * c - sample.im * s;
                im += sample.re * s + sample.im * c;
            }
            *slot = Complex::new(re, im);
        }

        fft_radix2(&mut input);

        for (got, exp) in input.iter().zip(expected.iter()) {
            assert!((got.re - exp.re).abs() < 0.01, "re mismatch: {} vs {}", got.re, exp.re);
            assert!((got.im - exp.im).abs() < 0.01, "im mismatch: {} vs {}", got.im, exp.im);
        }
    }
}
