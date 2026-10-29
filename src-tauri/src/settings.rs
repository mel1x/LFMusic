use std::collections::HashMap;
use std::path::Path;

use tauri::State;

use crate::audio_player::EqSettings;
use crate::enhance::EnhanceSettings;
use crate::json_store;
use crate::SettingsState;

pub const DEFAULT_ARTIST_SEPARATORS: &[&str] = &["; ", ", "];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct AppSettings {
    /// Free-form frontend options (bool/string/...): crossfade_on_seek,
    /// show_github_badge, static_accent_color, etc.
    #[serde(flatten)]
    pub options: HashMap<String, serde_json::Value>,

    #[serde(default)]
    pub eq_settings: EqSettings,

    /// Auto sound enhancement (device profile, normalization).
    #[serde(default)]
    pub enhance_settings: EnhanceSettings,

    /// Download folder for external sources; empty/None uses the default
    /// "LFMusic Downloads" under the system music dir. Kept as its own field
    /// (not inside `options`) so `save_settings`, which replaces `options`
    /// wholesale from the frontend, doesn't wipe it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_dir: Option<String>,

    /// Legacy proxy fields from the previous downloader; read once to seed
    /// `downloader` and then dropped on the next save.
    #[serde(default, skip_serializing)]
    pub proxy_mode: Option<String>,
    #[serde(default, skip_serializing)]
    pub proxy_url: Option<String>,

    /// Track downloader options. Own field for the same reason as `download_dir`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloader: Option<DownloaderSettings>,
}

/// Network/auth options for yt-dlp, edited on the "External sources" page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DownloaderSettings {
    /// "system" (yt-dlp picks up system/env proxy) | "none" | "custom".
    pub proxy_mode: String,
    /// Used for "custom", e.g. `socks5://127.0.0.1:1080`.
    pub proxy_url: String,
    /// "none" | "browser" (`--cookies-from-browser`) | "file" (`--cookies`).
    pub cookies_mode: String,
    /// yt-dlp browser name: firefox, chrome, edge, brave, opera, vivaldi, chromium, safari.
    pub cookies_browser: String,
    /// Netscape-format cookies.txt path.
    pub cookies_file: String,
    /// Keep the app-managed yt-dlp/Deno current (checked at most once a day,
    /// right before a download batch starts).
    pub auto_update: bool,
}

impl Default for DownloaderSettings {
    fn default() -> Self {
        Self {
            proxy_mode: "system".into(),
            proxy_url: String::new(),
            cookies_mode: "none".into(),
            cookies_browser: "firefox".into(),
            cookies_file: String::new(),
            auto_update: true,
        }
    }
}

impl AppSettings {
    /// Downloader options, seeded from the legacy proxy fields when the new
    /// block has never been saved.
    pub fn downloader(&self) -> DownloaderSettings {
        if let Some(d) = &self.downloader {
            return d.clone();
        }
        let mut d = DownloaderSettings::default();
        if let Some(mode) = self.proxy_mode.as_deref().filter(|m| !m.is_empty()) {
            d.proxy_mode = mode.to_string();
        }
        if let Some(url) = &self.proxy_url {
            d.proxy_url = url.clone();
        }
        d
    }
}

pub fn load(app_data_dir: &Path) -> AppSettings {
    let mut settings: AppSettings = json_store::load_or_default(&app_data_dir.join("settings.json"));
    // Legacy proxy fields aren't written back, so fold them into the
    // downloader block now or they'd be lost on the next save.
    if settings.downloader.is_none() && settings.proxy_mode.is_some() {
        settings.downloader = Some(settings.downloader());
    }
    settings
}

pub fn save(app_data_dir: &Path, settings: &AppSettings) -> Result<(), String> {
    json_store::save_pretty(&app_data_dir.join("settings.json"), settings)
}

pub fn option_bool(settings: &AppSettings, key: &str, default: bool) -> bool {
    settings
        .options
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

/// Reads artist separators from settings, falling back to the default if
/// missing or malformed.
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

    /// `download_dir` must not leak into `options` on deserialization, or
    /// the frontend's full-`options` replacement in `save_settings` would wipe it.
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

    #[test]
    fn proxy_fields_round_trip_and_are_absent_when_none() {
        let s: AppSettings = serde_json::from_str(
            r#"{"proxy_mode": "custom", "proxy_url": "socks5://127.0.0.1:1080"}"#,
        )
        .unwrap();
        assert_eq!(s.proxy_mode.as_deref(), Some("custom"));
        assert_eq!(s.proxy_url.as_deref(), Some("socks5://127.0.0.1:1080"));
        assert!(!s.options.contains_key("proxy_mode"));
        assert!(!s.options.contains_key("proxy_url"));

        let empty: AppSettings = serde_json::from_str("{}").unwrap();
        let out = serde_json::to_string(&empty).unwrap();
        assert!(!out.contains("proxy_mode"));
        assert!(!out.contains("proxy_url"));
    }

    #[test]
    fn eq_settings_defaults_when_absent_from_json() {
        let s: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(!s.eq_settings.enabled);
        assert_eq!(s.eq_settings.bands.len(), 10);
        assert!(s.eq_settings.bands.iter().all(|&b| b == 0.0));
    }

    #[test]
    fn option_bool_reads_present_value() {
        let mut s = AppSettings::default();
        s.options
            .insert("crossfade_on_seek".to_string(), serde_json::Value::Bool(true));
        assert!(option_bool(&s, "crossfade_on_seek", false));
    }

    #[test]
    fn option_bool_falls_back_to_default_when_missing_or_wrong_type() {
        let mut s = AppSettings::default();
        assert!(!option_bool(&s, "missing_key", false));
        assert!(option_bool(&s, "missing_key", true));

        // Wrong type (string instead of bool) treated as absent -> default.
        s.options.insert(
            "static_accent_color".to_string(),
            serde_json::Value::String("#3D3D3D".to_string()),
        );
        assert!(!option_bool(&s, "static_accent_color", false));
        assert!(option_bool(&s, "static_accent_color", true));
    }

    #[test]
    fn load_missing_file_returns_default_settings() {
        let dir = std::env::temp_dir().join(format!(
            "lfmusic_settings_missing_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // Directory intentionally not created — settings.json can't exist there.
        let s = load(&dir);
        assert!(s.options.is_empty());
        assert_eq!(s.download_dir, None);
    }

    #[test]
    fn save_then_load_round_trips_settings() {
        let dir = std::env::temp_dir().join(format!(
            "lfmusic_settings_roundtrip_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let mut original = AppSettings::default();
        original
            .options
            .insert("show_github_badge".to_string(), serde_json::Value::Bool(false));
        original.download_dir = Some("D:/Music/Downloads".to_string());
        original.downloader = Some(DownloaderSettings {
            proxy_mode: "none".into(),
            ..Default::default()
        });

        save(&dir, &original).expect("save settings");
        let loaded = load(&dir);

        assert_eq!(loaded.download_dir, original.download_dir);
        assert_eq!(loaded.downloader().proxy_mode, "none");
        assert_eq!(
            loaded.options.get("show_github_badge"),
            Some(&serde_json::Value::Bool(false))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_proxy_fields_migrate_into_downloader_block() {
        let dir = std::env::temp_dir().join(format!(
            "lfmusic_settings_legacy_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        std::fs::write(
            dir.join("settings.json"),
            r#"{"proxy_mode":"custom","proxy_url":"socks5://127.0.0.1:1080"}"#,
        )
        .expect("write legacy settings");

        let loaded = load(&dir);
        let d = loaded.downloader();
        assert_eq!(d.proxy_mode, "custom");
        assert_eq!(d.proxy_url, "socks5://127.0.0.1:1080");
        assert!(d.auto_update);

        // Survives a save/load cycle even though the legacy fields aren't written back.
        save(&dir, &loaded).expect("save");
        assert_eq!(load(&dir).downloader().proxy_url, "socks5://127.0.0.1:1080");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
