use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::json_store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomCoverEntry {
    pub kind: String,
    pub key: String,
    pub data_url: String,
}

fn covers_dir(app_data_dir: &Path) -> PathBuf {
    let dir = app_data_dir.join("custom_covers");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn manifest_path(app_data_dir: &Path) -> PathBuf {
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

fn load_manifest(app_data_dir: &Path) -> HashMap<String, String> {
    json_store::load_or_default(&manifest_path(app_data_dir))
}

fn save_manifest(app_data_dir: &Path, manifest: &HashMap<String, String>) -> Result<(), String> {
    json_store::save_pretty(&manifest_path(app_data_dir), manifest)
}

fn read_as_data_url(file_path: &Path) -> Option<String> {
    let data = fs::read(file_path).ok()?;
    let ext = file_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("img");
    let mime = mime_for_ext(ext);
    let b64 = STANDARD.encode(&data);
    Some(format!("data:{};base64,{}", mime, b64))
}

pub fn set_custom_cover(
    app_data_dir: &Path,
    kind: &str,
    key: &str,
    source_path: &str,
) -> Result<String, String> {
    // Read the source first so a failed read leaves the current cover intact.
    let data = fs::read(source_path).map_err(|e| format!("Не удалось прочитать файл: {}", e))?;

    let dir = covers_dir(app_data_dir);
    let mut manifest = load_manifest(app_data_dir);
    let id = entry_id(kind, key);
    let ext = detect_ext_from_path(source_path);
    let filename = format!("{:016x}.{}", key_hash(kind, key), ext);
    let target = dir.join(&filename);

    // Remove previous file (if any, with another extension) to avoid orphans.
    if let Some(prev) = manifest.get(&id).filter(|prev| **prev != filename) {
        let _ = fs::remove_file(dir.join(prev));
    }

    fs::write(&target, &data).map_err(|e| format!("Не удалось сохранить обложку: {}", e))?;

    manifest.insert(id, filename);
    save_manifest(app_data_dir, &manifest)?;

    let mime = mime_for_ext(ext);
    Ok(format!("data:{};base64,{}", mime, STANDARD.encode(&data)))
}

pub fn remove_custom_cover(app_data_dir: &Path, kind: &str, key: &str) -> Result<(), String> {
    let dir = covers_dir(app_data_dir);
    let mut manifest = load_manifest(app_data_dir);
    let id = entry_id(kind, key);
    if let Some(filename) = manifest.remove(&id) {
        let _ = fs::remove_file(dir.join(filename));
        save_manifest(app_data_dir, &manifest)?;
    }
    Ok(())
}

pub fn load_all(app_data_dir: &Path) -> Vec<CustomCoverEntry> {
    let dir = covers_dir(app_data_dir);
    let manifest = load_manifest(app_data_dir);
    let mut out = Vec::with_capacity(manifest.len());
    for (id, filename) in manifest.iter() {
        let Some((kind, key)) = id.split_once(':') else {
            continue;
        };
        if let Some(data_url) = read_as_data_url(&dir.join(filename)) {
            out.push(CustomCoverEntry {
                kind: kind.to_string(),
                key: key.to_string(),
                data_url,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(test_name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("lfmusic_covers_{test_name}_{unique}"))
    }

    #[test]
    fn detect_ext_from_path_recognizes_known_extensions_case_insensitively() {
        assert_eq!(detect_ext_from_path("cover.JPG"), "jpg");
        assert_eq!(detect_ext_from_path("cover.jpeg"), "jpg");
        assert_eq!(detect_ext_from_path("cover.PNG"), "png");
        assert_eq!(detect_ext_from_path("cover.webp"), "webp");
        assert_eq!(detect_ext_from_path("cover.gif"), "gif");
        assert_eq!(detect_ext_from_path("cover.bmp"), "bmp");
        assert_eq!(detect_ext_from_path("cover.tiff"), "img");
        assert_eq!(detect_ext_from_path("no_extension"), "img");
    }

    #[test]
    fn mime_for_ext_covers_all_detected_extensions() {
        assert_eq!(mime_for_ext("jpg"), "image/jpeg");
        assert_eq!(mime_for_ext("jpeg"), "image/jpeg");
        assert_eq!(mime_for_ext("png"), "image/png");
        assert_eq!(mime_for_ext("webp"), "image/webp");
        assert_eq!(mime_for_ext("gif"), "image/gif");
        assert_eq!(mime_for_ext("bmp"), "image/bmp");
        assert_eq!(mime_for_ext("img"), "application/octet-stream");
    }

    #[test]
    fn entry_id_combines_kind_and_key_with_colon_separator() {
        assert_eq!(entry_id("playlist", "42"), "playlist:42");
        assert_eq!(entry_id("artist", "Queen"), "artist:Queen");
    }

    /// `entry_id` doesn't escape `:` in kind/key, so different kind+key pairs
    /// could theoretically collide (e.g. kind="a", key="b:c" vs kind="a:b",
    /// key="c"). Not exploitable in practice: `kind` is always one of 4 fixed
    /// literals, never user input. Documents the intentional limitation.
    #[test]
    fn entry_id_is_not_collision_proof_for_arbitrary_kind_values() {
        assert_eq!(entry_id("a", "b:c"), entry_id("a:b", "c"));
    }

    #[test]
    fn entry_id_distinguishes_real_kinds_for_the_same_key_containing_colon() {
        let key_with_colon = "Greatest Hits: Volume 1";
        assert_ne!(
            entry_id("artist", key_with_colon),
            entry_id("album", key_with_colon)
        );
    }

    #[test]
    fn set_custom_cover_writes_file_and_returns_matching_data_url() {
        let dir = temp_dir("set_cover");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.png");
        let bytes = vec![137u8, 80, 78, 71, 1, 2, 3];
        std::fs::write(&source, &bytes).unwrap();

        let data_url =
            set_custom_cover(&dir, "playlist", "7", source.to_str().unwrap()).expect("set cover");
        assert!(data_url.starts_with("data:image/png;base64,"));

        let all = load_all(&dir);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].kind, "playlist");
        assert_eq!(all[0].key, "7");
        assert_eq!(all[0].data_url, data_url);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_custom_cover_replaces_previous_file_for_same_id() {
        let dir = temp_dir("replace_cover");
        std::fs::create_dir_all(&dir).unwrap();
        let source1 = dir.join("s1.png");
        std::fs::write(&source1, vec![1u8, 2, 3]).unwrap();
        let source2 = dir.join("s2.png");
        std::fs::write(&source2, vec![4u8, 5, 6, 7]).unwrap();

        set_custom_cover(&dir, "album", "X", source1.to_str().unwrap()).unwrap();
        set_custom_cover(&dir, "album", "X", source2.to_str().unwrap()).unwrap();

        let all = load_all(&dir);
        assert_eq!(all.len(), 1, "повторная установка должна заменить, а не дублировать");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_custom_cover_deletes_entry_and_is_idempotent() {
        let dir = temp_dir("remove_cover");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("s.png");
        std::fs::write(&source, vec![1u8, 2]).unwrap();
        set_custom_cover(&dir, "artist", "Queen", source.to_str().unwrap()).unwrap();
        assert_eq!(load_all(&dir).len(), 1);

        remove_custom_cover(&dir, "artist", "Queen").expect("remove cover");
        assert_eq!(load_all(&dir).len(), 0);

        remove_custom_cover(&dir, "artist", "Queen").expect("idempotent remove");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_all_on_empty_app_data_dir_is_empty() {
        let dir = temp_dir("empty");
        assert!(load_all(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
