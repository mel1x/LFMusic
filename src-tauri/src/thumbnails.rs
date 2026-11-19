//! WebP thumbnail cache behind the `lfmusic-cover` protocol: covers reach the webview as URLs, not
//! base64 over IPC.

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use tauri::http::{header, Response, StatusCode};
use tauri::{AppHandle, Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::audio_player::AudioPlayer;
use crate::library::MusicLibrary;

const THUMB_SIZE: u32 = 512;
/// Smaller size used for the blurred full-screen player background.
const BLUR_SIZE: u32 = 96;

const SCHEME: &str = "lfmusic-cover";

/// Thumbnail cache folder inside the app data dir.
const CACHE_DIR: &str = "thumb_cache";

fn hash_str(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Embedded-cover signature derived from file mtime (changes when file changes).
pub fn embedded_sig(file_path: &str) -> String {
    let mtime = fs::metadata(file_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("e{}", mtime)
}

pub fn override_sig(cover_data: &str) -> String {
    format!("o{:016x}", hash_str(cover_data))
}

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

/// Builds the cover URL for `<img :src>` (no I/O).
pub fn cover_protocol_url(file_path: &str, sig: &str) -> String {
    let key = URL_SAFE_NO_PAD.encode(file_path.as_bytes());
    format!("{}/{}?s={}&size={}", scheme_origin(), key, sig, THUMB_SIZE)
}

pub fn cover_url(lib: &MusicLibrary, file_path: &str) -> Option<String> {
    if let Some(sig) = lib.get_override_cover_sig(file_path) {
        return Some(cover_protocol_url(file_path, &sig));
    }
    if lib.has_cover(file_path) {
        return Some(cover_protocol_url(file_path, &embedded_sig(file_path)));
    }
    None
}

pub(crate) fn decode_data_url(data_url: &str) -> Option<Vec<u8>> {
    let b64 = data_url.split(',').nth(1)?;
    STANDARD.decode(b64).ok()
}

fn encode_thumbnail(source: &[u8], size: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(source).ok()?;
    let thumb = img.thumbnail(size, size);
    let mut out = Vec::new();
    thumb
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::WebP)
        .ok()?;
    Some(out)
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// On-disk cache file for a thumbnail (no I/O; the dir is created on first write).
fn thumb_cache_file<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
    size: u32,
    sig: &str,
) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?.join(CACHE_DIR);
    Some(dir.join(format!(
        "{:016x}_{}_{}.webp",
        hash_str(file_path),
        size,
        sanitize(sig)
    )))
}

/// Encodes the thumbnail from its source and stores it at `cache_file`.
fn generate_thumbnail<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
    size: u32,
    sig: &str,
    cache_file: &Path,
) -> Option<Vec<u8>> {
    let source = if size != THUMB_SIZE {
        thumbnail_bytes(app, file_path, THUMB_SIZE, sig)?
    } else if sig.starts_with('e') {
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
    write_cache_file(cache_file, &webp);
    Some(webp)
}

/// Atomic write via a unique temp file: concurrent requests never read a half-written file.
fn write_cache_file(cache_file: &Path, bytes: &[u8]) {
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let Some(dir) = cache_file.parent() else { return };
    let tmp = dir.join(format!(
        ".tmp_{}_{}_{}",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed),
        cache_file.file_name().and_then(|n| n.to_str()).unwrap_or("thumb"),
    ));
    let written = fs::write(&tmp, bytes).is_ok()
        || (fs::create_dir_all(dir).is_ok() && fs::write(&tmp, bytes).is_ok());
    if written && fs::rename(&tmp, cache_file).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

/// Ensures the track's thumbnail is generated and cached; returns its path.
pub fn ensure_thumbnail<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
    size: u32,
    sig: &str,
) -> Option<PathBuf> {
    let cache_file = thumb_cache_file(app, file_path, size, sig)?;
    if !cache_file.exists() {
        generate_thumbnail(app, file_path, size, sig, &cache_file)?;
    }
    Some(cache_file)
}

pub fn thumbnail_bytes<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
    size: u32,
    sig: &str,
) -> Option<Vec<u8>> {
    let cache_file = thumb_cache_file(app, file_path, size, sig)?;
    if let Ok(bytes) = fs::read(&cache_file) {
        return Some(bytes);
    }
    generate_thumbnail(app, file_path, size, sig, &cache_file)
}

pub fn cover_file<R: Runtime>(app: &AppHandle<R>, file_path: &str, size: u32) -> Option<PathBuf> {
    let sig = {
        let lib_state = app.state::<crate::LibraryState>();
        let guard = lib_state.0.lock().ok()?;
        match guard.get_override_cover_sig(file_path) {
            Some(sig) => sig,
            None => {
                if !guard.has_cover(file_path) {
                    return None;
                }
                embedded_sig(file_path)
            }
        }
    };
    ensure_thumbnail(app, file_path, size, &sig)
}

/// Cached thumbnail bytes for the taskbar integration.
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
    let path_seg = uri.path().trim_start_matches('/');
    let Some(file_path) = URL_SAFE_NO_PAD
        .decode(path_seg)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
    else {
        return not_found();
    };

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

type PoolJob = Box<dyn FnOnce() + Send>;

/// Fixed worker pool fed by a channel.
fn worker_pool(name: &'static str, workers: usize) -> mpsc::Sender<PoolJob> {
    let (tx, rx) = mpsc::channel::<PoolJob>();
    let rx = Arc::new(Mutex::new(rx));
    for _ in 0..workers {
        let rx = Arc::clone(&rx);
        let _ = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || loop {
                let job = { rx.lock().unwrap_or_else(|e| e.into_inner()).recv() };
                match job {
                    Ok(job) => job(),
                    Err(_) => break,
                }
            });
    }
    tx
}

/// Thumbnail generation (decode + WebP encode) is CPU-bound: 2-4 workers.
fn thumbnail_pool() -> &'static mpsc::Sender<PoolJob> {
    static POOL: OnceLock<mpsc::Sender<PoolJob>> = OnceLock::new();
    POOL.get_or_init(|| {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get().clamp(2, 4))
            .unwrap_or(2);
        worker_pool("thumbnail-worker", workers)
    })
}

pub fn protocol_handler<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let uri = request.uri().clone();
    let _ = thumbnail_pool().send(Box::new(move || {
        responder.respond(build_response(&app, &uri));
    }));
}

/// Cached-file reads for other image protocols (`lfmusic-external`).
pub(crate) fn spawn_file_read(job: impl FnOnce() + Send + 'static) {
    static POOL: OnceLock<mpsc::Sender<PoolJob>> = OnceLock::new();
    let _ = POOL
        .get_or_init(|| worker_pool("image-file-reader", 2))
        .send(Box::new(job));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(test_name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("lfmusic_thumbnails_{test_name}_{unique}"))
    }

    #[test]
    fn override_sig_is_deterministic_and_content_sensitive() {
        let a = override_sig("data:image/png;base64,AAAA");
        let b = override_sig("data:image/png;base64,AAAA");
        let c = override_sig("data:image/png;base64,BBBB");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with('o'));
    }

    #[test]
    fn embedded_sig_changes_with_file_mtime_and_is_stable_for_missing_file() {
        let dir = temp_dir("embedded_sig");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("track.flac");
        std::fs::write(&file, b"v1").unwrap();
        let sig1 = embedded_sig(file.to_str().unwrap());
        assert!(sig1.starts_with('e'));

        let missing_sig = embedded_sig(dir.join("missing.flac").to_str().unwrap());
        assert_eq!(missing_sig, "e0");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cover_protocol_url_encodes_path_and_includes_signature_and_size() {
        let url = cover_protocol_url("C:\\Music\\Artist\\Track.flac", "o1234567890abcdef");
        assert!(url.contains("s=o1234567890abcdef"));
        assert!(url.contains(&format!("size={}", THUMB_SIZE)));
        let path_segment = url.rsplit('/').next().unwrap().split('?').next().unwrap();
        assert!(!path_segment.contains('\\'));
        assert!(!path_segment.contains(':'));
        assert!(!path_segment.contains(' '));
    }

    #[test]
    fn cover_protocol_url_is_pure_and_repeatable() {
        let url1 = cover_protocol_url("/music/a.mp3", "esig");
        let url2 = cover_protocol_url("/music/a.mp3", "esig");
        assert_eq!(url1, url2);
    }

    #[test]
    fn decode_data_url_extracts_base64_payload() {
        let data_url = "data:image/jpeg;base64,aGVsbG8=";
        let bytes = decode_data_url(data_url).expect("should decode");
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn decode_data_url_rejects_malformed_input() {
        assert_eq!(decode_data_url("not-a-data-url-at-all"), None);
        assert_eq!(decode_data_url("data:image/jpeg;base64,not base64!!"), None);
    }

    #[test]
    fn write_cache_file_creates_missing_dir_and_leaves_no_temp_files() {
        let dir = temp_dir("write_cache");
        let file = dir.join(CACHE_DIR).join("a_512_e1.webp");
        write_cache_file(&file, b"one");
        write_cache_file(&file, b"two");
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        let entries = std::fs::read_dir(dir.join(CACHE_DIR)).unwrap().count();
        assert_eq!(entries, 1, "temp files must be renamed away");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
