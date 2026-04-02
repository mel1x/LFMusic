use base64::Engine;
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::probe::Probe;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc;
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
}

/// Состояние воспроизведения
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

/// Определить кодек через symphonia (нужно только для M4A/MP4 контейнеров)
fn probe_actual_codec(file_path: &str, extension: &str) -> Option<CodecType> {
    let file = File::open(file_path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(extension);

    symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()
        .and_then(|probed| {
            probed
                .format
                .tracks()
                .iter()
                .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
                .map(|t| t.codec_params.codec)
        })
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
) {
    let mut sink: Option<Sink> = None;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            AudioCmd::PlayFrom { file_path, seek_secs, volume, resp } => {
                if let Some(ref s) = sink { s.stop(); }
                let result = (|| -> Result<(), String> {
                    let new_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Не удалось создать sink: {}", e))?;
                    new_sink.set_volume(volume);
                    new_sink.append(SymphoniaSource::new_with_seek(&file_path, seek_secs)?);
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
                    new_sink.append(SymphoniaSource::new_with_seek(&file_path, position_secs)?);
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

                    // Fade-out на отдельном sink, уходит в фон
                    let fade_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Ошибка fade-out sink: {}", e))?;
                    fade_sink.set_volume(volume);
                    fade_sink.append(FadeOut::new(
                        SymphoniaSource::new_with_seek(&file_path, current_pos)?,
                        dur,
                    ));

                    if let Some(ref old) = sink { old.stop(); }
                    std::thread::spawn(move || { fade_sink.sleep_until_end(); });

                    // Новый sink с fade-in
                    let new_sink = Sink::try_new(&stream_handle)
                        .map_err(|e| format!("Ошибка нового sink: {}", e))?;
                    new_sink.set_volume(volume);
                    new_sink.append(
                        SymphoniaSource::new_with_seek(&file_path, position_secs)?
                            .fade_in(dur),
                    );
                    if was_paused { new_sink.pause(); }
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
}

impl AudioPlayer {
    /// Создать новый экземпляр плеера (запускает аудио-поток)
    pub fn new() -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (init_tx, init_rx) = mpsc::sync_channel::<Result<(), String>>(0);

        std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || {
                match OutputStream::try_default() {
                    Ok((stream, handle)) => {
                        let _ = init_tx.send(Ok(()));
                        audio_thread_loop(cmd_rx, stream, handle);
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
        })
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

    /// Извлечь обложку трека как data URL
    pub fn extract_cover_art(file_path: &str) -> Option<String> {
        let tagged_file = Probe::open(PathBuf::from(file_path)).ok()?.read().ok()?;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())?;
        cover_art_from_tag(tag)
    }

    /// Извлечь метаданные из аудиофайла.
    ///
    /// Для однозначных форматов (FLAC, MP3, WAV, OGG, ...) открывает файл один раз (lofty).
    /// Symphonia-проба используется только для контейнеров M4A/MP4, где кодек неоднозначен
    /// (AAC vs ALAC).
    pub fn get_metadata(file_path: &str) -> Result<TrackMetadata, String> {
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
        let mut duration_secs = None;
        let mut sample_rate = None;
        let mut bit_depth = None;
        let mut channels = None;
        let mut bitrate = None;
        let mut cover_art: Option<String> = None;

        if let Ok(probe) = Probe::open(&path) {
            if let Ok(tagged_file) = probe.read() {
                if let Some(tag) = tagged_file
                    .primary_tag()
                    .or_else(|| tagged_file.first_tag())
                {
                    title = tag.title().map(|s| s.to_string());
                    artist = tag.artist().map(|s| s.to_string());
                    album = tag.album().map(|s| s.to_string());
                    cover_art = cover_art_from_tag(tag);
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
            duration_secs,
            format,
            sample_rate,
            bit_depth,
            channels,
            bitrate,
            is_lossless,
            file_path: file_path.to_string(),
            cover_art,
        })
    }

    /// Получить список поддерживаемых форматов
    pub fn supported_formats() -> Vec<String> {
        vec![
            "flac".to_string(), "wav".to_string(), "mp3".to_string(),
            "ogg".to_string(), "aac".to_string(), "m4a".to_string(),
            "mp4".to_string(), "aiff".to_string(), "aif".to_string(),
            "opus".to_string(),
        ]
    }

    // ── Управление воспроизведением ─────────────────────────────────────────

    /// Загрузить и воспроизвести файл
    pub fn play(&mut self, file_path: &str) -> Result<TrackMetadata, String> {
        self.play_from(file_path, 0.0)
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
            state: self.state.clone(),
            volume: self.volume,
            position_secs: self.get_position(),
            current_file_path: self.current_track.as_ref().map(|t| t.file_path.clone()),
        }
    }
}
