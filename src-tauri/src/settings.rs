use std::collections::HashMap;
use std::path::PathBuf;

use crate::audio_player::EqSettings;

/// Настройки приложения: булевые опции + эквалайзер в одном файле
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct AppSettings {
    /// Булевые переключатели (crossfade_on_seek, show_github_badge и т.д.)
    #[serde(flatten)]
    pub options: HashMap<String, serde_json::Value>,

    /// Настройки эквалайзера
    #[serde(default)]
    pub eq_settings: EqSettings,
}

impl AppSettings {
    /// Получить только булевые опции (для фронтенда)
    pub fn get_options(&self) -> HashMap<String, bool> {
        self.options
            .iter()
            .filter_map(|(k, v)| v.as_bool().map(|b| (k.clone(), b)))
            .collect()
    }

    /// Установить булевые опции (от фронтенда)
    pub fn set_options(&mut self, opts: HashMap<String, bool>) {
        self.options = opts.into_iter().map(|(k, v)| (k, serde_json::Value::Bool(v))).collect();
    }
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
