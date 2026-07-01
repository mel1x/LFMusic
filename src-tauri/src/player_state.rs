use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPlayerState {
    pub current_track_path: Option<String>,
    pub position_secs: f64,
    /// 0.0 - 1.0
    pub volume: f32,
    pub shuffled: bool,
    /// "queue" or "track"
    pub repeat_mode: String,
    /// e.g. catalog, catalog:genre:Rock, playlist:3, ...
    pub active_playlist_source: Option<String>,
    pub current_track_index: Option<usize>,
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

pub fn load(app_data_dir: &PathBuf) -> SavedPlayerState {
    let path = app_data_dir.join("player_state.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(app_data_dir: &PathBuf, state: &SavedPlayerState) -> Result<(), String> {
    let path = app_data_dir.join("player_state.json");
    let content = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(test_name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("lfmusic_player_state_{test_name}_{unique}"))
    }

    #[test]
    fn default_state_has_sane_values() {
        let s = SavedPlayerState::default();
        assert_eq!(s.current_track_path, None);
        assert_eq!(s.position_secs, 0.0);
        assert_eq!(s.volume, 1.0);
        assert!(!s.shuffled);
        assert_eq!(s.repeat_mode, "queue");
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = temp_dir("missing");
        let s = load(&dir);
        assert_eq!(s.volume, 1.0);
        assert_eq!(s.repeat_mode, "queue");
    }

    #[test]
    fn save_then_load_round_trips_state() {
        let dir = temp_dir("roundtrip");
        std::fs::create_dir_all(&dir).unwrap();

        let original = SavedPlayerState {
            current_track_path: Some("/music/track.flac".to_string()),
            position_secs: 42.5,
            volume: 0.75,
            shuffled: true,
            repeat_mode: "track".to_string(),
            active_playlist_source: Some("album:Greatest Hits".to_string()),
            current_track_index: Some(3),
            current_route: Some("/catalog".to_string()),
        };
        save(&dir, &original).expect("save player state");

        let loaded = load(&dir);
        assert_eq!(loaded.current_track_path, original.current_track_path);
        assert_eq!(loaded.position_secs, original.position_secs);
        assert_eq!(loaded.volume, original.volume);
        assert_eq!(loaded.shuffled, original.shuffled);
        assert_eq!(loaded.repeat_mode, original.repeat_mode);
        assert_eq!(loaded.active_playlist_source, original.active_playlist_source);
        assert_eq!(loaded.current_track_index, original.current_track_index);
        assert_eq!(loaded.current_route, original.current_route);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_corrupted_json_falls_back_to_default() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("player_state.json"), b"not json at all").unwrap();

        let s = load(&dir);
        assert_eq!(s.volume, 1.0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
