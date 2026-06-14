//! Кэш уменьшенных WebP-миниатюр обложек + кастомный URI-протокол `lfmusic-cover`.
//!
//! Вместо передачи полноразмерных обложек как base64 data-URL через IPC, фронт
//! получает короткий URL вида `…/lfmusic-cover/<base64url(file_path)>?s=<sig>&size=<n>`
//! и подставляет его в `<img :src>`. Обработчик протокола **лениво** генерирует
//! миниатюру (downscale → WebP lossless) и кэширует её на диск; повторные запросы
//! отдаются мгновенно из кэша. Это разгружает CPU (нет base64), снижает RAM
//! (короткие строки вместо мегабайтных, декод 512px вместо полноразмерного)
//! и убирает чтение исходных картинок 2–5 МБ при каждом показе.

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use tauri::http::{header, Response, StatusCode};
use tauri::{AppHandle, Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::audio_player::AudioPlayer;
use crate::library::MusicLibrary;

/// Размер обычной миниатюры (вписывается в квадрат, пропорции сохраняются).
const THUMB_SIZE: u32 = 512;
/// Размер миниатюры для размытого фона полноэкранного плеера.
const BLUR_SIZE: u32 = 96;

const SCHEME: &str = "lfmusic-cover";

/// Папка кэша миниатюр (создаётся при первом обращении).
pub fn cache_dir(app_data_dir: &Path) -> PathBuf {
    let dir = app_data_dir.join("thumb_cache");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn hash_str(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Сигнатура встроенной обложки — по mtime файла (меняется при изменении файла).
pub fn embedded_sig(file_path: &str) -> String {
    let mtime = fs::metadata(file_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("e{}", mtime)
}

/// Сигнатура пользовательской (override) обложки — по хешу её содержимого.
/// Меняется при смене обложки → инвалидирует кэш вебвью и файл кэша.
pub fn override_sig(cover_data: &str) -> String {
    format!("o{:016x}", hash_str(cover_data))
}

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

/// Чистый билдер URL обложки для `<img :src>` (без I/O).
/// `file_path` кодируется base64url — безопасно для путей Windows (`\`, `:`, пробелы).
pub fn cover_protocol_url(file_path: &str, sig: &str) -> String {
    let key = URL_SAFE_NO_PAD.encode(file_path.as_bytes());
    format!("{}/{}?s={}&size={}", scheme_origin(), key, sig, THUMB_SIZE)
}

/// URL обложки трека с учётом override/встроенной обложки, либо `None` если
/// обложки нет вовсе (фронт подставит дефолтную). Картинки не декодирует.
pub fn cover_url(lib: &MusicLibrary, file_path: &str) -> Option<String> {
    if let Some(sig) = lib.get_override_cover_sig(file_path) {
        return Some(cover_protocol_url(file_path, &sig));
    }
    if AudioPlayer::has_embedded_cover(file_path) {
        return Some(cover_protocol_url(file_path, &embedded_sig(file_path)));
    }
    None
}

pub(crate) fn decode_data_url(data_url: &str) -> Option<Vec<u8>> {
    let b64 = data_url.split(',').nth(1)?;
    STANDARD.decode(b64).ok()
}

/// Декод исходных байтов → downscale → WebP lossless.
fn encode_thumbnail(source: &[u8], size: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(source).ok()?;
    let thumb = img.thumbnail(size, size);
    let mut out = Vec::new();
    thumb
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::WebP)
        .ok()?;
    Some(out)
}

/// Оставить в сигнатуре только безопасные для имени файла символы.
fn sanitize(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Убедиться, что миниатюра трека сгенерирована и закэширована; вернуть её путь.
/// Источник: override-обложка (из БД) либо встроенная обложка файла.
pub fn ensure_thumbnail<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
    size: u32,
    sig: &str,
) -> Option<PathBuf> {
    let app_data_dir = app.path().app_data_dir().ok()?;
    let dir = cache_dir(&app_data_dir);
    let cache_file = dir.join(format!(
        "{:016x}_{}_{}.webp",
        hash_str(file_path),
        size,
        sanitize(sig)
    ));
    if cache_file.exists() {
        return Some(cache_file);
    }

    // Источник выбираем по префиксу сигнатуры: `e…` — встроенная обложка из
    // тегов (в т.ч. превью «оригинал» в обход override), `o…`/прочее —
    // пользовательская (приоритет), иначе встроенная.
    let source = if sig.starts_with('e') {
        AudioPlayer::extract_cover_bytes(file_path)?
    } else {
        let override_bytes = {
            let lib_state = app.state::<crate::LibraryState>();
            let guard = lib_state.0.lock().ok()?;
            guard.override_cover_bytes(file_path)
        };
        match override_bytes {
            Some(bytes) => bytes,
            None => AudioPlayer::extract_cover_bytes(file_path)?,
        }
    };

    let webp = encode_thumbnail(&source, size)?;

    // Атомарная запись: temp + rename, чтобы параллельные запросы не читали
    // недописанный файл.
    let tmp = dir.join(format!(
        ".tmp_{:016x}_{}_{}.webp",
        hash_str(file_path),
        size,
        std::process::id()
    ));
    if fs::write(&tmp, &webp).is_ok() {
        let _ = fs::rename(&tmp, &cache_file);
    }
    Some(cache_file)
}

/// Прочитать байты миниатюры (генерируя при необходимости).
pub fn thumbnail_bytes<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
    size: u32,
    sig: &str,
) -> Option<Vec<u8>> {
    let path = ensure_thumbnail(app, file_path, size, sig)?;
    fs::read(path).ok()
}

/// Путь к кэшированной миниатюре обложки трека (генерируя при необходимости),
/// с учётом override/встроенной обложки — для нативных интеграций (MPRIS).
/// Сигнатура вычисляется внутри (как в `cover_url`).
pub fn cover_file<R: Runtime>(app: &AppHandle<R>, file_path: &str, size: u32) -> Option<PathBuf> {
    let override_sig = {
        let lib_state = app.state::<crate::LibraryState>();
        let guard = lib_state.0.lock().ok()?;
        guard.get_override_cover_sig(file_path)
    };
    let sig = match override_sig {
        Some(sig) => sig,
        None => {
            if !AudioPlayer::has_embedded_cover(file_path) {
                return None;
            }
            embedded_sig(file_path)
        }
    };
    ensure_thumbnail(app, file_path, size, &sig)
}

/// Байты кэшированной миниатюры обложки трека (для taskbar). См. `cover_file`.
pub fn cover_bytes<R: Runtime>(app: &AppHandle<R>, file_path: &str, size: u32) -> Option<Vec<u8>> {
    let path = cover_file(app, file_path, size)?;
    fs::read(path).ok()
}

fn not_found() -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(Vec::new())
        .unwrap()
}

fn build_response<R: Runtime>(app: &AppHandle<R>, uri: &tauri::http::Uri) -> Response<Vec<u8>> {
    // Путь = base64url(file_path)
    let path_seg = uri.path().trim_start_matches('/');
    let Some(file_path) = URL_SAFE_NO_PAD
        .decode(path_seg)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
    else {
        return not_found();
    };

    // Параметры запроса: size (512|blur|<n>), s (сигнатура).
    let mut size = THUMB_SIZE;
    let mut sig = String::new();
    if let Some(query) = uri.query() {
        for pair in query.split('&') {
            let mut it = pair.splitn(2, '=');
            match (it.next(), it.next()) {
                (Some("size"), Some(v)) => {
                    size = match v {
                        "blur" => BLUR_SIZE,
                        other => other.parse().unwrap_or(THUMB_SIZE),
                    };
                }
                (Some("s"), Some(v)) => sig = v.to_string(),
                _ => {}
            }
        }
    }

    match thumbnail_bytes(app, &file_path, size, &sig) {
        Some(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/webp")
            .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(bytes)
            .unwrap(),
        None => not_found(),
    }
}

/// Асинхронный обработчик протокола `lfmusic-cover`. Генерация (декод + resize +
/// кодирование WebP) выполняется в отдельном потоке, чтобы не блокировать вебвью.
pub fn protocol_handler<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let uri = request.uri().clone();
    std::thread::spawn(move || {
        responder.respond(build_response(&app, &uri));
    });
}
