use std::collections::HashMap;
use std::path::PathBuf;

use tauri::State;

use crate::audio_player::EqSettings;
use crate::SettingsState;

/// Разделители исполнителей по умолчанию
pub const DEFAULT_ARTIST_SEPARATORS: &[&str] = &["; ", ", "];

/// Настройки приложения: произвольные опции + эквалайзер в одном файле
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct AppSettings {
    /// Опции фронтенда произвольного типа (bool/string/...):
    /// crossfade_on_seek, show_github_badge, static_accent_color и т.д.
    #[serde(flatten)]
    pub options: HashMap<String, serde_json::Value>,

    /// Настройки эквалайзера
    #[serde(default)]
    pub eq_settings: EqSettings,
}

pub fn load(app_data_dir: &PathBuf) -> AppSettings {
    let path = app_data_dir.join("settings.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(app_data_dir: &PathBuf, settings: &AppSettings) -> Result<(), String> {
    let path = app_data_dir.join("settings.json");
    let content = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

/// Прочитать список разделителей исполнителей из текущих настроек.
/// Если в options нет artist_separators или значение некорректно — возвращает дефолт.
pub fn get_artist_separators(state: &State<SettingsState>) -> Vec<String> {
    let guard = match state.0.lock() {
        Ok(g) => g,
        Err(_) => return DEFAULT_ARTIST_SEPARATORS.iter().map(|s| s.to_string()).collect(),
    };
    if let Some(serde_json::Value::Array(arr)) = guard.0.options.get("artist_separators") {
        let parsed: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .collect();
        if !parsed.is_empty() {
            return parsed;
        }
    }
    DEFAULT_ARTIST_SEPARATORS.iter().map(|s| s.to_string()).collect()
}
