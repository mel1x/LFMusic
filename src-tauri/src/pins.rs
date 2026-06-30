use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Одно закрепление: тип + ключ + отображаемое имя.
/// kind: "catalog" | "playlist" | "album" | "artist"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinEntry {
    pub kind: String,
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PinsState {
    pub pins: Vec<PinEntry>,
}

pub fn load(app_data_dir: &PathBuf) -> PinsState {
    let path = app_data_dir.join("pins.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(app_data_dir: &PathBuf, state: &PinsState) -> Result<(), String> {
    let path = app_data_dir.join("pins.json");
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
        std::env::temp_dir().join(format!("lfmusic_pins_{test_name}_{unique}"))
    }

    #[test]
    fn load_missing_file_returns_empty_default() {
        let dir = temp_dir("missing");
        let state = load(&dir);
        assert!(state.pins.is_empty());
    }

    #[test]
    fn save_then_load_round_trips_pins() {
        let dir = temp_dir("roundtrip");
        std::fs::create_dir_all(&dir).unwrap();

        let state = PinsState {
            pins: vec![
                PinEntry {
                    kind: "playlist".to_string(),
                    key: "3".to_string(),
                    name: "My Mix".to_string(),
                },
                PinEntry {
                    kind: "artist".to_string(),
                    key: "Queen".to_string(),
                    name: "Queen".to_string(),
                },
            ],
        };
        save(&dir, &state).expect("save pins");

        let loaded = load(&dir);
        assert_eq!(loaded.pins.len(), 2);
        assert_eq!(loaded.pins[0].kind, "playlist");
        assert_eq!(loaded.pins[1].key, "Queen");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_corrupted_json_falls_back_to_default_instead_of_panicking() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pins.json"), b"{not valid json").unwrap();

        let state = load(&dir);
        assert!(state.pins.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
