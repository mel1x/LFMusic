//! Shared helpers for small JSON files in the app data dir (settings, pins,
//! player state, cover manifests).

use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::path::Path;

/// Reads and parses `path`; a missing or broken file yields `T::default()`.
pub fn load_or_default<T: DeserializeOwned + Default>(path: &Path) -> T {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Pretty-prints `value` into `path` atomically (temp -> rename), so a crash
/// or a concurrent reader never sees a partial file.
pub fn save_pretty<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let content = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    write_atomic(path, content.as_bytes())
}

pub fn write_atomic(target: &Path, data: &[u8]) -> Result<(), String> {
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    fs::write(&tmp, data).map_err(|e| e.to_string())?;
    fs::rename(&tmp, target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}
