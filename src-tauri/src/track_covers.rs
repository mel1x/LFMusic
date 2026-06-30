//! Файловое хранилище пользовательских (override) обложек треков.
//!
//! Раньше байты кастомной обложки лежали прямо в SQLite (`track_overrides.cover_data`,
//! base64 data-URL). При каждом запросе списка треков бэкенд тянул эти мегабайты в
//! память лишь ради вычисления хэш-сигнатуры. Теперь байты хранятся файлами в
//! `<app_data_dir>/track_covers/<sig>`, а в БД остаётся только короткая сигнатура
//! (`cover_sig`). Это резко уменьшает размер БД и ускоряет загрузку списков —
//! тяжёлые данные читаются лениво только когда реально нужны (генерация миниатюры,
//! расчёт акцентного цвета).

use std::fs;
use std::path::{Path, PathBuf};

/// Папка хранения файлов обложек (создаётся при первом обращении).
pub fn covers_dir(app_data_dir: &Path) -> PathBuf {
    let dir = app_data_dir.join("track_covers");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Оставить в имени файла только безопасные символы (сигнатура и так
/// ASCII-hex, но нормализуем на всякий случай).
fn sanitize(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Путь к файлу обложки по её сигнатуре.
pub fn cover_path(app_data_dir: &Path, sig: &str) -> PathBuf {
    covers_dir(app_data_dir).join(sanitize(sig))
}

/// Прочитать байты сохранённой обложки.
pub fn read_cover(app_data_dir: &Path, sig: &str) -> Option<Vec<u8>> {
    fs::read(cover_path(app_data_dir, sig)).ok()
}

/// Записать байты обложки под именем её сигнатуры.
pub fn write_cover(app_data_dir: &Path, sig: &str, bytes: &[u8]) -> Result<(), String> {
    fs::write(cover_path(app_data_dir, sig), bytes)
        .map_err(|e| format!("Не удалось сохранить обложку: {}", e))
}

/// Удалить файл обложки (молча игнорирует отсутствие файла).
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
        // Сигнатура с компонентами пути не должна выходить за пределы covers_dir.
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

        // Повторное удаление несуществующего файла не должно паниковать.
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
