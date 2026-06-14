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
