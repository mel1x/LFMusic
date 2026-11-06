use crate::audio_features::{self, AudioFeatures, AUDIO_FEATURE_ANALYZER_VERSION};
use crate::audio_player::{AudioPlayer, TrackMetadata};
use crate::external::ExternalTrack;
use crate::statistics::{
    self, CollectionKind, HistoryBuffer, LibraryEvent, LibraryEventKind, ListenEvent,
    TrackStatistics, TrackStatisticsDelta,
};
use rayon::prelude::*;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderInfo {
    pub path: String,
    pub track_count: usize,
    pub exists: bool,
}

/// Genre + track count, for catalog filter chips.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenreCount {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PlaylistKind {
    Album,
    Artist,
    User,
    /// The singleton "Мне нравится" system playlist. Accepts manual add/remove
    /// like a user playlist, but can't be renamed or deleted.
    Favorites,
}

/// Name of the built-in favorites playlist, created on first launch.
pub const FAVORITES_PLAYLIST_NAME: &str = "Мне нравится";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistInfo {
    pub id: i64,
    pub name: String,
    pub kind: PlaylistKind,
    pub track_count: usize,
    /// First track's path, used by the frontend to pick a cover.
    pub first_file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopArtist {
    pub name: String,
    pub listen_count: i64,
    pub total_listened_secs: f64,
    pub track_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopAlbum {
    pub name: String,
    pub artist: String,
    pub listen_count: i64,
    pub total_listened_secs: f64,
    pub first_file_path: Option<String>,
}

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

/// Split an artist string by a list of separators, applied in sequence; trims and drops empty pieces.
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

pub struct MusicLibrary {
    conn: Connection,
    db_path: PathBuf,
    /// Taste-centroid accumulator cache ("my mix"): lazy O(N) build once, then
    /// updated incrementally per listen (see `update_centroid_for_listen`).
    centroid_cache: RefCell<Option<CentroidAccumulators>>,
    /// In-memory catalog search index (lowercased fields), lazily rebuilt when `SEARCH_INDEX_DIRTY`.
    search_index: RefCell<Option<Vec<crate::catalog_search::SearchEntry>>>,
    /// "Already downloaded" index cache for the Downloads page (title -> local matches).
    /// Keyed by the separators it was built with since debounced search re-queries it per keystroke.
    catalog_index_cache: RefCell<Option<(Vec<String>, CatalogIndex)>>,
    /// Listening history waiting for its batched write (see `statistics::HistoryBuffer`).
    history: RefCell<HistoryBuffer>,
}

/// (file path, tags, mtime) of a file read during a scan.
type ScannedFile = (String, TrackMetadata, u64);

fn lock_library(lib: &Mutex<MusicLibrary>) -> Result<std::sync::MutexGuard<'_, MusicLibrary>, String> {
    lib.lock().map_err(|e| format!("Lock error: {}", e))
}

/// Title key -> local matches, see `catalog_index_cache`.
type CatalogIndex = Arc<HashMap<String, Vec<LibraryTrackMatch>>>;

/// One local file's identity for album-aware "already downloaded" matching on the
/// Downloads page: normalized artists + normalized album key (empty if untagged).
/// Multiple entries can share a title key to distinguish same-named tracks from
/// different albums.
#[derive(Debug, Clone)]
pub struct LibraryTrackMatch {
    pub artists: HashSet<String>,
    pub album_key: String,
}

pub type AudioFeatureProgressEmitter = Arc<dyn Fn(usize, usize) + Send + Sync + 'static>;

static AUDIO_FEATURE_WORKER_GENERATION: AtomicU64 = AtomicU64::new(0);
static AUDIO_FEATURE_WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Set by background/cross-thread ops (feature analysis, rescan) that change listen history.
static CENTROID_CACHE_DIRTY: AtomicBool = AtomicBool::new(false);
/// Set when track composition or fields change; starts `true` since the index is lazily built.
static SEARCH_INDEX_DIRTY: AtomicBool = AtomicBool::new(true);
/// Separate from SEARCH_INDEX_DIRTY so each cache's consumer can `swap(false)`
/// independently without one invalidation being silently consumed by the other.
static DOWNLOADS_INDEX_DIRTY: AtomicBool = AtomicBool::new(true);

fn mark_track_indexes_dirty() {
    for flag in [&SEARCH_INDEX_DIRTY, &DOWNLOADS_INDEX_DIRTY] {
        flag.store(true, Ordering::SeqCst);
    }
}

/// Only invalidates the catalog search index (ordered by `sort_order`). The
/// "already downloaded" index depends only on title/artist/album, not order,
/// so reordering (move-to-top etc.) must not force it to rebuild.
fn mark_search_index_dirty() {
    SEARCH_INDEX_DIRTY.store(true, Ordering::SeqCst);
}

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
                if let Some(progress) = &progress {
                    progress(completed, total);
                }
            }

            let Ok(mut conn) = Connection::open(db_path) else {
                if is_audio_feature_generation_current(generation) {
                    AUDIO_FEATURE_WORKER_ACTIVE.store(false, Ordering::SeqCst);
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
                        if let Some(progress) = &progress {
                            progress(total, total);
                        }
                    }
                    return;
                }
            };

            // Single unchunked parallel pass: rayon work-steals across the whole job
            // list, so one slow file no longer stalls every other thread the way a
            // fixed-size chunk-then-barrier loop used to (each chunk previously had to
            // fully finish, including its slowest file, before the next chunk started).
            // Results stream to this thread over a channel and are written in small
            // batches as they arrive, so progress stays live and partial work survives
            // an app close mid-run, instead of only ever writing at chunk boundaries.
            let (tx, rx) = mpsc::channel::<(String, u64, Option<AudioFeatures>)>();
            let analysis_thread = std::thread::spawn(move || {
                pool.install(|| {
                    jobs.into_par_iter().for_each_with(tx, |tx, (path, duration, mtime)| {
                        if !is_audio_feature_generation_current(generation) {
                            return;
                        }
                        let features = audio_features::analyze_file(&path, duration).ok();
                        if is_audio_feature_generation_current(generation) {
                            let _ = tx.send((path, mtime, features));
                        }
                    });
                });
            });

            const WRITE_BATCH: usize = 16;
            let mut pending: Vec<(String, u64, Option<AudioFeatures>)> =
                Vec::with_capacity(WRITE_BATCH);
            for result in rx {
                if !is_audio_feature_generation_current(generation) {
                    break;
                }
                pending.push(result);
                completed = (completed + 1).min(total);
                if pending.len() >= WRITE_BATCH {
                    flush_audio_features(&mut conn, &mut pending);
                }
                if let Some(progress) = &progress {
                    progress(completed, total);
                }
            }
            flush_audio_features(&mut conn, &mut pending);
            let _ = analysis_thread.join();

            if is_audio_feature_generation_current(generation) {
                AUDIO_FEATURE_WORKER_ACTIVE.store(false, Ordering::SeqCst);
                if let Some(progress) = &progress {
                    progress(total, total);
                }
            }
        });
}

/// Writes a batch of analysis results in one transaction and clears `pending`.
/// Used by [`spawn_audio_feature_analysis`] to commit incrementally as results
/// stream in, instead of waiting for the whole job list to finish.
fn flush_audio_features(
    conn: &mut Connection,
    pending: &mut Vec<(String, u64, Option<AudioFeatures>)>,
) {
    if pending.is_empty() {
        return;
    }
    if let Ok(tx) = conn.transaction() {
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
            for (file_path, mtime, features) in pending.iter() {
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
    }
    pending.clear();
    // New audio features may change the centroid; search/downloaded indexes
    // depend on title/artist/album/genre/order, none of which features touch.
    CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
}

/// SELECT list read by [`MusicLibrary::row_to_metadata_with_override`] over tracks `t`,
/// overrides `o` and audio features `af`. `$cover` is the cover-signature column:
/// `"o.cover_sig"`, or `"NULL"` when the caller doesn't need it.
macro_rules! track_select {
    ($cover:literal) => {
        concat!(
            "SELECT t.file_path, COALESCE(o.title, t.title), COALESCE(o.artist, t.artist), ",
            "t.album, t.genre, t.duration_secs, ",
            "t.format, t.sample_rate, t.bit_depth, t.channels, t.bitrate, t.is_lossless, ",
            "COALESCE(o.accent_color, t.accent_color), ",
            $cover,
            ", af.bpm, af.loudness, af.energy, af.danceability, ",
            "af.spectral_centroid, af.track_key, af.audio_fingerprint",
        )
    };
}

impl MusicLibrary {
    fn scan_thread_count() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1).clamp(1, 8))
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
        // Hot single-row lookups (cover protocol, accent color, overrides) go
        // through prepare_cached; the default capacity of 16 is too small.
        conn.set_prepared_statement_cache_capacity(64);

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
                 accent_color  TEXT,
                 has_cover     INTEGER
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
             -- PRIMARY KEY (playlist_id, file_path) already covers lookups by
             -- playlist_id. file_path needs its own index: the ON DELETE CASCADE
             -- from tracks and per-track playlist lookups would otherwise scan the
             -- whole table once per deleted/queried track.
             DROP INDEX IF EXISTS idx_pt_playlist;
             CREATE INDEX IF NOT EXISTS idx_pt_file ON playlist_tracks(file_path);

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

        // tracks.has_cover was added after initial release; there's no migration
        // system (see module docs), so a fresh CREATE TABLE already defines it but
        // an existing DB needs it added explicitly. Errors (column already exists)
        // are expected on every run after the first and are ignored. Existing rows
        // land as NULL ("unknown"); MusicLibrary::has_cover resolves and caches
        // those lazily instead of assuming "no cover" and silently losing art.
        let _ = conn.execute("ALTER TABLE tracks ADD COLUMN has_cover INTEGER", []);

        statistics::create_tables(&conn)
            .map_err(|e| format!("Ошибка создания таблиц статистики: {}", e))?;

        Ok(Self {
            conn,
            db_path,
            centroid_cache: RefCell::new(None),
            search_index: RefCell::new(None),
            catalog_index_cache: RefCell::new(None),
            history: RefCell::new(HistoryBuffer::default()),
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

    /// WAL stays enabled here (not disabled on shutdown): background DSP threads
    /// may still hold their own SQLite connection while the window closes.
    pub fn shutdown(&self) -> Result<(), String> {
        if let Err(e) = self.flush_history(true) {
            eprintln!("{}", e);
        }
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

    /// Fast first pass: collects audio file paths without reading metadata.
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

    fn list_audio_files(dir: &Path) -> Vec<PathBuf> {
        let supported: HashSet<&str> = AudioPlayer::supported_formats().iter().copied().collect();
        let mut files = Vec::new();
        Self::collect_audio_files(dir, &supported, 0, &mut files);
        files
    }

    /// Scans a folder; `progress(current, total)` is called per processed file.
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

        let all_files = Self::list_audio_files(&path);
        self.scan_folder_files(
            folder,
            &all_files,
            analyze_audio_features,
            audio_feature_progress,
            progress,
        )
    }

    pub fn get_folders(&self) -> Vec<FolderInfo> {
        self.get_folders_inner(true)
    }

    /// Skips filesystem existence checks, for frequent calls.
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
        CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        mark_track_indexes_dirty();
        Ok(self.get_all_tracks_light())
    }

    /// [`Self::add_folder`] that holds `lib` only for DB reads/writes: listing
    /// files and reading tags (the slow part) runs unlocked, so other commands
    /// (and the main thread waiting on them) aren't blocked during the scan.
    pub fn add_folder_unlocked<F>(
        lib: &Mutex<MusicLibrary>,
        folder: String,
        artist_separators: &[String],
        audio_feature_progress: Option<AudioFeatureProgressEmitter>,
        progress: F,
    ) -> Result<(), String>
    where
        F: Fn(usize, usize) + Send + Sync,
    {
        let path = PathBuf::from(&folder);
        if !path.is_dir() {
            return Err(format!("Папка не существует: {}", folder));
        }
        let cached_mtimes = {
            let library = lock_library(lib)?;
            library
                .conn
                .execute("INSERT OR IGNORE INTO folders (path) VALUES (?1)", params![folder])
                .map_err(|e| format!("Ошибка добавления папки: {}", e))?;
            library.folder_mtimes(&folder)?
        };

        let files = Self::list_audio_files(&path);
        let batch = Self::read_changed_files(&files, &cached_mtimes, &progress)?;

        let mut library = lock_library(lib)?;
        library.write_scanned(&folder, &batch, true, audio_feature_progress)?;
        library.rebuild_album_playlists()?;
        library.rebuild_artist_playlists(artist_separators)?;
        CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        mark_track_indexes_dirty();
        Ok(())
    }

    /// [`Self::rescan_all`] with the same locking scheme as [`Self::add_folder_unlocked`].
    pub fn rescan_all_unlocked<F>(
        lib: &Mutex<MusicLibrary>,
        artist_separators: &[String],
        audio_feature_progress: Option<AudioFeatureProgressEmitter>,
        progress: F,
    ) -> Result<(), String>
    where
        F: Fn(usize, usize) + Send + Sync,
    {
        let folders: Vec<(String, HashMap<String, u64>)> = {
            let library = lock_library(lib)?;
            library
                .folder_paths()?
                .into_iter()
                .map(|f| {
                    let mtimes = library.folder_mtimes(&f)?;
                    Ok((f, mtimes))
                })
                .collect::<Result<_, String>>()?
        };

        let listed: Vec<(String, HashMap<String, u64>, Vec<PathBuf>)> = folders
            .into_iter()
            .filter(|(f, _)| Path::new(f).is_dir())
            .map(|(f, mtimes)| {
                let files = Self::list_audio_files(Path::new(&f));
                (f, mtimes, files)
            })
            .collect();
        let grand_total: usize = listed.iter().map(|(_, _, files)| files.len()).sum();

        let mut offset = 0usize;
        let mut scanned: Vec<(String, Vec<ScannedFile>)> = Vec::new();
        for (folder, mtimes, files) in &listed {
            let batch = Self::read_changed_files(files, mtimes, &|i, _| progress(offset + i, grand_total))?;
            offset += files.len();
            scanned.push((folder.clone(), batch));
        }

        let mut library = lock_library(lib)?;
        let mut scanned_count = 0usize;
        for (folder, batch) in &scanned {
            scanned_count += library.write_scanned(folder, batch, false, None)?;
        }
        let deleted_count = library.delete_missing_tracks()?;
        library.rebuild_album_playlists()?;
        library.rebuild_artist_playlists(artist_separators)?;
        if scanned_count > 0 || deleted_count > 0 {
            let _ = library.restart_pending_audio_feature_analysis(audio_feature_progress);
        }
        CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        mark_track_indexes_dirty();
        Ok(())
    }

    fn folder_paths(&self) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM folders")
            .map_err(|e| format!("Ошибка чтения папок: {}", e))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Ошибка запроса: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Registered library folder that equals or contains `dir`, compared via
    /// canonicalized paths (case/separator-safe on Windows). Returns the stored
    /// path of the covering folder, so a downloads folder nested inside an
    /// already-added folder isn't registered (and scanned) twice.
    pub fn covering_folder(&self, dir: &str) -> Option<String> {
        let target = std::fs::canonicalize(dir).ok()?;
        let folders: Vec<String> = {
            let mut stmt = self.conn.prepare("SELECT path FROM folders").ok()?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .ok()?;
            rows.filter_map(|r| r.ok()).collect()
        };
        for f in folders {
            if let Ok(fp) = std::fs::canonicalize(&f) {
                if target.starts_with(&fp) {
                    return Some(f);
                }
            }
        }
        None
    }

    /// Adds a single downloaded file without a full folder scan: reads only
    /// this file's metadata and UPSERTs it, so cost doesn't grow with library size.
    pub fn insert_downloaded_track(
        &mut self,
        file_path: &str,
        downloads_dir: &str,
        artist_separators: &[String],
    ) -> Result<(), String> {
        let folder_path = match self.covering_folder(downloads_dir) {
            Some(parent) => parent,
            None => {
                self.conn
                    .execute(
                        "INSERT OR IGNORE INTO folders (path) VALUES (?1)",
                        params![downloads_dir],
                    )
                    .map_err(|e| format!("Ошибка добавления папки: {}", e))?;
                downloads_dir.to_string()
            }
        };

        let m = AudioPlayer::get_metadata_light(file_path)?;
        let mtime = Self::get_mtime(&PathBuf::from(file_path));
        // Catalog sorts by sort_order DESC, so a downloaded track goes to the top;
        // re-downloads (DO UPDATE branch) also bump it, as an explicit user action.
        let new_sort_order: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(sort_order), 0) FROM tracks", [], |r| r.get(0))
            .unwrap_or(0)
            + 1;
        self.conn
            .execute(
                "INSERT INTO tracks
                 (file_path, folder_path, title, artist, album, genre, duration_secs,
                  format, sample_rate, bit_depth, channels, bitrate, is_lossless, modified_secs, sort_order,
                  has_cover)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
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
                    sort_order = excluded.sort_order,
                    accent_color = NULL,
                    has_cover = excluded.has_cover",
                params![
                    file_path,
                    folder_path,
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
                    mtime as i64,
                    new_sort_order,
                    m.has_cover as i32,
                ],
            )
            .map_err(|e| format!("Ошибка вставки трека: {}", e))?;
        self.log_library_event(LibraryEventKind::Downloaded, Some(file_path), None);

        self.rebuild_album_playlists()?;
        self.rebuild_artist_playlists(artist_separators)?;
        // Feature analysis is intentionally NOT restarted here: during batch downloads
        // that would cancel+restart the background worker per track. The caller
        // (download worker) triggers it once after the batch via resume_pending_audio_feature_analysis.
        CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        mark_track_indexes_dirty();
        Ok(())
    }

    /// Removes one track (DB row + dependent data) and rebuilds playlists. Does NOT
    /// touch the file on disk (caller's job, see `downloads::delete_downloaded_track`).
    /// FK cascade handles playlist_tracks/track_audio_features; track_overrides/
    /// track_statistics are cleaned explicitly since they have no FK.
    pub fn remove_track(
        &mut self,
        file_path: &str,
        artist_separators: &[String],
    ) -> Result<(), String> {
        // Grab the cover signature before deleting the override row, so the
        // orphaned track_covers/ file (if any) can be cleaned up after commit.
        let cover_sig = self.get_override_cover_sig(file_path);
        {
            let tx = self
                .conn
                .transaction()
                .map_err(|e| format!("Ошибка транзакции: {}", e))?;
            tx.execute(
                "DELETE FROM track_overrides WHERE file_path = ?1",
                params![file_path],
            )
            .map_err(|e| format!("Ошибка удаления переопределений: {}", e))?;
            let _ = tx.execute(
                "DELETE FROM track_statistics WHERE file_path = ?1",
                params![file_path],
            );
            tx.execute("DELETE FROM tracks WHERE file_path = ?1", params![file_path])
                .map_err(|e| format!("Ошибка удаления трека: {}", e))?;
            tx.commit().map_err(|e| format!("Ошибка коммита: {}", e))?;
        }

        if let Some(sig) = cover_sig {
            self.remove_orphan_cover_file(&sig, file_path);
        }

        // Best-effort, errors not propagated: the tracks row is already committed
        // deleted, so failing here shouldn't make the caller think removal failed
        // (that could un-quarantine a file whose DB row is gone). Stale playlists
        // self-heal on the next rebuild.
        if let Err(e) = self.rebuild_album_playlists() {
            eprintln!("[library] remove_track: не удалось перестроить альбомы: {}", e);
        }
        if let Err(e) = self.rebuild_artist_playlists(artist_separators) {
            eprintln!("[library] remove_track: не удалось перестроить исполнителей: {}", e);
        }
        CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        mark_track_indexes_dirty();
        Ok(())
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

        // Overrides are only reset when a folder leaves the app's scope entirely.
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
        CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        mark_track_indexes_dirty();
        Ok(())
    }

    pub fn cleanup_missing_tracks(&mut self) -> Result<usize, String> {
        self.delete_missing_tracks()
    }

    fn delete_missing_tracks(&mut self) -> Result<usize, String> {
        let all_paths: Vec<(String, String)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT file_path, folder_path FROM tracks")
                .map_err(|e| format!("Ошибка: {}", e))?;

            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .map_err(|e| format!("Ошибка: {}", e))?;

            rows.filter_map(|r| r.ok()).collect()
        };

        // A library folder on an unplugged/unmounted drive makes every track in it
        // look missing. Deleting them would cascade-drop statistics, playlist
        // membership and overrides, so tracks of unavailable folders are kept.
        let available_roots: HashSet<String> = all_paths
            .iter()
            .map(|(_, folder)| folder.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .filter(|folder| Path::new(folder).is_dir())
            .collect();

        // Existence checks are IO-bound; parallelized since a serial walk is
        // very slow for large libraries / network drives.
        let dead: Vec<String> = all_paths
            .into_par_iter()
            .filter(|(p, folder)| available_roots.contains(folder) && !PathBuf::from(p).exists())
            .map(|(p, _)| p)
            .collect();

        let count = dead.len();
        if !dead.is_empty() {
            // track_overrides has no FK to tracks (unlike track_audio_features/
            // playlist_tracks/track_statistics, cleaned by ON DELETE CASCADE), so it
            // must be deleted explicitly here too, same as remove_track() does.
            let cover_sigs: Vec<String> = dead
                .iter()
                .filter_map(|p| self.get_override_cover_sig(p))
                .collect();

            let tx = self
                .conn
                .transaction()
                .map_err(|e| format!("Ошибка транзакции: {}", e))?;

            const BATCH: usize = 256;
            for chunk in dead.chunks(BATCH) {
                let placeholders = (0..chunk.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let params: Vec<&dyn rusqlite::ToSql> =
                    chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
                let overrides_sql =
                    format!("DELETE FROM track_overrides WHERE file_path IN ({})", placeholders);
                tx.execute(&overrides_sql, params.as_slice()).ok();
                let tracks_sql = format!("DELETE FROM tracks WHERE file_path IN ({})", placeholders);
                tx.execute(&tracks_sql, params.as_slice()).ok();
            }

            tx.commit()
                .map_err(|e| format!("Ошибка коммита: {}", e))?;

            for sig in &cover_sigs {
                self.remove_orphan_cover_file(sig, "");
            }

            CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        mark_track_indexes_dirty();
        }

        Ok(count)
    }

    pub fn get_all_tracks(&self) -> Vec<TrackMetadata> {
        self.get_all_tracks_inner(false)
    }

    /// Skips covers, to save memory on large libraries.
    pub fn get_all_tracks_light(&self) -> Vec<TrackMetadata> {
        self.get_all_tracks_inner(true)
    }

    /// Title -> local matches ([`LibraryTrackMatch`]) index for the Downloads page:
    /// a remote track counts as "already downloaded" if the title matches (noise
    /// like `(prod. by ...)` stripped), artist sets intersect, and the album is
    /// compatible (see `downloads_search`) — so a same-named track from a
    /// different album isn't marked as owned.
    pub fn catalog_artist_title_index(
        &self,
        separators: &[String],
    ) -> CatalogIndex {
        // Cached: debounced search on the Downloads page re-queries this per keystroke.
        if DOWNLOADS_INDEX_DIRTY.swap(false, Ordering::SeqCst) {
            self.catalog_index_cache.borrow_mut().take();
        }
        if let Some((sep, idx)) = self.catalog_index_cache.borrow().as_ref() {
            if sep.as_slice() == separators {
                return Arc::clone(idx);
            }
        }
        let built = Arc::new(self.build_catalog_artist_title_index(separators));
        *self.catalog_index_cache.borrow_mut() =
            Some((separators.to_vec(), Arc::clone(&built)));
        built
    }

    /// Builds the title -> local matches index from the DB (uncached).
    fn build_catalog_artist_title_index(
        &self,
        separators: &[String],
    ) -> HashMap<String, Vec<LibraryTrackMatch>> {
        let mut index: HashMap<String, Vec<LibraryTrackMatch>> = HashMap::new();
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT COALESCE(o.title, t.title), COALESCE(o.artist, t.artist), t.album
             FROM tracks t
             LEFT JOIN track_overrides o ON o.file_path = t.file_path",
        ) else {
            return index;
        };
        // Prepared once, not per track, to avoid thousands of reallocations on large libraries.
        let prepared_seps = crate::external::prepare_artist_separators(separators);
        let rows = stmt.query_map([], |row| {
            let title: Option<String> = row.get(0)?;
            let artist: Option<String> = row.get(1)?;
            let album: Option<String> = row.get(2)?;
            Ok((
                title.unwrap_or_default(),
                artist.unwrap_or_default(),
                album.unwrap_or_default(),
            ))
        });
        if let Ok(rows) = rows {
            for (title, artist, album) in rows.flatten() {
                let tkey = crate::external::title_match_key(&title);
                if tkey.is_empty() {
                    continue;
                }
                let artists: HashSet<String> =
                    crate::external::split_artists_for_match_prepared(&artist, &prepared_seps)
                        .into_iter()
                        .collect();
                if artists.is_empty() {
                    continue;
                }
                index.entry(tkey).or_default().push(LibraryTrackMatch {
                    artists,
                    album_key: crate::external::album_match_key(&album),
                });
            }
        }
        index
    }

    fn get_all_tracks_inner(&self, light: bool) -> Vec<TrackMetadata> {
        let sql = if light {
            concat!(
                track_select!("NULL"),
                " FROM tracks t
             LEFT JOIN track_overrides o ON o.file_path = t.file_path
             LEFT JOIN track_audio_features af ON af.file_path = t.file_path
             ORDER BY
                 t.sort_order DESC,
                 CASE WHEN COALESCE(o.artist, t.artist) IS NULL THEN 1 ELSE 0 END,
                 LOWER(COALESCE(o.artist, t.artist)),
                 LOWER(COALESCE(t.album, '')),
                 LOWER(COALESCE(o.title, t.title, t.file_path))",
            )
        } else {
            concat!(
                track_select!("o.cover_sig"),
                " FROM tracks t
             LEFT JOIN track_overrides o ON o.file_path = t.file_path
             LEFT JOIN track_audio_features af ON af.file_path = t.file_path
             ORDER BY
                 t.sort_order DESC,
                 CASE WHEN COALESCE(o.artist, t.artist) IS NULL THEN 1 ELSE 0 END,
                 LOWER(COALESCE(o.artist, t.artist)),
                 LOWER(COALESCE(t.album, '')),
                 LOWER(COALESCE(o.title, t.title, t.file_path))",
            )
        };

        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map([], Self::row_to_metadata_with_override) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok()).collect()
    }

    fn row_to_metadata_with_override(row: &rusqlite::Row) -> rusqlite::Result<TrackMetadata> {
        let file_path: String = row.get(0)?;
        // Override cover is served as a WebP thumbnail via the `lfmusic-cover` protocol
        // (no base64 in the list); embedded cover loads lazily elsewhere, so None here.
        // Column 13 is the cover_sig, not raw bytes: URL is built without hashing.
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
            // Not selected here (internal-only field, see TrackMetadata::has_cover) —
            // this row mapper's output never feeds back into a tracks INSERT, so a
            // placeholder is correct; use MusicLibrary::has_cover for the real value.
            has_cover: false,
        })
    }

    /// Matches the ORDER BY in get_all_tracks_inner (canonical catalog order).
    const CATALOG_ORDER_BY: &'static str = " ORDER BY
                 t.sort_order DESC,
                 CASE WHEN COALESCE(o.artist, t.artist) IS NULL THEN 1 ELSE 0 END,
                 LOWER(COALESCE(o.artist, t.artist)),
                 LOWER(COALESCE(t.album, '')),
                 LOWER(COALESCE(o.title, t.title, t.file_path))";

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

    fn ensure_search_index(&self) {
        if SEARCH_INDEX_DIRTY.swap(false, Ordering::SeqCst) {
            self.search_index.borrow_mut().take();
        }
        if self.search_index.borrow().is_none() {
            let built = self.build_search_index();
            *self.search_index.borrow_mut() = Some(built);
        }
    }

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

    /// Direct SQL query for catalog paths in canonical order, no search.
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
        // query_map closures differ by arity/type per branch, so each collects separately.
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

    /// Non-empty search uses the in-memory ranked index; otherwise direct SQL.
    pub fn get_catalog_paths(&self, genre: Option<&str>, search: Option<&str>) -> Vec<String> {
        let search_q = search.map(|s| s.trim()).filter(|s| !s.is_empty());
        match search_q {
            Some(q) => self.ranked_paths(genre, q, None),
            None => self.catalog_paths_sql(genre),
        }
    }

    /// Result order matches input path order; missing paths are skipped.
    pub fn get_tracks_by_paths(&self, paths: &[String]) -> Vec<TrackMetadata> {
        if paths.is_empty() {
            return Vec::new();
        }
        let mut by_path: HashMap<String, TrackMetadata> = HashMap::new();
        for chunk in paths.chunks(900) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                concat!(
                    track_select!("o.cover_sig"),
                    " FROM tracks t
                 LEFT JOIN track_overrides o ON o.file_path = t.file_path
                 LEFT JOIN track_audio_features af ON af.file_path = t.file_path
                 WHERE t.file_path IN ({})",
                ),
                placeholders
            );
            let mut stmt = match self.conn.prepare(&sql) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let params_vec: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
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

    pub fn search_tracks(&self, query: &str, limit: Option<usize>) -> Vec<TrackMetadata> {
        let paths = self.ranked_paths(None, query, limit);
        self.get_tracks_by_paths(&paths)
    }

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

    pub fn get_track(&self, file_path: &str) -> Option<TrackMetadata> {
        let mut stmt = self
            .conn
            .prepare_cached(
                concat!(
                    track_select!("o.cover_sig"),
                    " FROM tracks t
                 LEFT JOIN track_overrides o ON o.file_path = t.file_path
                 LEFT JOIN track_audio_features af ON af.file_path = t.file_path
                 WHERE t.file_path = ?1",
                ),
            )
            .ok()?;
        stmt.query_row(params![file_path], Self::row_to_metadata_with_override)
            .ok()
    }

    pub fn move_track_to_top(&self, file_path: &str) -> Result<(), String> {
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
            // sort_order changed, which the search index bakes in; the "downloaded" index doesn't depend on order.
            mark_search_index_dirty();
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
        let folder_file_lists: Vec<(String, Vec<PathBuf>)> = self
            .folder_paths()?
            .into_iter()
            .filter(|f| Path::new(f).is_dir())
            .map(|f| {
                let files = Self::list_audio_files(Path::new(&f));
                (f, files)
            })
            .collect();
        let grand_total: usize = folder_file_lists.iter().map(|(_, f)| f.len()).sum();
        let mut global_current: usize = 0;

        let mut scanned_count = 0usize;
        for (folder, files) in &folder_file_lists {
            let folder_total = files.len();
            let offset = global_current;

            scanned_count += self.scan_folder_files(folder, files, false, None, |i, _| {
                progress(offset + i, grand_total);
            })?;

            global_current += folder_total;
        }

        let deleted_count = self.delete_missing_tracks()?;

        self.rebuild_album_playlists()?;
        self.rebuild_artist_playlists(artist_separators)?;

        if scanned_count > 0 || deleted_count > 0 {
            let _ = self.restart_pending_audio_feature_analysis(audio_feature_progress);
        }

        CENTROID_CACHE_DIRTY.store(true, Ordering::SeqCst);
        mark_track_indexes_dirty();
        Ok(self.get_all_tracks())
    }

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
        if files.is_empty() {
            return Ok(0);
        }
        let cached_mtimes = self.folder_mtimes(folder)?;
        let batch = Self::read_changed_files(files, &cached_mtimes, &progress)?;
        self.write_scanned(folder, &batch, analyze_audio_features, audio_feature_progress)
    }

    fn folder_mtimes(&self, folder: &str) -> Result<HashMap<String, u64>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT file_path, modified_secs FROM tracks WHERE folder_path = ?1")
            .map_err(|e| format!("Ошибка запроса: {}", e))?;
        let rows = stmt
            .query_map(params![folder], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })
            .map_err(|e| format!("Ошибка запроса: {}", e))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Reads tags of new/changed files. No DB access, so callers can run it
    /// without holding the library lock (see [`Self::add_folder_unlocked`]).
    fn read_changed_files<F>(
        files: &[PathBuf],
        cached_mtimes: &HashMap<String, u64>,
        progress: &F,
    ) -> Result<Vec<ScannedFile>, String>
    where
        F: Fn(usize, usize) + Send + Sync,
    {
        let total = files.len();
        if total == 0 {
            return Ok(Vec::new());
        }

        // mtime reads are IO-bound, parallelized; par_iter preserves file order.
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

        let cached_count = total - to_scan.len();
        if cached_count > 0 {
            progress(cached_count, total);
        }

        if to_scan.is_empty() {
            progress(total, total);
            return Ok(Vec::new());
        }

        let counter = AtomicUsize::new(cached_count);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(Self::scan_thread_count())
            .build()
            .map_err(|e| format!("scan thread pool error: {}", e))?;

        let batch: Vec<ScannedFile> = pool.install(|| {
            to_scan
                .par_iter()
                .filter_map(|(path_str, mtime)| {
                    let result = AudioPlayer::get_metadata_light(path_str).ok()?;
                    let done = counter.fetch_add(1, Ordering::Relaxed) + 1;
                    if done.is_multiple_of(10) || done == total {
                        progress(done, total);
                    }
                    Some((path_str.clone(), result, *mtime))
                })
                .collect()
        });

        progress(total, total);

        Ok(batch)
    }

    fn write_scanned(
        &mut self,
        folder: &str,
        batch: &[ScannedFile],
        analyze_audio_features: bool,
        audio_feature_progress: Option<AudioFeatureProgressEmitter>,
    ) -> Result<usize, String> {
        if !batch.is_empty() {
            let tx = self
                .conn
                .transaction()
                .map_err(|e| format!("Ошибка транзакции: {}", e))?;

            // Catalog sorts by sort_order DESC; newly-seen files this scan share one
            // value above the current max (grouped at top). Already-known tracks hit
            // the DO UPDATE branch and keep their sort_order, so a bare mtime change
            // doesn't bump them to the top.
            let new_sort_order: i64 = tx
                .query_row("SELECT COALESCE(MAX(sort_order), 0) FROM tracks", [], |r| r.get(0))
                .unwrap_or(0)
                + 1;

            {
                let mut stmt = tx.prepare(
                    "INSERT INTO tracks
                     (file_path, folder_path, title, artist, album, genre, duration_secs,
                      format, sample_rate, bit_depth, channels, bitrate, is_lossless, modified_secs, sort_order,
                      has_cover)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
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
                        accent_color = NULL,
                        has_cover = excluded.has_cover"
                ).map_err(|e| format!("Ошибка подготовки INSERT: {}", e))?;

                for (file_path, m, mtime) in batch {
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
                        new_sort_order,
                        m.has_cover as i32,
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
                "favorites" => PlaylistKind::Favorites,
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

    pub fn get_playlist_tracks(&self, playlist_id: i64) -> Vec<TrackMetadata> {
        self.get_playlist_tracks_inner(playlist_id, false)
    }

    /// Skips covers, to save memory.
    pub fn get_playlist_tracks_light(&self, playlist_id: i64) -> Vec<TrackMetadata> {
        self.get_playlist_tracks_inner(playlist_id, true)
    }

    fn get_playlist_tracks_inner(&self, playlist_id: i64, light: bool) -> Vec<TrackMetadata> {
        let sql = if light {
            concat!(
                track_select!("NULL"),
                " FROM playlist_tracks pt
             JOIN tracks t ON t.file_path = pt.file_path
             LEFT JOIN track_overrides o ON o.file_path = t.file_path
             LEFT JOIN track_audio_features af ON af.file_path = t.file_path
             WHERE pt.playlist_id = ?1
             ORDER BY pt.sort_order ASC,
                      pt.rowid ASC",
            )
        } else {
            concat!(
                track_select!("o.cover_sig"),
                " FROM playlist_tracks pt
             JOIN tracks t ON t.file_path = pt.file_path
             LEFT JOIN track_overrides o ON o.file_path = t.file_path
             LEFT JOIN track_audio_features af ON af.file_path = t.file_path
             WHERE pt.playlist_id = ?1
             ORDER BY pt.sort_order ASC,
                      pt.rowid ASC",
            )
        };

        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = match stmt.query_map(params![playlist_id], Self::row_to_metadata_with_override) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn create_playlist(&mut self, name: String) -> Result<PlaylistInfo, String> {
        self.conn
            .execute(
                "INSERT INTO playlists (name, kind) VALUES (?1, 'user')",
                params![name],
            )
            .map_err(|e| format!("Ошибка создания плейлиста: {}", e))?;

        let id = self.conn.last_insert_rowid();
        self.log_library_event(LibraryEventKind::PlaylistCreate, None, Some(id));
        Ok(PlaylistInfo {
            id,
            name,
            kind: PlaylistKind::User,
            track_count: 0,
            first_file_path: None,
        })
    }

    /// Creates the favorites ("Мне нравится") playlist if it doesn't exist yet,
    /// returning its id either way. Called once on startup so it's present on
    /// first launch without waiting for the user to open collections.
    pub fn ensure_favorites_playlist(&self) -> Result<i64, String> {
        if let Ok(id) = self.conn.query_row(
            "SELECT id FROM playlists WHERE kind = 'favorites' LIMIT 1",
            [],
            |r| r.get::<_, i64>(0),
        ) {
            return Ok(id);
        }
        self.conn
            .execute(
                "INSERT INTO playlists (name, kind) VALUES (?1, 'favorites')",
                params![FAVORITES_PLAYLIST_NAME],
            )
            .map_err(|e| format!("Ошибка создания плейлиста избранного: {}", e))?;
        Ok(self.conn.last_insert_rowid())
    }

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

    pub fn delete_playlist(&mut self, playlist_id: i64) -> Result<(), String> {
        let kind: String = self.conn
            .query_row("SELECT kind FROM playlists WHERE id = ?1", params![playlist_id], |r| r.get(0))
            .map_err(|e| format!("Плейлист не найден: {}", e))?;

        if kind != "user" {
            return Err("Нельзя удалять системные плейлисты".into());
        }

        // Mirrors remove_track_from_playlist: -1 playlist_add_count per track.
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

    /// Adds a track to a playlist. `prepend` puts it at the top (used for new
    /// likes on the favorites playlist); otherwise it's appended to the bottom.
    /// Returns whether the track was added (false if it was already there).
    pub fn add_track_to_playlist(&self, playlist_id: i64, file_path: &str, prepend: bool) -> Result<bool, String> {
        let kind: String = self.conn
            .query_row("SELECT kind FROM playlists WHERE id = ?1", params![playlist_id], |r| r.get(0))
            .map_err(|e| format!("Плейлист не найден: {}", e))?;

        if kind != "user" && kind != "favorites" {
            return Err("Нельзя добавлять треки в системные плейлисты".into());
        }

        let sort_order: i64 = if prepend {
            let min_order: i64 = self.conn
                .query_row(
                    "SELECT COALESCE(MIN(sort_order), 0) FROM playlist_tracks WHERE playlist_id = ?1",
                    params![playlist_id],
                    |r| r.get(0),
                )
                .map_err(|e| format!("Ошибка чтения sort_order: {}", e))?;
            min_order - 1
        } else {
            let max_order: i64 = self.conn
                .query_row(
                    "SELECT COALESCE(MAX(sort_order), 0) FROM playlist_tracks WHERE playlist_id = ?1",
                    params![playlist_id],
                    |r| r.get(0),
                )
                .map_err(|e| format!("Ошибка чтения sort_order: {}", e))?;
            max_order + 1
        };

        let changed = self.conn
            .execute(
                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, file_path, sort_order) VALUES (?1, ?2, ?3)",
                params![playlist_id, file_path, sort_order],
            )
            .map_err(|e| format!("Ошибка добавления трека: {}", e))?;
        if changed > 0 {
            self.add_track_statistics_delta(TrackStatisticsDelta {
                file_path: file_path.to_string(),
                playlist_add_count_delta: 1,
                ..TrackStatisticsDelta::default()
            })?;
            self.log_library_event(LibraryEventKind::PlaylistAdd, Some(file_path), Some(playlist_id));
        }
        Ok(changed > 0)
    }

    /// Returns whether the track was removed (false if it wasn't there).
    pub fn remove_track_from_playlist(&self, playlist_id: i64, file_path: &str) -> Result<bool, String> {
        let kind: String = self.conn
            .query_row("SELECT kind FROM playlists WHERE id = ?1", params![playlist_id], |r| r.get(0))
            .map_err(|e| format!("Плейлист не найден: {}", e))?;

        if kind != "user" && kind != "favorites" {
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
            self.log_library_event(LibraryEventKind::PlaylistRemove, Some(file_path), Some(playlist_id));
        }
        Ok(changed > 0)
    }

    /// File paths of a playlist's tracks, in order. Lighter than
    /// `get_playlist_tracks` — used to hydrate the reactive favorites set.
    pub fn get_playlist_track_paths(&self, playlist_id: i64) -> Vec<String> {
        let mut stmt = match self.conn.prepare(
            "SELECT file_path FROM playlist_tracks
             WHERE playlist_id = ?1
             ORDER BY sort_order ASC, rowid ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params![playlist_id], |row| row.get::<_, String>(0)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    /// Playlists (user-created and the favorites playlist) that contain the
    /// track. Excludes album/artist playlists, which are auto-managed.
    pub fn get_user_playlist_ids_for_track(&self, file_path: &str) -> Vec<i64> {
        let mut stmt = match self.conn.prepare(
            "SELECT pt.playlist_id
             FROM playlist_tracks pt
             JOIN playlists p ON p.id = pt.playlist_id
             WHERE pt.file_path = ?1 AND p.kind IN ('user', 'favorites')",
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
        // A listen changes the taste centroid; update its cache incrementally
        // instead of an O(N) history rescan on every "mix" request.
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

    pub fn log_listen(&self, event: ListenEvent) {
        let due = self.history.borrow_mut().push_listen(event);
        if due {
            if let Err(e) = self.flush_history(false) {
                eprintln!("{}", e);
            }
        }
    }

    fn log_library_event(&self, kind: LibraryEventKind, file_path: Option<&str>, ref_id: Option<i64>) {
        let due = self.history.borrow_mut().push_library(LibraryEvent {
            at: crate::util::unix_secs() as i64,
            kind,
            file_path: file_path.map(str::to_string),
            ref_id,
        });
        if due {
            if let Err(e) = self.flush_history(false) {
                eprintln!("{}", e);
            }
        }
    }

    /// Writes buffered history; `include_open` also writes the session of the
    /// track still playing (shutdown only). A failed batch is dropped rather
    /// than retried forever.
    pub fn flush_history(&self, include_open: bool) -> Result<(), String> {
        let (listens, library) = self.history.borrow_mut().take(include_open);
        statistics::write_history(&self.conn, &listens, &library)
            .map_err(|e| format!("Ошибка записи истории прослушиваний: {}", e))
    }

    /// Recap of one calendar year from the listening history.
    pub fn year_recap(&self, year: i32, artist_separators: &[String]) -> Result<crate::recap::YearRecap, String> {
        self.flush_history(false)?;
        let mut recap = crate::recap::year_recap(&self.conn, year, artist_separators)
            .map_err(|e| format!("Ошибка подсчета итогов года: {}", e))?;
        recap.attach_media(
            |path| crate::thumbnails::cover_url(self, path),
            // Accents are normally cached on first playback; recap tracks may never
            // have been played in this install, so compute (and cache) them here.
            // Custom covers are skipped: their accent lives in track_overrides.
            |path| {
                if self.get_override_cover_sig(path).is_some() {
                    return None;
                }
                let bytes = AudioPlayer::extract_cover_bytes(path)?;
                let color = crate::audio_player::extract_accent_color(&bytes)?;
                let _ = self.set_accent_color(path, &color);
                Some(color)
            },
        );
        Ok(recap)
    }

    /// `None` when the file isn't a library track, otherwise its tagged duration.
    pub fn library_track_duration(&self, file_path: &str) -> Option<Option<f64>> {
        self.conn
            .prepare_cached("SELECT duration_secs FROM tracks WHERE file_path = ?1")
            .and_then(|mut stmt| stmt.query_row(params![file_path], |row| row.get::<_, Option<f64>>(0)))
            .ok()
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

    /// Incremental update: diffs each album against its existing playlist and
    /// applies only the delta; unchanged albums aren't touched.
    pub fn rebuild_album_playlists(&mut self) -> Result<(), String> {
        // Current albums from tracks, with >1 track.
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

        // Existing album playlists: name -> (id, Set<file_path>)
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

        for (name, (id, _)) in &existing_albums {
            if !actual_albums.contains_key(name) {
                tx.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", params![id]).ok();
                tx.execute("DELETE FROM playlists WHERE id = ?1", params![id]).ok();
            }
        }

        for (album_name, new_set) in &actual_albums {
            let playlist_id = if let Some((id, current_set)) = existing_albums.get(album_name) {
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
                    "INSERT INTO playlists (name, kind) VALUES (?1, 'album')",
                    params![album_name],
                )
                .map_err(|e| format!("Ошибка создания альбома: {}", e))?;
                tx.last_insert_rowid()
            };

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

    /// Splits each track's artist by `separators`; one playlist (kind='artist') per name.
    /// Names are grouped case-insensitively (so "MZLFF"/"mzlff" merge); per key we keep a
    /// spelling-variant counter (to pick a canonical name for new playlists) plus the track set.
    pub fn rebuild_artist_playlists(&mut self, separators: &[String]) -> Result<(), String> {
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

        // Grouped by lowercased name; a key can hold multiple legacy case-variant
        // playlists that need merging into one.
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

        for (key, playlists) in &existing_by_key {
            if !actual_artists.contains_key(key) {
                for (id, _, _) in playlists {
                    tx.execute("DELETE FROM playlist_tracks WHERE playlist_id = ?1", params![id]).ok();
                    tx.execute("DELETE FROM playlists WHERE id = ?1", params![id]).ok();
                }
            }
        }

        for (key, (variants, new_set)) in &actual_artists {
            // If a key has multiple legacy duplicates, keep the largest (by track
            // count, ties by oldest id) and merge the rest into it, preserving its
            // existing name so pins/covers/top-order aren't broken. A canonical name
            // (most common spelling) is only chosen when creating a brand-new playlist.
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

    fn app_data_dir(&self) -> &Path {
        self.db_path.parent().unwrap_or_else(|| Path::new("."))
    }

    /// Overrides survive rescans and reset only when the track's folder leaves the
    /// app's scope. `keep_existing_cover = true` leaves cover/accent untouched;
    /// otherwise both are replaced by the passed values (None clears them).
    pub fn set_track_override(
        &self,
        file_path: &str,
        title: Option<&str>,
        artist: Option<&str>,
        cover_data: Option<&str>,
        accent_color: Option<&str>,
        keep_existing_cover: bool,
    ) -> Result<(), String> {
        mark_track_indexes_dirty();
        // Empty string means "use the original".
        let title = title.filter(|&s| !s.trim().is_empty());
        let artist = artist.filter(|&s| !s.trim().is_empty());

        let existing: Option<(Option<String>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT cover_sig, accent_color FROM track_overrides WHERE file_path = ?1",
                params![file_path],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .ok();
        let old_sig = existing.as_ref().and_then(|(s, _)| s.clone());

        // New cover (data URL) is written to track_covers/; only its signature goes in the DB.
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

        // Covers are deduplicated by signature, so clean up the old file if it changed/cleared.
        if old_sig != final_sig {
            if let Some(ref sig) = old_sig {
                self.remove_orphan_cover_file(sig, file_path);
            }
        }

        // All fields empty -> delete the row so the LEFT JOIN falls back to the original.
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

    /// Removes cover file `sig` if no track (other than `excluding_path`) still references it.
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

    /// Also resets cached accent_color so the next playback recomputes it from
    /// the track's original embedded cover.
    pub fn clear_all_track_overrides(&self) -> Result<usize, String> {
        mark_track_indexes_dirty();
        let count = self
            .conn
            .execute("DELETE FROM track_overrides", [])
            .map_err(|e| format!("Ошибка сброса переопределений: {}", e))?;
        let _ = self
            .conn
            .execute("UPDATE tracks SET accent_color = NULL", []);
        let _ = std::fs::remove_dir_all(crate::track_covers::covers_dir(self.app_data_dir()));
        Ok(count)
    }

    pub fn has_override_cover(&self, file_path: &str) -> bool {
        self.conn
            .prepare_cached("SELECT cover_sig IS NOT NULL FROM track_overrides WHERE file_path = ?1")
            .and_then(|mut stmt| stmt.query_row(params![file_path], |row| row.get::<_, i64>(0)))
            .map(|v| v != 0)
            .unwrap_or(false)
    }

    /// Override accent_color wins if set, otherwise falls back to the one cached from the original cover.
    pub fn get_accent_color(&self, file_path: &str) -> Option<String> {
        self.conn
            .prepare_cached(
                "SELECT COALESCE(o.accent_color, t.accent_color)
                 FROM tracks t
                 LEFT JOIN track_overrides o ON o.file_path = t.file_path
                 WHERE t.file_path = ?1",
            )
            .and_then(|mut stmt| {
                stmt.query_row(params![file_path], |row| row.get::<_, Option<String>>(0))
            })
            .ok()
            .flatten()
    }

    /// Short string used as both the `track_covers/` file key and the `?s=` thumbnail URL param.
    pub fn get_override_cover_sig(&self, file_path: &str) -> Option<String> {
        self.conn
            .prepare_cached("SELECT cover_sig FROM track_overrides WHERE file_path = ?1")
            .and_then(|mut stmt| {
                stmt.query_row(params![file_path], |row| row.get::<_, Option<String>>(0))
            })
            .ok()
            .flatten()
    }

    /// Reads bytes from disk lazily, only when actually needed (thumbnail gen, accent color calc).
    pub fn override_cover_bytes(&self, file_path: &str) -> Option<Vec<u8>> {
        let sig = self.get_override_cover_sig(file_path)?;
        crate::track_covers::read_cover(self.app_data_dir(), &sig)
    }

    /// Whether `file_path` has an embedded cover picture, from the cached
    /// `tracks.has_cover` column instead of re-opening/re-parsing tags on every
    /// call (the old path — see [`AudioPlayer::has_embedded_cover`], still used
    /// as the one-time resolver here). A pre-existing row from before this
    /// column existed reads as NULL ("unknown"); resolved once here and written
    /// back, so later calls take the fast DB-only path.
    pub fn has_cover(&self, file_path: &str) -> bool {
        let cached: Option<Option<i64>> = self
            .conn
            .prepare_cached("SELECT has_cover FROM tracks WHERE file_path = ?1")
            .and_then(|mut stmt| {
                stmt.query_row(params![file_path], |row| row.get::<_, Option<i64>>(0))
            })
            .ok();
        match cached {
            Some(Some(v)) => v != 0,
            Some(None) => {
                let has_cover = AudioPlayer::has_embedded_cover(file_path);
                let _ = self.conn.execute(
                    "UPDATE tracks SET has_cover = ?1 WHERE file_path = ?2",
                    params![has_cover as i32, file_path],
                );
                has_cover
            }
            None => false,
        }
    }

    /// Paths confirmed (cached) to have an embedded cover — for the external-prep
    /// batch pass, which needs this per-track for many tracks at once and would
    /// otherwise re-parse tags per group (see `external_prep_worker`). Rows still
    /// NULL ("unknown", not yet resolved by [`Self::has_cover`]) are excluded
    /// rather than resolved here, so this stays a cheap indexed scan; a false
    /// negative just costs one extra (cached, rate-limited) external lookup.
    pub fn cover_present_paths(&self) -> HashSet<String> {
        let mut stmt = match self
            .conn
            .prepare("SELECT file_path FROM tracks WHERE has_cover = 1")
        {
            Ok(s) => s,
            Err(_) => return HashSet::new(),
        };
        let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
            Ok(r) => r,
            Err(_) => return HashSet::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn set_accent_color(&self, file_path: &str, color: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE tracks SET accent_color = ?1 WHERE file_path = ?2",
                params![color, file_path],
            )
            .map_err(|e| format!("Ошибка сохранения accent_color: {}", e))?;
        Ok(())
    }

    pub fn get_albums(&self) -> Vec<PlaylistInfo> {
        self.get_playlists()
            .into_iter()
            .filter(|p| p.kind == PlaylistKind::Album)
            .collect()
    }

    pub fn get_user_playlists(&self) -> Vec<PlaylistInfo> {
        self.get_playlists()
            .into_iter()
            .filter(|p| p.kind == PlaylistKind::User)
            .collect()
    }

    pub fn get_artists(&self) -> Vec<PlaylistInfo> {
        self.get_playlists()
            .into_iter()
            .filter(|p| p.kind == PlaylistKind::Artist)
            .collect()
    }

    pub fn get_recently_played(&self, limit: usize) -> Vec<TrackMetadata> {
        let sql = concat!(
     track_select!("o.cover_sig"),
     " FROM tracks t
                   JOIN track_statistics s ON s.file_path = t.file_path
                   LEFT JOIN track_overrides o ON o.file_path = t.file_path
                   LEFT JOIN track_audio_features af ON af.file_path = t.file_path
                   WHERE s.updated_at > 0
                   ORDER BY s.updated_at DESC
                   LIMIT ?1",
 );
        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params![limit as i64], Self::row_to_metadata_with_override) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn get_most_played(&self, limit: usize) -> Vec<TrackMetadata> {
        let sql = concat!(
     track_select!("o.cover_sig"),
     " FROM tracks t
                   JOIN track_statistics s ON s.file_path = t.file_path
                   LEFT JOIN track_overrides o ON o.file_path = t.file_path
                   LEFT JOIN track_audio_features af ON af.file_path = t.file_path
                   WHERE s.listen_count > 0
                   ORDER BY s.listen_count DESC, s.total_listened_secs DESC
                   LIMIT ?1",
 );
        let mut stmt = match self.conn.prepare(sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params![limit as i64], Self::row_to_metadata_with_override) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn get_top_artists(&self, limit: usize, separators: &[String]) -> Vec<TopArtist> {
        // Grouped by exact spelling in SQL, then split into single artists by the
        // user's separators ("A; B" counts for both A and B, like artist playlists)
        // and merged case-insensitively in Rust so "MZLFF"/"mzlff" count as one artist.
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

        struct Agg {
            variants: HashMap<String, usize>,
            listen_count: i64,
            total_listened_secs: f64,
            track_count: usize,
        }
        let mut map: HashMap<String, Agg> = HashMap::new();
        for (full, listens, secs, tracks) in rows.flatten() {
            for name in split_artists(&full, separators) {
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

    /// Listen-weighted mean/variance of audio features, used to pick similar
    /// ("favorite") or dissimilar ("fresh") tracks. Backed by the accumulator
    /// cache, so repeated calls don't rescan the whole history.
    fn listening_centroid(&self) -> Option<ListeningCentroid> {
        self.centroid_accumulators().to_centroid()
    }

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

    /// Called from `add_track_statistics_delta` when a listen completes.
    fn update_centroid_for_listen(&self, file_path: &str, old_count: i64, new_count: i64) {
        // Cache not loaded yet: skip, it will be rebuilt lazily in full.
        if self.centroid_cache.borrow().is_none() {
            return;
        }

        // Track weight = its listen count; tracks with none don't contribute.
        let old_w = if old_count > 0 { old_count.max(1) as f64 } else { 0.0 };
        let new_w = if new_count > 0 { new_count.max(1) as f64 } else { 0.0 };
        let dw = new_w - old_w;
        if dw == 0.0 {
            return;
        }

        // Must have all features computed, matching what the full recompute requires.
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

        // favorite = close to the taste centroid, fresh = farthest from it, balanced = random.
        let centroid = match familiarity {
            "favorite" | "fresh" => self.listening_centroid(),
            _ => None,
        };

        // Numeric literals are parenthesized so a negative value (loudness) can't
        // turn `--` into a SQL line comment.
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
                // Tracks without audio features sort last, used to fill any remaining slots.
                format!("(af.energy IS NULL) ASC, {dist} * {jitter} {dir}")
            }
            None => "RANDOM()".to_string(),
        };

        let sql = format!(
            concat!(
                track_select!("o.cover_sig"),
                " FROM tracks t
              LEFT JOIN track_overrides o ON o.file_path = t.file_path
              LEFT JOIN track_audio_features af ON af.file_path = t.file_path
              LEFT JOIN track_statistics s ON s.file_path = t.file_path
              WHERE {}
              ORDER BY {}
              LIMIT ?1",
            ),
            where_clause, order_by
        );

        let mut stmt = match self.conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map(params![limit as i64], Self::row_to_metadata_with_override) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(|r| r.ok()).collect()
    }

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

    /// Local album tracks for matching against an external list: (file_path,
    /// effective title, effective artist), with overrides applied.
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

    /// One-time cleanup: old "none" results may have been cached from transient
    /// network/parse errors that are no longer cached today, so they're worth
    /// re-fetching (see `FetchOutcome` in external.rs).
    pub fn clear_external_album_negative_cache(&self) -> Result<usize, String> {
        self.conn
            .execute("DELETE FROM external_album WHERE status = 'none'", [])
            .map_err(|e| format!("Ошибка очистки негативного кэша альбомов: {}", e))
    }

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

    /// Tracks with an override cover, used by the external-cover batch pass to
    /// skip ones that are already locally covered.
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

    /// Albums with no settled external layout (status ok/none) yet. Uses the
    /// same most-common-artist rule as the layout lookup, so cache keys match.
    pub fn pending_external_album_names(&self) -> Vec<String> {
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

/// Weighted sums and sums-of-squares of audio features (weight = listen count),
/// for incremental centroid updates; see [`CentroidAccumulators::to_centroid`].
#[derive(Clone, Default)]
struct CentroidAccumulators {
    /// Number of unique tracks in the centroid.
    n: i64,
    /// Sum of weights (sum of listen_count).
    wsum: f64,
    se: f64, se2: f64,
    sd: f64, sd2: f64,
    sb: f64, sb2: f64,
    sc: f64, sc2: f64,
    sl: f64, sl2: f64,
}

impl CentroidAccumulators {
    /// Adds a track's contribution (or removes it, via a negative weight/`dn`).
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

/// Listen-weighted mean audio-feature profile, plus variances.
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

/// Public taste profile shown in statistics and used by the "Favorite" filter.
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

    #[test]
    fn flush_audio_features_writes_pending_batch_and_clears_it() {
        let dir = temp_library_dir("flush_audio_features");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_bare_track(&library, "/m/a.flac", 100);

        let mut pending = vec![(
            "/m/a.flac".to_string(),
            100u64,
            Some(AudioFeatures {
                bpm: Some(120.0),
                loudness: Some(-10.0),
                energy: Some(0.5),
                danceability: Some(0.6),
                spectral_centroid: Some(2000.0),
                key: Some("C major".to_string()),
                audio_fingerprint: Some("abc123".to_string()),
            }),
        )];

        flush_audio_features(&mut library.conn, &mut pending);

        assert!(pending.is_empty(), "flush must clear the batch it just wrote");
        let (bpm, key): (f64, String) = library
            .conn
            .query_row(
                "SELECT bpm, track_key FROM track_audio_features WHERE file_path = ?1",
                params!["/m/a.flac"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("row should be written by flush");
        assert!((bpm - 120.0).abs() < 1e-6);
        assert_eq!(key, "C major");

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn flush_audio_features_second_batch_overwrites_the_first_for_the_same_track() {
        // Regression check for the streaming rewrite: results now arrive (and get
        // flushed) in small batches instead of one chunk per file, so a later
        // batch for an already-written track must upsert, not error/duplicate.
        let dir = temp_library_dir("flush_audio_features_upsert");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_bare_track(&library, "/m/a.flac", 100);

        let mut first = vec![(
            "/m/a.flac".to_string(),
            100u64,
            Some(AudioFeatures {
                bpm: Some(90.0),
                loudness: None,
                energy: None,
                danceability: None,
                spectral_centroid: None,
                key: None,
                audio_fingerprint: None,
            }),
        )];
        flush_audio_features(&mut library.conn, &mut first);

        let mut second = vec![(
            "/m/a.flac".to_string(),
            100u64,
            Some(AudioFeatures {
                bpm: Some(140.0),
                loudness: None,
                energy: None,
                danceability: None,
                spectral_centroid: None,
                key: None,
                audio_fingerprint: None,
            }),
        )];
        flush_audio_features(&mut library.conn, &mut second);

        let row_count: i64 = library
            .conn
            .query_row(
                "SELECT COUNT(*) FROM track_audio_features WHERE file_path = ?1",
                params!["/m/a.flac"],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(row_count, 1, "must upsert, not insert a duplicate row");

        let bpm: f64 = library
            .conn
            .query_row(
                "SELECT bpm FROM track_audio_features WHERE file_path = ?1",
                params!["/m/a.flac"],
                |row| row.get(0),
            )
            .expect("query");
        assert!((bpm - 140.0).abs() < 1e-6, "second flush should win");

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn flush_audio_features_on_empty_pending_is_a_no_op() {
        let dir = temp_library_dir("flush_audio_features_empty");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        let mut pending: Vec<(String, u64, Option<AudioFeatures>)> = Vec::new();

        // Must not panic/error on an empty batch — spawn_audio_feature_analysis
        // calls this unconditionally after draining the results channel.
        flush_audio_features(&mut library.conn, &mut pending);
        assert!(pending.is_empty());

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cleanup_keeps_tracks_of_unavailable_folders() {
        let dir = temp_library_dir("cleanup_unavailable_folder");
        let present_root = dir.join("present");
        fs::create_dir_all(&present_root).unwrap();
        let present_root = present_root.to_string_lossy().to_string();
        let gone_root = dir.join("unplugged-drive").to_string_lossy().to_string();
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        for (path, root) in [
            (format!("{}/deleted.flac", present_root), &present_root),
            (format!("{}/offline.flac", gone_root), &gone_root),
        ] {
            library
                .conn
                .execute(
                    "INSERT INTO tracks (file_path, folder_path, format) VALUES (?1, ?2, 'flac')",
                    params![path, root],
                )
                .expect("track insert");
        }

        // The file under an available folder is really gone; the one under a
        // missing folder may just be on a drive that isn't plugged in.
        assert_eq!(library.cleanup_missing_tracks().unwrap(), 1);
        let left: Vec<String> = library
            .conn
            .prepare("SELECT file_path FROM tracks")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(left, vec![format!("{}/offline.flac", gone_root)]);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    /// Inserts a bare `tracks` row (no audio features) with a given `modified_secs`.
    fn insert_bare_track(library: &MusicLibrary, path: &str, modified_secs: i64) {
        library
            .conn
            .execute(
                "INSERT INTO tracks
                 (file_path, folder_path, title, artist, album, duration_secs, format,
                  sample_rate, bit_depth, channels, bitrate, is_lossless, modified_secs, sort_order)
                 VALUES (?1, '/music', 'T', 'A', 'Al', 180.0, 'flac', 44100, 16, 2, 320, 0, ?2, 1)",
                params![path, modified_secs],
            )
            .expect("track insert");
    }

    #[test]
    fn pending_audio_feature_jobs_requeues_stale_analyzer_version() {
        let dir = temp_library_dir("pending_stale_version");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_bare_track(&library, "/music/stale.flac", 100);
        library
            .conn
            .execute(
                "INSERT INTO track_audio_features (file_path, analyzed_modified_secs, analyzer_version)
                 VALUES ('/music/stale.flac', 100, ?1)",
                params![AUDIO_FEATURE_ANALYZER_VERSION - 1],
            )
            .expect("features insert");

        let jobs = library.pending_audio_feature_jobs().expect("pending jobs");
        assert_eq!(
            jobs.iter().filter(|(p, _, _)| p == "/music/stale.flac").count(),
            1,
            "a track analyzed with an older analyzer version must be requeued"
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn pending_audio_feature_jobs_requeues_track_whose_mtime_changed() {
        let dir = temp_library_dir("pending_mtime_changed");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_bare_track(&library, "/music/touched.flac", 200);
        library
            .conn
            .execute(
                "INSERT INTO track_audio_features (file_path, analyzed_modified_secs, analyzer_version)
                 VALUES ('/music/touched.flac', 100, ?1)",
                params![AUDIO_FEATURE_ANALYZER_VERSION],
            )
            .expect("features insert");

        let jobs = library.pending_audio_feature_jobs().expect("pending jobs");
        assert_eq!(
            jobs.iter().filter(|(p, _, _)| p == "/music/touched.flac").count(),
            1,
            "a track whose file mtime changed since analysis must be requeued"
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn pending_audio_feature_jobs_excludes_up_to_date_track() {
        let dir = temp_library_dir("pending_up_to_date");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_bare_track(&library, "/music/done.flac", 300);
        library
            .conn
            .execute(
                "INSERT INTO track_audio_features (file_path, analyzed_modified_secs, analyzer_version)
                 VALUES ('/music/done.flac', 300, ?1)",
                params![AUDIO_FEATURE_ANALYZER_VERSION],
            )
            .expect("features insert");

        let jobs = library.pending_audio_feature_jobs().expect("pending jobs");
        assert!(
            jobs.iter().all(|(p, _, _)| p != "/music/done.flac"),
            "a track analyzed at its current mtime with the current analyzer version must not be requeued"
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn deleting_a_track_cascades_to_features_playlist_tracks_and_statistics_but_not_overrides() {
        let dir = temp_library_dir("fk_cascade");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_bare_track(&library, "/music/a.flac", 1);
        library
            .conn
            .execute(
                "INSERT INTO track_audio_features (file_path, analyzed_modified_secs, analyzer_version)
                 VALUES ('/music/a.flac', 1, ?1)",
                params![AUDIO_FEATURE_ANALYZER_VERSION],
            )
            .expect("features insert");
        let playlist_id: i64 = library
            .conn
            .query_row(
                "INSERT INTO playlists (name, kind) VALUES ('My list', 'user') RETURNING id",
                [],
                |r| r.get(0),
            )
            .expect("playlist insert");
        library
            .conn
            .execute(
                "INSERT INTO playlist_tracks (playlist_id, file_path, sort_order) VALUES (?1, '/music/a.flac', 0)",
                params![playlist_id],
            )
            .expect("playlist_tracks insert");
        library
            .set_track_override("/music/a.flac", Some("Override"), None, None, None, false)
            .expect("set override");
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/music/a.flac".to_string(),
                listen_count_delta: 1,
                ..Default::default()
            })
            .expect("record statistic");

        library
            .conn
            .execute("DELETE FROM tracks WHERE file_path = '/music/a.flac'", [])
            .expect("delete track row directly");

        let features: i64 = library
            .conn
            .query_row(
                "SELECT COUNT(*) FROM track_audio_features WHERE file_path = '/music/a.flac'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(features, 0, "track_audio_features has an FK and must cascade");
        let playlist_rows: i64 = library
            .conn
            .query_row(
                "SELECT COUNT(*) FROM playlist_tracks WHERE file_path = '/music/a.flac'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(playlist_rows, 0, "playlist_tracks has an FK and must cascade");
        let stats: i64 = library
            .conn
            .query_row(
                "SELECT COUNT(*) FROM track_statistics WHERE file_path = '/music/a.flac'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stats, 0, "track_statistics has an FK and must cascade");

        // track_overrides is the one table with NO FK to tracks (by design — see
        // delete_missing_tracks, which cleans it explicitly for that reason). A raw
        // DELETE FROM tracks, unlike remove_track()/rescan, does NOT clean it up;
        // documenting this so the asymmetry isn't "rediscovered" as a bug.
        let overrides: i64 = library
            .conn
            .query_row(
                "SELECT COUNT(*) FROM track_overrides WHERE file_path = '/music/a.flac'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(overrides, 1, "track_overrides has no FK, so a raw DELETE leaves it orphaned");

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    /// Inserts a track with a full set of audio features and a listen count.
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

        // First call loads the accumulator cache (O(N) scan).
        let _ = library.taste_profile();

        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/music/a.flac".to_string(),
                listen_count_delta: 3,
                ..Default::default()
            })
            .expect("delta a");
        // New track not previously in the centroid (listen_count 0 -> 1).
        insert_listened_track(&library, "/music/c.flac", 0, (0.5, 0.55, 110.0, 2400.0, -11.0));
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/music/c.flac".to_string(),
                listen_count_delta: 1,
                ..Default::default()
            })
            .expect("delta c");

        let incremental = library.taste_profile();

        // Reset cache and recompute from scratch; should match.
        library.centroid_cache.borrow_mut().take();
        let full = library.taste_profile();

        assert_eq!(incremental.analyzed_tracks, 3, "three tracks listened");
        assert_profiles_close(&incremental, &full);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    fn insert_track_row(
        library: &MusicLibrary,
        path: &str,
        title: Option<&str>,
        artist: Option<&str>,
        album: Option<&str>,
        genre: Option<&str>,
        sort_order: i64,
    ) {
        library
            .conn
            .execute(
                "INSERT INTO tracks
                 (file_path, folder_path, title, artist, album, genre, duration_secs, format,
                  sample_rate, bit_depth, channels, bitrate, is_lossless, modified_secs, sort_order)
                 VALUES (?1, '/music', ?2, ?3, ?4, ?5, 180.0, 'flac', 44100, 16, 2, 320, 1, 1, ?6)",
                params![path, title, artist, album, genre, sort_order],
            )
            .expect("insert track row");
    }

    /// Writes a minimal valid 16-bit PCM WAV for tests that need a real
    /// readable audio file on disk (insert_downloaded_track/add_folder read
    /// metadata via AudioPlayer::get_metadata_light).
    fn write_test_wav(path: &Path, sample_rate: u32, channels: u16, num_frames: u32) {
        let bits_per_sample: u16 = 16;
        let block_align: u16 = channels * (bits_per_sample / 8);
        let byte_rate: u32 = sample_rate * block_align as u32;
        let data_size: u32 = num_frames * block_align as u32;

        let mut buf = Vec::with_capacity(44 + data_size as usize);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_size).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bits_per_sample.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        for i in 0..(num_frames as usize * channels as usize) {
            let sample = (((i % 200) as i32 - 100) * 300) as i16;
            buf.extend_from_slice(&sample.to_le_bytes());
        }
        fs::write(path, buf).expect("write test wav");
    }

    #[test]
    fn split_artists_applies_each_separator_in_sequence() {
        let seps = vec!["; ".to_string(), ", ".to_string()];
        assert_eq!(
            split_artists("Artist A; Artist B, Artist C", &seps),
            vec!["Artist A", "Artist B", "Artist C"]
        );
    }

    #[test]
    fn split_artists_trims_and_drops_empty_pieces() {
        let seps = vec![";".to_string()];
        assert_eq!(split_artists("A; ;B; ", &seps), vec!["A", "B"]);
    }

    #[test]
    fn split_artists_with_no_separators_returns_trimmed_whole_string() {
        let seps: Vec<String> = vec![];
        assert_eq!(split_artists("  Single Artist  ", &seps), vec!["Single Artist"]);
    }

    #[test]
    fn split_artists_ignores_empty_separator_strings() {
        let seps = vec!["".to_string(), ", ".to_string()];
        assert_eq!(split_artists("A, B", &seps), vec!["A", "B"]);
    }

    #[test]
    fn split_artists_of_empty_input_is_empty_vec() {
        let seps = vec![", ".to_string()];
        assert!(split_artists("", &seps).is_empty());
        assert!(split_artists("   ", &seps).is_empty());
    }

    #[test]
    fn insert_downloaded_track_reads_real_metadata_and_registers_folder() {
        let dir = temp_library_dir("insert_downloaded");
        let downloads = dir.join("downloads");
        fs::create_dir_all(&downloads).unwrap();
        let track_path = downloads.join("clip.wav");
        write_test_wav(&track_path, 8_000, 1, 8_000 * 2);

        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        library
            .insert_downloaded_track(track_path.to_str().unwrap(), downloads.to_str().unwrap(), &[])
            .expect("insert downloaded track");

        let track = library.get_track(track_path.to_str().unwrap()).expect("track exists");
        assert_eq!(track.format, "WAV");
        assert!(track.is_lossless);

        let folders = library.get_folders();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].track_count, 1);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn insert_downloaded_track_into_already_covered_subfolder_does_not_duplicate_folder() {
        let dir = temp_library_dir("covering_folder");
        let root = dir.join("root");
        let sub = root.join("sub");
        fs::create_dir_all(&sub).unwrap();

        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        library
            .add_folder(root.to_str().unwrap().to_string(), &[], None, |_, _| {})
            .expect("add root folder");
        assert_eq!(library.get_folders().len(), 1);

        let track_path = sub.join("track.wav");
        write_test_wav(&track_path, 8_000, 1, 8_000);
        library
            .insert_downloaded_track(track_path.to_str().unwrap(), sub.to_str().unwrap(), &[])
            .expect("insert into subfolder");

        // Subfolder is already covered by the root folder — no new entry expected.
        assert_eq!(library.get_folders().len(), 1, "не должно появиться дублирующей папки");

        let folder_path: String = library
            .conn
            .query_row(
                "SELECT folder_path FROM tracks WHERE file_path = ?1",
                params![track_path.to_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(folder_path, root.to_str().unwrap());

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn add_folder_rejects_nonexistent_directory() {
        let dir = temp_library_dir("add_folder_missing");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        let result = library.add_folder(
            dir.join("does_not_exist").to_str().unwrap().to_string(),
            &[],
            None,
            |_, _| {},
        );
        assert!(result.is_err());
        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn add_folder_with_one_audio_file_scans_it_in() {
        let dir = temp_library_dir("add_folder_scan");
        let music = dir.join("music");
        fs::create_dir_all(&music).unwrap();
        write_test_wav(&music.join("a.wav"), 44_100, 2, 44_100);

        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        let tracks = library
            .add_folder(music.to_str().unwrap().to_string(), &[], None, |_, _| {})
            .expect("add folder");
        assert_eq!(tracks.len(), 1);
        assert_eq!(library.get_all_tracks().len(), 1);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unlocked_add_and_rescan_scan_files_and_drop_deleted_ones() {
        let dir = temp_library_dir("unlocked_scan");
        let music = dir.join("music");
        fs::create_dir_all(&music).unwrap();
        write_test_wav(&music.join("a.wav"), 44_100, 2, 44_100);
        write_test_wav(&music.join("b.wav"), 44_100, 2, 44_100);

        let lib = Mutex::new(MusicLibrary::new(dir.clone()).expect("library init"));
        MusicLibrary::add_folder_unlocked(&lib, music.to_str().unwrap().to_string(), &[], None, |_, _| {})
            .expect("add folder");
        assert_eq!(lib.lock().unwrap().get_all_tracks().len(), 2);

        fs::remove_file(music.join("b.wav")).unwrap();
        MusicLibrary::rescan_all_unlocked(&lib, &[], None, |_, _| {}).expect("rescan");
        let tracks = lib.lock().unwrap().get_all_tracks();
        assert_eq!(tracks.len(), 1);
        assert!(tracks[0].file_path.ends_with("a.wav"));

        drop(lib);
        let _ = fs::remove_dir_all(dir);
    }

    /// Bumps mtime by whole seconds (get_mtime only tracks second granularity),
    /// so the "file changed" branch is deterministic on a fast filesystem.
    fn bump_mtime(path: &Path, delta_secs: u64) {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open file for writing to bump mtime");
        let current = file.metadata().expect("metadata").modified().expect("modified");
        file.set_modified(current + std::time::Duration::from_secs(delta_secs))
            .expect("set_modified should succeed on a local filesystem");
    }

    #[test]
    fn rescan_all_skips_unchanged_file_and_keeps_accent_color() {
        let dir = temp_library_dir("rescan_unchanged");
        let music = dir.join("music");
        fs::create_dir_all(&music).unwrap();
        write_test_wav(&music.join("a.wav"), 44_100, 2, 44_100);

        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        library
            .add_folder(music.to_str().unwrap().to_string(), &[], None, |_, _| {})
            .expect("add folder");
        let path = library.get_all_tracks()[0].file_path.clone();
        library.set_accent_color(&path, "#abcdef").expect("set accent color");

        let tracks = library
            .rescan_all(&[], None, |_, _| {})
            .expect("rescan should succeed");

        assert_eq!(tracks.len(), 1, "rescan should not duplicate or drop the track");
        let track = library.get_track(&path).expect("track still present");
        assert_eq!(
            track.accent_color.as_deref(),
            Some("#abcdef"),
            "unchanged mtime must not touch the cached accent color"
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rescan_all_rewrites_changed_file_without_bumping_its_sort_order() {
        let dir = temp_library_dir("rescan_changed");
        let music = dir.join("music");
        fs::create_dir_all(&music).unwrap();
        write_test_wav(&music.join("a.wav"), 44_100, 2, 44_100);

        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        library
            .add_folder(music.to_str().unwrap().to_string(), &[], None, |_, _| {})
            .expect("add folder");
        let path = music.join("a.wav").to_string_lossy().to_string();
        library.set_accent_color(&path, "#abcdef").expect("set accent color");
        let sort_order_before: i64 = library
            .conn
            .query_row(
                "SELECT sort_order FROM tracks WHERE file_path = ?1",
                params![path],
                |r| r.get(0),
            )
            .expect("read sort_order");

        // File appears after the first scan — new tracks get sort_order above the current max.
        write_test_wav(&music.join("b.wav"), 44_100, 2, 44_100);
        library
            .rescan_all(&[], None, |_, _| {})
            .expect("rescan should pick up the new file");

        // Change content and mtime of an already-tracked file.
        write_test_wav(&music.join("a.wav"), 44_100, 2, 22_050);
        bump_mtime(&music.join("a.wav"), 120);

        library
            .rescan_all(&[], None, |_, _| {})
            .expect("rescan should pick up the changed file");

        let sort_order_after: i64 = library
            .conn
            .query_row(
                "SELECT sort_order FROM tracks WHERE file_path = ?1",
                params![path],
                |r| r.get(0),
            )
            .expect("read sort_order");
        assert_eq!(
            sort_order_before, sort_order_after,
            "a changed mtime on an already-known track must not bump it to the top of the catalog"
        );

        let track = library.get_track(&path).expect("track still present");
        assert_eq!(
            track.accent_color, None,
            "a content/mtime change must invalidate the cached accent color"
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rescan_all_removes_track_whose_file_disappeared() {
        let dir = temp_library_dir("rescan_deleted");
        let music = dir.join("music");
        fs::create_dir_all(&music).unwrap();
        let file = music.join("a.wav");
        write_test_wav(&file, 44_100, 2, 44_100);

        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        library
            .add_folder(music.to_str().unwrap().to_string(), &[], None, |_, _| {})
            .expect("add folder");
        let path = file.to_string_lossy().to_string();
        assert!(library.get_track(&path).is_some());

        fs::remove_file(&file).expect("delete underlying file");
        let tracks = library
            .rescan_all(&[], None, |_, _| {})
            .expect("rescan should succeed");

        assert!(tracks.is_empty());
        assert!(
            library.get_track(&path).is_none(),
            "a track whose file disappeared must be removed by rescan"
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rescan_all_clears_statistics_and_overrides_for_a_deleted_track() {
        let dir = temp_library_dir("rescan_deleted_orphans");
        let music = dir.join("music");
        fs::create_dir_all(&music).unwrap();
        let file = music.join("a.wav");
        write_test_wav(&file, 44_100, 2, 44_100);

        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        library
            .add_folder(music.to_str().unwrap().to_string(), &[], None, |_, _| {})
            .expect("add folder");
        let path = file.to_string_lossy().to_string();
        library
            .set_track_override(&path, Some("Override Title"), None, None, None, false)
            .expect("set override");
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: path.clone(),
                listen_count_delta: 1,
                ..Default::default()
            })
            .expect("record statistic");

        fs::remove_file(&file).expect("delete underlying file");
        library
            .rescan_all(&[], None, |_, _| {})
            .expect("rescan should succeed");

        let overrides: i64 = library
            .conn
            .query_row(
                "SELECT COUNT(*) FROM track_overrides WHERE file_path = ?1",
                params![path],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(overrides, 0, "rescan must clear the override of a removed track, like remove_track does");
        let stats: i64 = library
            .conn
            .query_row(
                "SELECT COUNT(*) FROM track_statistics WHERE file_path = ?1",
                params![path],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stats, 0, "rescan must clear the statistics of a removed track, like remove_track does");

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn remove_track_deletes_row_statistics_and_overrides() {
        let dir = temp_library_dir("remove_track");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("X"), None, None, 1);
        insert_track_row(&library, "/m/b.flac", Some("B"), Some("Y"), None, None, 2);
        library
            .set_track_override("/m/a.flac", Some("Override Title"), None, None, None, false)
            .unwrap();
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/m/a.flac".to_string(),
                listen_count_delta: 1,
                ..Default::default()
            })
            .unwrap();

        library.remove_track("/m/a.flac", &[]).expect("remove track");

        assert!(library.get_track("/m/a.flac").is_none());
        assert!(library.get_track("/m/b.flac").is_some());
        let remaining: i64 = library
            .conn
            .query_row("SELECT COUNT(*) FROM track_statistics WHERE file_path = '/m/a.flac'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0, "статистика удалённого трека должна быть очищена");
        let overrides: i64 = library
            .conn
            .query_row("SELECT COUNT(*) FROM track_overrides WHERE file_path = '/m/a.flac'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(overrides, 0, "переопределение удалённого трека должно быть очищено");

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn move_track_to_top_reorders_and_records_statistic() {
        let dir = temp_library_dir("move_to_top");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("X"), None, None, 1);
        insert_track_row(&library, "/m/b.flac", Some("B"), Some("Y"), None, None, 2);

        // B has a higher sort_order -> comes first by default.
        let before = library.get_all_tracks();
        assert_eq!(before[0].file_path, "/m/b.flac");

        library.move_track_to_top("/m/a.flac").expect("move to top");
        let after = library.get_all_tracks();
        assert_eq!(after[0].file_path, "/m/a.flac");

        let stats = library.get_track_statistics("/m/a.flac").unwrap();
        assert_eq!(stats.move_to_top_count, 1);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn list_genres_excludes_genres_with_only_one_track() {
        let dir = temp_library_dir("list_genres");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/1.flac", Some("1"), Some("A"), None, Some("Rock"), 1);
        insert_track_row(&library, "/m/2.flac", Some("2"), Some("A"), None, Some("Rock"), 2);
        insert_track_row(&library, "/m/3.flac", Some("3"), Some("A"), None, Some("Jazz"), 3);

        let genres = library.list_genres();
        let names: Vec<&str> = genres.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"Rock"));
        assert!(
            !names.contains(&"Jazz"),
            "жанр только с одним треком не должен попадать в список фильтров (HAVING cnt > 1)"
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn search_tracks_ranks_exact_substring_before_basename_only_match() {
        let dir = temp_library_dir("search_tracks");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/exact.flac", Some("Believer"), Some("Imagine Dragons"), None, None, 1);
        insert_track_row(&library, "/m/other.flac", Some("Unrelated"), Some("Someone"), None, None, 2);

        let results = library.search_tracks("believer", None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file_path, "/m/exact.flac");

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_catalog_paths_filters_by_genre_without_search_query() {
        let dir = temp_library_dir("catalog_paths_genre");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/rock.flac", Some("R"), Some("A"), None, Some("Rock"), 1);
        insert_track_row(&library, "/m/jazz.flac", Some("J"), Some("A"), None, Some("Jazz"), 2);

        let paths = library.get_catalog_paths(Some("Rock"), None);
        assert_eq!(paths, vec!["/m/rock.flac".to_string()]);

        let all = library.get_catalog_paths(None, None);
        assert_eq!(all.len(), 2);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn user_playlist_crud_round_trip() {
        let dir = temp_library_dir("playlist_crud");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("X"), None, None, 1);

        let playlist = library.create_playlist("My Mix".to_string()).expect("create playlist");
        assert_eq!(playlist.kind, PlaylistKind::User);
        assert_eq!(playlist.track_count, 0);

        library.add_track_to_playlist(playlist.id, "/m/a.flac", false).expect("add track");
        let tracks = library.get_playlist_tracks(playlist.id);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].file_path, "/m/a.flac");

        let ids = library.get_user_playlist_ids_for_track("/m/a.flac");
        assert_eq!(ids, vec![playlist.id]);

        library.rename_playlist(playlist.id, "Renamed".to_string()).expect("rename");
        let playlists = library.get_user_playlists();
        assert_eq!(playlists[0].name, "Renamed");

        library.remove_track_from_playlist(playlist.id, "/m/a.flac").expect("remove track");
        assert!(library.get_playlist_tracks(playlist.id).is_empty());

        library.delete_playlist(playlist.id).expect("delete playlist");
        assert!(library.get_user_playlists().is_empty());

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn add_track_to_playlist_increments_playlist_add_count_statistic() {
        let dir = temp_library_dir("playlist_stat");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("X"), None, None, 1);
        let playlist = library.create_playlist("Mix".to_string()).unwrap();

        library.add_track_to_playlist(playlist.id, "/m/a.flac", false).unwrap();
        let stats = library.get_track_statistics("/m/a.flac").unwrap();
        assert_eq!(stats.playlist_add_count, 1);

        library.remove_track_from_playlist(playlist.id, "/m/a.flac").unwrap();
        let stats_after = library.get_track_statistics("/m/a.flac").unwrap();
        assert_eq!(stats_after.playlist_add_count, 0);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cannot_rename_delete_or_modify_album_playlists() {
        let dir = temp_library_dir("album_playlist_protection");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/1.flac", Some("1"), Some("A"), Some("Greatest Hits"), None, 1);
        insert_track_row(&library, "/m/2.flac", Some("2"), Some("A"), Some("Greatest Hits"), None, 2);
        library.rebuild_album_playlists().expect("rebuild albums");

        let albums = library.get_albums();
        assert_eq!(albums.len(), 1, "альбом с >1 треком должен получить системный плейлист");
        let album_id = albums[0].id;

        assert!(library.rename_playlist(album_id, "Hacked".to_string()).is_err());
        assert!(library.add_track_to_playlist(album_id, "/m/1.flac", false).is_err());
        assert!(library.remove_track_from_playlist(album_id, "/m/1.flac").is_err());
        assert!(library.delete_playlist(album_id).is_err());

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn favorites_playlist_is_manageable_but_protected() {
        let dir = temp_library_dir("favorites_playlist");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("X"), None, None, 1);
        insert_track_row(&library, "/m/b.flac", Some("B"), Some("X"), None, None, 2);

        // Idempotent: two ensure calls yield the same singleton playlist.
        let fav_id = library.ensure_favorites_playlist().expect("ensure favorites");
        assert_eq!(fav_id, library.ensure_favorites_playlist().expect("ensure again"));

        let favorites: Vec<_> = library
            .get_playlists()
            .into_iter()
            .filter(|p| p.kind == PlaylistKind::Favorites)
            .collect();
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].name, FAVORITES_PLAYLIST_NAME);
        // Not a "user" playlist, so it stays out of the user-playlist list.
        assert!(library.get_user_playlists().is_empty());

        // Accepts manual add/remove; new likes are prepended to the top.
        library.add_track_to_playlist(fav_id, "/m/a.flac", false).expect("add to favorites");
        library.add_track_to_playlist(fav_id, "/m/b.flac", true).expect("prepend to favorites");
        assert_eq!(
            library.get_playlist_track_paths(fav_id),
            vec!["/m/b.flac".to_string(), "/m/a.flac".to_string()],
            "новый лайк должен попадать в начало плейлиста"
        );
        assert_eq!(library.get_user_playlist_ids_for_track("/m/a.flac"), vec![fav_id]);
        library.remove_track_from_playlist(fav_id, "/m/a.flac").expect("remove from favorites");
        library.remove_track_from_playlist(fav_id, "/m/b.flac").expect("remove from favorites");
        assert!(library.get_playlist_tracks(fav_id).is_empty());

        // ...but can't be renamed or deleted.
        let mut library = library;
        assert!(library.rename_playlist(fav_id, "Hacked".to_string()).is_err());
        assert!(library.delete_playlist(fav_id).is_err());

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rebuild_album_playlists_skips_albums_with_only_one_track() {
        let dir = temp_library_dir("album_single_track");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/1.flac", Some("1"), Some("A"), Some("Solo Album"), None, 1);
        library.rebuild_album_playlists().expect("rebuild");

        assert!(
            library.get_albums().is_empty(),
            "альбом с одним треком не должен формировать отдельный плейлист"
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rebuild_artist_playlists_splits_multi_artist_tracks_by_separators() {
        let dir = temp_library_dir("artist_split");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        let seps = vec!["; ".to_string(), ", ".to_string()];
        insert_track_row(&library, "/m/1.flac", Some("Collab"), Some("Artist A; Artist B"), None, None, 1);
        library.rebuild_artist_playlists(&seps).expect("rebuild artists");

        let names: Vec<String> = library.get_artists().into_iter().map(|p| p.name).collect();
        assert!(names.contains(&"Artist A".to_string()));
        assert!(names.contains(&"Artist B".to_string()));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rebuild_artist_playlists_merges_artists_case_insensitively() {
        let dir = temp_library_dir("artist_case_merge");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        let seps: Vec<String> = vec![];
        insert_track_row(&library, "/m/1.flac", Some("1"), Some("MZLFF"), None, None, 1);
        insert_track_row(&library, "/m/2.flac", Some("2"), Some("mzlff"), None, None, 2);
        library.rebuild_artist_playlists(&seps).expect("rebuild artists");

        let artists = library.get_artists();
        assert_eq!(
            artists.len(),
            1,
            "разные написания одного артиста должны слиться в один плейлист"
        );
        assert_eq!(artists[0].track_count, 2);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rebuild_artist_playlists_creates_playlist_even_for_single_track_artist() {
        // Unlike albums, artist playlists have no ">1 track" filter — intentional
        // asymmetry (see rebuild_album_playlists/rebuild_artist_playlists).
        let dir = temp_library_dir("artist_single_track");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/1.flac", Some("Solo"), Some("Solo Artist"), None, None, 1);
        library.rebuild_artist_playlists(&[]).expect("rebuild artists");

        assert_eq!(library.get_artists().len(), 1);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sum_track_listen_count_aggregates_across_paths() {
        let dir = temp_library_dir("sum_listen_count");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", None, None, None, None, 1);
        insert_track_row(&library, "/m/b.flac", None, None, None, None, 2);
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/m/a.flac".to_string(),
                listen_count_delta: 2,
                ..Default::default()
            })
            .unwrap();
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/m/b.flac".to_string(),
                listen_count_delta: 5,
                ..Default::default()
            })
            .unwrap();

        let total = library
            .sum_track_listen_count(&["/m/a.flac".to_string(), "/m/b.flac".to_string()])
            .unwrap();
        assert_eq!(total, 7);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn add_collection_full_listen_increments_collection_stats() {
        let dir = temp_library_dir("collection_listen");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        library.add_collection_full_listen(CollectionKind::Playlist, "3").unwrap();
        library.add_collection_full_listen(CollectionKind::Playlist, "3").unwrap();

        let count: i64 = library
            .conn
            .query_row(
                "SELECT full_listen_count FROM collection_statistics WHERE collection_key = '3'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    fn listen_rows(library: &MusicLibrary) -> i64 {
        library
            .conn
            .query_row("SELECT COUNT(*) FROM listen_history", [], |r| r.get(0))
            .unwrap()
    }

    fn finished_listen(path: &str, started_at: i64) -> ListenEvent {
        ListenEvent {
            file_path: path.to_string(),
            started_at,
            listened_secs: 200.4,
            counted: true,
            full: true,
            shuffle: true,
            source: Some("playlist:7".to_string()),
            finished: true,
            ..ListenEvent::default()
        }
    }

    #[test]
    fn listen_history_is_buffered_until_flush_and_survives_track_removal() {
        let dir = temp_library_dir("listen_history");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("X"), Some("Al"), Some("Rock"), 1);

        library.log_listen(finished_listen("/m/a.flac", 1_000));
        library.log_listen(finished_listen("/m/a.flac", 2_000));
        assert_eq!(listen_rows(&library), 0, "nothing is written before the batch is due");

        library.flush_history(false).unwrap();
        assert_eq!(listen_rows(&library), 2);

        let (secs, flags, kind, source_id): (i64, i64, i64, Option<i64>) = library
            .conn
            .query_row(
                "SELECT listened_secs, flags, source_kind, source_id FROM listen_history LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(secs, 200);
        assert_eq!(
            flags,
            statistics::LISTEN_COUNTED | statistics::LISTEN_FULL | statistics::LISTEN_SHUFFLE
        );
        assert_eq!((kind, source_id), (statistics::SOURCE_PLAYLIST, Some(7)));

        library.remove_track("/m/a.flac", &[]).unwrap();
        let (tracks, artist, genre): (i64, String, String) = library
            .conn
            .query_row("SELECT COUNT(*), artist, genre FROM history_tracks", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((tracks, artist.as_str(), genre.as_str()), (1, "X", "Rock"));
        assert_eq!(listen_rows(&library), 2);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn listen_history_writes_a_batch_once_enough_events_are_buffered() {
        let dir = temp_library_dir("listen_history_batch");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("X"), None, None, 1);

        for i in 0..15 {
            library.log_listen(finished_listen("/m/a.flac", i));
        }
        assert_eq!(listen_rows(&library), 0);
        library.log_listen(finished_listen("/m/a.flac", 15));
        assert_eq!(listen_rows(&library), 16);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn open_listen_is_written_only_on_shutdown_and_replaced_by_its_finish() {
        let dir = temp_library_dir("listen_history_open");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("X"), None, None, 1);

        let open = ListenEvent { finished: false, ..finished_listen("/m/a.flac", 50) };
        library.log_listen(open.clone());
        library.flush_history(false).unwrap();
        assert_eq!(listen_rows(&library), 0, "a regular flush leaves the playing track alone");

        // Finishing the same session drops the open copy: one row, not two.
        library.log_listen(finished_listen("/m/a.flac", 50));
        library.shutdown().unwrap();
        assert_eq!(listen_rows(&library), 1);

        library.log_listen(open);
        library.shutdown().unwrap();
        assert_eq!(listen_rows(&library), 2, "shutdown keeps the unfinished listen");

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn year_recap_aggregates_tops_days_and_obsession() {
        let dir = temp_library_dir("year_recap");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("X feat. Y"), Some("Al"), Some("Rock"), 1);
        insert_track_row(&library, "/m/b.flac", Some("B"), Some("Z"), None, Some("Jazz"), 2);

        // Local noon of 2025-03-10 and the next day, whatever the test machine's zone.
        let day1: i64 = library
            .conn
            .query_row("SELECT CAST(strftime('%s', '2025-03-10 12:00:00', 'utc') AS INTEGER)", [], |r| r.get(0))
            .unwrap();
        let day2 = day1 + 86_400;
        for i in 0..3 {
            library.log_listen(finished_listen("/m/a.flac", day1 + i * 250));
        }
        library.log_listen(finished_listen("/m/b.flac", day2));
        library.log_listen(ListenEvent { counted: false, listened_secs: 4.0, skipped: true, ..finished_listen("/m/b.flac", day2 + 300) });

        let recap = library.year_recap(2025, &[" feat. ".to_string()]).unwrap();
        assert_eq!(recap.plays, 4);
        assert_eq!(recap.unique_tracks, 2);
        assert_eq!(recap.top_tracks[0].file_path, "/m/a.flac");
        assert_eq!(recap.top_tracks[0].plays, 3);
        // "X feat. Y" is split, so both artists get the track's 3 plays.
        let mut leaders: Vec<&str> = recap.top_artists[..2].iter().map(|a| a.name.as_str()).collect();
        leaders.sort_unstable();
        assert_eq!(leaders, ["X", "Y"]);
        assert_eq!(recap.top_artists[2].name, "Z");
        assert_eq!(recap.top_genres[0].name, "Rock");
        assert_eq!((recap.days_listened, recap.longest_streak), (2, 2));
        assert_eq!(recap.skips, 1);
        let obsession = recap.obsession.expect("3 plays in one day");
        assert_eq!((obsession.track.file_path.as_str(), obsession.day.as_str()), ("/m/a.flac", "2025-03-10"));
        assert_eq!(recap.marathon.expect("back to back plays").tracks, 3);
        assert_eq!(recap.first_track.unwrap().file_path, "/m/a.flac");
        assert!(recap.top_tracks[0].snippet_url.is_some());
        assert_eq!(library.year_recap(2024, &[]).unwrap().plays, 0);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn library_history_records_playlist_events() {
        let dir = temp_library_dir("library_history");
        let mut library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("X"), None, None, 1);
        let playlist = library.create_playlist("Mix".to_string()).unwrap();
        library.add_track_to_playlist(playlist.id, "/m/a.flac", false).unwrap();
        library.remove_track_from_playlist(playlist.id, "/m/a.flac").unwrap();
        library.flush_history(false).unwrap();

        let kinds: Vec<(i64, Option<i64>)> = library
            .conn
            .prepare("SELECT kind, ref_id FROM library_history ORDER BY rowid")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            kinds,
            vec![
                (LibraryEventKind::PlaylistCreate as i64, Some(playlist.id)),
                (LibraryEventKind::PlaylistAdd as i64, Some(playlist.id)),
                (LibraryEventKind::PlaylistRemove as i64, Some(playlist.id)),
            ]
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn set_track_override_overlays_title_and_artist_on_get_track() {
        let dir = temp_library_dir("override_basic");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("Original"), Some("OrigArtist"), None, None, 1);

        library
            .set_track_override("/m/a.flac", Some("Custom Title"), Some("Custom Artist"), None, None, false)
            .unwrap();

        let track = library.get_track("/m/a.flac").unwrap();
        assert_eq!(track.title.as_deref(), Some("Custom Title"));
        assert_eq!(track.artist.as_deref(), Some("Custom Artist"));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn set_track_override_with_all_empty_fields_deletes_override_row() {
        let dir = temp_library_dir("override_clear_via_empty");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("Original"), Some("OrigArtist"), None, None, 1);
        library
            .set_track_override("/m/a.flac", Some("Custom"), None, None, None, false)
            .unwrap();
        assert_eq!(library.get_track("/m/a.flac").unwrap().title.as_deref(), Some("Custom"));

        // All None/empty without keep_existing_cover -> the override row should be deleted.
        library
            .set_track_override("/m/a.flac", None, None, None, None, false)
            .unwrap();
        let track = library.get_track("/m/a.flac").unwrap();
        assert_eq!(track.title.as_deref(), Some("Original"), "должен вернуться оригинал из tracks");

        let remaining: i64 = library
            .conn
            .query_row("SELECT COUNT(*) FROM track_overrides", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_all_track_overrides_removes_every_override_and_cached_accent() {
        let dir = temp_library_dir("clear_all_overrides");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), None, None, None, 1);
        insert_track_row(&library, "/m/b.flac", Some("B"), None, None, None, 2);
        library.set_track_override("/m/a.flac", Some("X"), None, None, Some("#112233"), false).unwrap();
        library.set_accent_color("/m/b.flac", "#445566").unwrap();

        let cleared = library.clear_all_track_overrides().expect("clear all");
        assert_eq!(cleared, 1, "должна была удалиться ровно одна запись overrides");

        assert_eq!(library.get_track("/m/a.flac").unwrap().title.as_deref(), Some("A"));
        assert_eq!(
            library.get_accent_color("/m/b.flac"),
            None,
            "clear_all_track_overrides должен сбрасывать accent_color и у НЕ-override треков"
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_accent_color_prefers_override_over_cached_original() {
        let dir = temp_library_dir("accent_priority");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), None, None, None, 1);
        library.set_accent_color("/m/a.flac", "#111111").unwrap();
        assert_eq!(library.get_accent_color("/m/a.flac").as_deref(), Some("#111111"));

        library
            .set_track_override("/m/a.flac", None, None, None, Some("#222222"), false)
            .unwrap();
        assert_eq!(
            library.get_accent_color("/m/a.flac").as_deref(),
            Some("#222222"),
            "override accent_color должен иметь приоритет над оригинальным"
        );

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_recently_played_orders_by_most_recent_first() {
        let dir = temp_library_dir("recently_played");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/old.flac", Some("Old"), None, None, None, 1);
        insert_track_row(&library, "/m/new.flac", Some("New"), None, None, None, 2);

        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/m/old.flac".to_string(),
                listen_count_delta: 1,
                ..Default::default()
            })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/m/new.flac".to_string(),
                listen_count_delta: 1,
                ..Default::default()
            })
            .unwrap();

        let recent = library.get_recently_played(10);
        assert_eq!(recent.first().map(|t| t.file_path.as_str()), Some("/m/new.flac"));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_most_played_orders_by_listen_count_descending() {
        let dir = temp_library_dir("most_played");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), None, None, None, 1);
        insert_track_row(&library, "/m/b.flac", Some("B"), None, None, None, 2);
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/m/a.flac".to_string(),
                listen_count_delta: 2,
                ..Default::default()
            })
            .unwrap();
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/m/b.flac".to_string(),
                listen_count_delta: 9,
                ..Default::default()
            })
            .unwrap();

        let most_played = library.get_most_played(10);
        assert_eq!(most_played.first().map(|t| t.file_path.as_str()), Some("/m/b.flac"));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_top_artists_merges_case_variants_and_sums_listens() {
        let dir = temp_library_dir("top_artists");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("Queen"), None, None, 1);
        insert_track_row(&library, "/m/b.flac", Some("B"), Some("queen"), None, None, 2);
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/m/a.flac".to_string(),
                listen_count_delta: 3,
                ..Default::default()
            })
            .unwrap();
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/m/b.flac".to_string(),
                listen_count_delta: 4,
                ..Default::default()
            })
            .unwrap();

        let top = library.get_top_artists(10, &[]);
        assert_eq!(top.len(), 1, "разные написания должны слиться в одного артиста");
        assert_eq!(top[0].listen_count, 7);
        assert_eq!(top[0].track_count, 2);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_top_artists_splits_joint_artists_by_separators() {
        let dir = temp_library_dir("top_artists_split");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), Some("frommee; maryy"), None, None, 1);
        insert_track_row(&library, "/m/b.flac", Some("B"), Some("maryy"), None, None, 2);
        for (path, n) in [("/m/a.flac", 5), ("/m/b.flac", 2)] {
            library
                .add_track_statistics_delta(TrackStatisticsDelta {
                    file_path: path.to_string(),
                    listen_count_delta: n,
                    ..Default::default()
                })
                .unwrap();
        }

        let top = library.get_top_artists(10, &[";".to_string()]);
        let names: Vec<&str> = top.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["maryy", "frommee"]);
        assert_eq!(top[0].listen_count, 7);
        assert_eq!(top[1].listen_count, 5);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_summary_stats_totals_tracks_folders_and_listens() {
        let dir = temp_library_dir("summary_stats");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        library.conn.execute("INSERT INTO folders(path) VALUES ('/music')", []).unwrap();
        insert_track_row(&library, "/m/a.flac", Some("A"), None, None, None, 1);
        insert_track_row(&library, "/m/b.flac", Some("B"), None, None, None, 2);
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/m/a.flac".to_string(),
                listen_count_delta: 2,
                total_listened_secs_delta: 100.0,
                ..Default::default()
            })
            .unwrap();

        let summary = library.get_summary_stats();
        assert_eq!(summary.total_tracks, 2);
        assert_eq!(summary.total_folders, 1);
        assert_eq!(summary.total_listens, 2);
        assert_eq!(summary.total_listened_secs, 100.0);

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_recommendations_by_mood_filters_energetic_tracks() {
        let dir = temp_library_dir("mood_energetic");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_listened_track(&library, "/m/energetic.flac", 1, (0.8, 0.7, 128.0, 3000.0, -6.0));
        insert_listened_track(&library, "/m/calm.flac", 1, (0.1, 0.2, 70.0, 1000.0, -20.0));

        let recs = library.get_recommendations_by_mood("energetic", "balanced", false, 10);
        let paths: Vec<&str> = recs.iter().map(|t| t.file_path.as_str()).collect();
        assert!(paths.contains(&"/m/energetic.flac"));
        assert!(!paths.contains(&"/m/calm.flac"));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_recommendations_by_mood_respects_lossless_only_filter() {
        let dir = temp_library_dir("mood_lossless");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        // insert_listened_track always sets format='flac', is_lossless=0 — patch
        // manually to get exactly one lossless and one lossy track.
        insert_listened_track(&library, "/m/lossless.flac", 1, (0.8, 0.7, 128.0, 3000.0, -6.0));
        library
            .conn
            .execute("UPDATE tracks SET is_lossless = 1 WHERE file_path = '/m/lossless.flac'", [])
            .unwrap();
        insert_listened_track(&library, "/m/lossy.mp3", 1, (0.8, 0.7, 128.0, 3000.0, -6.0));

        let recs = library.get_recommendations_by_mood("energetic", "balanced", true, 10);
        let paths: Vec<&str> = recs.iter().map(|t| t.file_path.as_str()).collect();
        assert!(paths.contains(&"/m/lossless.flac"));
        assert!(!paths.contains(&"/m/lossy.mp3"));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_recommendations_by_mood_forgotten_excludes_listened_tracks() {
        let dir = temp_library_dir("mood_forgotten");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/never_played.flac", Some("Never"), None, None, None, 1);
        insert_track_row(&library, "/m/played.flac", Some("Played"), None, None, None, 2);
        library
            .add_track_statistics_delta(TrackStatisticsDelta {
                file_path: "/m/played.flac".to_string(),
                listen_count_delta: 5,
                ..Default::default()
            })
            .unwrap();

        let recs = library.get_recommendations_by_mood("forgotten", "balanced", false, 10);
        let paths: Vec<&str> = recs.iter().map(|t| t.file_path.as_str()).collect();
        assert!(paths.contains(&"/m/never_played.flac"));
        assert!(!paths.contains(&"/m/played.flac"));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn taste_profile_on_empty_library_has_zero_analyzed_tracks() {
        let dir = temp_library_dir("taste_profile_empty");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        let profile = library.taste_profile();
        assert_eq!(profile.analyzed_tracks, 0);
        assert_eq!(profile.energy, None);
        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn album_track_order_round_trips_and_is_none_when_unset() {
        let dir = temp_library_dir("album_order");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        assert_eq!(library.get_album_track_order("Unknown Album").unwrap(), None);

        let order = vec!["/m/1.flac".to_string(), "/m/2.flac".to_string()];
        library.set_album_track_order("Greatest Hits", &order).unwrap();
        assert_eq!(library.get_album_track_order("Greatest Hits").unwrap(), Some(order));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn album_track_order_empty_list_reads_back_as_none() {
        let dir = temp_library_dir("album_order_empty");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        library.set_album_track_order("Some Album", &[]).unwrap();
        assert_eq!(
            library.get_album_track_order("Some Album").unwrap(),
            None,
            "пустой список треков должен читаться обратно как отсутствие порядка"
        );
        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn external_album_cache_round_trips_and_negative_cache_clears_selectively() {
        let dir = temp_library_dir("external_album_cache");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        let tracks = vec![ExternalTrack {
            position: 1,
            disc: 1,
            title: "Track One".to_string(),
            artist: "Artist".to_string(),
            duration_secs: Some(180),
        }];
        library
            .set_external_album("key-ok", Some("deezer"), "ok", &tracks, 1_700_000_000)
            .unwrap();
        library
            .set_external_album("key-none", None, "none", &[], 1_700_000_000)
            .unwrap();

        let (source, status, _fetched, got_tracks) = library.get_external_album("key-ok").unwrap();
        assert_eq!(source.as_deref(), Some("deezer"));
        assert_eq!(status, "ok");
        assert_eq!(got_tracks.len(), 1);
        assert_eq!(got_tracks[0].title, "Track One");

        assert!(library.get_external_album("key-missing").is_none());

        let cleared = library.clear_external_album_negative_cache().unwrap();
        assert_eq!(cleared, 1);
        assert!(library.get_external_album("key-none").is_none());
        assert!(library.get_external_album("key-ok").is_some(), "'ok' записи не должны затрагиваться");

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    // ── override_cover_paths / catalog_artist_title_index / album_local_tracks

    #[test]
    fn override_cover_paths_only_returns_tracks_with_nonempty_cover_sig() {
        let dir = temp_library_dir("override_cover_paths");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("A"), None, None, None, 1);
        insert_track_row(&library, "/m/b.flac", Some("B"), None, None, None, 2);
        // a.flac gets a real cover via override (data URL -> cover_sig).
        library
            .set_track_override(
                "/m/a.flac",
                None,
                None,
                Some("data:image/png;base64,aGVsbG8="),
                None,
                false,
            )
            .unwrap();
        // b.flac — title-only override, no cover.
        library.set_track_override("/m/b.flac", Some("B2"), None, None, None, false).unwrap();

        let paths = library.override_cover_paths();
        assert!(paths.contains("/m/a.flac"));
        assert!(!paths.contains("/m/b.flac"));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn has_cover_reads_cached_value_without_touching_disk() {
        let dir = temp_library_dir("has_cover_cached");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_bare_track(&library, "/m/missing.flac", 1);
        library
            .conn
            .execute(
                "UPDATE tracks SET has_cover = 1 WHERE file_path = ?1",
                params!["/m/missing.flac"],
            )
            .expect("set has_cover");

        // The path doesn't exist on disk, so a `true` result can only come from
        // the cached column — a real tag re-parse would fail and yield `false`.
        assert!(library.has_cover("/m/missing.flac"));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn has_cover_resolves_unknown_row_once_and_caches_the_result() {
        let dir = temp_library_dir("has_cover_unknown");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        // insert_bare_track doesn't set has_cover, so it lands as NULL ("unknown"),
        // matching a pre-existing row from before this column existed.
        insert_bare_track(&library, "/m/missing.flac", 1);

        assert!(
            !library.has_cover("/m/missing.flac"),
            "missing file has no tags to read, so the resolved answer is false"
        );

        let cached: i64 = library
            .conn
            .query_row(
                "SELECT has_cover FROM tracks WHERE file_path = ?1",
                params!["/m/missing.flac"],
                |row| row.get(0),
            )
            .expect("has_cover should be resolved (non-NULL) after the first call");
        assert_eq!(cached, 0, "resolved answer must be written back, not left NULL");

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cover_present_paths_excludes_unknown_and_confirmed_absent_rows() {
        let dir = temp_library_dir("cover_present_paths");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_bare_track(&library, "/m/has.flac", 1);
        insert_bare_track(&library, "/m/none.flac", 2);
        insert_bare_track(&library, "/m/unknown.flac", 3);
        library
            .conn
            .execute(
                "UPDATE tracks SET has_cover = 1 WHERE file_path = ?1",
                params!["/m/has.flac"],
            )
            .expect("set has_cover true");
        library
            .conn
            .execute(
                "UPDATE tracks SET has_cover = 0 WHERE file_path = ?1",
                params!["/m/none.flac"],
            )
            .expect("set has_cover false");
        // /m/unknown.flac stays NULL.

        let present = library.cover_present_paths();
        assert!(present.contains("/m/has.flac"));
        assert!(!present.contains("/m/none.flac"));
        assert!(!present.contains("/m/unknown.flac"));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn catalog_artist_title_index_matches_multi_artist_tracks_by_any_artist() {
        let dir = temp_library_dir("catalog_index");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(
            &library,
            "/m/collab.flac",
            Some("Collab Song"),
            Some("Artist A; Artist B"),
            Some("Some Album"),
            None,
            1,
        );

        let index = library.catalog_artist_title_index(&[]);
        let key = crate::external::title_match_key("Collab Song");
        let matches = index.get(&key).expect("должна найтись запись по нормализованному названию");
        assert_eq!(matches.len(), 1);
        assert!(matches[0].artists.contains("artist a"));
        assert!(matches[0].artists.contains("artist b"));

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_album_local_tracks_uses_override_title_and_artist_when_present() {
        let dir = temp_library_dir("album_local_tracks");
        let library = MusicLibrary::new(dir.clone()).expect("library init");
        insert_track_row(&library, "/m/a.flac", Some("Orig"), Some("OrigArtist"), Some("Album X"), None, 1);
        library
            .set_track_override("/m/a.flac", Some("Overridden"), None, None, None, false)
            .unwrap();

        let local = library.get_album_local_tracks("Album X");
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].1, "Overridden");

        drop(library);
        let _ = fs::remove_dir_all(dir);
    }
}
