use crate::audio_features::{self, AudioFeatures, AUDIO_FEATURE_ANALYZER_VERSION};
use crate::audio_player::{AudioPlayer, TrackMetadata};
use crate::external::ExternalTrack;
use crate::statistics::{self, CollectionKind, TrackStatistics, TrackStatisticsDelta};
use rayon::prelude::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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

/// Жанр + количество треков (для фильтр-чипов каталога).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenreCount {
    pub name: String,
    pub count: u32,
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

/// Информация о популярном артисте
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopArtist {
    pub name: String,
    pub listen_count: i64,
    pub total_listened_secs: f64,
    pub track_count: usize,
}

/// Информация о популярном альбоме
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopAlbum {
    pub name: String,
    pub artist: String,
    pub listen_count: i64,
    pub total_listened_secs: f64,
    pub first_file_path: Option<String>,
}

/// Сводная статистика библиотеки
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryStats {
    pub total_tracks: usize,
    pub total_folders: usize,
    pub total_listened_secs: f64,
    pub total_listens: i64,
    pub morning_listens: i64,
    pub day_listens: i64,
    pub evening_listens: i64,
    pub night_listens: i64,
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
    db_path: PathBuf,
    /// Кэш накопителей центроида вкусов («Мой микс»). Считается лениво один раз
    /// (O(N) скан истории) и далее обновляется инкрементально при завершении
    /// прослушивания трека — см. [`MusicLibrary::update_centroid_for_listen`].
    centroid_cache: RefCell<Option<CentroidAccumulators>>,
    /// In-memory индекс поиска по каталогу (приведённые к нижнему регистру поля).
    /// Строится лениво из БД, сбрасывается по флагу [`SEARCH_INDEX_DIRTY`].
    search_index: RefCell<Option<Vec<crate::catalog_search::SearchEntry>>>,
}

pub type AudioFeatureProgressEmitter = Arc<dyn Fn(usize, usize) + Send + Sync + 'static>;

static AUDIO_FEATURE_WORKER_GENERATION: AtomicU64 = AtomicU64::new(0);
static AUDIO_FEATURE_WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Помечается из фоновых/межпоточных операций (анализ фич, ре-скан), которые
/// меняют состав истории. Кэш центроида сбрасывается при следующем чтении.
static CENTROID_CACHE_DIRTY: AtomicBool = AtomicBool::new(false);
/// Помечается при изменении состава треков или их полей (название/исполнитель/
/// альбом/жанр/путь). In-memory индекс поиска по каталогу пересобирается при
/// следующем чтении. Стартовое значение `true` — индекс строится лениво.
static SEARCH_INDEX_DIRTY: AtomicBool = AtomicBool::new(true);

fn current_audio_feature_generation() -> u64 {
    AUDIO_FEATURE_WORKER_GENERATION.load(Ordering::SeqCst)
}

fn is_audio_feature_generation_current(generation: u64) -> bool {
    current_audio_feature_generation() == generation
}

fn spawn_audio_feature_analysis(
    db_path: PathBuf,
    jobs: Vec<(String, Option<f64>, u64)>,
    initial_completed: usize,
    total: usize,
    generation: u64,
    progress: Option<AudioFeatureProgressEmitter>,
) {
    if jobs.is_empty() {
        return;
    }

    let _ = std::thread::Builder::new()
        .name("audio-feature-analysis".into())
        .spawn(move || {
            let mut completed = initial_completed.min(total);
            if is_audio_feature_generation_current(generation) {
                AUDIO_FEATURE_WORKER_ACTIVE.store(true, Ordering::SeqCst);
            }

            if is_audio_feature_generation_current(generation) {
                if let Some(progress) = &progress {
                    progress(completed, total);
                }
            }

            let Ok(mut conn) = Connection::open(db_path) else {
                if is_audio_feature_generation_current(generation) {
                    AUDIO_FEATURE_WORKER_ACTIVE.store(false, Ordering::SeqCst);
                }
                if is_audio_feature_generation_current(generation) {
                    if let Some(progress) = &progress {
                        progress(total, total);
                    }
                }
                return;
            };
            let _ = conn.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA foreign_keys=ON;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA busy_timeout=5000;
                 PRAGMA temp_store=MEMORY;",
            );

            let pool = match rayon::ThreadPoolBuilder::new()
                .num_threads(MusicLibrary::scan_thread_count())
                .build()
            {
                Ok(pool) => pool,
                Err(_) => {
                    if is_audio_feature_generation_current(generation) {
                        AUDIO_FEATURE_WORKER_ACTIVE.store(false, Ordering::SeqCst);
                    }
                    if is_audio_feature_generation_current(generation) {
                        if let Some(progress) = &progress {
                            progress(total, total);
                        }
                    }
                    return;
                }
            };

            let chunk_size = MusicLibrary::scan_thread_count().max(1);
            for chunk in jobs.chunks(chunk_size) {
                if !is_audio_feature_generation_current(generation) {
                    return;
                }

                let results: Vec<(String, u64, Option<AudioFeatures>)> = pool.install(|| {
                    chunk
                        .par_iter()
                        .filter_map(|(path, duration, mtime)| {
                            if !is_audio_feature_generation_current(generation) {
                                return None;
                            }

                            let features = audio_features::analyze_file(path, *duration).ok();
                            if !is_audio_feature_generation_current(generation) {
                                return None;
                            }

                            Some((
                                path.clone(),
                                *mtime,
                                features,
                            ))
                        })
                        .collect()
                });

                if !is_audio_feature_generation_current(generation) {
                    return;
                }

                let Ok(tx) = conn.transaction() else {
                    continue;
                };

                if let Ok(mut stmt) = tx.prepare(
                        "INSERT INTO track_audio_features
                         (file_path, bpm, loudness, energy, danceability, spectral_centroid,
                          track_key, audio_fingerprint,
                          analyzed_modified_secs, analyzer_version)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                         ON CONFLICT(file_path) DO UPDATE SET
                            bpm = excluded.bpm,
                            loudness = excluded.loudness,
                            energy = excluded.energy,
                            danceability = excluded.danceability,
                            spectral_centroid = excluded.spectral_centroid,
                            track_key = excluded.track_key,
                            audio_fingerprint = excluded.audio_fingerprint,
                            analyzed_modified_secs = excluded.analyzed_modified_secs,
                            analyzer_version = excluded.analyzer_version",
                    ) {
                    for (file_path, mtime, features) in &results {
                        let _ = stmt.execute(params![
                            file_path,
                            features.as_ref().and_then(|f| f.bpm),
                            features.as_ref().and_then(|f| f.loudness),
                            features.as_ref().and_then(|f| f.energy),
                            features.as_ref().and_then(|f| f.danceability),
                            features.as_ref().and_then(|f| f.spectral_centroid),
                            features.as_ref().and_then(|f| f.key.as_deref()),
                            features.as_ref().and_then(|f| f.audio_fingerprint.as_deref()),
                            *mtime as i64,
                            AUDIO_FEATURE_ANALYZER_VERSION,
                        ]);
                    }
                }

                let _ = tx.commit();

                // Появились новые аудио-фичи — центроид мог измениться.
                CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        SEARCH_INDEX_DIRTY.store(true, Ordering::SeqCst);

                if !is_audio_feature_generation_current(generation) {
                    return;
                }

                completed = (completed + results.len()).min(total);
                if let Some(progress) = &progress {
                    progress(completed, total);
                }
            }

            if is_audio_feature_generation_current(generation) {
                AUDIO_FEATURE_WORKER_ACTIVE.store(false, Ordering::SeqCst);
                if let Some(progress) = &progress {
                    progress(total, total);
                }
            }
        });
}

impl MusicLibrary {
    fn scan_thread_count() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1).clamp(1, 4))
            .unwrap_or(2)
    }

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
             PRAGMA busy_timeout=5000;
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
                 sort_order    INTEGER NOT NULL DEFAULT 0,
                 accent_color  TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_tracks_folder ON tracks(folder_path);
             CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(artist);
             CREATE INDEX IF NOT EXISTS idx_tracks_album  ON tracks(album);
             CREATE INDEX IF NOT EXISTS idx_tracks_genre  ON tracks(genre);

             CREATE TABLE IF NOT EXISTS track_audio_features (
                 file_path              TEXT PRIMARY KEY NOT NULL,
                 bpm                    REAL,
                 loudness               REAL,
                 energy                 REAL,
                 danceability           REAL,
                 spectral_centroid      REAL,
                 track_key              TEXT,
                 audio_fingerprint      TEXT,
                 analyzed_modified_secs INTEGER NOT NULL DEFAULT 0,
                 analyzer_version       INTEGER NOT NULL DEFAULT 1,
                 FOREIGN KEY (file_path) REFERENCES tracks(file_path) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_track_audio_features_bpm
                 ON track_audio_features(bpm);
             CREATE INDEX IF NOT EXISTS idx_track_audio_features_fingerprint
                 ON track_audio_features(audio_fingerprint);

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
             CREATE INDEX IF NOT EXISTS idx_pt_playlist ON playlist_tracks(playlist_id);

             CREATE TABLE IF NOT EXISTS track_overrides (
                 file_path     TEXT PRIMARY KEY NOT NULL,
                 title         TEXT,
                 artist        TEXT,
                 cover_sig     TEXT,
                 accent_color  TEXT
             );
             CREATE TABLE IF NOT EXISTS album_track_order (
                 album_name  TEXT PRIMARY KEY NOT NULL,
                 track_paths TEXT NOT NULL DEFAULT '[]'
             );
             CREATE TABLE IF NOT EXISTS external_album (
                 album_key    TEXT PRIMARY KEY NOT NULL,
                 source       TEXT,
                 status       TEXT NOT NULL DEFAULT 'none',
                 fetched_secs INTEGER NOT NULL DEFAULT 0,
                 payload      TEXT NOT NULL DEFAULT '[]'
             );",
        )
        .map_err(|e| format!("Ошибка создания таблиц: {}", e))?;

        statistics::create_tables(&conn)
            .map_err(|e| format!("Ошибка создания таблиц статистики: {}", e))?;

        Ok(Self {
            conn,
            db_path,
            centroid_cache: RefCell::new(None),
            search_index: RefCell::new(None),
        })
    }

    pub fn resume_pending_audio_feature_analysis(
        &self,
        progress: Option<AudioFeatureProgressEmitter>,
    ) -> Result<usize, String> {
        self.restart_pending_audio_feature_analysis(progress)
    }

    fn restart_pending_audio_feature_analysis(
        &self,
        progress: Option<AudioFeatureProgressEmitter>,
    ) -> Result<usize, String> {
        let generation = AUDIO_FEATURE_WORKER_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        AUDIO_FEATURE_WORKER_ACTIVE.store(false, Ordering::SeqCst);

        let (completed, total) = self.audio_feature_progress_counts()?;
        let jobs = self.pending_audio_feature_jobs()?;
        let count = jobs.len();
        AUDIO_FEATURE_WORKER_ACTIVE.store(count > 0, Ordering::SeqCst);

        if count == 0 {
            return Ok(0);
        }

        spawn_audio_feature_analysis(
            self.db_path.clone(),
            jobs,
            completed,
            total,
            generation,
            progress,
        );
        Ok(count)
    }

    pub fn cancel_audio_feature_analysis(&self) {
        AUDIO_FEATURE_WORKER_GENERATION.fetch_add(1, Ordering::SeqCst);
        AUDIO_FEATURE_WORKER_ACTIVE.store(false, Ordering::SeqCst);
    }

    pub fn audio_feature_status(&self) -> Result<(usize, usize, bool), String> {
        let (completed, total) = self.audio_feature_progress_counts()?;
        let active = AUDIO_FEATURE_WORKER_ACTIVE.load(Ordering::SeqCst) && completed < total;
        Ok((completed, total, active))
    }

    fn pending_audio_feature_jobs(&self) -> Result<Vec<(String, Option<f64>, u64)>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT t.file_path, t.duration_secs, t.modified_secs
             FROM tracks t
             LEFT JOIN track_audio_features af
               ON af.file_path = t.file_path
              AND af.analyzed_modified_secs = t.modified_secs
              AND af.analyzer_version = ?1
             WHERE af.file_path IS NULL"
        ).map_err(|e| format!("Ошибка подготовки pending audio features: {}", e))?;

        let rows = stmt
            .query_map(params![AUDIO_FEATURE_ANALYZER_VERSION], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<f64>>(1)?,
                    row.get::<_, i64>(2)? as u64,
                ))
            })
            .map_err(|e| format!("Ошибка чтения pending audio features: {}", e))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    fn audio_feature_progress_counts(&self) -> Result<(usize, usize), String> {
        self.conn.query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN af.file_path IS NULL THEN 0 ELSE 1 END), 0),
                COUNT(*)
             FROM tracks t
             LEFT JOIN track_audio_features af
               ON af.file_path = t.file_path
              AND af.analyzed_modified_secs = t.modified_secs
              AND af.analyzer_version = ?1",
            params![AUDIO_FEATURE_ANALYZER_VERSION],
            |row| Ok((row.get::<_, i64>(0)? as usize, row.get::<_, i64>(1)? as usize)),
        ).map_err(|e| format!("Ошибка подсчёта прогресса audio features: {}", e))
    }

    /// Корректное закрытие БД перед выходом: делаем лёгкую оптимизацию и
    /// неблокирующий checkpoint. WAL оставляем включённым, потому что фоновые
    /// DSP-потоки могут ещё держать своё SQLite-соединение при закрытии окна.
    pub fn shutdown(&self) -> Result<(), String> {
        let _ = self.conn.execute_batch("PRAGMA optimize;");
        match self.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);") {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::DatabaseBusy
                    || err.code == rusqlite::ErrorCode::DatabaseLocked =>
            {
                Ok(())
            }
            Err(e) => Err(format!("Ошибка закрытия БД: {}", e)),
        }
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
        analyze_audio_features: bool,
        audio_feature_progress: Option<AudioFeatureProgressEmitter>,
        progress: F,
    ) -> Result<usize, String>
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

        self.scan_folder_files(
            folder,
            &all_files,
            analyze_audio_features,
            audio_feature_progress,
            progress,
        )
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
        audio_feature_progress: Option<AudioFeatureProgressEmitter>,
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

        let _ = self.scan_folder_internal(&folder, true, audio_feature_progress, progress)?;
        self.rebuild_album_playlists()?;
        self.rebuild_artist_playlists(artist_separators)?;
        // Состав треков изменился — кэш центроида вкусов больше не актуален.
        CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        SEARCH_INDEX_DIRTY.store(true, Ordering::SeqCst);
        Ok(self.get_all_tracks_light())
    }

    pub fn remove_folder(
        &mut self,
        folder: String,
        artist_separators: &[String],
        audio_feature_progress: Option<AudioFeatureProgressEmitter>,
    ) -> Result<(), String> {
        self.cancel_audio_feature_analysis();

        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("Ошибка транзакции: {}", e))?;

        // Сбрасываем все пользовательские правки только при удалении папки
        // из скоупа приложения — переопределения треков в этой папке исчезают.
        tx.execute(
            "DELETE FROM track_overrides
             WHERE file_path IN (SELECT file_path FROM tracks WHERE folder_path = ?1)",
            params![folder],
        )
        .map_err(|e| format!("Ошибка удаления переопределений: {}", e))?;

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
        let _ = self.restart_pending_audio_feature_analysis(audio_feature_progress);
        // Состав треков изменился — кэш центроида вкусов больше не актуален.
        CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        SEARCH_INDEX_DIRTY.store(true, Ordering::SeqCst);
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

        // Проверки существования файлов — IO-bound, параллелим
        // (на больших библиотеках и сетевых дисках серийный обход очень медленный)
        let dead: Vec<String> = all_paths
            .into_par_iter()
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

            // Удалены треки (вместе с их статистикой) — центроид мог измениться.
            CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        SEARCH_INDEX_DIRTY.store(true, Ordering::SeqCst);
        }

        Ok(count)
    }

    pub fn get_all_tracks(&self) -> Vec<TrackMetadata> {
        self.get_all_tracks_inner(false)
    }

    /// Получить все треки без обложек (для экономии памяти при больших библиотеках)
    pub fn get_all_tracks_light(&self) -> Vec<TrackMetadata> {
        self.get_all_tracks_inner(true)
    }

    fn get_all_tracks_inner(&self, light: bool) -> Vec<TrackMetadata> {
        let sql = if light {
            "SELECT t.file_path,
                    COALESCE(o.title, t.title),
                    COALESCE(o.artist, t.artist),
                    t.album, t.genre, t.duration_secs,
                    t.format, t.sample_rate, t.bit_depth, t.channels, t.bitrate, t.is_lossless,
                    COALESCE(o.accent_color, t.accent_color),
                    NULL,
                    af.bpm, af.loudness, af.energy, af.danceability,
                    af.spectral_centroid, af.track_key, af.audio_fingerprint
             FROM tracks t
             LEFT JOIN track_overrides o ON o.file_path = t.file_path
             LEFT JOIN track_audio_features af ON af.file_path = t.file_path
             ORDER BY
                 t.sort_order DESC,
                 CASE WHEN COALESCE(o.artist, t.artist) IS NULL THEN 1 ELSE 0 END,
                 LOWER(COALESCE(o.artist, t.artist)),
                 LOWER(COALESCE(t.album, '')),
                 LOWER(COALESCE(o.title, t.title, t.file_path))"
        } else {
            "SELECT t.file_path,
                    COALESCE(o.title, t.title),
                    COALESCE(o.artist, t.artist),
                    t.album, t.genre, t.duration_secs,
                    t.format, t.sample_rate, t.bit_depth, t.channels, t.bitrate, t.is_lossless,
                    COALESCE(o.accent_color, t.accent_color),
                    o.cover_sig,
                    af.bpm, af.loudness, af.energy, af.danceability,
                    af.spectral_centroid, af.track_key, af.audio_fingerprint
             FROM tracks t
             LEFT JOIN track_overrides o ON o.file_path = t.file_path
             LEFT JOIN track_audio_features af ON af.file_path = t.file_path
             ORDER BY
                 t.sort_order DESC,
                 CASE WHEN COALESCE(o.artist, t.artist) IS NULL THEN 1 ELSE 0 END,
                 LOWER(COALESCE(o.artist, t.artist)),
                 LOWER(COALESCE(t.album, '')),
                 LOWER(COALESCE(o.title, t.title, t.file_path))"
        };

        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map([], |row| Self::row_to_metadata_with_override(row)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok()).collect()
    }

    fn row_to_metadata_with_override(row: &rusqlite::Row) -> rusqlite::Result<TrackMetadata> {
        let file_path: String = row.get(0)?;
        // Пользовательская (override) обложка отдаётся уменьшенной WebP-миниатюрой
        // через протокол `lfmusic-cover` (без base64 в списке). Встроенная обложка
        // догружается лениво через cover loader, поэтому здесь None.
        // Колонка 13 — уже готовая сигнатура override-обложки (cover_sig), а не
        // её сырые данные: URL миниатюры строится без чтения байтов и хеширования.
        let cover_art = row
            .get::<_, Option<String>>(13)?
            .map(|sig| crate::thumbnails::cover_protocol_url(&file_path, &sig));
        Ok(TrackMetadata {
            file_path,
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
            cover_art,
            accent_color: row.get(12)?,
            bpm: row.get(14)?,
            loudness: row.get(15)?,
            energy: row.get(16)?,
            danceability: row.get(17)?,
            spectral_centroid: row.get(18)?,
            key: row.get(19)?,
            audio_fingerprint: row.get(20)?,
        })
    }

    // ── Серверная пагинация каталога + поиск (in-memory индекс) ────────────────

    /// ORDER BY канонического порядка каталога (совпадает с get_all_tracks_inner).
    const CATALOG_ORDER_BY: &'static str = " ORDER BY
                 t.sort_order DESC,
                 CASE WHEN COALESCE(o.artist, t.artist) IS NULL THEN 1 ELSE 0 END,
                 LOWER(COALESCE(o.artist, t.artist)),
                 LOWER(COALESCE(t.album, '')),
                 LOWER(COALESCE(o.title, t.title, t.file_path))";

    /// Построить in-memory индекс поиска из БД в каноническом порядке.
    fn build_search_index(&self) -> Vec<crate::catalog_search::SearchEntry> {
        let sql = format!(
            "SELECT t.file_path,
                    COALESCE(o.title, t.title),
                    COALESCE(o.artist, t.artist),
                    t.album, t.genre
             FROM tracks t
             LEFT JOIN track_overrides o ON o.file_path = t.file_path{}",
            Self::CATALOG_ORDER_BY
        );
        let mut stmt = match self.conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |row| {
            let path: String = row.get(0)?;
            let title: Option<String> = row.get(1)?;
            let artist: Option<String> = row.get(2)?;
            let album: Option<String> = row.get(3)?;
            let genre: Option<String> = row.get(4)?;
            Ok(crate::catalog_search::SearchEntry::new(
                path, title, artist, album, genre,
            ))
        });
        match rows {
            Ok(r) => r.filter_map(|x| x.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Гарантирует актуальность индекса (пересборка по dirty-флагу / при отсутствии).
    fn ensure_search_index(&self) {
        if SEARCH_INDEX_DIRTY.swap(false, Ordering::SeqCst) {
            self.search_index.borrow_mut().take();
        }
        if self.search_index.borrow().is_none() {
            let built = self.build_search_index();
            *self.search_index.borrow_mut() = Some(built);
        }
    }

    /// Ранжированные пути по запросу (+ опциональный фильтр по жанру).
    fn ranked_paths(&self, genre: Option<&str>, search: &str, limit: Option<usize>) -> Vec<String> {
        self.ensure_search_index();
        let guard = self.search_index.borrow();
        let entries = match guard.as_ref() {
            Some(e) => e,
            None => return Vec::new(),
        };
        let ranked = crate::catalog_search::rank(entries, search, genre);
        let iter = ranked.into_iter().map(|e| e.path.clone());
        match limit {
            Some(n) => iter.take(n).collect(),
            None => iter.collect(),
        }
    }

    /// Прямой SQL-запрос путей каталога в каноническом порядке (без поиска),
    /// с опциональным точным фильтром по жанру (индекс idx_tracks_genre).
    fn catalog_paths_sql(&self, genre: Option<&str>) -> Vec<String> {
        let base = "SELECT t.file_path
                    FROM tracks t
                    LEFT JOIN track_overrides o ON o.file_path = t.file_path";
        let sql = match genre {
            Some(_) => format!("{} WHERE t.genre = ?1{}", base, Self::CATALOG_ORDER_BY),
            None => format!("{}{}", base, Self::CATALOG_ORDER_BY),
        };
        let mut stmt = match self.conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        // Замыкания query_map с разными аргументами имеют разные типы, поэтому
        // собираем результат в Vec прямо внутри каждой ветки.
        match genre {
            Some(g) => match stmt.query_map(params![g], |row| row.get::<_, String>(0)) {
                Ok(rows) => rows.filter_map(|x| x.ok()).collect(),
                Err(_) => Vec::new(),
            },
            None => match stmt.query_map([], |row| row.get::<_, String>(0)) {
                Ok(rows) => rows.filter_map(|x| x.ok()).collect(),
                Err(_) => Vec::new(),
            },
        }
    }

    /// Упорядоченные пути каталога с учётом фильтра по жанру и поисковому запросу.
    /// Поиск (если непустой) использует in-memory ранжирование; иначе — прямой SQL.
    pub fn get_catalog_paths(&self, genre: Option<&str>, search: Option<&str>) -> Vec<String> {
        let search_q = search.map(|s| s.trim()).filter(|s| !s.is_empty());
        match search_q {
            Some(q) => self.ranked_paths(genre, q, None),
            None => self.catalog_paths_sql(genre),
        }
    }

    /// Пакетная материализация метаданных по списку путей (с overrides + audio-features).
    /// Порядок результата совпадает с порядком входных путей; отсутствующие пропускаются.
    pub fn get_tracks_by_paths(&self, paths: &[String]) -> Vec<TrackMetadata> {
        if paths.is_empty() {
            return Vec::new();
        }
        let mut by_path: HashMap<String, TrackMetadata> = HashMap::new();
        for chunk in paths.chunks(900) {
            let placeholders = std::iter::repeat("?")
                .take(chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT t.file_path,
                        COALESCE(o.title, t.title),
                        COALESCE(o.artist, t.artist),
                        t.album, t.genre, t.duration_secs,
                        t.format, t.sample_rate, t.bit_depth, t.channels, t.bitrate, t.is_lossless,
                        COALESCE(o.accent_color, t.accent_color),
                        o.cover_sig,
                        af.bpm, af.loudness, af.energy, af.danceability,
                        af.spectral_centroid, af.track_key, af.audio_fingerprint
                 FROM tracks t
                 LEFT JOIN track_overrides o ON o.file_path = t.file_path
                 LEFT JOIN track_audio_features af ON af.file_path = t.file_path
                 WHERE t.file_path IN ({})",
                placeholders
            );
            let mut stmt = match self.conn.prepare(&sql) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let params_vec: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
            // Собираем строки в пределах жизни `stmt`, чтобы заём не «утёк» из блока.
            let metas: Vec<TrackMetadata> =
                match stmt.query_map(params_vec.as_slice(), |row| {
                    Self::row_to_metadata_with_override(row)
                }) {
                    Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                    Err(_) => Vec::new(),
                };
            for meta in metas {
                by_path.insert(meta.file_path.clone(), meta);
            }
        }
        paths.iter().filter_map(|p| by_path.remove(p)).collect()
    }

    /// Ранжированные треки по запросу (для секции «Треки» страницы поиска).
    pub fn search_tracks(&self, query: &str, limit: Option<usize>) -> Vec<TrackMetadata> {
        let paths = self.ranked_paths(None, query, limit);
        self.get_tracks_by_paths(&paths)
    }

    /// Список жанров с количеством треков (count > 1, по убыванию) — как в Catalog.vue.
    pub fn list_genres(&self) -> Vec<GenreCount> {
        let sql = "SELECT genre, COUNT(*) AS cnt
                   FROM tracks
                   WHERE genre IS NOT NULL AND genre != ''
                   GROUP BY genre
                   HAVING cnt > 1
                   ORDER BY cnt DESC";
        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |row| {
            Ok(GenreCount {
                name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u32,
            })
        });
        match rows {
            Ok(r) => r.filter_map(|x| x.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Получить один трек по пути с применением переопределений
    pub fn get_track(&self, file_path: &str) -> Option<TrackMetadata> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT t.file_path,
                        COALESCE(o.title, t.title),
                        COALESCE(o.artist, t.artist),
                        t.album, t.genre, t.duration_secs,
                        t.format, t.sample_rate, t.bit_depth, t.channels, t.bitrate, t.is_lossless,
                        COALESCE(o.accent_color, t.accent_color),
                        o.cover_sig,
                        af.bpm, af.loudness, af.energy, af.danceability,
                        af.spectral_centroid, af.track_key, af.audio_fingerprint
                 FROM tracks t
                 LEFT JOIN track_overrides o ON o.file_path = t.file_path
                 LEFT JOIN track_audio_features af ON af.file_path = t.file_path
                 WHERE t.file_path = ?1",
            )
            .ok()?;
        stmt.query_row(params![file_path], |row| Self::row_to_metadata_with_override(row))
            .ok()
    }

    /// Поднимает трек на первое место в каталоге
    pub fn move_track_to_top(&self, file_path: &str) -> Result<(), String> {
        // Находим максимальный sort_order и ставим +1
        let max_order: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(sort_order), 0) FROM tracks", [], |r| r.get(0))
            .map_err(|e| format!("Ошибка чтения sort_order: {}", e))?;

        let changed = self.conn
            .execute(
                "UPDATE tracks SET sort_order = ?1 WHERE file_path = ?2",
                params![max_order + 1, file_path],
            )
            .map_err(|e| format!("Ошибка обновления sort_order: {}", e))?;

        if changed > 0 {
            // Изменился sort_order → канонический порядок каталога устарел.
            SEARCH_INDEX_DIRTY.store(true, Ordering::SeqCst);
            self.add_track_statistics_delta(TrackStatisticsDelta {
                file_path: file_path.to_string(),
                move_to_top_count_delta: 1,
                ..TrackStatisticsDelta::default()
            })?;
        }

        Ok(())
    }

    pub fn rescan_all<F>(
        &mut self,
        artist_separators: &[String],
        audio_feature_progress: Option<AudioFeatureProgressEmitter>,
        progress: F,
    ) -> Result<Vec<TrackMetadata>, String>
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
        let mut scanned_count = 0usize;
        for (folder, files) in &folder_file_lists {
            let folder_total = files.len();
            let offset = global_current;

            scanned_count += self.scan_folder_files(folder, files, false, None, |i, _| {
                progress(offset + i, grand_total);
            })?;

            global_current += folder_total;
        }

        // Удаляем треки, файлы которых больше не существуют
        let deleted_count = self.delete_missing_tracks()?;

        self.rebuild_album_playlists()?;
        self.rebuild_artist_playlists(artist_separators)?;

        if scanned_count > 0 || deleted_count > 0 {
            let _ = self.restart_pending_audio_feature_analysis(audio_feature_progress);
        }

        // Состав треков мог измениться — кэш центроида вкусов больше не актуален.
        CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        SEARCH_INDEX_DIRTY.store(true, Ordering::SeqCst);
        Ok(self.get_all_tracks())
    }

    /// Внутренний метод: обрабатывает уже собранный список файлов с прогрессом
    fn scan_folder_files<F>(
        &mut self,
        folder: &str,
        files: &[PathBuf],
        analyze_audio_features: bool,
        audio_feature_progress: Option<AudioFeatureProgressEmitter>,
        progress: F,
    ) -> Result<usize, String>
    where
        F: Fn(usize, usize) + Send + Sync,
    {
        let total = files.len();
        if total == 0 {
            return Ok(0);
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

        // 2. Фильтруем файлы, которые нужно (пере)сканировать.
        // Чтение mtime — IO-bound, параллелим; par_iter сохраняет порядок файлов.
        let to_scan: Vec<(String, u64)> = files
            .par_iter()
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
            return Ok(0);
        }

        // 3. Параллельное чтение метаданных (без обложек)
        let counter = AtomicUsize::new(cached_count);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(Self::scan_thread_count())
            .build()
            .map_err(|e| format!("scan thread pool error: {}", e))?;

        let batch: Vec<(String, TrackMetadata, u64)> = pool.install(|| {
            to_scan
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
                .collect()
        });

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

            if analyze_audio_features {
                let _ = self.restart_pending_audio_feature_analysis(audio_feature_progress);
            }
        }

        Ok(batch.len())
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
        self.get_playlist_tracks_inner(playlist_id, false)
    }

    /// Получить треки плейлиста без обложек (для экономии памяти)
    pub fn get_playlist_tracks_light(&self, playlist_id: i64) -> Vec<TrackMetadata> {
        self.get_playlist_tracks_inner(playlist_id, true)
    }

    fn get_playlist_tracks_inner(&self, playlist_id: i64, light: bool) -> Vec<TrackMetadata> {
        let sql = if light {
            "SELECT t.file_path,
                    COALESCE(o.title, t.title),
                    COALESCE(o.artist, t.artist),
                    t.album, t.genre, t.duration_secs,
                    t.format, t.sample_rate, t.bit_depth, t.channels, t.bitrate, t.is_lossless,
                    COALESCE(o.accent_color, t.accent_color),
                    NULL,
                    af.bpm, af.loudness, af.energy, af.danceability,
                    af.spectral_centroid, af.track_key, af.audio_fingerprint
             FROM playlist_tracks pt
             JOIN tracks t ON t.file_path = pt.file_path
             LEFT JOIN track_overrides o ON o.file_path = t.file_path
             LEFT JOIN track_audio_features af ON af.file_path = t.file_path
             WHERE pt.playlist_id = ?1
             ORDER BY pt.sort_order ASC,
                      pt.rowid ASC"
        } else {
            "SELECT t.file_path,
                    COALESCE(o.title, t.title),
                    COALESCE(o.artist, t.artist),
                    t.album, t.genre, t.duration_secs,
                    t.format, t.sample_rate, t.bit_depth, t.channels, t.bitrate, t.is_lossless,
                    COALESCE(o.accent_color, t.accent_color),
                    o.cover_sig,
                    af.bpm, af.loudness, af.energy, af.danceability,
                    af.spectral_centroid, af.track_key, af.audio_fingerprint
             FROM playlist_tracks pt
             JOIN tracks t ON t.file_path = pt.file_path
             LEFT JOIN track_overrides o ON o.file_path = t.file_path
             LEFT JOIN track_audio_features af ON af.file_path = t.file_path
             WHERE pt.playlist_id = ?1
             ORDER BY pt.sort_order ASC,
                      pt.rowid ASC"
        };

        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map(params![playlist_id], |row| Self::row_to_metadata_with_override(row)) {
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

        // Снимаем -1 с playlist_add_count для каждого трека удаляемого плейлиста,
        // симметрично remove_track_from_playlist.
        let removed_paths: Vec<String> = {
            let mut stmt = self.conn
                .prepare("SELECT file_path FROM playlist_tracks WHERE playlist_id = ?1")
                .map_err(|e| format!("Ошибка чтения треков плейлиста: {}", e))?;
            let rows = stmt
                .query_map(params![playlist_id], |row| row.get::<_, String>(0))
                .map_err(|e| format!("Ошибка чтения треков плейлиста: {}", e))?;
            rows.filter_map(|r| r.ok()).collect()
        };

        let tx = self.conn
            .transaction()
            .map_err(|e| format!("Ошибка транзакции: {}", e))?;

        tx.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", params![playlist_id]).ok();
        tx.execute("DELETE FROM playlists WHERE id = ?1", params![playlist_id]).ok();
        for file_path in &removed_paths {
            statistics::apply_track_delta(
                &tx,
                &TrackStatisticsDelta {
                    file_path: file_path.clone(),
                    playlist_add_count_delta: -1,
                    ..TrackStatisticsDelta::default()
                },
            )
            .ok();
        }

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

        let changed = self.conn
            .execute(
                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, file_path, sort_order) VALUES (?1, ?2, ?3)",
                params![playlist_id, file_path, max_order + 1],
            )
            .map_err(|e| format!("Ошибка добавления трека: {}", e))?;
        if changed > 0 {
            self.add_track_statistics_delta(TrackStatisticsDelta {
                file_path: file_path.to_string(),
                playlist_add_count_delta: 1,
                ..TrackStatisticsDelta::default()
            })?;
        }
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

        let changed = self.conn
            .execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND file_path = ?2",
                params![playlist_id, file_path],
            )
            .map_err(|e| format!("Ошибка удаления трека: {}", e))?;
        if changed > 0 {
            self.add_track_statistics_delta(TrackStatisticsDelta {
                file_path: file_path.to_string(),
                playlist_add_count_delta: -1,
                ..TrackStatisticsDelta::default()
            })?;
        }
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

    pub fn add_track_statistics_delta(&self, delta: TrackStatisticsDelta) -> Result<(), String> {
        // Прослушивание трека меняет центроид вкусов — обновляем его кэш
        // инкрементально (вместо O(N) пересчёта всей истории при выдаче «Микса»).
        let listen_changed = delta.listen_count_delta != 0 && !delta.file_path.trim().is_empty();
        let old_count = if listen_changed {
            statistics::get_track_statistics(&self.conn, &delta.file_path)
                .map(|s| s.listen_count)
                .unwrap_or(0)
        } else {
            0
        };

        statistics::apply_track_delta(&self.conn, &delta)
            .map_err(|e| format!("Ошибка записи статистики трека: {}", e))?;

        if listen_changed {
            let new_count = (old_count + delta.listen_count_delta).max(0);
            self.update_centroid_for_listen(&delta.file_path, old_count, new_count);
        }
        Ok(())
    }

    pub fn get_track_statistics(&self, file_path: &str) -> Result<TrackStatistics, String> {
        statistics::get_track_statistics(&self.conn, file_path)
            .map_err(|e| format!("Ошибка чтения статистики трека: {}", e))
    }

    pub fn sum_track_listen_count(&self, file_paths: &[String]) -> Result<i64, String> {
        statistics::sum_listen_count(&self.conn, file_paths)
            .map_err(|e| format!("Ошибка чтения прослушиваний треков: {}", e))
    }

    pub fn add_collection_full_listen(
        &self,
        kind: CollectionKind,
        key: &str,
    ) -> Result<(), String> {
        statistics::apply_collection_full_listen(&self.conn, kind, key)
            .map_err(|e| format!("Ошибка записи статистики подборки: {}", e))
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

        let changed = self.conn
            .execute(
                "UPDATE playlist_tracks SET sort_order = ?1 WHERE playlist_id = ?2 AND file_path = ?3",
                params![min_order - 1, playlist_id, file_path],
            )
            .map_err(|e| format!("Ошибка обновления sort_order: {}", e))?;
        if changed > 0 {
            self.add_track_statistics_delta(TrackStatisticsDelta {
                file_path: file_path.to_string(),
                move_to_top_count_delta: 1,
                ..TrackStatisticsDelta::default()
            })?;
        }
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
        // Собираем актуальные плейлисты исполнителей из tracks с учётом
        // переопределений (track_overrides.artist имеет приоритет).
        // Исполнители группируются регистронезависимо (ключ = имя в нижнем
        // регистре), чтобы «MZLFF» и «mzlff» считались одним артистом. Для каждого
        // ключа храним счётчик вариантов написания (для выбора каноничного имени
        // у новых плейлистов) и объединённый набор треков.
        let actual_artists: HashMap<String, (HashMap<String, usize>, HashSet<String>)> = {
            let mut stmt = self.conn.prepare(
                "SELECT COALESCE(o.artist, t.artist) AS effective_artist, t.file_path
                 FROM tracks t
                 LEFT JOIN track_overrides o ON o.file_path = t.file_path
                 WHERE COALESCE(o.artist, t.artist) IS NOT NULL
                   AND COALESCE(o.artist, t.artist) != ''"
            ).map_err(|e| format!("Ошибка запроса исполнителей: {}", e))?;

            let mut map: HashMap<String, (HashMap<String, usize>, HashSet<String>)> = HashMap::new();
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .map_err(|e| format!("Ошибка: {}", e))?;

            for row in rows.flatten() {
                let names = split_artists(&row.0, separators);
                for name in names {
                    let key = name.to_lowercase();
                    let entry = map.entry(key).or_default();
                    *entry.0.entry(name).or_insert(0) += 1;
                    entry.1.insert(row.1.clone());
                }
            }
            map
        };

        // Существующие плейлисты исполнителей, сгруппированные по ключу (нижний
        // регистр имени). В одном ключе может оказаться несколько плейлистов —
        // legacy-дубликаты разного регистра, которые нужно слить в один.
        let existing_by_key: HashMap<String, Vec<(i64, String, HashSet<String>)>> = {
            let mut stmt = self.conn.prepare(
                "SELECT p.id, p.name, pt.file_path
                 FROM playlists p
                 LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
                 WHERE p.kind = 'artist'"
            ).map_err(|e| format!("Ошибка запроса плейлистов исполнителей: {}", e))?;

            let mut by_id: HashMap<i64, (String, HashSet<String>)> = HashMap::new();
            let rows = stmt
                .query_map([], |row| Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                )))
                .map_err(|e| format!("Ошибка: {}", e))?;

            for row in rows.flatten() {
                let entry = by_id.entry(row.0).or_insert_with(|| (row.1.clone(), HashSet::new()));
                if let Some(p) = row.2 { entry.1.insert(p); }
            }

            let mut map: HashMap<String, Vec<(i64, String, HashSet<String>)>> = HashMap::new();
            for (id, (name, set)) in by_id {
                map.entry(name.to_lowercase()).or_default().push((id, name, set));
            }
            map
        };

        let tx = self.conn
            .transaction()
            .map_err(|e| format!("Ошибка транзакции: {}", e))?;

        // Удаляем плейлисты исполнителей, которых больше нет
        for (key, playlists) in &existing_by_key {
            if !actual_artists.contains_key(key) {
                for (id, _, _) in playlists {
                    tx.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", params![id]).ok();
                    tx.execute("DELETE FROM playlists WHERE id = ?1", params![id]).ok();
                }
            }
        }

        for (key, (variants, new_set)) in &actual_artists {
            // Определяем основной плейлист для ключа. Если их несколько (legacy
            // дубликаты), оставляем самый «крупный» (по числу треков, при равенстве
            // — самый старый id) и сливаем в него остальные. Имя существующего
            // плейлиста сохраняем как есть, чтобы не оборвать закрепления, обложки
            // и порядок «сверху». Каноничное имя (по самому частому написанию)
            // выбираем только при создании нового плейлиста.
            let (playlist_id, current_set) = match existing_by_key.get(key) {
                Some(playlists) if !playlists.is_empty() => {
                    let mut sorted: Vec<&(i64, String, HashSet<String>)> = playlists.iter().collect();
                    sorted.sort_by(|a, b| b.2.len().cmp(&a.2.len()).then(a.0.cmp(&b.0)));
                    let primary = sorted[0];
                    let mut merged_current = primary.2.clone();
                    for dup in &sorted[1..] {
                        tx.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", params![dup.0]).ok();
                        tx.execute("DELETE FROM playlists WHERE id = ?1", params![dup.0]).ok();
                        for p in &dup.2 { merged_current.insert(p.clone()); }
                    }
                    (primary.0, merged_current)
                }
                _ => {
                    let canonical = variants
                        .iter()
                        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                        .map(|(name, _)| name.clone())
                        .unwrap_or_else(|| key.clone());
                    tx.execute(
                        "INSERT INTO playlists (name, kind) VALUES (?1, 'artist')",
                        params![canonical],
                    )
                    .map_err(|e| format!("Ошибка создания плейлиста исполнителя: {}", e))?;
                    (tx.last_insert_rowid(), HashSet::new())
                }
            };

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
                    ps.push(&playlist_id as &dyn rusqlite::ToSql);
                    for p in chunk { ps.push(*p as &dyn rusqlite::ToSql); }
                    tx.execute(&sql, ps.as_slice()).ok();
                }
            }

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

    /// Папка данных приложения (родитель файла БД) — где лежат файлы обложек.
    fn app_data_dir(&self) -> &Path {
        self.db_path.parent().unwrap_or_else(|| Path::new("."))
    }

    /// Сохранить пользовательское переопределение метаданных трека.
    /// Переопределения переживают пересканирование и сбрасываются только
    /// при удалении папки трека из скоупа приложения.
    /// `keep_existing_cover = true` означает «не трогать обложку и accent»,
    /// иначе оба поля заменяются переданными значениями (None → очистка).
    pub fn set_track_override(
        &self,
        file_path: &str,
        title: Option<&str>,
        artist: Option<&str>,
        cover_data: Option<&str>,
        accent_color: Option<&str>,
        keep_existing_cover: bool,
    ) -> Result<(), String> {
        // Меняются title/artist → индекс поиска устарел.
        SEARCH_INDEX_DIRTY.store(true, Ordering::SeqCst);
        // Нормализуем пустые строки в None — пустое поле = «использовать оригинал».
        let title = title.and_then(|s| if s.trim().is_empty() { None } else { Some(s) });
        let artist = artist.and_then(|s| if s.trim().is_empty() { None } else { Some(s) });

        // Текущая сигнатура обложки (для очистки осиротевшего файла) и accent.
        let existing: Option<(Option<String>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT cover_sig, accent_color FROM track_overrides WHERE file_path = ?1",
                params![file_path],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .ok();
        let old_sig = existing.as_ref().and_then(|(s, _)| s.clone());

        // Новую обложку (data-URL) пишем файлом в track_covers/, в БД кладём
        // только её сигнатуру. При keep_existing_cover оставляем как есть.
        let (final_sig, final_accent) = if keep_existing_cover {
            (old_sig.clone(), existing.and_then(|(_, a)| a))
        } else {
            match cover_data {
                Some(data_url) => {
                    let sig = crate::thumbnails::override_sig(data_url);
                    if let Some(bytes) = crate::thumbnails::decode_data_url(data_url) {
                        crate::track_covers::write_cover(self.app_data_dir(), &sig, &bytes)?;
                        (Some(sig), accent_color.map(|s| s.to_string()))
                    } else {
                        return Err("Не удалось декодировать обложку".into());
                    }
                }
                None => (None, accent_color.map(|s| s.to_string())),
            }
        };

        // Чистим старый файл обложки, если она сменилась/сброшена и больше ни
        // на один трек не ссылается (обложки дедуплицированы по сигнатуре).
        if old_sig != final_sig {
            if let Some(ref sig) = old_sig {
                self.remove_orphan_cover_file(sig, file_path);
            }
        }

        // Если все поля пусты — удаляем запись, чтобы LEFT JOIN отдал оригинал
        if title.is_none() && artist.is_none() && final_sig.is_none() && final_accent.is_none() {
            self.conn
                .execute(
                    "DELETE FROM track_overrides WHERE file_path = ?1",
                    params![file_path],
                )
                .map_err(|e| format!("Ошибка удаления переопределения: {}", e))?;
            return Ok(());
        }

        self.conn
            .execute(
                "INSERT INTO track_overrides (file_path, title, artist, cover_sig, accent_color)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(file_path) DO UPDATE SET
                    title = excluded.title,
                    artist = excluded.artist,
                    cover_sig = excluded.cover_sig,
                    accent_color = excluded.accent_color",
                params![file_path, title, artist, final_sig, final_accent],
            )
            .map_err(|e| format!("Ошибка сохранения переопределения: {}", e))?;
        Ok(())
    }

    /// Удалить файл обложки `sig`, если он больше не используется ни одним
    /// треком (кроме `excluding_path`, чью запись мы как раз меняем/удаляем).
    fn remove_orphan_cover_file(&self, sig: &str, excluding_path: &str) {
        let still_used: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM track_overrides
                 WHERE cover_sig = ?1 AND file_path != ?2",
                params![sig, excluding_path],
                |r| r.get(0),
            )
            .unwrap_or(1);
        if still_used == 0 {
            crate::track_covers::remove_cover(self.app_data_dir(), sig);
        }
    }

    /// Полностью удалить переопределение для трека (вернуть оригинальные значения)
    pub fn clear_track_override(&self, file_path: &str) -> Result<(), String> {
        SEARCH_INDEX_DIRTY.store(true, Ordering::SeqCst);
        // Сначала подчищаем осиротевший файл обложки (если он больше ничей).
        if let Some(sig) = self.get_override_cover_sig(file_path) {
            self.remove_orphan_cover_file(&sig, file_path);
        }
        self.conn
            .execute(
                "DELETE FROM track_overrides WHERE file_path = ?1",
                params![file_path],
            )
            .map_err(|e| format!("Ошибка удаления переопределения: {}", e))?;
        Ok(())
    }

    /// Удалить ВСЕ переопределения треков. Возвращает количество сброшенных треков.
    /// Также сбрасывает кэшированный accent_color, чтобы следующее воспроизведение
    /// пересчитало его из оригинальной обложки файла.
    pub fn clear_all_track_overrides(&self) -> Result<usize, String> {
        SEARCH_INDEX_DIRTY.store(true, Ordering::SeqCst);
        let count = self
            .conn
            .execute("DELETE FROM track_overrides", [])
            .map_err(|e| format!("Ошибка сброса переопределений: {}", e))?;
        let _ = self
            .conn
            .execute("UPDATE tracks SET accent_color = NULL", []);
        // Сносим папку с файлами кастомных обложек целиком — все записи удалены.
        let _ = std::fs::remove_dir_all(crate::track_covers::covers_dir(self.app_data_dir()));
        Ok(count)
    }

    /// Проверить, есть ли пользовательская обложка у трека.
    pub fn has_override_cover(&self, file_path: &str) -> bool {
        self.conn
            .query_row(
                "SELECT cover_sig IS NOT NULL FROM track_overrides WHERE file_path = ?1",
                params![file_path],
                |row| row.get::<_, i64>(0),
            )
            .map(|v| v != 0)
            .unwrap_or(false)
    }

    /// Получить кэшированный accent_color для трека.
    /// Если у трека есть пользовательское переопределение обложки — возвращаем
    /// его accent, иначе фолбэчимся на тот, что вычислен из оригинальной обложки.
    pub fn get_accent_color(&self, file_path: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT COALESCE(o.accent_color, t.accent_color)
                 FROM tracks t
                 LEFT JOIN track_overrides o ON o.file_path = t.file_path
                 WHERE t.file_path = ?1",
                params![file_path],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
    }

    /// Сигнатура пользовательской обложки (если установлена). Это короткая строка
    /// — ключ файла в `track_covers/` и параметр `?s=` в URL миниатюры.
    pub fn get_override_cover_sig(&self, file_path: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT cover_sig FROM track_overrides WHERE file_path = ?1",
                params![file_path],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
    }

    /// Сырые байты пользовательской обложки (читаются с диска лениво — только
    /// когда реально нужны: генерация миниатюры, расчёт акцентного цвета).
    pub fn override_cover_bytes(&self, file_path: &str) -> Option<Vec<u8>> {
        let sig = self.get_override_cover_sig(file_path)?;
        crate::track_covers::read_cover(self.app_data_dir(), &sig)
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

    pub fn get_recently_played(&self, limit: usize) -> Vec<TrackMetadata> {
        let sql = "SELECT t.file_path,
                          COALESCE(o.title, t.title),
                          COALESCE(o.artist, t.artist),
                          t.album, t.genre, t.duration_secs,
                          t.format, t.sample_rate, t.bit_depth, t.channels, t.bitrate, t.is_lossless,
                          COALESCE(o.accent_color, t.accent_color),
                          o.cover_sig,
                          af.bpm, af.loudness, af.energy, af.danceability,
                          af.spectral_centroid, af.track_key, af.audio_fingerprint
                   FROM tracks t
                   JOIN track_statistics s ON s.file_path = t.file_path
                   LEFT JOIN track_overrides o ON o.file_path = t.file_path
                   LEFT JOIN track_audio_features af ON af.file_path = t.file_path
                   WHERE s.updated_at > 0
                   ORDER BY s.updated_at DESC
                   LIMIT ?1";
        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params![limit as i64], |row| Self::row_to_metadata_with_override(row)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn get_most_played(&self, limit: usize) -> Vec<TrackMetadata> {
        let sql = "SELECT t.file_path,
                          COALESCE(o.title, t.title),
                          COALESCE(o.artist, t.artist),
                          t.album, t.genre, t.duration_secs,
                          t.format, t.sample_rate, t.bit_depth, t.channels, t.bitrate, t.is_lossless,
                          COALESCE(o.accent_color, t.accent_color),
                          o.cover_sig,
                          af.bpm, af.loudness, af.energy, af.danceability,
                          af.spectral_centroid, af.track_key, af.audio_fingerprint
                   FROM tracks t
                   JOIN track_statistics s ON s.file_path = t.file_path
                   LEFT JOIN track_overrides o ON o.file_path = t.file_path
                   LEFT JOIN track_audio_features af ON af.file_path = t.file_path
                   WHERE s.listen_count > 0
                   ORDER BY s.listen_count DESC, s.total_listened_secs DESC
                   LIMIT ?1";
        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params![limit as i64], |row| Self::row_to_metadata_with_override(row)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn get_top_artists(&self, limit: usize) -> Vec<TopArtist> {
        // Группируем по точному написанию в SQL, затем сливаем регистронезависимо
        // в Rust, чтобы «MZLFF» и «mzlff» были одним артистом в статистике.
        let sql = "SELECT COALESCE(o.artist, t.artist) AS artist_name,
                          SUM(s.listen_count) AS total_listens,
                          SUM(s.total_listened_secs) AS total_secs,
                          COUNT(t.file_path) AS tracks_cnt
                   FROM tracks t
                   JOIN track_statistics s ON s.file_path = t.file_path
                   LEFT JOIN track_overrides o ON o.file_path = t.file_path
                   WHERE COALESCE(o.artist, t.artist) IS NOT NULL AND COALESCE(o.artist, t.artist) != ''
                   GROUP BY artist_name";
        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, i64>(3)? as usize,
            ))
        }) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        // ключ (нижний регистр) -> (варианты написания с числом треков, агрегаты)
        struct Agg {
            variants: HashMap<String, usize>,
            listen_count: i64,
            total_listened_secs: f64,
            track_count: usize,
        }
        let mut map: HashMap<String, Agg> = HashMap::new();
        for (name, listens, secs, tracks) in rows.flatten() {
            let entry = map.entry(name.to_lowercase()).or_insert_with(|| Agg {
                variants: HashMap::new(),
                listen_count: 0,
                total_listened_secs: 0.0,
                track_count: 0,
            });
            *entry.variants.entry(name).or_insert(0) += tracks;
            entry.listen_count += listens;
            entry.total_listened_secs += secs;
            entry.track_count += tracks;
        }

        let mut result: Vec<TopArtist> = map
            .into_values()
            .map(|a| {
                let name = a
                    .variants
                    .iter()
                    .max_by(|x, y| x.1.cmp(y.1).then(y.0.cmp(x.0)))
                    .map(|(n, _)| n.clone())
                    .unwrap_or_default();
                TopArtist {
                    name,
                    listen_count: a.listen_count,
                    total_listened_secs: a.total_listened_secs,
                    track_count: a.track_count,
                }
            })
            .collect();
        result.sort_by(|a, b| {
            b.listen_count
                .cmp(&a.listen_count)
                .then(b.total_listened_secs.partial_cmp(&a.total_listened_secs).unwrap_or(std::cmp::Ordering::Equal))
        });
        result.truncate(limit);
        result
    }

    pub fn get_top_albums(&self, limit: usize) -> Vec<TopAlbum> {
        let sql = "SELECT t.album AS album_name,
                          COALESCE(o.artist, t.artist) AS artist_name,
                          SUM(s.listen_count) AS total_listens,
                          SUM(s.total_listened_secs) AS total_secs,
                          MIN(t.file_path) AS first_path
                   FROM tracks t
                   JOIN track_statistics s ON s.file_path = t.file_path
                   LEFT JOIN track_overrides o ON o.file_path = t.file_path
                   WHERE t.album IS NOT NULL AND t.album != ''
                   GROUP BY album_name, artist_name
                   ORDER BY total_listens DESC, total_secs DESC
                   LIMIT ?1";
        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params![limit as i64], |row| {
            Ok(TopAlbum {
                name: row.get(0)?,
                artist: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                listen_count: row.get(2)?,
                total_listened_secs: row.get(3)?,
                first_file_path: row.get(4)?,
            })
        }) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn get_summary_stats(&self) -> SummaryStats {
        let total_tracks: usize = self.conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))
            .unwrap_or(0);
        let total_folders: usize = self.conn
            .query_row("SELECT COUNT(*) FROM folders", [], |r| r.get(0))
            .unwrap_or(0);
        let (total_listened_secs, total_listens, morning, day, evening, night) = self.conn
            .query_row(
                "SELECT COALESCE(SUM(total_listened_secs), 0),
                        COALESCE(SUM(listen_count), 0),
                        COALESCE(SUM(morning_listen_count), 0),
                        COALESCE(SUM(day_listen_count), 0),
                        COALESCE(SUM(evening_listen_count), 0),
                        COALESCE(SUM(night_listen_count), 0)
                 FROM track_statistics",
                [],
                |row| {
                    Ok((
                        row.get::<_, f64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .unwrap_or((0.0, 0, 0, 0, 0, 0));

        SummaryStats {
            total_tracks,
            total_folders,
            total_listened_secs,
            total_listens,
            morning_listens: morning,
            day_listens: day,
            evening_listens: evening,
            night_listens: night,
        }
    }

    /// Средний «характер» прослушанной музыки: взвешенные (по числу
    /// прослушиваний) средние и дисперсии аудио-фич. Используется, чтобы
    /// подбирать похожие («любимое») либо непохожие («незнакомое») треки.
    ///
    /// Берётся из кэша накопителей (см. [`MusicLibrary::centroid_accumulators`]),
    /// поэтому повторные вызовы при выдаче рекомендаций не пересчитывают всю
    /// историю заново.
    fn listening_centroid(&self) -> Option<ListeningCentroid> {
        self.centroid_accumulators().to_centroid()
    }

    /// Накопители центроида: либо из кэша, либо однократный O(N) пересчёт по БД.
    /// Если межпоточный флаг отметил историю как изменившуюся (анализ фич,
    /// ре-скан) — кэш сбрасывается и считается заново.
    fn centroid_accumulators(&self) -> CentroidAccumulators {
        if CENTROID_CACHE_DIRTY.swap(false, Ordering::SeqCst) {
            self.centroid_cache.borrow_mut().take();
        }
        if let Some(acc) = self.centroid_cache.borrow().as_ref() {
            return acc.clone();
        }
        let acc = self.compute_centroid_accumulators();
        *self.centroid_cache.borrow_mut() = Some(acc.clone());
        acc
    }

    /// Полный пересчёт накопителей по всей истории прослушиваний.
    fn compute_centroid_accumulators(&self) -> CentroidAccumulators {
        let mut acc = CentroidAccumulators::default();
        let mut stmt = match self.conn.prepare(
            "SELECT af.energy, af.danceability, af.bpm, af.spectral_centroid, af.loudness, s.listen_count
             FROM track_statistics s
             JOIN track_audio_features af ON af.file_path = s.file_path
             WHERE s.listen_count > 0 AND af.energy IS NOT NULL",
        ) {
            Ok(s) => s,
            Err(_) => return acc,
        };
        let rows = match stmt.query_map([], |row| {
            Ok((
                row.get::<_, Option<f64>>(0)?,
                row.get::<_, Option<f64>>(1)?,
                row.get::<_, Option<f64>>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        }) {
            Ok(r) => r,
            Err(_) => return acc,
        };

        for r in rows.flatten() {
            let (e, d, b, c, l) = match r {
                (Some(e), Some(d), Some(b), Some(c), Some(l), _cnt) => (e, d, b, c, l),
                _ => continue,
            };
            acc.apply(r.5.max(1) as f64, e, d, b, c, l, 1);
        }
        acc
    }

    /// Инкрементально обновить кэш центроида при изменении числа прослушиваний
    /// одного трека. Вызывается из [`MusicLibrary::add_track_statistics_delta`]
    /// в момент завершения воспроизведения.
    fn update_centroid_for_listen(&self, file_path: &str, old_count: i64, new_count: i64) {
        // Кэш ещё не загружен — пропускаем, он лениво пересчитается целиком.
        if self.centroid_cache.borrow().is_none() {
            return;
        }

        // Вес трека = его число прослушиваний (треки без прослушиваний не входят).
        let old_w = if old_count > 0 { old_count.max(1) as f64 } else { 0.0 };
        let new_w = if new_count > 0 { new_count.max(1) as f64 } else { 0.0 };
        let dw = new_w - old_w;
        if dw == 0.0 {
            return;
        }

        // Фичи трека должны быть полностью посчитаны — иначе он не участвует в
        // центроиде (полный пересчёт такие треки пропускает).
        let feats: Option<(f64, f64, f64, f64, f64)> = self
            .conn
            .query_row(
                "SELECT energy, danceability, bpm, spectral_centroid, loudness
                 FROM track_audio_features
                 WHERE file_path = ?1
                   AND energy IS NOT NULL AND danceability IS NOT NULL AND bpm IS NOT NULL
                   AND spectral_centroid IS NOT NULL AND loudness IS NOT NULL",
                params![file_path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .ok();
        let (e, d, b, c, l) = match feats {
            Some(f) => f,
            None => return,
        };

        // Изменение числа уникальных треков в центроиде.
        let dn = if old_count <= 0 && new_count > 0 {
            1
        } else if old_count > 0 && new_count <= 0 {
            -1
        } else {
            0
        };

        if let Some(acc) = self.centroid_cache.borrow_mut().as_mut() {
            acc.apply(dw, e, d, b, c, l, dn);
        }
    }

    /// Публичный профиль вкуса для раздела статистики.
    pub fn taste_profile(&self) -> TasteProfile {
        match self.listening_centroid() {
            Some(c) => TasteProfile {
                analyzed_tracks: c.n,
                energy: Some(c.e),
                danceability: Some(c.d),
                bpm: Some(c.b),
                spectral_centroid: Some(c.c),
                loudness: Some(c.l),
            },
            None => TasteProfile {
                analyzed_tracks: 0,
                energy: None,
                danceability: None,
                bpm: None,
                spectral_centroid: None,
                loudness: None,
            },
        }
    }

    pub fn get_recommendations_by_mood(
        &self,
        mood: &str,
        familiarity: &str,
        lossless_only: bool,
        limit: usize,
    ) -> Vec<TrackMetadata> {
        let mut clauses: Vec<&str> = Vec::new();

        // Настроение — по аудио-фичам трека.
        match mood {
            "energetic" => clauses.push("af.energy >= 0.55 AND af.danceability >= 0.45"),
            "relax" => clauses.push("af.energy < 0.4 AND (af.bpm < 100 OR af.bpm IS NULL)"),
            "dance" => clauses.push("af.danceability >= 0.65"),
            "melancholic" => clauses.push("af.spectral_centroid < 2000.0 AND af.energy < 0.4"),
            "forgotten" => clauses.push("(s.listen_count IS NULL OR s.listen_count = 0)"),
            _ => {}
        }

        if lossless_only {
            clauses.push("t.is_lossless = 1");
        }

        let where_clause = if clauses.is_empty() {
            "1=1".to_string()
        } else {
            clauses.join(" AND ")
        };

        // Знакомость теперь определяется по «характеру» прослушанного:
        //   favorite — близкие к среднему профилю треки,
        //   fresh    — наиболее далёкие от него,
        //   balanced — без учёта характера (случайно).
        let centroid = match familiarity {
            "favorite" | "fresh" => self.listening_centroid(),
            _ => None,
        };

        // ORDER BY: для favorite/fresh — по нормированному расстоянию до центроида
        // (с лёгким джиттером, чтобы поток обновлялся), иначе — случайно.
        // Литералы оборачиваем в скобки, чтобы отрицательные значения (loudness)
        // не превращались в SQL-комментарий «--».
        let order_by = match centroid {
            Some(ct) => {
                let ke = 1.0 / ct.e_var.max(1e-4);
                let kd = 1.0 / ct.d_var.max(1e-4);
                let kb = 1.0 / ct.b_var.max(1.0);
                let kc = 1.0 / ct.c_var.max(1.0);
                let kl = 1.0 / ct.l_var.max(1e-2);
                let dist = format!(
                    "((COALESCE(af.energy,({e}))-({e}))*(COALESCE(af.energy,({e}))-({e}))*({ke}) \
                     + (COALESCE(af.danceability,({d}))-({d}))*(COALESCE(af.danceability,({d}))-({d}))*({kd}) \
                     + (COALESCE(af.bpm,({b}))-({b}))*(COALESCE(af.bpm,({b}))-({b}))*({kb}) \
                     + (COALESCE(af.spectral_centroid,({c}))-({c}))*(COALESCE(af.spectral_centroid,({c}))-({c}))*({kc}) \
                     + (COALESCE(af.loudness,({l}))-({l}))*(COALESCE(af.loudness,({l}))-({l}))*({kl}))",
                    e = ct.e, d = ct.d, b = ct.b, c = ct.c, l = ct.l,
                    ke = ke, kd = kd, kb = kb, kc = kc, kl = kl,
                );
                let jitter = "(0.5 + (ABS(RANDOM()) % 1000) / 1000.0)";
                let dir = if familiarity == "favorite" { "ASC" } else { "DESC" };
                // Треки без аудио-фич уходят в конец, оставшийся объём добивается ими.
                format!("(af.energy IS NULL) ASC, {dist} * {jitter} {dir}")
            }
            None => "RANDOM()".to_string(),
        };

        let sql = format!(
            "SELECT t.file_path,
                     COALESCE(o.title, t.title),
                     COALESCE(o.artist, t.artist),
                     t.album, t.genre, t.duration_secs,
                     t.format, t.sample_rate, t.bit_depth, t.channels, t.bitrate, t.is_lossless,
                     COALESCE(o.accent_color, t.accent_color),
                     o.cover_sig,
                     af.bpm, af.loudness, af.energy, af.danceability,
                     af.spectral_centroid, af.track_key, af.audio_fingerprint
              FROM tracks t
              LEFT JOIN track_overrides o ON o.file_path = t.file_path
              LEFT JOIN track_audio_features af ON af.file_path = t.file_path
              LEFT JOIN track_statistics s ON s.file_path = t.file_path
              WHERE {}
              ORDER BY {}
              LIMIT ?1",
            where_clause, order_by
        );

        let mut stmt = match self.conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params![limit as i64], |row| Self::row_to_metadata_with_override(row)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    // ── Порядок треков в альбоме ────────────────────────────────────────────────

    pub fn get_album_track_order(&self, album_name: &str) -> Result<Option<Vec<String>>, String> {
        let result: Result<String, rusqlite::Error> = self.conn.query_row(
            "SELECT track_paths FROM album_track_order WHERE album_name = ?1",
            params![album_name],
            |row| row.get(0),
        );
        match result {
            Ok(json) => {
                let paths: Vec<String> = serde_json::from_str(&json)
                    .map_err(|e| format!("Ошибка разбора порядка треков: {}", e))?;
                Ok(if paths.is_empty() { None } else { Some(paths) })
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("Ошибка чтения порядка треков: {}", e)),
        }
    }

    pub fn set_album_track_order(&self, album_name: &str, paths: &[String]) -> Result<(), String> {
        let json = serde_json::to_string(paths)
            .map_err(|e| format!("Ошибка сериализации порядка треков: {}", e))?;
        self.conn
            .execute(
                "INSERT INTO album_track_order (album_name, track_paths) VALUES (?1, ?2)
                 ON CONFLICT(album_name) DO UPDATE SET track_paths = excluded.track_paths",
                params![album_name, json],
            )
            .map_err(|e| format!("Ошибка сохранения порядка треков: {}", e))?;
        Ok(())
    }

    // ── Внешние источники: треки альбома ────────────────────────────────────────

    /// Локальные треки альбома для сопоставления с внешним списком:
    /// (file_path, эффективное название, эффективный исполнитель). Эффективные
    /// значения учитывают пользовательские переопределения (track_overrides).
    pub fn get_album_local_tracks(&self, album_name: &str) -> Vec<(String, String, String)> {
        let mut stmt = match self.conn.prepare(
            "SELECT t.file_path,
                    COALESCE(NULLIF(o.title, ''), t.title, ''),
                    COALESCE(NULLIF(o.artist, ''), t.artist, '')
             FROM tracks t
             LEFT JOIN track_overrides o ON o.file_path = t.file_path
             WHERE t.album = ?1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map(params![album_name], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        });
        match rows {
            Ok(it) => it.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Прочитать кэш внешнего альбома: (source, status, fetched_secs, треки).
    pub fn get_external_album(
        &self,
        album_key: &str,
    ) -> Option<(Option<String>, String, u64, Vec<ExternalTrack>)> {
        let result: Result<(Option<String>, String, i64, String), rusqlite::Error> =
            self.conn.query_row(
                "SELECT source, status, fetched_secs, payload FROM external_album WHERE album_key = ?1",
                params![album_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            );
        match result {
            Ok((source, status, fetched, payload)) => {
                let tracks: Vec<ExternalTrack> =
                    serde_json::from_str(&payload).unwrap_or_default();
                Some((source, status, fetched.max(0) as u64, tracks))
            }
            Err(_) => None,
        }
    }

    /// Сохранить кэш внешнего альбома.
    pub fn set_external_album(
        &self,
        album_key: &str,
        source: Option<&str>,
        status: &str,
        tracks: &[ExternalTrack],
        fetched_secs: u64,
    ) -> Result<(), String> {
        let payload = serde_json::to_string(tracks)
            .map_err(|e| format!("Ошибка сериализации внешнего альбома: {}", e))?;
        self.conn
            .execute(
                "INSERT INTO external_album (album_key, source, status, fetched_secs, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(album_key) DO UPDATE SET
                     source = excluded.source,
                     status = excluded.status,
                     fetched_secs = excluded.fetched_secs,
                     payload = excluded.payload",
                params![album_key, source, status, fetched_secs as i64, payload],
            )
            .map_err(|e| format!("Ошибка сохранения внешнего альбома: {}", e))?;
        Ok(())
    }

    /// Пути треков с пользовательской (override) обложкой — для батч-прохода
    /// внешних обложек: такие треки локально покрыты, внешняя не нужна.
    pub fn override_cover_paths(&self) -> std::collections::HashSet<String> {
        let mut stmt = match self
            .conn
            .prepare("SELECT file_path FROM track_overrides WHERE cover_sig IS NOT NULL AND cover_sig != ''")
        {
            Ok(s) => s,
            Err(_) => return std::collections::HashSet::new(),
        };
        let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
            Ok(r) => r,
            Err(_) => return std::collections::HashSet::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    /// Альбомы, для которых ещё нет окончательного внешнего макета (ok/none).
    /// Для каждого берётся самый частый исполнитель — так же, как при запросе
    /// макета, — чтобы ключ кэша совпадал. Кэш бессрочный, поэтому уже
    /// обработанные альбомы в сеть не ходят.
    pub fn pending_external_album_names(&self) -> Vec<String> {
        // Самый частый исполнитель на альбом (с учётом override).
        let mut best_artist: std::collections::HashMap<String, (String, i64)> =
            std::collections::HashMap::new();
        if let Ok(mut stmt) = self.conn.prepare(
            "SELECT t.album, COALESCE(NULLIF(o.artist,''), t.artist, ''), COUNT(*)
             FROM tracks t
             LEFT JOIN track_overrides o ON o.file_path = t.file_path
             WHERE t.album IS NOT NULL AND t.album != ''
             GROUP BY t.album, COALESCE(NULLIF(o.artist,''), t.artist, '')",
        ) {
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            });
            if let Ok(rows) = rows {
                for (album, artist, cnt) in rows.filter_map(|r| r.ok()) {
                    let entry = best_artist.entry(album).or_insert((artist.clone(), 0));
                    if cnt > entry.1 {
                        *entry = (artist, cnt);
                    }
                }
            }
        }

        // Уже обработанные ключи (ok/none) — пропускаем.
        let mut settled: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Ok(mut stmt) = self
            .conn
            .prepare("SELECT album_key FROM external_album WHERE status IN ('ok','none')")
        {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
                settled = rows.filter_map(|r| r.ok()).collect();
            }
        }

        best_artist
            .into_iter()
            .filter(|(album, (artist, _))| {
                let key = crate::external::album_cache_key(album, artist);
                !settled.contains(&key)
            })
            .map(|(album, _)| album)
            .collect()
    }
}

/// Накопители для инкрементального расчёта центроида вкусов: взвешенные суммы
/// и суммы квадратов аудио-фич (вес = число прослушиваний). Из них один раз
/// выводятся средние и дисперсии, см. [`CentroidAccumulators::to_centroid`].
#[derive(Clone, Default)]
struct CentroidAccumulators {
    /// Число уникальных треков в центроиде.
    n: i64,
    /// Сумма весов (Σ listen_count).
    wsum: f64,
    se: f64, se2: f64,
    sd: f64, sd2: f64,
    sb: f64, sb2: f64,
    sc: f64, sc2: f64,
    sl: f64, sl2: f64,
}

impl CentroidAccumulators {
    /// Добавить (или, при отрицательном весе/`dn`, убрать) вклад трека.
    #[allow(clippy::too_many_arguments)]
    fn apply(&mut self, w: f64, e: f64, d: f64, b: f64, c: f64, l: f64, dn: i64) {
        self.n += dn;
        self.wsum += w;
        self.se += w * e; self.se2 += w * e * e;
        self.sd += w * d; self.sd2 += w * d * d;
        self.sb += w * b; self.sb2 += w * b * b;
        self.sc += w * c; self.sc2 += w * c * c;
        self.sl += w * l; self.sl2 += w * l * l;
    }

    fn to_centroid(&self) -> Option<ListeningCentroid> {
        if self.wsum <= 0.0 {
            return None;
        }
        let mean = |s: f64| s / self.wsum;
        let var = |s2: f64, m: f64| (s2 / self.wsum - m * m).max(0.0);
        let (e, d, b, c, l) = (
            mean(self.se), mean(self.sd), mean(self.sb), mean(self.sc), mean(self.sl),
        );
        Some(ListeningCentroid {
            n: self.n.max(0) as usize,
            e, e_var: var(self.se2, e),
            d, d_var: var(self.sd2, d),
            b, b_var: var(self.sb2, b),
            c, c_var: var(self.sc2, c),
            l, l_var: var(self.sl2, l),
        })
    }
}

/// Взвешенный средний профиль аудио-фич прослушанных треков (+ дисперсии).
struct ListeningCentroid {
    n: usize,
    e: f64,
    e_var: f64,
    d: f64,
    d_var: f64,
    b: f64,
    b_var: f64,
    c: f64,
    c_var: f64,
    l: f64,
    l_var: f64,
}

/// Средний «характер» прослушанной музыки — публичный профиль вкуса.
/// Показывается в статистике и используется фильтром «Любимое».
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasteProfile {
    pub analyzed_tracks: usize,
    pub energy: Option<f64>,
    pub danceability: Option<f64>,
    pub bpm: Option<f64>,
    pub spectral_centroid: Option<f64>,
    pub loudness: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_library_dir(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be valid")
            .as_nanos();
        std::env::temp_dir().join(format!("lfmusic_{test_name}_{unique}"))
    }

    #[test]
    fn resume_audio_features_does_not_emit_done_progress_when_nothing_is_pending() {
        let dir = temp_library_dir("audio_features_done");
        let library = MusicLibrary::new(dir.clone()).expect("library should initialize");
        library
            .conn
            .execute("INSERT INTO folders(path) VALUES (?1)", params!["/music"])
            .expect("folder insert should work");
        library
            .conn
            .execute(
                "INSERT INTO tracks
                 (file_path, folder_path, title, artist, album, duration_secs, format,
                  sample_rate, bit_depth, channels, bitrate, is_lossless, modified_secs, sort_order)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    "/music/done.flac",
                    "/music",
                    "Done",
                    "LFMusic",
                    "Tests",
                    180.0,
                    "flac",
                    44100,
                    16,
                    2,
                    900000,
                    1,
                    777,
                    1,
                ],
            )
            .expect("track insert should work");
        library
            .conn
            .execute(
                "INSERT INTO track_audio_features
                 (file_path, analyzed_modified_secs, analyzer_version)
                 VALUES (?1, ?2, ?3)",
                params!["/music/done.flac", 777, AUDIO_FEATURE_ANALYZER_VERSION],
            )
            .expect("features insert should work");

        let emissions = Arc::new(StdMutex::new(Vec::new()));
        let seen = Arc::clone(&emissions);
        let pending = library
            .resume_pending_audio_feature_analysis(Some(Arc::new(move |current, total| {
                seen.lock().expect("emissions lock").push((current, total));
            })))
            .expect("resume should succeed");

        assert_eq!(pending, 0);
        assert!(
            emissions.lock().expect("emissions lock").is_empty(),
            "resume should stay silent when all tracks are already analyzed"
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    /// Вставить трек с полным набором аудио-фич и числом прослушиваний.
    fn insert_listened_track(
        library: &MusicLibrary,
        path: &str,
        listen_count: i64,
        feats: (f64, f64, f64, f64, f64),
    ) {
        library
            .conn
            .execute(
                "INSERT INTO tracks
                 (file_path, folder_path, title, artist, album, duration_secs, format,
                  sample_rate, bit_depth, channels, bitrate, is_lossless, modified_secs, sort_order)
                 VALUES (?1, '/music', 'T', 'A', 'Al', 180.0, 'flac', 44100, 16, 2, 320, 0, 1, 1)",
                params![path],
            )
            .expect("track insert");
        library
            .conn
            .execute(
                "INSERT INTO track_audio_features
                 (file_path, energy, danceability, bpm, spectral_centroid, loudness,
                  analyzed_modified_secs, analyzer_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
                params![
                    path,
                    feats.0,
                    feats.1,
                    feats.2,
                    feats.3,
                    feats.4,
                    AUDIO_FEATURE_ANALYZER_VERSION
                ],
            )
            .expect("features insert");
        library
            .conn
            .execute(
                "INSERT INTO track_statistics (file_path, listen_count) VALUES (?1, ?2)",
                params![path, listen_count],
            )
            .expect("stats insert");
    }

    fn assert_profiles_close(a: &TasteProfile, b: &TasteProfile) {
        assert_eq!(a.analyzed_tracks, b.analyzed_tracks, "track count");
        let close = |x: Option<f64>, y: Option<f64>, label: &str| match (x, y) {
            (Some(x), Some(y)) => assert!(
                (x - y).abs() < 1e-6,
                "{label}: incremental {x} != full {y}"
            ),
            (None, None) => {}
            _ => panic!("{label}: one side is None"),
        };
        close(a.energy, b.energy, "energy");
        close(a.danceability, b.danceability, "danceability");
        close(a.bpm, b.bpm, "bpm");
        close(a.spectral_centroid, b.spectral_centroid, "spectral_centroid");
        close(a.loudness, b.loudness, "loudness");
    }

    #[test]
    fn centroid_cache_matches_full_recompute_after_incremental_updates() {
        let dir = temp_library_dir("centroid_cache");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        library
            .conn
            .execute("INSERT INTO folders(path) VALUES ('/music')", [])
            .expect("folder insert");

        insert_listened_track(&library, "/music/a.flac", 2, (0.8, 0.6, 128.0, 3200.0, -8.0));
        insert_listened_track(&library, "/music/b.flac", 5, (0.3, 0.4, 90.0, 1500.0, -14.0));

        // Первый вызов загружает кэш накопителей (O(N) скан).
        let _ = library.taste_profile();

        // Инкрементальные обновления через завершение прослушиваний.
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/music/a.flac".to_string(),
                listen_count_delta: 3,
                ..Default::default()
            })
            .expect("delta a");
        // Новый трек, ранее не входивший в центроид (listen_count 0 -> 1).
        insert_listened_track(&library, "/music/c.flac", 0, (0.5, 0.55, 110.0, 2400.0, -11.0));
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/music/c.flac".to_string(),
                listen_count_delta: 1,
                ..Default::default()
            })
            .expect("delta c");

        let incremental = library.taste_profile();

        // Сбрасываем кэш и считаем профиль с нуля — должен совпасть.
        library.centroid_cache.borrow_mut().take();
        let full = library.taste_profile();

        assert_eq!(incremental.analyzed_tracks, 3, "three tracks listened");
        assert_profiles_close(&incremental, &full);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }
}
