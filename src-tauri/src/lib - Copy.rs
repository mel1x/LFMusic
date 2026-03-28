mod audio_player;

use audio_player::{AudioPlayer, PlayerStatus, TrackMetadata};
use std::sync::Mutex;
use tauri::State;

/// Обёртка для хранения плеера в Tauri State
pub struct PlayerState(pub Mutex<AudioPlayer>);

/// Воспроизвести аудиофайл по указанному пути
#[tauri::command]
fn player_play(state: State<PlayerState>, file_path: String) -> Result<TrackMetadata, String> {
    let mut player = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    player.play(&file_path)
}

/// Поставить на паузу
#[tauri::command]
fn player_pause(state: State<PlayerState>) -> Result<(), String> {
    let mut player = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    player.pause();
    Ok(())
}

/// Продолжить воспроизведение
#[tauri::command]
fn player_resume(state: State<PlayerState>) -> Result<(), String> {
    let mut player = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    player.resume();
    Ok(())
}

/// Остановить воспроизведение
#[tauri::command]
fn player_stop(state: State<PlayerState>) -> Result<(), String> {
    let mut player = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    player.stop();
    Ok(())
}

/// Установить громкость (0.0 - 1.0)
#[tauri::command]
fn player_set_volume(state: State<PlayerState>, volume: f32) -> Result<(), String> {
    let mut player = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    player.set_volume(volume);
    Ok(())
}

/// Получить текущую громкость
#[tauri::command]
fn player_get_volume(state: State<PlayerState>) -> Result<f32, String> {
    let player = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_volume())
}

/// Перемотка на указанную позицию (в секундах)
#[tauri::command]
fn player_seek(state: State<PlayerState>, position_secs: f64) -> Result<(), String> {
    let mut player = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    player.seek(position_secs)
}

/// Получить текущий статус плеера
#[tauri::command]
fn player_get_status(state: State<PlayerState>) -> Result<PlayerStatus, String> {
    let mut player = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_status())
}

/// Получить метаданные файла без воспроизведения
#[tauri::command]
fn get_track_metadata(file_path: String) -> Result<TrackMetadata, String> {
    AudioPlayer::get_metadata(&file_path)
}

/// Получить список поддерживаемых форматов
#[tauri::command]
fn get_supported_formats() -> Vec<String> {
    AudioPlayer::supported_formats()
}

/// Сканировать директорию и вернуть список аудиофайлов
#[tauri::command]
fn scan_directory(dir_path: String) -> Result<Vec<TrackMetadata>, String> {
    AudioPlayer::scan_directory(&dir_path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Инициализируем плеер
    let player = AudioPlayer::new().expect("Не удалось инициализировать аудиоплеер");

    tauri::Builder::default()
        .manage(PlayerState(Mutex::new(player)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            player_play,
            player_pause,
            player_resume,
            player_stop,
            player_set_volume,
            player_get_volume,
            player_seek,
            player_get_status,
            get_track_metadata,
            get_supported_formats,
            scan_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
