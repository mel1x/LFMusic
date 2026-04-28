use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// Описание одной кастомной обложки на фронте: уникальный ключ и data-URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomCoverEntry {
    pub kind: String,
    pub key: String,
    pub data_url: String,
}

fn covers_dir(app_data_dir: &PathBuf) -> PathBuf {
    let dir = app_data_dir.join("custom_covers");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn manifest_path(app_data_dir: &PathBuf) -> PathBuf {
    app_data_dir.join("custom_covers.json")
}

fn entry_id(kind: &str, key: &str) -> String {
    format!("{}:{}", kind, key)
}

fn key_hash(kind: &str, key: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut h);
    "::".hash(&mut h);
    key.hash(&mut h);
    h.finish()
}

fn detect_ext_from_path(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".jpeg") || lower.ends_with(".jpg") {
        "jpg"
    } else if lower.ends_with(".png") {
        "png"
    } else if lower.ends_with(".webp") {
        "webp"
    } else if lower.ends_with(".gif") {
        "gif"
    } else if lower.ends_with(".bmp") {
        "bmp"
    } else {
        "img"
    }
}

fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

fn load_manifest(app_data_dir: &PathBuf) -> HashMap<String, String> {
    fs::read_to_string(manifest_path(app_data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_manifest(app_data_dir: &PathBuf, manifest: &HashMap<String, String>) -> Result<(), String> {
    let content = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    fs::write(manifest_path(app_data_dir), content).map_err(|e| e.to_string())
}

fn read_as_data_url(file_path: &PathBuf) -> Option<String> {
    let data = fs::read(file_path).ok()?;
    let ext = file_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("img");
    let mime = mime_for_ext(ext);
    let b64 = STANDARD.encode(&data);
    Some(format!("data:{};base64,{}", mime, b64))
}

/// Сохранить кастомную обложку из файла на диске пользователя.
/// Возвращает data-URL новой обложки.
pub fn set_custom_cover(
    app_data_dir: &PathBuf,
    kind: &str,
    key: &str,
    source_path: &str,
) -> Result<String, String> {
    let dir = covers_dir(app_data_dir);
    let mut manifest = load_manifest(app_data_dir);
    let id = entry_id(kind, key);

    // Удаляем предыдущий файл (если был), чтобы не оставались сироты.
    if let Some(prev) = manifest.get(&id) {
        let _ = fs::remove_file(dir.join(prev));
    }

    let ext = detect_ext_from_path(source_path);
    let filename = format!("{:016x}.{}", key_hash(kind, key), ext);
    let target = dir.join(&filename);

    let data = fs::read(source_path).map_err(|e| format!("Не удалось прочитать файл: {}", e))?;
    fs::write(&target, &data).map_err(|e| format!("Не удалось сохранить обложку: {}", e))?;

    manifest.insert(id, filename);
    save_manifest(app_data_dir, &manifest)?;

    let mime = mime_for_ext(ext);
    Ok(format!("data:{};base64,{}", mime, STANDARD.encode(&data)))
}

pub fn remove_custom_cover(app_data_dir: &PathBuf, kind: &str, key: &str) -> Result<(), String> {
    let dir = covers_dir(app_data_dir);
    let mut manifest = load_manifest(app_data_dir);
    let id = entry_id(kind, key);
    if let Some(filename) = manifest.remove(&id) {
        let _ = fs::remove_file(dir.join(filename));
        save_manifest(app_data_dir, &manifest)?;
    }
    Ok(())
}

/// Получить все кастомные обложки в виде data-URL, чтобы фронт мог
/// быстро ими отрисоваться без последующих запросов.
pub fn load_all(app_data_dir: &PathBuf) -> Vec<CustomCoverEntry> {
    let dir = covers_dir(app_data_dir);
    let manifest = load_manifest(app_data_dir);
    let mut out = Vec::with_capacity(manifest.len());
    for (id, filename) in manifest.iter() {
        let parts: Vec<&str> = id.splitn(2, ':').collect();
        if parts.len() != 2 {
            continue;
        }
        let path = dir.join(filename);
        if let Some(data_url) = read_as_data_url(&path) {
            out.push(CustomCoverEntry {
                kind: parts[0].to_string(),
                key: parts[1].to_string(),
                data_url,
            });
        }
    }
    out
}
