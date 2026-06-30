use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
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
             ON track_statistics(listen_count);",
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
        let placeholders = std::iter::repeat("?")
            .take(chunk.len())
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── listen_threshold_secs ───────────────────────────────────────────────

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

    // ── day_part_for_hour ────────────────────────────────────────────────────

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
        // 29 % 24 == 5 -> утро, как и час 5.
        assert_eq!(day_part_for_hour(29), DayPart::Morning);
        assert_eq!(day_part_for_hour(24), day_part_for_hour(0));
    }

    // ── DB-backed: create_tables / apply_track_delta / get_track_statistics ──

    fn memory_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        // track_statistics.file_path REFERENCES tracks(file_path) — нужна хотя бы
        // заглушка таблицы tracks, иначе INSERT в track_statistics падает с
        // "no such table: main.tracks" (этот SQLite-сборка проверяет FK даже без
        // явного PRAGMA foreign_keys=ON).
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
        // Удаление трека из плейлиста на «свежей» (нулевой) статистике не должно
        // увести счётчик в минус — иначе UI показывал бы отрицательные счётчики.
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
