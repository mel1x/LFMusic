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
