//! Real-time spectrum analyzer for playing audio.
//!
//! Producer: `AnalyzerSource` in the audio thread (see `audio_player`) mixes
//! the post-EQ output to mono into a ring buffer. Consumer: the
//! `audio_get_spectrum` IPC command, polled by the frontend every rAF frame
//! (30/60 fps), which FFTs the latest window into `bars` log-spaced bands.

use crate::audio_features::hann_window;
use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// FFT window size (power of two). At 44.1kHz this is ~46ms — enough bass
/// resolution while staying responsive at 60fps.
const FFT_SIZE: usize = 2048;

/// Visible spectrum range: below 30Hz is inaudible on bars, above 16kHz is cut.
const FREQ_MIN: f32 = 30.0;
const FREQ_MAX: f32 = 16_000.0;

/// dB range mapped linearly to [0, 1] for bar height.
const DB_FLOOR: f32 = -72.0;
const DB_CEIL: f32 = -14.0;

pub struct SpectrumAnalyzer {
    /// Single global flag (not per-visualizer): kept outside the mutex so the
    /// audio thread can check it with a lock-free relaxed load per sample.
    /// Multiple visualizer instances share this one flag.
    enabled: AtomicBool,
    inner: Mutex<Inner>,
}

struct Inner {
    ring: Box<[f32; FFT_SIZE]>,
    /// Next write index; also the position of the oldest sample.
    write: usize,
    /// Samples written so far, capped at FFT_SIZE.
    filled: usize,
    sample_rate: u32,
    /// Whether audio is currently flowing (returns zeros when idle/stopped).
    active: bool,
}

impl SpectrumAnalyzer {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            inner: Mutex::new(Inner {
                ring: Box::new([0.0; FFT_SIZE]),
                write: 0,
                filled: 0,
                sample_rate: 44_100,
                active: false,
            }),
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if !enabled {
            // Clear buffer so re-enabling doesn't flash a stale tail.
            if let Ok(mut g) = self.inner.lock() {
                g.active = false;
                g.filled = 0;
                g.write = 0;
            }
        }
    }

    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Producer: append a mono sample block from the audio thread.
    pub fn push_mono(&self, block: &[f32], sample_rate: u32) {
        if let Ok(mut g) = self.inner.lock() {
            if sample_rate > 0 {
                g.sample_rate = sample_rate;
            }
            g.active = true;
            for &s in block {
                let w = g.write;
                g.ring[w] = s;
                g.write = (w + 1) % FFT_SIZE;
                if g.filled < FFT_SIZE {
                    g.filled += 1;
                }
            }
        }
    }

    pub fn set_active(&self, active: bool) {
        if let Ok(mut g) = self.inner.lock() {
            g.active = active;
        }
    }

    /// Consumer: compute `bars` normalized (0..1) frequency bands from the
    /// latest buffer window; zeros when idle or buffer isn't full yet.
    pub fn compute_bars(&self, bars: usize) -> Vec<f32> {
        let bars = bars.clamp(1, 128);
        if !self.is_enabled() {
            return vec![0.0; bars];
        }

        let (samples, sample_rate) = {
            let g = match self.inner.lock() {
                Ok(g) => g,
                Err(_) => return vec![0.0; bars],
            };
            if !g.active || g.filled < FFT_SIZE {
                return vec![0.0; bars];
            }
            // Copy the ring in chronological (oldest-to-newest) order.
            let mut buf = vec![0.0f32; FFT_SIZE];
            let start = g.write;
            for (i, slot) in buf.iter_mut().enumerate() {
                *slot = g.ring[(start + i) % FFT_SIZE];
            }
            (buf, g.sample_rate)
        };

        // FFT itself runs without holding the lock.
        compute_bars_from_samples(&samples, sample_rate, bars)
    }
}

/// rustfft caches the plan internally, so repeated per-frame calls are cheap.
fn fft_plan(n: usize) -> Arc<dyn Fft<f32>> {
    static PLANNER: OnceLock<Mutex<FftPlanner<f32>>> = OnceLock::new();
    PLANNER
        .get_or_init(|| Mutex::new(FftPlanner::new()))
        .lock()
        .map(|mut p| p.plan_fft_forward(n))
        .expect("FFT planner poisoned")
}

thread_local! {
    static HANN_CACHE: RefCell<(usize, Vec<f32>)> = const { RefCell::new((0, Vec::new())) };
    /// Reused rustfft scratch buffer, avoids per-frame allocation.
    static FFT_SCRATCH: RefCell<Vec<Complex32>> = const { RefCell::new(Vec::new()) };
}

fn compute_bars_from_samples(samples: &[f32], sample_rate: u32, bars: usize) -> Vec<f32> {
    let n = samples.len();
    if n == 0 || sample_rate == 0 {
        return vec![0.0; bars];
    }

    let mut buffer: Vec<Complex32> = HANN_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != n {
            *cache = (n, hann_window(n));
        }
        let window = &cache.1;
        (0..n)
            .map(|i| Complex32::new(samples[i] * window[i], 0.0))
            .collect()
    });

    let fft = fft_plan(n);
    FFT_SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        let need = fft.get_inplace_scratch_len();
        if scratch.len() < need {
            scratch.resize(need, Complex32::default());
        }
        fft.process_with_scratch(&mut buffer, &mut scratch[..need]);
    });

    let half = n / 2;
    let bin_hz = sample_rate as f32 / n as f32;
    let f_max = FREQ_MAX.min(sample_rate as f32 / 2.0);
    let ratio = (f_max / FREQ_MIN).max(1.0);

    let mut out = vec![0.0f32; bars];
    for (b, slot) in out.iter_mut().enumerate() {
        // Log-spaced band boundaries.
        let lo = FREQ_MIN * ratio.powf(b as f32 / bars as f32);
        let hi = FREQ_MIN * ratio.powf((b + 1) as f32 / bars as f32);
        let bin_lo = ((lo / bin_hz).floor() as usize).max(1);
        let bin_hi = ((hi / bin_hz).ceil() as usize).clamp(bin_lo + 1, half);

        // Normalize by window size so scale doesn't depend on FFT_SIZE.
        let sum = buffer[bin_lo..bin_hi].iter().map(|c| c.norm()).sum::<f32>() / n as f32;
        let avg = sum / (bin_hi - bin_lo) as f32;

        let db = 20.0 * (avg + 1e-7).log10();
        let norm = ((db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0);

        // Slight high-frequency boost: without it, high bands look flat/dead.
        let tilt = 1.0 + 0.5 * (b as f32 / bars.max(1) as f32);
        *slot = (norm * tilt).clamp(0.0, 1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sample_rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * freq * t).sin() * 0.8
            })
            .collect()
    }

    #[test]
    fn compute_bars_clamps_count_into_valid_range() {
        let analyzer = SpectrumAnalyzer::new();
        assert_eq!(analyzer.compute_bars(0).len(), 1);
        assert_eq!(analyzer.compute_bars(500).len(), 128);
        assert_eq!(analyzer.compute_bars(16).len(), 16);
    }

    #[test]
    fn compute_bars_is_zero_when_disabled() {
        let analyzer = SpectrumAnalyzer::new();
        assert!(!analyzer.is_enabled());
        let bars = analyzer.compute_bars(8);
        assert!(bars.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn compute_bars_is_zero_before_buffer_fills_even_when_enabled() {
        let analyzer = SpectrumAnalyzer::new();
        analyzer.set_enabled(true);
        // Fewer samples than FFT_SIZE -> buffer not yet full.
        analyzer.push_mono(&vec![0.5f32; 100], 44_100);
        let bars = analyzer.compute_bars(8);
        assert!(bars.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn compute_bars_is_zero_for_silence_once_filled() {
        let analyzer = SpectrumAnalyzer::new();
        analyzer.set_enabled(true);
        analyzer.push_mono(&vec![0.0f32; FFT_SIZE], 44_100);
        let bars = analyzer.compute_bars(8);
        assert!(bars.iter().all(|&v| v == 0.0), "тишина не должна давать полос: {bars:?}");
    }

    #[test]
    fn set_enabled_false_resets_buffer_so_reenabling_starts_silent() {
        let analyzer = SpectrumAnalyzer::new();
        analyzer.set_enabled(true);
        analyzer.push_mono(&sine(1000.0, 44_100, FFT_SIZE * 2), 44_100);
        let active_bars = analyzer.compute_bars(8);
        assert!(active_bars.iter().any(|&v| v > 0.0), "ожидался ненулевой спектр от синусоиды");

        analyzer.set_enabled(false);
        analyzer.set_enabled(true);
        let after_reset = analyzer.compute_bars(8);
        assert!(
            after_reset.iter().all(|&v| v == 0.0),
            "после выключения/включения буфер должен быть пуст: {after_reset:?}"
        );
    }

    #[test]
    fn set_active_false_silences_output_without_clearing_enabled_flag() {
        let analyzer = SpectrumAnalyzer::new();
        analyzer.set_enabled(true);
        analyzer.push_mono(&sine(1000.0, 44_100, FFT_SIZE * 2), 44_100);
        assert!(analyzer.compute_bars(8).iter().any(|&v| v > 0.0));

        analyzer.set_active(false);
        let bars = analyzer.compute_bars(8);
        assert!(bars.iter().all(|&v| v == 0.0));
        assert!(analyzer.is_enabled(), "set_active не должен трогать enabled");
    }

    #[test]
    fn compute_bars_from_samples_handles_degenerate_input() {
        assert_eq!(compute_bars_from_samples(&[], 44_100, 8), vec![0.0; 8]);
        assert_eq!(compute_bars_from_samples(&[0.1, 0.2], 0, 8), vec![0.0; 8]);
    }

    #[test]
    fn compute_bars_from_samples_peaks_near_tone_frequency() {
        // 1kHz sine: expect energy in the mid band, near-nothing in the lowest.
        let sample_rate = 44_100u32;
        let samples = sine(1000.0, sample_rate, FFT_SIZE);
        let bars = compute_bars_from_samples(&samples, sample_rate, 16);

        let max_idx = bars
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();

        // Bands are log-spaced from FREQ_MIN(30) to FREQ_MAX(16000); 1kHz
        // should land mid-scale, not in the lowest (bass) bin.
        assert!(max_idx > 0, "пик не должен быть в самом нижнем (басовом) бине");
    }
}
