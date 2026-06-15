//! Реальный анализатор спектра воспроизводимого аудио.
//!
//! Продюсер — это `AnalyzerSource` в аудио-потоке (см. `audio_player`), который
//! сводит выходной сигнал (после EQ, т.е. то, что реально слышно) в моно и
//! складывает блоки сэмплов в кольцевой буфер. Консьюмер — IPC-команда
//! `audio_get_spectrum`, которую фронтенд опрашивает на каждом кадре rAF
//! (30/60 fps). Для каждого запроса по последнему окну буфера считается БПФ
//! и сворачивается в `bars` логарифмически разнесённых частотных полос.

use crate::audio_features::hann_window;
use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Размер окна БПФ (степень двойки). При 44.1 кГц это ~46 мс сигнала —
/// достаточно для разрешения низов и при этом отзывчиво на кадрах 60 fps.
const FFT_SIZE: usize = 2048;

/// Нижняя/верхняя границы видимого спектра. Низ обрезаем по 30 Гц (ниже —
/// субинфразвук, на полосах не виден), верх — по 16 кГц.
const FREQ_MIN: f32 = 30.0;
const FREQ_MAX: f32 = 16_000.0;

/// Диапазон отображаемой громкости в дБ. Магнитуды переводятся в дБ и
/// линейно нормируются из [FLOOR, CEIL] в [0, 1].
const DB_FLOOR: f32 = -72.0;
const DB_CEIL: f32 = -14.0;

pub struct SpectrumAnalyzer {
    /// Нужен ли визуализатор сейчас (включён ли блок на фронтенде). Пока
    /// выключен — продюсер (`AnalyzerSource`) не делает вообще ничего, кроме
    /// одной relaxed-загрузки этого флага на сэмпл. Лежит отдельно от мьютекса,
    /// чтобы проверка в аудио-потоке была без блокировки.
    enabled: AtomicBool,
    inner: Mutex<Inner>,
}

struct Inner {
    /// Кольцевой буфер моно-сэмплов.
    ring: Box<[f32; FFT_SIZE]>,
    /// Индекс следующей записи (он же — позиция самого старого сэмпла).
    write: usize,
    /// Сколько сэмплов уже записано (до FFT_SIZE).
    filled: usize,
    sample_rate: u32,
    /// Идёт ли сейчас подача звука (для возврата нулей в покое/после стопа).
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

    /// Включить/выключить визуализатор. Управляется фронтендом по настройке
    /// блока плеера: пока выключено — спектр не снимается и не считается.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if !enabled {
            // Чтобы при повторном включении не мигнуть «хвостом» прошлого звука.
            if let Ok(mut g) = self.inner.lock() {
                g.active = false;
                g.filled = 0;
                g.write = 0;
            }
        }
    }

    /// Снимается ли сейчас спектр (проверяется продюсером без блокировки).
    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Продюсер: добавить блок моно-сэмплов из аудио-потока.
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

    /// Пометить поток как активный/неактивный (стоп воспроизведения).
    pub fn set_active(&self, active: bool) {
        if let Ok(mut g) = self.inner.lock() {
            g.active = active;
        }
    }

    /// Консьюмер: посчитать `bars` нормированных (0..1) частотных полос по
    /// последнему окну буфера. В покое и при недоборе сэмплов возвращает нули.
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
            // Копируем кольцо в хронологическом порядке (от старого к новому).
            let mut buf = vec![0.0f32; FFT_SIZE];
            let start = g.write;
            for (i, slot) in buf.iter_mut().enumerate() {
                *slot = g.ring[(start + i) % FFT_SIZE];
            }
            (buf, g.sample_rate)
        };

        // Тяжёлую часть (БПФ) считаем уже без блокировки.
        compute_bars_from_samples(&samples, sample_rate, bars)
    }
}

/// Спланированный rustfft на текущий размер окна. Планировщик кэширует план
/// внутри, поэтому повторные вызовы (на каждом кадре) дёшевы; SIMD-ядро
/// (SSE/AVX/NEON) выбирается под CPU автоматически.
fn fft_plan(n: usize) -> Arc<dyn Fft<f32>> {
    static PLANNER: OnceLock<Mutex<FftPlanner<f32>>> = OnceLock::new();
    PLANNER
        .get_or_init(|| Mutex::new(FftPlanner::new()))
        .lock()
        .map(|mut p| p.plan_fft_forward(n))
        .expect("FFT planner poisoned")
}

thread_local! {
    /// Кэш окна Ханна (пара «размер → окно»): косинусы считаются один раз, а не
    /// на каждом кадре rAF.
    static HANN_CACHE: RefCell<(usize, Vec<f32>)> = const { RefCell::new((0, Vec::new())) };
    /// Переиспользуемый scratch-буфер rustfft — без аллокаций на кадр.
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
        // Логарифмически разнесённые границы полосы.
        let lo = FREQ_MIN * ratio.powf(b as f32 / bars as f32);
        let hi = FREQ_MIN * ratio.powf((b + 1) as f32 / bars as f32);
        let bin_lo = ((lo / bin_hz).floor() as usize).max(1);
        let bin_hi = ((hi / bin_hz).ceil() as usize).clamp(bin_lo + 1, half);

        let mut sum = 0.0f32;
        for bin in bin_lo..bin_hi {
            // Нормируем магнитуду на размер окна, чтобы шкала не зависела от FFT_SIZE.
            sum += buffer[bin].norm() / n as f32;
        }
        let avg = sum / (bin_hi - bin_lo) as f32;

        // Перцептивная шкала: магнитуда → дБ → [0, 1].
        let db = 20.0 * (avg + 1e-7).log10();
        let norm = ((db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0);

        // Лёгкий подъём верхов: высокочастотные полосы обычно тише, без наклона
        // визуализатор «проседает» справа.
        let tilt = 1.0 + 0.5 * (b as f32 / bars.max(1) as f32);
        *slot = (norm * tilt).clamp(0.0, 1.0);
    }
    out
}
