use lf_music_lib::statistics::{
    apply_collection_full_listen, apply_track_delta, create_tables, day_part_for_hour,
    get_track_statistics, listen_threshold_secs, sum_listen_count, CollectionKind, DayPart,
    TrackStatisticsDelta,
};
use rusqlite::{params, Connection};

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db");
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE tracks (
             file_path TEXT PRIMARY KEY NOT NULL
         );",
    )
    .expect("tracks table");
    create_tables(&conn).expect("statistics tables");
    conn.execute("INSERT INTO tracks (file_path) VALUES (?1)", params!["track.flac"])
        .expect("track row");
    conn
}

#[test]
fn listen_threshold_is_min_of_ninety_seconds_or_one_third() {
    assert_eq!(listen_threshold_secs(Some(600.0)), 90.0);
    assert_eq!(listen_threshold_secs(Some(120.0)), 40.0);
    assert_eq!(listen_threshold_secs(Some(0.0)), 90.0);
    assert_eq!(listen_threshold_secs(None), 90.0);
}

#[test]
fn day_part_uses_requested_boundaries() {
    assert_eq!(day_part_for_hour(5), DayPart::Morning);
    assert_eq!(day_part_for_hour(11), DayPart::Morning);
    assert_eq!(day_part_for_hour(12), DayPart::Day);
    assert_eq!(day_part_for_hour(17), DayPart::Day);
    assert_eq!(day_part_for_hour(18), DayPart::Evening);
    assert_eq!(day_part_for_hour(22), DayPart::Evening);
    assert_eq!(day_part_for_hour(23), DayPart::Night);
    assert_eq!(day_part_for_hour(4), DayPart::Night);
}

#[test]
fn track_delta_upserts_and_clamps_playlist_removals() {
    let conn = setup_conn();

    apply_track_delta(
        &conn,
        &TrackStatisticsDelta {
            file_path: "track.flac".into(),
            early_skip_count_delta: 1,
            skip_count_delta: 1,
            listen_count_delta: 1,
            full_listen_count_delta: 1,
            playlist_add_count_delta: 1,
            search_start_count_delta: 1,
            manual_start_count_delta: 1,
            repeat_enabled_count_delta: 1,
            move_to_top_count_delta: 1,
            morning_listen_count_delta: 1,
            day_listen_count_delta: 0,
            evening_listen_count_delta: 0,
            night_listen_count_delta: 0,
            total_listened_secs_delta: 42.5,
        },
    )
    .expect("first delta");

    apply_track_delta(
        &conn,
        &TrackStatisticsDelta {
            file_path: "track.flac".into(),
            playlist_add_count_delta: -3,
            ..TrackStatisticsDelta::default()
        },
    )
    .expect("removal delta");

    let row = conn
        .query_row(
            "SELECT early_skip_count, skip_count, listen_count, full_listen_count,
                    playlist_add_count, search_start_count, manual_start_count,
                    repeat_enabled_count, move_to_top_count, morning_listen_count,
                    total_listened_secs
             FROM track_statistics
             WHERE file_path = ?1",
            params!["track.flac"],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, f64>(10)?,
                ))
            },
        )
        .expect("statistics row");

    assert_eq!(row, (1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 42.5));
}

#[test]
fn track_statistics_can_be_read_for_track_and_artist_sum() {
    let conn = setup_conn();
    conn.execute(
        "INSERT INTO tracks (file_path) VALUES (?1)",
        params!["other.flac"],
    )
    .expect("second track row");

    apply_track_delta(
        &conn,
        &TrackStatisticsDelta {
            file_path: "track.flac".into(),
            listen_count_delta: 2,
            ..TrackStatisticsDelta::default()
        },
    )
    .expect("first track listens");
    apply_track_delta(
        &conn,
        &TrackStatisticsDelta {
            file_path: "other.flac".into(),
            listen_count_delta: 3,
            full_listen_count_delta: 9,
            ..TrackStatisticsDelta::default()
        },
    )
    .expect("second track listens");

    let track = get_track_statistics(&conn, "track.flac").expect("track statistics");
    assert_eq!(track.listen_count, 2);

    let paths = vec!["track.flac".to_string(), "other.flac".to_string()];
    assert_eq!(sum_listen_count(&conn, &paths).expect("artist listen sum"), 5);
    assert_eq!(sum_listen_count(&conn, &[]).expect("empty sum"), 0);
}

#[test]
fn collection_full_listens_are_counted_by_kind_and_key() {
    let conn = setup_conn();

    apply_collection_full_listen(&conn, CollectionKind::Album, "Album A")
        .expect("album first listen");
    apply_collection_full_listen(&conn, CollectionKind::Album, "Album A")
        .expect("album second listen");
    apply_collection_full_listen(&conn, CollectionKind::Playlist, "42")
        .expect("playlist listen");

    let album_count: i64 = conn
        .query_row(
            "SELECT full_listen_count FROM collection_statistics
             WHERE collection_kind = 'album' AND collection_key = 'Album A'",
            [],
            |row| row.get(0),
        )
        .expect("album stat");
    let playlist_count: i64 = conn
        .query_row(
            "SELECT full_listen_count FROM collection_statistics
             WHERE collection_kind = 'playlist' AND collection_key = '42'",
            [],
            |row| row.get(0),
        )
        .expect("playlist stat");

    assert_eq!(album_count, 2);
    assert_eq!(playlist_count, 1);
}
