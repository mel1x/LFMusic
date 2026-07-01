//! File-based storage for per-track override cover images.
//!
//! Previously cover bytes lived in SQLite (`track_overrides.cover_data`, base64),
//! forcing every track-list query to pull megabytes into memory just to hash a
//! signature. Now bytes live at `<app_data_dir>/track_covers/<sig>` and only the
//! short signature (`cover_sig`) stays in the DB — heavy data is read lazily
//! only when actually needed (thumbnail generation, accent color).

use std::fs;
use std::path::{Path, PathBuf};

pub fn covers_dir(app_data_dir: &Path) -> PathBuf {
    let dir = app_data_dir.join("track_covers");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

pub fn cover_path(app_data_dir: &Path, sig: &str) -> PathBuf {
    covers_dir(app_data_dir).join(sanitize(sig))
}

pub fn read_cover(app_data_dir: &Path, sig: &str) -> Option<Vec<u8>> {
    fs::read(cover_path(app_data_dir, sig)).ok()
}

pub fn write_cover(app_data_dir: &Path, sig: &str, bytes: &[u8]) -> Result<(), String> {
    fs::write(cover_path(app_data_dir, sig), bytes)
        .map_err(|e| format!("Не удалось сохранить обложку: {}", e))
}

/// Silently ignores a missing file.
pub fn remove_cover(app_data_dir: &Path, sig: &str) {
    let _ = fs::remove_file(cover_path(app_data_dir, sig));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(test_name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("lfmusic_track_covers_{test_name}_{unique}"))
    }

    #[test]
    fn sanitize_strips_non_alphanumeric_characters() {
        assert_eq!(sanitize("abc123"), "abc123");
        assert_eq!(sanitize("a/b\\c:d"), "abcd");
        assert_eq!(sanitize("o0123456789abcdef"), "o0123456789abcdef");
    }

    #[test]
    fn cover_path_sanitizes_signature_for_traversal_safety() {
        let dir = temp_dir("traversal");
        let path = cover_path(&dir, "../../evil");
        assert_eq!(path, covers_dir(&dir).join("evil"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_then_read_cover_round_trips_bytes() {
        let dir = temp_dir("roundtrip");
        let bytes = vec![1u8, 2, 3, 4, 250, 251];
        write_cover(&dir, "osig123", &bytes).expect("write cover");
        let read = read_cover(&dir, "osig123").expect("cover should exist");
        assert_eq!(read, bytes);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_missing_cover_returns_none() {
        let dir = temp_dir("missing");
        assert_eq!(read_cover(&dir, "nope"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_cover_deletes_file_and_is_idempotent() {
        let dir = temp_dir("remove");
        write_cover(&dir, "sig1", &[9u8, 8, 7]).expect("write cover");
        assert!(read_cover(&dir, "sig1").is_some());

        remove_cover(&dir, "sig1");
        assert!(read_cover(&dir, "sig1").is_none());

        remove_cover(&dir, "sig1");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn different_signatures_do_not_collide() {
        let dir = temp_dir("distinct");
        write_cover(&dir, "sigA", &[1u8]).unwrap();
        write_cover(&dir, "sigB", &[2u8]).unwrap();
        assert_eq!(read_cover(&dir, "sigA"), Some(vec![1u8]));
        assert_eq!(read_cover(&dir, "sigB"), Some(vec![2u8]));
        let _ = fs::remove_dir_all(&dir);
    }
}
