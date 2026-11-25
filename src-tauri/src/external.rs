//! Free, keyless metadata sources (iTunes, Deezer, MusicBrainz/CAA, Wikipedia) for album order,
//! covers and artist avatars. Images are cached as WebP and served via `lfmusic-external://`.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::json_store;
use crate::util::{levenshtein, unix_secs};
use tauri::http::{header, Response, StatusCode};
use tauri::{AppHandle, Manager, Runtime, UriSchemeContext, UriSchemeResponder};

const USER_AGENT: &str =
    "LFMusic/0.1.0 ( https://github.com/melix/LFMusic )";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalTrack {
    pub position: u32,
    pub disc: u32,
    pub title: String,
    pub artist: String,
    pub duration_secs: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ExternalAlbum {
    pub source: String,
    pub artist: String,
    pub title: String,
    pub tracks: Vec<ExternalTrack>,
}

#[derive(Debug, Clone)]
pub struct ExternalImage {
    pub source: String,
    pub url: String,
    /// Name as returned by the source (used for match scoring).
    pub matched_name: String,
}

/// Separates "not found" from a temporary error; only the former is cached.
pub enum FetchOutcome<T> {
    Found(T),
    /// All providers genuinely reported nothing found.
    NotFound,
    /// At least one provider errored and none found a match - retry later.
    TemporaryError,
}

trait Provider: Send + Sync {
    fn name(&self) -> &'static str;

    fn album_tracklist(
        &self,
        _client: &reqwest::blocking::Client,
        _artist: &str,
        _album: &str,
    ) -> Result<Option<ExternalAlbum>, String> {
        Ok(None)
    }

    fn artist_image(
        &self,
        _client: &reqwest::blocking::Client,
        _artist: &str,
    ) -> Result<Option<ExternalImage>, String> {
        Ok(None)
    }

    fn cover(
        &self,
        _client: &reqwest::blocking::Client,
        _artist: &str,
        _album: &str,
        _title: &str,
    ) -> Result<Option<ExternalImage>, String> {
        Ok(None)
    }
}

fn client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .tls_backend_preconfigured(crate::util::http_tls())
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(6))
            .build()
            .expect("Не удалось создать HTTP-клиент")
    })
}

/// Per-host rate limiter state.
#[derive(Default)]
struct HostState {
    next_slot: Option<Instant>,
    /// Host is in backoff until this instant (after 429/503 + Retry-After).
    blocked_until: Option<Instant>,
}

fn host_registry() -> &'static Mutex<HashMap<String, HostState>> {
    static REG: OnceLock<Mutex<HashMap<String, HostState>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Per-host request gaps: MusicBrainz/CAA <= 1 req/s, iTunes ~20 req/min, Deezer 50 req/5s.
fn host_min_gap(host: &str) -> Duration {
    match host {
        "musicbrainz.org" | "coverartarchive.org" => Duration::from_millis(1100),
        "itunes.apple.com" => Duration::from_millis(3000),
        "api.deezer.com" => Duration::from_millis(140),
        _ => Duration::from_millis(120),
    }
}

/// Rough host extraction from a URL (no full URL parsing).
fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// Strict hosts (MusicBrainz/CAA) also get a per-host lock across the whole request: overlapping
/// connections got reset.
fn host_needs_serialized_requests(host: &str) -> bool {
    matches!(host, "musicbrainz.org" | "coverartarchive.org")
}

fn host_send_lock(host: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let table = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = table.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .entry(host.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// Waits until the host's next slot is free, then reserves it.
fn host_throttle(host: &str) {
    let sleep_dur = {
        let reg = host_registry();
        let mut guard = reg.lock().unwrap_or_else(|e| e.into_inner());
        let st = guard.entry(host.to_string()).or_default();
        let now = Instant::now();
        let mut earliest = st.next_slot.unwrap_or(now);
        if let Some(b) = st.blocked_until
            && b > earliest {
                earliest = b;
            }
        let target = if earliest > now { earliest } else { now };
        st.next_slot = Some(target + host_min_gap(host));
        target.saturating_duration_since(now)
    };
    if !sleep_dur.is_zero() {
        std::thread::sleep(sleep_dur);
    }
}

fn register_backoff(host: &str, secs: u64) {
    let reg = host_registry();
    let mut guard = reg.lock().unwrap_or_else(|e| e.into_inner());
    let st = guard.entry(host.to_string()).or_default();
    let until = Instant::now() + Duration::from_secs(secs);
    st.blocked_until = Some(match st.blocked_until {
        Some(b) if b > until => b,
        _ => until,
    });
}

/// Parses `Retry-After` (delta-seconds form only; default 5s, clamped).
fn retry_after_secs(resp: &reqwest::blocking::Response) -> u64 {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(5)
        .clamp(1, 120)
}

/// Sends a request honoring per-host rate limit + backoff.
fn send_throttled(
    builder: reqwest::blocking::RequestBuilder,
    host: &str,
) -> Result<reqwest::blocking::Response, String> {
    // Strict hosts: hold the per-host lock for the whole request; `lock` must outlive `_guard`.
    let lock = host_needs_serialized_requests(host).then(|| host_send_lock(host));
    let _guard = lock.as_ref().map(|l| l.lock().unwrap_or_else(|e| e.into_inner()));

    host_throttle(host);
    let resp = builder.send().map_err(|e| e.to_string())?;
    let code = resp.status().as_u16();
    if code == 429 || code == 503 {
        let secs = retry_after_secs(&resp);
        register_backoff(host, secs);
        return Err(format!("{} ответил {} (backoff {}с)", host, code, secs));
    }
    Ok(resp)
}

const SOFT_RATE_LIMIT_BACKOFF_SECS: u64 = 20;

/// Like [`send_throttled`] but decodes JSON directly.
fn send_throttled_json<T: serde::de::DeserializeOwned>(
    builder: reqwest::blocking::RequestBuilder,
    host: &str,
) -> Result<T, String> {
    let resp = send_throttled(builder, host)?;
    let text = resp.text().map_err(|e| e.to_string())?;
    if text.trim().is_empty() {
        register_backoff(host, SOFT_RATE_LIMIT_BACKOFF_SECS);
        return Err(format!("{}: пустой ответ (мягкий лимит)", host));
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn get_image(url: &str) -> Result<reqwest::blocking::Response, String> {
    let host = host_of(url);
    send_throttled(client().get(url), &host)
}

/// Canonicalizes a string for comparison: lowercase, alphanumeric/spaces only, collapsed
/// whitespace.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for ch in s.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_alphanumeric() {
            out.push(ch);
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Strips noise (feat., remastered, bracketed notes) for matching.
fn normalize_title(s: &str) -> String {
    let cleaned: String = strip_title_noise(s)
        .replace("remastered", " ")
        .replace("remaster", " ")
        .replace("original mix", " ")
        .replace("explicit", " ");
    normalize(&cleaned)
}

/// Similarity in 0.0..=1.0 (1.0 = identical after normalization).
fn similarity(a: &str, b: &str) -> f32 {
    let na = normalize(a);
    let nb = normalize(b);
    if na.is_empty() || nb.is_empty() {
        return 0.0;
    }
    if na == nb {
        return 1.0;
    }
    let ca: Vec<char> = na.chars().collect();
    let cb: Vec<char> = nb.chars().collect();
    let dist = levenshtein(&ca, &cb);
    let max_len = ca.len().max(cb.len()) as f32;
    let ratio = 1.0 - (dist as f32 / max_len);
    let contains_bonus = if na.contains(&nb) || nb.contains(&na) { 0.15 } else { 0.0 };
    (ratio + contains_bonus).clamp(0.0, 1.0)
}

fn is_placeholder_image(url: &str) -> bool {
    url.contains("/images/artist//")
        || url.contains("/images/cover//")
        || url.contains("/images/user//")
        || url.contains("d41d8cd98f00b204e9800998ecf8427e")
        || url.contains("1fecff61b191427ec4d2760a77527bf5")
        || url.contains("e460b01306de4c8d790508e0d80d1fdf")
}

/// Below this size (bytes) an image is treated as a source's placeholder.
const MIN_REAL_IMAGE_BYTES: usize = 45_000;

/// Hard cap on downloaded image size (guards against oversized/junk responses).
const MAX_DOWNLOAD_BYTES: usize = 20 * 1024 * 1024;

/// Max side length when re-encoding to WebP (downscale, like track thumbnails).
const ARTIST_IMAGE_MAX_DIM: u32 = 800;
const COVER_IMAGE_MAX_DIM: u32 = 1000;

/// How well a found album matches the query (artist + title).
fn album_score(query_artist: &str, query_album: &str, album: &ExternalAlbum) -> f32 {
    let title_sim = similarity(query_album, &album.title);
    let artist_sim = if query_artist.is_empty() {
        0.7
    } else {
        similarity(query_artist, &album.artist)
    };
    title_sim * 0.7 + artist_sim * 0.3
}

struct ITunes;

impl Provider for ITunes {
    fn name(&self) -> &'static str {
        "iTunes"
    }

    fn album_tracklist(
        &self,
        client: &reqwest::blocking::Client,
        artist: &str,
        album: &str,
    ) -> Result<Option<ExternalAlbum>, String> {
        #[derive(Deserialize)]
        struct SearchResp {
            results: Vec<Collection>,
        }
        #[derive(Deserialize)]
        struct Collection {
            #[serde(rename = "collectionId")]
            collection_id: Option<i64>,
            #[serde(rename = "collectionName")]
            collection_name: Option<String>,
            #[serde(rename = "artistName")]
            artist_name: Option<String>,
        }

        let term = format!("{} {}", artist, album);
        let search: SearchResp = send_throttled_json(
            client.get("https://itunes.apple.com/search").query(&[
                ("term", term.as_str()),
                ("entity", "album"),
                ("limit", "6"),
                ("media", "music"),
            ]),
            "itunes.apple.com",
        )?;

        let best = search
            .results
            .into_iter()
            .filter_map(|c| {
                let id = c.collection_id?;
                let name = c.collection_name?;
                let sim = similarity(album, &name);
                Some((sim, id, name, c.artist_name.unwrap_or_default()))
            })
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let Some((sim, collection_id, collection_name, collection_artist)) = best else {
            return Ok(None);
        };
        if sim < 0.45 {
            return Ok(None);
        }

        #[derive(Deserialize)]
        struct LookupResp {
            results: Vec<Entry>,
        }
        #[derive(Deserialize)]
        struct Entry {
            #[serde(rename = "wrapperType")]
            wrapper_type: Option<String>,
            #[serde(rename = "trackName")]
            track_name: Option<String>,
            #[serde(rename = "artistName")]
            artist_name: Option<String>,
            #[serde(rename = "trackNumber")]
            track_number: Option<u32>,
            #[serde(rename = "discNumber")]
            disc_number: Option<u32>,
            #[serde(rename = "trackTimeMillis")]
            track_time_millis: Option<u64>,
        }

        let lookup: LookupResp = send_throttled_json(
            client.get("https://itunes.apple.com/lookup").query(&[
                ("id", collection_id.to_string().as_str()),
                ("entity", "song"),
                ("limit", "300"),
            ]),
            "itunes.apple.com",
        )?;

        let mut tracks: Vec<ExternalTrack> = lookup
            .results
            .into_iter()
            .filter(|e| e.wrapper_type.as_deref() == Some("track"))
            .filter_map(|e| {
                let title = e.track_name?;
                Some(ExternalTrack {
                    position: e.track_number.unwrap_or(0),
                    disc: e.disc_number.unwrap_or(1),
                    title,
                    artist: e.artist_name.unwrap_or_else(|| collection_artist.clone()),
                    duration_secs: e.track_time_millis.map(|ms| (ms / 1000) as u32),
                })
            })
            .collect();

        if tracks.is_empty() {
            return Ok(None);
        }
        tracks.sort_by_key(|t| (t.disc, t.position));

        Ok(Some(ExternalAlbum {
            source: "iTunes".to_string(),
            artist: collection_artist,
            title: collection_name,
            tracks,
        }))
    }

    fn cover(
        &self,
        client: &reqwest::blocking::Client,
        artist: &str,
        album: &str,
        title: &str,
    ) -> Result<Option<ExternalImage>, String> {
        #[derive(Deserialize)]
        struct SearchResp {
            results: Vec<Hit>,
        }
        #[derive(Deserialize)]
        struct Hit {
            #[serde(rename = "collectionName")]
            collection_name: Option<String>,
            #[serde(rename = "trackName")]
            track_name: Option<String>,
            #[serde(rename = "artworkUrl100")]
            artwork_url100: Option<String>,
        }

        let by_album = !album.is_empty();
        let (entity, query, target) = if by_album {
            ("album", format!("{} {}", artist, album), album)
        } else {
            ("song", format!("{} {}", artist, title), title)
        };

        let resp: SearchResp = send_throttled_json(
            client.get("https://itunes.apple.com/search").query(&[
                ("term", query.as_str()),
                ("entity", entity),
                ("limit", "5"),
                ("media", "music"),
            ]),
            "itunes.apple.com",
        )?;

        let best = resp
            .results
            .into_iter()
            .filter_map(|h| {
                let name = if by_album { h.collection_name } else { h.track_name }?;
                let art = h.artwork_url100?;
                let sim = similarity(target, &name);
                Some((sim, name, art))
            })
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let Some((sim, name, art)) = best else {
            return Ok(None);
        };
        if sim < 0.5 {
            return Ok(None);
        }
        let url = art.replace("100x100bb", "1000x1000bb");
        Ok(Some(ExternalImage {
            source: "iTunes".to_string(),
            url,
            matched_name: name,
        }))
    }
}

struct Deezer;

impl Provider for Deezer {
    fn name(&self) -> &'static str {
        "Deezer"
    }

    fn album_tracklist(
        &self,
        client: &reqwest::blocking::Client,
        artist: &str,
        album: &str,
    ) -> Result<Option<ExternalAlbum>, String> {
        #[derive(Deserialize)]
        struct SearchResp {
            data: Vec<AlbumHit>,
        }
        #[derive(Deserialize)]
        struct AlbumHit {
            id: i64,
            title: String,
        }
        #[derive(Deserialize)]
        struct ArtistRef {
            name: String,
        }

        let q = format!("artist:\"{}\" album:\"{}\"", artist, album);
        let search: SearchResp = send_throttled(
            client
                .get("https://api.deezer.com/search/album")
                .query(&[("q", q.as_str()), ("limit", "6")]),
            "api.deezer.com",
        )?
        .json()
        .map_err(|e| e.to_string())?;

        let best = search
            .data
            .into_iter()
            .map(|a| {
                let sim = similarity(album, &a.title);
                (sim, a)
            })
            .max_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));

        let Some((sim, hit)) = best else {
            return Ok(None);
        };
        if sim < 0.45 {
            return Ok(None);
        }

        #[derive(Deserialize)]
        struct AlbumResp {
            title: String,
            artist: ArtistRef,
            tracks: TracksWrap,
        }
        #[derive(Deserialize)]
        struct TracksWrap {
            data: Vec<TrackHit>,
        }
        #[derive(Deserialize)]
        struct TrackHit {
            title: String,
            duration: Option<u32>,
            track_position: Option<u32>,
            disk_number: Option<u32>,
            artist: Option<ArtistRef>,
        }

        let album_resp: AlbumResp = send_throttled(
            client.get(format!("https://api.deezer.com/album/{}", hit.id)),
            "api.deezer.com",
        )?
        .json()
        .map_err(|e| e.to_string())?;

        let mut tracks: Vec<ExternalTrack> = album_resp
            .tracks
            .data
            .into_iter()
            .enumerate()
            .map(|(i, t)| ExternalTrack {
                position: t.track_position.unwrap_or((i + 1) as u32),
                disc: t.disk_number.unwrap_or(1),
                title: t.title,
                artist: t
                    .artist
                    .map(|a| a.name)
                    .unwrap_or_else(|| album_resp.artist.name.clone()),
                duration_secs: t.duration,
            })
            .collect();

        if tracks.is_empty() {
            return Ok(None);
        }
        tracks.sort_by_key(|t| (t.disc, t.position));

        Ok(Some(ExternalAlbum {
            source: "Deezer".to_string(),
            artist: album_resp.artist.name,
            title: album_resp.title,
            tracks,
        }))
    }

    fn artist_image(
        &self,
        client: &reqwest::blocking::Client,
        artist: &str,
    ) -> Result<Option<ExternalImage>, String> {
        #[derive(Deserialize)]
        struct SearchResp {
            data: Vec<ArtistHit>,
        }
        #[derive(Deserialize)]
        struct ArtistHit {
            name: String,
            picture_xl: Option<String>,
            picture_big: Option<String>,
        }

        let search: SearchResp = send_throttled(
            client
                .get("https://api.deezer.com/search/artist")
                .query(&[("q", artist), ("limit", "8")]),
            "api.deezer.com",
        )?
        .json()
        .map_err(|e| e.to_string())?;

        let mut best: Option<(f32, String, String)> = None;
        for a in search.data {
            let Some(url) = a.picture_xl.filter(|u| !u.is_empty()).or(a.picture_big) else {
                continue;
            };
            if url.is_empty() || is_placeholder_image(&url) {
                continue;
            }
            let sim = similarity(artist, &a.name);
            if best.as_ref().map(|(s, _, _)| sim > *s).unwrap_or(true) {
                best = Some((sim, a.name, url));
            }
        }

        let Some((sim, name, url)) = best else {
            return Ok(None);
        };
        if sim < 0.6 {
            return Ok(None);
        }
        Ok(Some(ExternalImage {
            source: "Deezer".to_string(),
            url,
            matched_name: name,
        }))
    }

    fn cover(
        &self,
        client: &reqwest::blocking::Client,
        artist: &str,
        album: &str,
        title: &str,
    ) -> Result<Option<ExternalImage>, String> {
        #[derive(Deserialize)]
        struct AlbumRef {
            cover_xl: Option<String>,
            cover_big: Option<String>,
        }
        if !album.is_empty() {
            #[derive(Deserialize)]
            struct SearchResp {
                data: Vec<Hit>,
            }
            #[derive(Deserialize)]
            struct Hit {
                title: String,
                cover_xl: Option<String>,
                cover_big: Option<String>,
            }
            let q = format!("artist:\"{}\" album:\"{}\"", artist, album);
            let resp: SearchResp = send_throttled(
                client
                    .get("https://api.deezer.com/search/album")
                    .query(&[("q", q.as_str()), ("limit", "5")]),
                "api.deezer.com",
            )?
            .json()
            .map_err(|e| e.to_string())?;
            let best = resp
                .data
                .into_iter()
                .filter_map(|h| {
                    let url = h.cover_xl.filter(|u| !u.is_empty()).or(h.cover_big)?;
                    if url.is_empty() {
                        return None;
                    }
                    let sim = similarity(album, &h.title);
                    Some((sim, h.title, url))
                })
                .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((sim, name, url)) = best
                && sim >= 0.5 {
                    return Ok(Some(ExternalImage {
                        source: "Deezer".to_string(),
                        url,
                        matched_name: name,
                    }));
                }
            return Ok(None);
        }

        #[derive(Deserialize)]
        struct TrackSearchResp {
            data: Vec<TrackHit>,
        }
        #[derive(Deserialize)]
        struct TrackHit {
            title: String,
            album: Option<AlbumRef>,
        }
        let q = format!("artist:\"{}\" track:\"{}\"", artist, title);
        let resp: TrackSearchResp = send_throttled(
            client
                .get("https://api.deezer.com/search/track")
                .query(&[("q", q.as_str()), ("limit", "5")]),
            "api.deezer.com",
        )?
        .json()
        .map_err(|e| e.to_string())?;
        let best = resp
            .data
            .into_iter()
            .filter_map(|h| {
                let alb = h.album?;
                let url = alb.cover_xl.filter(|u| !u.is_empty()).or(alb.cover_big)?;
                if url.is_empty() {
                    return None;
                }
                let sim = similarity(title, &h.title);
                Some((sim, h.title, url))
            })
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((sim, name, url)) = best
            && sim >= 0.5 {
                return Ok(Some(ExternalImage {
                    source: "Deezer".to_string(),
                    url,
                    matched_name: name,
                }));
            }
        Ok(None)
    }
}

struct MusicBrainz;

impl Provider for MusicBrainz {
    fn name(&self) -> &'static str {
        "MusicBrainz"
    }

    fn album_tracklist(
        &self,
        client: &reqwest::blocking::Client,
        artist: &str,
        album: &str,
    ) -> Result<Option<ExternalAlbum>, String> {
        #[derive(Deserialize)]
        struct SearchResp {
            releases: Vec<ReleaseHit>,
        }
        #[derive(Deserialize)]
        struct ReleaseHit {
            id: String,
            title: String,
            #[serde(rename = "artist-credit", default)]
            artist_credit: Vec<ArtistCredit>,
            #[serde(rename = "track-count")]
            track_count: Option<u32>,
        }
        #[derive(Deserialize)]
        struct ArtistCredit {
            name: String,
        }

        let query = format!("release:\"{}\" AND artist:\"{}\"", album, artist);
        let search: SearchResp = send_throttled(
            client
                .get("https://musicbrainz.org/ws/2/release/")
                .query(&[("query", query.as_str()), ("fmt", "json"), ("limit", "6")]),
            "musicbrainz.org",
        )?
        .json()
        .map_err(|e| e.to_string())?;

        let best = search
            .releases
            .into_iter()
            .map(|r| {
                let sim = similarity(album, &r.title);
                (sim, r.track_count.unwrap_or(0), r)
            })
            .filter(|(sim, _, _)| *sim >= 0.5)
            .max_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.1.cmp(&b.1))
            });

        let Some((_, _, release)) = best else {
            return Ok(None);
        };
        let release_artist = release
            .artist_credit
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_default();

        #[derive(Deserialize)]
        struct ReleaseResp {
            #[serde(default)]
            media: Vec<Medium>,
        }
        #[derive(Deserialize)]
        struct Medium {
            #[serde(default)]
            position: Option<u32>,
            #[serde(default)]
            tracks: Vec<TrackHit>,
        }
        #[derive(Deserialize)]
        struct TrackHit {
            title: String,
            position: Option<u32>,
            length: Option<u64>,
        }

        let detail: ReleaseResp = send_throttled(
            client
                .get(format!("https://musicbrainz.org/ws/2/release/{}", release.id))
                .query(&[("inc", "recordings"), ("fmt", "json")]),
            "musicbrainz.org",
        )?
        .json()
        .map_err(|e| e.to_string())?;

        let mut tracks: Vec<ExternalTrack> = Vec::new();
        for medium in &detail.media {
            let disc = medium.position.unwrap_or(1);
            for (i, t) in medium.tracks.iter().enumerate() {
                tracks.push(ExternalTrack {
                    position: t.position.unwrap_or((i + 1) as u32),
                    disc,
                    title: t.title.clone(),
                    artist: release_artist.clone(),
                    duration_secs: t.length.map(|ms| (ms / 1000) as u32),
                });
            }
        }

        if tracks.is_empty() {
            return Ok(None);
        }
        tracks.sort_by_key(|t| (t.disc, t.position));

        Ok(Some(ExternalAlbum {
            source: "MusicBrainz".to_string(),
            artist: release_artist,
            title: release.title,
            tracks,
        }))
    }
}

fn mb_release_group_id(
    client: &reqwest::blocking::Client,
    artist: &str,
    album: &str,
) -> Result<Option<(String, String)>, String> {
    #[derive(Deserialize)]
    struct Resp {
        #[serde(rename = "release-groups", default)]
        groups: Vec<Group>,
    }
    #[derive(Deserialize)]
    struct Group {
        id: String,
        title: String,
        #[serde(rename = "artist-credit", default)]
        artist_credit: Vec<Credit>,
    }
    #[derive(Deserialize)]
    struct Credit {
        name: String,
    }

    let query = format!("releasegroup:\"{}\" AND artist:\"{}\"", album, artist);
    let resp: Resp = send_throttled(
        client
            .get("https://musicbrainz.org/ws/2/release-group/")
            .query(&[("query", query.as_str()), ("fmt", "json"), ("limit", "6")]),
        "musicbrainz.org",
    )?
    .json()
    .map_err(|e| e.to_string())?;

    let best = resp
        .groups
        .into_iter()
        .filter_map(|g| {
            let sim = similarity(album, &g.title);
            let artist_ok = artist.is_empty()
                || g
                    .artist_credit
                    .first()
                    .map(|c| similarity(artist, &c.name) >= 0.4)
                    .unwrap_or(false);
            if sim >= 0.5 && artist_ok {
                Some((sim, g.id, g.title))
            } else {
                None
            }
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    Ok(best.map(|(_, id, title)| (id, title)))
}

struct CoverArtArchive;

impl Provider for CoverArtArchive {
    fn name(&self) -> &'static str {
        "Cover Art Archive"
    }

    fn cover(
        &self,
        client: &reqwest::blocking::Client,
        artist: &str,
        album: &str,
        _title: &str,
    ) -> Result<Option<ExternalImage>, String> {
        if album.trim().is_empty() {
            return Ok(None);
        }
        let Some((rg_id, title)) = mb_release_group_id(client, artist, album)? else {
            return Ok(None);
        };

        #[derive(Deserialize)]
        struct CaaResp {
            #[serde(default)]
            images: Vec<CaaImage>,
        }
        #[derive(Deserialize)]
        struct CaaImage {
            #[serde(default)]
            front: bool,
            image: String,
            #[serde(default)]
            thumbnails: Thumbs,
        }
        #[derive(Deserialize, Default)]
        struct Thumbs {
            #[serde(rename = "1200")]
            x1200: Option<String>,
            #[serde(rename = "500")]
            x500: Option<String>,
            large: Option<String>,
        }

        let resp = send_throttled(
            client.get(format!("https://coverartarchive.org/release-group/{}", rg_id)),
            "coverartarchive.org",
        )?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let caa: CaaResp = resp.json().map_err(|e| e.to_string())?;

        let Some(img) = caa.images.iter().find(|i| i.front).or_else(|| caa.images.first())
        else {
            return Ok(None);
        };
        let url = img
            .thumbnails
            .x1200
            .clone()
            .or_else(|| img.thumbnails.x500.clone())
            .or_else(|| img.thumbnails.large.clone())
            .unwrap_or_else(|| img.image.clone());

        Ok(Some(ExternalImage {
            source: "Cover Art Archive".to_string(),
            url,
            matched_name: title,
        }))
    }
}

struct Wikipedia;

/// Whether a Wikidata short description indicates a music artist.
fn is_music_description(desc: Option<&str>) -> bool {
    let Some(d) = desc else {
        return false;
    };
    let d = d.to_lowercase();
    const KW: &[&str] = &[
        "musician", "singer", "rapper", "band", "composer", "songwriter", "vocalist",
        "guitarist", "drummer", " dj", "duo", "group", "producer", "music",
        "певец", "певица", "музыкант", "группа", "рэпер", "композитор", "исполнит",
        "музыкальн", "дуэт", "диджей", "вокалист", "гитарист", "ансамбль", "бэнд",
        "рок-", "поп-", "музыка",
    ];
    KW.iter().any(|k| d.contains(k))
}

/// Looks up an artist article in one Wikipedia language edition and returns its lead photo.
fn wiki_lookup(
    client: &reqwest::blocking::Client,
    lang: &str,
    artist: &str,
) -> Result<Option<ExternalImage>, String> {
    #[derive(Deserialize)]
    struct Resp {
        query: Option<Query>,
    }
    #[derive(Deserialize)]
    struct Query {
        #[serde(default)]
        pages: HashMap<String, Page>,
    }
    #[derive(Deserialize)]
    struct Page {
        title: String,
        description: Option<String>,
        thumbnail: Option<Thumb>,
        index: Option<i64>,
    }
    #[derive(Deserialize)]
    struct Thumb {
        source: String,
    }

    let endpoint = format!("https://{}.wikipedia.org/w/api.php", lang);
    let host = format!("{}.wikipedia.org", lang);
    let resp: Resp = send_throttled(
        client.get(&endpoint).query(&[
            ("action", "query"),
            ("format", "json"),
            ("prop", "pageimages|description"),
            ("piprop", "thumbnail"),
            ("pithumbsize", "800"),
            ("generator", "search"),
            ("gsrsearch", artist),
            ("gsrlimit", "5"),
            ("gsrnamespace", "0"),
        ]),
        &host,
    )?
    .json()
    .map_err(|e| e.to_string())?;

    let Some(query) = resp.query else {
        return Ok(None);
    };

    let mut candidates: Vec<(f32, i64, String, String)> = Vec::new();
    for p in query.pages.values() {
        let Some(thumb) = &p.thumbnail else {
            continue;
        };
        if !is_music_description(p.description.as_deref()) {
            continue;
        }
        let sim = similarity(artist, &p.title);
        if sim < 0.7 {
            continue;
        }
        candidates.push((sim, p.index.unwrap_or(999), p.title.clone(), thumb.source.clone()));
    }
    candidates.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });

    Ok(candidates.into_iter().next().map(|(_, _, title, url)| ExternalImage {
        source: "Wikipedia".to_string(),
        url,
        matched_name: title,
    }))
}

impl Provider for Wikipedia {
    fn name(&self) -> &'static str {
        "Wikipedia"
    }

    fn artist_image(
        &self,
        client: &reqwest::blocking::Client,
        artist: &str,
    ) -> Result<Option<ExternalImage>, String> {
        for lang in ["en", "ru"] {
            if let Some(img) = wiki_lookup(client, lang, artist)? {
                return Ok(Some(img));
            }
        }
        Ok(None)
    }
}

fn album_providers() -> Vec<Box<dyn Provider>> {
    vec![Box::new(Deezer), Box::new(ITunes), Box::new(MusicBrainz)]
}

fn image_providers() -> Vec<Box<dyn Provider>> {
    vec![Box::new(Deezer), Box::new(Wikipedia)]
}

fn cover_providers() -> Vec<Box<dyn Provider>> {
    vec![Box::new(Deezer), Box::new(ITunes), Box::new(CoverArtArchive)]
}

/// Queries sources for a track/album cover, returns first match.
pub fn fetch_track_cover(artist: &str, album: &str, title: &str) -> FetchOutcome<ExternalImage> {
    let client = client();
    let mut had_error = false;
    for provider in cover_providers() {
        match provider.cover(client, artist, album, title) {
            Ok(Some(img)) => return FetchOutcome::Found(img),
            Ok(None) => {}
            Err(e) => {
                had_error = true;
                eprintln!("[external] {} cover error: {}", provider.name(), e);
            }
        }
    }
    if had_error {
        FetchOutcome::TemporaryError
    } else {
        FetchOutcome::NotFound
    }
}

/// Queries sources and returns the best-matching album tracklist.
pub fn fetch_album_tracklist(artist: &str, album: &str) -> FetchOutcome<ExternalAlbum> {
    let client = client();
    let mut best: Option<(f32, ExternalAlbum)> = None;
    let mut had_error = false;
    for provider in album_providers() {
        match provider.album_tracklist(client, artist, album) {
            Ok(Some(candidate)) => {
                let score = album_score(artist, album, &candidate);
                if score < 0.5 {
                    continue;
                }
                if !artist.is_empty() && similarity(artist, &candidate.artist) < 0.35 {
                    continue;
                }
                if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                    best = Some((score, candidate));
                }
                if score >= 0.97 {
                    break;
                }
            }
            Ok(None) => {}
            Err(e) => {
                had_error = true;
                eprintln!("[external] {} album error: {}", provider.name(), e);
            }
        }
    }
    match best {
        Some((_, album)) => FetchOutcome::Found(album),
        None if had_error => FetchOutcome::TemporaryError,
        None => FetchOutcome::NotFound,
    }
}

/// Queries sources for the best artist avatar.
pub fn fetch_artist_image(artist: &str) -> FetchOutcome<ExternalImage> {
    let client = client();
    let mut best: Option<(f32, ExternalImage)> = None;
    let mut had_error = false;
    for provider in image_providers() {
        match provider.artist_image(client, artist) {
            Ok(Some(img)) => {
                let score = similarity(artist, &img.matched_name);
                if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                    best = Some((score, img));
                }
                if score >= 0.85 {
                    break;
                }
            }
            Ok(None) => {}
            Err(e) => {
                had_error = true;
                eprintln!("[external] {} image error: {}", provider.name(), e);
            }
        }
    }
    match best {
        Some((_, img)) => FetchOutcome::Found(img),
        None if had_error => FetchOutcome::TemporaryError,
        None => FetchOutcome::NotFound,
    }
}

/// A track candidate found via an external source (for the download page).
#[derive(Debug, Clone, Serialize)]
pub struct ExternalSearchTrack {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_secs: Option<u32>,
    pub cover_url: Option<String>,
    pub source: String,
    /// Set by the `downloads_search` command; external search itself doesn't know.
    #[serde(default)]
    pub in_library: bool,
    /// Deezer artist id for a clickable link; `None` for iTunes results.
    #[serde(default)]
    pub artist_id: Option<u64>,
    #[serde(default)]
    pub album_id: Option<u64>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
}

/// Deezer album search result for the download page, identified by Deezer id (see
/// [`album_detail`]).
#[derive(Debug, Clone, Serialize)]
pub struct ExternalSearchAlbum {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub artist_id: Option<u64>,
    pub cover_url: Option<String>,
    pub nb_tracks: Option<u32>,
    pub source: String,
}

/// Deezer artist search result for the download page (see [`artist_detail`]).
#[derive(Debug, Clone, Serialize)]
pub struct ExternalSearchArtist {
    pub id: u64,
    pub name: String,
    pub picture_url: Option<String>,
    pub nb_album: Option<u32>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalAlbumDetail {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub artist_id: Option<u64>,
    pub cover_url: Option<String>,
    pub tracks: Vec<ExternalSearchTrack>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalArtistDetail {
    pub id: u64,
    pub name: String,
    pub picture_url: Option<String>,
    pub albums: Vec<ExternalSearchAlbum>,
    pub top_tracks: Vec<ExternalSearchTrack>,
}

/// Identity key: normalized artist+title+album. Keep in sync with `trackKey` in useDownloads.ts.
pub fn track_identity_key(artist: &str, title: &str, album: Option<&str>) -> String {
    normalize(&format!("{} {} {}", artist, title, album.unwrap_or("")))
}

/// Bracket tokens treated as noise; version markers (remix/live/...) are not listed and stay.
const TITLE_NOISE_TOKENS: &[&str] = &[
    "feat", "ft", "featuring", "prod", "produced", "official", "lyric", "lyrics",
    "visualizer", "remaster", "remastered", "explicit", "mv",
];

fn group_is_noise(buf: &str) -> bool {
    buf.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|w| TITLE_NOISE_TOKENS.contains(&w))
}

/// Strips bracketed credits (`(prod.
fn strip_title_noise(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut buf = String::new();
    let mut depth: i32 = 0;
    for ch in lower.chars() {
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                buf.push(ch);
            }
            ')' | ']' | '}' if depth > 0 => {
                buf.push(ch);
                depth -= 1;
                if depth == 0 {
                    if !group_is_noise(&buf) {
                        out.push_str(&buf);
                    }
                    buf.clear();
                }
            }
            _ => {
                if depth > 0 {
                    buf.push(ch);
                } else {
                    out.push(ch);
                }
            }
        }
    }
    if !buf.is_empty() && !group_is_noise(&buf) {
        out.push_str(&buf);
    }
    let mut base = out.as_str();
    for marker in [
        " feat.", " feat ", " ft.", " ft ", " featuring ", " prod.", " prod ",
    ] {
        if let Some(idx) = base.find(marker) {
            base = &base[..idx];
        }
    }
    base.to_string()
}

/// Normalized title key for library matching (noise stripped).
pub fn title_match_key(title: &str) -> String {
    normalize(&strip_title_noise(title))
}

/// Normalized album key; both sides of a comparison must go through it.
pub fn album_match_key(album: &str) -> String {
    normalize(album)
}

/// Separators sorted longest-first (" feat " must not split " featuring "); built once, outside the
/// hot path.
pub fn prepare_artist_separators(separators: &[String]) -> Vec<String> {
    let mut seps: Vec<String> = separators.iter().map(|x| x.to_lowercase()).collect();
    for extra in [
        "&", ",", ";", "/", " feat ", " feat. ", " ft ", " ft. ", " featuring ",
        " x ", " vs ", " vs. ", " and ", " и ",
    ] {
        seps.push(extra.to_string());
    }
    seps.sort_by_key(|s| std::cmp::Reverse(s.len()));
    seps.retain(|s| !s.is_empty());
    seps
}

pub fn split_artists_for_match_prepared(artist: &str, prepared_seps: &[String]) -> Vec<String> {
    let mut s = format!(" {} ", artist.to_lowercase());
    for sep in prepared_seps {
        s = s.replace(sep.as_str(), "\u{1}");
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for part in s.split('\u{1}') {
        let n = normalize(part);
        if !n.is_empty() && seen.insert(n.clone()) {
            out.push(n);
        }
    }
    out
}

/// Builds a Deezer advanced query with field operators `artist:"..." track:"..."`.
fn deezer_advanced_query(artist: &str, title: &str) -> String {
    let a = artist.replace('"', " ");
    let t = title.replace('"', " ");
    format!("artist:\"{}\" track:\"{}\"", a.trim(), t.trim())
}

/// Offset separating Deezer `/artist/{id}/top` indices from `/search/track` indices.
const ARTIST_SEARCH_OFFSET: u32 = 100_000;

fn artist_top_next(index: u32, raw: u32, total: u32, has_next: bool) -> (u32, bool) {
    if (has_next || index + raw < total) && raw > 0 {
        (index + raw, true)
    } else {
        (ARTIST_SEARCH_OFFSET, true)
    }
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: serde::Deserialize<'de>"))]
struct DeezerPageResp<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
    #[serde(default)]
    total: u32,
    next: Option<String>,
    #[serde(default)]
    error: Option<serde_json::Value>,
}

fn deezer_retry_json<T>(
    build_req: impl Fn() -> reqwest::blocking::RequestBuilder,
) -> Result<Option<DeezerPageResp<T>>, String>
where
    T: serde::de::DeserializeOwned,
{
    for attempt in 0..2 {
        let resp = send_throttled(build_req(), "api.deezer.com")?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let parsed: DeezerPageResp<T> = resp.json().map_err(|e| e.to_string())?;
        if parsed.error.is_none() {
            return Ok(Some(parsed));
        }
        if attempt == 0 {
            std::thread::sleep(Duration::from_millis(600));
        }
    }
    Err("Deezer API error".to_string())
}

/// Pagination uses the raw Deezer count: filtering would zero `has_more` too early.
fn deezer_search_tracks(
    client: &reqwest::blocking::Client,
    query: &str,
    index: u32,
    limit: u32,
) -> Result<(Vec<ExternalSearchTrack>, u32, bool), String> {
    #[derive(Deserialize)]
    struct Hit {
        title: String,
        duration: Option<u32>,
        artist: Option<ArtistRef>,
        album: Option<AlbumRef>,
    }
    #[derive(Deserialize)]
    struct ArtistRef {
        id: Option<u64>,
        name: String,
    }
    #[derive(Deserialize)]
    struct AlbumRef {
        id: Option<u64>,
        title: Option<String>,
        cover_xl: Option<String>,
        cover_big: Option<String>,
        cover_medium: Option<String>,
    }

    let idx_str = index.to_string();
    let lim_str = limit.to_string();
    let resp: DeezerPageResp<Hit> = deezer_retry_json(|| {
        client.get("https://api.deezer.com/search/track").query(&[
            ("q", query),
            ("index", idx_str.as_str()),
            ("limit", lim_str.as_str()),
        ])
    })?
    .ok_or_else(|| "Deezer HTTP error".to_string())?;

    let raw_count = resp.data.len() as u32;
    let has_next = (resp.next.is_some() || index + raw_count < resp.total) && raw_count > 0;
    let tracks = resp
        .data
        .into_iter()
        .filter_map(|h| {
            let (artist, artist_id) = match h.artist {
                Some(a) => (a.name, a.id),
                None => (String::new(), None),
            };
            if artist.is_empty() {
                return None;
            }
            let (album, album_id, cover_url) = match h.album {
                Some(a) => (
                    a.title.filter(|t| !t.is_empty()),
                    a.id,
                    a.cover_xl
                        .filter(|u| !u.is_empty())
                        .or(a.cover_big)
                        .or(a.cover_medium)
                        .filter(|u| !u.is_empty()),
                ),
                None => (None, None, None),
            };
            Some(ExternalSearchTrack {
                title: h.title,
                artist,
                album,
                duration_secs: h.duration,
                cover_url,
                source: "Deezer".to_string(),
                in_library: false,
                artist_id,
                album_id,
                genre: None,
                source_url: None,
            })
        })
        .collect();
    Ok((tracks, raw_count, has_next))
}

fn pick_image(candidates: &[Option<String>]) -> Option<String> {
    candidates
        .iter()
        .flatten()
        .find(|u| !u.is_empty() && !is_placeholder_image(u))
        .cloned()
}

fn deezer_search_albums(
    client: &reqwest::blocking::Client,
    query: &str,
    limit: u32,
) -> Result<Vec<ExternalSearchAlbum>, String> {
    #[derive(Deserialize)]
    struct SearchResp {
        data: Vec<Hit>,
    }
    #[derive(Deserialize)]
    struct Hit {
        id: u64,
        title: String,
        cover_xl: Option<String>,
        cover_big: Option<String>,
        cover_medium: Option<String>,
        nb_tracks: Option<u32>,
        artist: Option<ArtistRef>,
    }
    #[derive(Deserialize)]
    struct ArtistRef {
        id: Option<u64>,
        name: Option<String>,
    }

    let resp: SearchResp = send_throttled(
        client.get("https://api.deezer.com/search/album").query(&[
            ("q", query),
            ("limit", limit.to_string().as_str()),
        ]),
        "api.deezer.com",
    )?
    .json()
    .map_err(|e| e.to_string())?;

    Ok(resp
        .data
        .into_iter()
        .map(|h| {
            let (artist, artist_id) = match h.artist {
                Some(a) => (a.name.unwrap_or_default(), a.id),
                None => (String::new(), None),
            };
            ExternalSearchAlbum {
                id: h.id,
                title: h.title,
                artist,
                artist_id,
                cover_url: pick_image(&[h.cover_xl, h.cover_big, h.cover_medium]),
                nb_tracks: h.nb_tracks,
                source: "Deezer".to_string(),
            }
        })
        .collect())
}

fn deezer_search_artists(
    client: &reqwest::blocking::Client,
    query: &str,
    limit: u32,
) -> Result<Vec<ExternalSearchArtist>, String> {
    #[derive(Deserialize)]
    struct SearchResp {
        data: Vec<Hit>,
    }
    #[derive(Deserialize)]
    struct Hit {
        id: u64,
        name: String,
        picture_xl: Option<String>,
        picture_big: Option<String>,
        picture_medium: Option<String>,
        nb_album: Option<u32>,
    }

    let resp: SearchResp = send_throttled(
        client.get("https://api.deezer.com/search/artist").query(&[
            ("q", query),
            ("limit", limit.to_string().as_str()),
        ]),
        "api.deezer.com",
    )?
    .json()
    .map_err(|e| e.to_string())?;

    Ok(resp
        .data
        .into_iter()
        .map(|h| ExternalSearchArtist {
            id: h.id,
            name: h.name,
            picture_url: pick_image(&[h.picture_xl, h.picture_big, h.picture_medium]),
            nb_album: h.nb_album,
            source: "Deezer".to_string(),
        })
        .collect())
}

/// `Ok(None)` = album not found; `Err` = network/parse error.
pub fn album_detail(id: u64) -> Result<Option<ExternalAlbumDetail>, String> {
    let client = client();
    #[derive(Deserialize)]
    struct AlbumResp {
        id: u64,
        title: String,
        cover_xl: Option<String>,
        cover_big: Option<String>,
        cover_medium: Option<String>,
        artist: Option<ArtistRef>,
        tracks: Option<TracksWrap>,
    }
    #[derive(Deserialize)]
    struct ArtistRef {
        id: Option<u64>,
        name: Option<String>,
    }
    #[derive(Deserialize)]
    struct TracksWrap {
        #[serde(default)]
        data: Vec<TrackHit>,
    }
    #[derive(Deserialize)]
    struct TrackHit {
        title: String,
        duration: Option<u32>,
        track_position: Option<u32>,
        disk_number: Option<u32>,
        artist: Option<ArtistRef>,
    }

    let resp = send_throttled(
        client.get(format!("https://api.deezer.com/album/{}", id)),
        "api.deezer.com",
    )?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let album: AlbumResp = resp.json().map_err(|e| e.to_string())?;
    let (album_artist, album_artist_id) = match album.artist {
        Some(a) => (a.name.unwrap_or_default(), a.id),
        None => (String::new(), None),
    };
    let cover_url = pick_image(&[album.cover_xl, album.cover_big, album.cover_medium]);

    let mut indexed: Vec<(u32, u32, ExternalSearchTrack)> = album
        .tracks
        .map(|w| w.data)
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let (artist, artist_id) = match t.artist {
                Some(a) => (
                    a.name.filter(|n| !n.is_empty()).unwrap_or_else(|| album_artist.clone()),
                    a.id.or(album_artist_id),
                ),
                None => (album_artist.clone(), album_artist_id),
            };
            let disc = t.disk_number.unwrap_or(1);
            let pos = t.track_position.unwrap_or((i + 1) as u32);
            (
                disc,
                pos,
                ExternalSearchTrack {
                    title: t.title,
                    artist,
                    album: Some(album.title.clone()),
                    duration_secs: t.duration,
                    cover_url: cover_url.clone(),
                    source: "Deezer".to_string(),
                    in_library: false,
                    artist_id,
                    album_id: Some(album.id),
                    genre: None,
                    source_url: None,
                },
            )
        })
        .collect();
    indexed.sort_by_key(|(disc, pos, _)| (*disc, *pos));
    let tracks = indexed.into_iter().map(|(_, _, t)| t).collect();

    Ok(Some(ExternalAlbumDetail {
        id: album.id,
        title: album.title,
        artist: album_artist,
        artist_id: album_artist_id,
        cover_url,
        tracks,
    }))
}

pub fn deezer_album_genre(album_id: u64) -> Option<String> {
    #[derive(Deserialize)]
    struct AlbumResp {
        genres: Option<GenresWrap>,
    }
    #[derive(Deserialize)]
    struct GenresWrap {
        #[serde(default)]
        data: Vec<GenreHit>,
    }
    #[derive(Deserialize)]
    struct GenreHit {
        name: String,
    }

    let resp = send_throttled(
        client().get(format!("https://api.deezer.com/album/{}", album_id)),
        "api.deezer.com",
    )
    .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let album: AlbumResp = resp.json().ok()?;
    album
        .genres?
        .data
        .into_iter()
        .map(|g| g.name)
        .find(|n| !n.is_empty() && n != "All")
}

/// `Ok(None)` = artist not found; `Err` = network/parse error.
pub fn artist_detail(id: u64) -> Result<Option<ExternalArtistDetail>, String> {
    let client = client();

    #[derive(Deserialize)]
    struct ArtistResp {
        id: u64,
        name: String,
        picture_xl: Option<String>,
        picture_big: Option<String>,
        picture_medium: Option<String>,
    }
    let resp = send_throttled(
        client.get(format!("https://api.deezer.com/artist/{}", id)),
        "api.deezer.com",
    )?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let artist: ArtistResp = resp.json().map_err(|e| e.to_string())?;
    let picture_url = pick_image(&[artist.picture_xl, artist.picture_big, artist.picture_medium]);

    #[derive(Deserialize)]
    struct AlbumsResp {
        #[serde(default)]
        data: Vec<AlbumHit>,
    }
    #[derive(Deserialize)]
    struct AlbumHit {
        id: u64,
        title: String,
        cover_xl: Option<String>,
        cover_big: Option<String>,
        cover_medium: Option<String>,
        nb_tracks: Option<u32>,
    }
    let albums = match send_throttled(
        client
            .get(format!("https://api.deezer.com/artist/{}/albums", id))
            .query(&[("limit", "50")]),
        "api.deezer.com",
    ) {
        Ok(r) if r.status().is_success() => {
            let parsed: AlbumsResp = r.json().map_err(|e| e.to_string())?;
            parsed
                .data
                .into_iter()
                .map(|a| ExternalSearchAlbum {
                    id: a.id,
                    title: a.title,
                    artist: artist.name.clone(),
                    artist_id: Some(artist.id),
                    cover_url: pick_image(&[a.cover_xl, a.cover_big, a.cover_medium]),
                    nb_tracks: a.nb_tracks,
                    source: "Deezer".to_string(),
                })
                .collect()
        }
        _ => Vec::new(),
    };

    let top_tracks = match artist_tracks(id, Some(&artist.name), 0, 20) {
        Ok(page) => page.tracks,
        Err(e) => {
            eprintln!("[external] deezer artist tracks error: {}", e);
            Vec::new()
        }
    };

    Ok(Some(ExternalArtistDetail {
        id: artist.id,
        name: artist.name,
        picture_url,
        albums,
        top_tracks,
    }))
}

/// Artists whose `/top` came back empty (Deezer geo-restriction) page through name search instead.
fn top_unavailable() -> &'static Mutex<std::collections::HashSet<u64>> {
    static S: OnceLock<Mutex<std::collections::HashSet<u64>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// One page of an artist's tracks, most popular first.
pub fn artist_tracks(
    id: u64,
    name: Option<&str>,
    index: u32,
    limit: u32,
) -> Result<ExternalSearchPage, String> {
    let client = client();
    let limit = limit.clamp(1, 100);
    let use_top = !top_unavailable().lock().map(|s| s.contains(&id)).unwrap_or(false);

    if index < ARTIST_SEARCH_OFFSET && use_top {
        #[derive(Deserialize)]
        struct TopHit {
            title: String,
            duration: Option<u32>,
            artist: Option<ArtistRef>,
            album: Option<AlbumRef>,
        }
        #[derive(Deserialize)]
        struct ArtistRef {
            id: Option<u64>,
            name: Option<String>,
        }
        #[derive(Deserialize)]
        struct AlbumRef {
            id: Option<u64>,
            title: Option<String>,
            cover_xl: Option<String>,
            cover_big: Option<String>,
            cover_medium: Option<String>,
        }
        let top_url = format!("https://api.deezer.com/artist/{}/top", id);
        let idx_str = index.to_string();
        let lim_str = limit.to_string();
        if let Some(parsed) = deezer_retry_json::<TopHit>(|| {
            client
                .get(&top_url)
                .query(&[("index", idx_str.as_str()), ("limit", lim_str.as_str())])
        })? && (parsed.total > 0 || !parsed.data.is_empty())
        {
            let raw = parsed.data.len() as u32;
            let (next_index, has_more) =
                artist_top_next(index, raw, parsed.total, parsed.next.is_some());
            let tracks = parsed
                .data
                .into_iter()
                .map(|t| {
                    let (album, album_id, cover_url) = match t.album {
                        Some(a) => (
                            a.title.filter(|t| !t.is_empty()),
                            a.id,
                            pick_image(&[a.cover_xl, a.cover_big, a.cover_medium]),
                        ),
                        None => (None, None, None),
                    };
                    let artist = t
                        .artist
                        .and_then(|a| a.name.filter(|n| !n.is_empty()).map(|n| (n, a.id)));
                    let (artist, artist_id) = match artist {
                        Some((n, aid)) => (n, aid.or(Some(id))),
                        None => (name.unwrap_or_default().to_string(), Some(id)),
                    };
                    ExternalSearchTrack {
                        title: t.title,
                        artist,
                        album,
                        duration_secs: t.duration,
                        cover_url,
                        source: "Deezer".to_string(),
                        in_library: false,
                        artist_id,
                        album_id,
                        genre: None,
                        source_url: None,
                    }
                })
                .collect();
            return Ok(ExternalSearchPage {
                tracks,
                albums: Vec::new(),
                artists: Vec::new(),
                next_index,
                has_more,
            });
        }
        if index == 0
            && let Ok(mut s) = top_unavailable().lock()
        {
            s.insert(id);
        }
    }

    let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => {
            #[derive(Deserialize)]
            struct ArtistResp {
                name: String,
            }
            let resp = send_throttled(
                client.get(format!("https://api.deezer.com/artist/{}", id)),
                "api.deezer.com",
            )?;
            if !resp.status().is_success() {
                return Err(format!("artist {} not found", id));
            }
            resp.json::<ArtistResp>().map_err(|e| e.to_string())?.name
        }
    };

    // A short first page may not overflow the view: keep pulling pages until it can scroll.
    const MAX_REQUESTS: u32 = 4;
    let batch = limit.max(50);
    let mut tracks: Vec<ExternalSearchTrack> = Vec::new();
    let mut cursor = index.saturating_sub(ARTIST_SEARCH_OFFSET);
    let mut has_more = true;
    for _ in 0..MAX_REQUESTS {
        let (hits, raw, has_next) = deezer_search_tracks(client, &name, cursor, batch)?;
        cursor += raw;
        has_more = has_next;
        tracks.extend(hits.into_iter().filter(|t| t.artist_id == Some(id)));
        if !has_more || tracks.len() as u32 >= limit / 2 {
            break;
        }
    }
    Ok(ExternalSearchPage {
        tracks,
        albums: Vec::new(),
        artists: Vec::new(),
        next_index: ARTIST_SEARCH_OFFSET + cursor,
        has_more,
    })
}

fn itunes_search_tracks(
    client: &reqwest::blocking::Client,
    query: &str,
    limit: u32,
) -> Result<Vec<ExternalSearchTrack>, String> {
    #[derive(Deserialize)]
    struct SearchResp {
        results: Vec<Hit>,
    }
    #[derive(Deserialize)]
    struct Hit {
        #[serde(rename = "trackName")]
        track_name: Option<String>,
        #[serde(rename = "artistName")]
        artist_name: Option<String>,
        #[serde(rename = "collectionName")]
        collection_name: Option<String>,
        #[serde(rename = "artworkUrl100")]
        artwork_url100: Option<String>,
        #[serde(rename = "trackTimeMillis")]
        track_time_millis: Option<u64>,
        #[serde(rename = "primaryGenreName")]
        primary_genre_name: Option<String>,
    }

    let resp: SearchResp = send_throttled_json(
        client.get("https://itunes.apple.com/search").query(&[
            ("term", query),
            ("entity", "song"),
            ("limit", limit.to_string().as_str()),
            ("media", "music"),
        ]),
        "itunes.apple.com",
    )?;

    Ok(resp
        .results
        .into_iter()
        .filter_map(|h| {
            let title = h.track_name?;
            let artist = h.artist_name.unwrap_or_default();
            if artist.is_empty() {
                return None;
            }
            let cover_url = h
                .artwork_url100
                .map(|u| u.replace("100x100bb", "600x600bb"));
            let genre = h
                .primary_genre_name
                .filter(|g| !g.is_empty() && g != "Music");
            Some(ExternalSearchTrack {
                title,
                artist,
                album: h.collection_name.filter(|t| !t.is_empty()),
                duration_secs: h.track_time_millis.map(|ms| (ms / 1000) as u32),
                cover_url,
                source: "iTunes".to_string(),
                in_library: false,
                artist_id: None,
                album_id: None,
                genre,
                source_url: None,
            })
        })
        .collect())
}

/// The pagination cursor counts only Deezer results.
#[derive(Debug, Clone, Serialize)]
pub struct ExternalSearchPage {
    pub tracks: Vec<ExternalSearchTrack>,
    /// Populated only on the first page (`index == 0`).
    #[serde(default)]
    pub albums: Vec<ExternalSearchAlbum>,
    #[serde(default)]
    pub artists: Vec<ExternalSearchArtist>,
    pub next_index: u32,
    pub has_more: bool,
}

/// Deezer is the primary paginated source.
pub fn search_tracks(
    query: &str,
    artist: Option<&str>,
    title: Option<&str>,
    index: u32,
    limit: u32,
) -> ExternalSearchPage {
    let q = query.trim();
    if q.is_empty() {
        return ExternalSearchPage {
            tracks: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
            next_index: index,
            has_more: false,
        };
    }
    let client = client();

    let (mut results, deezer_raw, deezer_has_more) = match deezer_search_tracks(client, q, index, limit) {
        Ok(triple) => triple,
        Err(e) => {
            eprintln!("[external] deezer track search error: {}", e);
            (Vec::new(), 0, false)
        }
    };

    let next_index = index + deezer_raw;
    let has_more = deezer_has_more;

    if index == 0
        && let (Some(a), Some(t)) = (artist, title) {
            let (a, t) = (a.trim(), t.trim());
            if !a.is_empty() && !t.is_empty() {
                let adv = deezer_advanced_query(a, t);
                match deezer_search_tracks(client, &adv, 0, limit) {
                    Ok((exact, _, _)) if !exact.is_empty() => {
                        let mut seen: std::collections::HashSet<String> = exact
                            .iter()
                            .map(|t| track_identity_key(&t.artist, &t.title, t.album.as_deref()))
                            .collect();
                        let tail = std::mem::replace(&mut results, exact);
                        for t in tail {
                            if seen.insert(track_identity_key(&t.artist, &t.title, t.album.as_deref())) {
                                results.push(t);
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("[external] deezer advanced search error: {}", e),
                }
            }
        }

    if index == 0 && deezer_raw < limit {
        match itunes_search_tracks(client, q, limit) {
            Ok(itunes) => {
                let mut seen: std::collections::HashSet<String> = results
                    .iter()
                    .map(|t| track_identity_key(&t.artist, &t.title, t.album.as_deref()))
                    .collect();
                for t in itunes {
                    if seen.insert(track_identity_key(&t.artist, &t.title, t.album.as_deref())) {
                        results.push(t);
                    }
                }
            }
            Err(e) => eprintln!("[external] itunes track search error: {}", e),
        }
    }

    let (albums, artists) = if index == 0 {
        let albums = deezer_search_albums(client, q, 12).unwrap_or_else(|e| {
            eprintln!("[external] deezer album search error: {}", e);
            Vec::new()
        });
        let artists = deezer_search_artists(client, q, 12).unwrap_or_else(|e| {
            eprintln!("[external] deezer artist search error: {}", e);
            Vec::new()
        });
        (albums, artists)
    } else {
        (Vec::new(), Vec::new())
    };

    ExternalSearchPage { tracks: results, albums, artists, next_index, has_more }
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumLayout {
    /// "ok" = external order found; "none" = no external data.
    pub status: String,
    pub source: Option<String>,
    pub entries: Vec<AlbumLayoutEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlbumLayoutEntry {
    pub position: u32,
    pub disc: u32,
    pub title: String,
    pub artist: String,
    pub duration_secs: Option<u32>,
    /// `None` = "phantom" track not in the local library.
    pub file_path: Option<String>,
}

/// Maps external tracks onto local (file_path, title) pairs, returning the canonical-order layout.
pub fn build_layout(
    tracks: &[ExternalTrack],
    local: &[(String, String)],
) -> Vec<AlbumLayoutEntry> {
    let mut by_title: HashMap<String, Vec<String>> = HashMap::new();
    for (path, title) in local {
        by_title
            .entry(normalize_title(title))
            .or_default()
            .push(path.clone());
    }

    let mut out = Vec::with_capacity(tracks.len());
    for t in tracks {
        let key = normalize_title(&t.title);
        let file_path = by_title
            .get_mut(&key)
            .and_then(|paths| if paths.is_empty() { None } else { Some(paths.remove(0)) });
        out.push(AlbumLayoutEntry {
            position: t.position,
            disc: t.disc,
            title: t.title.clone(),
            artist: t.artist.clone(),
            duration_secs: t.duration_secs,
            file_path,
        });
    }
    out
}

const SCHEME: &str = "lfmusic-external";

fn scheme_origin() -> String {
    #[cfg(any(windows, target_os = "android"))]
    {
        format!("http://{}.localhost", SCHEME)
    }
    #[cfg(not(any(windows, target_os = "android")))]
    {
        format!("{}://localhost", SCHEME)
    }
}

fn external_protocol_url(kind: &str, hex: &str, version: u64) -> String {
    format!("{}/{}/{}?v={}", scheme_origin(), kind, hex, version)
}

struct ImageCache {
    /// URL path segment of the `lfmusic-external://` protocol.
    kind: &'static str,
    /// Base name of the files dir, `<name>.json` manifest and `<name>.version`.
    name: &'static str,
    /// Bumping this wipes the disk cache once (e.g. on storage format changes).
    version: u32,
    /// Distinguishes cover files from artist avatar files.
    file_prefix: &'static str,
    max_dim: u32,
    /// Placeholder-size threshold, see [`download_validate_encode`].
    min_real_bytes: Option<usize>,
    label: &'static str,
    /// Serializes manifest read-modify-write across prep threads; downloads stay outside the lock.
    lock: Mutex<()>,
}

static ARTIST_IMAGES: ImageCache = ImageCache {
    kind: "artist",
    name: "external_artists",
    version: 5,
    file_prefix: "",
    max_dim: ARTIST_IMAGE_MAX_DIM,
    min_real_bytes: Some(MIN_REAL_IMAGE_BYTES),
    label: "artist image",
    lock: Mutex::new(()),
};

static TRACK_COVERS: ImageCache = ImageCache {
    kind: "cover",
    name: "external_covers",
    version: 1,
    file_prefix: "c",
    max_dim: COVER_IMAGE_MAX_DIM,
    min_real_bytes: None,
    label: "cover",
    lock: Mutex::new(()),
};

fn key_hex(key: &str) -> String {
    format!("{:016x}", key_hash(key))
}

fn not_found() -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(Vec::new())
        .unwrap()
}

fn build_external_response<R: Runtime>(
    app: &AppHandle<R>,
    uri: &tauri::http::Uri,
) -> Response<Vec<u8>> {
    let mut segs = uri.path().trim_matches('/').splitn(2, '/');
    let kind = segs.next().unwrap_or("");
    let hex = segs.next().unwrap_or("");
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return not_found();
    }
    let Ok(app_data_dir) = app.path().app_data_dir() else {
        return not_found();
    };
    let cache = match kind {
        "artist" => &ARTIST_IMAGES,
        "cover" => &TRACK_COVERS,
        _ => return not_found(),
    };
    let path = app_data_dir.join(cache.name).join(cache.file_name_for_hex(hex));
    match fs::read(&path) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/webp")
            .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(bytes)
            .unwrap(),
        Err(_) => not_found(),
    }
}

pub fn protocol_handler<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let uri = request.uri().clone();
    crate::thumbnails::spawn_file_read(move || {
        responder.respond(build_external_response(&app, &uri));
    });
}

enum ImageDownload {
    Saved,
    /// Valid response but it's a source placeholder -> cache as "not found".
    Placeholder,
    /// Network/format error -> do NOT cache as "not found", retry later.
    Error(String),
}

/// Downloads, validates (status/content-type/size/decode), and re-encodes to WebP capped at
/// `max_dim`.
fn download_validate_encode(
    url: &str,
    target: &Path,
    max_dim: u32,
    min_real_bytes: Option<usize>,
) -> ImageDownload {
    let resp = match get_image(url) {
        Ok(r) => r,
        Err(e) => return ImageDownload::Error(e),
    };
    let resp = match resp.error_for_status() {
        Ok(r) => r,
        Err(e) => return ImageDownload::Error(e.to_string()),
    };
    if let Some(ct) = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        && !ct.trim().to_ascii_lowercase().starts_with("image/") {
            return ImageDownload::Error(format!("не изображение: {}", ct));
        }
    let bytes = match resp.bytes() {
        Ok(b) => b,
        Err(e) => return ImageDownload::Error(e.to_string()),
    };
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        return ImageDownload::Error(format!("слишком большой файл: {} Б", bytes.len()));
    }
    if let Some(min) = min_real_bytes
        && bytes.len() < min {
            return ImageDownload::Placeholder;
        }
    let img = match image::load_from_memory(&bytes) {
        Ok(i) => i,
        Err(_) => return ImageDownload::Error("не удалось декодировать изображение".into()),
    };
    if img.width() < 32 || img.height() < 32 {
        return ImageDownload::Placeholder;
    }
    let thumb = img.thumbnail(max_dim, max_dim);
    let mut out = Vec::new();
    if thumb
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::WebP)
        .is_err()
    {
        return ImageDownload::Error("не удалось закодировать WebP".into());
    }
    let Some(dir) = target.parent() else {
        return ImageDownload::Error("нет родительской директории".into());
    };
    let tmp = dir.join(format!(".tmp_{}_{}.webp", std::process::id(), key_hash(url)));
    if fs::write(&tmp, &out).is_err() {
        return ImageDownload::Error("ошибка записи файла".into());
    }
    if fs::rename(&tmp, target).is_err() {
        let _ = fs::remove_file(&tmp);
        return ImageDownload::Error("ошибка переименования файла".into());
    }
    ImageDownload::Saved
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ArtistImageRecord {
    file: Option<String>,
    source: Option<String>,
    /// "ok" | "none" - negative cache to avoid hitting the network repeatedly.
    status: String,
    fetched_secs: u64,
}

/// `url` is a `lfmusic-external://...` protocol URL, not base64.
#[derive(Debug, Clone, Serialize)]
pub struct ArtistImageResult {
    /// "ok" | "none"
    pub status: String,
    pub source: Option<String>,
    pub url: Option<String>,
}

fn none_result() -> ArtistImageResult {
    ArtistImageResult {
        status: "none".into(),
        source: None,
        url: None,
    }
}

impl ImageCache {
    fn url(&self, key: &str, version: u64) -> String {
        external_protocol_url(self.kind, &key_hex(key), version)
    }

    fn file_name_for_hex(&self, hex: &str) -> String {
        format!("{}{}.webp", self.file_prefix, hex)
    }

    fn file_name(&self, key: &str) -> String {
        self.file_name_for_hex(&key_hex(key))
    }

    fn dir(&self, app_data_dir: &Path) -> PathBuf {
        let dir = app_data_dir.join(self.name);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn manifest_path(&self, app_data_dir: &Path) -> PathBuf {
        app_data_dir.join(format!("{}.json", self.name))
    }

    fn ensure_version(&self, app_data_dir: &Path) {
        let path = app_data_dir.join(format!("{}.version", self.name));
        let current = fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);
        if current != self.version {
            let _ = fs::remove_dir_all(app_data_dir.join(self.name));
            let _ = fs::remove_file(self.manifest_path(app_data_dir));
            let _ = fs::write(&path, self.version.to_string());
        }
    }

    /// No lock needed for reads: manifest writes are atomic, so any read sees a whole file.
    fn load_manifest(&self, app_data_dir: &Path) -> HashMap<String, ArtistImageRecord> {
        json_store::load_or_default(&self.manifest_path(app_data_dir))
    }

    fn update_manifest(
        &self,
        app_data_dir: &Path,
        f: impl FnOnce(&mut HashMap<String, ArtistImageRecord>),
    ) {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut manifest = self.load_manifest(app_data_dir);
        f(&mut manifest);
        let _ = json_store::save_pretty(&self.manifest_path(app_data_dir), &manifest);
    }

    fn store(&self, app_data_dir: &Path, key: String, rec: ArtistImageRecord) {
        self.update_manifest(app_data_dir, |m| {
            m.insert(key, rec);
        });
    }

    /// Cache-first lookup; `refresh` forces a re-fetch from network.
    fn get_or_fetch(
        &self,
        app_data_dir: &Path,
        key: String,
        refresh: bool,
        fetch: impl FnOnce() -> FetchOutcome<ExternalImage>,
    ) -> ArtistImageResult {
        if key.is_empty() {
            return none_result();
        }

        self.ensure_version(app_data_dir);
        if !refresh
            && let Some(rec) = self.load_manifest(app_data_dir).get(&key) {
                if rec.status == "ok" {
                    if let Some(file) = &rec.file
                        && self.dir(app_data_dir).join(file).exists() {
                            return ArtistImageResult {
                                status: "ok".into(),
                                source: rec.source.clone(),
                                url: Some(self.url(&key, rec.fetched_secs)),
                            };
                        }
                } else if rec.status == "none" {
                    return none_result();
                }
            }

        let now = unix_secs();
        let filename = self.file_name(&key);
        let target = self.dir(app_data_dir).join(&filename);

        match fetch() {
            FetchOutcome::Found(img) => {
                match download_validate_encode(&img.url, &target, self.max_dim, self.min_real_bytes)
                {
                    ImageDownload::Saved => {
                        let url = self.url(&key, now);
                        self.store(
                            app_data_dir,
                            key,
                            ArtistImageRecord {
                                file: Some(filename),
                                source: Some(img.source.clone()),
                                status: "ok".into(),
                                fetched_secs: now,
                            },
                        );
                        ArtistImageResult {
                            status: "ok".into(),
                            source: Some(img.source),
                            url: Some(url),
                        }
                    }
                    ImageDownload::Placeholder => {
                        self.store(app_data_dir, key, none_image_record(now));
                        none_result()
                    }
                    ImageDownload::Error(e) => {
                        eprintln!("[external] download {} failed: {}", self.label, e);
                        none_result()
                    }
                }
            }
            FetchOutcome::NotFound => {
                self.store(app_data_dir, key, none_image_record(now));
                none_result()
            }
            FetchOutcome::TemporaryError => none_result(),
        }
    }

    /// Keys with a settled (ok/none) result, which the batch pass can skip.
    fn settled_keys(&self, app_data_dir: &Path) -> HashSet<String> {
        self.ensure_version(app_data_dir);
        self.load_manifest(app_data_dir)
            .into_iter()
            .filter(|(_, r)| r.status == "ok" || r.status == "none")
            .map(|(k, _)| k)
            .collect()
    }
}

fn artist_key(name: &str) -> String {
    normalize(name)
}

fn key_hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn none_image_record(now: u64) -> ArtistImageRecord {
    ArtistImageRecord {
        file: None,
        source: None,
        status: "none".into(),
        fetched_secs: now,
    }
}

/// Cache-first artist avatar lookup; `refresh` forces a re-fetch from network.
pub fn get_or_fetch_artist_image(
    app_data_dir: &Path,
    artist_name: &str,
    refresh: bool,
) -> ArtistImageResult {
    ARTIST_IMAGES.get_or_fetch(app_data_dir, artist_key(artist_name), refresh, || {
        fetch_artist_image(artist_name)
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct CachedArtistImage {
    pub artist_key: String,
    pub url: String,
}

pub fn load_cached_artist_images(app_data_dir: &Path) -> Vec<CachedArtistImage> {
    ARTIST_IMAGES.ensure_version(app_data_dir);
    let manifest = ARTIST_IMAGES.load_manifest(app_data_dir);
    let dir = ARTIST_IMAGES.dir(app_data_dir);
    manifest
        .iter()
        .filter(|(_, rec)| rec.status == "ok")
        .filter(|(_, rec)| rec.file.as_ref().is_some_and(|f| dir.join(f).exists()))
        .map(|(key, rec)| CachedArtistImage {
            artist_key: key.clone(),
            url: ARTIST_IMAGES.url(key, rec.fetched_secs),
        })
        .collect()
}

/// Artist+album (one cover per album), or artist+title when album is empty.
fn cover_cache_key(artist: &str, album: &str, title: &str) -> String {
    if !album.trim().is_empty() {
        normalize(&format!("{} {}", artist, album))
    } else {
        normalize(&format!("{} {} t", artist, title))
    }
}

pub fn get_or_fetch_track_cover(
    app_data_dir: &Path,
    artist: &str,
    album: &str,
    title: &str,
    refresh: bool,
) -> ArtistImageResult {
    let key = cover_cache_key(artist, album, title);
    TRACK_COVERS.get_or_fetch(app_data_dir, key, refresh, || {
        fetch_track_cover(artist, album, title)
    })
}

/// Includes artist so two same-titled albums by different artists don't share a cache entry.
pub fn album_cache_key(album: &str, artist: &str) -> String {
    format!("{}\u{1}{}", normalize(album), normalize(artist))
}

pub fn track_cover_key(artist: &str, album: &str, title: &str) -> String {
    cover_cache_key(artist, album, title)
}

pub fn artist_image_key(name: &str) -> String {
    artist_key(name)
}

pub fn cover_settled_keys(app_data_dir: &Path) -> HashSet<String> {
    TRACK_COVERS.settled_keys(app_data_dir)
}

pub fn artist_settled_keys(app_data_dir: &Path) -> HashSet<String> {
    ARTIST_IMAGES.settled_keys(app_data_dir)
}

/// Marks cover keys whose tracks all have a local cover (not a settled status).
pub fn mark_track_covers_local(app_data_dir: &Path, keys: &[String]) {
    if keys.is_empty() {
        return;
    }
    TRACK_COVERS.ensure_version(app_data_dir);
    let now = unix_secs();
    TRACK_COVERS.update_manifest(app_data_dir, |manifest| {
        for key in keys.iter().filter(|k| !k.is_empty()) {
            manifest
                .entry(key.clone())
                .or_insert_with(|| ArtistImageRecord {
                    status: "local".into(),
                    ..none_image_record(now)
                });
        }
    });
}

/// Keys marked by [`mark_track_covers_local`].
pub fn cover_local_keys(app_data_dir: &Path) -> HashSet<String> {
    TRACK_COVERS.ensure_version(app_data_dir);
    TRACK_COVERS
        .load_manifest(app_data_dir)
        .into_iter()
        .filter(|(_, r)| r.status == "local")
        .map(|(k, _)| k)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lowercases_and_collapses_punctuation_to_single_spaces() {
        assert_eq!(normalize("  Hello,   World!!  "), "hello world");
        assert_eq!(normalize("Bohemian-Rhapsody"), "bohemian rhapsody");
    }

    #[test]
    fn normalize_of_empty_and_all_punctuation_is_empty() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("!!!---..."), "");
    }

    #[test]
    fn normalize_title_strips_bare_feat_marker_without_parens() {
        assert_eq!(normalize_title("Title ft. Someone"), normalize("Title"));
        assert_eq!(normalize_title("Title feat Someone"), normalize("Title"));
    }

    #[test]
    fn normalize_title_strips_parenthesized_feat_and_bracketed_remaster_marker() {
        assert_eq!(
            normalize_title("Song Title (feat. Artist X) [Remastered 2011]"),
            normalize("Song Title")
        );
    }

    #[test]
    fn similarity_is_one_for_identical_after_normalization() {
        assert_eq!(similarity("Believer", "  BELIEVER!!"), 1.0);
    }

    #[test]
    fn similarity_is_zero_when_either_side_normalizes_to_empty() {
        assert_eq!(similarity("", "Believer"), 0.0);
        assert_eq!(similarity("!!!", "Believer"), 0.0);
    }

    #[test]
    fn similarity_gives_containment_bonus_for_deluxe_edition_style_suffix() {
        let with_bonus = similarity("Album", "Album (Deluxe Edition)");
        assert!(with_bonus > 0.35, "ожидали эффект contains_bonus, similarity={with_bonus}");
        assert!(with_bonus < 0.5, "similarity={with_bonus}");
    }

    #[test]
    fn similarity_is_symmetric() {
        let a = similarity("Imagine Dragons", "Imagin Dragons");
        let b = similarity("Imagin Dragons", "Imagine Dragons");
        assert!((a - b).abs() < 1e-6);
    }

    #[test]
    fn similarity_stays_within_unit_range() {
        let s = similarity("A", "Completely Different Long String Of Text");
        assert!((0.0..=1.0).contains(&s));
    }

    #[test]
    fn is_placeholder_image_flags_known_deezer_defaults() {
        assert!(is_placeholder_image(
            "https://e-cdns-images.dzcdn.net/images/artist//1000x1000-000000-80-0-0.jpg"
        ));
        assert!(is_placeholder_image(
            "https://e-cdns-images.dzcdn.net/images/cover/d41d8cd98f00b204e9800998ecf8427e/1000x1000.jpg"
        ));
    }

    #[test]
    fn is_placeholder_image_allows_real_looking_urls() {
        assert!(!is_placeholder_image(
            "https://e-cdns-images.dzcdn.net/images/artist/abc123def456/1000x1000.jpg"
        ));
    }

    #[test]
    fn album_score_rewards_title_match_more_than_artist_match() {
        let album_title_match = ExternalAlbum {
            source: "test".into(),
            artist: "Totally Different Artist".into(),
            title: "Greatest Hits".into(),
            tracks: vec![],
        };
        let album_artist_match = ExternalAlbum {
            source: "test".into(),
            artist: "Queen".into(),
            title: "Something Else Entirely".into(),
            tracks: vec![],
        };
        let by_title = album_score("Queen", "Greatest Hits", &album_title_match);
        let by_artist = album_score("Queen", "Greatest Hits", &album_artist_match);
        assert!(by_title > by_artist, "title weight (0.7) should dominate over artist weight (0.3)");
    }

    #[test]
    fn album_score_uses_neutral_artist_score_when_query_artist_is_empty() {
        let album = ExternalAlbum {
            source: "test".into(),
            artist: "Anyone".into(),
            title: "Best Of".into(),
            tracks: vec![],
        };
        let score = album_score("", "Best Of", &album);
        assert!((score - (1.0 * 0.7 + 0.7 * 0.3)).abs() < 0.05);
    }

    #[test]
    fn track_identity_key_is_order_sensitive_concatenation() {
        let key = track_identity_key("Queen", "Bohemian Rhapsody", Some("A Night at the Opera"));
        assert_eq!(key, normalize("Queen Bohemian Rhapsody A Night at the Opera"));
    }

    #[test]
    fn track_identity_key_handles_missing_album() {
        let with_none = track_identity_key("Queen", "Bohemian Rhapsody", None);
        let with_empty = track_identity_key("Queen", "Bohemian Rhapsody", Some(""));
        assert_eq!(with_none, with_empty);
    }

    #[test]
    fn title_match_key_strips_credit_noise_in_parentheses() {
        assert_eq!(
            title_match_key("Song Name (prod. by Someone)"),
            normalize("Song Name")
        );
        assert_eq!(
            title_match_key("Song Name (Official Video)"),
            normalize("Song Name")
        );
        assert_eq!(
            title_match_key("Song Name (feat. Other Artist)"),
            normalize("Song Name")
        );
    }

    #[test]
    fn title_match_key_keeps_version_markers_in_parentheses() {
        let key = title_match_key("Song Name (Live)");
        assert_ne!(key, normalize("Song Name"));
        assert_eq!(key, normalize("Song Name (Live)"));
    }

    #[test]
    fn title_match_key_strips_trailing_feat_outside_parentheses() {
        assert_eq!(
            title_match_key("Song Name feat. Other Artist"),
            normalize("Song Name")
        );
    }

    #[test]
    fn title_match_key_handles_unclosed_parenthesis_without_panicking() {
        let key = title_match_key("Song Name (Live forever");
        assert!(!key.is_empty());
    }

    #[test]
    fn album_match_key_normalizes_like_normalize() {
        assert_eq!(album_match_key("Greatest Hits!"), normalize("Greatest Hits!"));
        assert_eq!(album_match_key(""), "");
    }

    #[test]
    fn split_artists_for_match_splits_on_common_conjunctions() {
        let prepared = prepare_artist_separators(&[]);
        let parts = split_artists_for_match_prepared("Queen & David Bowie", &prepared);
        assert_eq!(parts, vec!["queen".to_string(), "david bowie".to_string()]);
    }

    #[test]
    fn split_artists_for_match_handles_feat_and_vs() {
        let prepared = prepare_artist_separators(&[]);
        let a = split_artists_for_match_prepared("Artist A feat. Artist B", &prepared);
        assert_eq!(a, vec!["artist a".to_string(), "artist b".to_string()]);
        let b = split_artists_for_match_prepared("Artist One vs Artist Two", &prepared);
        assert_eq!(b, vec!["artist one".to_string(), "artist two".to_string()]);
    }

    #[test]
    fn split_artists_for_match_respects_custom_separators() {
        let prepared = prepare_artist_separators(&["; ".to_string()]);
        let parts = split_artists_for_match_prepared("Artist A; Artist B", &prepared);
        assert_eq!(parts, vec!["artist a".to_string(), "artist b".to_string()]);
    }

    #[test]
    fn split_artists_for_match_deduplicates_normalized_results() {
        let prepared = prepare_artist_separators(&[]);
        let parts = split_artists_for_match_prepared("Queen & Queen", &prepared);
        assert_eq!(parts, vec!["queen".to_string()]);
    }

    #[test]
    fn split_artists_for_match_single_artist_returns_one_entry() {
        let prepared = prepare_artist_separators(&[]);
        let parts = split_artists_for_match_prepared("Adele", &prepared);
        assert_eq!(parts, vec!["adele".to_string()]);
    }

    #[test]
    fn prepare_artist_separators_sorts_longest_first_to_avoid_prefix_collisions() {
        let prepared = prepare_artist_separators(&[]);
        let feat_idx = prepared.iter().position(|s| s == " feat ").unwrap();
        let featuring_idx = prepared.iter().position(|s| s == " featuring ").unwrap();
        assert!(featuring_idx < feat_idx, "более длинный разделитель должен идти раньше");
    }

    #[test]
    fn prepare_artist_separators_drops_empty_user_separators() {
        let prepared = prepare_artist_separators(&["".to_string(), "; ".to_string()]);
        assert!(!prepared.iter().any(|s| s.is_empty()));
    }

    #[test]
    fn host_of_extracts_hostname_from_various_url_shapes() {
        assert_eq!(host_of("https://api.deezer.com/search/track"), "api.deezer.com");
        assert_eq!(host_of("https://MusicBrainz.org/ws/2/"), "musicbrainz.org");
        assert_eq!(host_of("https://user:pass@itunes.apple.com/path"), "itunes.apple.com");
        assert_eq!(host_of("https://coverartarchive.org:443/release/x"), "coverartarchive.org");
    }

    #[test]
    fn host_of_handles_malformed_input_without_panicking() {
        assert_eq!(host_of(""), "");
        assert_eq!(host_of("not-a-url"), "");
    }

    #[test]
    fn host_min_gap_gives_itunes_the_longest_interval() {
        let itunes = host_min_gap("itunes.apple.com");
        let deezer = host_min_gap("api.deezer.com");
        let unknown = host_min_gap("example.com");
        assert!(itunes > deezer);
        assert!(deezer > unknown);
    }

    #[test]
    fn artist_key_and_album_cache_key_are_normalized_and_deterministic() {
        assert_eq!(artist_image_key("Queen"), artist_image_key("  queen  "));
        let k1 = album_cache_key("Greatest Hits", "Queen");
        let k2 = album_cache_key("greatest hits", "QUEEN");
        assert_eq!(k1, k2);
    }

    #[test]
    fn album_cache_key_distinguishes_album_and_artist_order() {
        let a = album_cache_key("X Y", "Z");
        let b = album_cache_key("X", "Y Z");
        assert_ne!(a, b);
    }

    #[test]
    fn track_cover_key_is_deterministic_for_same_inputs() {
        let a = track_cover_key("Queen", "A Night at the Opera", "Bohemian Rhapsody");
        let b = track_cover_key("Queen", "A Night at the Opera", "Bohemian Rhapsody");
        assert_eq!(a, b);
    }

    #[test]
    fn artist_image_url_and_cover_image_url_differ_for_same_key() {
        let artist_url = ARTIST_IMAGES.url("queen", 1);
        let cover_url = TRACK_COVERS.url("queen", 1);
        assert_ne!(artist_url, cover_url);
        assert!(artist_url.contains("/artist/"));
        assert!(cover_url.contains("/cover/"));
    }

    #[test]
    fn image_urls_change_with_version_for_cache_busting() {
        let v1 = ARTIST_IMAGES.url("queen", 1);
        let v2 = ARTIST_IMAGES.url("queen", 2);
        assert_ne!(v1, v2);
    }

    #[test]
    fn artist_file_name_and_cover_file_name_use_distinct_prefixes() {
        let key = "queen";
        let artist_file = ARTIST_IMAGES.file_name(key);
        let cover_file = TRACK_COVERS.file_name(key);
        assert!(artist_file.ends_with(".webp"));
        assert!(cover_file.ends_with(".webp"));
        assert!(cover_file.starts_with('c'));
        assert_ne!(artist_file, cover_file);
    }

    #[test]
    fn artist_top_next_advances_then_switches_to_search_offset() {
        assert_eq!(artist_top_next(0, 40, 94, true), (40, true));
        assert_eq!(artist_top_next(40, 40, 94, false), (80, true));
        assert_eq!(artist_top_next(0, 32, 32, false), (ARTIST_SEARCH_OFFSET, true));
        assert_eq!(artist_top_next(80, 20, 100, false), (ARTIST_SEARCH_OFFSET, true));
        assert_eq!(artist_top_next(80, 0, 100, true), (ARTIST_SEARCH_OFFSET, true));
    }

    #[test]
    fn deezer_page_resp_captures_http_200_error_payload() {
        let raw = r#"{"error":{"type":"QuotaException","message":"Quota limit exceeded","code":4}}"#;
        let parsed: DeezerPageResp<serde_json::Value> = serde_json::from_str(raw).unwrap();
        assert!(parsed.error.is_some());
        assert!(parsed.data.is_empty());
        assert_eq!(parsed.total, 0);
    }

}
