//! Интеграция с системным медиа-управлением на Linux через MPRIS (souvlaki):
//!  * в системе (GNOME/KDE/панели, экран блокировки) отображается текущий трек
//!    (название, исполнитель, альбом, обложка);
//!  * системные кнопки play/pause/next/prev/stop прокидываются в плеер.
//!
//! Метаданные и состояние присылает фронтенд (он держит всю реактивную модель),
//! а входящие команды эмитятся как событие `mpris_media` — его слушает Player.vue.
//!
//! Весь модуль — no-op на не-Linux платформах (см. заглушки в конце файла).

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use souvlaki::{
        MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition,
        PlatformConfig,
    };
    use tauri::{AppHandle, Emitter};

    // MediaControls держит dbus-поток (souvlaki). Создаётся один раз при старте.
    static CONTROLS: OnceLock<Mutex<MediaControls>> = OnceLock::new();
    // Директория для временного файла обложки (MPRIS art принимает file:// URL).
    static COVER_DIR: OnceLock<PathBuf> = OnceLock::new();
    // Сигнатура последней обложки + её file:// URL — чтобы не перезаписывать файл
    // на каждое обновление позиции и менять путь только при смене обложки.
    static LAST_COVER_SIG: AtomicU64 = AtomicU64::new(0);
    static COVER_SEQ: AtomicU64 = AtomicU64::new(0);
    static LAST_COVER_URL: Mutex<Option<String>> = Mutex::new(None);

    pub fn init(app: &AppHandle, cover_dir: PathBuf) {
        let config = PlatformConfig {
            dbus_name: "lfmusic",
            display_name: "LFMusic",
            hwnd: None,
        };

        let mut controls = match MediaControls::new(config) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("MPRIS: не удалось инициализировать медиа-управление: {:?}", e);
                return;
            }
        };

        let handle = app.clone();
        let attach = controls.attach(move |event: MediaControlEvent| {
            let action = match event {
                MediaControlEvent::Play => Some("play"),
                MediaControlEvent::Pause => Some("pause"),
                MediaControlEvent::Toggle => Some("playpause"),
                MediaControlEvent::Next => Some("next"),
                MediaControlEvent::Previous => Some("prev"),
                MediaControlEvent::Stop => Some("stop"),
                _ => None,
            };
            if let Some(action) = action {
                let _ = handle.emit("mpris_media", action);
            }
        });

        if let Err(e) = attach {
            eprintln!("MPRIS: не удалось подписаться на события: {:?}", e);
            return;
        }

        let _ = COVER_DIR.set(cover_dir);
        let _ = CONTROLS.set(Mutex::new(controls));
    }

    pub fn update_metadata(
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        cover: Option<String>,
        duration_secs: f64,
    ) {
        let Some(lock) = CONTROLS.get() else { return };
        let cover_url = cover_to_url(cover);
        let Ok(mut controls) = lock.lock() else { return };
        let duration = if duration_secs > 0.0 {
            Some(Duration::from_secs_f64(duration_secs))
        } else {
            None
        };
        let _ = controls.set_metadata(MediaMetadata {
            title: title.as_deref(),
            album: album.as_deref(),
            artist: artist.as_deref(),
            cover_url: cover_url.as_deref(),
            duration,
        });
    }

    pub fn update_playback(state: &str, position_secs: f64) {
        let Some(lock) = CONTROLS.get() else { return };
        let Ok(mut controls) = lock.lock() else { return };
        let progress = Some(MediaPosition(Duration::from_secs_f64(position_secs.max(0.0))));
        let playback = match state {
            "Playing" => MediaPlayback::Playing { progress },
            "Paused" => MediaPlayback::Paused { progress },
            _ => MediaPlayback::Stopped,
        };
        let _ = controls.set_playback(playback);
    }

    /// data:URL обложки → временный файл → file:// URL. Кэшируется по сигнатуре,
    /// чтобы не декодировать/не писать файл повторно для того же изображения.
    fn cover_to_url(cover: Option<String>) -> Option<String> {
        let Some(cover) = cover else {
            LAST_COVER_SIG.store(0, Ordering::Relaxed);
            if let Ok(mut last) = LAST_COVER_URL.lock() {
                *last = None;
            }
            return None;
        };

        let sig = {
            let mut h = DefaultHasher::new();
            cover.hash(&mut h);
            h.finish().max(1)
        };
        if LAST_COVER_SIG.load(Ordering::Relaxed) == sig {
            if let Ok(last) = LAST_COVER_URL.lock() {
                return last.clone();
            }
        }

        let url = write_cover_file(&cover);
        LAST_COVER_SIG.store(sig, Ordering::Relaxed);
        if let Ok(mut last) = LAST_COVER_URL.lock() {
            *last = url.clone();
        }
        url
    }

    fn write_cover_file(data_url: &str) -> Option<String> {
        let dir = COVER_DIR.get()?;
        // Парсим `data:image/<ext>;base64,<payload>`.
        let comma = data_url.find(',')?;
        let header = &data_url[..comma];
        let payload = &data_url[comma + 1..];
        let ext = if header.contains("png") {
            "png"
        } else if header.contains("webp") {
            "webp"
        } else {
            "jpg"
        };
        let bytes = STANDARD.decode(payload.as_bytes()).ok()?;

        // Уникальное имя на смену обложки — клиенты MPRIS кэшируют art по URL.
        let seq = COVER_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("mpris-cover-{seq}.{ext}"));
        std::fs::write(&path, &bytes).ok()?;

        // Подчищаем предыдущий файл, чтобы не копить временные обложки.
        if seq > 0 {
            for old_ext in ["png", "jpg", "webp"] {
                let prev = dir.join(format!("mpris-cover-{}.{}", seq - 1, old_ext));
                let _ = std::fs::remove_file(prev);
            }
        }

        Some(format!("file://{}", path.to_string_lossy()))
    }
}

// ── Заглушки для не-Linux платформ ───────────────────────────────────────────
#[cfg(not(target_os = "linux"))]
mod imp {
    use std::path::PathBuf;
    use tauri::AppHandle;

    // `init` вызывается только из linux-ветки setup(), на прочих платформах не нужен.
    #[allow(dead_code)]
    pub fn init(_app: &AppHandle, _cover_dir: PathBuf) {}
    pub fn update_metadata(
        _title: Option<String>,
        _artist: Option<String>,
        _album: Option<String>,
        _cover: Option<String>,
        _duration_secs: f64,
    ) {
    }
    pub fn update_playback(_state: &str, _position_secs: f64) {}
}

#[allow(unused_imports)]
pub use imp::{init, update_metadata, update_playback};
