//! Download queue manager (the "Download" page).
//!
//! One background worker sequentially downloads tracks via **yt-dlp** into
//! "LFMusic Downloads", tags them with `lofty`, then the folder is registered
//! as a library folder and rescanned so tracks show up in the catalog.
//!
//! Queue state streams to the frontend via the `download_queue` event
//! ([`DownloadSnapshot`]); rough progress also feeds the titlebar's shared
//! status bar as the `download` phase (via [`crate::report_download_progress`]),
//! last after analysis and external sources.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::tools;

#[derive(Debug, Clone, Deserialize)]
pub struct DownloadRequest {
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    /// Explicit yt-dlp search query; defaults to "artist title".
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub cover_url: Option<String>,
    /// Deezer album id (when known from a search result) — used to look up the
    /// genre tag, which Deezer's track search response doesn't include.
    #[serde(default)]
    pub album_id: Option<u64>,
    /// Genre, when the source already provided one directly (iTunes results —
    /// see [`crate::external::ExternalSearchTrack::genre`]); takes priority
    /// over an `album_id` lookup.
    #[serde(default)]
    pub genre: Option<String>,
    /// Force re-download even if already queued/downloaded (used by the
    /// "already in library" modal).
    #[serde(default)]
    pub force: Option<bool>,
    /// Exact yt-dlp download target (the "paste a link" flow, see
    /// [`resolve_url_metadata`]) — when set, this URL is downloaded directly
    /// instead of a `ytsearch`/YouTube Music search built from title/artist.
    #[serde(default)]
    pub direct_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadItem {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub query: String,
    pub cover_url: Option<String>,
    pub album_id: Option<u64>,
    pub genre: Option<String>,
    pub direct_url: Option<String>,
    /// "queued" | "downloading" | "done" | "error".
    pub status: String,
    pub progress: f64,
    pub error: Option<String>,
    pub error_kind: Option<DownloadErrorKind>,
    pub file_path: Option<String>,
}

/// Error classification for the UI (distinct hints/icons), not text parsing.
/// A single "YouTube requires login" marker used to conflate the IP/VPN
/// bot-wall with actual cookie expiry; now the cause is carried as a type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadErrorKind {
    /// YouTube bot-wall — almost always IP/network reputation (VPN/datacenter),
    /// not cookies. Fixed by network/proxy change or retry (login is one workaround).
    BotWallNetwork,
    /// yt-dlp explicitly rejected the cookies themselves (expired/rotated) — needs re-login.
    CookiesExpired,
    /// Network timeout/drop/resolve failure — transient.
    NetworkTimeout,
    /// No suitable audio format found (missing ffmpeg / no matching format).
    NoFormat,
    /// Required tool missing/won't launch (yt-dlp).
    ToolMissing,
    Cancelled,
    /// Unrecognized yt-dlp error or library import failure.
    Unknown,
}

#[derive(Debug, Clone)]
pub struct DownloadError {
    pub kind: DownloadErrorKind,
    pub message: String,
}

impl DownloadError {
    fn new(kind: DownloadErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolsStatus {
    pub ytdlp: bool,
    pub ffmpeg: bool,
    /// Deno JS engine available (installed/removed alongside yt-dlp).
    pub deno: bool,
    /// PO-token provider for YouTube's bot-wall — optional, installed separately.
    pub pot: bool,
    pub preparing: bool,
    pub message: Option<String>,
    pub cookies_authorized: bool,
    /// Cookies passed validation — downloading is unlocked.
    pub cookies_valid: bool,
    pub cookies_checking: bool,
    pub cookies_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadSnapshot {
    pub items: Vec<DownloadItem>,
    pub tools: ToolsStatus,
}

/// Queue manager, behind a global mutex ([`DownloadsState`]).
pub struct DownloadManager {
    items: Vec<DownloadItem>,
    next_id: u64,
    worker_running: bool,
    /// Counters for the current "run", for the titlebar progress phase.
    run_total: usize,
    run_done: usize,
    tools_preparing: bool,
    tools_message: Option<String>,
    ytdlp_ready: bool,
    ffmpeg_ready: bool,
    deno_ready: bool,
    pot_ready: bool,
    cookies_authorized: bool,
    cookies_valid: bool,
    cookies_checking: bool,
    cookies_message: Option<String>,
    /// Whether cookie state was loaded from disk yet this run; until then the
    /// cookies_* fields are zeroed and don't reflect a saved login.
    cookies_seeded: bool,
    /// Active downloads' child processes, keyed by job id (for cancellation).
    /// Multiple entries when [`DOWNLOAD_CONCURRENCY`] > 1 workers are each
    /// mid-download at once.
    active_children: HashMap<u64, Arc<Mutex<Option<Child>>>>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            next_id: 1,
            worker_running: false,
            run_total: 0,
            run_done: 0,
            tools_preparing: false,
            tools_message: None,
            ytdlp_ready: false,
            ffmpeg_ready: false,
            deno_ready: false,
            pot_ready: false,
            cookies_authorized: false,
            cookies_valid: false,
            cookies_checking: false,
            cookies_message: None,
            cookies_seeded: false,
            active_children: HashMap::new(),
        }
    }
}

impl DownloadManager {
    fn snapshot(&self) -> DownloadSnapshot {
        DownloadSnapshot {
            items: self.items.clone(),
            tools: ToolsStatus {
                ytdlp: self.ytdlp_ready,
                ffmpeg: self.ffmpeg_ready,
                deno: self.deno_ready,
                pot: self.pot_ready,
                preparing: self.tools_preparing,
                message: self.tools_message.clone(),
                cookies_authorized: self.cookies_authorized,
                cookies_valid: self.cookies_valid,
                cookies_checking: self.cookies_checking,
                cookies_message: self.cookies_message.clone(),
            },
        }
    }

    /// Pop the next queued item into "downloading". Pure (no AppHandle/subprocess),
    /// so unit-testable directly.
    fn take_next_queued(&mut self) -> Option<DownloadItem> {
        let pos = self.items.iter().position(|i| i.status == "queued")?;
        self.items[pos].status = "downloading".into();
        self.items[pos].progress = 0.0;
        Some(self.items[pos].clone())
    }

    /// Apply a download outcome (success/import-error/error) to the item and
    /// count it toward run progress.
    fn apply_download_result(
        &mut self,
        job_id: u64,
        result: &Result<PathBuf, DownloadError>,
        import_err: Option<&str>,
    ) {
        self.run_done += 1;
        let Some(item) = self.items.iter_mut().find(|i| i.id == job_id) else {
            return;
        };
        match result {
            Ok(path) => {
                item.file_path = Some(path.to_string_lossy().to_string());
                if let Some(err) = import_err {
                    item.status = "error".into();
                    item.progress = 100.0;
                    item.error = Some(format!("Трек скачан, но не добавлен в библиотеку: {}", err));
                    item.error_kind = Some(DownloadErrorKind::Unknown);
                } else {
                    item.status = "done".into();
                    item.progress = 100.0;
                    item.error = None;
                    item.error_kind = None;
                }
            }
            Err(e) => {
                item.status = "error".into();
                item.error = Some(e.message.clone());
                item.error_kind = Some(e.kind);
            }
        }
    }

    /// Remove an item: queued just drops it (and shrinks run_total); downloading
    /// drops it and returns that job's active child process for cancellation (if
    /// its worker hasn't already taken it); done/error just drops it from
    /// history. Killing the process/emitting is left to the caller ([`remove`]).
    fn remove_item(&mut self, id: u64) -> Option<Child> {
        let pos = self.items.iter().position(|i| i.id == id)?;
        let status = self.items[pos].status.clone();
        match status.as_str() {
            "queued" => {
                self.items.remove(pos);
                self.run_total = self.run_total.saturating_sub(1);
                None
            }
            "downloading" => {
                // Worker will notice it's gone and count it toward run_done.
                self.items.remove(pos);
                self.active_children
                    .get(&id)
                    .and_then(|slot| slot.lock().unwrap_or_else(|e| e.into_inner()).take())
            }
            _ => {
                self.items.remove(pos);
                None
            }
        }
    }

    /// Requeue an errored item (error -> queued), resetting progress/error, and
    /// count it toward the current run. `true` if found and was in error status.
    fn retry_item(&mut self, id: u64) -> bool {
        let Some(item) = self.items.iter_mut().find(|i| i.id == id && i.status == "error") else {
            return false;
        };
        item.status = "queued".into();
        item.progress = 0.0;
        item.error = None;
        item.error_kind = None;
        item.file_path = None;
        self.run_total += 1;
        true
    }

    fn clear_finished_items(&mut self) {
        self.items
            .retain(|i| i.status == "queued" || i.status == "downloading");
    }

    /// End the run: reset counters/worker flag, return (done, total) as they
    /// were before reset, for the final titlebar progress push.
    fn take_run_counts(&mut self) -> (usize, usize) {
        let r = (self.run_done, self.run_total);
        self.run_done = 0;
        self.run_total = 0;
        self.worker_running = false;
        r
    }
}

pub struct DownloadsState(pub Mutex<DownloadManager>);

impl Default for DownloadsState {
    fn default() -> Self {
        Self(Mutex::new(DownloadManager::default()))
    }
}

fn with_manager<R>(app: &AppHandle, f: impl FnOnce(&mut DownloadManager) -> R) -> R {
    let state = app.state::<DownloadsState>();
    let mut guard = state.0.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

fn emit_snapshot(app: &AppHandle) {
    let snap = with_manager(app, |m| m.snapshot());
    let _ = app.emit("download_queue", snap);
}

fn report_titlebar(app: &AppHandle) {
    let (done, total) = with_manager(app, |m| (m.run_done, m.run_total));
    crate::report_download_progress(app, done, total);
}

/// Same, but with a fractional progress of the current track, so the titlebar
/// percentage moves smoothly instead of jumping 0->100 per track.
fn report_titlebar_frac(app: &AppHandle, frac: f64) {
    let (done, total) = with_manager(app, |m| (m.run_done, m.run_total));
    crate::report_download_progress_frac(app, done, total, frac);
}

pub fn snapshot(app: &AppHandle) -> DownloadSnapshot {
    ensure_cookie_seed(app);
    with_manager(app, |m| m.snapshot())
}

/// Kill all active yt-dlp child processes on app exit. Rust doesn't kill
/// running `Child`s when the parent exits, so without this yt-dlp would linger
/// as a zombie background process (one per concurrent download worker).
/// Hooked to `RunEvent::Exit` in lib.rs.
pub fn cleanup_on_exit(app: &AppHandle) {
    let children: Vec<Child> = with_manager(app, |m| {
        m.active_children
            .drain()
            .filter_map(|(_, slot)| slot.lock().unwrap_or_else(|e| e.into_inner()).take())
            .collect()
    });
    for mut c in children {
        kill_child_tree(&mut c);
    }
}

/// Kill the whole download process tree (yt-dlp + its ffmpeg/deno children).
/// A plain `child.kill()` would only kill yt-dlp, leaving children holding the
/// file/pipes. Windows: `taskkill /T` (by PID tree); Unix: signal the whole
/// process group (yt-dlp is the group leader, see `process_group(0)` on spawn).
fn kill_child_tree(child: &mut Child) {
    let pid = child.id();
    #[cfg(windows)]
    {
        let _ = tools::silent_command(Path::new("taskkill"))
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // Negative PID = whole process group.
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{}", pid))
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Whether any queue items are active (queued/downloading). Used as a guard
/// against removing tools while the worker might have them resolved and be
/// actively downloading — removal would yank the binary out from under it.
pub fn has_active_jobs(app: &AppHandle) -> bool {
    with_manager(app, |m| {
        m.items
            .iter()
            .any(|i| i.status == "queued" || i.status == "downloading")
    })
}

/// Absolute downloads folder path; frontend uses it to tell whether a track
/// was downloaded by the app (for the "Delete track" context menu item).
pub fn downloads_dir_path(app: &AppHandle) -> String {
    downloads_dir(app).to_string_lossy().to_string()
}

/// Whether `file` is inside `dir`, checked by path components rather than a
/// naive string `starts_with` (which would let a sibling folder with the same
/// prefix through, e.g. `.../LFMusic Downloads-old/x.mp3`). Canonicalizes both
/// paths (also defeats `..` and symlinks). If the file no longer exists
/// (canonicalize fails), falls back to a lexical comparison with a mandatory
/// separator boundary — enough to decide whether an orphaned DB entry can be
/// cleaned up.
fn is_within_downloads(dir: &Path, file: &Path) -> bool {
    if let (Ok(dir_c), Ok(file_c)) = (dir.canonicalize(), file.canonicalize()) {
        return file_c.starts_with(&dir_c);
    }
    // Fallback (file gone -> canonicalize fails): must resolve `.`/`..`
    // lexically without touching disk, otherwise
    // `.../LFMusic Downloads/../../Desktop/x` would pass a string-prefix check
    // and allow deleting a file outside the downloads folder (path traversal).
    let norm = |p: &Path| {
        lexical_normalize(p)
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase()
    };
    let mut d = norm(dir);
    if !d.ends_with('/') {
        d.push('/');
    }
    norm(file).starts_with(&d)
}

/// Lexically normalizes a path: collapses `.` and resolves `..` without
/// touching the filesystem (unlike `canonicalize`, which requires the path to
/// exist). Used for safe containment checks when the file is already gone.
fn lexical_normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Deletes an app-downloaded track: removes the file from disk and the
/// library. Only deletes files INSIDE the downloads folder, as a safety check.
pub fn delete_downloaded_track(app: &AppHandle, file_path: &str) -> Result<(), String> {
    let dir = downloads_dir(app);
    if !is_within_downloads(&dir, Path::new(file_path)) {
        return Err("Можно удалять только треки, скачанные приложением".to_string());
    }

    // If this track is currently playing, stop first: the decoder (symphonia)
    // holds the file handle, and deletion would hit "file in use" on Windows.
    if let Ok(mut player) = app.state::<crate::PlayerState>().0.write() {
        if player.current_file_path().as_deref() == Some(file_path) {
            player.stop();
        }
    }

    // Quarantine: rename to a temp name first (same-folder rename is atomic),
    // making deletion reversible — if the DB removal fails, the file is moved
    // back. If the rename itself fails (file busy/no perms), the DB is left
    // untouched. Retries cover the audio thread asynchronously releasing its
    // handle after stop().
    let path = PathBuf::from(file_path);
    let quarantine: Option<PathBuf> = if path.exists() {
        let q = quarantine_path(&path);
        rename_with_retries(&path, &q).map_err(|e| format!("Не удалось удалить файл: {}", e))?;
        Some(q)
    } else {
        None
    };

    // Remove from library; on DB error, restore the file from quarantine so
    // it isn't lost while the record still exists (no desync).
    let separators = crate::settings::get_artist_separators(&app.state::<crate::SettingsState>());
    let remove_result = {
        let state = app.state::<crate::LibraryState>();
        state
            .0
            .lock()
            .map_err(|e| format!("библиотека занята: {}", e))
            .and_then(|mut lib| lib.remove_track(file_path, &separators))
    };
    if let Err(e) = remove_result {
        if let Some(q) = &quarantine {
            let _ = std::fs::rename(q, &path); // rollback
        }
        return Err(e);
    }

    // DB updated — physically remove the quarantined file. Best-effort: even
    // if it fails, state stays consistent (library record is gone already),
    // and `cleanup_orphan_temp_files` sweeps leftovers on the next worker run.
    if let Some(q) = quarantine {
        let _ = remove_file_with_retries(&q);
    }
    let _ = std::fs::remove_file(path.with_extension("lrc"));
    let _ = app.emit("downloads_library_updated", ());
    Ok(())
}

/// Quarantine filename next to the original (same-folder rename is atomic).
/// The `.lfm-trash-` prefix is recognized as temp and swept on worker startup.
fn quarantine_path(path: &Path) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("track");
    path.with_file_name(format!(".lfm-trash-{}-{}", ts, name))
}

/// Rename with retries: a just-played file may briefly stay locked while the
/// audio thread releases its handle after stop() (Windows).
fn rename_with_retries(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last: Option<std::io::Error> = None;
    for _ in 0..15 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "rename failed")))
}

/// Same rationale as [`rename_with_retries`], for deletion.
fn remove_file_with_retries(path: &Path) -> std::io::Result<()> {
    let mut last: Option<std::io::Error> = None;
    for _ in 0..15 {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "remove failed")))
}

/// Queues a track and starts the worker if needed. Non-error duplicates
/// (same normalized artist+title+album) aren't re-added, guarding against
/// double-clicks/repeated invokes; returns the existing item's id instead.
pub fn enqueue(app: &AppHandle, req: DownloadRequest) -> u64 {
    let (id, is_new) = with_manager(app, |m| insert_request(m, req));
    if is_new {
        emit_snapshot(app);
        report_titlebar(app);
        ensure_worker(app);
    }
    id
}

/// Batch version of [`enqueue`] (used by "Download all tracks" on an album):
/// same dedup, but snapshot/worker are triggered once for the whole batch.
pub fn enqueue_many(app: &AppHandle, reqs: Vec<DownloadRequest>) -> Vec<u64> {
    if reqs.is_empty() {
        return Vec::new();
    }
    let (ids, any_new) = with_manager(app, |m| {
        let mut ids = Vec::with_capacity(reqs.len());
        let mut any_new = false;
        for req in reqs {
            let (id, is_new) = insert_request(m, req);
            any_new |= is_new;
            ids.push(id);
        }
        (ids, any_new)
    });
    if any_new {
        emit_snapshot(app);
        report_titlebar(app);
        ensure_worker(app);
    }
    ids
}

/// Inserts one request into the queue (side-effect free; caller triggers the
/// snapshot/worker). `is_new == false` if the track is already queued/
/// downloading/downloaded (force bypasses this).
fn insert_request(m: &mut DownloadManager, req: DownloadRequest) -> (u64, bool) {
    let query = req
        .query
        .filter(|q| !q.trim().is_empty())
        .unwrap_or_else(|| format!("{} {}", req.artist, req.title).trim().to_string());
    let key = crate::external::track_identity_key(&req.artist, &req.title, req.album.as_deref());
    let force = req.force.unwrap_or(false);

    if !force {
        if let Some(existing) = m.items.iter().find(|i| {
            i.status != "error"
                && crate::external::track_identity_key(&i.artist, &i.title, i.album.as_deref())
                    == key
        }) {
            return (existing.id, false);
        }
    }
    // Adding a new item: drop any previous ERRORED version of the same track
    // so it's replaced rather than left as a stale duplicate. run_total/
    // run_done are untouched: the errored item already counted its +1 in
    // run_done on failure, so the remainder (total-done) is unaffected.
    m.items.retain(|i| {
        !(i.status == "error"
            && crate::external::track_identity_key(&i.artist, &i.title, i.album.as_deref()) == key)
    });
    let id = m.next_id;
    m.next_id += 1;
    m.items.push(DownloadItem {
        id,
        title: req.title,
        artist: req.artist,
        album: req.album,
        query,
        cover_url: req.cover_url,
        album_id: req.album_id,
        genre: req.genre,
        direct_url: req.direct_url,
        status: "queued".into(),
        progress: 0.0,
        error: None,
        error_kind: None,
        file_path: None,
    });
    m.run_total += 1;
    (id, true)
}

/// Removes an item from the queue; an active download is cancelled (killed).
pub fn remove(app: &AppHandle, id: u64) {
    let child_to_kill = with_manager(app, |m| m.remove_item(id));

    if let Some(mut child) = child_to_kill {
        kill_child_tree(&mut child);
    }

    emit_snapshot(app);
    report_titlebar(app);
}

/// Retries an errored item: moves it back to queued, clears error/progress,
/// and starts the worker. Metadata/query are kept, so nothing needs re-entering.
pub fn retry(app: &AppHandle, id: u64) {
    let restarted = with_manager(app, |m| m.retry_item(id));
    if restarted {
        emit_snapshot(app);
        report_titlebar(app);
        ensure_worker(app);
    }
}

pub fn clear_finished(app: &AppHandle) {
    with_manager(app, DownloadManager::clear_finished_items);
    emit_snapshot(app);
}

pub fn refresh_tools_status(app: &AppHandle) -> ToolsStatus {
    let app_data_dir = app_data_dir(app);
    let (ytdlp, ffmpeg) = tools::check(&app_data_dir);
    let deno = tools::resolve_deno(&app_data_dir).is_some();
    let pot = tools::resolve_pot(&app_data_dir).is_some();
    let authorized = is_authorized(app);
    let invalid = cookies_invalid_marker(app).exists();
    with_manager(app, |m| {
        m.ytdlp_ready = ytdlp;
        m.ffmpeg_ready = ffmpeg;
        m.deno_ready = deno;
        m.pot_ready = pot;
        m.cookies_authorized = authorized;
        // Valid = cookies exist AND aren't marked stale (login remembered from disk).
        m.cookies_valid = authorized && !invalid;
        m.cookies_seeded = true;
        m.snapshot().tools
    })
}

// YouTube cookies: imported via a companion browser extension (see
// `browser_bridge`), which reads the browser's live cookie jar through the
// privileged `chrome.cookies` API and posts it to a tiny local HTTP server
// this app runs. That sidesteps two dead ends tried before:
// - An in-app WebView2 login popup: the resulting session was treated as
//   untrustworthy by YouTube's bot-detection at the player-API level
//   (`LOGIN_REQUIRED` on every client, even with a valid PO token).
// - yt-dlp's own `--cookies-from-browser` (reading the browser's cookie DB
//   file directly): requires the browser closed, and fails outright on
//   Chrome/Edge since 2024 (App-Bound Encryption blocks other processes from
//   decrypting the DB). The extension has neither problem — the browser
//   decrypts cookies for its own extension regardless of whether it's the
//   process reading its own DB file, and regardless of whether it's running.

/// Regular desktop Chrome UA: makes yt-dlp's requests look like a normal
/// browser session.
const YOUTUBE_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36";

/// Stable video for a cookie validity check (popular, unlikely to be removed).
const COOKIE_CHECK_URL: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

fn cookies_file(app: &AppHandle) -> PathBuf {
    app_data_dir(app).join("youtube_cookies.txt")
}

pub fn is_authorized(app: &AppHandle) -> bool {
    cookies_file(app).exists()
}

/// One cookie as reported by the browser extension's `chrome.cookies.getAll()`
/// — richer than a flattened `Cookie:` header string, so the Netscape file
/// below carries an accurate domain/secure/expiry per cookie rather than
/// guessing them.
/// Field names match `chrome.cookies.Cookie` (camelCase) — the extension
/// forwards its `chrome.cookies.getAll()` results essentially as-is.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    /// Seconds since epoch; absent = session cookie.
    #[serde(default)]
    pub expiration_date: Option<f64>,
}

/// YouTube auth cookie names; their presence means the browser session is signed in.
const AUTH_COOKIE_NAMES: &[&str] = &["__Secure-3PSID", "__Secure-1PSID", "LOGIN_INFO", "SID"];

fn has_bridge_auth_cookie(cookies: &[BridgeCookie]) -> bool {
    cookies.iter().any(|c| {
        let domain = c.domain.trim_start_matches('.').to_ascii_lowercase();
        domain.contains("youtube.com") && AUTH_COOKIE_NAMES.contains(&c.name.as_str())
    })
}

/// Maps a cookie's domain to the wildcard form yt-dlp expects (`.youtube.com`
/// etc., `include_sub=TRUE`) regardless of the browser-reported domain's exact
/// shape, so a cookie scoped to e.g. `accounts.google.com` still matches
/// requests to `music.youtube.com`. `None` for domains outside the three we
/// care about (everything else is dropped, not just left host-only).
fn netscape_youtube_domain_field(domain: &str, http_only: bool) -> Option<(String, &'static str)> {
    let trimmed = domain.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bare = trimmed.trim_start_matches('.').to_ascii_lowercase();
    let normalized = if bare == "youtube.com" || bare.ends_with(".youtube.com") {
        ".youtube.com"
    } else if bare == "google.com" || bare.ends_with(".google.com") {
        ".google.com"
    } else if bare == "googlevideo.com" || bare.ends_with(".googlevideo.com") {
        ".googlevideo.com"
    } else {
        return None;
    };
    let field = if http_only {
        format!("#HttpOnly_{}", normalized)
    } else {
        normalized.to_string()
    };
    Some((field, "TRUE"))
}

/// Renders cookies received from the browser-extension bridge into a
/// Netscape-format cookies file body for yt-dlp's `--cookies`. Pure (no
/// AppHandle/filesystem), so it's directly unit-testable. Returns the
/// rendered content and row count.
fn render_bridge_cookies_file(cookies: &[BridgeCookie]) -> Result<(String, usize), String> {
    let mut out = String::from("# Netscape HTTP Cookie File\n# Generated by LFMusic from the browser extension\n\n");
    let mut count = 0usize;
    for c in cookies {
        let Some((domain_field, include_sub)) = netscape_youtube_domain_field(&c.domain, c.http_only) else {
            continue;
        };
        let path = c.path.as_deref().unwrap_or("/");
        let secure = if c.secure { "TRUE" } else { "FALSE" };
        // Session cookies (no expiry date) -> 0.
        let expiry = c
            .expiration_date
            .map(|d| d as i64)
            .filter(|&t| t > 0)
            .unwrap_or(0);
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            domain_field, include_sub, path, secure, expiry, c.name, c.value
        ));
        count += 1;
    }
    if count == 0 {
        return Err("Cookies YouTube не найдены — войдите в аккаунт в браузере и отправьте сессию снова.".to_string());
    }
    Ok((out, count))
}

/// Writes cookies received from the browser-extension bridge to
/// `cookies_file`. Returns the row count.
fn write_bridge_cookies_file(app: &AppHandle, cookies: &[BridgeCookie]) -> Result<usize, String> {
    let (content, count) = render_bridge_cookies_file(cookies)?;
    write_private_atomic(&cookies_file(app), content.as_bytes())?;
    Ok(count)
}

/// Writes a file atomically and privately. Cookies carry an active session, so:
/// (1) write to a temp sibling and atomically rename, avoiding a partial file
/// on failure; (2) set 0600 perms on Unix (umask might otherwise leave it
/// readable by other users).
fn write_private_atomic(target: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = target.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    // rename atomically replaces target, but it can be briefly locked by a
    // concurrent yt-dlp reading cookies (sharing violation on Windows) —
    // retry a few times before giving up.
    let mut last: Option<std::io::Error> = None;
    for _ in 0..10 {
        match std::fs::rename(&tmp, target) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(format!(
        "не удалось сохранить cookies (файл занят?): {}",
        last.map(|e| e.to_string()).unwrap_or_default()
    ))
}

/// Called by the local browser-bridge server (see `browser_bridge`) when the
/// extension posts a captured session. Validates it actually contains an auth
/// cookie, writes the Netscape file, and marks the session valid immediately
/// (no network re-check here — mirrors the old WebView2 login's rationale:
/// avoids a "sent -> checking" flicker every time; a real failure surfaces via
/// manual "Check" or a bot-wall on download).
pub fn import_bridge_cookies(app: &AppHandle, cookies: Vec<BridgeCookie>) -> Result<usize, String> {
    if !has_bridge_auth_cookie(&cookies) {
        return Err(
            "В присланной сессии нет cookies авторизации YouTube — убедитесь, что вы вошли в аккаунт в браузере."
                .to_string(),
        );
    }
    let count = write_bridge_cookies_file(app, &cookies)?;
    clear_cookies_invalid(app);
    with_manager(app, |m| {
        m.cookies_authorized = true;
        m.cookies_valid = true;
        m.cookies_checking = false;
        m.cookies_message = None;
        m.cookies_seeded = true;
    });
    emit_snapshot(app);
    Ok(count)
}

/// Clears the imported session: removes cookies.txt and resets status.
pub fn clear_bridge_cookies(app: &AppHandle) {
    let _ = std::fs::remove_file(cookies_file(app));
    clear_cookies_invalid(app);
    with_manager(app, |m| {
        m.cookies_authorized = false;
        m.cookies_valid = false;
        m.cookies_checking = false;
        m.cookies_message = None;
        m.cookies_seeded = true;
    });
    emit_snapshot(app);
}

/// Marker file: its PRESENCE means the login is no longer valid (a check
/// failed or a download hit the bot-wall despite having cookies). Its absence
/// (with a cookies file present) means the login is considered valid — so a
/// successful login is remembered across restarts without a network re-check,
/// and only clears once cookies actually stop working.
fn cookies_invalid_marker(app: &AppHandle) -> PathBuf {
    app_data_dir(app).join("youtube_cookies_invalid")
}

fn mark_cookies_invalid(app: &AppHandle) {
    let _ = std::fs::write(cookies_invalid_marker(app), b"1");
    with_manager(app, |m| {
        m.cookies_valid = false;
        m.cookies_seeded = true;
    });
}

fn clear_cookies_invalid(app: &AppHandle) {
    let _ = std::fs::remove_file(cookies_invalid_marker(app));
}

/// A successful cookie-backed download proves the login still works, so clear
/// the "stale" marker if set. Needed because the marker used to be set by a
/// transient IP-based bot-wall and never got cleared by later successes,
/// leaving users stuck on "log in again" even after changing networks.
/// Snapshot is emitted only on an actual change.
fn mark_cookies_valid_after_success(app: &AppHandle) {
    if !cookies_invalid_marker(app).exists() {
        return;
    }
    clear_cookies_invalid(app);
    with_manager(app, |m| {
        if m.cookies_authorized {
            m.cookies_valid = true;
        }
        m.cookies_message = None;
    });
    emit_snapshot(app);
}

/// Loads the saved cookies-import state from disk once per run (fields
/// default to zero until then, not reflecting an actual imported session).
fn ensure_cookie_seed(app: &AppHandle) {
    if with_manager(app, |m| m.cookies_seeded) {
        return;
    }
    let authorized = is_authorized(app);
    let invalid = cookies_invalid_marker(app).exists();
    with_manager(app, |m| {
        if m.cookies_seeded {
            return;
        }
        m.cookies_authorized = authorized;
        m.cookies_valid = authorized && !invalid;
        m.cookies_seeded = true;
    });
}

/// Validates saved cookies via a background `--simulate` extraction of a
/// stable video. Result is cached and streamed via snapshot.
pub fn validate_youtube_cookies(app: &AppHandle) {
    if !is_authorized(app) {
        with_manager(app, |m| {
            m.cookies_authorized = false;
            m.cookies_valid = false;
            m.cookies_checking = false;
        });
        emit_snapshot(app);
        return;
    }
    with_manager(app, |m| {
        m.cookies_checking = true;
        m.cookies_message = None;
    });
    emit_snapshot(app);

    let app2 = app.clone();
    std::thread::Builder::new()
        .name("youtube-cookies-validate".into())
        .spawn(move || {
            let (valid, message) = run_youtube_cookie_check(&app2);
            // Success clears the stale marker persistently. Failure is NOT
            // written to disk: it could be a missing Deno or transient
            // network issue, not proof cookies are actually bad.
            if valid {
                clear_cookies_invalid(&app2);
            }
            with_manager(&app2, |m| {
                m.cookies_checking = false;
                m.cookies_valid = valid;
                m.cookies_seeded = true;
                m.cookies_message = message;
            });
            emit_snapshot(&app2);
        })
        .ok();
}

/// Runs yt-dlp in `--simulate` mode with the imported cookies to check
/// they're still good for downloading. Returns `(valid, error_message)`.
fn run_youtube_cookie_check(app: &AppHandle) -> (bool, Option<String>) {
    let app_data_dir = app_data_dir(app);
    let Some(ytdlp) = tools::resolve_ytdlp(&app_data_dir) else {
        return (
            false,
            Some("yt-dlp не установлен — установите его в «Внешние источники».".to_string()),
        );
    };
    let cookies = cookies_file(app);
    if !cookies.exists() {
        return (
            false,
            Some("Cookies не отправлены — используйте расширение браузера в «Внешних источниках».".to_string()),
        );
    }

    // Without the JS engine, YouTube won't serve audio formats (signature/n
    // challenge unsolved) — report this explicitly instead of a cryptic
    // "Requested format is not available".
    let Some(deno_rt) = tools::deno_js_runtime_arg(&app_data_dir) else {
        return (
            false,
            Some(
                "Не установлен JS-движок Deno — он обязателен для YouTube. \
                 Нажмите «Установить Deno» в карточке yt-dlp («Внешние источники»)."
                    .to_string(),
            ),
        );
    };

    let mut cmd = tools::silent_command(&ytdlp);
    cmd.arg("--cookies")
        .arg(&cookies)
        .arg("--simulate")
        .arg("--no-warnings")
        .arg("--playlist-items")
        .arg("1")
        .arg("-f")
        .arg("bestaudio/best")
        .arg("--js-runtimes")
        .arg(deno_rt)
        // Same client/UA as a real download (download_one_with_stem), so the
        // check and the actual download don't diverge.
        .arg("--extractor-args")
        .arg(format!("youtube:player_client={}", YT_PLAYER_CLIENTS[0]))
        .arg("--user-agent")
        .arg(YOUTUBE_UA);
    // Same proxy as a real download, for the same reason.
    if let Some(proxy) = proxy_arg(app) {
        cmd.arg("--proxy").arg(proxy);
    }
    cmd.arg(COOKIE_CHECK_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => return (false, Some(format!("Не удалось запустить yt-dlp: {}", e))),
    };

    if output.status.success() {
        return (true, None);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    (false, Some(extract_ytdlp_error(&stderr)))
}

/// Metadata probe for the "paste a link" download flow: a `--skip-download`
/// yt-dlp pass that returns title/uploader/thumbnail/duration without
/// fetching the file, so the frontend can show a normal-looking result row
/// and hand it back to [`enqueue`] (via `DownloadRequest::direct_url`) exactly
/// like a search hit. Uses the same client/cookies/proxy setup as a real
/// download so a link that probes fine won't unexpectedly bot-wall later.
pub fn resolve_url_metadata(
    app: &AppHandle,
    url: &str,
) -> Result<crate::external::ExternalSearchTrack, String> {
    let app_data_dir = app_data_dir(app);
    let ytdlp = tools::resolve_ytdlp(&app_data_dir).ok_or_else(|| {
        "yt-dlp не установлен — установите его в «Внешних источниках».".to_string()
    })?;

    let mut cmd = tools::silent_command(&ytdlp);
    cmd.arg("--dump-json")
        .arg("--no-warnings")
        .arg("--no-playlist")
        .arg("--skip-download")
        .arg("--socket-timeout")
        .arg("15");
    let cookies = cookies_file(app);
    if cookies.exists() {
        cmd.arg("--cookies").arg(&cookies);
    }
    if let Some(deno) = tools::resolve_deno(&app_data_dir) {
        cmd.arg("--js-runtimes").arg(tools::deno_runtime_value(&deno));
    }
    cmd.arg("--extractor-args")
        .arg(format!("youtube:player_client={}", YT_PLAYER_CLIENTS[0]))
        .arg("--user-agent")
        .arg(YOUTUBE_UA);
    if let Some(proxy) = proxy_arg(app) {
        cmd.arg("--proxy").arg(proxy);
    }
    cmd.arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd
        .output()
        .map_err(|e| format!("Не удалось запустить yt-dlp: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(extract_ytdlp_error(&stderr));
    }

    #[derive(Debug, Deserialize)]
    struct Probe {
        title: Option<String>,
        uploader: Option<String>,
        channel: Option<String>,
        duration: Option<f64>,
        thumbnail: Option<String>,
        webpage_url: Option<String>,
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // --dump-json prints one JSON object per line; a single video always
    // yields exactly one, but take the first non-empty line defensively
    // rather than assume.
    let line = stdout
        .lines()
        .find(|l| !l.trim().is_empty())
        .ok_or_else(|| "yt-dlp не вернул информацию о ссылке".to_string())?;
    let probe: Probe = serde_json::from_str(line)
        .map_err(|e| format!("Не удалось разобрать ответ yt-dlp: {}", e))?;
    let title = probe
        .title
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| "Не удалось получить название по ссылке".to_string())?;
    let artist = probe
        .uploader
        .or(probe.channel)
        .filter(|a| !a.trim().is_empty())
        .unwrap_or_default();
    // YouTube video titles are conventionally "Artist - Song"; without this the
    // tagged title/filename repeats the artist twice (e.g. "Rick Astley - Rick
    // Astley - Never Gonna Give You Up").
    let title = strip_artist_prefix(&title, &artist).unwrap_or(title);

    Ok(crate::external::ExternalSearchTrack {
        title,
        artist,
        album: None,
        duration_secs: probe.duration.map(|d| d.round().max(0.0) as u32),
        cover_url: probe.thumbnail,
        source: "Ссылка".to_string(),
        in_library: false,
        artist_id: None,
        album_id: None,
        genre: None,
        source_url: Some(probe.webpage_url.unwrap_or_else(|| url.to_string())),
    })
}

/// Strips a leading "{artist} - " from a YouTube-style title (case-insensitive,
/// tolerant of "-"/"–"/"—"), so a video titled "Artist - Song" with an uploader
/// name of "Artist" is stored as just "Song". Requires an actual dash right
/// after the artist name (not just any prefix match) so e.g. artist "Rick"
/// doesn't eat "Rickroll Video". `None` when the title doesn't start with the
/// artist name or nothing meaningful is left after stripping.
fn strip_artist_prefix(title: &str, artist: &str) -> Option<String> {
    let artist = artist.trim();
    if artist.is_empty() {
        return None;
    }
    let title_trim = title.trim_start();
    if title_trim.len() < artist.len() || !title_trim.is_char_boundary(artist.len()) {
        return None;
    }
    let (head, rest) = title_trim.split_at(artist.len());
    if !head.eq_ignore_ascii_case(artist) {
        return None;
    }
    let rest = rest.trim_start().strip_prefix(['-', '–', '—'])?.trim_start();
    (!rest.is_empty()).then(|| rest.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// "system" | "none" (direct) | "custom" (explicit URL).
    pub mode: String,
    pub url: String,
}

/// Value for yt-dlp's `--proxy` from current settings (see [`ResolvedTools::proxy`]):
/// `None` = flag omitted (system/env proxy); `Some("")` = direct connection;
/// `Some(url)` = explicit proxy (empty URL falls back to system).
fn proxy_arg(app: &AppHandle) -> Option<String> {
    let state = app.state::<crate::SettingsState>();
    let guard = state.0.lock().ok()?;
    match guard.0.proxy_mode.as_deref().unwrap_or("system") {
        "none" => Some(String::new()),
        "custom" => guard
            .0
            .proxy_url
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        _ => None, // "system" and any unknown value
    }
}

pub fn get_proxy(app: &AppHandle) -> ProxyConfig {
    let state = app.state::<crate::SettingsState>();
    // Bind guard to a local (rather than matching on the temporary `lock()`
    // directly): otherwise its destructor would run after `state` is dropped (E0597).
    let guard = state.0.lock();
    match guard {
        Ok(g) => ProxyConfig {
            mode: g
                .0
                .proxy_mode
                .clone()
                .unwrap_or_else(|| "system".to_string()),
            url: g.0.proxy_url.clone().unwrap_or_default(),
        },
        Err(_) => ProxyConfig {
            mode: "system".to_string(),
            url: String::new(),
        },
    }
}

/// Saves proxy config; unknown mode falls back to "system". An empty URL is
/// saved as None so "custom" with a blank field behaves like system, not a broken proxy.
pub fn set_proxy(app: &AppHandle, mode: String, url: String) {
    let mode = match mode.as_str() {
        "none" | "system" | "custom" => mode,
        _ => "system".to_string(),
    };
    let trimmed = url.trim().to_string();
    let state = app.state::<crate::SettingsState>();
    let mut guard = match state.0.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    guard.0.proxy_mode = Some(mode);
    guard.0.proxy_url = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };
    let path = guard.1.clone();
    let _ = crate::settings::save(&path, &guard.0);
}

fn ensure_worker(app: &AppHandle) {
    let start = with_manager(app, |m| {
        if m.worker_running {
            return false;
        }
        let has_queued = m.items.iter().any(|i| i.status == "queued");
        if has_queued {
            m.worker_running = true;
            true
        } else {
            false
        }
    });
    if !start {
        return;
    }
    let app2 = app.clone();
    if std::thread::Builder::new()
        .name("download-worker".into())
        .spawn(move || worker_loop(app2))
        .is_err()
    {
        // Thread failed to spawn — reset the flag, or worker_running would
        // stay true forever and the queue would be stuck.
        with_manager(app, |m| m.worker_running = false);
    }
}

/// Concurrent download workers. Kept conservative (not user-configurable): each
/// extra worker is another simultaneous yt-dlp/YouTube session, and too many at
/// once raises the odds of tripping YouTube's bot-wall for the whole batch.
const DOWNLOAD_CONCURRENCY: usize = 2;

fn worker_loop(app: AppHandle) {
    let app_data_dir = app_data_dir(&app);
    let dl_dir = downloads_dir(&app);

    // Sweep leftover temp files from a prior crashed/interrupted session
    // before the loop starts (no active download yet, so nothing of ours gets hit).
    cleanup_orphan_temp_files(&dl_dir);

    // Resolve tools without auto-downloading (supply-chain): only already
    // installed binaries are used; installation is explicit, from settings.
    // yt-dlp is required; ffmpeg is optional (limits format choice without it).
    let ytdlp = match tools::resolve_ytdlp(&app_data_dir) {
        Some(p) => p,
        None => {
            with_manager(&app, |m| {
                m.tools_preparing = false;
                m.ytdlp_ready = false;
                m.tools_message = None;
                for item in m.items.iter_mut() {
                    if item.status == "queued" || item.status == "downloading" {
                        item.status = "error".into();
                        item.error = Some(
                            "yt-dlp не установлен. Установите его в настройках → Внешние источники."
                                .into(),
                        );
                        item.error_kind = Some(DownloadErrorKind::ToolMissing);
                    }
                }
                m.run_done = m.run_total;
            });
            emit_snapshot(&app);
            finish_run(&app);
            return;
        }
    };
    let ffmpeg = tools::resolve_ffmpeg(&app_data_dir);
    let deno = tools::resolve_deno(&app_data_dir);
    let pot = tools::resolve_pot(&app_data_dir);
    with_manager(&app, |m| {
        m.tools_preparing = false;
        m.tools_message = None;
        m.ytdlp_ready = true;
        m.ffmpeg_ready = ffmpeg.is_some();
        m.deno_ready = deno.is_some();
        m.pot_ready = pot.is_some();
    });
    emit_snapshot(&app);
    let cookies_path = cookies_file(&app);
    let cookies_file_opt = cookies_path.exists().then_some(cookies_path);
    let resolved = Arc::new(tools::ResolvedTools {
        ytdlp,
        ffmpeg,
        deno,
        cookies_file: cookies_file_opt,
        proxy: proxy_arg(&app),
        pot,
    });

    // N-1 background workers + current thread as one more, all pulling from the
    // same queue — a conservative concurrency so a burst of downloads finishes
    // faster without multiplying YouTube bot-wall risk. finish_run below only
    // runs once every worker's queue has drained, matching the single-worker
    // "restart if items were added mid-run" behavior finish_run already provides.
    let worker_count = DOWNLOAD_CONCURRENCY;
    let mut handles = Vec::with_capacity(worker_count.saturating_sub(1));
    for _ in 1..worker_count {
        let app = app.clone();
        let resolved = Arc::clone(&resolved);
        let dl_dir = dl_dir.clone();
        if let Ok(h) = std::thread::Builder::new()
            .name("download-worker".into())
            .spawn(move || download_worker_loop(app, resolved, dl_dir))
        {
            handles.push(h);
        }
    }
    download_worker_loop(app.clone(), resolved, dl_dir);
    for h in handles {
        let _ = h.join();
    }

    // Run mood analysis once per batch (not per track), or an N-track queue
    // would restart the background analyzer N times, hindering its progress.
    restart_audio_feature_analysis(&app);

    finish_run(&app);
}

/// One worker's pull loop: pops jobs off the shared queue until it's empty,
/// downloading/tagging/importing each. Several of these run concurrently (see
/// [`DOWNLOAD_CONCURRENCY`]), each with its own child-process slot registered
/// under the job's id so [`DownloadManager::remove_item`] can cancel the right one.
fn download_worker_loop(app: AppHandle, resolved: Arc<tools::ResolvedTools>, dl_dir: PathBuf) {
    loop {
        let job = with_manager(&app, DownloadManager::take_next_queued);
        let Some(job) = job else {
            break;
        };
        emit_snapshot(&app);

        let child_slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        with_manager(&app, |m| {
            m.active_children.insert(job.id, Arc::clone(&child_slot));
        });

        let result = download_one(&app, &resolved, &dl_dir, &job, &child_slot);

        with_manager(&app, |m| {
            m.active_children.remove(&job.id);
        });

        // Import happens right after a successful download, before the final
        // status is set: otherwise the UI would show "done" even if the scan
        // failed to actually add the track. Import errors are surfaced separately.
        let import_err: Option<String> = match &result {
            Ok(path) => import_track(&app, &dl_dir, path).err(),
            Err(_) => None,
        };

        with_manager(&app, |m| m.apply_download_result(job.id, &result, import_err.as_deref()));
        emit_snapshot(&app);
        report_titlebar(&app);
    }
}

/// Restarts the background audio-feature analyzer so it picks up newly added
/// tracks. Best-effort: skipped if the library is busy (will catch up next run).
fn restart_audio_feature_analysis(app: &AppHandle) {
    if let Ok(lib) = app.state::<crate::LibraryState>().0.lock() {
        let _ = lib.resume_pending_audio_feature_analysis(None);
    }
}

/// Ends the run: resets counters/worker flag, pushes titlebar progress to
/// "done", and restarts the worker if items were added mid-run.
fn finish_run(app: &AppHandle) {
    let (done, total) = with_manager(app, DownloadManager::take_run_counts);
    crate::report_download_progress(app, total.max(done), total.max(done));
    emit_snapshot(app);
    ensure_worker(app);
}

/// YouTube player clients to rotate through on bot-wall retries. The default
/// `web` client is the most bot-walled and now needs a PO token, so we use
/// less-restricted clients and rotate: a failure is often client-specific, and
/// another client passes where the previous one hit a wall. Ordered from most
/// robust (`tv` also works with cookies) to fallbacks.
const YT_PLAYER_CLIENTS: &[&str] = &["tv", "web_safari", "mweb"];

fn is_bot_wall(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("confirm you")
        || s.contains("not a bot")
        || s.contains("sign in to confirm")
        || s.contains("sign in to prove")
        || (s.contains("cookies") && s.contains("authentication") && s.contains("required"))
}

/// Explicit signal that yt-dlp rejected the cookies themselves (not a generic
/// bot-wall) — expired/rotated. Only here does asking to re-login make sense;
/// a plain "sign in to confirm you're not a bot" is IP/network-related and
/// must not invalidate a working login (common under VPN/datacenter IPs).
fn is_cookies_expired(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("cookies are no longer valid")
        || s.contains("account cookies are no longer valid")
        || s.contains("cookies have been rotated")
        || (s.contains("cookies") && s.contains("no longer valid"))
}

fn is_network_timeout(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("timed out")
        || s.contains("timeout")
        || s.contains("connection reset")
        || s.contains("connection refused")
        || s.contains("connection aborted")
        || s.contains("unable to connect")
        || s.contains("failed to resolve")
        || s.contains("getaddrinfo")
        || s.contains("temporary failure in name resolution")
        || s.contains("network is unreachable")
        || s.contains("urlopen error")
}

fn is_no_format(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("requested format is not available")
        || s.contains("no video formats")
        || s.contains("no formats found")
        || s.contains("only images are available")
}

/// Classifies a yt-dlp stderr into a [`DownloadErrorKind`]. Order matters:
/// specific causes (rejected cookies, bot-wall) are checked before generic
/// network/format text, so the latter can't shadow a more specific match.
fn classify_ytdlp_error(stderr: &str) -> DownloadErrorKind {
    if is_cookies_expired(stderr) {
        DownloadErrorKind::CookiesExpired
    } else if is_bot_wall(stderr) {
        DownloadErrorKind::BotWallNetwork
    } else if is_network_timeout(stderr) {
        DownloadErrorKind::NetworkTimeout
    } else if is_no_format(stderr) {
        DownloadErrorKind::NoFormat
    } else {
        DownloadErrorKind::Unknown
    }
}

/// Search targets in order of reliability: YouTube Music first (clean audio
/// releases without video filler), then `ytsearch1` as a robust fallback.
fn search_targets(query: &str) -> Vec<String> {
    vec![
        format!(
            "https://music.youtube.com/search?q={}#songs",
            percent_encode_query(query)
        ),
        format!("ytsearch1:{}", query),
    ]
}

/// One raw download attempt (specific stem/target/client) plus its side
/// effects (temp cleanup, YouTube login status). Exists so [`download_one_core`]'s
/// client-rotation/bot-wall/cookie-expiry logic can be unit-tested without a
/// real yt-dlp subprocess or `AppHandle`: production uses
/// [`download_one_with_stem`]/`mark_cookies_*`/`emit_snapshot`, tests just
/// push fake responses to a `Vec`.
trait DownloadAttempt {
    fn try_download(&self, stem: &str, target: &str, client: &str) -> Result<PathBuf, DownloadError>;
    fn cleanup_stem(&self, stem: &str, keep: Option<&Path>);
    fn mark_login_still_valid(&self);
    fn mark_login_expired(&self);
}

struct RealDownloadAttempt<'a> {
    app: &'a AppHandle,
    resolved: &'a tools::ResolvedTools,
    dir: &'a Path,
    job: &'a DownloadItem,
    child_slot: &'a Arc<Mutex<Option<Child>>>,
}

impl DownloadAttempt for RealDownloadAttempt<'_> {
    fn try_download(&self, stem: &str, target: &str, client: &str) -> Result<PathBuf, DownloadError> {
        download_one_with_stem(
            self.app,
            self.resolved,
            self.dir,
            self.job,
            self.child_slot,
            stem,
            target,
            client,
        )
    }

    fn cleanup_stem(&self, stem: &str, keep: Option<&Path>) {
        remove_stem_files(self.dir, stem, keep);
    }

    fn mark_login_still_valid(&self) {
        mark_cookies_valid_after_success(self.app);
    }

    fn mark_login_expired(&self) {
        mark_cookies_invalid(self.app);
        emit_snapshot(self.app);
    }
}

/// Downloads one track via yt-dlp and tags it, returning the file path. A
/// `direct_url` job (paste-a-link flow) downloads that exact URL as its only
/// target; otherwise tries search targets in order ([`search_targets`]) as a
/// hedge against an unstable music extractor.
fn download_one(
    app: &AppHandle,
    resolved: &tools::ResolvedTools,
    dir: &Path,
    job: &DownloadItem,
    child_slot: &Arc<Mutex<Option<Child>>>,
) -> Result<PathBuf, DownloadError> {
    let stem = unique_stem(dir, &job.artist, &job.title);
    let targets = match &job.direct_url {
        Some(url) => vec![url.clone()],
        None => search_targets(&job.query),
    };
    let attempt = RealDownloadAttempt {
        app,
        resolved,
        dir,
        job,
        child_slot,
    };
    download_one_core(
        resolved.cookies_file.is_some(),
        &stem,
        &targets,
        Duration::from_secs(2),
        &attempt,
    )
}

/// Orchestrates client/target rotation around [`DownloadAttempt::try_download`],
/// independent of AppHandle/subprocess so it's directly unit-testable with a
/// fake implementation. `has_cookies` is `resolved.cookies_file.is_some()`.
fn download_one_core(
    has_cookies: bool,
    stem: &str,
    targets: &[String],
    retry_delay: Duration,
    attempt: &impl DownloadAttempt,
) -> Result<PathBuf, DownloadError> {
    // YouTube's bot-wall is almost always transient and IP-bound (not
    // cookie-bound): the same cookies pass on the 2nd-3rd attempt. So on a
    // bot-wall we retry while rotating player_client ([`YT_PLAYER_CLIENTS`]) —
    // failures are often client-specific. Login is marked expired ONLY when
    // yt-dlp explicitly rejects the cookies (kind == CookiesExpired); a
    // generic bot-wall must not do that. Non-bot-wall errors aren't retried.
    let mut last_err =
        DownloadError::new(DownloadErrorKind::Unknown, "yt-dlp завершился с ошибкой");
    let mut bot_wall_seen = false;
    let mut cookies_expired_seen = false;

    for (idx, &client) in YT_PLAYER_CLIENTS.iter().enumerate() {
        let mut should_retry = false;
        for target in targets {
            let result = attempt.try_download(stem, target, client);
            // Clean up yt-dlp's temp tails regardless of outcome: `stem` is
            // unique per job, so on error all `stem.*` can be removed safely,
            // and on success everything except the final file.
            match result {
                Ok(final_path) => {
                    attempt.cleanup_stem(stem, Some(&final_path));
                    if has_cookies {
                        attempt.mark_login_still_valid();
                    }
                    return Ok(final_path);
                }
                Err(e) => {
                    attempt.cleanup_stem(stem, None);
                    match e.kind {
                        DownloadErrorKind::Cancelled => return Err(e),
                        // Cookies themselves rejected: keep rotating (a
                        // cookie-less client might pass), but remember it in
                        // case nothing ends up succeeding.
                        DownloadErrorKind::CookiesExpired => {
                            cookies_expired_seen = true;
                            should_retry = true;
                        }
                        DownloadErrorKind::BotWallNetwork => {
                            bot_wall_seen = true;
                            should_retry = true;
                        }
                        _ => {}
                    }
                    last_err = e;
                }
            }
        }
        // Only retry a transient bot-wall/cookie rejection; otherwise bail.
        if !should_retry {
            break;
        }
        if idx + 1 < YT_PLAYER_CLIENTS.len() && !retry_delay.is_zero() {
            std::thread::sleep(retry_delay);
        }
    }

    if cookies_expired_seen && has_cookies {
        attempt.mark_login_expired();
        return Err(DownloadError::new(
            DownloadErrorKind::CookiesExpired,
            "Cookies YouTube больше не подходят — отправьте сессию браузера снова в «Внешних источниках»",
        ));
    }
    // Generic bot-wall survived every client — IP/network issue, not cookies,
    // so the login isn't invalidated. Hint depends on whether cookies are configured.
    if bot_wall_seen {
        let message = if has_cookies {
            "YouTube запросил подтверждение (IP/сеть) — попробуйте позже, смените сеть/VPN или прокси"
        } else {
            "YouTube запросил подтверждение (IP/сеть) — отправьте cookies браузера через расширение или смените сеть/прокси в «Внешних источниках»"
        };
        return Err(DownloadError::new(
            DownloadErrorKind::BotWallNetwork,
            message,
        ));
    }
    Err(last_err)
}

fn download_one_with_stem(
    app: &AppHandle,
    resolved: &tools::ResolvedTools,
    dir: &Path,
    job: &DownloadItem,
    child_slot: &Arc<Mutex<Option<Child>>>,
    stem: &str,
    search_target: &str,
    player_client: &str,
) -> Result<PathBuf, DownloadError> {
    let out_template = dir.join(format!("{}.%(ext)s", stem));

    // The library only scans specific containers (m4a/opus/ogg/mp3/...), but
    // YouTube's best audio is usually .webm, which isn't supported. With
    // ffmpeg, grab the absolute best audio and extract to .m4a (`-x
    // --audio-format m4a` below); without it, hard-restrict to supported
    // containers with no `best` fallback, so yt-dlp errors instead of
    // downloading an unsupported .webm.
    let ffmpeg_available = resolved.ffmpeg.is_some();
    let format = if ffmpeg_available {
        // YouTube's plain "bestaudio" is almost always a higher-bitrate Opus/WebM
        // stream, which `-x --audio-format m4a` below then has to actually
        // transcode (Opus decode + AAC encode). Preferring an m4a/AAC stream when
        // one exists (nearly always does) makes that step a copy instead — same
        // final container, no re-encode CPU cost or generation loss. Falls back to
        // plain bestaudio (and an actual transcode) only when no m4a stream exists.
        "bestaudio[ext=m4a]/bestaudio/best"
    } else {
        "bestaudio[ext=m4a]/bestaudio[ext=mp3]/bestaudio[ext=opus]/\
         bestaudio[ext=ogg]/bestaudio[ext=aac]/bestaudio[ext=flac]/bestaudio[ext=wav]"
    };

    let mut cmd = tools::silent_command(&resolved.ytdlp);
    cmd.arg("--playlist-items")
        .arg("1")
        .arg("--no-warnings")
        .arg("--newline")
        // More retries + socket timeout so a single drop/timeout doesn't fail
        // the whole track (the main source of "instability").
        .arg("--retries")
        .arg("10")
        .arg("--fragment-retries")
        .arg("10")
        .arg("--extractor-retries")
        .arg("3")
        .arg("--socket-timeout")
        .arg("30")
        // No --concurrent-fragments: parallel fragment writes use seek/
        // preallocation, which throws `OSError: [Errno 22]` on Windows for
        // some YouTube streams (SABR/fragmented DASH). Single audio streams
        // don't benefit from parallelism anyway.
        .arg("-f")
        .arg(format)
        .arg("-o")
        .arg(&out_template)
        .arg(search_target);
    if let Some(ff) = &resolved.ffmpeg {
        // -x/--audio-format m4a instead of a plain remux: YouTube's bestaudio
        // is often real Opus in WebM, and our player (symphonia 0.5) has no
        // Opus decoder at all — a bare remux produced a file with the right
        // container but nothing could decode the stream (playback silently
        // failed, see AudioPlayer::supported_formats). Forcing m4a transcodes
        // to AAC, which symphonia decodes; if the source is already AAC,
        // yt-dlp just copies the stream without re-encoding. -x also always
        // drops the video track (-vn) — some videos carry an embedded cover
        // as an mjpeg stream inside bestaudio, which a bare remux used to keep.
        cmd.arg("-x").arg("--audio-format").arg("m4a");
        // Best VBR quality for the rare case where a transcode is actually
        // needed (source isn't already m4a/AAC) — yt-dlp's own default (5,
        // mid-range) otherwise re-encodes at a needlessly low bitrate. When
        // the source is already AAC, yt-dlp stream-copies regardless and this
        // is a no-op.
        cmd.arg("--audio-quality").arg("0");
        // --ffmpeg-location only makes sense for our own downloaded binary
        // (absolute path); for a bare name from PATH, parent() == "" would
        // break the call, so rely on PATH instead.
        if ff.is_absolute() {
            if let Some(ff_dir) = ff.parent() {
                cmd.arg("--ffmpeg-location").arg(ff_dir);
            }
        }
    }
    // Imported cookies are optional: pass them if present to work around the
    // bot-wall, otherwise try without (most tracks download fine either way).
    if let Some(cookies) = &resolved.cookies_file {
        cmd.arg("--cookies").arg(cookies);
    }
    // Deno JS engine: needed for correct YouTube format extraction (nsig
    // decryption); avoids degraded SABR streams that cause `[Errno 22]` on Windows.
    if let Some(deno) = &resolved.deno {
        cmd.arg("--js-runtimes")
            .arg(tools::deno_runtime_value(deno));
    }
    // PO-token provider (bgutil-pot, script/CLI mode): mitigates YouTube's
    // "Sign in to confirm you're not a bot" wall even when cookies are valid
    // (that wall is IP/client-attestation based, not account-based — see
    // is_bot_wall/is_cookies_expired below). `--plugin-dirs` points yt-dlp at
    // our app-managed plugin copy; the extractor-arg tells that plugin where
    // the bundled executable lives (it's not on PATH).
    if let Some(pot) = &resolved.pot {
        cmd.arg("--plugin-dirs")
            .arg(tools::normalize_path_arg(&pot.plugin_dir));
        cmd.arg("--extractor-args").arg(format!(
            "youtubepot-bgutilcli:cli_path={}",
            tools::normalize_path_arg(&pot.cli_path)
        ));
    }
    cmd.arg("--extractor-args")
        .arg(format!("youtube:player_client={}", player_client));
    // Matching a real desktop Chrome fingerprint ([`YOUTUBE_UA`]) is less
    // likely to trigger a bot challenge than yt-dlp's default UA.
    cmd.arg("--user-agent").arg(YOUTUBE_UA);
    if let Some(proxy) = &resolved.proxy {
        cmd.arg("--proxy").arg(proxy);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Unix: put yt-dlp in its own process group so cancellation can kill the
    // whole tree (yt-dlp + its ffmpeg/deno children) via a group signal.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = cmd.spawn().map_err(|e| {
        DownloadError::new(
            DownloadErrorKind::ToolMissing,
            format!("Не удалось запустить yt-dlp: {}", e),
        )
    })?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    {
        let mut slot = child_slot.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(child);
    }

    // Drain stderr on its own thread (its buffer could otherwise block).
    // Reads raw bytes + lossy decode: yt-dlp's local encoding must not break
    // stderr collection (read_to_string would fail on invalid UTF-8 and lose
    // the whole output, including the error text).
    let stderr_handle = stderr.map(|err| {
        std::thread::spawn(move || {
            let mut bytes: Vec<u8> = Vec::new();
            let _ = BufReader::new(err).read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    });

    // Parses progress from stdout (yt-dlp with --newline writes percentages
    // per line). Reads raw bytes + lossy decode rather than BufRead::lines()
    // (strict UTF-8): yt-dlp can write stdout in a local encoding (cp1251 on
    // Windows), and a Cyrillic line would be invalid UTF-8 — lines() would
    // stop reading, closing the pipe and crashing yt-dlp with `[Errno 22]`
    // on its next stdout write.
    if let Some(out) = stdout {
        let mut reader = BufReader::new(out);
        let mut last_emit = Instant::now() - Duration::from_millis(500);
        let mut raw: Vec<u8> = Vec::new();
        loop {
            raw.clear();
            match reader.read_until(b'\n', &mut raw) {
                Ok(0) => break, // EOF
                Ok(_) => {}
                Err(_) => break,
            }
            let line = String::from_utf8_lossy(&raw);
            if let Some(pct) = parse_download_percent(&line) {
                let now = Instant::now();
                if now.duration_since(last_emit) >= Duration::from_millis(200) {
                    last_emit = now;
                    with_manager(app, |m| {
                        if let Some(item) = m.items.iter_mut().find(|i| i.id == job.id) {
                            item.progress = pct;
                        }
                    });
                    emit_snapshot(app);
                    report_titlebar_frac(app, pct / 100.0);
                }
            }
        }
    }

    // Take the process back and wait for it to finish.
    let status = {
        let taken = child_slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        match taken {
            Some(mut c) => c
                .wait()
                .map_err(|e| DownloadError::new(DownloadErrorKind::Unknown, e.to_string()))?,
            // Process already taken (cancellation) -> treat as cancelled.
            None => {
                return Err(DownloadError::new(
                    DownloadErrorKind::Cancelled,
                    "Скачивание отменено",
                ))
            }
        }
    };

    let stderr_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    if !status.success() {
        // Full stderr goes to the app log: the user-facing message is
        // trimmed, but diagnosing cryptic yt-dlp errors (e.g. `[Errno 22]`)
        // needs the full output with context.
        eprintln!(
            "[downloads] yt-dlp ошибка (запрос «{}»):\n{}",
            job.query,
            stderr_text.trim()
        );
        let kind = classify_ytdlp_error(&stderr_text);
        return Err(DownloadError::new(kind, extract_ytdlp_error(&stderr_text)));
    }

    let file = find_output(dir, stem).ok_or_else(|| {
        DownloadError::new(
            DownloadErrorKind::Unknown,
            "Файл не найден после скачивания",
        )
    })?;

    // Safety net: if the final container isn't scanned by the library (e.g.
    // .webm slipped through without ffmpeg), don't report false success — delete and error.
    let ext = file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !is_supported_audio_ext(&ext) {
        let _ = std::fs::remove_file(&file);
        return Err(DownloadError::new(
            DownloadErrorKind::NoFormat,
            format!(
                "формат {} не поддерживается (нужен ffmpeg для конвертации)",
                if ext.is_empty() { "файла" } else { &ext }
            ),
        ));
    }

    // Tagging is best-effort: the file is already downloaded and playable
    // even if tags fail to write.
    write_tags(
        &file,
        &job.title,
        &job.artist,
        job.album.as_deref(),
        job.cover_url.as_deref(),
        job.genre.clone(),
        job.album_id,
    );

    Ok(file)
}

/// Extensions the library scans (see `AudioPlayer::supported_formats`).
const SUPPORTED_AUDIO_EXTS: &[&str] = &[
    "m4a", "opus", "ogg", "mp3", "flac", "wav", "aac", "mp4", "aiff", "aif",
];

fn is_supported_audio_ext(ext: &str) -> bool {
    SUPPORTED_AUDIO_EXTS.contains(&ext)
}

/// Percent-encodes a URL query value (UTF-8 bytes -> %XX).
fn percent_encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Windows reserved device names (`CON.ogg`, `NUL`, etc. can't be created).
/// Checked against the base name, case-insensitively.
fn is_reserved_windows_name(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0')
}

fn app_data_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Downloads folder: "LFMusic Downloads" under the system music directory by
/// default, or a user-configured path. Any failure preparing the configured
/// path (no perms, invalid, unreachable disk) falls back to the default Music folder.
fn downloads_dir(app: &AppHandle) -> PathBuf {
    let dir = configured_downloads_dir(app).unwrap_or_else(|| default_downloads_dir(app));
    if std::fs::create_dir_all(&dir).is_ok() {
        return dir;
    }
    let fallback = default_downloads_dir(app);
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

fn default_downloads_dir(app: &AppHandle) -> PathBuf {
    let base = app
        .path()
        .audio_dir()
        .ok()
        .unwrap_or_else(|| app_data_dir(app));
    base.join("LFMusic Downloads")
}

/// User-configured downloads path from settings, if non-empty. Not created
/// here — `downloads_dir` handles that (with fallback on failure).
fn configured_downloads_dir(app: &AppHandle) -> Option<PathBuf> {
    let state = app.state::<crate::SettingsState>();
    let guard = state.0.lock().ok()?;
    guard
        .0
        .download_dir
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Changes the downloads folder: prepares the new one (falls back to the
/// default Music folder on any failure), saves it, and returns the actual
/// `(old, new)` paths. Re-registering library folders is left to the caller
/// in lib.rs, which has access to progress emitters. `requested` = None/empty
/// resets to the default folder.
pub fn apply_downloads_dir(app: &AppHandle, requested: Option<String>) -> (PathBuf, PathBuf) {
    let old_dir = downloads_dir(app);

    // Prepare the requested path eagerly; on failure (no perms/invalid), save
    // None instead, which means the default folder (the fallback).
    let prepared: Option<String> = requested
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .and_then(|p| {
            let pb = PathBuf::from(&p);
            if std::fs::create_dir_all(&pb).is_ok() && pb.is_dir() {
                Some(pb.to_string_lossy().to_string())
            } else {
                None
            }
        });

    persist_download_dir(app, prepared);
    let new_dir = downloads_dir(app);
    (old_dir, new_dir)
}

/// Persists the download dir path to settings as its own field, not inside `options`.
fn persist_download_dir(app: &AppHandle, value: Option<String>) {
    let state = app.state::<crate::SettingsState>();
    let mut guard = match state.0.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    guard.0.download_dir = value;
    let path = guard.1.clone();
    let _ = crate::settings::save(&path, &guard.0);
}

/// Whether two paths refer to the same location. Canonicalizes both (handles
/// case/separator/`..` differences on Windows); falls back to a lexical
/// comparison if the file doesn't exist yet.
pub fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => lexical_normalize(a) == lexical_normalize(b),
    }
}

/// Strips Windows-forbidden filename characters and collapses whitespace.
fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "<>:\"/\\|?*".contains(c) || c.is_control() {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|c| c == '.' || c == ' ').to_string();
    trimmed.chars().take(120).collect()
}

/// Truncates the base name so the full path fits Windows `MAX_PATH` (260).
/// yt-dlp creates temp files next to the output (`{stem}.f251.webm.part`,
/// `{stem}.<ext>.ytdl`, fragments), so even a character-valid but long
/// "Artist - Title" can trigger `OSError: [Errno 22]` in `sanitize_open`
/// (yt-dlp #11251). Trims with headroom for the separator, worst-case temp
/// suffix, and " (NN)".
fn clamp_stem_for_path(dir: &Path, base: String) -> String {
    // Stay well under 260; SUFFIX_RESERVE ~ ".f251.webm.part" + " (12)".
    const SAFE_MAX_PATH: usize = 250;
    const SUFFIX_RESERVE: usize = 24;
    let dir_len = dir.to_string_lossy().chars().count();
    let budget = SAFE_MAX_PATH
        .saturating_sub(dir_len + 1 + SUFFIX_RESERVE)
        .max(16);
    if base.chars().count() <= budget {
        return base;
    }
    let truncated: String = base.chars().take(budget).collect();
    // No trailing spaces/dots after truncation (Windows dislikes them).
    let trimmed = truncated.trim_matches(|c| c == '.' || c == ' ').to_string();
    if trimmed.is_empty() {
        "track".to_string()
    } else {
        trimmed
    }
}

/// Extension-less filename ("Artist - Title"), made unique with a suffix.
/// Directory contents are read once into a `HashSet` so collisions are
/// checked in-memory (not a repeated `read_dir` per iteration).
fn unique_stem(dir: &Path, artist: &str, title: &str) -> String {
    let a = sanitize(artist);
    let t = sanitize(title);
    // Either part can be sanitized away entirely (title made only of special
    // chars) — avoid a dangling " - " or an empty stem in that case.
    let base = match (a.is_empty(), t.is_empty()) {
        (false, false) => format!("{} - {}", a, t),
        (false, true) => a,
        (true, false) => t,
        (true, true) => "track".to_string(),
    };
    let mut base = clamp_stem_for_path(dir, base);
    if is_reserved_windows_name(&base) {
        base.push('_');
    }

    let existing: HashSet<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(String::from)
            })
            .collect(),
        Err(_) => HashSet::new(),
    };

    if !existing.contains(&base) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{} ({})", base, n);
        if !existing.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Removes `stem.*` files in a folder (yt-dlp tails: fragments, `.part`, `.ytdl`).
/// `keep` is the final file to preserve, on success.
fn remove_stem_files(dir: &Path, stem: &str, keep: Option<&Path>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if let Some(k) = keep {
            if p == *k {
                continue;
            }
        }
        let name = match p.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name == stem || name.starts_with(&format!("{}.", stem)) {
            // `.lrc` sidecar is a deliberate output, not a yt-dlp temp tail —
            // don't remove it here, or lyrics would vanish right after being written.
            let is_lrc = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|e| e.eq_ignore_ascii_case("lrc"))
                .unwrap_or(false);
            if is_lrc {
                continue;
            }
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// Removes orphaned yt-dlp temp files (top-level of the downloads folder)
/// left behind by a crash or interrupted download. Finished audio files are
/// never touched — the filter is strictly on temp markers.
fn cleanup_orphan_temp_files(dir: &Path) {
    // Only touch files untouched for a long time: a concurrent session (a
    // second app instance or external downloader in the same folder) could
    // be actively writing its own `.part` right now, and an active `.part`
    // constantly updates its mtime. "Untouched for a day" is almost certainly
    // a crash leftover, not a live download. Any uncertainty about age -> don't delete.
    const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let lower = name.to_lowercase();
        // `.part`/`.ytdl`/`.part-*` (DASH/HLS fragments), plus abandoned
        // deletion quarantine files (`.lfm-trash-...`).
        let is_temp = lower.ends_with(".part")
            || lower.ends_with(".ytdl")
            || lower.contains(".part-")
            || lower.starts_with(".lfm-trash-");
        if !is_temp {
            continue;
        }
        // Skip recent (possibly active) files; any error reading mtime is
        // treated as "fresh" and left alone (race-safety).
        let old_enough = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map(|e| e >= MAX_AGE).unwrap_or(false))
            .unwrap_or(false);
        if old_enough {
            let _ = std::fs::remove_file(&p);
        }
    }
}

fn find_output(dir: &Path, stem: &str) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let ext = p
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext == "part" {
            continue;
        }
        if p.file_stem().and_then(|s| s.to_str()) == Some(stem) {
            candidates.push(p);
        }
    }
    // Prefer a supported audio extension (in case a remux intermediate file
    // is left behind), otherwise the first match.
    candidates
        .iter()
        .find(|p| {
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            is_supported_audio_ext(&ext)
        })
        .cloned()
        .or_else(|| candidates.into_iter().next())
}

/// Extracts a meaningful error message from yt-dlp's stderr: prefers yt-dlp's
/// own error (last `ERROR: …` line); for a bare Python traceback (e.g.
/// `OSError: [Errno 22] Invalid argument`) returns the last few non-empty
/// lines for context instead of one cryptic exception line.
fn extract_ytdlp_error(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return "yt-dlp завершился с ошибкой".to_string();
    }
    if let Some(err) = lines
        .iter()
        .rev()
        .find(|l| l.trim_start().starts_with("ERROR:"))
    {
        return err.trim().to_string();
    }
    let start = lines.len().saturating_sub(3);
    lines[start..].join(" | ")
}

/// Extracts the percentage from a yt-dlp progress line like `[download]  12.3% of ...`.
fn parse_download_percent(line: &str) -> Option<f64> {
    if !line.contains("[download]") {
        return None;
    }
    let idx = line.find('%')?;
    let start = line[..idx]
        .rfind(|c: char| c == ' ' || c == '\t')
        .map(|i| i + 1)
        .unwrap_or(0);
    line[start..idx].trim().parse::<f64>().ok()
}

// Tagging (lofty) + library registration

/// Shared HTTP client for covers (pool/cert init is expensive — one instance per process).
fn cover_client() -> &'static reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            // Without a User-Agent some CDNs (Apple/iTunes included) return 403.
            .user_agent("LFMusic/0.1.0 (+https://github.com/mel1x/LFMusic)")
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    })
}

fn fetch_cover_bytes(url: &str) -> Option<Vec<u8>> {
    const MAX: usize = 8 * 1024 * 1024;
    let resp = cover_client()
        .get(url)
        .send()
        .ok()?
        .error_for_status()
        .ok()?;
    // Reject obviously oversized images by Content-Length before downloading fully.
    if let Some(len) = resp.content_length() {
        if len > MAX as u64 {
            return None;
        }
    }
    // Read at most MAX+1 bytes — guards against an oversized/malformed chunked
    // response with no Content-Length, capping memory use.
    let mut bytes: Vec<u8> = Vec::new();
    resp.take((MAX + 1) as u64).read_to_end(&mut bytes).ok()?;
    if bytes.len() < 1000 || bytes.len() > MAX {
        return None;
    }
    Some(bytes)
}

/// Detects cover MIME by magic bytes (jpeg/png/webp/gif/bmp) — lofty must not
/// mislabel PNG/WebP as Jpeg or the cover may fail to render.
fn detect_image_mime(bytes: &[u8]) -> lofty::picture::MimeType {
    use lofty::picture::MimeType;
    if bytes.len() >= 8 && bytes[0..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        MimeType::Png
    } else if bytes.len() >= 3 && bytes[0..3] == [0xFF, 0xD8, 0xFF] {
        MimeType::Jpeg
    } else if bytes.len() >= 6 && (&bytes[0..6] == b"GIF87a" || &bytes[0..6] == b"GIF89a") {
        MimeType::Gif
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        MimeType::Unknown("image/webp".to_string())
    } else if bytes.len() >= 2 && &bytes[0..2] == b"BM" {
        MimeType::Bmp
    } else {
        MimeType::Jpeg
    }
}

// Lyrics (.lrc) — LRCLIB (lrclib.net): a free, keyless source

/// LRCLIB response (camelCase keys). `instrumental` tracks have no lyrics.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LrclibLyrics {
    #[serde(default)]
    plain_lyrics: Option<String>,
    #[serde(default)]
    synced_lyrics: Option<String>,
    #[serde(default)]
    instrumental: bool,
}

/// Fetched lyrics: synced (LRC timecodes) and/or plain.
struct FetchedLyrics {
    /// Synced lyrics in LRC format (`[mm:ss.xx] line`).
    synced: Option<String>,
    /// Plain lyrics with no timecodes.
    plain: Option<String>,
}

impl FetchedLyrics {
    /// Content for the `.lrc` file: prefers the synced version.
    fn lrc_body(&self) -> Option<&str> {
        self.synced.as_deref().or(self.plain.as_deref())
    }
    /// Text to embed in the tag (USLT/`LYRICS`): prefers plain.
    fn tag_body(&self) -> Option<&str> {
        self.plain.as_deref().or(self.synced.as_deref())
    }
}

/// Converts an LRCLIB response into [`FetchedLyrics`], filtering out
/// instrumentals and empty fields. `None` means nothing usable.
fn usable_lyrics(l: LrclibLyrics) -> Option<FetchedLyrics> {
    if l.instrumental {
        return None;
    }
    let nonempty = |s: Option<String>| s.filter(|v| !v.trim().is_empty());
    let synced = nonempty(l.synced_lyrics);
    let plain = nonempty(l.plain_lyrics);
    if synced.is_none() && plain.is_none() {
        return None;
    }
    Some(FetchedLyrics { synced, plain })
}

/// Fetches lyrics from LRCLIB. Tries the exact `/api/get` first (requires
/// duration, matched within ±2s); on a miss, falls back to `/api/search`
/// (first result with synced lyrics, else plain). Any failure returns `None`
/// (the track downloads without lyrics). Open/keyless source, like [`crate::external`].
fn fetch_lyrics(
    artist: &str,
    title: &str,
    album: Option<&str>,
    duration_secs: Option<u64>,
) -> Option<FetchedLyrics> {
    if artist.trim().is_empty() || title.trim().is_empty() {
        return None;
    }
    let client = cover_client();

    // Exact query: artist + title (+ album) + duration.
    if let Some(dur) = duration_secs {
        let url = format!(
            "https://lrclib.net/api/get?artist_name={}&track_name={}&album_name={}&duration={}",
            percent_encode_query(artist),
            percent_encode_query(title),
            percent_encode_query(album.unwrap_or("")),
            dur
        );
        if let Some(found) = client
            .get(&url)
            .send()
            .ok()
            .and_then(|r| r.error_for_status().ok())
            .and_then(|r| r.json::<LrclibLyrics>().ok())
            .and_then(usable_lyrics)
        {
            return Some(found);
        }
    }

    // Fallback: search without a hard duration constraint.
    let url = format!(
        "https://lrclib.net/api/search?artist_name={}&track_name={}",
        percent_encode_query(artist),
        percent_encode_query(title)
    );
    let results: Vec<LrclibLyrics> = client
        .get(&url)
        .send()
        .ok()
        .and_then(|r| r.error_for_status().ok())
        .and_then(|r| r.json().ok())
        .unwrap_or_default();

    let mut first_plain: Option<FetchedLyrics> = None;
    for l in results {
        if let Some(f) = usable_lyrics(l) {
            if f.synced.is_some() {
                return Some(f); // synced version is the best option
            }
            first_plain.get_or_insert(f);
        }
    }
    first_plain
}

/// Writes a `.lrc` file next to the track (same name, `.lrc` extension),
/// preferring synced lyrics. Best-effort: no lyrics means no-op.
fn write_lyrics_sidecar(audio_path: &Path, lyrics: Option<FetchedLyrics>) {
    let Some(body) = lyrics.as_ref().and_then(|l| l.lrc_body()) else {
        return;
    };
    let lrc_path = audio_path.with_extension("lrc");
    if let Err(e) = std::fs::write(&lrc_path, body) {
        eprintln!("[downloads] не удалось записать .lrc: {}", e);
    }
}

/// Writes tags (title/artist/album + cover + lyrics) to the downloaded file
/// and a `.lrc` sidecar ([`write_lyrics_sidecar`]). Best-effort: errors are
/// only logged (some containers like webm may not be taggable).
fn write_tags(
    path: &Path,
    title: &str,
    artist: &str,
    album: Option<&str>,
    cover_url: Option<&str>,
    genre_hint: Option<String>,
    album_id: Option<u64>,
) {
    use lofty::config::WriteOptions;
    use lofty::picture::{Picture, PictureType};
    use lofty::prelude::*;
    use lofty::tag::{ItemKey, Tag};

    // IMPORTANT: file type is detected by content (see `read_tagged_file`), not
    // extension. yt-dlp remuxes Opus audio into a container where lofty's
    // extension-based guess expects Vorbis; without magic-byte detection, opening
    // fails with "File missing magic signature" and tags/cover never get written.
    let mut tagged = match crate::audio_player::AudioPlayer::read_tagged_file(path) {
        Some(t) => t,
        None => {
            eprintln!(
                "[downloads] не удалось открыть для тегирования: {}",
                path.display()
            );
            // Container isn't taggable, but lyrics can still be saved alongside
            // (duration for an exact query is unknown — falls back to title search).
            write_lyrics_sidecar(path, fetch_lyrics(artist, title, album, None));
            return;
        }
    };

    // LRCLIB needs duration for exact matching (±2s) — read it from the file.
    let duration_secs = {
        let secs = tagged.properties().duration().as_secs();
        (secs > 0).then_some(secs)
    };

    // Lyrics (LRCLIB), cover (direct URL, else external providers) and genre
    // (Deezer album lookup, when not already known) hit different hosts and
    // don't depend on each other — fetched concurrently instead of paying
    // all three round-trips back to back.
    let (lyrics, cover_bytes, genre) = std::thread::scope(|scope| {
        let lyrics_task = scope.spawn(|| fetch_lyrics(artist, title, album, duration_secs));
        let cover_task =
            scope.spawn(|| resolve_cover_bytes(cover_url, artist, album.unwrap_or(""), title));
        let genre_task = scope.spawn(|| {
            genre_hint.or_else(|| album_id.and_then(crate::external::deezer_album_genre))
        });
        (
            lyrics_task.join().unwrap_or(None),
            cover_task.join().unwrap_or(None),
            genre_task.join().unwrap_or(None),
        )
    });

    let tag_type = tagged.primary_tag_type();
    if tagged.primary_tag().is_none() {
        tagged.insert_tag(Tag::new(tag_type));
    }
    let Some(tag) = tagged.primary_tag_mut() else {
        // Couldn't embed the tag — still save lyrics alongside.
        write_lyrics_sidecar(path, lyrics);
        return;
    };

    tag.set_title(title.to_string());
    tag.set_artist(artist.to_string());
    if let Some(al) = album {
        if !al.is_empty() {
            tag.set_album(al.to_string());
        }
    }
    if let Some(g) = genre {
        tag.set_genre(g);
    }
    // Cover: try the direct URL from search results (Deezer/iTunes CDN) first;
    // if missing or the download fails, fall back to external providers by
    // artist/album/title (same mechanism used for local tracks).
    if let Some(bytes) = cover_bytes {
        let mime = detect_image_mime(&bytes);
        let pic = Picture::new_unchecked(PictureType::CoverFront, Some(mime), None, bytes);
        tag.push_picture(pic);
    }
    // Embed lyrics in the tag (USLT/`LYRICS`) — plain text preferred.
    if let Some(body) = lyrics.as_ref().and_then(|l| l.tag_body()) {
        tag.insert_text(ItemKey::Lyrics, body.to_string());
    }

    if let Err(e) = tag.save_to_path(path, WriteOptions::default()) {
        eprintln!("[downloads] не удалось записать теги: {}", e);
    }

    // Write the `.lrc` sidecar after saving tags; the final `stem.*` cleanup
    // won't touch it (see [`remove_stem_files`]).
    write_lyrics_sidecar(path, lyrics);
}

/// Gets cover bytes: tries the direct `cover_url` first, then falls back to
/// external sources (iTunes/Deezer/Cover Art Archive) by track metadata.
/// Any failure returns `None` (tagging proceeds without a cover).
fn resolve_cover_bytes(
    cover_url: Option<&str>,
    artist: &str,
    album: &str,
    title: &str,
) -> Option<Vec<u8>> {
    if let Some(url) = cover_url {
        if let Some(bytes) = fetch_cover_bytes(url) {
            return Some(bytes);
        }
    }
    if let crate::external::FetchOutcome::Found(img) =
        crate::external::fetch_track_cover(artist, album, title)
    {
        if let Some(bytes) = fetch_cover_bytes(&img.url) {
            return Some(bytes);
        }
    }
    None
}

/// Adds a single just-downloaded file to the library (no full-folder walk —
/// the path is already known) and returns the outcome. On success emits
/// `downloads_library_updated` (frontend refreshes the catalog); on error it
/// does NOT emit — the worker surfaces it as a track-import error. `dir` is
/// the download folder (needed to derive `folder_path` and dedupe against
/// already-added folders).
fn import_track(app: &AppHandle, dir: &Path, file_path: &Path) -> Result<(), String> {
    let separators = crate::settings::get_artist_separators(&app.state::<crate::SettingsState>());
    let state = app.state::<crate::LibraryState>();
    let result = {
        let mut lib = state
            .0
            .lock()
            .map_err(|e| format!("библиотека занята: {}", e))?;
        lib.insert_downloaded_track(
            &file_path.to_string_lossy(),
            &dir.to_string_lossy(),
            &separators,
        )
    };
    if result.is_ok() {
        let _ = app.emit("downloads_library_updated", ());
    } else if let Err(e) = &result {
        eprintln!("[downloads] не удалось добавить трек в библиотеку: {}", e);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::path::Path;

    fn temp_downloads_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lfmusic_dl_{name}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn queued_item(id: u64) -> DownloadItem {
        DownloadItem {
            id,
            title: "T".into(),
            artist: "A".into(),
            album: None,
            query: "A T".into(),
            cover_url: None,
            album_id: None,
            genre: None,
            direct_url: None,
            status: "queued".into(),
            progress: 0.0,
            error: None,
            error_kind: None,
            file_path: None,
        }
    }

    fn item_with_status(id: u64, status: &str) -> DownloadItem {
        DownloadItem {
            status: status.into(),
            ..queued_item(id)
        }
    }

    #[test]
    fn take_next_queued_marks_downloading_and_clones_it() {
        let mut m = DownloadManager::default();
        m.items.push(queued_item(1));

        let job = m.take_next_queued().expect("should return the queued item");
        assert_eq!(job.status, "downloading");
        assert_eq!(m.items[0].status, "downloading");
        assert_eq!(m.items[0].progress, 0.0);
    }

    #[test]
    fn take_next_queued_returns_none_when_nothing_queued() {
        let mut m = DownloadManager::default();
        m.items.push(item_with_status(1, "downloading"));
        m.items.push(item_with_status(2, "done"));
        assert!(m.take_next_queued().is_none());
    }

    #[test]
    fn apply_download_result_marks_done_on_success() {
        let mut m = DownloadManager::default();
        m.items.push(item_with_status(1, "downloading"));

        m.apply_download_result(1, &Ok(PathBuf::from("/music/a.mp3")), None);

        assert_eq!(m.items[0].status, "done");
        assert_eq!(m.items[0].progress, 100.0);
        assert_eq!(m.items[0].file_path.as_deref(), Some("/music/a.mp3"));
        assert!(m.items[0].error.is_none());
        assert_eq!(m.run_done, 1);
    }

    #[test]
    fn apply_download_result_marks_error_when_import_fails_after_a_successful_download() {
        let mut m = DownloadManager::default();
        m.items.push(item_with_status(1, "downloading"));

        m.apply_download_result(1, &Ok(PathBuf::from("/music/a.mp3")), Some("disk full"));

        assert_eq!(m.items[0].status, "error");
        assert_eq!(m.items[0].file_path.as_deref(), Some("/music/a.mp3"));
        assert!(m.items[0].error.as_deref().unwrap().contains("disk full"));
        assert_eq!(m.items[0].error_kind, Some(DownloadErrorKind::Unknown));
    }

    #[test]
    fn apply_download_result_marks_error_on_download_failure() {
        let mut m = DownloadManager::default();
        m.items.push(item_with_status(1, "downloading"));

        let err = DownloadError::new(DownloadErrorKind::NoFormat, "no suitable format");
        m.apply_download_result(1, &Err(err), None);

        assert_eq!(m.items[0].status, "error");
        assert_eq!(m.items[0].error.as_deref(), Some("no suitable format"));
        assert_eq!(m.items[0].error_kind, Some(DownloadErrorKind::NoFormat));
    }

    #[test]
    fn apply_download_result_still_counts_run_done_if_item_was_removed_meanwhile() {
        // The user could remove the item from the queue (see remove_item) while
        // it was downloading — the worker doesn't know and must still count it
        // toward run_done, or the run's progress bar would stall short of 100%.
        let mut m = DownloadManager::default();
        m.run_total = 1;
        m.apply_download_result(999, &Ok(PathBuf::from("/music/a.mp3")), None);
        assert_eq!(m.run_done, 1);
    }

    #[test]
    fn remove_item_from_queued_status_decrements_run_total() {
        let mut m = DownloadManager::default();
        m.items.push(queued_item(1));
        m.run_total = 1;

        let child = m.remove_item(1);
        assert!(child.is_none());
        assert!(m.items.is_empty());
        assert_eq!(m.run_total, 0);
    }

    #[test]
    fn remove_item_from_downloading_status_with_no_active_child_returns_none() {
        let mut m = DownloadManager::default();
        m.items.push(item_with_status(1, "downloading"));

        let child = m.remove_item(1);
        assert!(child.is_none());
        assert!(m.items.is_empty());
    }

    #[test]
    fn remove_item_only_touches_its_own_job_id_child_slot() {
        // With concurrent download workers, active_children holds one slot per
        // in-flight job id — removing one job must not disturb another's slot.
        let mut m = DownloadManager::default();
        m.items.push(item_with_status(1, "downloading"));
        m.items.push(item_with_status(2, "downloading"));
        m.active_children.insert(2, Arc::new(Mutex::new(None)));

        assert!(m.remove_item(1).is_none());
        assert!(
            m.active_children.contains_key(&2),
            "removing job 1 must not touch job 2's registered child slot"
        );
    }

    #[test]
    fn remove_item_from_done_or_error_status_just_drops_it() {
        let mut m = DownloadManager::default();
        m.items.push(item_with_status(1, "done"));
        m.items.push(item_with_status(2, "error"));

        assert!(m.remove_item(1).is_none());
        assert!(m.remove_item(2).is_none());
        assert!(m.items.is_empty());
    }

    #[test]
    fn remove_item_of_unknown_id_is_a_no_op() {
        let mut m = DownloadManager::default();
        m.items.push(queued_item(1));
        assert!(m.remove_item(404).is_none());
        assert_eq!(m.items.len(), 1);
    }

    #[test]
    fn retry_item_requeues_an_errored_item_and_grows_run_total() {
        let mut m = DownloadManager::default();
        let mut errored = item_with_status(1, "error");
        errored.error = Some("boom".into());
        errored.error_kind = Some(DownloadErrorKind::NetworkTimeout);
        errored.progress = 42.0;
        m.items.push(errored);
        m.run_total = 0;

        assert!(m.retry_item(1));
        assert_eq!(m.items[0].status, "queued");
        assert_eq!(m.items[0].progress, 0.0);
        assert!(m.items[0].error.is_none());
        assert!(m.items[0].error_kind.is_none());
        assert_eq!(m.run_total, 1);
    }

    #[test]
    fn retry_item_refuses_non_error_items() {
        let mut m = DownloadManager::default();
        m.items.push(item_with_status(1, "downloading"));
        assert!(!m.retry_item(1));
        assert_eq!(m.items[0].status, "downloading");
    }

    #[test]
    fn retry_item_of_unknown_id_returns_false() {
        let mut m = DownloadManager::default();
        assert!(!m.retry_item(1));
    }

    #[test]
    fn clear_finished_items_keeps_only_queued_and_downloading() {
        let mut m = DownloadManager::default();
        m.items.push(item_with_status(1, "queued"));
        m.items.push(item_with_status(2, "downloading"));
        m.items.push(item_with_status(3, "done"));
        m.items.push(item_with_status(4, "error"));

        m.clear_finished_items();

        let remaining: Vec<u64> = m.items.iter().map(|i| i.id).collect();
        assert_eq!(remaining, vec![1, 2]);
    }

    #[test]
    fn take_run_counts_resets_counters_and_worker_flag() {
        let mut m = DownloadManager::default();
        m.run_done = 3;
        m.run_total = 5;
        m.worker_running = true;

        let (done, total) = m.take_run_counts();
        assert_eq!((done, total), (3, 5));
        assert_eq!(m.run_done, 0);
        assert_eq!(m.run_total, 0);
        assert!(!m.worker_running);
    }

    fn request(artist: &str, title: &str) -> DownloadRequest {
        DownloadRequest {
            title: title.into(),
            artist: artist.into(),
            album: None,
            query: None,
            cover_url: None,
            album_id: None,
            genre: None,
            force: None,
            direct_url: None,
        }
    }

    fn bridge_cookie(name: &str, domain: &str, http_only: bool) -> BridgeCookie {
        BridgeCookie {
            name: name.to_string(),
            value: "secret".to_string(),
            domain: domain.to_string(),
            path: None,
            secure: true,
            http_only,
            expiration_date: Some(4_000_000_000.0),
        }
    }

    #[test]
    fn netscape_youtube_domain_field_uses_wildcard_domains_for_ytdlp() {
        assert_eq!(
            netscape_youtube_domain_field("youtube.com", false).unwrap(),
            (".youtube.com".to_string(), "TRUE")
        );
        assert_eq!(
            netscape_youtube_domain_field("music.youtube.com", true).unwrap(),
            ("#HttpOnly_.youtube.com".to_string(), "TRUE")
        );
        assert_eq!(
            netscape_youtube_domain_field("accounts.google.com", false).unwrap(),
            (".google.com".to_string(), "TRUE")
        );
        assert!(netscape_youtube_domain_field("example.com", false).is_none());
    }

    #[test]
    fn has_bridge_auth_cookie_requires_a_known_name_on_a_youtube_domain() {
        assert!(has_bridge_auth_cookie(&[bridge_cookie(
            "SID",
            "music.youtube.com",
            false
        )]));
        assert!(!has_bridge_auth_cookie(&[bridge_cookie(
            "CONSENT",
            "youtube.com",
            false
        )]));
        assert!(!has_bridge_auth_cookie(&[bridge_cookie(
            "SID",
            "accounts.google.com",
            false
        )]));
    }

    #[test]
    fn render_bridge_cookies_file_skips_unrelated_domains_and_writes_wildcard_rows() {
        let cookies = vec![
            bridge_cookie("__Secure-3PSID", "music.youtube.com", true),
            bridge_cookie("unrelated", "example.com", false),
        ];
        let (content, count) = render_bridge_cookies_file(&cookies).unwrap();
        assert_eq!(count, 1);
        assert!(content.contains("#HttpOnly_.youtube.com\tTRUE\t/\tTRUE"));
        assert!(!content.contains("example.com"));
    }

    #[test]
    fn render_bridge_cookies_file_errors_when_nothing_matches() {
        let cookies = vec![bridge_cookie("unrelated", "example.com", false)];
        assert!(render_bridge_cookies_file(&cookies).is_err());
    }

    #[test]
    fn insert_request_of_a_new_track_queues_it_and_grows_run_total() {
        let mut m = DownloadManager::default();
        let (id, is_new) = insert_request(&mut m, request("Artist", "Title"));
        assert!(is_new);
        assert_eq!(m.items.len(), 1);
        assert_eq!(m.items[0].id, id);
        assert_eq!(m.items[0].status, "queued");
        assert_eq!(m.run_total, 1);
    }

    #[test]
    fn insert_request_of_an_already_queued_track_does_not_duplicate() {
        let mut m = DownloadManager::default();
        let (first_id, _) = insert_request(&mut m, request("Artist", "Title"));
        let (second_id, is_new) = insert_request(&mut m, request("Artist", "Title"));
        assert!(!is_new);
        assert_eq!(first_id, second_id);
        assert_eq!(m.items.len(), 1);
        assert_eq!(m.run_total, 1, "duplicate must not inflate the run counter");
    }

    #[test]
    fn insert_request_with_force_bypasses_dedup() {
        let mut m = DownloadManager::default();
        let (first_id, _) = insert_request(&mut m, request("Artist", "Title"));
        let mut forced = request("Artist", "Title");
        forced.force = Some(true);
        let (second_id, is_new) = insert_request(&mut m, forced);
        assert!(is_new);
        assert_ne!(first_id, second_id);
        assert_eq!(m.items.len(), 2);
    }

    #[test]
    fn insert_request_replaces_a_previously_errored_duplicate() {
        let mut m = DownloadManager::default();
        let (first_id, _) = insert_request(&mut m, request("Artist", "Title"));
        m.items[0].status = "error".into();

        let (second_id, is_new) = insert_request(&mut m, request("Artist", "Title"));
        assert!(is_new);
        assert_ne!(first_id, second_id);
        assert_eq!(m.items.len(), 1, "the stale errored duplicate must be dropped");
        assert_eq!(m.items[0].status, "queued");
    }

    #[derive(Default)]
    struct FakeAttempt {
        responses: RefCell<VecDeque<Result<PathBuf, DownloadError>>>,
        calls: RefCell<Vec<(String, String, String)>>,
        cleanup_calls: RefCell<Vec<(String, Option<PathBuf>)>>,
        login_still_valid_calls: RefCell<u32>,
        login_expired_calls: RefCell<u32>,
    }

    impl DownloadAttempt for FakeAttempt {
        fn try_download(&self, stem: &str, target: &str, client: &str) -> Result<PathBuf, DownloadError> {
            self.calls
                .borrow_mut()
                .push((stem.to_string(), target.to_string(), client.to_string()));
            self.responses
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err(DownloadError::new(DownloadErrorKind::Unknown, "no fake response left")))
        }

        fn cleanup_stem(&self, stem: &str, keep: Option<&Path>) {
            self.cleanup_calls
                .borrow_mut()
                .push((stem.to_string(), keep.map(|p| p.to_path_buf())));
        }

        fn mark_login_still_valid(&self) {
            *self.login_still_valid_calls.borrow_mut() += 1;
        }

        fn mark_login_expired(&self) {
            *self.login_expired_calls.borrow_mut() += 1;
        }
    }

    #[test]
    fn download_one_core_succeeds_on_first_attempt_and_marks_login_valid_with_cookies() {
        let fake = FakeAttempt::default();
        fake.responses
            .borrow_mut()
            .push_back(Ok(PathBuf::from("/music/song.opus")));

        let result = download_one_core(true, "song", &["t1".to_string()], Duration::ZERO, &fake);

        assert_eq!(result.unwrap(), PathBuf::from("/music/song.opus"));
        assert_eq!(fake.calls.borrow().len(), 1);
        assert_eq!(*fake.login_still_valid_calls.borrow(), 1);
        assert_eq!(*fake.login_expired_calls.borrow(), 0);
        assert_eq!(
            fake.cleanup_calls.borrow()[0],
            ("song".to_string(), Some(PathBuf::from("/music/song.opus")))
        );
    }

    #[test]
    fn download_one_core_does_not_touch_login_state_without_cookies() {
        let fake = FakeAttempt::default();
        fake.responses
            .borrow_mut()
            .push_back(Ok(PathBuf::from("/music/song.opus")));

        let result = download_one_core(false, "song", &["t1".to_string()], Duration::ZERO, &fake);

        assert!(result.is_ok());
        assert_eq!(*fake.login_still_valid_calls.borrow(), 0);
    }

    #[test]
    fn download_one_core_stops_immediately_on_cancelled() {
        let fake = FakeAttempt::default();
        fake.responses.borrow_mut().push_back(Err(DownloadError::new(
            DownloadErrorKind::Cancelled,
            "cancelled",
        )));

        let result = download_one_core(
            true,
            "song",
            &["t1".to_string(), "t2".to_string()],
            Duration::ZERO,
            &fake,
        );

        assert_eq!(result.unwrap_err().kind, DownloadErrorKind::Cancelled);
        assert_eq!(fake.calls.borrow().len(), 1, "must not retry after a cancellation");
    }

    #[test]
    fn download_one_core_rotates_player_client_on_bot_wall_then_succeeds() {
        let fake = FakeAttempt::default();
        fake.responses.borrow_mut().push_back(Err(DownloadError::new(
            DownloadErrorKind::BotWallNetwork,
            "bot wall",
        )));
        fake.responses
            .borrow_mut()
            .push_back(Ok(PathBuf::from("/music/song.opus")));

        let result = download_one_core(true, "song", &["t1".to_string()], Duration::ZERO, &fake);

        assert!(result.is_ok());
        let calls = fake.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].2, "tv");
        assert_eq!(calls[1].2, "web_safari");
        assert_eq!(*fake.login_expired_calls.borrow(), 0);
    }

    #[test]
    fn download_one_core_tries_all_targets_before_rotating_client() {
        let fake = FakeAttempt::default();
        fake.responses.borrow_mut().push_back(Err(DownloadError::new(
            DownloadErrorKind::BotWallNetwork,
            "bot wall",
        )));
        fake.responses.borrow_mut().push_back(Err(DownloadError::new(
            DownloadErrorKind::BotWallNetwork,
            "bot wall",
        )));
        fake.responses
            .borrow_mut()
            .push_back(Ok(PathBuf::from("/music/song.opus")));

        let result = download_one_core(
            false,
            "song",
            &["t1".to_string(), "t2".to_string()],
            Duration::ZERO,
            &fake,
        );

        assert!(result.is_ok());
        let calls = fake.calls.borrow();
        assert_eq!(
            calls.as_slice(),
            &[
                ("song".to_string(), "t1".to_string(), "tv".to_string()),
                ("song".to_string(), "t2".to_string(), "tv".to_string()),
                ("song".to_string(), "t1".to_string(), "web_safari".to_string()),
            ]
        );
    }

    #[test]
    fn download_one_core_marks_cookies_expired_only_when_every_client_rejects_them() {
        let fake = FakeAttempt::default();
        for _ in 0..3 {
            fake.responses.borrow_mut().push_back(Err(DownloadError::new(
                DownloadErrorKind::CookiesExpired,
                "cookies rejected",
            )));
        }

        let result = download_one_core(true, "song", &["t1".to_string()], Duration::ZERO, &fake);

        let err = result.unwrap_err();
        assert_eq!(err.kind, DownloadErrorKind::CookiesExpired);
        assert_eq!(*fake.login_expired_calls.borrow(), 1);
        assert_eq!(fake.calls.borrow().len(), 3, "should have rotated through all clients");
    }

    #[test]
    fn download_one_core_bot_wall_message_mentions_login_only_without_cookies() {
        let fake_no_cookies = FakeAttempt::default();
        for _ in 0..3 {
            fake_no_cookies
                .responses
                .borrow_mut()
                .push_back(Err(DownloadError::new(DownloadErrorKind::BotWallNetwork, "x")));
        }
        let err = download_one_core(false, "song", &["t1".to_string()], Duration::ZERO, &fake_no_cookies)
            .unwrap_err();
        assert!(err.message.contains("отправьте cookies браузера"));

        let fake_with_cookies = FakeAttempt::default();
        for _ in 0..3 {
            fake_with_cookies
                .responses
                .borrow_mut()
                .push_back(Err(DownloadError::new(DownloadErrorKind::BotWallNetwork, "x")));
        }
        let err = download_one_core(true, "song", &["t1".to_string()], Duration::ZERO, &fake_with_cookies)
            .unwrap_err();
        assert!(err.message.contains("смените сеть"));
    }

    #[test]
    fn download_one_core_generic_error_does_not_rotate_clients() {
        let fake = FakeAttempt::default();
        fake.responses.borrow_mut().push_back(Err(DownloadError::new(
            DownloadErrorKind::NoFormat,
            "no format",
        )));

        let result = download_one_core(true, "song", &["t1".to_string()], Duration::ZERO, &fake);

        assert_eq!(result.unwrap_err().kind, DownloadErrorKind::NoFormat);
        assert_eq!(fake.calls.borrow().len(), 1, "a non-transient error must not rotate clients");
        assert_eq!(*fake.login_expired_calls.borrow(), 0);
    }

    #[test]
    fn cleanup_orphan_temp_files_removes_only_old_temp_markers() {
        let dir = temp_downloads_dir("cleanup_orphans");
        std::fs::create_dir_all(&dir).unwrap();

        let old_part = dir.join("stale.part");
        let old_ytdl = dir.join("stale.ytdl");
        let old_frag = dir.join("stale.part-Frag1.part");
        let old_trash = dir.join(".lfm-trash-abc123");
        let fresh_part = dir.join("active.part");
        let finished_audio = dir.join("done.opus");
        for f in [&old_part, &old_ytdl, &old_frag, &old_trash, &fresh_part, &finished_audio] {
            std::fs::write(f, b"x").unwrap();
        }
        // Emulate "untouched for a while" by explicitly setting mtime 2 days back.
        let two_days_ago = std::time::SystemTime::now() - Duration::from_secs(2 * 24 * 60 * 60);
        for f in [&old_part, &old_ytdl, &old_frag, &old_trash] {
            let file = std::fs::OpenOptions::new().write(true).open(f).unwrap();
            file.set_modified(two_days_ago).unwrap();
        }

        cleanup_orphan_temp_files(&dir);

        assert!(!old_part.exists(), "old .part must be cleaned up");
        assert!(!old_ytdl.exists(), "old .ytdl must be cleaned up");
        assert!(!old_frag.exists(), "old fragment temp file must be cleaned up");
        assert!(!old_trash.exists(), "old quarantine leftover must be cleaned up");
        assert!(fresh_part.exists(), "a recently-touched .part might be an active download — must survive");
        assert!(finished_audio.exists(), "cleanup must never touch finished audio files");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_stem_files_keeps_lrc_sidecar_and_the_excluded_final_file() {
        let dir = temp_downloads_dir("remove_stem");
        std::fs::create_dir_all(&dir).unwrap();

        let part = dir.join("song.part");
        let frag = dir.join("song.f140.m4a");
        let lrc = dir.join("song.lrc");
        let final_file = dir.join("song.opus");
        let unrelated = dir.join("other.mp3");
        for f in [&part, &frag, &lrc, &final_file, &unrelated] {
            std::fs::write(f, b"x").unwrap();
        }

        remove_stem_files(&dir, "song", Some(&final_file));

        assert!(!part.exists());
        assert!(!frag.exists());
        assert!(lrc.exists(), ".lrc sidecar is a deliberate output, not a temp tail");
        assert!(final_file.exists(), "the kept final file must survive");
        assert!(unrelated.exists(), "files outside the stem must be untouched");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_stem_files_without_a_keep_path_removes_the_whole_stem_except_lrc() {
        let dir = temp_downloads_dir("remove_stem_all");
        std::fs::create_dir_all(&dir).unwrap();

        let webm = dir.join("song.webm");
        let lrc = dir.join("song.lrc");
        for f in [&webm, &lrc] {
            std::fs::write(f, b"x").unwrap();
        }

        remove_stem_files(&dir, "song", None);

        assert!(!webm.exists());
        assert!(lrc.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clamp_keeps_normal_names() {
        let dir = Path::new("C:\\Users\\melix\\Music\\LFMusic Downloads");
        let base = "Imagine Dragons - Believer".to_string();
        assert_eq!(clamp_stem_for_path(dir, base.clone()), base);
    }

    #[test]
    fn clamp_keeps_full_path_under_max_path() {
        let dir = Path::new("C:\\Users\\melix\\Music\\LFMusic Downloads");
        let stem = clamp_stem_for_path(dir, "x".repeat(400));
        assert!(!stem.is_empty());
        // Even with uniquification and yt-dlp's worst-case temp suffix, the path
        // must stay under Windows MAX_PATH (260) or sanitize_open gives [Errno 22].
        let worst = format!("{}\\{} (12).f251.webm.part", dir.display(), stem);
        assert!(
            worst.chars().count() < 260,
            "len = {}",
            worst.chars().count()
        );
    }

    #[test]
    fn clamp_no_trailing_space_or_dot() {
        let dir = Path::new("C:\\d");
        let base: String = std::iter::repeat("a ").take(200).collect();
        let stem = clamp_stem_for_path(dir, base);
        assert!(!stem.ends_with(' ') && !stem.ends_with('.'));
    }

    #[test]
    fn error_prefers_yt_dlp_error_line() {
        let stderr = "[debug] foo\nERROR: [youtube] abc: Video unavailable\n";
        assert_eq!(
            extract_ytdlp_error(stderr),
            "ERROR: [youtube] abc: Video unavailable"
        );
    }

    #[test]
    fn error_falls_back_to_traceback_tail() {
        let stderr = "Traceback (most recent call last):\n  \
            File \"common.py\", line 1, in f\n    open(p)\n\
            OSError: [Errno 22] Invalid argument\n";
        let msg = extract_ytdlp_error(stderr);
        assert!(msg.contains("OSError: [Errno 22]"));
        assert!(msg.contains("File"));
    }

    #[test]
    fn error_empty_stderr_has_fallback() {
        assert_eq!(extract_ytdlp_error("  \n\n"), "yt-dlp завершился с ошибкой");
    }

    #[test]
    fn classify_detects_cookies_expired_before_generic_bot_wall() {
        // Message contains both markers — "cookies"+"no longer valid" must win
        // since it's more specific and check order guarantees that.
        let stderr = "ERROR: [youtube] Sign in to confirm you're not a bot. \
                       Your account cookies are no longer valid.";
        assert_eq!(classify_ytdlp_error(stderr), DownloadErrorKind::CookiesExpired);
    }

    #[test]
    fn classify_detects_bot_wall_network() {
        let stderr = "ERROR: [youtube] abc: Sign in to confirm you're not a bot";
        assert_eq!(classify_ytdlp_error(stderr), DownloadErrorKind::BotWallNetwork);
    }

    #[test]
    fn classify_detects_bot_wall_via_cookies_authentication_required_combo() {
        let stderr = "Use --cookies for the authentication; cookies required";
        assert_eq!(classify_ytdlp_error(stderr), DownloadErrorKind::BotWallNetwork);
    }

    #[test]
    fn classify_detects_network_timeout() {
        for msg in [
            "URLError: <urlopen error [Errno 11001] getaddrinfo failed>",
            "Connection reset by peer",
            "Read timed out",
            "Temporary failure in name resolution",
        ] {
            assert_eq!(
                classify_ytdlp_error(msg),
                DownloadErrorKind::NetworkTimeout,
                "msg={msg}"
            );
        }
    }

    #[test]
    fn classify_detects_no_format() {
        assert_eq!(
            classify_ytdlp_error("ERROR: Requested format is not available"),
            DownloadErrorKind::NoFormat
        );
        assert_eq!(
            classify_ytdlp_error("ERROR: No video formats found!"),
            DownloadErrorKind::NoFormat
        );
    }

    #[test]
    fn classify_falls_back_to_unknown() {
        assert_eq!(
            classify_ytdlp_error("ERROR: something completely unrelated happened"),
            DownloadErrorKind::Unknown
        );
    }

    #[test]
    fn classify_is_case_insensitive() {
        assert_eq!(
            classify_ytdlp_error("SIGN IN TO CONFIRM YOU'RE NOT A BOT"),
            DownloadErrorKind::BotWallNetwork
        );
    }

    #[test]
    fn is_bot_wall_does_not_misfire_on_unrelated_cookie_mentions() {
        // A plain "cookies" mention without "authentication"+"required" together
        // must not classify as a bot wall.
        assert!(!is_bot_wall("Loaded 12 cookies from file"));
    }

    #[test]
    fn parse_download_percent_extracts_value_from_progress_line() {
        assert_eq!(
            parse_download_percent("[download]  42.3% of 5.21MiB at 1.23MiB/s ETA 00:03"),
            Some(42.3)
        );
        assert_eq!(parse_download_percent("[download] 100% of 5.21MiB"), Some(100.0));
    }

    #[test]
    fn parse_download_percent_ignores_unrelated_lines() {
        assert_eq!(parse_download_percent("[info] Writing video metadata"), None);
        assert_eq!(parse_download_percent("some 50% unrelated text"), None);
    }

    #[test]
    fn parse_download_percent_handles_malformed_percent_gracefully() {
        assert_eq!(parse_download_percent("[download]  abc% of 1MiB"), None);
    }

    #[test]
    fn is_supported_audio_ext_matches_known_extensions_only() {
        assert!(is_supported_audio_ext("mp3"));
        assert!(is_supported_audio_ext("opus"));
        assert!(!is_supported_audio_ext("exe"));
        assert!(!is_supported_audio_ext(""));
    }

    #[test]
    fn percent_encode_query_keeps_safe_chars_and_encodes_rest() {
        assert_eq!(percent_encode_query("abc-_.~"), "abc-_.~");
        assert_eq!(percent_encode_query("a b"), "a%20b");
        assert_eq!(percent_encode_query("Imagine Dragons"), "Imagine%20Dragons");
    }

    #[test]
    fn percent_encode_query_encodes_non_ascii_as_utf8_bytes() {
        // "ё" in UTF-8 is two bytes: 0xD1 0x91.
        let encoded = percent_encode_query("ё");
        assert_eq!(encoded, "%D1%91");
    }

    #[test]
    fn strip_artist_prefix_removes_leading_artist_and_dash() {
        assert_eq!(
            strip_artist_prefix("Rick Astley - Never Gonna Give You Up", "Rick Astley"),
            Some("Never Gonna Give You Up".to_string())
        );
    }

    #[test]
    fn strip_artist_prefix_is_case_insensitive_and_accepts_em_dash() {
        assert_eq!(
            strip_artist_prefix("RICK ASTLEY — Never Gonna Give You Up", "Rick Astley"),
            Some("Never Gonna Give You Up".to_string())
        );
    }

    #[test]
    fn strip_artist_prefix_requires_a_dash_right_after_the_artist_name() {
        // "Rick" is a prefix of "Rickroll", but there's no separator after it.
        assert_eq!(strip_artist_prefix("Rickroll Video", "Rick"), None);
    }

    #[test]
    fn strip_artist_prefix_returns_none_for_empty_artist_or_no_match() {
        assert_eq!(strip_artist_prefix("Some Video Title", ""), None);
        assert_eq!(strip_artist_prefix("Some Video Title", "Other Artist"), None);
    }

    #[test]
    fn strip_artist_prefix_returns_none_when_nothing_meaningful_remains() {
        assert_eq!(strip_artist_prefix("Rick Astley -   ", "Rick Astley"), None);
    }

    #[test]
    fn is_reserved_windows_name_matches_device_names_case_insensitively() {
        for name in ["CON", "con", "PRN", "AUX", "NUL", "COM1", "com9", "LPT1", "lpt9"] {
            assert!(is_reserved_windows_name(name), "{name} should be reserved");
        }
    }

    #[test]
    fn is_reserved_windows_name_rejects_normal_and_lookalike_names() {
        assert!(!is_reserved_windows_name("Console"));
        assert!(!is_reserved_windows_name("COM"));
        assert!(!is_reserved_windows_name("COM0")); // COM0 isn't reserved (digit must be 1-9)
        assert!(!is_reserved_windows_name("COM10")); // longer than 4 chars
        assert!(!is_reserved_windows_name("Imagine Dragons - Believer"));
    }

    #[test]
    fn sanitize_replaces_forbidden_windows_chars_and_collapses_whitespace() {
        assert_eq!(sanitize("AC/DC: Back   in   Black"), "AC DC Back in Black");
        assert_eq!(sanitize("  ..hello..  "), "hello");
    }

    #[test]
    fn sanitize_truncates_to_120_chars() {
        let long = "a".repeat(500);
        assert_eq!(sanitize(&long).chars().count(), 120);
    }

    #[test]
    fn unique_stem_combines_artist_and_title() {
        let dir = std::env::temp_dir().join(format!(
            "lfmusic_unique_stem_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let stem = unique_stem(&dir, "Queen", "Bohemian Rhapsody");
        assert_eq!(stem, "Queen - Bohemian Rhapsody");

        // Create a file with this name — the repeat call must uniquify.
        std::fs::write(dir.join(format!("{stem}.mp3")), b"x").unwrap();
        let stem2 = unique_stem(&dir, "Queen", "Bohemian Rhapsody");
        assert_eq!(stem2, "Queen - Bohemian Rhapsody (2)");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_stem_falls_back_to_track_when_both_parts_are_fully_sanitized_away() {
        let dir = std::env::temp_dir().join(format!(
            "lfmusic_unique_stem_empty_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // Artist and title consist only of forbidden characters — both become
        // empty after sanitize.
        let stem = unique_stem(&dir, "///", ":::");
        assert_eq!(stem, "track");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_stem_appends_underscore_for_reserved_windows_name() {
        let dir = std::env::temp_dir().join(format!(
            "lfmusic_unique_stem_reserved_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // Only artist is set and title is empty after sanitize -> base = "CON".
        let stem = unique_stem(&dir, "CON", ":::");
        assert_eq!(stem, "CON_");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lexical_normalize_resolves_parent_dir_without_touching_filesystem() {
        let p = Path::new("/music/downloads/../../etc/passwd");
        assert_eq!(lexical_normalize(p), PathBuf::from("/etc/passwd"));
    }

    #[test]
    fn is_within_downloads_blocks_parent_dir_traversal_for_nonexistent_file() {
        // File doesn't exist -> canonicalize fails -> falls back to lexical
        // normalization. A path with `..` escaping the downloads folder must be
        // blocked — this guards against path traversal when deleting tracks.
        let downloads_dir = Path::new("C:/Users/test/Music/LFMusic Downloads");
        let traversal = Path::new("C:/Users/test/Music/LFMusic Downloads/../../Desktop/x.flac");
        assert!(!is_within_downloads(downloads_dir, traversal));
    }

    #[test]
    fn is_within_downloads_allows_direct_child_for_nonexistent_file() {
        let downloads_dir = Path::new("C:/Users/test/Music/LFMusic Downloads");
        let inside = Path::new("C:/Users/test/Music/LFMusic Downloads/Artist - Title.flac");
        assert!(is_within_downloads(downloads_dir, inside));
    }

    #[test]
    fn same_path_uses_lexical_fallback_for_nonexistent_paths() {
        let a = Path::new("C:/music/../music/track.flac");
        let b = Path::new("C:/music/track.flac");
        assert!(same_path(a, b));

        let c = Path::new("C:/music/other.flac");
        assert!(!same_path(a, c));
    }
}
