use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayPart {
    Morning,
    Day,
    Evening,
    Night,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CollectionKind {
    Album,
    Playlist,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TrackStatisticsDelta {
    pub file_path: String,
    pub early_skip_count_delta: i64,
    pub skip_count_delta: i64,
    pub listen_count_delta: i64,
    pub full_listen_count_delta: i64,
    pub playlist_add_count_delta: i64,
    pub search_start_count_delta: i64,
    pub manual_start_count_delta: i64,
    pub repeat_enabled_count_delta: i64,
    pub move_to_top_count_delta: i64,
    pub morning_listen_count_delta: i64,
    pub day_listen_count_delta: i64,
    pub evening_listen_count_delta: i64,
    pub night_listen_count_delta: i64,
    pub total_listened_secs_delta: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TrackStatistics {
    pub file_path: String,
    pub early_skip_count: i64,
    pub skip_count: i64,
    pub listen_count: i64,
    pub full_listen_count: i64,
    pub playlist_add_count: i64,
    pub search_start_count: i64,
    pub manual_start_count: i64,
    pub repeat_enabled_count: i64,
    pub move_to_top_count: i64,
    pub morning_listen_count: i64,
    pub day_listen_count: i64,
    pub evening_listen_count: i64,
    pub night_listen_count: i64,
    pub total_listened_secs: f64,
}

impl CollectionKind {
    fn as_str(self) -> &'static str {
        match self {
            CollectionKind::Album => "album",
            CollectionKind::Playlist => "playlist",
        }
    }
}

pub fn listen_threshold_secs(duration_secs: Option<f64>) -> f64 {
    duration_secs
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .map(|duration| 90.0_f64.min(duration / 3.0))
        .unwrap_or(90.0)
}

pub fn day_part_for_hour(hour: u32) -> DayPart {
    match hour % 24 {
        5..=11 => DayPart::Morning,
        12..=17 => DayPart::Day,
        18..=22 => DayPart::Evening,
        _ => DayPart::Night,
    }
}

pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS track_statistics (
             file_path                  TEXT PRIMARY KEY NOT NULL,
             early_skip_count           INTEGER NOT NULL DEFAULT 0,
             skip_count                 INTEGER NOT NULL DEFAULT 0,
             listen_count               INTEGER NOT NULL DEFAULT 0,
             full_listen_count          INTEGER NOT NULL DEFAULT 0,
             playlist_add_count         INTEGER NOT NULL DEFAULT 0,
             search_start_count         INTEGER NOT NULL DEFAULT 0,
             manual_start_count         INTEGER NOT NULL DEFAULT 0,
             repeat_enabled_count       INTEGER NOT NULL DEFAULT 0,
             move_to_top_count          INTEGER NOT NULL DEFAULT 0,
             morning_listen_count       INTEGER NOT NULL DEFAULT 0,
             day_listen_count           INTEGER NOT NULL DEFAULT 0,
             evening_listen_count       INTEGER NOT NULL DEFAULT 0,
             night_listen_count         INTEGER NOT NULL DEFAULT 0,
             total_listened_secs        REAL NOT NULL DEFAULT 0,
             updated_at                 INTEGER NOT NULL DEFAULT (unixepoch()),
             FOREIGN KEY (file_path) REFERENCES tracks(file_path) ON DELETE CASCADE
         );
         CREATE TABLE IF NOT EXISTS collection_statistics (
             collection_kind            TEXT NOT NULL,
             collection_key             TEXT NOT NULL,
             full_listen_count          INTEGER NOT NULL DEFAULT 0,
             updated_at                 INTEGER NOT NULL DEFAULT (unixepoch()),
             PRIMARY KEY (collection_kind, collection_key)
         );
         -- Индексы под сортировки истории/рекомендаций: без них get_recently_played
         -- (ORDER BY updated_at DESC) и get_most_played (ORDER BY listen_count DESC)
         -- на большой медиатеке сканируют всю таблицу и сортируют во временных файлах.
         CREATE INDEX IF NOT EXISTS idx_track_statistics_updated_at
             ON track_statistics(updated_at);
         CREATE INDEX IF NOT EXISTS idx_track_statistics_listen_count
             ON track_statistics(listen_count);
         -- Listening history for the year recap (see HistoryBuffer). No foreign
         -- keys on purpose: history must outlive tracks deleted from the library.
         -- No indexes either: the logs are append-only and rowid order is time
         -- order, so one yearly scan is cheap and every insert touches one page.
         CREATE TABLE IF NOT EXISTS history_tracks (
             id                         INTEGER PRIMARY KEY,
             file_path                  TEXT NOT NULL UNIQUE,
             title                      TEXT,
             artist                     TEXT,
             album                      TEXT,
             genre                      TEXT,
             duration_secs              REAL
         );
         CREATE TABLE IF NOT EXISTS listen_history (
             started_at                 INTEGER NOT NULL,
             track_id                   INTEGER NOT NULL,
             listened_secs              INTEGER NOT NULL,
             flags                      INTEGER NOT NULL DEFAULT 0,
             source_kind                INTEGER NOT NULL DEFAULT 0,
             source_id                  INTEGER
         );
         CREATE TABLE IF NOT EXISTS library_history (
             at                         INTEGER NOT NULL,
             kind                       INTEGER NOT NULL,
             track_id                   INTEGER,
             ref_id                     INTEGER
         );",
    )
}

pub fn apply_track_delta(
    conn: &Connection,
    delta: &TrackStatisticsDelta,
) -> rusqlite::Result<()> {
    if delta.file_path.trim().is_empty() {
        return Ok(());
    }

    conn.execute(
        "INSERT INTO track_statistics (file_path)
         VALUES (?1)
         ON CONFLICT(file_path) DO NOTHING",
        params![delta.file_path],
    )?;

    conn.execute(
        "UPDATE track_statistics
         SET early_skip_count = max(0, early_skip_count + ?2),
             skip_count = max(0, skip_count + ?3),
             listen_count = max(0, listen_count + ?4),
             full_listen_count = max(0, full_listen_count + ?5),
             playlist_add_count = max(0, playlist_add_count + ?6),
             search_start_count = max(0, search_start_count + ?7),
             manual_start_count = max(0, manual_start_count + ?8),
             repeat_enabled_count = max(0, repeat_enabled_count + ?9),
             move_to_top_count = max(0, move_to_top_count + ?10),
             morning_listen_count = max(0, morning_listen_count + ?11),
             day_listen_count = max(0, day_listen_count + ?12),
             evening_listen_count = max(0, evening_listen_count + ?13),
             night_listen_count = max(0, night_listen_count + ?14),
             total_listened_secs = max(0, total_listened_secs + ?15),
             updated_at = unixepoch()
         WHERE file_path = ?1",
        params![
            delta.file_path,
            delta.early_skip_count_delta,
            delta.skip_count_delta,
            delta.listen_count_delta,
            delta.full_listen_count_delta,
            delta.playlist_add_count_delta,
            delta.search_start_count_delta,
            delta.manual_start_count_delta,
            delta.repeat_enabled_count_delta,
            delta.move_to_top_count_delta,
            delta.morning_listen_count_delta,
            delta.day_listen_count_delta,
            delta.evening_listen_count_delta,
            delta.night_listen_count_delta,
            delta.total_listened_secs_delta,
        ],
    )?;

    Ok(())
}

pub fn get_track_statistics(
    conn: &Connection,
    file_path: &str,
) -> rusqlite::Result<TrackStatistics> {
    let stats = conn
        .query_row(
            "SELECT early_skip_count, skip_count, listen_count, full_listen_count,
                    playlist_add_count, search_start_count, manual_start_count,
                    repeat_enabled_count, move_to_top_count, morning_listen_count,
                    day_listen_count, evening_listen_count, night_listen_count,
                    total_listened_secs
             FROM track_statistics
             WHERE file_path = ?1",
            params![file_path],
            |row| {
                Ok(TrackStatistics {
                    file_path: file_path.to_string(),
                    early_skip_count: row.get(0)?,
                    skip_count: row.get(1)?,
                    listen_count: row.get(2)?,
                    full_listen_count: row.get(3)?,
                    playlist_add_count: row.get(4)?,
                    search_start_count: row.get(5)?,
                    manual_start_count: row.get(6)?,
                    repeat_enabled_count: row.get(7)?,
                    move_to_top_count: row.get(8)?,
                    morning_listen_count: row.get(9)?,
                    day_listen_count: row.get(10)?,
                    evening_listen_count: row.get(11)?,
                    night_listen_count: row.get(12)?,
                    total_listened_secs: row.get(13)?,
                })
            },
        )
        .optional()?;

    Ok(stats.unwrap_or_else(|| TrackStatistics {
        file_path: file_path.to_string(),
        ..TrackStatistics::default()
    }))
}

pub fn sum_listen_count(conn: &Connection, file_paths: &[String]) -> rusqlite::Result<i64> {
    if file_paths.is_empty() {
        return Ok(0);
    }

    let mut total = 0;
    for chunk in file_paths.chunks(500) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT COALESCE(SUM(listen_count), 0)
             FROM track_statistics
             WHERE file_path IN ({})",
            placeholders,
        );
        let chunk_total = conn.query_row(
            &sql,
            params_from_iter(chunk.iter().map(|file_path| file_path.as_str())),
            |row| row.get::<_, i64>(0),
        )?;
        total += chunk_total;
    }

    Ok(total)
}

pub fn apply_collection_full_listen(
    conn: &Connection,
    kind: CollectionKind,
    key: &str,
) -> rusqlite::Result<()> {
    if key.trim().is_empty() {
        return Ok(());
    }

    conn.execute(
        "INSERT INTO collection_statistics
             (collection_kind, collection_key, full_listen_count)
         VALUES (?1, ?2, 1)
         ON CONFLICT(collection_kind, collection_key) DO UPDATE SET
             full_listen_count = collection_statistics.full_listen_count + 1,
             updated_at = unixepoch()",
        params![kind.as_str(), key],
    )?;
    Ok(())
}

// ── Listening history ────────────────────────────────────────────────────────
//
// One compact row per play session (`listen_history`) plus rare library events
// (`library_history`), the raw material for year recap cards. Events are kept in
// memory and written in one transaction per batch, so playback adds no disk
// writes of its own: a batch lands about once an hour of music.

/// Bits of `listen_history.flags`.
pub const LISTEN_COUNTED: i64 = 1;
pub const LISTEN_FULL: i64 = 1 << 1;
pub const LISTEN_SKIPPED: i64 = 1 << 2;
pub const LISTEN_EARLY_SKIP: i64 = 1 << 3;
pub const LISTEN_SHUFFLE: i64 = 1 << 4;
pub const LISTEN_REPEAT_ONE: i64 = 1 << 5;
pub const LISTEN_SEARCH_START: i64 = 1 << 6;
pub const LISTEN_MANUAL_START: i64 = 1 << 7;
pub const LISTEN_SEEKED: i64 = 1 << 8;
pub const LISTEN_MOBILE: i64 = 1 << 9;

/// `listen_history.source_kind`: what the track was played from.
pub const SOURCE_OTHER: i64 = 0;
pub const SOURCE_CATALOG: i64 = 1;
pub const SOURCE_ALBUM: i64 = 2;
pub const SOURCE_ARTIST: i64 = 3;
/// `source_id` holds the playlist id (favorites included).
pub const SOURCE_PLAYLIST: i64 = 4;
pub const SOURCE_SEARCH: i64 = 5;
pub const SOURCE_MIX: i64 = 6;

const HISTORY_FLUSH_EVENTS: usize = 16;
const HISTORY_FLUSH_AGE: Duration = Duration::from_secs(20 * 60);

/// One play session as reported by the player.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ListenEvent {
    pub file_path: String,
    /// Unix seconds when the session started.
    pub started_at: i64,
    pub listened_secs: f64,
    pub counted: bool,
    pub full: bool,
    pub skipped: bool,
    pub early_skip: bool,
    pub shuffle: bool,
    pub repeat_one: bool,
    pub search_start: bool,
    pub manual_start: bool,
    pub seeked: bool,
    /// Player source id: "album:...", "playlist:12", "mix", ...
    pub source: Option<String>,
    /// false while the track is still playing (see `HistoryBuffer::open_listen`).
    pub finished: bool,
}

impl ListenEvent {
    fn flags(&self) -> i64 {
        [
            (self.counted, LISTEN_COUNTED),
            (self.full, LISTEN_FULL),
            (self.skipped, LISTEN_SKIPPED),
            (self.early_skip, LISTEN_EARLY_SKIP),
            (self.shuffle, LISTEN_SHUFFLE),
            (self.repeat_one, LISTEN_REPEAT_ONE),
            (self.search_start, LISTEN_SEARCH_START),
            (self.manual_start, LISTEN_MANUAL_START),
            (self.seeked, LISTEN_SEEKED),
            (cfg!(target_os = "android"), LISTEN_MOBILE),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .fold(0, |acc, (_, bit)| acc | bit)
    }

    fn same_session(&self, other: &ListenEvent) -> bool {
        self.file_path == other.file_path && self.started_at == other.started_at
    }
}

pub fn parse_source(source: Option<&str>) -> (i64, Option<i64>) {
    let Some(source) = source else {
        return (SOURCE_OTHER, None);
    };
    if let Some(id) = source.strip_prefix("playlist:") {
        return (SOURCE_PLAYLIST, id.parse().ok());
    }
    let kind = if source.starts_with("catalog") {
        SOURCE_CATALOG
    } else if source.starts_with("album:") {
        SOURCE_ALBUM
    } else if source.starts_with("artist:") {
        SOURCE_ARTIST
    } else if source.starts_with("search:") {
        SOURCE_SEARCH
    } else if source == "mix" {
        SOURCE_MIX
    } else {
        SOURCE_OTHER
    };
    (kind, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryEventKind {
    /// `ref_id` = playlist id (favorites is a playlist too).
    PlaylistAdd = 1,
    PlaylistRemove = 2,
    /// `ref_id` = new playlist id, no track.
    PlaylistCreate = 3,
    Downloaded = 4,
}

#[derive(Debug, Clone)]
pub struct LibraryEvent {
    pub at: i64,
    pub kind: LibraryEventKind,
    pub file_path: Option<String>,
    pub ref_id: Option<i64>,
}

#[derive(Debug, Default)]
pub struct HistoryBuffer {
    listens: Vec<ListenEvent>,
    /// Session of the track that is still playing, refreshed on every forced
    /// progress save. Written only by a shutdown flush, so closing the app
    /// mid-track keeps that listen without logging it twice otherwise.
    open_listen: Option<ListenEvent>,
    library: Vec<LibraryEvent>,
    first_buffered: Option<Instant>,
}

impl HistoryBuffer {
    /// Returns true when the buffer is due to be written.
    pub fn push_listen(&mut self, event: ListenEvent) -> bool {
        if !event.finished {
            self.open_listen = Some(event);
            return false;
        }
        if self.open_listen.as_ref().is_some_and(|open| open.same_session(&event)) {
            self.open_listen = None;
        }
        // Sub-second sessions are just clicking through the queue.
        if event.file_path.trim().is_empty() || (event.listened_secs < 1.0 && !event.skipped) {
            return false;
        }
        self.listens.push(event);
        self.mark_buffered()
    }

    pub fn push_library(&mut self, event: LibraryEvent) -> bool {
        self.library.push(event);
        self.mark_buffered()
    }

    fn mark_buffered(&mut self) -> bool {
        let first = *self.first_buffered.get_or_insert_with(Instant::now);
        self.listens.len() + self.library.len() >= HISTORY_FLUSH_EVENTS
            || first.elapsed() >= HISTORY_FLUSH_AGE
    }

    /// Drains the buffer; `include_open` also takes the unfinished session.
    pub fn take(&mut self, include_open: bool) -> (Vec<ListenEvent>, Vec<LibraryEvent>) {
        let mut listens = std::mem::take(&mut self.listens);
        if include_open {
            listens.extend(self.open_listen.take());
        }
        self.first_buffered = None;
        (listens, std::mem::take(&mut self.library))
    }
}

/// Id of the track in `history_tracks`, refreshing its metadata snapshot so the
/// recap can still name tracks removed from the library later in the year.
fn history_track_id(
    conn: &Connection,
    ids: &mut HashMap<String, i64>,
    file_path: &str,
) -> rusqlite::Result<i64> {
    if let Some(id) = ids.get(file_path) {
        return Ok(*id);
    }
    // The WHERE on the update skips the page write when nothing changed.
    conn.prepare_cached(
        "INSERT INTO history_tracks (file_path, title, artist, album, genre, duration_secs)
         SELECT t.file_path, COALESCE(o.title, t.title), COALESCE(o.artist, t.artist),
                t.album, t.genre, t.duration_secs
         FROM tracks t
         LEFT JOIN track_overrides o ON o.file_path = t.file_path
         WHERE t.file_path = ?1
         ON CONFLICT(file_path) DO UPDATE SET
             title = excluded.title,
             artist = excluded.artist,
             album = excluded.album,
             genre = excluded.genre,
             duration_secs = excluded.duration_secs
         WHERE history_tracks.title IS NOT excluded.title
            OR history_tracks.artist IS NOT excluded.artist
            OR history_tracks.album IS NOT excluded.album
            OR history_tracks.genre IS NOT excluded.genre
            OR history_tracks.duration_secs IS NOT excluded.duration_secs",
    )?
    .execute(params![file_path])?;
    conn.prepare_cached("INSERT OR IGNORE INTO history_tracks (file_path) VALUES (?1)")?
        .execute(params![file_path])?;
    let id: i64 = conn
        .prepare_cached("SELECT id FROM history_tracks WHERE file_path = ?1")?
        .query_row(params![file_path], |row| row.get(0))?;
    ids.insert(file_path.to_string(), id);
    Ok(id)
}

/// Writes a drained batch in a single transaction.
pub fn write_history(
    conn: &Connection,
    listens: &[ListenEvent],
    library: &[LibraryEvent],
) -> rusqlite::Result<()> {
    if listens.is_empty() && library.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    let mut ids = HashMap::new();

    for event in listens {
        let track_id = history_track_id(&tx, &mut ids, &event.file_path)?;
        let (source_kind, source_id) = parse_source(event.source.as_deref());
        tx.prepare_cached(
            "INSERT INTO listen_history
                 (started_at, track_id, listened_secs, flags, source_kind, source_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![
            event.started_at,
            track_id,
            event.listened_secs.max(0.0).round() as i64,
            event.flags(),
            source_kind,
            source_id,
        ])?;
    }

    for event in library {
        let track_id = match &event.file_path {
            Some(path) => Some(history_track_id(&tx, &mut ids, path)?),
            None => None,
        };
        tx.prepare_cached(
            "INSERT INTO library_history (at, kind, track_id, ref_id) VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute(params![event.at, event.kind as i64, track_id, event.ref_id])?;
    }

    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listen_threshold_defaults_to_90_without_duration() {
        assert_eq!(listen_threshold_secs(None), 90.0);
    }

    #[test]
    fn listen_threshold_uses_a_third_of_duration_when_below_cap() {
        assert_eq!(listen_threshold_secs(Some(30.0)), 10.0);
        assert_eq!(listen_threshold_secs(Some(269.9)), 269.9 / 3.0);
    }

    #[test]
    fn listen_threshold_caps_at_90_for_long_tracks() {
        assert_eq!(listen_threshold_secs(Some(1000.0)), 90.0);
        assert_eq!(listen_threshold_secs(Some(270.0)), 90.0);
    }

    #[test]
    fn listen_threshold_rejects_non_positive_or_non_finite_duration() {
        assert_eq!(listen_threshold_secs(Some(0.0)), 90.0);
        assert_eq!(listen_threshold_secs(Some(-5.0)), 90.0);
        assert_eq!(listen_threshold_secs(Some(f64::NAN)), 90.0);
        assert_eq!(listen_threshold_secs(Some(f64::INFINITY)), 90.0);
    }

    #[test]
    fn day_part_boundaries_are_correct() {
        assert_eq!(day_part_for_hour(0), DayPart::Night);
        assert_eq!(day_part_for_hour(4), DayPart::Night);
        assert_eq!(day_part_for_hour(5), DayPart::Morning);
        assert_eq!(day_part_for_hour(11), DayPart::Morning);
        assert_eq!(day_part_for_hour(12), DayPart::Day);
        assert_eq!(day_part_for_hour(17), DayPart::Day);
        assert_eq!(day_part_for_hour(18), DayPart::Evening);
        assert_eq!(day_part_for_hour(22), DayPart::Evening);
        assert_eq!(day_part_for_hour(23), DayPart::Night);
    }

    #[test]
    fn day_part_wraps_hours_above_24() {
        // 29 % 24 == 5 -> morning, same as hour 5.
        assert_eq!(day_part_for_hour(29), DayPart::Morning);
        assert_eq!(day_part_for_hour(24), day_part_for_hour(0));
    }

    fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        // track_statistics.file_path has an FK to tracks; this SQLite build
        // enforces it even without PRAGMA foreign_keys=ON, so a stub table is needed.
        conn.execute_batch("CREATE TABLE tracks (file_path TEXT PRIMARY KEY NOT NULL);")
            .expect("create stub tracks table");
        create_tables(&conn).expect("create tables");
        conn
    }

    fn stub_track(conn: &Connection, path: &str) {
        conn.execute("INSERT OR IGNORE INTO tracks (file_path) VALUES (?1)", params![path])
            .expect("insert stub track");
    }

    #[test]
    fn get_track_statistics_for_unknown_track_returns_zeroed_default() {
        let conn = memory_conn();
        let stats = get_track_statistics(&conn, "/music/unknown.flac").expect("query stats");
        assert_eq!(stats.file_path, "/music/unknown.flac");
        assert_eq!(stats.listen_count, 0);
        assert_eq!(stats.total_listened_secs, 0.0);
    }

    #[test]
    fn apply_track_delta_accumulates_across_calls() {
        let conn = memory_conn();
        let path = "/music/a.flac".to_string();
        stub_track(&conn, &path);

        apply_track_delta(
            &conn,
            &TrackStatisticsDelta {
                file_path: path.clone(),
                listen_count_delta: 1,
                total_listened_secs_delta: 120.5,
                ..Default::default()
            },
        )
        .unwrap();
        apply_track_delta(
            &conn,
            &TrackStatisticsDelta {
                file_path: path.clone(),
                listen_count_delta: 2,
                total_listened_secs_delta: 30.0,
                ..Default::default()
            },
        )
        .unwrap();

        let stats = get_track_statistics(&conn, &path).unwrap();
        assert_eq!(stats.listen_count, 3);
        assert_eq!(stats.total_listened_secs, 150.5);
    }

    #[test]
    fn apply_track_delta_clamps_counters_at_zero() {
        // Removing from a playlist on zeroed stats must not go negative, or
        // the UI would show negative counters.
        let conn = memory_conn();
        let path = "/music/b.flac".to_string();
        stub_track(&conn, &path);

        apply_track_delta(
            &conn,
            &TrackStatisticsDelta {
                file_path: path.clone(),
                playlist_add_count_delta: -1,
                skip_count_delta: -100,
                ..Default::default()
            },
        )
        .unwrap();

        let stats = get_track_statistics(&conn, &path).unwrap();
        assert_eq!(stats.playlist_add_count, 0);
        assert_eq!(stats.skip_count, 0);
    }

    #[test]
    fn apply_track_delta_ignores_empty_file_path() {
        let conn = memory_conn();
        apply_track_delta(
            &conn,
            &TrackStatisticsDelta {
                file_path: "   ".to_string(),
                listen_count_delta: 5,
                ..Default::default()
            },
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM track_statistics", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "пустой file_path не должен создавать строку");
    }

    #[test]
    fn sum_listen_count_aggregates_multiple_tracks_and_skips_missing() {
        let conn = memory_conn();
        stub_track(&conn, "/m/a.flac");
        stub_track(&conn, "/m/b.flac");
        apply_track_delta(
            &conn,
            &TrackStatisticsDelta {
                file_path: "/m/a.flac".to_string(),
                listen_count_delta: 3,
                ..Default::default()
            },
        )
        .unwrap();
        apply_track_delta(
            &conn,
            &TrackStatisticsDelta {
                file_path: "/m/b.flac".to_string(),
                listen_count_delta: 4,
                ..Default::default()
            },
        )
        .unwrap();

        let total = sum_listen_count(
            &conn,
            &[
                "/m/a.flac".to_string(),
                "/m/b.flac".to_string(),
                "/m/does-not-exist.flac".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(total, 7);
    }

    #[test]
    fn sum_listen_count_of_empty_list_is_zero() {
        let conn = memory_conn();
        assert_eq!(sum_listen_count(&conn, &[]).unwrap(), 0);
    }

    #[test]
    fn apply_collection_full_listen_increments_and_ignores_empty_key() {
        let conn = memory_conn();
        apply_collection_full_listen(&conn, CollectionKind::Album, "Greatest Hits").unwrap();
        apply_collection_full_listen(&conn, CollectionKind::Album, "Greatest Hits").unwrap();
        apply_collection_full_listen(&conn, CollectionKind::Album, "   ").unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT full_listen_count FROM collection_statistics
                 WHERE collection_kind = 'album' AND collection_key = 'Greatest Hits'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);

        let total_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM collection_statistics", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total_rows, 1, "пустой ключ не должен создавать запись");
    }
}
