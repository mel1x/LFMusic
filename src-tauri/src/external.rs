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
//!
//! Картинки (аватарки/обложки) НЕ передаются через IPC как base64: они
//! скачиваются, валидируются и пережимаются в WebP на диск, а фронт получает
//! короткий URL кастомного протокола `lfmusic-external://…` (см.
//! [`protocol_handler`]) — по образцу `lfmusic-cover` для обложек треков.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::http::{header, Response, StatusCode};
use tauri::{AppHandle, Manager, Runtime, UriSchemeContext, UriSchemeResponder};

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

/// Итог опроса источников. Ключевое отличие от «просто `Option`»: различаем
/// «точно не нашли» (все провайдеры ответили «нет») и «временную ошибку»
/// (сеть/парсинг/rate-limit) — последнюю НЕ кэшируем как «нет навсегда».
pub enum FetchOutcome<T> {
    /// Нашли подходящий результат.
    Found(T),
    /// Все провайдеры реально ответили «не найдено».
    NotFound,
    /// Хотя бы один провайдер упал с ошибкой, и никто не нашёл — повторим позже.
    TemporaryError,
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
// HTTP-клиент (один на процесс) + пер-хост лимитер с backoff
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

/// Состояние лимитера по одному хосту.
#[derive(Default)]
struct HostState {
    /// Время, на которое зарезервирован следующий запрос (выдержка интервала).
    next_slot: Option<Instant>,
    /// До этого момента хост в backoff (после 429/503 с Retry-After).
    blocked_until: Option<Instant>,
}

fn host_registry() -> &'static Mutex<HashMap<String, HostState>> {
    static REG: OnceLock<Mutex<HashMap<String, HostState>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Минимальный интервал между запросами к хосту. MusicBrainz / Cover Art Archive
/// требуют ≤1 запроса в секунду; iTunes/Deezer ограничиваем мягче (плюс backoff
/// на 429/503). Это сериализует и ленивые запросы фронта, и батч-подготовку,
/// идущие параллельно.
fn host_min_gap(host: &str) -> Duration {
    match host {
        "musicbrainz.org" | "coverartarchive.org" => Duration::from_millis(1100),
        "itunes.apple.com" => Duration::from_millis(300),
        "api.deezer.com" => Duration::from_millis(220),
        _ => Duration::from_millis(120),
    }
}

/// Грубо извлекает хост из URL (без полноценного парсинга URL).
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

/// Ждёт, пока хосту можно слать следующий запрос (интервал + backoff), и
/// резервирует слот. Мьютекс под сон НЕ держим: считаем нужную задержку,
/// сдвигаем слот и спим уже после освобождения мьютекса.
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
        // Следующий слот — не раньше target + интервал.
        st.next_slot = Some(target + host_min_gap(host));
        target.saturating_duration_since(now)
    };
    if !sleep_dur.is_zero() {
        std::thread::sleep(sleep_dur);
    }
}

/// Регистрирует backoff для хоста после 429/503 (берём максимум из текущего и
/// нового, чтобы не сокращать уже назначенную паузу).
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

/// Сколько секунд ждать по заголовку `Retry-After` (или дефолт 5с). Поддерживаем
/// только формат «дельта-секунды»; ограничиваем сверху, чтобы не зависнуть.
fn retry_after_secs(resp: &reqwest::blocking::Response) -> u64 {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(5)
        .clamp(1, 120)
}

/// Отправляет запрос с учётом пер-хост лимита и backoff. На 429/503 регистрирует
/// backoff и возвращает `Err` (провайдер расценит это как временную ошибку и не
/// закэширует «нет»).
fn send_throttled(
    builder: reqwest::blocking::RequestBuilder,
    host: &str,
) -> Result<reqwest::blocking::Response, String> {
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

/// GET изображения с лимитом по хосту (хост берётся из самого URL).
fn get_image(url: &str) -> Result<reqwest::blocking::Response, String> {
    let host = host_of(url);
    send_throttled(client().get(url), &host)
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

/// Жёсткий предел размера скачиваемого изображения (защита от «бомбы»/мусора).
const MAX_DOWNLOAD_BYTES: usize = 20 * 1024 * 1024;

/// Целевой максимум стороны при пережатии в WebP (downscale, как у миниатюр).
const ARTIST_IMAGE_MAX_DIM: u32 = 800;
const COVER_IMAGE_MAX_DIM: u32 = 1000;

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
        let search: SearchResp = send_throttled(
            client.get("https://itunes.apple.com/search").query(&[
                ("term", term.as_str()),
                ("entity", "album"),
                ("limit", "6"),
                ("media", "music"),
            ]),
            "itunes.apple.com",
        )?
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

        let lookup: LookupResp = send_throttled(
            client.get("https://itunes.apple.com/lookup").query(&[
                ("id", collection_id.to_string().as_str()),
                ("entity", "song"),
                ("limit", "300"),
            ]),
            "itunes.apple.com",
        )?
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

        let resp: SearchResp = send_throttled(
            client.get("https://itunes.apple.com/search").query(&[
                ("term", query.as_str()),
                ("entity", entity),
                ("limit", "5"),
                ("media", "music"),
            ]),
            "itunes.apple.com",
        )?
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
        let search: SearchResp = send_throttled(
            client
                .get("https://musicbrainz.org/ws/2/release/")
                .query(&[("query", query.as_str()), ("fmt", "json"), ("limit", "6")]),
            "musicbrainz.org",
        )?
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

        let resp = send_throttled(
            client.get(format!("https://coverartarchive.org/release-group/{}", rg_id)),
            "coverartarchive.org",
        )?;
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
/// `TemporaryError`, если никто не нашёл, но хотя бы один провайдер упал.
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

/// Опрашивает источники и возвращает лучший по совпадению список треков альбома.
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

/// Опрашивает источники и возвращает лучшую аватарку исполнителя. Провайдеры
/// идут по приоритету: как только один уверенно совпал (≥0.85), последующие
/// (например, Wikipedia) не опрашиваются — экономим сеть и снижаем риск
/// подставить чужое фото из менее точного источника.
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
// Кастомный протокол lfmusic-external (отдаёт кэшированные WebP без base64 в IPC)
// ──────────────────────────────────────────────────────────────────────────────

const SCHEME: &str = "lfmusic-external";

/// Платформенный origin кастомного протокола (как его видит вебвью).
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

/// Чистый билдер URL картинки для `<img :src>`. `kind` — "artist"|"cover",
/// `hex` — hex-хеш ключа кэша, `version` — для cache-busting (момент скачивания):
/// при refresh время меняется → меняется URL → вебвью перезагружает картинку.
fn external_protocol_url(kind: &str, hex: &str, version: u64) -> String {
    format!("{}/{}/{}?v={}", scheme_origin(), kind, hex, version)
}

fn artist_image_url(key: &str, version: u64) -> String {
    external_protocol_url("artist", &format!("{:016x}", key_hash(key)), version)
}

fn cover_image_url(key: &str, version: u64) -> String {
    external_protocol_url("cover", &format!("{:016x}", key_hash(key)), version)
}

/// Имя файла кэшированной аватарки (всегда WebP — после пережатия).
fn artist_file_name(key: &str) -> String {
    format!("{:016x}.webp", key_hash(key))
}

/// Имя файла кэшированной обложки (префикс `c`, чтобы не путать с аватарками).
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
    // Путь: /<kind>/<hex>. kind = artist|cover, hex — hex от key_hash.
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

/// Асинхронный обработчик протокола `lfmusic-external`. Чтение файла — в
/// отдельном потоке, чтобы не блокировать вебвью.
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

// ──────────────────────────────────────────────────────────────────────────────
// Скачивание + валидация + пережатие изображений
// ──────────────────────────────────────────────────────────────────────────────

/// Итог скачивания изображения.
enum ImageDownload {
    /// Успешно скачано, провалидировано и пережато в WebP (файл записан).
    Saved,
    /// Валидный ответ, но это дефолтный плейсхолдер источника → кэшируем «нет».
    Placeholder,
    /// Сетевая/форматная ошибка → НЕ кэшируем «нет», повторим позже.
    Error(String),
}

/// Скачивает изображение и валидирует его: HTTP-статус (`error_for_status`),
/// `Content-Type: image/*`, лимит размера, декодирование (отсев HTML-ошибок и
/// «битых» файлов) — затем пережимает в WebP с ограничением стороны `max_dim`.
/// `min_real_bytes` — порог «плейсхолдера» по размеру оригинала (для аватарок;
/// для обложек `None`, т.к. бывают легитимные мелкие арты).
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
    // HTTP-ошибка (4xx/5xx) — не сохраняем её тело как «обложку».
    let resp = match resp.error_for_status() {
        Ok(r) => r,
        Err(e) => return ImageDownload::Error(e.to_string()),
    };
    // Content-Type должен быть image/* (иначе это HTML-ошибка/редирект-страница).
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
    // Подозрительно маленький оригинал — дефолтный силуэт источника.
    if let Some(min) = min_real_bytes {
        if bytes.len() < min {
            return ImageDownload::Placeholder;
        }
    }
    // Декодируем — заодно отсекаем не-картинки и узнаём размеры.
    let img = match image::load_from_memory(&bytes) {
        Ok(i) => i,
        Err(_) => return ImageDownload::Error("не удалось декодировать изображение".into()),
    };
    if img.width() < 32 || img.height() < 32 {
        return ImageDownload::Placeholder;
    }
    // Пережатие: downscale до max_dim + WebP (как миниатюры обложек треков).
    let thumb = img.thumbnail(max_dim, max_dim);
    let mut out = Vec::new();
    if thumb
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::WebP)
        .is_err()
    {
        return ImageDownload::Error("не удалось закодировать WebP".into());
    }
    // Атомарная запись (temp + rename), чтобы протокол не прочитал недописанный.
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

/// Результат запроса аватарки/обложки для фронтенда. Вместо base64 — короткий
/// URL кастомного протокола `lfmusic-external://…` (или `None`, если нет).
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

/// Версия кэша аватарок. Повышение сбрасывает дисковый кэш один раз — например,
/// при смене формата хранения (теперь WebP + отдача через кастомный протокол).
const ARTIST_IMAGE_CACHE_VERSION: u32 = 5;

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

/// Пометить ключ как «нет внешней картинки» (негативный кэш) и сохранить.
fn cache_none_artist(
    app_data_dir: &PathBuf,
    manifest: &mut HashMap<String, ArtistImageRecord>,
    key: String,
) {
    manifest.insert(
        key,
        ArtistImageRecord {
            file: None,
            source: None,
            status: "none".into(),
            fetched_secs: now_secs(),
        },
    );
    let _ = save_artists_manifest(app_data_dir, manifest);
}

/// Получить аватарку исполнителя: сначала из кэша, иначе из сети (если включено
/// настройкой — проверяется на стороне команды). `refresh` форсирует повторную
/// загрузку из сети. Возвращает URL кастомного протокола (не base64).
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
    let mut manifest = load_artists_manifest(app_data_dir);
    if !refresh {
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
                // Негативный кэш: «не найдено» больше не перепрашиваем. Сюда
                // попадают только реальные «нет» — временные ошибки не кэшируются.
                return none_result();
            }
        }
    }

    // Идём в сеть.
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
                    manifest.insert(
                        key.clone(),
                        ArtistImageRecord {
                            file: Some(filename),
                            source: Some(img.source.clone()),
                            status: "ok".into(),
                            fetched_secs: now,
                        },
                    );
                    let _ = save_artists_manifest(app_data_dir, &manifest);
                    ArtistImageResult {
                        status: "ok".into(),
                        source: Some(img.source),
                        url: Some(artist_image_url(&key, now)),
                    }
                }
                // Скачали дефолтный плейсхолдер — это легитимное «нет реального фото».
                ImageDownload::Placeholder => {
                    cache_none_artist(app_data_dir, &mut manifest, key);
                    none_result()
                }
                // Сетевая/форматная ошибка — НЕ кэшируем, попробуем позже.
                ImageDownload::Error(e) => {
                    eprintln!("[external] download artist image failed: {}", e);
                    none_result()
                }
            }
        }
        // Все провайдеры ответили «нет» — кэшируем негативный результат.
        FetchOutcome::NotFound => {
            cache_none_artist(app_data_dir, &mut manifest, key);
            none_result()
        }
        // Был сбой провайдера — НЕ кэшируем «нет», повторим при следующем заходе.
        FetchOutcome::TemporaryError => none_result(),
    }
}

/// Загрузить все уже закэшированные аватарки (для быстрой отрисовки на старте).
/// Возвращает протокол-URL (картинки тянутся вебвью лениво, без base64 в IPC).
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

/// Версия кэша обложек. Повышение сбрасывает дисковый кэш один раз (смена формата
/// хранения на WebP + отдача через кастомный протокол).
const COVERS_CACHE_VERSION: u32 = 1;

/// Сбрасывает кэш обложек, если его версия устарела (выполняется один раз).
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

/// Пометить ключ обложки как «нет внешней» (негативный кэш) и сохранить.
fn cache_none_cover(
    app_data_dir: &PathBuf,
    manifest: &mut HashMap<String, ArtistImageRecord>,
    key: String,
) {
    manifest.insert(
        key,
        ArtistImageRecord {
            file: None,
            source: None,
            status: "none".into(),
            fetched_secs: now_secs(),
        },
    );
    let _ = save_covers_manifest(app_data_dir, manifest);
}

/// Получить обложку трека/альбома: сначала из кэша, иначе из сети.
/// Возвращает URL кастомного протокола (не base64).
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
    let mut manifest = load_covers_manifest(app_data_dir);
    if !refresh {
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
                // Негативный кэш (только реальные «нет», без временных ошибок).
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
                    manifest.insert(
                        key.clone(),
                        ArtistImageRecord {
                            file: Some(filename),
                            source: Some(img.source.clone()),
                            status: "ok".into(),
                            fetched_secs: now,
                        },
                    );
                    let _ = save_covers_manifest(app_data_dir, &manifest);
                    ArtistImageResult {
                        status: "ok".into(),
                        source: Some(img.source),
                        url: Some(cover_image_url(&key, now)),
                    }
                }
                ImageDownload::Placeholder => {
                    cache_none_cover(app_data_dir, &mut manifest, key);
                    none_result()
                }
                ImageDownload::Error(e) => {
                    eprintln!("[external] download cover failed: {}", e);
                    none_result()
                }
            }
        }
        FetchOutcome::NotFound => {
            cache_none_cover(app_data_dir, &mut manifest, key);
            none_result()
        }
        FetchOutcome::TemporaryError => none_result(),
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
    ensure_covers_cache_version(app_data_dir);
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
    ensure_covers_cache_version(app_data_dir);
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
