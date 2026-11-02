//! Year recap ("итоги года"): aggregates one year of `listen_history` into the
//! numbers behind the recap cards. Runs only when the recap is opened, so it
//! just loads the year's rows and folds them in memory. Rows are taken in
//! time order: insert order can differ (an unfinished session is written at
//! shutdown, imported history lands after newer rows).

use crate::library::split_artists;
use crate::statistics::{
    LISTEN_COUNTED, LISTEN_EARLY_SKIP, LISTEN_FULL, LISTEN_MOBILE, LISTEN_SEARCH_START,
    LISTEN_SHUFFLE, LISTEN_SKIPPED, SOURCE_PLAYLIST,
};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// Two plays further apart than this start a new listening session.
const SESSION_GAP_SECS: i64 = 10 * 60;
/// A track counts as an "obsession" from this many plays in one day.
const OBSESSION_MIN_PLAYS: i64 = 3;
const DISCOVERY_MIN_PLAYS: i64 = 3;

#[derive(Debug, Clone, Default, Serialize)]
pub struct RecapTrack {
    pub file_path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub accent_color: Option<String>,
    pub duration_secs: Option<f64>,
    /// false once the file was removed from the library (no cover, no playback).
    pub in_library: bool,
    pub cover_url: Option<String>,
    pub snippet_url: Option<String>,
    pub plays: i64,
    pub secs: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RecapArtist {
    pub name: String,
    pub plays: i64,
    pub secs: i64,
    pub track_count: usize,
    pub top_track: RecapTrack,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RecapAlbum {
    pub name: String,
    pub artist: Option<String>,
    pub plays: i64,
    pub secs: i64,
    pub top_track: RecapTrack,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecapGenre {
    pub name: String,
    pub plays: i64,
    /// Share of plays among plays with a known genre, 0..1.
    pub share: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecapDay {
    /// Local date, YYYY-MM-DD.
    pub day: String,
    pub secs: i64,
    pub plays: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecapObsession {
    pub track: RecapTrack,
    pub day: String,
    pub plays: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecapSession {
    pub day: String,
    pub start_hour: u32,
    pub secs: i64,
    pub tracks: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecapPlaylist {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub plays: i64,
    pub first_file_path: Option<String>,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct YearRecap {
    pub year: i32,
    pub total_secs: i64,
    /// Counted listens (passed the listen threshold).
    pub plays: i64,
    pub unique_tracks: usize,
    pub unique_artists: usize,
    pub unique_albums: usize,
    pub unique_genres: usize,
    pub days_listened: usize,
    pub longest_streak: usize,
    pub busiest_day: Option<RecapDay>,
    pub first_track: Option<RecapTrack>,
    /// Unix seconds of the first listen of the year.
    pub first_listen_at: Option<i64>,
    pub top_tracks: Vec<RecapTrack>,
    pub top_artists: Vec<RecapArtist>,
    pub top_albums: Vec<RecapAlbum>,
    pub top_genres: Vec<RecapGenre>,
    /// Listened seconds per local hour, weekday (0 = Sunday) and month.
    pub hours: Vec<i64>,
    pub weekdays: Vec<i64>,
    pub months: Vec<i64>,
    /// Counted listens between 00:00 and 05:00.
    pub night_plays: i64,
    /// Counted listens between 05:00 and 09:00.
    pub morning_plays: i64,
    pub obsession: Option<RecapObsession>,
    pub marathon: Option<RecapSession>,
    pub discovery: Option<RecapArtist>,
    /// Net tracks added to favorites this year.
    pub likes: i64,
    pub playlists_created: i64,
    pub downloads: i64,
    pub top_playlist: Option<RecapPlaylist>,
    pub skips: i64,
    pub early_skips: i64,
    pub full_listens: i64,
    pub search_starts: i64,
    /// 0..1 shares of listened time.
    pub shuffle_share: f64,
    pub mobile_share: f64,
    /// Play-weighted averages over tracks with analyzed audio features.
    pub energy: Option<f64>,
    pub danceability: Option<f64>,
    pub bpm: Option<f64>,
}

impl YearRecap {
    /// Fills cover and snippet URLs; `cover_url` resolves a library track's cover
    /// and `accent` computes a missing cover accent color (cards are tinted by it).
    pub fn attach_media(
        &mut self,
        cover_url: impl Fn(&str) -> Option<String>,
        accent: impl Fn(&str) -> Option<String>,
    ) {
        let mut tracks: Vec<&mut RecapTrack> = Vec::new();
        tracks.extend(self.first_track.as_mut());
        tracks.extend(self.top_tracks.iter_mut());
        tracks.extend(self.top_artists.iter_mut().map(|a| &mut a.top_track));
        tracks.extend(self.top_albums.iter_mut().map(|a| &mut a.top_track));
        tracks.extend(self.obsession.as_mut().map(|o| &mut o.track));
        tracks.extend(self.discovery.as_mut().map(|a| &mut a.top_track));
        for track in tracks.into_iter().filter(|t| t.in_library) {
            track.cover_url = cover_url(&track.file_path);
            track.snippet_url = Some(crate::snippets::snippet_url(&track.file_path));
            if track.accent_color.is_none() && track.cover_url.is_some() {
                track.accent_color = accent(&track.file_path);
            }
        }
        if let Some(playlist) = &mut self.top_playlist {
            playlist.cover_url = playlist.first_file_path.as_deref().and_then(&cover_url);
        }
    }
}

struct Row {
    started_at: i64,
    secs: i64,
    flags: i64,
    source_kind: i64,
    source_id: Option<i64>,
    day: String,
    day_num: i64,
    hour: u32,
    weekday: usize,
    month: usize,
    track_id: i64,
    genre: Option<String>,
    energy: Option<f64>,
    danceability: Option<f64>,
    bpm: Option<f64>,
}

#[derive(Default)]
struct Totals {
    plays: i64,
    secs: i64,
}

impl Totals {
    fn add(&mut self, row: &Row) {
        self.secs += row.secs;
        if row.flags & LISTEN_COUNTED != 0 {
            self.plays += 1;
        }
    }
}

/// Plays first, listened time as the tie-break.
fn rank<K>(map: HashMap<K, Totals>) -> Vec<(K, Totals)> {
    let mut items: Vec<(K, Totals)> = map.into_iter().filter(|(_, t)| t.plays > 0).collect();
    items.sort_by(|a, b| b.1.plays.cmp(&a.1.plays).then(b.1.secs.cmp(&a.1.secs)));
    items
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Unix seconds of local midnight on January 1st of `year` and of the next year.
/// Rows are filtered by this range instead of formatting every timestamp.
fn year_bounds(conn: &Connection, year: i32) -> rusqlite::Result<(i64, i64)> {
    conn.query_row(
        "SELECT CAST(strftime('%s', ?1, 'utc') AS INTEGER), CAST(strftime('%s', ?2, 'utc') AS INTEGER)",
        params![format!("{:04}-01-01", year), format!("{:04}-01-01", year + 1)],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

pub fn year_recap(conn: &Connection, year: i32, separators: &[String]) -> rusqlite::Result<YearRecap> {
    let (from, to) = year_bounds(conn, year)?;
    let mut tracks: HashMap<i64, RecapTrack> = HashMap::new();
    let mut rows: Vec<Row> = Vec::new();

    {
        let mut stmt = conn.prepare(
            "SELECT h.started_at, h.listened_secs, h.flags, h.source_kind, h.source_id,
                    date(h.started_at, 'unixepoch', 'localtime'),
                    CAST(julianday(date(h.started_at, 'unixepoch', 'localtime')) AS INTEGER),
                    CAST(strftime('%H', h.started_at, 'unixepoch', 'localtime') AS INTEGER),
                    CAST(strftime('%w', h.started_at, 'unixepoch', 'localtime') AS INTEGER),
                    CAST(strftime('%m', h.started_at, 'unixepoch', 'localtime') AS INTEGER),
                    ht.id, ht.file_path,
                    COALESCE(o.title, t.title, ht.title),
                    COALESCE(o.artist, t.artist, ht.artist),
                    COALESCE(t.album, ht.album),
                    COALESCE(t.genre, ht.genre),
                    COALESCE(t.duration_secs, ht.duration_secs),
                    COALESCE(o.accent_color, t.accent_color),
                    t.file_path IS NOT NULL,
                    af.energy, af.danceability, af.bpm
             FROM listen_history h
             JOIN history_tracks ht ON ht.id = h.track_id
             LEFT JOIN tracks t ON t.file_path = ht.file_path
             LEFT JOIN track_overrides o ON o.file_path = ht.file_path
             LEFT JOIN track_audio_features af ON af.file_path = ht.file_path
             WHERE h.started_at >= ?1 AND h.started_at < ?2
             ORDER BY h.started_at, h.rowid",
        )?;
        let mut query = stmt.query(params![from, to])?;
        while let Some(r) = query.next()? {
            let track_id: i64 = r.get(10)?;
            tracks.entry(track_id).or_insert_with(|| RecapTrack {
                file_path: r.get(11).unwrap_or_default(),
                title: non_empty(r.get(12).ok().flatten()),
                artist: non_empty(r.get(13).ok().flatten()),
                album: non_empty(r.get(14).ok().flatten()),
                duration_secs: r.get(16).ok().flatten(),
                accent_color: r.get(17).ok().flatten(),
                in_library: r.get(18).unwrap_or(false),
                ..RecapTrack::default()
            });
            rows.push(Row {
                started_at: r.get(0)?,
                secs: r.get(1)?,
                flags: r.get(2)?,
                source_kind: r.get(3)?,
                source_id: r.get(4)?,
                day: r.get(5)?,
                day_num: r.get(6)?,
                hour: r.get::<_, i64>(7)?.clamp(0, 23) as u32,
                weekday: r.get::<_, i64>(8)?.clamp(0, 6) as usize,
                month: (r.get::<_, i64>(9)?.clamp(1, 12) - 1) as usize,
                track_id,
                genre: non_empty(r.get(15).ok().flatten()),
                energy: r.get(19).ok().flatten(),
                danceability: r.get(20).ok().flatten(),
                bpm: r.get(21).ok().flatten(),
            });
        }
    }

    let mut recap = YearRecap {
        year,
        hours: vec![0; 24],
        weekdays: vec![0; 7],
        months: vec![0; 12],
        ..YearRecap::default()
    };
    if rows.is_empty() {
        return Ok(recap);
    }

    let mut by_track: HashMap<i64, Totals> = HashMap::new();
    // Artist key (lowercase) -> display name, totals, per-track plays.
    let mut artist_names: HashMap<String, String> = HashMap::new();
    let mut by_artist: HashMap<String, Totals> = HashMap::new();
    let mut artist_tracks: HashMap<String, HashMap<i64, i64>> = HashMap::new();
    let mut by_album: HashMap<String, Totals> = HashMap::new();
    let mut album_tracks: HashMap<String, HashMap<i64, i64>> = HashMap::new();
    let mut genre_names: HashMap<String, String> = HashMap::new();
    let mut by_genre: HashMap<String, Totals> = HashMap::new();
    let mut by_day: HashMap<String, (i64, Totals)> = HashMap::new();
    let mut track_day_plays: HashMap<(i64, String), i64> = HashMap::new();
    let mut by_playlist: HashMap<i64, Totals> = HashMap::new();
    let mut shuffle_secs = 0i64;
    let mut mobile_secs = 0i64;
    let (mut energy_sum, mut energy_n) = (0.0, 0.0);
    let (mut dance_sum, mut dance_n) = (0.0, 0.0);
    let (mut bpm_sum, mut bpm_n) = (0.0, 0.0);

    let mut session: Option<(RecapSession, i64)> = None;
    let mut marathon: Option<RecapSession> = None;
    let close_session = |session: Option<(RecapSession, i64)>, marathon: &mut Option<RecapSession>| {
        if let Some((s, _)) = session {
            if marathon.as_ref().is_none_or(|m| s.secs > m.secs) {
                *marathon = Some(s);
            }
        }
    };

    for row in &rows {
        let counted = row.flags & LISTEN_COUNTED != 0;
        let track = &tracks[&row.track_id];

        recap.total_secs += row.secs;
        recap.hours[row.hour as usize] += row.secs;
        recap.weekdays[row.weekday] += row.secs;
        recap.months[row.month] += row.secs;
        by_track.entry(row.track_id).or_default().add(row);

        let day = by_day.entry(row.day.clone()).or_insert((row.day_num, Totals::default()));
        day.1.add(row);

        if row.flags & LISTEN_SHUFFLE != 0 {
            shuffle_secs += row.secs;
        }
        if row.flags & LISTEN_MOBILE != 0 {
            mobile_secs += row.secs;
        }
        if row.flags & LISTEN_SKIPPED != 0 {
            recap.skips += 1;
        }
        if row.flags & LISTEN_EARLY_SKIP != 0 {
            recap.early_skips += 1;
        }
        if row.flags & LISTEN_FULL != 0 {
            recap.full_listens += 1;
        }
        if row.flags & LISTEN_SEARCH_START != 0 {
            recap.search_starts += 1;
        }
        if row.source_kind == SOURCE_PLAYLIST {
            if let Some(id) = row.source_id {
                by_playlist.entry(id).or_default().add(row);
            }
        }

        // Sessions: consecutive plays with short gaps, timed by what was heard.
        let ends_at = row.started_at + row.secs;
        match session.as_mut() {
            Some((s, last_end)) if row.started_at - *last_end <= SESSION_GAP_SECS => {
                s.secs += row.secs;
                s.tracks += 1;
                *last_end = (*last_end).max(ends_at);
            }
            _ => {
                close_session(session.take(), &mut marathon);
                session = Some((
                    RecapSession { day: row.day.clone(), start_hour: row.hour, secs: row.secs, tracks: 1 },
                    ends_at,
                ));
            }
        }

        if !counted {
            continue;
        }
        recap.plays += 1;
        if recap.first_track.is_none() {
            recap.first_track = Some(track.clone());
            recap.first_listen_at = Some(row.started_at);
        }
        match row.hour {
            0..=4 => recap.night_plays += 1,
            5..=8 => recap.morning_plays += 1,
            _ => {}
        }
        *track_day_plays.entry((row.track_id, row.day.clone())).or_default() += 1;

        if let Some(artist) = &track.artist {
            for name in split_artists(artist, separators) {
                let key = name.to_lowercase();
                artist_names.entry(key.clone()).or_insert(name);
                by_artist.entry(key.clone()).or_default().add(row);
                *artist_tracks.entry(key).or_default().entry(row.track_id).or_default() += 1;
            }
        }
        if let Some(album) = &track.album {
            let key = album.to_lowercase();
            by_album.entry(key.clone()).or_default().add(row);
            *album_tracks.entry(key).or_default().entry(row.track_id).or_default() += 1;
        }
        if let Some(genre) = &row.genre {
            let key = genre.to_lowercase();
            genre_names.entry(key.clone()).or_insert_with(|| genre.clone());
            by_genre.entry(key).or_default().add(row);
        }
        if let Some(v) = row.energy {
            energy_sum += v;
            energy_n += 1.0;
        }
        if let Some(v) = row.danceability {
            dance_sum += v;
            dance_n += 1.0;
        }
        if let Some(v) = row.bpm.filter(|v| *v > 0.0) {
            bpm_sum += v;
            bpm_n += 1.0;
        }
    }
    close_session(session, &mut marathon);
    recap.marathon = marathon.filter(|m| m.tracks > 1);

    let with_totals = |id: i64, totals: &Totals| RecapTrack {
        plays: totals.plays,
        secs: totals.secs,
        ..tracks.get(&id).cloned().unwrap_or_default()
    };
    let best_track = |plays: &HashMap<i64, i64>| -> RecapTrack {
        let id = plays
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(id, _)| *id)
            .unwrap_or_default();
        with_totals(id, by_track.get(&id).unwrap_or(&Totals::default()))
    };

    recap.unique_tracks = by_track.values().filter(|t| t.plays > 0).count();
    recap.unique_artists = by_artist.len();
    recap.unique_albums = by_album.len();
    recap.unique_genres = by_genre.len();
    recap.top_tracks = rank(by_track.iter().map(|(k, v)| (*k, Totals { plays: v.plays, secs: v.secs })).collect())
        .into_iter()
        .take(5)
        .map(|(id, t)| with_totals(id, &t))
        .collect();

    let artist_ranking = rank(by_artist);
    recap.top_artists = artist_ranking
        .iter()
        .take(5)
        .map(|(key, t)| RecapArtist {
            name: artist_names[key].clone(),
            plays: t.plays,
            secs: t.secs,
            track_count: artist_tracks[key].len(),
            top_track: best_track(&artist_tracks[key]),
        })
        .collect();

    recap.top_albums = rank(by_album)
        .into_iter()
        .take(3)
        .map(|(key, t)| {
            let top_track = best_track(&album_tracks[&key]);
            RecapAlbum {
                name: top_track.album.clone().unwrap_or(key),
                artist: top_track.artist.clone(),
                plays: t.plays,
                secs: t.secs,
                top_track,
            }
        })
        .collect();

    let genre_plays: i64 = by_genre.values().map(|t| t.plays).sum();
    recap.top_genres = rank(by_genre)
        .into_iter()
        .take(3)
        .map(|(key, t)| RecapGenre {
            name: genre_names[&key].clone(),
            plays: t.plays,
            share: if genre_plays > 0 { t.plays as f64 / genre_plays as f64 } else { 0.0 },
        })
        .collect();

    // Days: count, busiest, longest run of consecutive days.
    recap.days_listened = by_day.len();
    let mut day_nums: Vec<i64> = by_day.values().map(|(n, _)| *n).collect();
    day_nums.sort_unstable();
    let mut run = 0usize;
    for (i, n) in day_nums.iter().enumerate() {
        run = if i > 0 && *n == day_nums[i - 1] + 1 { run + 1 } else { 1 };
        recap.longest_streak = recap.longest_streak.max(run);
    }
    recap.busiest_day = by_day
        .iter()
        .max_by(|a, b| a.1 .1.secs.cmp(&b.1 .1.secs).then(b.0.cmp(a.0)))
        .map(|(day, (_, t))| RecapDay { day: day.clone(), secs: t.secs, plays: t.plays });

    recap.obsession = track_day_plays
        .iter()
        .filter(|(_, plays)| **plays >= OBSESSION_MIN_PLAYS)
        .max_by(|a, b| a.1.cmp(b.1).then(b.0 .1.cmp(&a.0 .1)))
        .map(|((id, day), plays)| RecapObsession {
            track: with_totals(*id, &by_track[id]),
            day: day.clone(),
            plays: *plays,
        });

    recap.discovery = find_discovery(conn, from, separators, &artist_ranking, &artist_names, &artist_tracks, &best_track)?;

    let top_playlist = rank(by_playlist).into_iter().next();
    if let Some((id, t)) = top_playlist {
        recap.top_playlist = conn
            .query_row(
                "SELECT p.name, p.kind,
                        (SELECT file_path FROM playlist_tracks WHERE playlist_id = p.id
                         ORDER BY sort_order ASC, rowid ASC LIMIT 1)
                 FROM playlists p WHERE p.id = ?1",
                params![id],
                |r| {
                    Ok(RecapPlaylist {
                        id,
                        name: r.get(0)?,
                        kind: r.get(1)?,
                        plays: t.plays,
                        first_file_path: r.get(2)?,
                        cover_url: None,
                    })
                },
            )
            .ok();
    }

    let (likes, created, downloads) = conn.query_row(
        "SELECT
             COALESCE(SUM(CASE WHEN kind = 1 AND ref_id = f.id THEN 1
                               WHEN kind = 2 AND ref_id = f.id THEN -1 ELSE 0 END), 0),
             COALESCE(SUM(kind = 3), 0),
             COALESCE(SUM(kind = 4), 0)
         FROM library_history
         LEFT JOIN (SELECT id FROM playlists WHERE kind = 'favorites' LIMIT 1) f
         WHERE at >= ?1 AND at < ?2",
        params![from, to],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
    )?;
    recap.likes = likes.max(0);
    recap.playlists_created = created;
    recap.downloads = downloads;

    if recap.total_secs > 0 {
        recap.shuffle_share = shuffle_secs as f64 / recap.total_secs as f64;
        recap.mobile_share = mobile_secs as f64 / recap.total_secs as f64;
    }
    recap.energy = (energy_n > 0.0).then(|| energy_sum / energy_n);
    recap.danceability = (dance_n > 0.0).then(|| dance_sum / dance_n);
    recap.bpm = (bpm_n > 0.0).then(|| bpm_sum / bpm_n);
    Ok(recap)
}

/// Most played artist first heard this year: no history in earlier years and
/// no listens before the history log existed (lifetime counters would be higher).
fn find_discovery(
    conn: &Connection,
    year_start: i64,
    separators: &[String],
    artist_ranking: &[(String, Totals)],
    artist_names: &HashMap<String, String>,
    artist_tracks: &HashMap<String, HashMap<i64, i64>>,
    best_track: &dyn Fn(&HashMap<i64, i64>) -> RecapTrack,
) -> rusqlite::Result<Option<RecapArtist>> {
    let candidates: Vec<&(String, Totals)> = artist_ranking
        .iter()
        .filter(|(_, t)| t.plays >= DISCOVERY_MIN_PLAYS)
        .collect();
    if candidates.len() < 2 {
        return Ok(None);
    }

    let mut known: HashSet<String> = HashSet::new();
    let mut stmt = conn.prepare(
        "SELECT DISTINCT COALESCE(o.artist, t.artist, ht.artist)
         FROM listen_history h
         JOIN history_tracks ht ON ht.id = h.track_id
         LEFT JOIN tracks t ON t.file_path = ht.file_path
         LEFT JOIN track_overrides o ON o.file_path = ht.file_path
         WHERE h.started_at < ?1",
    )?;
    for artist in stmt.query_map(params![year_start], |r| r.get::<_, Option<String>>(0))? {
        if let Some(artist) = artist? {
            known.extend(split_artists(&artist, separators).into_iter().map(|a| a.to_lowercase()));
        }
    }

    let mut lifetime: HashMap<String, i64> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT COALESCE(o.artist, t.artist), s.listen_count
         FROM track_statistics s
         JOIN tracks t ON t.file_path = s.file_path
         LEFT JOIN track_overrides o ON o.file_path = s.file_path
         WHERE s.listen_count > 0",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?)))? {
        let (artist, count) = row?;
        if let Some(artist) = artist {
            for name in split_artists(&artist, separators) {
                *lifetime.entry(name.to_lowercase()).or_default() += count;
            }
        }
    }

    // The top artist is the "artist of the year" card already.
    Ok(candidates
        .into_iter()
        .skip(1)
        .find(|(key, t)| !known.contains(key) && lifetime.get(key).copied().unwrap_or(0) <= t.plays)
        .map(|(key, t)| RecapArtist {
            name: artist_names[key].clone(),
            plays: t.plays,
            secs: t.secs,
            track_count: artist_tracks[key].len(),
            top_track: best_track(&artist_tracks[key]),
        }))
}
