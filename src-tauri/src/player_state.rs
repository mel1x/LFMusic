use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Состояние плеера, сохраняемое между сессиями
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPlayerState {
    /// Путь к текущему треку
    pub current_track_path: Option<String>,
    /// Позиция воспроизведения в секундах
    pub position_secs: f64,
    /// Громкость (0.0 - 1.0)
    pub volume: f32,
    /// Включен ли shuffle
    pub shuffled: bool,
    /// Режим повтора: "queue" или "track"
    pub repeat_mode: String,
    /// Источник активного плейлиста (catalog, catalog:genre:Rock, playlist:3, ...)
    pub active_playlist_source: Option<String>,
    /// Индекс текущего трека в источнике
    pub current_track_index: Option<usize>,
    /// Текущий маршрут (страница)
    pub current_route: Option<String>,
}

impl Default for SavedPlayerState {
    fn default() -> Self {
        Self {
            current_track_path: None,
            position_secs: 0.0,
            volume: 1.0,
            shuffled: false,
            repeat_mode: "queue".to_string(),
            active_playlist_source: None,
            current_track_index: None,
            current_route: None,
        }
    }
}

/// Загрузить состояние плеера из файла
pub fn load(app_data_dir: &PathBuf) -> SavedPlayerState {
    let path = app_data_dir.join("player_state.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Сохранить состояние плеера в файл
pub fn save(app_data_dir: &PathBuf, state: &SavedPlayerState) -> Result<(), String> {
    let path = app_data_dir.join("player_state.json");
    let content = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    std::fs::write(&path, content).map_err(|e| e.to_string())
}
