//! Free, no-API-key metadata sources (iTunes, Deezer, MusicBrainz/CAA, Wikipedia)
//! behind a common [`Provider`] interface. The aggregator queries providers,
//! scores matches by title/artist similarity, and picks the best result.
//!
//! Used for: canonical album track order (+ "phantom" tracks not in the local
//! library), and artist avatars.
//!
//! Images are never sent through IPC as base64: they're downloaded, validated,
//! and re-encoded to WebP on disk; the frontend gets a `lfmusic-external://...`
//! URL (see [`protocol_handler`]), same pattern as `lfmusic-cover`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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

/// Distinguishes "confirmed not found" (all providers said no) from a
/// "temporary error" (network/parse/rate-limit) — the latter must not be
/// cached as a permanent negative result.
pub enum FetchOutcome<T> {
    Found(T),
    /// All providers genuinely reported nothing found.
    NotFound,
    /// At least one provider errored and none found a match — retry later.
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

    /// Cover lookup prefers matching by album (one cover for all its tracks);
    /// falls back to track title when album is empty.
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

/// MusicBrainz/CAA require ≤1 req/s. iTunes Search API is hard-limited to
/// ~20 req/min per IP, so we use 3s between requests — 300ms (≈200/min) used
/// to hit 429s and empty "error decoding" bodies once a parallel pool kept the
/// rate maxed out. Deezer's documented limit is 50 req/5s (~10 req/s); 140ms
/// (~7 req/s) leaves headroom for bursts across the prep pool's several
/// concurrent workers while still comfortably outrunning the old 220ms gap
/// (kept as the main prep source, so this is the one that matters most for
/// first-run batch prep time).
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

/// Hosts strict enough (MusicBrainz/CAA document ≤1 req/s) that pacing request
/// *starts* isn't sufficient on its own: `host_throttle` only spaces out when
/// requests begin, so a response that's still in flight when the next thread's
/// slot opens leaves two connections to the same host open at once. Under the
/// prep pool's several concurrent workers that reliably produced connection
/// resets ("error sending request") from these hosts, so they additionally get
/// a per-host lock held across the whole request (see [`send_throttled`]).
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

/// Waits until the host's next slot is free, then reserves it. The sleep
/// happens after releasing the mutex, not while holding it.
fn host_throttle(host: &str) {
    let sleep_dur = {
        let reg = host_registry();
        let mut guard = reg.lock().unwrap_or_else(|e| e.into_inner());
        let st = guard.entry(host.to_string()).or_default();
        let now = Instant::now();
        let mut earliest = st.next_slot.unwrap_or(now);
        if let Some(b) = st.blocked_until {
            if b > earliest {
                earliest = b;
            }
        }
        let target = if earliest > now { earliest } else { now };
        st.next_slot = Some(target + host_min_gap(host));
        target.saturating_duration_since(now)
    };
    if !sleep_dur.is_zero() {
        std::thread::sleep(sleep_dur);
    }
}

/// Registers backoff after 429/503 (keeps the max of current/new so we never
/// shorten an already-scheduled pause).
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

/// Sends a request honoring per-host rate limit + backoff. On 429/503,
/// registers backoff and returns `Err` (treated as a temporary error upstream,
/// not cached as "not found").
fn send_throttled(
    builder: reqwest::blocking::RequestBuilder,
    host: &str,
) -> Result<reqwest::blocking::Response, String> {
    // For strict hosts, hold a per-host lock across the whole request (not just
    // the pacing wait) so no two requests to that host are ever in flight at
    // once — see host_needs_serialized_requests. `lock` must outlive `_guard`.
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

/// Backoff for "soft" rate-limiting: iTunes sometimes responds 200 with an
/// empty body instead of 429 when overloaded.
const SOFT_RATE_LIMIT_BACKOFF_SECS: u64 = 20;

/// Like [`send_throttled`] but decodes JSON directly. An empty 200 body is
/// treated as a soft rate-limit (registers backoff, returns `Err`) instead of
/// surfacing a JSON decode error.
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

/// Canonicalizes a string for comparison: lowercase, alphanumeric/spaces only,
/// collapsed whitespace.
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

/// Strips noise (feat., remastered, bracketed notes) for more robust matching.
/// Uses `strip_title_noise` for bracketed noise like `"Song (feat. X)"`; the
/// extra `replace()` pass below still catches the no-parens legacy form
/// `"Song Title - Remastered"`, which `strip_title_noise` doesn't touch.
fn normalize_title(s: &str) -> String {
    let cleaned: String = strip_title_noise(s)
        .replace("remastered", " ")
        .replace("remaster", " ")
        .replace("original mix", " ")
        .replace("explicit", " ");
    normalize(&cleaned)
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let n = a.len();
    let m = b.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
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
    // Containment bonus for e.g. "Album" vs "Album (Deluxe Edition)".
    let contains_bonus = if na.contains(&nb) || nb.contains(&na) { 0.15 } else { 0.0 };
    (ratio + contains_bonus).clamp(0.0, 1.0)
}

/// Detects a source's default "no photo" placeholder image; prefer falling
/// back to the local cover instead of using these.
fn is_placeholder_image(url: &str) -> bool {
    // Deezer uses an empty md5 segment in the path when there's no photo, e.g.
    // https://e-cdns-images.dzcdn.net/images/artist//1000x1000-000000-80-0-0.jpg
    url.contains("/images/artist//")
        || url.contains("/images/cover//")
        || url.contains("/images/user//")
        // Known Deezer default images (same silhouette/wave under different md5s).
        || url.contains("d41d8cd98f00b204e9800998ecf8427e")
        || url.contains("1fecff61b191427ec4d2760a77527bf5")
        || url.contains("e460b01306de4c8d790508e0d80d1fdf")
}

/// Below this size (bytes) an image is treated as a source's placeholder.
/// Deezer's placeholder is ≤33KB at 1000x1000; real photos are 90KB+.
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
    // Title matters more than artist (library artist tags are often formatted differently).
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
        // iTunes previews are 100x100 but Apple serves full-res via URL substitution.
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

        // Best name match wins; ties go to the earlier result (Deezer ranks by
        // relevance, so the first same-named artist is usually the intended one).
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
            if let Some((sim, name, url)) = best {
                if sim >= 0.5 {
                    return Ok(Some(ExternalImage {
                        source: "Deezer".to_string(),
                        url,
                        matched_name: name,
                    }));
                }
            }
            return Ok(None);
        }

        // No album given: search by track, use its album's cover.
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
        if let Some((sim, name, url)) = best {
            if sim >= 0.5 {
                return Ok(Some(ExternalImage {
                    source: "Deezer".to_string(),
                    url,
                    matched_name: name,
                }));
            }
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

        // Prefer best title match, then most tracks (full releases over singles).
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

/// Finds a MusicBrainz release-group (the "album" spanning all editions, one
/// cover for all reissues) best matching title (and artist, if given).
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
            // If artist is known, reject a same-titled album by someone else.
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
        // CAA indexes covers by MusicBrainz release-group, so album is required.
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
        // CAA responds 404 when the release-group has no cover art.
        if !resp.status().is_success() {
            return Ok(None);
        }
        let caa: CaaResp = resp.json().map_err(|e| e.to_string())?;

        // Prefer the image marked "front", else the first available.
        let Some(img) = caa.images.iter().find(|i| i.front).or_else(|| caa.images.first())
        else {
            return Ok(None);
        };
        // Prefer a pre-sized 1200/500 thumbnail over the (potentially huge) original.
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

/// Whether a Wikidata short description indicates a music artist. Without
/// this check, search easily returns an unrelated same-named article (city,
/// film, color).
fn is_music_description(desc: Option<&str>) -> bool {
    let Some(d) = desc else {
        return false;
    };
    let d = d.to_lowercase();
    const KW: &[&str] = &[
        // en
        "musician", "singer", "rapper", "band", "composer", "songwriter", "vocalist",
        "guitarist", "drummer", " dj", "duo", "group", "producer", "music",
        // ru
        "певец", "певица", "музыкант", "группа", "рэпер", "композитор", "исполнит",
        "музыкальн", "дуэт", "диджей", "вокалист", "гитарист", "ансамбль", "бэнд",
        "рок-", "поп-", "музыка",
    ];
    KW.iter().any(|k| d.contains(k))
}

/// Looks up an artist article in one Wikipedia language edition and returns
/// its lead photo. Strict filtering by description + name match: when in
/// doubt, return `None` rather than risk the wrong picture.
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

    // Candidate needs: photo + music description + name match. Best name
    // match wins; ties go to higher search rank (lower index).
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
        // English first (wider coverage, stable descriptions), then Russian
        // (local artists missing from en).
        for lang in ["en", "ru"] {
            if let Some(img) = wiki_lookup(client, lang, artist)? {
                return Ok(Some(img));
            }
        }
        Ok(None)
    }
}

fn album_providers() -> Vec<Box<dyn Provider>> {
    // Deezer first: much higher rate limit than iTunes and rich data without
    // a key. A near-exact match (score >= 0.97) short-circuits the rest.
    // iTunes/MusicBrainz only get the tail of requests, keeping us under
    // iTunes's strict limit.
    vec![Box::new(Deezer), Box::new(ITunes), Box::new(MusicBrainz)]
}

fn image_providers() -> Vec<Box<dyn Provider>> {
    // Deezer primary (rich artist photos, no key); Wikipedia fallback for
    // artists missing from Deezer.
    vec![Box::new(Deezer), Box::new(Wikipedia)]
}

fn cover_providers() -> Vec<Box<dyn Provider>> {
    // Deezer first: most tracks resolve here (short-circuits on first match),
    // keeping iTunes's strict limit from being hit during bulk prep.
    // Cover Art Archive is the last resort for niche/regional releases.
    vec![Box::new(Deezer), Box::new(ITunes), Box::new(CoverArtArchive)]
}

/// Queries sources for a track/album cover, returns first match.
/// `TemporaryError` if nothing was found but at least one provider errored.
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
                // Reject a same-titled album by a different artist.
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

/// Queries sources for the best artist avatar. Stops at the first confident
/// match (>=0.85) to save network calls and avoid a less-accurate fallback
/// image (e.g. Wikipedia) overriding a good one.
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
/// Unlike [`ExternalTrack`], carries a remote cover URL (loaded directly by
/// the webview) and source name — a "phantom" result not in the library.
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
    /// Genre, when the source exposes it directly (iTunes only — Deezer's
    /// track/album search responses don't include it; for Deezer results the
    /// download path looks it up separately via `album_id`, see
    /// [`deezer_album_genre`]).
    #[serde(default)]
    pub genre: Option<String>,
    /// Set only for the "paste a link" flow (`downloads_resolve_url`) — the
    /// exact yt-dlp target URL, passed straight through to `downloads_enqueue`
    /// as `DownloadRequest::direct_url` so the worker downloads it directly
    /// instead of searching. `None` for Deezer/iTunes search results.
    #[serde(default)]
    pub source_url: Option<String>,
}

/// Deezer album search result for the download page, identified by Deezer id
/// (see [`album_detail`]).
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

/// Identity key for a download candidate: normalized artist+title+album.
/// Includes album so same-titled tracks from different albums stay distinct;
/// empty album falls back to artist+title only. Keep in sync with `trackKey`
/// in `useDownloads.ts`.
pub fn track_identity_key(artist: &str, title: &str, album: Option<&str>) -> String {
    normalize(&format!("{} {} {}", artist, title, album.unwrap_or("")))
}

/// Bracketed tokens treated as noise (credits/labels), not a distinct version.
/// A bracket group containing one of these is dropped so "Song (prod. by X)"
/// matches "Song". Version markers (remix/live/acoustic/...) are NOT listed
/// and are preserved.
const TITLE_NOISE_TOKENS: &[&str] = &[
    "feat", "ft", "featuring", "prod", "produced", "official", "lyric", "lyrics",
    "visualizer", "remaster", "remastered", "explicit", "mv",
];

fn group_is_noise(buf: &str) -> bool {
    buf.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|w| TITLE_NOISE_TOKENS.contains(&w))
}

/// Strips bracketed credits (`(prod. by ...)`/`(feat. ...)`/`(Official Video)`)
/// and trailing `feat.`/`prod.` for title matching. Version markers in
/// brackets (remix/live/...) are preserved.
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
    // Unclosed bracket: keep the remainder if it's not noise.
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

/// Normalized album key for album-aware library matching. Both sides of a
/// comparison (local tag and external source album) must go through this or
/// the keys won't line up.
pub fn album_match_key(album: &str) -> String {
    normalize(album)
}

/// Builds the separator list once (user separators + built-in feat/ft/and...),
/// sorted longest-first so a short prefix like " feat " doesn't fragment
/// " featuring " before it matches. Hoisted out of the hot path: rebuilding
/// this per-track on a tens-of-thousands library caused UI freezes during
/// index rebuilds. See [`split_artists_for_match_prepared`].
pub fn prepare_artist_separators(separators: &[String]) -> Vec<String> {
    let mut seps: Vec<String> = separators.iter().map(|x| x.to_lowercase()).collect();
    for extra in [
        "&", ",", ";", "/", " feat ", " feat. ", " ft ", " ft. ", " featuring ",
        " x ", " vs ", " vs. ", " and ", " и ",
    ] {
        seps.push(extra.to_string());
    }
    seps.sort_by(|a, b| b.len().cmp(&a.len()));
    seps.retain(|s| !s.is_empty());
    seps
}

/// Splits an artist string into normalized names for fuzzy matching, using an
/// already-[`prepare_artist_separators`]-prepared separator list (no
/// per-call allocation/sort). Unlike [`crate::library::split_artists`], splits
/// more aggressively (also on feat/ft/x/vs/"и" etc.) so multi-artist tracks
/// can match on any one artist.
pub fn split_artists_for_match_prepared(artist: &str, prepared_seps: &[String]) -> Vec<String> {
    // Pad with spaces so separators like " feat " match at boundaries.
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
/// Exact field matching rescues niche tracks with generic titles (e.g. "Intro")
/// that get buried by same-named hits in free-text `q=` search. Quotes in
/// values would break the syntax, so they're replaced with a space.
fn deezer_advanced_query(artist: &str, title: &str) -> String {
    let a = artist.replace('"', " ");
    let t = title.replace('"', " ");
    format!("artist:\"{}\" track:\"{}\"", a.trim(), t.trim())
}

/// Returns filtered tracks plus the raw (pre-filter) Deezer result count —
/// pagination must use the raw count, since filtering out entries (e.g. no
/// artist) would prematurely zero `has_more` and skew `next_index`.
fn deezer_search_tracks(
    client: &reqwest::blocking::Client,
    query: &str,
    index: u32,
    limit: u32,
) -> Result<(Vec<ExternalSearchTrack>, u32), String> {
    #[derive(Deserialize)]
    struct SearchResp {
        data: Vec<Hit>,
    }
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

    let resp: SearchResp = send_throttled(
        client.get("https://api.deezer.com/search/track").query(&[
            ("q", query),
            ("index", index.to_string().as_str()),
            ("limit", limit.to_string().as_str()),
        ]),
        "api.deezer.com",
    )?
    .json()
    .map_err(|e| e.to_string())?;

    let raw_count = resp.data.len() as u32;
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
    Ok((tracks, raw_count))
}

/// Picks the best available image URL (xl -> big -> medium), skipping empty
/// and placeholder Deezer defaults.
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

/// Looks up a track's genre via its Deezer album id — the track search
/// endpoint doesn't include genre, only `/album/{id}` does. Best-effort:
/// `None` on any network/parse error or when the album has no genre set
/// (Deezer uses id 0 / an empty list for "All"/unclassified).
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

    #[derive(Deserialize)]
    struct TopResp {
        #[serde(default)]
        data: Vec<TopHit>,
    }
    #[derive(Deserialize)]
    struct TopHit {
        title: String,
        duration: Option<u32>,
        album: Option<AlbumRef>,
    }
    #[derive(Deserialize)]
    struct AlbumRef {
        id: Option<u64>,
        title: Option<String>,
        cover_xl: Option<String>,
        cover_big: Option<String>,
        cover_medium: Option<String>,
    }
    let top_tracks = match send_throttled(
        client
            .get(format!("https://api.deezer.com/artist/{}/top", id))
            .query(&[("limit", "20")]),
        "api.deezer.com",
    ) {
        Ok(r) if r.status().is_success() => {
            let parsed: TopResp = r.json().map_err(|e| e.to_string())?;
            parsed
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
                    ExternalSearchTrack {
                        title: t.title,
                        artist: artist.name.clone(),
                        album,
                        duration_secs: t.duration,
                        cover_url,
                        source: "Deezer".to_string(),
                        in_library: false,
                        artist_id: Some(artist.id),
                        album_id,
                        genre: None,
                        source_url: None,
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    };

    Ok(Some(ExternalArtistDetail {
        id: artist.id,
        name: artist.name,
        picture_url,
        albums,
        top_tracks,
    }))
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
            // 100x100 -> 600x600 preview via URL substitution (as in cover lookup).
            let cover_url = h
                .artwork_url100
                .map(|u| u.replace("100x100bb", "600x600bb"));
            // "Music" is iTunes's generic fallback bucket, not a real genre.
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
                // iTunes doesn't expose an id, so artist/album aren't clickable.
                artist_id: None,
                album_id: None,
                genre,
                source_url: None,
            })
        })
        .collect())
}

/// Pagination cursor is based ONLY on Deezer results; mixed-in iTunes tracks
/// don't affect it, or the frontend would misread the offset and skip results.
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

/// Deezer is the primary paginated source. iTunes is mixed in only on the
/// first page and only when Deezer's page was short (`< limit`) — avoids an
/// extra network call for popular queries. Deezer order is preserved; iTunes
/// tracks are appended with dedup by `track_identity_key`.
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

    let (mut results, deezer_raw) = match deezer_search_tracks(client, q, index, limit) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[external] deezer track search error: {}", e);
            (Vec::new(), 0)
        }
    };

    let next_index = index + deezer_raw;
    let has_more = deezer_raw >= limit;

    // Exact field search (first page only, when artist+title are known, e.g.
    // clicking a "phantom" in an album view). Free-text `q=` ranks by
    // popularity, so a niche generically-titled track ("Intro") gets buried;
    // field-exact hits are put at the front instead.
    if index == 0 {
        if let (Some(a), Some(t)) = (artist, title) {
            let (a, t) = (a.trim(), t.trim());
            if !a.is_empty() && !t.is_empty() {
                let adv = deezer_advanced_query(a, t);
                match deezer_search_tracks(client, &adv, 0, limit) {
                    Ok((exact, _)) if !exact.is_empty() => {
                        // Album-aware dedup: same-titled tracks from different
                        // albums both stay; only the same release collapses.
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

/// Maps external tracks onto local (file_path, title) pairs, returning the
/// canonical-order layout. Local tracks that don't fit the canon are omitted
/// here; the frontend appends them at the end itself.
pub fn build_layout(
    tracks: &[ExternalTrack],
    local: &[(String, String)],
) -> Vec<AlbumLayoutEntry> {
    // Normalized title -> local paths (may have duplicates).
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

/// `version` is for cache-busting: a `refresh` changes the fetch time, so the
/// URL changes and the webview reloads the image.
fn external_protocol_url(kind: &str, hex: &str, version: u64) -> String {
    format!("{}/{}/{}?v={}", scheme_origin(), kind, hex, version)
}

fn artist_image_url(key: &str, version: u64) -> String {
    external_protocol_url("artist", &format!("{:016x}", key_hash(key)), version)
}

fn cover_image_url(key: &str, version: u64) -> String {
    external_protocol_url("cover", &format!("{:016x}", key_hash(key)), version)
}

fn artist_file_name(key: &str) -> String {
    format!("{:016x}.webp", key_hash(key))
}

/// `c` prefix distinguishes cover files from artist avatar files.
fn cover_file_name(key: &str) -> String {
    format!("c{:016x}.webp", key_hash(key))
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
    // Path shape: /<kind>/<hex>, kind = artist|cover.
    let mut segs = uri.path().trim_matches('/').splitn(2, '/');
    let kind = segs.next().unwrap_or("");
    let hex = segs.next().unwrap_or("");
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return not_found();
    }
    let Ok(app_data_dir) = app.path().app_data_dir() else {
        return not_found();
    };
    let path = match kind {
        "artist" => artists_dir(&app_data_dir).join(format!("{}.webp", hex)),
        "cover" => covers_dir(&app_data_dir).join(format!("c{}.webp", hex)),
        _ => return not_found(),
    };
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

/// File read happens on a separate thread so the webview isn't blocked.
pub fn protocol_handler<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let uri = request.uri().clone();
    std::thread::spawn(move || {
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

/// Downloads, validates (status/content-type/size/decode), and re-encodes to
/// WebP capped at `max_dim`. `min_real_bytes` is the placeholder-size
/// threshold for avatars; `None` for covers since small legitimate art exists.
fn download_validate_encode(
    url: &str,
    target: &PathBuf,
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
    {
        if !ct.trim().to_ascii_lowercase().starts_with("image/") {
            return ImageDownload::Error(format!("не изображение: {}", ct));
        }
    }
    let bytes = match resp.bytes() {
        Ok(b) => b,
        Err(e) => return ImageDownload::Error(e.to_string()),
    };
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        return ImageDownload::Error(format!("слишком большой файл: {} Б", bytes.len()));
    }
    if let Some(min) = min_real_bytes {
        if bytes.len() < min {
            return ImageDownload::Placeholder;
        }
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
    // Atomic write (temp + rename) so the protocol handler never reads a partial file.
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
    /// "ok" | "none" — negative cache to avoid hitting the network repeatedly.
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

fn artists_dir(app_data_dir: &PathBuf) -> PathBuf {
    let dir = app_data_dir.join("external_artists");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Bumping this wipes the disk cache once (e.g. on storage format changes).
const ARTIST_IMAGE_CACHE_VERSION: u32 = 5;

fn ensure_artist_cache_version(app_data_dir: &PathBuf) {
    let path = app_data_dir.join("external_artists.version");
    let current = fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0);
    if current != ARTIST_IMAGE_CACHE_VERSION {
        let _ = fs::remove_dir_all(app_data_dir.join("external_artists"));
        let _ = fs::remove_file(artists_manifest_path(app_data_dir));
        let _ = fs::write(&path, ARTIST_IMAGE_CACHE_VERSION.to_string());
    }
}

fn artists_manifest_path(app_data_dir: &PathBuf) -> PathBuf {
    app_data_dir.join("external_artists.json")
}

fn load_artists_manifest(app_data_dir: &PathBuf) -> HashMap<String, ArtistImageRecord> {
    fs::read_to_string(artists_manifest_path(app_data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_artists_manifest(
    app_data_dir: &PathBuf,
    manifest: &HashMap<String, ArtistImageRecord>,
) -> Result<(), String> {
    let content = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    write_manifest_atomic(&artists_manifest_path(app_data_dir), content.as_bytes())
}

/// Atomic write (temp -> rename) so concurrent readers never see a partial file.
fn write_manifest_atomic(target: &PathBuf, data: &[u8]) -> Result<(), String> {
    let tmp = target.with_extension("json.tmp");
    fs::write(&tmp, data).map_err(|e| e.to_string())?;
    fs::rename(&tmp, target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Serializes read-modify-write of the manifest across parallel prep threads,
/// so concurrent "load -> insert -> save" cycles don't clobber each other.
/// Network downloads happen outside this lock — only the disk merge is serialized.
fn artists_manifest_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

fn store_artist_record(app_data_dir: &PathBuf, key: String, rec: ArtistImageRecord) {
    let _guard = artists_manifest_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut manifest = load_artists_manifest(app_data_dir);
    manifest.insert(key, rec);
    let _ = save_artists_manifest(app_data_dir, &manifest);
}

fn artist_key(name: &str) -> String {
    normalize(name)
}

fn key_hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
    app_data_dir: &PathBuf,
    artist_name: &str,
    refresh: bool,
) -> ArtistImageResult {
    let key = artist_key(artist_name);
    if key.is_empty() {
        return none_result();
    }

    ensure_artist_cache_version(app_data_dir);
    // No lock needed for reads: manifest writes are atomic, so any read sees a whole file.
    if !refresh {
        let manifest = load_artists_manifest(app_data_dir);
        if let Some(rec) = manifest.get(&key) {
            if rec.status == "ok" {
                if let Some(file) = &rec.file {
                    if artists_dir(app_data_dir).join(file).exists() {
                        return ArtistImageResult {
                            status: "ok".into(),
                            source: rec.source.clone(),
                            url: Some(artist_image_url(&key, rec.fetched_secs)),
                        };
                    }
                }
            } else if rec.status == "none" {
                return none_result();
            }
        }
    }

    // Network fetch stays outside the manifest lock so parallel prep threads
    // can download concurrently.
    let now = now_secs();
    let filename = artist_file_name(&key);
    let target = artists_dir(app_data_dir).join(&filename);

    match fetch_artist_image(artist_name) {
        FetchOutcome::Found(img) => {
            match download_validate_encode(
                &img.url,
                &target,
                ARTIST_IMAGE_MAX_DIM,
                Some(MIN_REAL_IMAGE_BYTES),
            ) {
                ImageDownload::Saved => {
                    store_artist_record(
                        app_data_dir,
                        key.clone(),
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
                        url: Some(artist_image_url(&key, now)),
                    }
                }
                ImageDownload::Placeholder => {
                    store_artist_record(app_data_dir, key, none_image_record(now));
                    none_result()
                }
                ImageDownload::Error(e) => {
                    eprintln!("[external] download artist image failed: {}", e);
                    none_result()
                }
            }
        }
        FetchOutcome::NotFound => {
            store_artist_record(app_data_dir, key, none_image_record(now));
            none_result()
        }
        FetchOutcome::TemporaryError => none_result(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CachedArtistImage {
    pub artist_key: String,
    pub url: String,
}

pub fn load_cached_artist_images(app_data_dir: &PathBuf) -> Vec<CachedArtistImage> {
    ensure_artist_cache_version(app_data_dir);
    let manifest = load_artists_manifest(app_data_dir);
    let dir = artists_dir(app_data_dir);
    let mut out = Vec::new();
    for (key, rec) in manifest.iter() {
        if rec.status != "ok" {
            continue;
        }
        if let Some(file) = &rec.file {
            if dir.join(file).exists() {
                out.push(CachedArtistImage {
                    artist_key: key.clone(),
                    url: artist_image_url(key, rec.fetched_secs),
                });
            }
        }
    }
    out
}

fn covers_dir(app_data_dir: &PathBuf) -> PathBuf {
    let dir = app_data_dir.join("external_covers");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn covers_manifest_path(app_data_dir: &PathBuf) -> PathBuf {
    app_data_dir.join("external_covers.json")
}

const COVERS_CACHE_VERSION: u32 = 1;

fn ensure_covers_cache_version(app_data_dir: &PathBuf) {
    let path = app_data_dir.join("external_covers.version");
    let current = fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0);
    if current != COVERS_CACHE_VERSION {
        let _ = fs::remove_dir_all(app_data_dir.join("external_covers"));
        let _ = fs::remove_file(covers_manifest_path(app_data_dir));
        let _ = fs::write(&path, COVERS_CACHE_VERSION.to_string());
    }
}

fn load_covers_manifest(app_data_dir: &PathBuf) -> HashMap<String, ArtistImageRecord> {
    fs::read_to_string(covers_manifest_path(app_data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_covers_manifest(
    app_data_dir: &PathBuf,
    manifest: &HashMap<String, ArtistImageRecord>,
) -> Result<(), String> {
    let content = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    write_manifest_atomic(&covers_manifest_path(app_data_dir), content.as_bytes())
}

/// See [`artists_manifest_lock`].
fn covers_manifest_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

fn store_cover_record(app_data_dir: &PathBuf, key: String, rec: ArtistImageRecord) {
    let _guard = covers_manifest_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut manifest = load_covers_manifest(app_data_dir);
    manifest.insert(key, rec);
    let _ = save_covers_manifest(app_data_dir, &manifest);
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
    app_data_dir: &PathBuf,
    artist: &str,
    album: &str,
    title: &str,
    refresh: bool,
) -> ArtistImageResult {
    let key = cover_cache_key(artist, album, title);
    if key.is_empty() {
        return none_result();
    }

    ensure_covers_cache_version(app_data_dir);
    if !refresh {
        let manifest = load_covers_manifest(app_data_dir);
        if let Some(rec) = manifest.get(&key) {
            if rec.status == "ok" {
                if let Some(file) = &rec.file {
                    if covers_dir(app_data_dir).join(file).exists() {
                        return ArtistImageResult {
                            status: "ok".into(),
                            source: rec.source.clone(),
                            url: Some(cover_image_url(&key, rec.fetched_secs)),
                        };
                    }
                }
            } else if rec.status == "none" {
                return none_result();
            }
        }
    }

    let now = now_secs();
    let filename = cover_file_name(&key);
    let target = covers_dir(app_data_dir).join(&filename);

    match fetch_track_cover(artist, album, title) {
        FetchOutcome::Found(img) => {
            match download_validate_encode(&img.url, &target, COVER_IMAGE_MAX_DIM, None) {
                ImageDownload::Saved => {
                    store_cover_record(
                        app_data_dir,
                        key.clone(),
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
                        url: Some(cover_image_url(&key, now)),
                    }
                }
                ImageDownload::Placeholder => {
                    store_cover_record(app_data_dir, key, none_image_record(now));
                    none_result()
                }
                ImageDownload::Error(e) => {
                    eprintln!("[external] download cover failed: {}", e);
                    none_result()
                }
            }
        }
        FetchOutcome::NotFound => {
            store_cover_record(app_data_dir, key, none_image_record(now));
            none_result()
        }
        FetchOutcome::TemporaryError => none_result(),
    }
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

/// Keys with a settled (ok/none) result, which the batch pass can skip.
pub fn cover_settled_keys(app_data_dir: &PathBuf) -> std::collections::HashSet<String> {
    ensure_covers_cache_version(app_data_dir);
    load_covers_manifest(app_data_dir)
        .into_iter()
        .filter(|(_, r)| r.status == "ok" || r.status == "none")
        .map(|(k, _)| k)
        .collect()
}

pub fn artist_settled_keys(app_data_dir: &PathBuf) -> std::collections::HashSet<String> {
    ensure_artist_cache_version(app_data_dir);
    load_artists_manifest(app_data_dir)
        .into_iter()
        .filter(|(_, r)| r.status == "ok" || r.status == "none")
        .map(|(k, _)| k)
        .collect()
}

/// Bulk-marks cover keys as "none" without network calls, for tracks that
/// already have a local cover — avoids re-reading their tags on next startup.
pub fn mark_track_covers_none(app_data_dir: &PathBuf, keys: &[String]) {
    if keys.is_empty() {
        return;
    }
    ensure_covers_cache_version(app_data_dir);
    let _guard = covers_manifest_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut manifest = load_covers_manifest(app_data_dir);
    let now = now_secs();
    for key in keys {
        if key.is_empty() {
            continue;
        }
        manifest.entry(key.clone()).or_insert_with(|| ArtistImageRecord {
            file: None,
            source: None,
            status: "none".into(),
            fetched_secs: now,
        });
    }
    let _ = save_covers_manifest(app_data_dir, &manifest);
}

pub fn unix_secs() -> u64 {
    now_secs()
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

    /// Regression: `normalize_title` used to look for `" feat."` without
    /// handling the opening paren, so bracketed tags like "(feat. Artist X)"
    /// weren't stripped and matching local tracks showed as phantoms.
    #[test]
    fn normalize_title_strips_parenthesized_feat_and_bracketed_remaster_marker() {
        assert_eq!(
            normalize_title("Song Title (feat. Artist X) [Remastered 2011]"),
            normalize("Song Title")
        );
    }

    #[test]
    fn levenshtein_matches_known_distances() {
        assert_eq!(levenshtein(&[], &[]), 0);
        let empty: Vec<char> = vec![];
        let abc: Vec<char> = "abc".chars().collect();
        assert_eq!(levenshtein(&empty, &abc), 3);
        let kitten: Vec<char> = "kitten".chars().collect();
        let sitting: Vec<char> = "sitting".chars().collect();
        assert_eq!(levenshtein(&kitten, &sitting), 3);
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
        // Without contains_bonus this would be ~0.25; the +0.15 bonus pushes
        // it to ~0.40 — higher, but still far from "similar".
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
        // title_sim ~= 1.0 (exact match), artist_sim fixed at 0.7.
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
        // remix/live/acoustic are distinct versions, not noise, and must be kept.
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
        // " feat " is a prefix of " featuring "; applying it first would
        // fragment " featuring " before the longer separator gets a chance.
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
        // Different (album, artist) pairs must not collapse when words shift across fields.
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
        // Same logical key must not yield the same URL for an artist avatar vs.
        // an album cover, or the protocol handler would serve the wrong image.
        let artist_url = artist_image_url("queen", 1);
        let cover_url = cover_image_url("queen", 1);
        assert_ne!(artist_url, cover_url);
        assert!(artist_url.contains("/artist/"));
        assert!(cover_url.contains("/cover/"));
    }

    #[test]
    fn image_urls_change_with_version_for_cache_busting() {
        let v1 = artist_image_url("queen", 1);
        let v2 = artist_image_url("queen", 2);
        assert_ne!(v1, v2);
    }

    #[test]
    fn artist_file_name_and_cover_file_name_use_distinct_prefixes() {
        let key = "queen";
        let artist_file = artist_file_name(key);
        let cover_file = cover_file_name(key);
        assert!(artist_file.ends_with(".webp"));
        assert!(cover_file.ends_with(".webp"));
        assert!(cover_file.starts_with('c'));
        assert_ne!(artist_file, cover_file);
    }

    #[test]
    fn unix_secs_returns_a_plausible_current_timestamp() {
        let secs = unix_secs();
        assert!(secs > 1_700_000_000, "secs={secs}");
    }
}
