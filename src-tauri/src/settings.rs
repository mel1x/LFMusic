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

    /// Папка для скачиваемых треков (внешние источники). Пусто/None — стандартная
    /// папка «LFMusic Downloads» в системной музыкальной директории. Храним
    /// ОТДЕЛЬНЫМ полем (а не в `options`), чтобы `save_settings` с фронта, который
    /// целиком заменяет `options`, не затирал её (как и `eq_settings`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_dir: Option<String>,

    /// Режим прокси для yt-dlp: `"system"` (по умолчанию/None — системный прокси
    /// или из окружения), `"none"` (прямое соединение, `--proxy ""`), `"custom"`
    /// (заданный `proxy_url`). Отдельное поле — по той же причине, что и
    /// `download_dir` (не часть flatten-`options`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_mode: Option<String>,

    /// URL прокси для режима `"custom"` (напр. `socks5://127.0.0.1:1080` для
    /// ByeDPI/ByeByeDPI). Пусто/None — режим `custom` сводится к системному.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
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

/// Прочитать булеву опцию из настроек (или `default`, если её нет/иной тип).
pub fn option_bool(settings: &AppSettings, key: &str, default: bool) -> bool {
    settings
        .options
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

/// Прочитать список разделителей исполнителей из текущих настроек.
/// Если в options нет artist_separators или значение некорректно — возвращает дефолт.
pub fn get_artist_separators(state: &State<SettingsState>) -> Vec<String> {
    let guard = match state.0.lock() {
        Ok(g) => g,
        Err(_) => {
            return DEFAULT_ARTIST_SEPARATORS
                .iter()
                .map(|s| s.to_string())
                .collect()
        }
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
    DEFAULT_ARTIST_SEPARATORS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `download_dir` — отдельное поле, а НЕ часть flatten-`options`: оно не должно
    /// просачиваться в `options` при десериализации (иначе `save_settings` с фронта,
    /// заменяющий `options` целиком, затирал бы его). И обратно: значение из
    /// options-ключей не должно попадать в это поле.
    #[test]
    fn download_dir_is_separate_from_options() {
        let json = r##"{
            "crossfade_on_seek": true,
            "static_accent_color": "#3D3D3D",
            "download_dir": "D:/Music/My Downloads"
        }"##;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.download_dir.as_deref(), Some("D:/Music/My Downloads"));
        assert!(
            !s.options.contains_key("download_dir"),
            "download_dir утёк в options"
        );
        assert!(s.options.contains_key("crossfade_on_seek"));
        assert!(s.options.contains_key("static_accent_color"));
    }

    /// Старые настройки без `download_dir` грузятся (serde default → None), и при
    /// сериализации None не пишется на диск (skip_serializing_if).
    #[test]
    fn download_dir_defaults_to_none_and_skips_when_empty() {
        let s: AppSettings = serde_json::from_str(r#"{"show_github_badge": false}"#).unwrap();
        assert_eq!(s.download_dir, None);
        let out = serde_json::to_string(&s).unwrap();
        assert!(
            !out.contains("download_dir"),
            "пустой download_dir не должен сериализоваться"
        );
    }
}
