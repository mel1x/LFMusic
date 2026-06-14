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
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use souvlaki::{
        MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition,
        PlatformConfig,
    };
    use tauri::{AppHandle, Emitter};

    // MediaControls держит dbus-поток (souvlaki). Создаётся один раз при старте.
    static CONTROLS: OnceLock<Mutex<MediaControls>> = OnceLock::new();

    pub fn init(app: &AppHandle, _cover_dir: PathBuf) {
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

        let _ = CONTROLS.set(Mutex::new(controls));
    }

    pub fn update_metadata(
        app: &AppHandle,
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        file_path: Option<String>,
        duration_secs: f64,
    ) {
        let Some(lock) = CONTROLS.get() else { return };
        // Обложка — file:// на кэшированную WebP-миниатюру по file_path трека
        // (генерируется лениво). Путь меняется при смене обложки (сигнатура в
        // имени файла), поэтому MPRIS-клиенты корректно обновляют artwork.
        let cover_url = file_path
            .as_deref()
            .and_then(|fp| crate::thumbnails::cover_file(app, fp, 512))
            .map(|p| format!("file://{}", p.to_string_lossy()));
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
        _app: &AppHandle,
        _title: Option<String>,
        _artist: Option<String>,
        _album: Option<String>,
        _file_path: Option<String>,
        _duration_secs: f64,
    ) {
    }
    pub fn update_playback(_state: &str, _position_secs: f64) {}
}

#[allow(unused_imports)]
pub use imp::{init, update_metadata, update_playback};
