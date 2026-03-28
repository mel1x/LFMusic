use lofty::prelude::*;
use lofty::probe::Probe;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::BufReader;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

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
}

/// Состояние воспроизведения
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Информация о текущем состоянии плеера
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStatus {
    pub state: PlaybackState,
    pub current_track: Option<TrackMetadata>,
    pub volume: f32,
    pub position_secs: f64,
}

/// Symphonia-based аудио источник для rodio
pub struct SymphoniaSource {
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    format_reader: Box<dyn symphonia::core::formats::FormatReader>,
    track_id: u32,
    sample_rate: u32,
    channels: u16,
    current_samples: Vec<f32>,
    current_pos: usize,
}

/// Извлечь сэмплы из AudioBufferRef в вектор f32
fn extract_samples_into(buf: &AudioBufferRef, samples: &mut Vec<f32>) {
    match buf {
        AudioBufferRef::F32(b) => {
            for frame in 0..b.frames() {
                for ch in 0..b.spec().channels.count() {
                    samples.push(b.chan(ch)[frame]);
                }
            }
        }
        AudioBufferRef::S32(b) => {
            for frame in 0..b.frames() {
                for ch in 0..b.spec().channels.count() {
                    samples.push(b.chan(ch)[frame] as f32 / i32::MAX as f32);
                }
            }
        }
        AudioBufferRef::S16(b) => {
            for frame in 0..b.frames() {
                for ch in 0..b.spec().channels.count() {
                    samples.push(b.chan(ch)[frame] as f32 / i16::MAX as f32);
                }
            }
        }
        AudioBufferRef::U8(b) => {
            for frame in 0..b.frames() {
                for ch in 0..b.spec().channels.count() {
                    samples.push((b.chan(ch)[frame] as f32 - 128.0) / 128.0);
                }
            }
        }
        _ => {
            // Fallback для других форматов
        }
    }
}

impl SymphoniaSource {
    fn new_with_seek(file_path: &str, seek_secs: f64) -> Result<Self, String> {
        let mut source = Self::new(file_path)?;
        
        if seek_secs > 0.0 {
            use symphonia::core::formats::{SeekMode, SeekTo};
            use symphonia::core::units::Time;

            let time = Time::from(seek_secs);
            source
                .format_reader
                .seek(
                    SeekMode::Accurate,
                    SeekTo::Time { time, track_id: Some(source.track_id) },
                )
                .map_err(|e| format!("Ошибка seek в файле: {}", e))?;

            // Сбрасываем декодер — он мог накопить состояние (важно для FLAC!)
            source.decoder.reset();
            source.current_samples.clear();
            source.current_pos = 0;
        }

        Ok(source)
    }

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
                        Err(symphonia::core::errors::Error::DecodeError(_)) => {
                            // Пропускаем повреждённые пакеты
                            continue;
                        }
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
        if remaining > 0 {
            Some(remaining)
        } else {
            None
        }
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Основная структура аудиоплеера
pub struct AudioPlayer {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sink: Option<Sink>,
    current_track: Option<TrackMetadata>,
    state: PlaybackState,
    volume: f32,
    /// Момент начала воспроизведения (или возобновления)
    playback_started_at: Option<Instant>,
    /// Накопленная позиция до паузы (в секундах)
    accumulated_position: f64,
}

// OutputStream содержит *mut () который не Send/Sync,
// но мы гарантируем безопасность через Mutex в lib.rs
unsafe impl Send for AudioPlayer {}
unsafe impl Sync for AudioPlayer {}

impl AudioPlayer {
    /// Создать новый экземпляр плеера
    pub fn new() -> Result<Self, String> {
        let (stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| format!("Не удалось инициализировать аудио выход: {}", e))?;

        Ok(Self {
            _stream: stream,
            stream_handle,
            sink: None,
            current_track: None,
            state: PlaybackState::Stopped,
            volume: 1.0,
            playback_started_at: None,
            accumulated_position: 0.0,
        })
    }

    /// Проверить, является ли формат MP4-контейнером (m4a, aac, mp4)
    fn is_mp4_container(extension: &str) -> bool {
        matches!(extension, "m4a" | "aac" | "mp4" | "m4b" | "m4p" | "m4r")
    }

    /// Извлечь метаданные из аудиофайла
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

        let actual_codec = {
            let file = File::open(file_path)
                .map_err(|e| format!("Не удалось открыть файл: {}", e))?;
            let mss = MediaSourceStream::new(Box::new(file), Default::default());
            let mut hint = Hint::new();
            hint.with_extension(&extension);

            symphonia::default::get_probe()
                .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
                .ok()
                .and_then(|probed| {
                    probed.format
                        .tracks()
                        .iter()
                        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
                        .map(|t| t.codec_params.codec)
                })
        };

        let is_lossless = match actual_codec {
            Some(codec) => matches!(
                codec,
                symphonia::core::codecs::CODEC_TYPE_FLAC
                    | symphonia::core::codecs::CODEC_TYPE_ALAC
                    | symphonia::core::codecs::CODEC_TYPE_PCM_S16LE
                    | symphonia::core::codecs::CODEC_TYPE_PCM_S16BE
                    | symphonia::core::codecs::CODEC_TYPE_PCM_S24LE
                    | symphonia::core::codecs::CODEC_TYPE_PCM_S32LE
                    | symphonia::core::codecs::CODEC_TYPE_PCM_F32LE
            ),
            // Фолбэк по расширению если Symphonia не смог определить
            None => matches!(
                extension.as_str(),
                "flac" | "wav" | "alac" | "ape" | "wv" | "aiff" | "aif"
            ),
        };

        let format = match extension.as_str() {
            "flac" => "FLAC",
            "wav" => "WAV",
            "mp3" => "MP3",
            "ogg" => "OGG Vorbis",
            "aac" => "AAC",
            "m4a" => "M4A (AAC)",
            "mp4" => "MP4",
            "alac" => "ALAC",
            "ape" => "APE",
            "wv" => "WavPack",
            "aiff" | "aif" => "AIFF",
            "opus" => "Opus",
            other => other,
        }
        .to_string();

        // Читаем метаданные через lofty
        let mut title = None;
        let mut artist = None;
        let mut album = None;
        let mut duration_secs = None;
        let mut sample_rate = None;
        let mut bit_depth = None;
        let mut channels = None;
        let mut bitrate = None;

        if let Ok(tagged_file) = Probe::open(&path)
            .map_err(|e| format!("Ошибка открытия файла: {}", e))
            .and_then(|probe| {
                probe
                    .read()
                    .map_err(|e| format!("Ошибка чтения тегов: {}", e))
            })
        {
            // Получаем теги
            if let Some(tag) = tagged_file
                .primary_tag()
                .or_else(|| tagged_file.first_tag())
            {
                title = tag.title().map(|s| s.to_string());
                artist = tag.artist().map(|s| s.to_string());
                album = tag.album().map(|s| s.to_string());
            }

            // Получаем свойства аудио
            let properties = tagged_file.properties();
            duration_secs = Some(properties.duration().as_secs_f64());
            sample_rate = properties.sample_rate();
            bit_depth = properties.bit_depth();
            channels = properties.channels();
            bitrate = properties.audio_bitrate();
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
        })
    }

    /// Загрузить и воспроизвести файл
    pub fn play(&mut self, file_path: &str) -> Result<TrackMetadata, String> {
        self.play_from(file_path, 0.0)
    }

    /// Поставить на паузу
    pub fn pause(&mut self) {
        if let Some(ref sink) = self.sink {
            if !sink.is_paused() {
                sink.pause();
                // Накапливаем время, прошедшее с момента начала/возобновления
                if let Some(started) = self.playback_started_at.take() {
                    self.accumulated_position += started.elapsed().as_secs_f64();
                }
                self.state = PlaybackState::Paused;
            }
        }
    }

    fn play_from(&mut self, file_path: &str, seek_secs: f64) -> Result<TrackMetadata, String> {
        let metadata = Self::get_metadata(file_path)?;

        if let Some(ref sink) = self.sink {
            sink.stop();
        }

        let new_sink = Sink::try_new(&self.stream_handle)
            .map_err(|e| format!("Не удалось создать sink: {}", e))?;
        new_sink.set_volume(self.volume);

        // Используем SymphoniaSource для ВСЕХ форматов — он поддерживает seek
        let source = SymphoniaSource::new_with_seek(file_path, seek_secs)?;
        new_sink.append(source);

        self.sink = Some(new_sink);
        self.current_track = Some(metadata.clone());
        self.state = PlaybackState::Playing;
        self.playback_started_at = Some(Instant::now());
        self.accumulated_position = seek_secs;

        Ok(metadata)
    }

    /// Продолжить воспроизведение
    pub fn resume(&mut self) {
        if let Some(ref sink) = self.sink {
            if sink.is_paused() {
                sink.play();
                self.playback_started_at = Some(Instant::now());
                self.state = PlaybackState::Playing;
            }
        }
    }

    /// Остановить воспроизведение
    pub fn stop(&mut self) {
        if let Some(ref sink) = self.sink {
            sink.stop();
        }
        self.sink = None;
        self.state = PlaybackState::Stopped;
        self.current_track = None;
        self.playback_started_at = None;
        self.accumulated_position = 0.0;
    }

    /// Установить громкость (0.0 - 1.0)
    pub fn set_volume(&mut self, volume: f32) {
        let vol = volume.clamp(0.0, 1.0);
        if let Some(ref sink) = self.sink {
            sink.set_volume(vol);
        }
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

        let was_paused = self.sink.as_ref().map(|s| s.is_paused()).unwrap_or(false);

        // Останавливаем старый sink — очищает буфер, убирает "whoosh"
        if let Some(ref sink) = self.sink {
            sink.stop();
        }

        let new_sink = Sink::try_new(&self.stream_handle)
            .map_err(|e| format!("Не удалось создать sink: {}", e))?;
        new_sink.set_volume(self.volume);

        let source = SymphoniaSource::new_with_seek(&file_path, position_secs)?;
        new_sink.append(source);

        if was_paused {
            new_sink.pause();
        }

        self.sink = Some(new_sink);
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
        // Ограничиваем позицию длительностью трека
        if let Some(ref track) = self.current_track {
            if let Some(duration) = track.duration_secs {
                return pos.min(duration);
            }
        }
        pos
    }

    /// Получить текущий статус плеера
    pub fn get_status(&mut self) -> PlayerStatus {
        // Проверяем, не закончился ли трек
        if self.state == PlaybackState::Playing {
            if let Some(ref sink) = self.sink {
                if sink.empty() {
                    self.state = PlaybackState::Stopped;
                    self.current_track = None;
                    self.sink = None;
                    self.playback_started_at = None;
                    self.accumulated_position = 0.0;
                }
            }
        }

        let position_secs = self.get_position();

        PlayerStatus {
            state: self.state.clone(),
            current_track: self.current_track.clone(),
            volume: self.volume,
            position_secs,
        }
    }

    /// Получить список поддерживаемых форматов
    pub fn supported_formats() -> Vec<String> {
        vec![
            "flac".to_string(),
            "wav".to_string(),
            "mp3".to_string(),
            "ogg".to_string(),
            "aac".to_string(),
            "m4a".to_string(),
            "mp4".to_string(),
            "aiff".to_string(),
            "aif".to_string(),
            "opus".to_string(),
        ]
    }

    /// Сканировать директорию и вернуть список аудиофайлов с метаданными
    pub fn scan_directory(dir_path: &str) -> Result<Vec<TrackMetadata>, String> {
        let path = PathBuf::from(dir_path);
        if !path.is_dir() {
            return Err(format!("Не является директорией: {}", dir_path));
        }

        let supported = Self::supported_formats();
        let mut tracks = Vec::new();

        let entries = fs::read_dir(&path)
            .map_err(|e| format!("Не удалось прочитать директорию: {}", e))?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let file_path = entry.path();
            if !file_path.is_file() {
                continue;
            }

            let extension = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            if supported.contains(&extension) {
                if let Ok(metadata) = Self::get_metadata(
                    &file_path.to_string_lossy()
                ) {
                    tracks.push(metadata);
                }
            }
        }

        // Сортируем по имени файла
        tracks.sort_by(|a, b| a.file_path.cmp(&b.file_path));

        Ok(tracks)
    }
}
