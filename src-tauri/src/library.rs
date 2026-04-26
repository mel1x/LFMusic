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
    Artist,
    User,
}

/// Информация о плейлисте для фронтенда
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistInfo {
    pub id: i64,
    pub name: String,
    pub kind: PlaylistKind,
    pub track_count: usize,
    /// Путь до первого трека (для подбора обложки на фронтенде)
    pub first_file_path: Option<String>,
}

/// Разделить строку с исполнителями по списку разделителей.
/// Применяет каждый разделитель последовательно, обрезает пробелы и убирает пустые элементы.
pub fn split_artists(artist: &str, separators: &[String]) -> Vec<String> {
    let mut parts: Vec<String> = vec![artist.to_string()];
    for sep in separators {
        if sep.is_empty() { continue; }
        let mut next: Vec<String> = Vec::with_capacity(parts.len());
        for p in parts.drain(..) {
            for piece in p.split(sep.as_str()) {
                next.push(piece.to_string());
            }
        }
        parts = next;
    }
    parts.into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
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
             PRAGMA foreign_keys=ON;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-8000;
             PRAGMA temp_store=MEMORY;",
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

        // Миграции: добавляем столбцы только если их ещё нет
        let existing_columns: HashSet<String> = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(tracks)")
                .map_err(|e| format!("Ошибка PRAGMA table_info: {}", e))?;
            let cols = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|e| format!("Ошибка чтения столбцов: {}", e))?;
            cols.filter_map(|r| r.ok()).collect()
        };

        if !existing_columns.contains("sort_order") {
            let _ = conn.execute_batch(
                "ALTER TABLE tracks ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;"
            );
        }

        if !existing_columns.contains("genre") {
            conn.execute_batch(
                "ALTER TABLE tracks ADD COLUMN genre TEXT;"
            ).map_err(|e| format!("Ошибка миграции genre: {}", e))?;
            // Сбрасываем modified_secs только один раз — при добавлении столбца
            conn.execute_batch(
                "UPDATE tracks SET modified_secs = 0;"
            ).map_err(|e| format!("Ошибка сброса modified_secs: {}", e))?;
        }

        if !existing_columns.contains("accent_color") {
            let _ = conn.execute_batch(
                "ALTER TABLE tracks ADD COLUMN accent_color TEXT;"
            );
        }

        // Версионированные миграции кэша через PRAGMA user_version.
        // v1: пересчёт accent_color по обновлённому алгоритму извлечения.
        let user_version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);
        if user_version < 1 {
            let _ = conn.execute_batch(
                "UPDATE tracks SET accent_color = NULL; PRAGMA user_version = 1;",
            );
        }

        Ok(Self { conn })
    }

    /// Корректное закрытие БД перед выходом: сливаем WAL в основной файл,
    /// запускаем PRAGMA optimize и переключаем журнал в DELETE,
    /// чтобы SQLite удалил sidecar-файлы -wal и -shm.
    pub fn shutdown(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "PRAGMA wal_checkpoint(TRUNCATE);
                 PRAGMA optimize;
                 PRAGMA journal_mode=DELETE;",
            )
            .map_err(|e| format!("Ошибка закрытия БД: {}", e))
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
        F: Fn(usize, usize) + Send + Sync,
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
        artist_separators: &[String],
        progress: F,
    ) -> Result<Vec<TrackMetadata>, String>
    where
        F: Fn(usize, usize) + Send + Sync,
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
        self.rebuild_artist_playlists(artist_separators)?;
        Ok(self.get_all_tracks())
    }

    pub fn remove_folder(&mut self, folder: String, artist_separators: &[String]) -> Result<(), String> {
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
        self.rebuild_artist_playlists(artist_separators)?;
        Ok(())
    }

    /// Удаляет из кэша треки, файлы которых больше не существуют на диске
    pub fn cleanup_missing_tracks(&mut self) -> Result<usize, String> {
        self.delete_missing_tracks()
    }

    /// Внутренний метод: удаляет треки с несуществующими файлами, возвращает количество удалённых
    fn delete_missing_tracks(&mut self) -> Result<usize, String> {
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

            // Пакетный DELETE батчами по N — один prepared statement, N плейсхолдеров
            const BATCH: usize = 256;
            for chunk in dead.chunks(BATCH) {
                let placeholders = (0..chunk.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!("DELETE FROM tracks WHERE file_path IN ({})", placeholders);
                let params: Vec<&dyn rusqlite::ToSql> =
                    chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
                tx.execute(&sql, params.as_slice()).ok();
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

    pub fn rescan_all<F>(&mut self, artist_separators: &[String], progress: F) -> Result<Vec<TrackMetadata>, String>
    where
        F: Fn(usize, usize) + Send + Sync,
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
        self.delete_missing_tracks()?;

        self.rebuild_album_playlists()?;
        self.rebuild_artist_playlists(artist_separators)?;
        Ok(self.get_all_tracks())
    }

    /// Внутренний метод: обрабатывает уже собранный список файлов с прогрессом
    fn scan_folder_files<F>(
        &mut self,
        folder: &str,
        files: &[PathBuf],
        progress: F,
    ) -> Result<(), String>
    where
        F: Fn(usize, usize) + Send + Sync,
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
                // Обновляем прогресс каждые 10 файлов или при завершении
                if done % 10 == 0 || done == total {
                    progress(done, total);
                }
                Some((path_str.clone(), result, *mtime))
            })
            .collect();

        progress(total, total);

        // 4. Пакетная вставка в одной транзакции, подготовленный statement
        if !batch.is_empty() {
            let tx = self
                .conn
                .transaction()
                .map_err(|e| format!("Ошибка транзакции: {}", e))?;

            {
                let mut stmt = tx.prepare(
                    "INSERT INTO tracks
                     (file_path, folder_path, title, artist, album, genre, duration_secs,
                      format, sample_rate, bit_depth, channels, bitrate, is_lossless, modified_secs)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
                     ON CONFLICT(file_path) DO UPDATE SET
                        folder_path = excluded.folder_path,
                        title = excluded.title,
                        artist = excluded.artist,
                        album = excluded.album,
                        genre = excluded.genre,
                        duration_secs = excluded.duration_secs,
                        format = excluded.format,
                        sample_rate = excluded.sample_rate,
                        bit_depth = excluded.bit_depth,
                        channels = excluded.channels,
                        bitrate = excluded.bitrate,
                        is_lossless = excluded.is_lossless,
                        modified_secs = excluded.modified_secs,
                        accent_color = NULL"
                ).map_err(|e| format!("Ошибка подготовки INSERT: {}", e))?;

                for (file_path, m, mtime) in &batch {
                    stmt.execute(params![
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
                    ])
                    .map_err(|e| format!("Ошибка вставки: {}", e))?;
                }
            }

            tx.commit()
                .map_err(|e| format!("Ошибка коммита: {}", e))?;
        }

        Ok(())
    }

    // ── Плейлисты ─────────────────────────────────────────────────────────────

    /// Получить все плейлисты с количеством треков и путём первого трека
    pub fn get_playlists(&self) -> Vec<PlaylistInfo> {
        let mut stmt = match self.conn.prepare(
            "SELECT p.id, p.name, p.kind, COUNT(pt.file_path) as cnt,
                    (SELECT pt2.file_path FROM playlist_tracks pt2
                     WHERE pt2.playlist_id = p.id
                     ORDER BY pt2.sort_order ASC, pt2.rowid ASC LIMIT 1) as first_path
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
            let kind = match kind_str.as_str() {
                "album" => PlaylistKind::Album,
                "artist" => PlaylistKind::Artist,
                _ => PlaylistKind::User,
            };
            Ok(PlaylistInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                kind,
                track_count: row.get(3)?,
                first_file_path: row.get::<_, Option<String>>(4)?,
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
             ORDER BY pt.sort_order ASC,
                      pt.rowid ASC",
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
            first_file_path: None,
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

        let max_order: i64 = self.conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM playlist_tracks WHERE playlist_id = ?1",
                params![playlist_id],
                |r| r.get(0),
            )
            .map_err(|e| format!("Ошибка чтения sort_order: {}", e))?;

        self.conn
            .execute(
                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, file_path, sort_order) VALUES (?1, ?2, ?3)",
                params![playlist_id, file_path, max_order + 1],
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

    /// Вернуть id пользовательских плейлистов, содержащих указанный трек
    pub fn get_user_playlist_ids_for_track(&self, file_path: &str) -> Vec<i64> {
        let mut stmt = match self.conn.prepare(
            "SELECT pt.playlist_id
             FROM playlist_tracks pt
             JOIN playlists p ON p.id = pt.playlist_id
             WHERE pt.file_path = ?1 AND p.kind = 'user'",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map(params![file_path], |row| row.get::<_, i64>(0)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok()).collect()
    }

    /// Поднять трек на первое место в плейлисте
    pub fn move_playlist_track_to_top(&self, playlist_id: i64, file_path: &str) -> Result<(), String> {
        let min_order: i64 = self.conn
            .query_row(
                "SELECT COALESCE(MIN(sort_order), 0) FROM playlist_tracks WHERE playlist_id = ?1",
                params![playlist_id],
                |r| r.get(0),
            )
            .map_err(|e| format!("Ошибка чтения sort_order: {}", e))?;

        self.conn
            .execute(
                "UPDATE playlist_tracks SET sort_order = ?1 WHERE playlist_id = ?2 AND file_path = ?3",
                params![min_order - 1, playlist_id, file_path],
            )
            .map_err(|e| format!("Ошибка обновления sort_order: {}", e))?;
        Ok(())
    }

    /// Инкрементальное обновление альбомных плейлистов.
    /// Для каждого существующего альбома вычисляет diff (added/removed)
    /// и применяет только его — неизменившиеся альбомы не трогаются вообще.
    pub fn rebuild_album_playlists(&mut self) -> Result<(), String> {
        // Собираем актуальные альбомы из tracks (с >1 треком)
        let actual_albums: HashMap<String, HashSet<String>> = {
            let mut stmt = self.conn.prepare(
                "SELECT album, file_path FROM tracks
                 WHERE album IS NOT NULL AND album != ''"
            ).map_err(|e| format!("Ошибка запроса альбомов: {}", e))?;

            let mut map: HashMap<String, HashSet<String>> = HashMap::new();
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .map_err(|e| format!("Ошибка: {}", e))?;

            for row in rows.flatten() {
                map.entry(row.0).or_default().insert(row.1);
            }

            map.retain(|_, tracks| tracks.len() > 1);
            map
        };

        // Существующие альбомные плейлисты: name -> (id, Set<file_path>)
        let existing_albums: HashMap<String, (i64, HashSet<String>)> = {
            let mut stmt = self.conn.prepare(
                "SELECT p.id, p.name, pt.file_path
                 FROM playlists p
                 LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
                 WHERE p.kind = 'album'"
            ).map_err(|e| format!("Ошибка запроса плейлистов: {}", e))?;

            let mut map: HashMap<String, (i64, HashSet<String>)> = HashMap::new();
            let rows = stmt
                .query_map([], |row| Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                )))
                .map_err(|e| format!("Ошибка: {}", e))?;

            for row in rows.flatten() {
                let entry = map.entry(row.1).or_insert_with(|| (row.0, HashSet::new()));
                if let Some(p) = row.2 { entry.1.insert(p); }
            }
            map
        };

        let tx = self.conn
            .transaction()
            .map_err(|e| format!("Ошибка транзакции: {}", e))?;

        // Удаляем плейлисты альбомов, которых больше нет
        for (name, (id, _)) in &existing_albums {
            if !actual_albums.contains_key(name) {
                tx.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", params![id]).ok();
                tx.execute("DELETE FROM playlists WHERE id = ?1", params![id]).ok();
            }
        }

        // Для каждого актуального альбома применяем только diff
        for (album_name, new_set) in &actual_albums {
            let playlist_id = if let Some((id, current_set)) = existing_albums.get(album_name) {
                // Пропускаем неизменившиеся альбомы целиком
                if current_set == new_set {
                    continue;
                }
                // Удаляем треки, которых больше нет
                let to_remove: Vec<&String> = current_set.difference(new_set).collect();
                if !to_remove.is_empty() {
                    const BATCH: usize = 256;
                    for chunk in to_remove.chunks(BATCH) {
                        let placeholders = (0..chunk.len())
                            .map(|i| format!("?{}", i + 2))
                            .collect::<Vec<_>>()
                            .join(",");
                        let sql = format!(
                            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND file_path IN ({})",
                            placeholders
                        );
                        let mut ps: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk.len() + 1);
                        ps.push(id as &dyn rusqlite::ToSql);
                        for p in chunk { ps.push(*p as &dyn rusqlite::ToSql); }
                        tx.execute(&sql, ps.as_slice()).ok();
                    }
                }
                *id
            } else {
                tx.execute(
                    "INSERT INTO playlists (name, kind) VALUES (?1, 'album')",
                    params![album_name],
                )
                .map_err(|e| format!("Ошибка создания альбома: {}", e))?;
                tx.last_insert_rowid()
            };

            // Добавляем только новые треки
            let current_set = existing_albums
                .get(album_name)
                .map(|(_, s)| s)
                .cloned()
                .unwrap_or_default();
            let to_add: Vec<&String> = new_set.difference(&current_set).collect();
            if !to_add.is_empty() {
                let mut stmt = tx.prepare(
                    "INSERT OR IGNORE INTO playlist_tracks (playlist_id, file_path, sort_order)
                     VALUES (?1, ?2, 0)"
                ).map_err(|e| format!("Ошибка подготовки INSERT playlist_tracks: {}", e))?;
                for file_path in to_add {
                    stmt.execute(params![playlist_id, file_path]).ok();
                }
            }
        }

        tx.commit().map_err(|e| format!("Ошибка коммита: {}", e))?;
        Ok(())
    }

    /// Инкрементальное обновление плейлистов исполнителей.
    /// Каждый трек разбивается по списку разделителей, и для каждого имени
    /// создаётся/обновляется плейлист с kind='artist'.
    pub fn rebuild_artist_playlists(&mut self, separators: &[String]) -> Result<(), String> {
        // Собираем актуальные плейлисты исполнителей из tracks
        let actual_artists: HashMap<String, HashSet<String>> = {
            let mut stmt = self.conn.prepare(
                "SELECT artist, file_path FROM tracks
                 WHERE artist IS NOT NULL AND artist != ''"
            ).map_err(|e| format!("Ошибка запроса исполнителей: {}", e))?;

            let mut map: HashMap<String, HashSet<String>> = HashMap::new();
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .map_err(|e| format!("Ошибка: {}", e))?;

            for row in rows.flatten() {
                let names = split_artists(&row.0, separators);
                for name in names {
                    map.entry(name).or_default().insert(row.1.clone());
                }
            }
            map
        };

        // Существующие плейлисты исполнителей: name -> (id, Set<file_path>)
        let existing_artists: HashMap<String, (i64, HashSet<String>)> = {
            let mut stmt = self.conn.prepare(
                "SELECT p.id, p.name, pt.file_path
                 FROM playlists p
                 LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
                 WHERE p.kind = 'artist'"
            ).map_err(|e| format!("Ошибка запроса плейлистов исполнителей: {}", e))?;

            let mut map: HashMap<String, (i64, HashSet<String>)> = HashMap::new();
            let rows = stmt
                .query_map([], |row| Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                )))
                .map_err(|e| format!("Ошибка: {}", e))?;

            for row in rows.flatten() {
                let entry = map.entry(row.1).or_insert_with(|| (row.0, HashSet::new()));
                if let Some(p) = row.2 { entry.1.insert(p); }
            }
            map
        };

        let tx = self.conn
            .transaction()
            .map_err(|e| format!("Ошибка транзакции: {}", e))?;

        // Удаляем плейлисты исполнителей, которых больше нет
        for (name, (id, _)) in &existing_artists {
            if !actual_artists.contains_key(name) {
                tx.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", params![id]).ok();
                tx.execute("DELETE FROM playlists WHERE id = ?1", params![id]).ok();
            }
        }

        for (artist_name, new_set) in &actual_artists {
            let playlist_id = if let Some((id, current_set)) = existing_artists.get(artist_name) {
                if current_set == new_set {
                    continue;
                }
                let to_remove: Vec<&String> = current_set.difference(new_set).collect();
                if !to_remove.is_empty() {
                    const BATCH: usize = 256;
                    for chunk in to_remove.chunks(BATCH) {
                        let placeholders = (0..chunk.len())
                            .map(|i| format!("?{}", i + 2))
                            .collect::<Vec<_>>()
                            .join(",");
                        let sql = format!(
                            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND file_path IN ({})",
                            placeholders
                        );
                        let mut ps: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk.len() + 1);
                        ps.push(id as &dyn rusqlite::ToSql);
                        for p in chunk { ps.push(*p as &dyn rusqlite::ToSql); }
                        tx.execute(&sql, ps.as_slice()).ok();
                    }
                }
                *id
            } else {
                tx.execute(
                    "INSERT INTO playlists (name, kind) VALUES (?1, 'artist')",
                    params![artist_name],
                )
                .map_err(|e| format!("Ошибка создания плейлиста исполнителя: {}", e))?;
                tx.last_insert_rowid()
            };

            let current_set = existing_artists
                .get(artist_name)
                .map(|(_, s)| s)
                .cloned()
                .unwrap_or_default();
            let to_add: Vec<&String> = new_set.difference(&current_set).collect();
            if !to_add.is_empty() {
                let mut stmt = tx.prepare(
                    "INSERT OR IGNORE INTO playlist_tracks (playlist_id, file_path, sort_order)
                     VALUES (?1, ?2, 0)"
                ).map_err(|e| format!("Ошибка подготовки INSERT playlist_tracks: {}", e))?;
                for file_path in to_add {
                    stmt.execute(params![playlist_id, file_path]).ok();
                }
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

    /// Получить все плейлисты исполнителей
    pub fn get_artists(&self) -> Vec<PlaylistInfo> {
        self.get_playlists()
            .into_iter()
            .filter(|p| p.kind == PlaylistKind::Artist)
            .collect()
    }
}
