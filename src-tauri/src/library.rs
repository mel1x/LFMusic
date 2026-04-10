use crate::audio_player::{AudioPlayer, TrackMetadata};
use rayon::prelude::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

// ──────────────────────────────────────────────────────────────────────────────
// Публичные типы
// ──────────────────────────────────────────────────────────────────────────────

/// Информация о папке для отображения во Vue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderInfo {
    pub path: String,
    pub track_count: usize,
    pub exists: bool,
}

/// Тип плейлиста
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PlaylistKind {
    Album,
    User,
}

/// Информация о плейлисте для фронтенда
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistInfo {
    pub id: i64,
    pub name: String,
    pub kind: PlaylistKind,
    pub track_count: usize,
}

// ──────────────────────────────────────────────────────────────────────────────
// MusicLibrary
// ──────────────────────────────────────────────────────────────────────────────

pub struct MusicLibrary {
    conn: Connection,
}

impl MusicLibrary {
    pub fn new(app_data_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&app_data_dir)
            .map_err(|e| format!("Не удалось создать директорию данных: {}", e))?;

        let db_path = app_data_dir.join("library.db");
        let conn = Connection::open(&db_path)
            .map_err(|e| format!("Не удалось открыть БД: {}", e))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| format!("Ошибка PRAGMA: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS folders (
                 path TEXT PRIMARY KEY NOT NULL
             );
             CREATE TABLE IF NOT EXISTS tracks (
                 file_path     TEXT PRIMARY KEY NOT NULL,
                 folder_path   TEXT NOT NULL,
                 title         TEXT,
                 artist        TEXT,
                 album         TEXT,
                 genre         TEXT,
                 duration_secs REAL,
                 format        TEXT NOT NULL,
                 sample_rate   INTEGER,
                 bit_depth     INTEGER,
                 channels      INTEGER,
                 bitrate       INTEGER,
                 is_lossless   INTEGER NOT NULL DEFAULT 0,
                 modified_secs INTEGER NOT NULL DEFAULT 0,
                 sort_order    INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_tracks_folder ON tracks(folder_path);
             CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
             CREATE INDEX IF NOT EXISTS idx_tracks_album  ON tracks(album);

             CREATE TABLE IF NOT EXISTS playlists (
                 id   INTEGER PRIMARY KEY AUTOINCREMENT,
                 name TEXT NOT NULL,
                 kind TEXT NOT NULL DEFAULT 'user'
             );
             CREATE TABLE IF NOT EXISTS playlist_tracks (
                 playlist_id INTEGER NOT NULL,
                 file_path   TEXT NOT NULL,
                 sort_order  INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (playlist_id, file_path),
                 FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
                 FOREIGN KEY (file_path) REFERENCES tracks(file_path) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_pt_playlist ON playlist_tracks(playlist_id);",
        )
        .map_err(|e| format!("Ошибка создания таблиц: {}", e))?;

        // Миграция: добавляем sort_order если таблица уже существовала без него
        let _ = conn.execute_batch(
            "ALTER TABLE tracks ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;"
        );

        // Миграция: добавляем genre если таблица уже существовала без него
        let _ = conn.execute_batch(
            "ALTER TABLE tracks ADD COLUMN genre TEXT;"
        );
        // Сбрасываем modified_secs для треков без жанра, чтобы пересканировать
        let _ = conn.execute_batch(
            "UPDATE tracks SET modified_secs = 0 WHERE genre IS NULL;"
        );

        // Миграция: добавляем accent_color для кэширования
        let _ = conn.execute_batch(
            "ALTER TABLE tracks ADD COLUMN accent_color TEXT;"
        );

        Ok(Self { conn })
    }

    // ── Вспомогательные ────────────────────────────────────────────────────

    fn get_mtime(path: &std::path::Path) -> u64 {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .map(|t| {
                t.duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            })
            .unwrap_or(0)
    }

    fn row_to_metadata(row: &rusqlite::Row) -> rusqlite::Result<TrackMetadata> {
        Ok(TrackMetadata {
            file_path: row.get(0)?,
            title: row.get(1)?,
            artist: row.get(2)?,
            album: row.get(3)?,
            genre: row.get(4)?,
            duration_secs: row.get(5)?,
            format: row.get(6)?,
            sample_rate: row.get(7)?,
            bit_depth: row.get::<_, Option<i32>>(8)?.map(|v| v as u8),
            channels: row.get::<_, Option<i32>>(9)?.map(|v| v as u8),
            bitrate: row.get(10)?,
            is_lossless: row.get::<_, i32>(11)? != 0,
            cover_art: None,
            accent_color: row.get(12)?,
        })
    }

    /// Быстрый первый проход: собирает пути всех аудиофайлов без чтения метаданных
    fn collect_audio_files(
        dir: &std::path::Path,
        supported: &HashSet<&str>,
        depth: usize,
        result: &mut Vec<PathBuf>,
    ) {
        if depth > 8 {
            return;
        }

        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            let path = entry.path();

            if ft.is_dir() {
                if entry
                    .file_name()
                    .to_str()
                    .map(|n| n.starts_with('.'))
                    .unwrap_or(false)
                {
                    continue;
                }
                Self::collect_audio_files(&path, supported, depth + 1, result);
                continue;
            }

            if !ft.is_file() {
                continue;
            }

            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            if supported.contains(ext.as_str()) {
                result.push(path);
            }
        }
    }

    /// Сканирует папку. `progress(current, total)` вызывается для каждого обработанного файла.
    fn scan_folder_internal<F>(
        &mut self,
        folder: &str,
        progress: F,
    ) -> Result<(), String>
    where
        F: FnMut(usize, usize),
    {
        let path = PathBuf::from(folder);
        if !path.is_dir() {
            return Err(format!("Не является директорией: {}", folder));
        }

        let supported: HashSet<&str> = AudioPlayer::supported_formats().iter().copied().collect();

        let mut all_files: Vec<PathBuf> = Vec::new();
        Self::collect_audio_files(&path, &supported, 0, &mut all_files);

        self.scan_folder_files(folder, &all_files, progress)
    }

    // ── Публичный API ───────────────────────────────────────────────────────

    pub fn get_folders(&self) -> Vec<FolderInfo> {
        self.get_folders_inner(true)
    }

    /// Быстрый вариант без проверки файловой системы (для частых вызовов)
    pub fn get_folders_fast(&self) -> Vec<FolderInfo> {
        self.get_folders_inner(false)
    }

    fn get_folders_inner(&self, check_exists: bool) -> Vec<FolderInfo> {
        let mut stmt = match self.conn.prepare(
            "SELECT f.path, COUNT(t.file_path) as cnt
             FROM folders f
             LEFT JOIN tracks t ON t.folder_path = f.path
             GROUP BY f.path",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map([], |row| {
            let path: String = row.get(0)?;
            let track_count: usize = row.get(1)?;
            let exists = if check_exists { PathBuf::from(&path).is_dir() } else { true };
            Ok(FolderInfo {
                path,
                track_count,
                exists,
            })
        }) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn add_folder<F>(
        &mut self,
        folder: String,
        progress: F,
    ) -> Result<Vec<TrackMetadata>, String>
    where
        F: FnMut(usize, usize),
    {
        let path = PathBuf::from(&folder);
        if !path.is_dir() {
            return Err(format!("Папка не существует: {}", folder));
        }

        self.conn
            .execute(
                "INSERT OR IGNORE INTO folders (path) VALUES (?1)",
                params![folder],
            )
            .map_err(|e| format!("Ошибка добавления папки: {}", e))?;

        self.scan_folder_internal(&folder, progress)?;
        self.rebuild_album_playlists()?;
        Ok(self.get_all_tracks())
    }

    pub fn remove_folder(&mut self, folder: String) -> Result<(), String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("Ошибка транзакции: {}", e))?;

        tx.execute(
            "DELETE FROM tracks WHERE folder_path = ?1",
            params![folder],
        )
        .map_err(|e| format!("Ошибка удаления треков: {}", e))?;

        tx.execute("DELETE FROM folders WHERE path = ?1", params![folder])
            .map_err(|e| format!("Ошибка удаления папки: {}", e))?;

        tx.commit()
            .map_err(|e| format!("Ошибка коммита: {}", e))?;

        self.rebuild_album_playlists()?;
        Ok(())
    }

    /// Удаляет из кэша треки, файлы которых больше не существуют на диске
    pub fn cleanup_missing_tracks(&mut self) -> Result<usize, String> {
        let all_paths: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT file_path FROM tracks")
                .map_err(|e| format!("Ошибка: {}", e))?;

            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| format!("Ошибка: {}", e))?;

            rows.filter_map(|r| r.ok()).collect()
        };

        let dead: Vec<String> = all_paths
            .into_iter()
            .filter(|p| !PathBuf::from(p).exists())
            .collect();

        let count = dead.len();
        if !dead.is_empty() {
            let tx = self
                .conn
                .transaction()
                .map_err(|e| format!("Ошибка транзакции: {}", e))?;

            for path in &dead {
                tx.execute("DELETE FROM tracks WHERE file_path = ?1", params![path])
                    .ok();
            }

            tx.commit()
                .map_err(|e| format!("Ошибка коммита: {}", e))?;
        }

        Ok(count)
    }

    pub fn get_all_tracks(&self) -> Vec<TrackMetadata> {
        let mut stmt = match self.conn.prepare(
            "SELECT file_path, title, artist, album, genre, duration_secs,
                    format, sample_rate, bit_depth, channels, bitrate, is_lossless, accent_color
             FROM tracks
             ORDER BY
                 sort_order DESC,
                 CASE WHEN artist IS NULL THEN 1 ELSE 0 END,
                 LOWER(artist),
                 LOWER(COALESCE(album, '')),
                 LOWER(COALESCE(title, file_path))",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map([], |row| Self::row_to_metadata(row)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok()).collect()
    }

    /// Поднимает трек на первое место в каталоге
    pub fn move_track_to_top(&self, file_path: &str) -> Result<(), String> {
        // Находим максимальный sort_order и ставим +1
        let max_order: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(sort_order), 0) FROM tracks", [], |r| r.get(0))
            .map_err(|e| format!("Ошибка чтения sort_order: {}", e))?;

        self.conn
            .execute(
                "UPDATE tracks SET sort_order = ?1 WHERE file_path = ?2",
                params![max_order + 1, file_path],
            )
            .map_err(|e| format!("Ошибка обновления sort_order: {}", e))?;

        Ok(())
    }

    pub fn rescan_all<F>(&mut self, mut progress: F) -> Result<Vec<TrackMetadata>, String>
    where
        F: FnMut(usize, usize),
    {
        let folders: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT path FROM folders")
                .map_err(|e| format!("Ошибка чтения папок: {}", e))?;

            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| format!("Ошибка запроса: {}", e))?;

            rows.filter_map(|r| r.ok()).collect()
        };

        let supported: HashSet<&str> = AudioPlayer::supported_formats().iter().copied().collect();

        // Считаем общее количество файлов по всем папкам
        let mut folder_file_lists: Vec<(String, Vec<PathBuf>)> = Vec::new();
        for folder in &folders {
            if PathBuf::from(folder).is_dir() {
                let mut files = Vec::new();
                Self::collect_audio_files(PathBuf::from(folder).as_path(), &supported, 0, &mut files);
                folder_file_lists.push((folder.clone(), files));
            }
        }
        let grand_total: usize = folder_file_lists.iter().map(|(_, f)| f.len()).sum();
        let mut global_current: usize = 0;

        // Сканируем каждую папку с учётом общего прогресса
        for (folder, files) in &folder_file_lists {
            let folder_total = files.len();
            let offset = global_current;

            self.scan_folder_files(folder, files, |i, _| {
                progress(offset + i, grand_total);
            })?;

            global_current += folder_total;
        }

        // Удаляем треки, файлы которых больше не существуют
        let all_paths: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT file_path FROM tracks")
                .map_err(|e| format!("Ошибка: {}", e))?;

            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| format!("Ошибка: {}", e))?;

            rows.filter_map(|r| r.ok()).collect()
        };

        let dead: Vec<String> = all_paths
            .into_iter()
            .filter(|p| !PathBuf::from(p).exists())
            .collect();

        if !dead.is_empty() {
            let tx = self
                .conn
                .transaction()
                .map_err(|e| format!("Ошибка транзакции: {}", e))?;

            for path in &dead {
                tx.execute("DELETE FROM tracks WHERE file_path = ?1", params![path])
                    .ok();
            }

            tx.commit()
                .map_err(|e| format!("Ошибка коммита: {}", e))?;
        }

        self.rebuild_album_playlists()?;
        Ok(self.get_all_tracks())
    }

    /// Внутренний метод: обрабатывает уже собранный список файлов с прогрессом
    fn scan_folder_files<F>(
        &mut self,
        folder: &str,
        files: &[PathBuf],
        mut progress: F,
    ) -> Result<(), String>
    where
        F: FnMut(usize, usize),
    {
        let total = files.len();
        if total == 0 {
            return Ok(());
        }

        // 1. Загружаем все cached mtimes одним запросом
        let cached_mtimes: HashMap<String, u64> = {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT file_path, modified_secs FROM tracks WHERE folder_path = ?1",
                )
                .map_err(|e| format!("Ошибка запроса: {}", e))?;

            let rows = stmt
                .query_map(params![folder], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
                })
                .map_err(|e| format!("Ошибка запроса: {}", e))?;

            rows.filter_map(|r| r.ok()).collect()
        };

        // 2. Фильтруем файлы, которые нужно (пере)сканировать
        let to_scan: Vec<(String, u64)> = files
            .iter()
            .filter_map(|file_path| {
                let path_str = file_path.to_string_lossy().to_string();
                let mtime = Self::get_mtime(file_path);

                if let Some(&cached) = cached_mtimes.get(&path_str) {
                    if cached == mtime {
                        return None;
                    }
                }

                Some((path_str, mtime))
            })
            .collect();

        // Прогресс: кэшированные файлы считаются уже обработанными
        let cached_count = total - to_scan.len();
        if cached_count > 0 {
            progress(cached_count, total);
        }

        if to_scan.is_empty() {
            progress(total, total);
            return Ok(());
        }

        // 3. Параллельное чтение метаданных (без обложек)
        let counter = AtomicUsize::new(cached_count);
        let batch: Vec<(String, TrackMetadata, u64)> = to_scan
            .par_iter()
            .filter_map(|(path_str, mtime)| {
                let result = AudioPlayer::get_metadata_light(path_str).ok()?;
                let done = counter.fetch_add(1, Ordering::Relaxed) + 1;
                // progress вызывается из разных потоков, но это лишь emit события —
                // пропуск нескольких значений не критичен
                if done % 50 == 0 || done == total {
                    // progress нельзя вызвать из par_iter (не Send),
                    // прогресс обновится после join
                }
                Some((path_str.clone(), result, *mtime))
            })
            .collect();

        progress(total, total);

        // 4. Пакетная вставка в одной транзакции
        if !batch.is_empty() {
            let tx = self
                .conn
                .transaction()
                .map_err(|e| format!("Ошибка транзакции: {}", e))?;

            for (file_path, m, mtime) in &batch {
                tx.execute(
                    "INSERT OR REPLACE INTO tracks
                     (file_path, folder_path, title, artist, album, genre, duration_secs,
                      format, sample_rate, bit_depth, channels, bitrate, is_lossless, modified_secs)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                    params![
                        file_path,
                        folder,
                        m.title,
                        m.artist,
                        m.album,
                        m.genre,
                        m.duration_secs,
                        m.format,
                        m.sample_rate,
                        m.bit_depth.map(|v| v as i32),
                        m.channels.map(|v| v as i32),
                        m.bitrate,
                        m.is_lossless as i32,
                        *mtime as i64,
                    ],
                )
                .map_err(|e| format!("Ошибка вставки: {}", e))?;
            }

            tx.commit()
                .map_err(|e| format!("Ошибка коммита: {}", e))?;
        }

        Ok(())
    }

    // ── Плейлисты ─────────────────────────────────────────────────────────────

    /// Получить все плейлисты с количеством треков
    pub fn get_playlists(&self) -> Vec<PlaylistInfo> {
        let mut stmt = match self.conn.prepare(
            "SELECT p.id, p.name, p.kind, COUNT(pt.file_path) as cnt
             FROM playlists p
             LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
             GROUP BY p.id
             ORDER BY p.kind, LOWER(p.name)",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map([], |row| {
            let kind_str: String = row.get(2)?;
            let kind = if kind_str == "album" { PlaylistKind::Album } else { PlaylistKind::User };
            Ok(PlaylistInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                kind,
                track_count: row.get(3)?,
            })
        }) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok()).collect()
    }

    /// Получить треки плейлиста
    pub fn get_playlist_tracks(&self, playlist_id: i64) -> Vec<TrackMetadata> {
        let mut stmt = match self.conn.prepare(
            "SELECT t.file_path, t.title, t.artist, t.album, t.genre, t.duration_secs,
                    t.format, t.sample_rate, t.bit_depth, t.channels, t.bitrate, t.is_lossless, t.accent_color
             FROM playlist_tracks pt
             JOIN tracks t ON t.file_path = pt.file_path
             WHERE pt.playlist_id = ?1
             ORDER BY pt.sort_order DESC,
                      LOWER(COALESCE(t.title, t.file_path))",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map(params![playlist_id], |row| Self::row_to_metadata(row)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok()).collect()
    }

    /// Создать пользовательский плейлист
    pub fn create_playlist(&mut self, name: String) -> Result<PlaylistInfo, String> {
        self.conn
            .execute(
                "INSERT INTO playlists (name, kind) VALUES (?1, 'user')",
                params![name],
            )
            .map_err(|e| format!("Ошибка создания плейлиста: {}", e))?;

        let id = self.conn.last_insert_rowid();
        Ok(PlaylistInfo {
            id,
            name,
            kind: PlaylistKind::User,
            track_count: 0,
        })
    }

    /// Переименовать плейлист
    pub fn rename_playlist(&self, playlist_id: i64, name: String) -> Result<(), String> {
        let kind: String = self.conn
            .query_row("SELECT kind FROM playlists WHERE id = ?1", params![playlist_id], |r| r.get(0))
            .map_err(|e| format!("Плейлист не найден: {}", e))?;

        if kind != "user" {
            return Err("Нельзя переименовывать системные плейлисты".into());
        }

        self.conn
            .execute(
                "UPDATE playlists SET name = ?1 WHERE id = ?2",
                params![name, playlist_id],
            )
            .map_err(|e| format!("Ошибка переименования: {}", e))?;
        Ok(())
    }

    /// Удалить пользовательский плейлист
    pub fn delete_playlist(&mut self, playlist_id: i64) -> Result<(), String> {
        let kind: String = self.conn
            .query_row("SELECT kind FROM playlists WHERE id = ?1", params![playlist_id], |r| r.get(0))
            .map_err(|e| format!("Плейлист не найден: {}", e))?;

        if kind != "user" {
            return Err("Нельзя удалять системные плейлисты".into());
        }

        let tx = self.conn
            .transaction()
            .map_err(|e| format!("Ошибка транзакции: {}", e))?;

        tx.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", params![playlist_id]).ok();
        tx.execute("DELETE FROM playlists WHERE id = ?1", params![playlist_id]).ok();

        tx.commit().map_err(|e| format!("Ошибка коммита: {}", e))?;
        Ok(())
    }

    /// Добавить трек в пользовательский плейлист
    pub fn add_track_to_playlist(&self, playlist_id: i64, file_path: &str) -> Result<(), String> {
        let kind: String = self.conn
            .query_row("SELECT kind FROM playlists WHERE id = ?1", params![playlist_id], |r| r.get(0))
            .map_err(|e| format!("Плейлист не найден: {}", e))?;

        if kind != "user" {
            return Err("Нельзя добавлять треки в системные плейлисты".into());
        }

        self.conn
            .execute(
                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, file_path, sort_order) VALUES (?1, ?2, 0)",
                params![playlist_id, file_path],
            )
            .map_err(|e| format!("Ошибка добавления трека: {}", e))?;
        Ok(())
    }

    /// Удалить трек из пользовательского плейлиста
    pub fn remove_track_from_playlist(&self, playlist_id: i64, file_path: &str) -> Result<(), String> {
        let kind: String = self.conn
            .query_row("SELECT kind FROM playlists WHERE id = ?1", params![playlist_id], |r| r.get(0))
            .map_err(|e| format!("Плейлист не найден: {}", e))?;

        if kind != "user" {
            return Err("Нельзя удалять треки из системных плейлистов".into());
        }

        self.conn
            .execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND file_path = ?2",
                params![playlist_id, file_path],
            )
            .map_err(|e| format!("Ошибка удаления трека: {}", e))?;
        Ok(())
    }

    /// Поднять трек на первое место в плейлисте
    pub fn move_playlist_track_to_top(&self, playlist_id: i64, file_path: &str) -> Result<(), String> {
        let max_order: i64 = self.conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM playlist_tracks WHERE playlist_id = ?1",
                params![playlist_id],
                |r| r.get(0),
            )
            .map_err(|e| format!("Ошибка чтения sort_order: {}", e))?;

        self.conn
            .execute(
                "UPDATE playlist_tracks SET sort_order = ?1 WHERE playlist_id = ?2 AND file_path = ?3",
                params![max_order + 1, playlist_id, file_path],
            )
            .map_err(|e| format!("Ошибка обновления sort_order: {}", e))?;
        Ok(())
    }

    /// Пересоздать альбомные плейлисты из метаданных треков.
    /// Альбом создаётся если в нём больше 1 трека.
    pub fn rebuild_album_playlists(&mut self) -> Result<(), String> {
        let tx = self.conn
            .transaction()
            .map_err(|e| format!("Ошибка транзакции: {}", e))?;

        // Удаляем все старые альбомные плейлисты и их треки
        tx.execute_batch(
            "DELETE FROM playlist_tracks WHERE playlist_id IN (SELECT id FROM playlists WHERE kind = 'album');
             DELETE FROM playlists WHERE kind = 'album';",
        )
        .map_err(|e| format!("Ошибка очистки альбомов: {}", e))?;

        // Находим альбомы с >1 треком
        {
            let mut stmt = tx.prepare(
                "SELECT album, COUNT(*) as cnt FROM tracks
                 WHERE album IS NOT NULL AND album != ''
                 GROUP BY album HAVING cnt > 1
                 ORDER BY LOWER(album)"
            ).map_err(|e| format!("Ошибка запроса альбомов: {}", e))?;

            let albums: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| format!("Ошибка: {}", e))?
                .filter_map(|r| r.ok())
                .collect();

            for album_name in &albums {
                tx.execute(
                    "INSERT INTO playlists (name, kind) VALUES (?1, 'album')",
                    params![album_name],
                )
                .map_err(|e| format!("Ошибка создания альбома: {}", e))?;

                let playlist_id = tx.last_insert_rowid();

                tx.execute(
                    "INSERT INTO playlist_tracks (playlist_id, file_path, sort_order)
                     SELECT ?1, file_path, 0 FROM tracks WHERE album = ?2",
                    params![playlist_id, album_name],
                )
                .map_err(|e| format!("Ошибка добавления треков альбома: {}", e))?;
            }
        }

        tx.commit().map_err(|e| format!("Ошибка коммита: {}", e))?;
        Ok(())
    }

    /// Получить кэшированный accent_color для трека
    pub fn get_accent_color(&self, file_path: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT accent_color FROM tracks WHERE file_path = ?1",
                params![file_path],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
    }

    /// Сохранить accent_color для трека в кэш
    pub fn set_accent_color(&self, file_path: &str, color: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE tracks SET accent_color = ?1 WHERE file_path = ?2",
                params![color, file_path],
            )
            .map_err(|e| format!("Ошибка сохранения accent_color: {}", e))?;
        Ok(())
    }

    /// Получить все альбомы (удобный метод)
    pub fn get_albums(&self) -> Vec<PlaylistInfo> {
        self.get_playlists()
            .into_iter()
            .filter(|p| p.kind == PlaylistKind::Album)
            .collect()
    }

    /// Получить пользовательские плейлисты
    pub fn get_user_playlists(&self) -> Vec<PlaylistInfo> {
        self.get_playlists()
            .into_iter()
            .filter(|p| p.kind == PlaylistKind::User)
            .collect()
    }
}
