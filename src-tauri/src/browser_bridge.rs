//! Local HTTP bridge for the companion browser extension (`browser-extension/`
//! at the repo root, embedded via `include_str!` and staged to disk on demand).
//!
//! The extension reads YouTube/Google cookies through the browser's own
//! privileged `chrome.cookies` API and POSTs them here, so LFMusic can build
//! a `cookies.txt` for yt-dlp without an in-app login window or direct access
//! to the browser's cookie database (both tried before — see the comment
//! above `downloads::cookies_file`). No new HTTP-server crate: the protocol
//! surface is one GET and one POST, hand-rolled over `std::net::TcpListener`
//! to match this codebase's existing sync/thread style (worker loops,
//! subprocess calls) rather than pulling in an async web framework for it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

use serde::Deserialize;
use tauri::{AppHandle, Manager};

use crate::downloads;

/// Localhost-only, fixed port the extension's popup is hardcoded to hit.
const BRIDGE_PORT: u16 = 51823;

const MANIFEST_JSON: &str = include_str!("../browser-extension/manifest.json");
const POPUP_HTML: &str = include_str!("../browser-extension/popup.html");
const POPUP_JS: &str = include_str!("../browser-extension/popup.js");

/// Starts the bridge server on a background thread for the app's lifetime.
/// Binding failure (e.g. the port is already taken by a previous instance
/// that didn't exit cleanly) is logged and non-fatal — downloads just proceed
/// without cookies, same as if none had ever been imported.
pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("browser-bridge".into())
        .spawn(move || run_server(app))
        .ok();
}

fn run_server(app: AppHandle) {
    let listener = match TcpListener::bind(("127.0.0.1", BRIDGE_PORT)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[browser-bridge] failed to bind 127.0.0.1:{}: {}",
                BRIDGE_PORT, e
            );
            return;
        }
    };
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let app = app.clone();
        // One popup click = one request; a thread per connection keeps a
        // slow/broken client from blocking the next one.
        std::thread::spawn(move || {
            let _ = handle_connection(stream, &app);
        });
    }
}

struct ParsedRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

/// Minimal HTTP/1.1 request-line + headers + fixed-length body parser —
/// no chunked transfer, no keep-alive/pipelining, since the extension's
/// `fetch()` never needs either for a single small JSON POST.
fn read_request(stream: &TcpStream) -> Option<ParsedRequest> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).ok()?;
    }

    Some(ParsedRequest { method, path, body })
}

/// CORS headers are sent unconditionally (matching the reference muxics
/// implementation this was ported from): some Chromium versions still send a
/// preflight `OPTIONS` for a JSON POST even from an extension page that
/// already has host permission for `http://127.0.0.1/*`.
fn write_response(mut stream: &TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {len}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type\r\n\
         Connection: close\r\n\r\n\
         {body}",
        status = status,
        len = body.len(),
        body = body,
    );
    let _ = stream.write_all(response.as_bytes());
}

#[derive(Deserialize)]
struct ImportSessionRequest {
    cookies: Vec<downloads::BridgeCookie>,
}

fn handle_connection(stream: TcpStream, app: &AppHandle) -> Option<()> {
    let request = read_request(&stream)?;

    if request.method.eq_ignore_ascii_case("OPTIONS") {
        write_response(&stream, "204 No Content", "");
        return Some(());
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/bridge/ping") => {
            write_response(&stream, "200 OK", r#"{"success":true,"app":"LFMusic"}"#);
        }
        ("POST", "/bridge/import-session") => {
            match serde_json::from_slice::<ImportSessionRequest>(&request.body) {
                Ok(payload) => match downloads::import_bridge_cookies(app, payload.cookies) {
                    Ok(count) => {
                        let body = serde_json::json!({"success": true, "count": count}).to_string();
                        write_response(&stream, "200 OK", &body);
                    }
                    Err(error) => {
                        let body = serde_json::json!({"success": false, "error": error}).to_string();
                        write_response(&stream, "400 Bad Request", &body);
                    }
                },
                Err(e) => {
                    let body = serde_json::json!({
                        "success": false,
                        "error": format!("Bad request body: {}", e),
                    })
                    .to_string();
                    write_response(&stream, "400 Bad Request", &body);
                }
            }
        }
        _ => {
            write_response(
                &stream,
                "404 Not Found",
                r#"{"success":false,"error":"Not found"}"#,
            );
        }
    }
    Some(())
}

fn extension_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("browser-extension")
}

/// Stages the bundled extension source into `<app_data_dir>/browser-extension/`
/// so the user can load it unpacked from a real browser's extensions page.
/// Overwrites on every call so an app update always refreshes a stale copy.
pub fn prepare_extension_bundle(app: &AppHandle) -> Result<String, String> {
    let dir = extension_dir(app);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("manifest.json"), MANIFEST_JSON).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("popup.html"), POPUP_HTML).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("popup.js"), POPUP_JS).map_err(|e| e.to_string())?;
    Ok(dir.to_string_lossy().to_string())
}
