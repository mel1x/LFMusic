use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

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
    pub morning_listen_count_delta: i64,
    pub day_listen_count_delta: i64,
    pub evening_listen_count_delta: i64,
    pub night_listen_count_delta: i64,
    pub total_listened_secs_delta: f64,
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
             morning_listen_count = max(0, morning_listen_count + ?10),
             day_listen_count = max(0, day_listen_count + ?11),
             evening_listen_count = max(0, evening_listen_count + ?12),
             night_listen_count = max(0, night_listen_count + ?13),
             total_listened_secs = max(0, total_listened_secs + ?14),
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
            delta.morning_listen_count_delta,
            delta.day_listen_count_delta,
            delta.evening_listen_count_delta,
            delta.night_listen_count_delta,
            delta.total_listened_secs_delta,
        ],
    )?;

    Ok(())
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
