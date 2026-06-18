//! Внешние (открытые, без API-ключей) источники метаданных.
//!
//! Единый интерфейс [`Provider`], под который подгоняются разные сервисы
//! (iTunes Search API, Deezer, MusicBrainz / Cover Art Archive). Агрегатор
//! опрашивает провайдеры, оценивает совпадение по схожести названий и
//! выбирает лучший результат.
//!
//! Используется для двух задач:
//!  - получить «канонический» порядок треков в альбоме (+ список треков,
//!    которых нет в локальной библиотеке — «фантомные»);
//!  - получить аватарку исполнителя.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

const USER_AGENT: &str =
    "LFMusic/0.1.0 ( https://github.com/melix/LFMusic )";

/// Один трек из внешнего источника.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalTrack {
    pub position: u32,
    pub disc: u32,
    pub title: String,
    pub artist: String,
    pub duration_secs: Option<u32>,
}

/// Результат поиска альбома во внешнем источнике.
#[derive(Debug, Clone)]
pub struct ExternalAlbum {
    pub source: String,
    pub artist: String,
    pub title: String,
    pub tracks: Vec<ExternalTrack>,
}

/// Найденная аватарка исполнителя (ссылка на изображение).
#[derive(Debug, Clone)]
pub struct ExternalImage {
    pub source: String,
    pub url: String,
    /// Имя исполнителя, как его вернул источник (для оценки совпадения).
    pub matched_name: String,
}

/// Общий интерфейс источника. Любой метод может вернуть `Ok(None)`, если
/// источник ничего не нашёл, или `Err` при сетевой/парс-ошибке.
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

    /// Обложка трека/альбома. Приоритет — по альбому (одна обложка на все треки
    /// альбома), при пустом альбоме — по названию трека.
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

// ──────────────────────────────────────────────────────────────────────────────
// HTTP-клиент (один на процесс)
// ──────────────────────────────────────────────────────────────────────────────

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

/// Глобальный троттл для MusicBrainz / Cover Art Archive: их условия требуют не
/// более ~1 запроса в секунду на клиента, иначе сыпется 503 и временный бан.
/// Запросы выполняются на blocking-пуле, поэтому короткий `sleep` под мьютексом
/// просто сериализует обращения и держит интервал.
fn mb_throttle() {
    use std::sync::Mutex;
    use std::time::Instant;
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    let lock = LAST.get_or_init(|| Mutex::new(None));
    let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let min_gap = Duration::from_millis(1100);
    if let Some(prev) = *guard {
        let elapsed = prev.elapsed();
        if elapsed < min_gap {
            std::thread::sleep(min_gap - elapsed);
        }
    }
    *guard = Some(Instant::now());
}

// ──────────────────────────────────────────────────────────────────────────────
// Нормализация и оценка схожести
// ──────────────────────────────────────────────────────────────────────────────

/// Приводит строку к каноничному виду для сравнения: нижний регистр,
/// без диакритики (грубо), только буквы/цифры/пробелы, схлопнутые пробелы.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true; // схлопываем ведущие пробелы
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

/// Убирает «шум» из названия трека/альбома: feat., remastered, скобочные
/// уточнения и т.п. — чтобы сопоставление было устойчивее.
fn normalize_title(s: &str) -> String {
    let lower = s.to_lowercase();
    // Срезаем всё после маркеров фитов.
    let mut base = lower.as_str();
    for marker in [" feat.", " feat ", " ft.", " ft ", " featuring "] {
        if let Some(idx) = base.find(marker) {
            base = &base[..idx];
        }
    }
    let cleaned: String = base
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

/// Схожесть двух строк в диапазоне 0.0..=1.0 (1.0 — идентичны после нормализации).
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
    // Бонус за вхождение более короткой строки в более длинную (для «Album» vs
    // «Album (Deluxe Edition)»).
    let contains_bonus = if na.contains(&nb) || nb.contains(&na) { 0.15 } else { 0.0 };
    (ratio + contains_bonus).clamp(0.0, 1.0)
}

/// Дефолтная картинка-«силуэт» источника (исполнитель без реального фото).
/// Такие использовать не нужно — лучше откатиться на локальную обложку.
fn is_placeholder_image(url: &str) -> bool {
    // Deezer отдаёт URL с пустым md5 в пути, когда фото нет, напр.
    // https://e-cdns-images.dzcdn.net/images/artist//1000x1000-000000-80-0-0.jpg
    url.contains("/images/artist//")
        || url.contains("/images/cover//")
        || url.contains("/images/user//")
        // Известные дефолтные картинки Deezer (один и тот же силуэт/«волна»
        // отдаётся под разными md5): md5 пустой строки, серый силуэт, волна.
        || url.contains("d41d8cd98f00b204e9800998ecf8427e")
        || url.contains("1fecff61b191427ec4d2760a77527bf5")
        || url.contains("e460b01306de4c8d790508e0d80d1fdf")
}

/// Минимальный размер картинки (в байтах), ниже которого считаем её дефолтным
/// плейсхолдером источника. Дефолты Deezer (силуэт/волна) — ≤33 КБ при 1000×1000,
/// реальные фото — 90 КБ и больше; порог с большим запасом между ними.
const MIN_REAL_IMAGE_BYTES: usize = 45_000;

/// Насколько найденный альбом соответствует запросу (артист + название).
fn album_score(query_artist: &str, query_album: &str, album: &ExternalAlbum) -> f32 {
    let title_sim = similarity(query_album, &album.title);
    let artist_sim = if query_artist.is_empty() {
        0.7
    } else {
        similarity(query_artist, &album.artist)
    };
    // Название важнее имени артиста (артист часто указан в библиотеке иначе).
    title_sim * 0.7 + artist_sim * 0.3
}

// ──────────────────────────────────────────────────────────────────────────────
// Провайдер: iTunes Search API
// ──────────────────────────────────────────────────────────────────────────────

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
        let search: SearchResp = client
            .get("https://itunes.apple.com/search")
            .query(&[
                ("term", term.as_str()),
                ("entity", "album"),
                ("limit", "6"),
                ("media", "music"),
            ])
            .send()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())?;

        // Лучший альбом-кандидат по названию.
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

        let lookup: LookupResp = client
            .get("https://itunes.apple.com/lookup")
            .query(&[
                ("id", collection_id.to_string().as_str()),
                ("entity", "song"),
                ("limit", "300"),
            ])
            .send()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())?;

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

        let resp: SearchResp = client
            .get("https://itunes.apple.com/search")
            .query(&[
                ("term", query.as_str()),
                ("entity", entity),
                ("limit", "5"),
                ("media", "music"),
            ])
            .send()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())?;

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
        // Апскейл превью iTunes (100x100) до 1000x1000 — Apple отдаёт исходник в
        // полном разрешении, превью масштабируется простой заменой в URL.
        let url = art.replace("100x100bb", "1000x1000bb");
        Ok(Some(ExternalImage {
            source: "iTunes".to_string(),
            url,
            matched_name: name,
        }))
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Провайдер: Deezer
// ──────────────────────────────────────────────────────────────────────────────

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
        let search: SearchResp = client
            .get("https://api.deezer.com/search/album")
            .query(&[("q", q.as_str()), ("limit", "6")])
            .send()
            .map_err(|e| e.to_string())?
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

        let album_resp: AlbumResp = client
            .get(format!("https://api.deezer.com/album/{}", hit.id))
            .send()
            .map_err(|e| e.to_string())?
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

        let search: SearchResp = client
            .get("https://api.deezer.com/search/artist")
            .query(&[("q", artist), ("limit", "8")])
            .send()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())?;

        // Берём лучшего по совпадению имени, при равенстве — более раннего
        // (Deezer отдаёт результаты по релевантности, поэтому первый
        // одноимённый артист — самый популярный/нужный). Пустые и дефолтные
        // картинки-силуэты пропускаем.
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
            let resp: SearchResp = client
                .get("https://api.deezer.com/search/album")
                .query(&[("q", q.as_str()), ("limit", "5")])
                .send()
                .map_err(|e| e.to_string())?
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

        // Без альбома — ищем по треку, берём обложку его альбома.
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
        let resp: TrackSearchResp = client
            .get("https://api.deezer.com/search/track")
            .query(&[("q", q.as_str()), ("limit", "5")])
            .send()
            .map_err(|e| e.to_string())?
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

// ──────────────────────────────────────────────────────────────────────────────
// Провайдер: MusicBrainz (+ Cover Art Archive по сути для обложек релизов)
// ──────────────────────────────────────────────────────────────────────────────

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
        mb_throttle();
        let search: SearchResp = client
            .get("https://musicbrainz.org/ws/2/release/")
            .query(&[("query", query.as_str()), ("fmt", "json"), ("limit", "6")])
            .send()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())?;

        // Среди кандидатов выбираем с лучшим совпадением названия и наибольшим
        // числом треков (предпочитаем полные релизы, а не синглы).
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

        mb_throttle();
        let detail: ReleaseResp = client
            .get(format!("https://musicbrainz.org/ws/2/release/{}", release.id))
            .query(&[("inc", "recordings"), ("fmt", "json")])
            .send()
            .map_err(|e| e.to_string())?
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

// ──────────────────────────────────────────────────────────────────────────────
// Провайдер: Cover Art Archive (обложки релизов из MusicBrainz)
// ──────────────────────────────────────────────────────────────────────────────

/// Ищет release-group в MusicBrainz и возвращает `(mbid, title)` лучшего по
/// совпадению названия (и, если задан, артиста). Release-group — это «альбом»
/// поверх всех изданий, одна обложка на все переиздания.
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
    mb_throttle();
    let resp: Resp = client
        .get("https://musicbrainz.org/ws/2/release-group/")
        .query(&[("query", query.as_str()), ("fmt", "json"), ("limit", "6")])
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    let best = resp
        .groups
        .into_iter()
        .filter_map(|g| {
            let sim = similarity(album, &g.title);
            // Если артист известен — отбрасываем одноимённый альбом другого автора.
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
        // CAA индексирует обложки по релизам/группам релизов MusicBrainz —
        // работаем только когда известен альбом.
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

        mb_throttle();
        let resp = client
            .get(format!("https://coverartarchive.org/release-group/{}", rg_id))
            .send()
            .map_err(|e| e.to_string())?;
        // Нет обложки для группы релизов — CAA отвечает 404.
        if !resp.status().is_success() {
            return Ok(None);
        }
        let caa: CaaResp = resp.json().map_err(|e| e.to_string())?;

        // Предпочитаем помеченную как «front», иначе первую доступную.
        let Some(img) = caa.images.iter().find(|i| i.front).or_else(|| caa.images.first())
        else {
            return Ok(None);
        };
        // Берём готовый превью 1200/500, иначе оригинал (бывает очень крупным).
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

// ──────────────────────────────────────────────────────────────────────────────
// Провайдер: Wikipedia (аватарки исполнителей — фолбэк, когда нет в Deezer)
// ──────────────────────────────────────────────────────────────────────────────

struct Wikipedia;

/// Содержит ли краткое описание из Wikidata признак музыкального исполнителя.
/// Без этой проверки поиск легко подсунет статью-омоним (город, фильм, цвет).
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

/// Ищет в конкретной языковой Wikipedia статью об исполнителе и возвращает её
/// заглавное фото. Жёсткая фильтрация по «музыкальности» описания и совпадению
/// имени: при сомнении возвращаем `None`, чтобы не подставить чужую картинку.
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
    let resp: Resp = client
        .get(&endpoint)
        .query(&[
            ("action", "query"),
            ("format", "json"),
            ("prop", "pageimages|description"),
            ("piprop", "thumbnail"),
            ("pithumbsize", "800"),
            ("generator", "search"),
            ("gsrsearch", artist),
            ("gsrlimit", "5"),
            ("gsrnamespace", "0"),
        ])
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    let Some(query) = resp.query else {
        return Ok(None);
    };

    // Кандидаты: фото есть + описание музыкальное + имя совпадает. Лучший по
    // совпадению имени, при равенстве — выше в поисковой выдаче (меньший index).
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
        // Сначала англоязычная (шире покрытие и стабильные англ. описания),
        // затем русская (локальные исполнители, которых нет в en).
        for lang in ["en", "ru"] {
            if let Some(img) = wiki_lookup(client, lang, artist)? {
                return Ok(Some(img));
            }
        }
        Ok(None)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Агрегатор
// ──────────────────────────────────────────────────────────────────────────────

fn album_providers() -> Vec<Box<dyn Provider>> {
    vec![Box::new(ITunes), Box::new(Deezer), Box::new(MusicBrainz)]
}

fn image_providers() -> Vec<Box<dyn Provider>> {
    // Deezer — основной (богатые фото артистов без ключа); Wikipedia — фолбэк
    // для исполнителей, которых у Deezer нет (см. семантику в fetch_artist_image).
    vec![Box::new(Deezer), Box::new(Wikipedia)]
}

fn cover_providers() -> Vec<Box<dyn Provider>> {
    // iTunes — хорошее качество артов; Deezer — фолбэк; Cover Art Archive —
    // последний рубеж (нишевые/региональные релизы, которых нет в стримингах).
    vec![Box::new(ITunes), Box::new(Deezer), Box::new(CoverArtArchive)]
}

/// Опрашивает источники и возвращает обложку трека/альбома (первое совпадение).
pub fn fetch_track_cover(artist: &str, album: &str, title: &str) -> Option<ExternalImage> {
    let client = client();
    for provider in cover_providers() {
        match provider.cover(client, artist, album, title) {
            Ok(Some(img)) => return Some(img),
            Ok(None) => {}
            Err(e) => {
                eprintln!("[external] {} cover error: {}", provider.name(), e);
            }
        }
    }
    None
}

/// Опрашивает источники и возвращает лучший по совпадению список треков альбома.
pub fn fetch_album_tracklist(artist: &str, album: &str) -> Option<ExternalAlbum> {
    let client = client();
    let mut best: Option<(f32, ExternalAlbum)> = None;
    for provider in album_providers() {
        match provider.album_tracklist(client, artist, album) {
            Ok(Some(candidate)) => {
                let score = album_score(artist, album, &candidate);
                if score < 0.5 {
                    continue;
                }
                // Если известен артист — отбрасываем альбом другого исполнителя
                // (тот же тайтл, но чужой артист): иначе можно подтянуть не тот
                // одноимённый альбом.
                if !artist.is_empty() && similarity(artist, &candidate.artist) < 0.35 {
                    continue;
                }
                if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                    best = Some((score, candidate));
                }
                // Идеальное совпадение — дальше можно не искать.
                if score >= 0.97 {
                    break;
                }
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("[external] {} album error: {}", provider.name(), e);
            }
        }
    }
    best.map(|(_, album)| album)
}

/// Опрашивает источники и возвращает лучшую аватарку исполнителя. Провайдеры
/// идут по приоритету: как только один уверенно совпал (≥0.85), последующие
/// (например, Wikipedia) не опрашиваются — экономим сеть и снижаем риск
/// подставить чужое фото из менее точного источника.
pub fn fetch_artist_image(artist: &str) -> Option<ExternalImage> {
    let client = client();
    let mut best: Option<(f32, ExternalImage)> = None;
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
                eprintln!("[external] {} image error: {}", provider.name(), e);
            }
        }
    }
    best.map(|(_, img)| img)
}

// ──────────────────────────────────────────────────────────────────────────────
// Сопоставление внешних треков с локальными файлами
// ──────────────────────────────────────────────────────────────────────────────

/// Финальный макет альбома для фронтенда.
#[derive(Debug, Clone, Serialize)]
pub struct AlbumLayout {
    /// "ok" — есть внешний порядок; "none" — внешних данных нет.
    pub status: String,
    pub source: Option<String>,
    pub entries: Vec<AlbumLayoutEntry>,
}

/// Запись финального макета альбома для фронтенда.
#[derive(Debug, Clone, Serialize)]
pub struct AlbumLayoutEntry {
    pub position: u32,
    pub disc: u32,
    pub title: String,
    pub artist: String,
    pub duration_secs: Option<u32>,
    /// Путь к локальному файлу, если трек есть в библиотеке. `None` — «фантом».
    pub file_path: Option<String>,
}

/// Сопоставляет внешний список треков с локальными (file_path, title).
/// Возвращает макет в каноничном порядке. Локальные треки, не нашедшие места
/// в каноне, в макет не попадают — их фронт дописывает в конец сам.
pub fn build_layout(
    tracks: &[ExternalTrack],
    local: &[(String, String)],
) -> Vec<AlbumLayoutEntry> {
    // Нормализованный заголовок -> список локальных путей (могут быть дубли).
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

// ──────────────────────────────────────────────────────────────────────────────
// Дисковый кэш аватарок исполнителей (по образцу covers.rs)
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ArtistImageRecord {
    /// Имя файла на диске (если есть успешно скачанная аватарка).
    file: Option<String>,
    source: Option<String>,
    /// "ok" | "none" — для негативного кэша (чтобы не дёргать сеть постоянно).
    status: String,
    fetched_secs: u64,
}

/// Результат запроса аватарки исполнителя для фронтенда.
#[derive(Debug, Clone, Serialize)]
pub struct ArtistImageResult {
    /// "ok" | "none"
    pub status: String,
    pub source: Option<String>,
    pub data_url: Option<String>,
}

fn artists_dir(app_data_dir: &PathBuf) -> PathBuf {
    let dir = app_data_dir.join("external_artists");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Версия кэша аватарок. Повышение сбрасывает дисковый кэш один раз —
/// например, чтобы выкинуть дефолтные картинки-силуэты, закэшированные до
/// добавления их фильтрации.
const ARTIST_IMAGE_CACHE_VERSION: u32 = 4;

/// Сбрасывает кэш аватарок, если его версия устарела (выполняется один раз).
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
    fs::write(artists_manifest_path(app_data_dir), content).map_err(|e| e.to_string())
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

fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    }
}

fn ext_from_url_or_ct(url: &str, content_type: Option<&str>) -> &'static str {
    let lower = url.to_lowercase();
    if lower.contains(".png") {
        return "png";
    }
    if lower.contains(".webp") {
        return "webp";
    }
    if lower.contains(".gif") {
        return "gif";
    }
    if lower.contains(".jpg") || lower.contains(".jpeg") {
        return "jpg";
    }
    match content_type {
        Some(ct) if ct.contains("png") => "png",
        Some(ct) if ct.contains("webp") => "webp",
        Some(ct) if ct.contains("gif") => "gif",
        _ => "jpg",
    }
}

fn read_as_data_url(file_path: &PathBuf) -> Option<String> {
    let data = fs::read(file_path).ok()?;
    let ext = file_path.extension().and_then(|s| s.to_str()).unwrap_or("jpg");
    Some(format!(
        "data:{};base64,{}",
        mime_for_ext(ext),
        STANDARD.encode(&data)
    ))
}

/// Скачивает изображение по ссылке и сохраняет в кэш аватарок.
/// `Ok(Some((имя файла, data-URL)))` — успех; `Ok(None)` — скачали, но это
/// дефолтный плейсхолдер (пустой/слишком маленький) → ставить не нужно;
/// `Err` — сетевая/IO-ошибка (можно повторить позже).
fn download_artist_image(
    app_data_dir: &PathBuf,
    artist_key: &str,
    url: &str,
) -> Result<Option<(String, String)>, String> {
    let resp = client().get(url).send().map_err(|e| e.to_string())?;
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = resp.bytes().map_err(|e| e.to_string())?;
    // Пустой или подозрительно маленький файл — дефолтный плейсхолдер источника.
    if bytes.len() < MIN_REAL_IMAGE_BYTES {
        return Ok(None);
    }
    let ext = ext_from_url_or_ct(url, content_type.as_deref());
    let filename = format!("{:016x}.{}", key_hash(artist_key), ext);
    let dir = artists_dir(app_data_dir);
    let target = dir.join(&filename);
    fs::write(&target, &bytes).map_err(|e| e.to_string())?;
    let data_url = format!("data:{};base64,{}", mime_for_ext(ext), STANDARD.encode(&bytes));
    Ok(Some((filename, data_url)))
}

/// Получить аватарку исполнителя: сначала из кэша, иначе из сети (если включено
/// настройкой — проверяется на стороне команды). `refresh` форсирует повторную
/// загрузку из сети.
pub fn get_or_fetch_artist_image(
    app_data_dir: &PathBuf,
    artist_name: &str,
    refresh: bool,
) -> ArtistImageResult {
    let key = artist_key(artist_name);
    if key.is_empty() {
        return ArtistImageResult { status: "none".into(), source: None, data_url: None };
    }

    ensure_artist_cache_version(app_data_dir);
    let mut manifest = load_artists_manifest(app_data_dir);
    if !refresh {
        if let Some(rec) = manifest.get(&key) {
            if rec.status == "ok" {
                if let Some(file) = &rec.file {
                    let path = artists_dir(app_data_dir).join(file);
                    if let Some(data_url) = read_as_data_url(&path) {
                        return ArtistImageResult {
                            status: "ok".into(),
                            source: rec.source.clone(),
                            data_url: Some(data_url),
                        };
                    }
                }
            } else if rec.status == "none" {
                // Кэш без срока истечения: «не найдено» больше не перепрашиваем
                // (см. батч-проход подготовки внешних источников).
                return ArtistImageResult { status: "none".into(), source: None, data_url: None };
            }
        }
    }

    // Идём в сеть.
    let none_result = || ArtistImageResult { status: "none".into(), source: None, data_url: None };
    let cache_none = |manifest: &mut HashMap<String, ArtistImageRecord>, key: String| {
        manifest.insert(
            key,
            ArtistImageRecord {
                file: None,
                source: None,
                status: "none".into(),
                fetched_secs: now_secs(),
            },
        );
    };

    match fetch_artist_image(artist_name) {
        Some(img) => match download_artist_image(app_data_dir, &key, &img.url) {
            Ok(Some((file, data_url))) => {
                manifest.insert(
                    key,
                    ArtistImageRecord {
                        file: Some(file),
                        source: Some(img.source.clone()),
                        status: "ok".into(),
                        fetched_secs: now_secs(),
                    },
                );
                let _ = save_artists_manifest(app_data_dir, &manifest);
                ArtistImageResult { status: "ok".into(), source: Some(img.source), data_url: Some(data_url) }
            }
            // Скачали дефолтный плейсхолдер — не ставим, кэшируем «нет».
            Ok(None) => {
                cache_none(&mut manifest, key);
                let _ = save_artists_manifest(app_data_dir, &manifest);
                none_result()
            }
            // Сетевая ошибка — не кэшируем, попробуем позже.
            Err(e) => {
                eprintln!("[external] download artist image failed: {}", e);
                none_result()
            }
        },
        None => {
            cache_none(&mut manifest, key);
            let _ = save_artists_manifest(app_data_dir, &manifest);
            none_result()
        }
    }
}

/// Загрузить все уже закэшированные аватарки (для быстрой отрисовки на старте).
#[derive(Debug, Clone, Serialize)]
pub struct CachedArtistImage {
    pub artist_key: String,
    pub data_url: String,
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
            if let Some(data_url) = read_as_data_url(&dir.join(file)) {
                out.push(CachedArtistImage { artist_key: key.clone(), data_url });
            }
        }
    }
    out
}

// ──────────────────────────────────────────────────────────────────────────────
// Дисковый кэш обложек треков/альбомов (для треков без встроенной обложки)
// ──────────────────────────────────────────────────────────────────────────────

fn covers_dir(app_data_dir: &PathBuf) -> PathBuf {
    let dir = app_data_dir.join("external_covers");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn covers_manifest_path(app_data_dir: &PathBuf) -> PathBuf {
    app_data_dir.join("external_covers.json")
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
    fs::write(covers_manifest_path(app_data_dir), content).map_err(|e| e.to_string())
}

/// Ключ кэша обложки: артист + альбом (одна обложка на альбом), при пустом
/// альбоме — артист + название трека.
fn cover_cache_key(artist: &str, album: &str, title: &str) -> String {
    if !album.trim().is_empty() {
        normalize(&format!("{} {}", artist, album))
    } else {
        normalize(&format!("{} {} t", artist, title))
    }
}

/// Скачивает обложку и кладёт в кэш. `Ok(None)` — пустой ответ.
fn download_cover_image(
    app_data_dir: &PathBuf,
    key: &str,
    url: &str,
) -> Result<Option<(String, String)>, String> {
    let resp = client().get(url).send().map_err(|e| e.to_string())?;
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = resp.bytes().map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let ext = ext_from_url_or_ct(url, content_type.as_deref());
    let filename = format!("c{:016x}.{}", key_hash(key), ext);
    let dir = covers_dir(app_data_dir);
    fs::write(dir.join(&filename), &bytes).map_err(|e| e.to_string())?;
    let data_url = format!("data:{};base64,{}", mime_for_ext(ext), STANDARD.encode(&bytes));
    Ok(Some((filename, data_url)))
}

/// Получить обложку трека/альбома: сначала из кэша, иначе из сети.
pub fn get_or_fetch_track_cover(
    app_data_dir: &PathBuf,
    artist: &str,
    album: &str,
    title: &str,
    refresh: bool,
) -> ArtistImageResult {
    let none_result = || ArtistImageResult { status: "none".into(), source: None, data_url: None };
    let key = cover_cache_key(artist, album, title);
    if key.is_empty() {
        return none_result();
    }

    let mut manifest = load_covers_manifest(app_data_dir);
    if !refresh {
        if let Some(rec) = manifest.get(&key) {
            if rec.status == "ok" {
                if let Some(file) = &rec.file {
                    let path = covers_dir(app_data_dir).join(file);
                    if let Some(data_url) = read_as_data_url(&path) {
                        return ArtistImageResult {
                            status: "ok".into(),
                            source: rec.source.clone(),
                            data_url: Some(data_url),
                        };
                    }
                }
            } else if rec.status == "none" {
                // Бессрочный негативный кэш (см. батч-проход подготовки).
                return none_result();
            }
        }
    }

    let cache_none = |manifest: &mut HashMap<String, ArtistImageRecord>, key: String| {
        manifest.insert(
            key,
            ArtistImageRecord {
                file: None,
                source: None,
                status: "none".into(),
                fetched_secs: now_secs(),
            },
        );
    };

    match fetch_track_cover(artist, album, title) {
        Some(img) => match download_cover_image(app_data_dir, &key, &img.url) {
            Ok(Some((file, data_url))) => {
                manifest.insert(
                    key,
                    ArtistImageRecord {
                        file: Some(file),
                        source: Some(img.source.clone()),
                        status: "ok".into(),
                        fetched_secs: now_secs(),
                    },
                );
                let _ = save_covers_manifest(app_data_dir, &manifest);
                ArtistImageResult { status: "ok".into(), source: Some(img.source), data_url: Some(data_url) }
            }
            Ok(None) => {
                cache_none(&mut manifest, key);
                let _ = save_covers_manifest(app_data_dir, &manifest);
                none_result()
            }
            Err(e) => {
                eprintln!("[external] download cover failed: {}", e);
                none_result()
            }
        },
        None => {
            cache_none(&mut manifest, key);
            let _ = save_covers_manifest(app_data_dir, &manifest);
            none_result()
        }
    }
}

/// Ключ кэша альбома с учётом исполнителя — чтобы два одноимённых альбома от
/// разных артистов не делили один кэш.
pub fn album_cache_key(album: &str, artist: &str) -> String {
    format!("{}\u{1}{}", normalize(album), normalize(artist))
}

// ──────────────────────────────────────────────────────────────────────────────
// Поддержка батч-прохода подготовки внешних источников
// ──────────────────────────────────────────────────────────────────────────────

/// Ключ кэша обложки трека/альбома (тот же, что у `get_or_fetch_track_cover`).
/// Нужен батч-проходу, чтобы определить, обработан ли уже трек/альбом.
pub fn track_cover_key(artist: &str, album: &str, title: &str) -> String {
    cover_cache_key(artist, album, title)
}

/// Нормализованный ключ исполнителя (тот же, что у кэша аватарок).
pub fn artist_image_key(name: &str) -> String {
    artist_key(name)
}

/// Ключи обложек с окончательным результатом (ok/none) — такие батч-проход
/// пропускает (сеть не дёргает, теги не перечитывает).
pub fn cover_settled_keys(app_data_dir: &PathBuf) -> std::collections::HashSet<String> {
    load_covers_manifest(app_data_dir)
        .into_iter()
        .filter(|(_, r)| r.status == "ok" || r.status == "none")
        .map(|(k, _)| k)
        .collect()
}

/// Ключи исполнителей с окончательным результатом (ok/none) аватарки.
pub fn artist_settled_keys(app_data_dir: &PathBuf) -> std::collections::HashSet<String> {
    ensure_artist_cache_version(app_data_dir);
    load_artists_manifest(app_data_dir)
        .into_iter()
        .filter(|(_, r)| r.status == "ok" || r.status == "none")
        .map(|(k, _)| k)
        .collect()
}

/// Пометить набор обложек как «нет внешней» одним сохранением манифеста (без
/// сети). Используется батч-проходом для альбомов/треков, у которых уже есть
/// локальная обложка, — чтобы при следующем старте не перечитывать их теги.
pub fn mark_track_covers_none(app_data_dir: &PathBuf, keys: &[String]) {
    if keys.is_empty() {
        return;
    }
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

/// Текущее время в секундах с эпохи (для команд).
pub fn unix_secs() -> u64 {
    now_secs()
}
