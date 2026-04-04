mod audio_player;
mod library;
mod player_state;
mod settings;

use audio_player::{AudioPlayer, PlayerStatus, TrackMetadata};
use library::{FolderInfo, MusicLibrary};
use player_state::SavedPlayerState;
use settings::AppSettings;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use tauri::{Emitter, Manager, State};

#[derive(Clone, serde::Serialize)]
struct ScanProgress {
    current: usize,
    total: usize,
    percent: f64,
}

// ──────────────────────────────────────────────────────────────────────────────
// State types
// ──────────────────────────────────────────────────────────────────────────────

pub struct PlayerState(pub RwLock<AudioPlayer>);
pub struct LibraryState(pub Mutex<MusicLibrary>);
pub struct SettingsState(pub Mutex<(AppSettings, PathBuf)>);

// ──────────────────────────────────────────────────────────────────────────────
// Команды плеера (без изменений)
// ──────────────────────────────────────────────────────────────────────────────

#[tauri::command]
fn player_play(state: State<PlayerState>, file_path: String) -> Result<TrackMetadata, String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.play(&file_path)
}

#[tauri::command]
fn player_play_crossfade(
    state: State<PlayerState>,
    file_path: String,
    crossfade_secs: f64,
) -> Result<TrackMetadata, String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.play_crossfade(&file_path, crossfade_secs)
}

#[tauri::command]
fn player_pause(state: State<PlayerState>) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.pause();
    Ok(())
}

#[tauri::command]
fn player_resume(state: State<PlayerState>) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.resume();
    Ok(())
}

#[tauri::command]
fn player_stop(state: State<PlayerState>) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.stop();
    Ok(())
}

#[tauri::command]
fn player_set_volume(state: State<PlayerState>, volume: f32) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.set_volume(volume);
    Ok(())
}

#[tauri::command]
fn player_get_volume(state: State<PlayerState>) -> Result<f32, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_volume())
}

#[tauri::command]
fn player_seek(state: State<PlayerState>, position_secs: f64) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.seek(position_secs)
}

#[tauri::command]
fn player_seek_crossfade(
    state: State<PlayerState>,
    position_secs: f64,
    crossfade_secs: f64,
) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.seek_crossfade(position_secs, crossfade_secs)
}

#[tauri::command]
fn player_get_status(state: State<PlayerState>) -> Result<PlayerStatus, String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_status())
}

#[tauri::command]
fn get_track_metadata(file_path: String) -> Result<TrackMetadata, String> {
    AudioPlayer::get_metadata(&file_path)
}

#[tauri::command]
fn get_cover_art(file_path: String) -> Result<Option<String>, String> {
    Ok(AudioPlayer::extract_cover_art(&file_path))
}

#[tauri::command]
fn get_supported_formats() -> Vec<String> {
    AudioPlayer::supported_formats().iter().map(|s| s.to_string()).collect()
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды библиотеки (новые)
// ──────────────────────────────────────────────────────────────────────────────

/// Получить список папок с количеством треков в каждой
#[tauri::command]
fn library_get_folders(state: State<LibraryState>) -> Result<Vec<FolderInfo>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_folders())
}

/// Добавить папку, просканировать, вернуть все треки из библиотеки
#[tauri::command]
async fn library_add_folder(
    app: tauri::AppHandle,
    folder_path: String,
) -> Result<Vec<TrackMetadata>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<LibraryState>();
        let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
        library.add_folder(folder_path, |current, total| {
            let percent = if total > 0 { (current as f64 / total as f64) * 100.0 } else { 100.0 };
            let _ = app.emit("scan_progress", ScanProgress { current, total, percent });
        })
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

/// Удалить папку и все её треки из кэша
#[tauri::command]
fn library_remove_folder(
    state: State<LibraryState>,
    folder_path: String,
) -> Result<(), String> {
    let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.remove_folder(folder_path)
}

/// Получить все треки из кэша (без пересканирования)
#[tauri::command]
fn library_get_tracks(state: State<LibraryState>) -> Result<Vec<TrackMetadata>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_all_tracks())
}

/// Удалить из кэша треки, файлы которых больше не существуют на диске
#[tauri::command]
fn library_cleanup(state: State<LibraryState>) -> Result<usize, String> {
    let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.cleanup_missing_tracks()
}

/// Пересканировать все папки: обновить изменённые файлы, удалить несуществующие
#[tauri::command]
async fn library_rescan(
    app: tauri::AppHandle,
) -> Result<Vec<TrackMetadata>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<LibraryState>();
        let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
        library.rescan_all(|current, total| {
            let percent = if total > 0 { (current as f64 / total as f64) * 100.0 } else { 100.0 };
            let _ = app.emit("scan_progress", ScanProgress { current, total, percent });
        })
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды настроек
// ──────────────────────────────────────────────────────────────────────────────

#[tauri::command]
fn get_settings(state: State<SettingsState>) -> Result<AppSettings, String> {
    let state = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(state.0.clone())
}

#[tauri::command]
fn save_settings(state: State<SettingsState>, settings: AppSettings) -> Result<(), String> {
    let mut state = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    settings::save(&state.1, &settings)?;
    state.0 = settings;
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды состояния плеера
// ──────────────────────────────────────────────────────────────────────────────

#[tauri::command]
fn save_player_state(
    state: State<SettingsState>,
    player_state: SavedPlayerState,
) -> Result<(), String> {
    let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    player_state::save(&settings.1, &player_state)
}

#[tauri::command]
fn load_player_state(state: State<SettingsState>) -> Result<SavedPlayerState, String> {
    let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player_state::load(&settings.1))
}

// ──────────────────────────────────────────────────────────────────────────────
// Точка входа
// ──────────────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let player = AudioPlayer::new().expect("Не удалось инициализировать аудиоплеер");

    tauri::Builder::default()
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Не удалось получить директорию данных приложения");

            let app_settings = settings::load(&app_data_dir);
            app.manage(SettingsState(Mutex::new((app_settings, app_data_dir.clone()))));

            let library =
                MusicLibrary::new(app_data_dir).expect("Не удалось инициализировать библиотеку");

            app.manage(LibraryState(Mutex::new(library)));
            Ok(())
        })
        .manage(PlayerState(RwLock::new(player)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            // Плеер
            player_play,
            player_play_crossfade,
            player_pause,
            player_resume,
            player_stop,
            player_set_volume,
            player_get_volume,
            player_seek,
            player_seek_crossfade,
            player_get_status,
            get_track_metadata,
            get_cover_art,
            get_supported_formats,
            // Библиотека
            library_get_folders,
            library_add_folder,
            library_remove_folder,
            library_get_tracks,
            library_cleanup,
            library_rescan,
            // Настройки
            get_settings,
            save_settings,
            // Состояние плеера
            save_player_state,
            load_player_state,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}