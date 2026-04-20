mod audio_player;
mod library;
mod player_state;
mod settings;

use audio_player::{AudioPlayer, EqSettings, PlayerStatus, TrackMetadata};
use library::{FolderInfo, MusicLibrary, PlaylistInfo};
use player_state::SavedPlayerState;
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
pub struct SettingsState(pub Mutex<(settings::AppSettings, PathBuf)>);

// ──────────────────────────────────────────────────────────────────────────────
// Команды плеера
// ──────────────────────────────────────────────────────────────────────────────

/// Применить кэшированный accent_color из БД или сохранить новый
fn apply_accent_color(library: &State<LibraryState>, file_path: &str, metadata: &mut TrackMetadata) {
    let cached_accent = library.0.lock().ok().and_then(|lib| lib.get_accent_color(file_path));
    if let Some(cached) = cached_accent {
        metadata.accent_color = Some(cached);
    } else if let Some(ref color) = metadata.accent_color {
        if let Ok(lib) = library.0.lock() {
            let _ = lib.set_accent_color(file_path, color);
        }
    }
}

#[tauri::command]
fn player_play(
    state: State<PlayerState>,
    library: State<LibraryState>,
    file_path: String,
) -> Result<TrackMetadata, String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    let mut metadata = player.play(&file_path)?;
    drop(player);
    apply_accent_color(&library, &file_path, &mut metadata);
    Ok(metadata)
}

#[tauri::command]
fn player_play_crossfade(
    state: State<PlayerState>,
    library: State<LibraryState>,
    file_path: String,
    crossfade_secs: f64,
) -> Result<TrackMetadata, String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    let mut metadata = player.play_crossfade(&file_path, crossfade_secs)?;
    drop(player);
    apply_accent_color(&library, &file_path, &mut metadata);
    Ok(metadata)
}

#[tauri::command]
fn player_play_paused(
    state: State<PlayerState>,
    library: State<LibraryState>,
    file_path: String,
    position_secs: f64,
) -> Result<TrackMetadata, String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    let mut metadata = player.play_paused(&file_path, position_secs)?;
    drop(player);
    apply_accent_color(&library, &file_path, &mut metadata);
    Ok(metadata)
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

/// Поднять трек на первое место в каталоге (навсегда)
#[tauri::command]
fn library_move_track_to_top(file_path: String, state: State<LibraryState>) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.move_track_to_top(&file_path)
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
// Команды плейлистов
// ──────────────────────────────────────────────────────────────────────────────

/// Получить все плейлисты
#[tauri::command]
fn playlist_get_all(state: State<LibraryState>) -> Result<Vec<PlaylistInfo>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_playlists())
}

/// Получить треки плейлиста
#[tauri::command]
fn playlist_get_tracks(state: State<LibraryState>, playlist_id: i64) -> Result<Vec<TrackMetadata>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_playlist_tracks(playlist_id))
}

/// Создать пользовательский плейлист
#[tauri::command]
fn playlist_create(state: State<LibraryState>, name: String) -> Result<PlaylistInfo, String> {
    let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.create_playlist(name)
}

/// Переименовать плейлист
#[tauri::command]
fn playlist_rename(state: State<LibraryState>, playlist_id: i64, name: String) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.rename_playlist(playlist_id, name)
}

/// Удалить пользовательский плейлист
#[tauri::command]
fn playlist_delete(state: State<LibraryState>, playlist_id: i64) -> Result<(), String> {
    let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.delete_playlist(playlist_id)
}

/// Добавить трек в плейлист
#[tauri::command]
fn playlist_add_track(state: State<LibraryState>, playlist_id: i64, file_path: String) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.add_track_to_playlist(playlist_id, &file_path)
}

/// Удалить трек из плейлиста
#[tauri::command]
fn playlist_remove_track(state: State<LibraryState>, playlist_id: i64, file_path: String) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.remove_track_from_playlist(playlist_id, &file_path)
}

/// Поднять трек на первое место в плейлисте
#[tauri::command]
fn playlist_move_track_to_top(state: State<LibraryState>, playlist_id: i64, file_path: String) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.move_playlist_track_to_top(playlist_id, &file_path)
}

/// Получить id пользовательских плейлистов, в которых уже есть указанный трек
#[tauri::command]
fn playlist_get_ids_for_track(state: State<LibraryState>, file_path: String) -> Result<Vec<i64>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_user_playlist_ids_for_track(&file_path))
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды эквалайзера
// ──────────────────────────────────────────────────────────────────────────────

/// Сохранить EQ в общий settings.json
fn eq_persist(settings_state: &State<SettingsState>, eq: &EqSettings) -> Result<(), String> {
    let mut state = settings_state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    state.0.eq_settings = eq.clone();
    settings::save(&state.1, &state.0)
}

#[tauri::command]
fn eq_set_bands(
    player_state: State<PlayerState>,
    settings_state: State<SettingsState>,
    bands: Vec<f32>,
) -> Result<(), String> {
    let player = player_state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    player.set_eq_bands(bands);
    let eq = player.get_eq_settings();
    eq_persist(&settings_state, &eq)
}

#[tauri::command]
fn eq_get_bands(state: State<PlayerState>) -> Result<Vec<f32>, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_eq_bands())
}

#[tauri::command]
fn eq_set_gain(
    player_state: State<PlayerState>,
    settings_state: State<SettingsState>,
    gain: f32,
) -> Result<(), String> {
    let player = player_state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    player.set_eq_gain(gain);
    let eq = player.get_eq_settings();
    eq_persist(&settings_state, &eq)
}

#[tauri::command]
fn eq_get_gain(state: State<PlayerState>) -> Result<f32, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_eq_gain())
}

#[tauri::command]
fn eq_set_enabled(
    player_state: State<PlayerState>,
    settings_state: State<SettingsState>,
    enabled: bool,
) -> Result<(), String> {
    let player = player_state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    player.set_eq_enabled(enabled);
    let eq = player.get_eq_settings();
    eq_persist(&settings_state, &eq)
}

#[tauri::command]
fn eq_get_enabled(state: State<PlayerState>) -> Result<bool, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_eq_enabled())
}

#[tauri::command]
fn eq_get_settings(state: State<PlayerState>) -> Result<EqSettings, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_eq_settings())
}

#[tauri::command]
fn eq_set_settings(
    player_state: State<PlayerState>,
    settings_state: State<SettingsState>,
    eq_settings: EqSettings,
) -> Result<(), String> {
    let player = player_state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    player.set_eq_settings(eq_settings);
    let eq = player.get_eq_settings();
    eq_persist(&settings_state, &eq)
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды настроек
// ──────────────────────────────────────────────────────────────────────────────

/// Фронтенд может хранить опции разных типов (булевые, строковые и т.д.)
#[tauri::command]
fn get_settings(state: State<SettingsState>) -> Result<std::collections::HashMap<String, serde_json::Value>, String> {
    let state = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(state.0.options.clone())
}

#[tauri::command]
fn save_settings(state: State<SettingsState>, settings: std::collections::HashMap<String, serde_json::Value>) -> Result<(), String> {
    let mut state = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    state.0.options = settings;
    settings::save(&state.1, &state.0)
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

            // Загружаем сохранённые настройки эквалайзера из settings.json
            let player_state = app.state::<PlayerState>();
            let player = player_state.0.read().expect("Lock error");
            player.set_eq_settings(app_settings.eq_settings.clone());
            drop(player);

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
            player_play_paused,
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
            library_move_track_to_top,
            library_cleanup,
            library_rescan,
            // Плейлисты
            playlist_get_all,
            playlist_get_tracks,
            playlist_create,
            playlist_rename,
            playlist_delete,
            playlist_add_track,
            playlist_remove_track,
            playlist_move_track_to_top,
            playlist_get_ids_for_track,
            // Эквалайзер
            eq_set_bands,
            eq_get_bands,
            eq_set_gain,
            eq_get_gain,
            eq_set_enabled,
            eq_get_enabled,
            eq_get_settings,
            eq_set_settings,
            // Настройки
            get_settings,
            save_settings,
            // Состояние плеера
            save_player_state,
            load_player_state,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app_handle.try_state::<LibraryState>() {
                    if let Ok(library) = state.0.lock() {
                        if let Err(e) = library.shutdown() {
                            eprintln!("Library shutdown error: {}", e);
                        }
                    }
                }
            }
        });
}