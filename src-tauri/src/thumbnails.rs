//! WebP thumbnail cache + the custom `lfmusic-cover` URI protocol.
//!
//! Instead of shipping full-size covers as base64 data-URLs over IPC, the
//! frontend gets a short URL `…/lfmusic-cover/<base64url(file_path)>?s=<sig>&size=<n>`
//! for `<img :src>`. The protocol handler lazily generates the thumbnail
//! (downscale -> WebP lossless) and caches it to disk; repeat requests are
//! served instantly from cache. Avoids base64 CPU cost, keeps RAM down, and
//! avoids re-reading multi-MB source images on every render.

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

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

/// Override-cover signature from a content hash; changes on cover change,
/// invalidating both the webview and on-disk cache.
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

/// Builds the cover URL for `<img :src>` (no I/O). `file_path` is base64url
/// encoded to survive Windows path chars (`\`, `:`, spaces).
pub fn cover_protocol_url(file_path: &str, sig: &str) -> String {
    let key = URL_SAFE_NO_PAD.encode(file_path.as_bytes());
    format!("{}/{}?s={}&size={}", scheme_origin(), key, sig, THUMB_SIZE)
}

/// Cover URL preferring override over embedded, or `None` if there's no cover
/// at all (frontend falls back to a default). Doesn't decode any image.
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

/// Ensures the track's thumbnail is generated and cached; returns its path.
/// Source is the override cover (DB) if present, else the embedded cover.
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

    // Signature prefix picks the source: `e…` forces the embedded tag cover
    // (used to preview the "original" even with an override set); anything
    // else prefers the override, falling back to embedded.
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

    // Atomic write via temp + rename so concurrent requests never read a
    // partially-written file.
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

pub fn thumbnail_bytes<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
    size: u32,
    sig: &str,
) -> Option<Vec<u8>> {
    let path = ensure_thumbnail(app, file_path, size, sig)?;
    fs::read(path).ok()
}

/// Cached thumbnail path for native integrations (MPRIS); signature computed
/// internally like `cover_url`.
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

/// Cached thumbnail bytes for the taskbar integration. See `cover_file`.
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
    // Path segment is base64url(file_path).
    let path_seg = uri.path().trim_start_matches('/');
    let Some(file_path) = URL_SAFE_NO_PAD
        .decode(path_seg)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
    else {
        return not_found();
    };

    // Query params: size (512|blur|<n>), s (signature).
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

/// Async `lfmusic-cover` protocol handler; generation runs on its own thread
/// so it doesn't block the webview.
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

        // Missing file must not panic, falls back to "e0".
        let missing_sig = embedded_sig(dir.join("missing.flac").to_str().unwrap());
        assert_eq!(missing_sig, "e0");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cover_protocol_url_encodes_path_and_includes_signature_and_size() {
        let url = cover_protocol_url("C:\\Music\\Artist\\Track.flac", "o1234567890abcdef");
        assert!(url.contains("s=o1234567890abcdef"));
        assert!(url.contains(&format!("size={}", THUMB_SIZE)));
        // Path segment must be pure base64url — raw Windows path chars would
        // break webview URL parsing.
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
        // "hello" base64-encoded.
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
    fn cache_dir_is_created_and_idempotent() {
        let dir = temp_dir("cache_dir");
        let cache1 = cache_dir(&dir);
        assert!(cache1.exists());
        let cache2 = cache_dir(&dir);
        assert_eq!(cache1, cache2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
