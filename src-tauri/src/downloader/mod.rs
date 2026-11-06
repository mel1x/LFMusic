//! Track downloader: catalog search results (Deezer) or pasted links are
//! turned into audio files via yt-dlp, tagged, and imported into the library.
//!
//! Pipeline per job:
//! 1. **Find a source** — search YouTube (then SoundCloud as a fallback) and
//!    pick the hit that really is the requested recording ([`matching`]),
//!    skipping covers/live versions and checking the duration.
//! 2. **Download** the best audio-only stream into a private temp folder,
//!    retrying transient network failures; unavailable/age-gated hits fall
//!    through to the next candidate, a YouTube bot-check to the next source.
//! 3. **Post-process** — convert to m4a if the player can't decode the stream,
//!    write tags/cover/lyrics ([`postprocess`]).
//! 4. **Import** — move to `Artist - Title.ext` in the downloads folder and
//!    UPSERT that one file into the library.
//!
//! Queue state is pushed to the frontend as a full snapshot on the
//! `downloads_queue` event; tool status on `downloads_tools`.

mod matching;
mod postprocess;
pub mod tools;
mod ytdlp;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager as _};

use matching::{Candidate, Target};
use tools::{Resolved, Tool, ToolsInfo};
use ytdlp::{Control, Cookies, NetOptions, Source};

/// Parallel downloads. Two keeps a batch moving while one job is searching
/// or tagging, without looking like a scraper to YouTube.
const MAX_WORKERS: usize = 2;
/// Candidates tried per source before moving on.
const CANDIDATES_PER_SOURCE: usize = 3;
const AUTO_UPDATE_INTERVAL_SECS: u64 = 24 * 60 * 60;
const QUEUE_FILE: &str = "downloads.json";
const TEMP_DIR: &str = ".lfm-tmp";

// ── Public types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    ToolMissing,
    NotFound,
    BotCheck,
    Cookies,
    AuthRequired,
    Unavailable,
    RateLimited,
    Network,
    Format,
    Outdated,
    Disk,
    Import,
    Cancelled,
    Unknown,
}

impl ErrorKind {
    /// Which failure to report when several sources failed differently —
    /// the one the user can act on wins.
    fn priority(self) -> u8 {
        match self {
            ErrorKind::Cookies => 12,
            ErrorKind::ToolMissing => 11,
            ErrorKind::Disk => 10,
            ErrorKind::BotCheck => 9,
            ErrorKind::Outdated => 8,
            ErrorKind::RateLimited => 7,
            ErrorKind::Network => 6,
            ErrorKind::AuthRequired => 5,
            ErrorKind::Format => 4,
            ErrorKind::Unavailable => 3,
            ErrorKind::NotFound => 2,
            ErrorKind::Unknown | ErrorKind::Import => 1,
            ErrorKind::Cancelled => 0,
        }
    }

    fn is_transient(self) -> bool {
        matches!(self, ErrorKind::Network | ErrorKind::RateLimited)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobError {
    pub kind: ErrorKind,
    pub message: String,
}

impl JobError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "Отменено")
    }
}

/// What the frontend asks to download.
#[derive(Debug, Clone, Deserialize)]
pub struct DownloadRequest {
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_secs: Option<u32>,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub album_id: Option<u64>,
    #[serde(default)]
    pub genre: Option<String>,
    /// Pasted link: download exactly this URL instead of searching.
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    /// Queue even if the same track already finished/is in the library.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Queued,
    Searching,
    Downloading,
    Processing,
    Done,
    Error,
}

impl Stage {
    fn is_active(self) -> bool {
        matches!(self, Stage::Searching | Stage::Downloading | Stage::Processing)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadItem {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_secs: Option<u32>,
    pub cover_url: Option<String>,
    pub album_id: Option<u64>,
    pub genre: Option<String>,
    pub source_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub stage: Stage,
    /// 0..1 within the download step.
    pub progress: f64,
    pub speed_bps: Option<f64>,
    pub eta_secs: Option<u64>,
    /// "YouTube" / "SoundCloud" / link host, once known.
    pub source: Option<String>,
    /// The page actually downloaded.
    pub media_url: Option<String>,
    pub error: Option<JobError>,
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueSnapshot {
    pub items: Vec<DownloadItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolProgress {
    pub tool: Tool,
    pub message: String,
    pub fraction: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ToolsState {
    pub info: ToolsInfo,
    /// An install/update/check is running.
    pub busy: bool,
    pub progress: Option<ToolProgress>,
    pub error: Option<String>,
}

/// One entry behind a pasted link, shaped like a catalog search hit.
#[derive(Debug, Clone, Serialize)]
pub struct LinkTrack {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_secs: Option<u32>,
    pub source_url: String,
    pub thumbnail_url: Option<String>,
    pub in_library: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkResult {
    /// Playlist/album title when the link had several entries.
    pub collection: Option<String>,
    pub tracks: Vec<LinkTrack>,
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Manager {
    items: Vec<DownloadItem>,
    next_id: u64,
    loaded: bool,
    workers: usize,
    /// Auto-update running before the batch starts — workers wait for it.
    updating: bool,
    last_auto_update_attempt: Option<Instant>,
    controls: HashMap<u64, Arc<Control>>,
    /// Per-job download fraction, for the titlebar's combined progress.
    fractions: HashMap<u64, f64>,
    batch_total: usize,
    batch_done: usize,
    last_progress_emit: Option<Instant>,
    resolved: Option<Resolved>,
    tools: ToolsState,
}

#[derive(Default)]
pub struct DownloaderState(Mutex<Manager>);

fn with<R>(app: &AppHandle, f: impl FnOnce(&mut Manager) -> R) -> R {
    let state = app.state::<DownloaderState>();
    let mut guard = state.0.lock().unwrap_or_else(|e| e.into_inner());
    if !guard.loaded {
        guard.loaded = true;
        load_queue(app, &mut guard);
    }
    f(&mut guard)
}

fn data_dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn settings(app: &AppHandle) -> crate::settings::DownloaderSettings {
    app.state::<crate::SettingsState>()
        .0
        .lock()
        .map(|g| g.0.downloader())
        .unwrap_or_default()
}

// ── Persistence ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct SavedQueue {
    next_id: u64,
    items: Vec<DownloadItem>,
}

/// Restores unfinished/failed jobs from the last session. Jobs that were
/// mid-flight are re-queued from scratch (their temp folder is gone).
fn load_queue(app: &AppHandle, m: &mut Manager) {
    cleanup_legacy_files(app);
    // No worker can be running yet: anything here is left over from a crash.
    let _ = std::fs::remove_dir_all(downloads_dir(app).join(TEMP_DIR));
    let path = data_dir(app).join(QUEUE_FILE);
    let saved: SavedQueue = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    m.next_id = saved.next_id.max(1);
    for mut item in saved.items {
        if item.stage == Stage::Done {
            continue;
        }
        if item.stage.is_active() {
            item.stage = Stage::Queued;
        }
        item.progress = 0.0;
        item.speed_bps = None;
        item.eta_secs = None;
        m.next_id = m.next_id.max(item.id + 1);
        m.items.push(item);
    }
}

fn save_queue(app: &AppHandle, m: &Manager) {
    let saved = SavedQueue {
        next_id: m.next_id,
        items: m.items.iter().filter(|i| i.stage != Stage::Done).cloned().collect(),
    };
    let path = data_dir(app).join(QUEUE_FILE);
    if let Ok(json) = serde_json::to_string(&saved) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Files left behind by the previous downloader implementation.
fn cleanup_legacy_files(app: &AppHandle) {
    let dir = data_dir(app);
    for name in [
        "download_queue.json",
        "download_attempts.jsonl",
        "youtube_cookies.txt",
        "youtube_cookies_invalid",
    ] {
        let _ = std::fs::remove_file(dir.join(name));
    }
    let _ = std::fs::remove_dir_all(dir.join("browser-extension"));
    let bin = tools::bin_dir(&dir);
    let _ = std::fs::remove_dir_all(bin.join("yt-dlp-plugins"));
    for name in ["bgutil-pot", "bgutil-pot.exe", "tools_meta.json"] {
        let _ = std::fs::remove_file(bin.join(name));
    }
}

// ── Events ──────────────────────────────────────────────────────────────────

fn emit_queue(app: &AppHandle) {
    let snap = with(app, |m| QueueSnapshot { items: m.items.clone() });
    let _ = app.emit("downloads_queue", snap);
}

fn emit_tools(app: &AppHandle) {
    let state = with(app, |m| m.tools.clone());
    let _ = app.emit("downloads_tools", state);
}

fn report_titlebar(app: &AppHandle) {
    let (done, total, frac) = with(app, |m| {
        (m.batch_done, m.batch_total, m.fractions.values().sum::<f64>())
    });
    crate::report_download_progress_frac(app, done, total, frac.min(1.0));
}

pub fn snapshot(app: &AppHandle) -> QueueSnapshot {
    let snap = with(app, |m| QueueSnapshot { items: m.items.clone() });
    // Resume whatever was left queued last session.
    pump(app);
    snap
}

// ── Queue operations ────────────────────────────────────────────────────────

fn identity(artist: &str, title: &str, album: Option<&str>, url: Option<&str>) -> String {
    match url {
        Some(u) => format!("url:{}", u.trim()),
        None => crate::external::track_identity_key(artist, title, album),
    }
}

fn item_identity(i: &DownloadItem) -> String {
    identity(&i.artist, &i.title, i.album.as_deref(), i.source_url.as_deref())
}

/// Queues tracks; returns their item ids (an existing id for duplicates).
pub fn enqueue(app: &AppHandle, requests: Vec<DownloadRequest>) -> Vec<u64> {
    let ids = with(app, |m| {
        let mut ids = Vec::with_capacity(requests.len());
        for req in requests {
            let title = req.title.trim().to_string();
            let artist = req.artist.trim().to_string();
            if title.is_empty() && req.source_url.is_none() {
                continue;
            }
            let key = identity(&artist, &title, req.album.as_deref(), req.source_url.as_deref());
            if let Some(existing) = m.items.iter_mut().find(|i| item_identity(i) == key) {
                match existing.stage {
                    Stage::Error => {
                        reset_for_retry(existing);
                        m.batch_total += 1;
                        ids.push(existing.id);
                        continue;
                    }
                    Stage::Done if req.force => {}
                    _ => {
                        ids.push(existing.id);
                        continue;
                    }
                }
            }
            let id = m.next_id.max(1);
            m.next_id = id + 1;
            m.items.push(DownloadItem {
                id,
                title,
                artist,
                album: req.album.filter(|s| !s.trim().is_empty()),
                duration_secs: req.duration_secs,
                cover_url: req.cover_url,
                album_id: req.album_id,
                genre: req.genre,
                source_url: req.source_url,
                thumbnail_url: req.thumbnail_url,
                stage: Stage::Queued,
                progress: 0.0,
                speed_bps: None,
                eta_secs: None,
                source: None,
                media_url: None,
                error: None,
                file_path: None,
            });
            m.batch_total += 1;
            ids.push(id);
        }
        save_queue(app, m);
        ids
    });
    emit_queue(app);
    report_titlebar(app);
    pump(app);
    ids
}

fn reset_for_retry(item: &mut DownloadItem) {
    item.stage = Stage::Queued;
    item.progress = 0.0;
    item.speed_bps = None;
    item.eta_secs = None;
    item.error = None;
    item.source = None;
    item.media_url = None;
}

/// Cancels an active job or removes a finished one from the list.
pub fn cancel(app: &AppHandle, id: u64) {
    // Killing the process tree takes a moment — don't hold the queue lock for it.
    if let Some(control) = with(app, |m| m.controls.get(&id).cloned()) {
        control.cancel();
    }
    with(app, |m| {
        if let Some(pos) = m.items.iter().position(|i| i.id == id) {
            let stage = m.items[pos].stage;
            m.items.remove(pos);
            if stage == Stage::Queued || stage.is_active() {
                m.batch_total = m.batch_total.saturating_sub(1);
            }
        }
        m.fractions.remove(&id);
        save_queue(app, m);
    });
    emit_queue(app);
    report_titlebar(app);
}

pub fn cancel_all(app: &AppHandle) {
    let ids: Vec<u64> = with(app, |m| {
        m.items
            .iter()
            .filter(|i| i.stage == Stage::Queued || i.stage.is_active())
            .map(|i| i.id)
            .collect()
    });
    for id in ids {
        cancel(app, id);
    }
}

pub fn retry(app: &AppHandle, ids: Option<Vec<u64>>) {
    with(app, |m| {
        for item in m.items.iter_mut() {
            let wanted = ids.as_ref().map(|v| v.contains(&item.id)).unwrap_or(true);
            if wanted && item.stage == Stage::Error {
                reset_for_retry(item);
                m.batch_total += 1;
            }
        }
        save_queue(app, m);
    });
    emit_queue(app);
    pump(app);
}

/// Drops finished (done + failed) items from the list.
pub fn clear_finished(app: &AppHandle) {
    with(app, |m| {
        m.items.retain(|i| !matches!(i.stage, Stage::Done | Stage::Error));
        save_queue(app, m);
    });
    emit_queue(app);
}

pub fn has_active_jobs(app: &AppHandle) -> bool {
    with(app, |m| m.items.iter().any(|i| i.stage.is_active()) || m.workers > 0)
}

/// Kills running yt-dlp/ffmpeg processes; called on app exit.
pub fn shutdown(app: &AppHandle) {
    let Some(state) = app.try_state::<DownloaderState>() else { return };
    let controls: Vec<Arc<Control>> = state
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .controls
        .values()
        .cloned()
        .collect();
    for control in controls {
        control.cancel();
    }
}

// ── Worker pool ─────────────────────────────────────────────────────────────

/// Starts workers for queued jobs (up to [`MAX_WORKERS`]), running the daily
/// tool auto-update first when a new batch begins.
fn pump(app: &AppHandle) {
    if !tools::platform_supported() {
        with(app, |m| {
            for item in m.items.iter_mut().filter(|i| i.stage == Stage::Queued) {
                item.stage = Stage::Error;
                item.error = Some(JobError::new(ErrorKind::ToolMissing, "Скачивание недоступно на этой платформе"));
            }
        });
        emit_queue(app);
        return;
    }
    enum Next {
        Nothing,
        Update,
        Spawn(usize),
    }
    let next = with(app, |m| {
        let queued = m.items.iter().filter(|i| i.stage == Stage::Queued).count();
        if queued == 0 || m.updating {
            return Next::Nothing;
        }
        if m.workers == 0 && auto_update_due(app, m) {
            m.updating = true;
            m.last_auto_update_attempt = Some(Instant::now());
            return Next::Update;
        }
        let n = MAX_WORKERS.saturating_sub(m.workers).min(queued);
        m.workers += n;
        Next::Spawn(n)
    });
    match next {
        Next::Nothing => {}
        Next::Update => {
            let app = app.clone();
            std::thread::spawn(move || {
                auto_update(&app);
                with(&app, |m| m.updating = false);
                pump(&app);
            });
        }
        Next::Spawn(n) => {
            for _ in 0..n {
                let worker_app = app.clone();
                let spawned = std::thread::Builder::new()
                    .name("download-worker".into())
                    .spawn(move || worker_loop(worker_app));
                if spawned.is_err() {
                    with(app, |m| m.workers -= 1);
                }
            }
        }
    }
}

fn auto_update_due(app: &AppHandle, m: &Manager) -> bool {
    if !settings(app).auto_update || !tools::is_managed(&data_dir(app), Tool::Ytdlp) {
        return false;
    }
    // Once per session window even when offline, so a failing check doesn't
    // delay every batch.
    if m.last_auto_update_attempt
        .map(|t| t.elapsed() < Duration::from_secs(6 * 60 * 60))
        .unwrap_or(false)
    {
        return false;
    }
    tools::secs_since_update_check(&data_dir(app)) >= AUTO_UPDATE_INTERVAL_SECS
}

/// Updates app-managed yt-dlp/Deno when a newer release exists. yt-dlp has to
/// track YouTube's changes closely — a weeks-old build is the most common
/// reason downloads stop working.
fn auto_update(app: &AppHandle) {
    let dir = data_dir(app);
    let Ok(info) = tools::check_updates(&dir) else { return };
    let outdated: Vec<Tool> = [(Tool::Ytdlp, &info.ytdlp), (Tool::Deno, &info.deno)]
        .into_iter()
        .filter(|(_, t)| t.managed && t.update_available)
        .map(|(tool, _)| tool)
        .collect();
    if outdated.is_empty() {
        with(app, |m| m.tools.info = info);
        emit_tools(app);
        return;
    }
    let _ = run_install(app, outdated);
}

/// Takes the next queued job; when there is none, retires this worker under
/// the same lock (so an `enqueue` racing with it always sees a free slot).
fn take_job(app: &AppHandle) -> Option<(DownloadItem, Arc<Control>)> {
    with(app, |m| {
        let Some(item) = m.items.iter_mut().find(|i| i.stage == Stage::Queued) else {
            m.workers -= 1;
            return None;
        };
        item.stage = if item.source_url.is_some() { Stage::Downloading } else { Stage::Searching };
        let job = item.clone();
        let control = Arc::new(Control::default());
        m.controls.insert(job.id, control.clone());
        m.fractions.insert(job.id, 0.0);
        Some((job, control))
    })
}

fn worker_loop(app: AppHandle) {
    while let Some((job, control)) = take_job(&app) {
        emit_queue(&app);
        let result = run_job(&app, &job, &control);
        finish_job(&app, job.id, result);
    }
    let idle = with(&app, |m| {
        m.workers == 0 && !m.items.iter().any(|i| i.stage == Stage::Queued || i.stage.is_active())
    });
    if idle {
        // Analyze the new tracks' audio features once per batch, not per track.
        if let Ok(lib) = app.state::<crate::LibraryState>().0.lock() {
            let _ = lib.resume_pending_audio_feature_analysis(None);
        }
        let (done, total) = with(&app, |m| {
            let r = (m.batch_done, m.batch_total);
            m.batch_done = 0;
            m.batch_total = 0;
            m.fractions.clear();
            r
        });
        crate::report_download_progress(&app, total.max(done), total.max(done));
    }
}

fn finish_job(app: &AppHandle, id: u64, result: Result<PathBuf, JobError>) {
    with(app, |m| {
        m.controls.remove(&id);
        m.fractions.remove(&id);
        // Removed by the user meanwhile (cancel) — nothing to record.
        let Some(item) = m.items.iter_mut().find(|i| i.id == id) else { return };
        item.speed_bps = None;
        item.eta_secs = None;
        match result {
            Ok(path) => {
                item.stage = Stage::Done;
                item.progress = 1.0;
                item.file_path = Some(path.to_string_lossy().into_owned());
            }
            Err(e) if e.kind == ErrorKind::Cancelled => {
                let id = item.id;
                m.items.retain(|i| i.id != id);
            }
            Err(e) => {
                eprintln!("[downloader] job {} failed ({:?}): {}", id, e.kind, e.message);
                item.stage = Stage::Error;
                item.error = Some(e);
            }
        }
        m.batch_done += 1;
        save_queue(app, m);
    });
    emit_queue(app);
    report_titlebar(app);
}

// ── Job execution ───────────────────────────────────────────────────────────

fn update_item(app: &AppHandle, id: u64, f: impl FnOnce(&mut DownloadItem)) {
    with(app, |m| {
        if let Some(item) = m.items.iter_mut().find(|i| i.id == id) {
            f(item);
        }
    });
    emit_queue(app);
}

fn resolved_tools(app: &AppHandle) -> Result<Resolved, JobError> {
    if let Some(r) = with(app, |m| m.resolved.clone()) {
        return Ok(r);
    }
    let resolved = tools::resolve(&data_dir(app)).ok_or_else(|| {
        JobError::new(
            ErrorKind::ToolMissing,
            "yt-dlp не установлен - установите компоненты для скачивания",
        )
    })?;
    with(app, |m| m.resolved = Some(resolved.clone()));
    Ok(resolved)
}

fn net_options(app: &AppHandle) -> NetOptions {
    let s = settings(app);
    let proxy = match s.proxy_mode.as_str() {
        "none" => Some(String::new()),
        "custom" if !s.proxy_url.trim().is_empty() => Some(s.proxy_url.trim().to_string()),
        _ => None,
    };
    let cookies = match s.cookies_mode.as_str() {
        "browser" if !s.cookies_browser.trim().is_empty() => {
            Some(Cookies::Browser(s.cookies_browser.trim().to_string()))
        }
        "file" if Path::new(s.cookies_file.trim()).is_file() => {
            Some(Cookies::File(PathBuf::from(s.cookies_file.trim())))
        }
        _ => None,
    };
    NetOptions { proxy, cookies }
}

/// Per-job scratch folder inside the downloads dir (same volume, so the
/// final move is a cheap rename). Removed on drop, whatever happened.
struct WorkDir(PathBuf);

impl WorkDir {
    fn create(downloads: &Path, id: u64) -> Result<WorkDir, JobError> {
        let dir = downloads.join(TEMP_DIR).join(format!("job-{}", id));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| {
            JobError::new(ErrorKind::Disk, format!("Не удалось создать временную папку: {}", e))
        })?;
        Ok(WorkDir(dir))
    }

    /// Empties the folder between attempts.
    fn clear(&self) {
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for e in entries.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        // Drop the parent too once the last job is gone.
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

fn run_job(app: &AppHandle, job: &DownloadItem, control: &Control) -> Result<PathBuf, JobError> {
    let tools = resolved_tools(app)?;
    let net = net_options(app);
    let downloads = downloads_dir(app);
    let work = WorkDir::create(&downloads, job.id)?;

    let (file, source, media_url) = match &job.source_url {
        Some(url) => {
            let label = link_label(url);
            update_item(app, job.id, |i| i.source = Some(label.clone()));
            let file = download_with_retries(app, job.id, &tools, &net, url, &work, control)?;
            (file, label, url.clone())
        }
        None => find_and_download(app, job, &tools, &net, &work, control)?,
    };
    control.check()?;
    update_item(app, job.id, |i| {
        i.stage = Stage::Processing;
        i.source = Some(source);
        i.media_url = Some(media_url);
        i.speed_bps = None;
        i.eta_secs = None;
    });

    let (playable, ext) = postprocess::ensure_playable(&file, &tools, control)?;
    control.check()?;
    let lyrics = postprocess::write_tags(
        &playable,
        &postprocess::TagInfo {
            title: &job.title,
            artist: &job.artist,
            album: job.album.as_deref(),
            genre: job.genre.as_deref(),
            album_id: job.album_id,
            cover_url: job.cover_url.as_deref(),
            thumbnail_url: job.thumbnail_url.as_deref(),
        },
    );
    control.check()?;

    let target = unique_path(&downloads, &job.artist, &job.title, ext);
    // `target` is an empty placeholder reserved by unique_path; rename replaces it.
    std::fs::rename(&playable, &target)
        .or_else(|_| std::fs::copy(&playable, &target).map(|_| ()))
        .map_err(|e| {
            let _ = std::fs::remove_file(&target);
            JobError::new(ErrorKind::Disk, format!("Не удалось сохранить файл: {}", e))
        })?;
    postprocess::write_lrc(&target, lyrics.as_ref());

    import_track(app, &downloads, &target)
        .map_err(|e| JobError::new(ErrorKind::Import, format!("Файл скачан, но не добавлен в библиотеку: {}", e)))?;
    Ok(target)
}

fn link_label(url: &str) -> String {
    let host = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("")
        .trim_start_matches("www.")
        .trim_start_matches("m.")
        .trim_start_matches("music.");
    match host {
        h if h.contains("youtube") || h == "youtu.be" => "YouTube".into(),
        h if h.contains("soundcloud") => "SoundCloud".into(),
        h if h.contains("bandcamp") => "Bandcamp".into(),
        "" => "Ссылка".into(),
        h => h.to_string(),
    }
}

fn keep_most_actionable(slot: &mut Option<JobError>, e: JobError) {
    if slot.as_ref().map(|cur| e.kind.priority() > cur.kind.priority()).unwrap_or(true) {
        *slot = Some(e);
    }
}

fn find_and_download(
    app: &AppHandle,
    job: &DownloadItem,
    tools: &Resolved,
    net: &NetOptions,
    work: &WorkDir,
    control: &Control,
) -> Result<(PathBuf, String, String), JobError> {
    let target = Target {
        title: &job.title,
        artist: &job.artist,
        duration_secs: job.duration_secs,
    };
    let query = format!("{} - {}", job.artist, job.title);
    let mut failure: Option<JobError> = None;

    for source in [Source::YouTube, Source::SoundCloud] {
        control.check()?;
        update_item(app, job.id, |i| {
            i.stage = Stage::Searching;
            i.source = Some(source.label().into());
            i.progress = 0.0;
        });
        let candidates = match search_with_retries(tools, net, source, &query, control) {
            Ok(c) => c,
            Err(e) if e.kind == ErrorKind::Cancelled => return Err(e),
            Err(e) => {
                let fatal = matches!(e.kind, ErrorKind::Cookies | ErrorKind::ToolMissing);
                keep_most_actionable(&mut failure, e);
                if fatal {
                    break;
                }
                continue;
            }
        };
        let ranked: Vec<Candidate> = matching::rank(&target, &candidates)
            .into_iter()
            .take(CANDIDATES_PER_SOURCE)
            .cloned()
            .collect();
        if ranked.is_empty() {
            keep_most_actionable(
                &mut failure,
                JobError::new(ErrorKind::NotFound, format!("На {} нет подходящей записи", source.label())),
            );
            continue;
        }
        for cand in ranked {
            control.check()?;
            update_item(app, job.id, |i| {
                i.stage = Stage::Downloading;
                i.media_url = Some(cand.url.clone());
            });
            match download_with_retries(app, job.id, tools, net, &cand.url, work, control) {
                Ok(file) => return Ok((file, source.label().into(), cand.url)),
                Err(e) if e.kind == ErrorKind::Cancelled => return Err(e),
                Err(e) => {
                    work.clear();
                    // This particular upload is gone/gated — another one may work.
                    let next_candidate =
                        matches!(e.kind, ErrorKind::Unavailable | ErrorKind::AuthRequired | ErrorKind::Format);
                    let fatal = matches!(e.kind, ErrorKind::Cookies | ErrorKind::Disk | ErrorKind::ToolMissing);
                    keep_most_actionable(&mut failure, e);
                    if fatal {
                        return Err(failure.unwrap_or_else(|| JobError::new(ErrorKind::Unknown, "Ошибка")));
                    }
                    if !next_candidate {
                        break;
                    }
                }
            }
        }
    }
    Err(failure.unwrap_or_else(|| JobError::new(ErrorKind::NotFound, "Не удалось найти трек")))
}

fn search_with_retries(
    tools: &Resolved,
    net: &NetOptions,
    source: Source,
    query: &str,
    control: &Control,
) -> Result<Vec<Candidate>, JobError> {
    let mut attempt = 0;
    loop {
        match ytdlp::search(tools, net, source, query, 8, control) {
            Err(e) if e.kind.is_transient() && attempt < 1 => {
                attempt += 1;
                control.sleep(Duration::from_secs(3))?;
            }
            other => return other,
        }
    }
}

fn download_with_retries(
    app: &AppHandle,
    id: u64,
    tools: &Resolved,
    net: &NetOptions,
    url: &str,
    work: &WorkDir,
    control: &Control,
) -> Result<PathBuf, JobError> {
    let mut attempt = 0u64;
    loop {
        let mut on_progress = |p: ytdlp::Progress| on_progress(app, id, p);
        match ytdlp::download(tools, net, url, &work.0, control, &mut on_progress) {
            Err(e) if e.kind.is_transient() && attempt < 2 => {
                attempt += 1;
                work.clear();
                control.sleep(Duration::from_secs(3 * attempt * attempt))?;
            }
            other => return other,
        }
    }
}

/// Progress updates arrive many times a second; the queue snapshot is
/// re-emitted at most every 200 ms.
fn on_progress(app: &AppHandle, id: u64, p: ytdlp::Progress) {
    let emit = with(app, |m| {
        if let Some(item) = m.items.iter_mut().find(|i| i.id == id) {
            if let Some(f) = p.fraction {
                item.progress = f;
                m.fractions.insert(id, f);
            }
            item.speed_bps = p.speed_bps;
            item.eta_secs = p.eta_secs;
        }
        let due = m
            .last_progress_emit
            .map(|t| t.elapsed() >= Duration::from_millis(200))
            .unwrap_or(true);
        if due {
            m.last_progress_emit = Some(Instant::now());
        }
        due
    });
    if emit {
        emit_queue(app);
        report_titlebar(app);
    }
}

// ── Links ───────────────────────────────────────────────────────────────────

/// Probes a pasted link (single video or playlist) for its tracks.
pub fn resolve_link(app: &AppHandle, url: &str) -> Result<LinkResult, String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Это не ссылка".into());
    }
    let tools = resolved_tools(app).map_err(|e| e.message)?;
    let control = Control::default();
    let (collection, entries) = ytdlp::probe_link(&tools, &net_options(app), url, &control)
        .map_err(|e| link_error_message(&e))?;
    let tracks = entries
        .into_iter()
        .map(|e| LinkTrack {
            title: e.title,
            artist: e.artist,
            album: e.album,
            duration_secs: e.duration_secs,
            source_url: e.url,
            thumbnail_url: e.thumbnail,
            in_library: false,
        })
        .collect();
    Ok(LinkResult { collection, tracks })
}

fn link_error_message(e: &JobError) -> String {
    match e.kind {
        ErrorKind::Unavailable => "Видео недоступно или ссылка не поддерживается".into(),
        ErrorKind::BotCheck => "YouTube просит подтвердить, что вы не бот - подключите cookies в настройках".into(),
        ErrorKind::Network => "Нет соединения с сайтом - проверьте интернет или прокси".into(),
        _ => e.message.clone(),
    }
}

// ── Tools ───────────────────────────────────────────────────────────────────

pub fn tools_state(app: &AppHandle) -> ToolsState {
    let info = tools::info(&data_dir(app));
    with(app, |m| {
        m.tools.info = info;
        m.resolved = None;
        m.tools.clone()
    })
}

pub fn check_updates(app: &AppHandle) -> Result<ToolsState, String> {
    let busy = with(app, |m| std::mem::replace(&mut m.tools.busy, true));
    if busy {
        return Ok(with(app, |m| m.tools.clone()));
    }
    emit_tools(app);
    let result = tools::check_updates(&data_dir(app));
    with(app, |m| {
        m.tools.busy = false;
        match &result {
            Ok(info) => {
                m.tools.info = info.clone();
                m.tools.error = None;
            }
            Err(e) => m.tools.error = Some(format!("Не удалось проверить обновления: {}", e)),
        }
    });
    emit_tools(app);
    result.map(|_| with(app, |m| m.tools.clone()))
}

/// Installs the given tools; `None` = everything missing or outdated.
pub fn install(app: &AppHandle, which: Option<Vec<Tool>>) -> Result<ToolsState, String> {
    let list = which.unwrap_or_else(|| {
        let info = tools::info(&data_dir(app));
        Tool::ALL
            .into_iter()
            .filter(|t| {
                let i = match t {
                    Tool::Ytdlp => &info.ytdlp,
                    Tool::Deno => &info.deno,
                    Tool::Ffmpeg => &info.ffmpeg,
                };
                !i.installed || (i.managed && i.update_available)
            })
            .collect()
    });
    run_install(app, list)?;
    Ok(with(app, |m| m.tools.clone()))
}

fn run_install(app: &AppHandle, list: Vec<Tool>) -> Result<(), String> {
    let busy = with(app, |m| std::mem::replace(&mut m.tools.busy, true));
    if busy {
        return Err("Установка уже идёт".into());
    }
    with(app, |m| m.tools.error = None);
    emit_tools(app);
    let dir = data_dir(app);
    let progress_app = app.clone();
    let progress = move |tool: Tool, message: String, fraction: Option<f64>| {
        let emit = with(&progress_app, |m| {
            m.tools.progress = Some(ToolProgress { tool, message, fraction });
            let due = m
                .last_progress_emit
                .map(|t| t.elapsed() >= Duration::from_millis(150))
                .unwrap_or(true);
            if due {
                m.last_progress_emit = Some(Instant::now());
            }
            due
        });
        if emit {
            emit_tools(&progress_app);
        }
    };
    let mut first_error: Option<String> = None;
    for tool in list {
        if let Err(e) = tools::install(&dir, tool, &progress) {
            eprintln!("[downloader] {} install failed: {}", tool.label(), e);
            first_error.get_or_insert(format!("{}: {}", tool.label(), e));
        }
    }
    let info = tools::info(&dir);
    with(app, |m| {
        let ready = info.ytdlp.installed;
        m.tools.busy = false;
        m.tools.progress = None;
        m.tools.info = info;
        m.tools.error = first_error.clone();
        m.resolved = None;
        if ready {
            for item in m.items.iter_mut() {
                if item.error.as_ref().map(|e| e.kind == ErrorKind::ToolMissing).unwrap_or(false) {
                    reset_for_retry(item);
                    m.batch_total += 1;
                }
            }
        }
    });
    emit_tools(app);
    emit_queue(app);
    // Jobs may have been waiting for the tools.
    pump(app);
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

pub fn remove_tools(app: &AppHandle) -> Result<ToolsState, String> {
    if has_active_jobs(app) {
        return Err("Дождитесь окончания скачивания".into());
    }
    let dir = data_dir(app);
    let mut errors = Vec::new();
    for tool in Tool::ALL {
        if let Err(e) = tools::remove(&dir, tool) {
            errors.push(e);
        }
    }
    let state = tools_state(app);
    emit_tools(app);
    if errors.is_empty() {
        Ok(state)
    } else {
        Err(errors.join("; "))
    }
}

// ── Settings ────────────────────────────────────────────────────────────────

pub fn get_settings(app: &AppHandle) -> crate::settings::DownloaderSettings {
    settings(app)
}

pub fn set_settings(app: &AppHandle, mut s: crate::settings::DownloaderSettings) -> Result<(), String> {
    s.proxy_url = s.proxy_url.trim().to_string();
    s.cookies_file = s.cookies_file.trim().to_string();
    if !matches!(s.proxy_mode.as_str(), "system" | "none" | "custom") {
        s.proxy_mode = "system".into();
    }
    if !matches!(s.cookies_mode.as_str(), "none" | "browser" | "file") {
        s.cookies_mode = "none".into();
    }
    if s.cookies_mode == "file" && !Path::new(&s.cookies_file).is_file() {
        return Err("Файл cookies не найден".into());
    }
    let state = app.state::<crate::SettingsState>();
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    guard.0.downloader = Some(s);
    let path = guard.1.clone();
    crate::settings::save(&path, &guard.0)
}

// ── Downloads folder ────────────────────────────────────────────────────────

fn default_downloads_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .audio_dir()
        .unwrap_or_else(|_| data_dir(app))
        .join("LFMusic Downloads")
}

fn configured_downloads_dir(app: &AppHandle) -> Option<PathBuf> {
    let state = app.state::<crate::SettingsState>();
    let guard = state.0.lock().ok()?;
    guard
        .0
        .download_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// The configured folder, or the default one if it can't be created.
pub fn downloads_dir(app: &AppHandle) -> PathBuf {
    if let Some(dir) = configured_downloads_dir(app) {
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    let dir = default_downloads_dir(app);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Saves a new downloads folder (`None` = default). Returns `(old, new)`;
/// re-registering library folders is the caller's job.
pub fn set_downloads_dir(app: &AppHandle, requested: Option<String>) -> (PathBuf, PathBuf) {
    let old = downloads_dir(app);
    let value = requested
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| std::fs::create_dir_all(s).is_ok());
    {
        let state = app.state::<crate::SettingsState>();
        let saved = state.0.lock().map(|mut guard| {
            guard.0.download_dir = value;
            let path = guard.1.clone();
            let _ = crate::settings::save(&path, &guard.0);
        });
        drop(saved);
    }
    (old, downloads_dir(app))
}

fn normalize_lexically(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => normalize_lexically(a) == normalize_lexically(b),
    }
}

/// Whether `file` lives inside `dir` (component-wise, `..`-safe).
fn is_inside(dir: &Path, file: &Path) -> bool {
    if let (Ok(d), Ok(f)) = (dir.canonicalize(), file.canonicalize()) {
        return f.starts_with(d);
    }
    let key = |p: &Path| normalize_lexically(p).to_string_lossy().replace('\\', "/").to_lowercase();
    let mut d = key(dir);
    if !d.ends_with('/') {
        d.push('/');
    }
    key(file).starts_with(&d)
}

/// Removes an app-downloaded track from disk and the library. Refuses files
/// outside the downloads folder.
pub fn delete_downloaded_track(app: &AppHandle, file_path: &str) -> Result<(), String> {
    let path = PathBuf::from(file_path);
    if !is_inside(&downloads_dir(app), &path) {
        return Err("Удалять можно только треки, скачанные приложением".into());
    }
    // The decoder holds the file open while playing; Windows won't delete it.
    if let Ok(mut player) = app.state::<crate::PlayerState>().0.write() {
        if player.current_file_path().as_deref() == Some(file_path) {
            player.stop();
        }
    }
    // The audio thread may need a moment to release the handle. The file goes
    // first: dropping the library row for a file that stays on disk would just
    // bring it back (without its statistics) on the next rescan.
    let mut last_err = None;
    for _ in 0..20 {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                last_err = Some(e);
                std::thread::sleep(Duration::from_millis(150))
            }
            _ => {
                last_err = None;
                break;
            }
        }
    }
    if let Some(e) = last_err {
        return Err(format!("Не удалось удалить файл: {}", e));
    }
    let _ = std::fs::remove_file(path.with_extension("lrc"));
    let separators = crate::settings::get_artist_separators(&app.state::<crate::SettingsState>());
    app.state::<crate::LibraryState>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .remove_track(file_path, &separators)?;
    let _ = app.emit("downloads_library_updated", ());
    Ok(())
}

fn sanitize_component(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed: String = collapsed
        .trim_matches(|c| c == '.' || c == ' ')
        .chars()
        .take(90)
        .collect();
    let upper = trimmed.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit());
    if trimmed.is_empty() || reserved {
        format!("_{}", trimmed)
    } else {
        trimmed
    }
}

/// `Artist - Title.ext`, with " (2)", " (3)"... when taken. The name is reserved by
/// creating an empty file atomically, so two workers finishing same-named tracks
/// can't pick one path (the later rename would silently overwrite on Unix).
fn unique_path(dir: &Path, artist: &str, title: &str, ext: &str) -> PathBuf {
    let base = match (artist.trim().is_empty(), title.trim().is_empty()) {
        (false, false) => format!("{} - {}", sanitize_component(artist), sanitize_component(title)),
        (true, false) => sanitize_component(title),
        (false, true) => sanitize_component(artist),
        (true, true) => "track".to_string(),
    };
    let mut candidate = dir.join(format!("{}.{}", base, ext));
    let mut n = 2;
    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                candidate = dir.join(format!("{} ({}).{}", base, n, ext));
                n += 1;
            }
            // Created, or an unrelated error the following rename will report.
            _ => return candidate,
        }
    }
}

fn import_track(app: &AppHandle, dir: &Path, file: &Path) -> Result<(), String> {
    let separators = crate::settings::get_artist_separators(&app.state::<crate::SettingsState>());
    app.state::<crate::LibraryState>()
        .0
        .lock()
        .map_err(|e| format!("библиотека занята: {}", e))?
        .insert_downloaded_track(&file.to_string_lossy(), &dir.to_string_lossy(), &separators)?;
    let _ = app.emit("downloads_library_updated", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe_and_unique() {
        let dir = std::env::temp_dir().join(format!("lfm-dl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_path(&dir, "AC/DC", "Back In Black?", "m4a");
        assert_eq!(first.file_name().unwrap().to_str().unwrap(), "AC DC - Back In Black.m4a");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir, "AC/DC", "Back In Black?", "m4a");
        assert_eq!(second.file_name().unwrap().to_str().unwrap(), "AC DC - Back In Black (2).m4a");
        assert_eq!(sanitize_component("CON"), "_CON");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn containment_check_rejects_traversal() {
        let dir = Path::new("C:/Music/LFMusic Downloads");
        assert!(is_inside(dir, Path::new("C:/Music/LFMusic Downloads/a.m4a")));
        assert!(!is_inside(dir, Path::new("C:/Music/LFMusic Downloads/../secret.m4a")));
        assert!(!is_inside(dir, Path::new("C:/Music/LFMusic Downloads-old/a.m4a")));
    }

    #[test]
    fn most_actionable_error_wins() {
        let mut slot = None;
        keep_most_actionable(&mut slot, JobError::new(ErrorKind::NotFound, "a"));
        keep_most_actionable(&mut slot, JobError::new(ErrorKind::BotCheck, "b"));
        keep_most_actionable(&mut slot, JobError::new(ErrorKind::Unavailable, "c"));
        assert_eq!(slot.unwrap().kind, ErrorKind::BotCheck);
    }

    /// End-to-end check against the real services: installs the tools into
    /// `LFM_LIVE_DIR` (or a temp folder), then searches, matches, downloads and
    /// tags a few tracks. Network-heavy, so ignored by default:
    /// `LFM_LIVE_DIR=... cargo test --lib live_pipeline -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_pipeline() {
        let dir = std::env::var("LFM_LIVE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("lfm-live"));
        std::fs::create_dir_all(&dir).unwrap();
        let progress = |t: Tool, msg: String, _f: Option<f64>| {
            if msg.contains("Подготовка") || msg.contains("Распаковка") {
                println!("[{}] {}", t.label(), msg);
            }
        };
        let info = tools::info(&dir);
        for (tool, ti) in [(Tool::Ytdlp, &info.ytdlp), (Tool::Deno, &info.deno)] {
            if !ti.installed {
                let v = tools::install(&dir, tool, &progress).expect("install");
                println!("installed {} {}", tool.label(), v);
            }
        }
        if std::env::var("LFM_LIVE_FFMPEG").is_ok() && !tools::info(&dir).ffmpeg.installed {
            tools::install(&dir, Tool::Ffmpeg, &progress).expect("ffmpeg install");
        }
        let resolved = tools::resolve(&dir).expect("resolve");
        println!("tools: {:?}", resolved);
        let net = NetOptions::default();
        let control = Control::default();
        let queries = std::env::var("LFM_LIVE_QUERIES")
            .unwrap_or_else(|_| "the weeknd blinding lights|кино группа крови|daft punk get lucky".into());
        for q in queries.split('|') {
            let page = crate::external::search_tracks(q, None, None, 0, 5);
            let track = page.tracks.first().expect("deezer hit");
            println!("\n== {} — {} ({:?}s)", track.artist, track.title, track.duration_secs);
            let target = Target {
                title: &track.title,
                artist: &track.artist,
                duration_secs: track.duration_secs,
            };
            let query = format!("{} - {}", track.artist, track.title);
            for source in [Source::YouTube, Source::SoundCloud] {
                let started = Instant::now();
                match ytdlp::search(&resolved, &net, source, &query, 8, &control) {
                    Ok(cands) => {
                        let ranked = matching::rank(&target, &cands);
                        println!(
                            "  {} search: {} hits in {:.1}s, best = {:?}",
                            source.label(),
                            cands.len(),
                            started.elapsed().as_secs_f32(),
                            ranked.first().map(|c| (&c.title, &c.uploader, c.duration_secs, matching::score(&target, c)))
                        );
                    }
                    Err(e) => println!("  {} search failed: {:?} {}", source.label(), e.kind, e.message),
                }
            }
            let work = WorkDir::create(&dir, 1).unwrap();
            let started = Instant::now();
            let mut last = 0.0;
            let mut cb = |p: ytdlp::Progress| {
                if let Some(f) = p.fraction {
                    last = f;
                }
            };
            // Same source walk as a real job, minus the AppHandle plumbing.
            let mut outcome = Err(JobError::new(ErrorKind::NotFound, "no candidates"));
            'sources: for source in [Source::YouTube, Source::SoundCloud] {
                let Ok(cands) = ytdlp::search(&resolved, &net, source, &query, 8, &control) else { continue };
                for cand in matching::rank(&target, &cands).into_iter().take(CANDIDATES_PER_SOURCE) {
                    match ytdlp::download(&resolved, &net, &cand.url, &work.0, &control, &mut cb) {
                        Ok(f) => {
                            outcome = Ok((f, source));
                            break 'sources;
                        }
                        Err(e) => {
                            println!("  {} download failed: {:?} {}", source.label(), e.kind, e.message);
                            outcome = Err(e);
                            work.clear();
                        }
                    }
                }
            }
            match outcome {
                Ok((file, source)) => {
                    let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                    println!(
                        "  downloaded from {} in {:.1}s: {} ({} KB, progress {:.0}%)",
                        source.label(),
                        started.elapsed().as_secs_f32(),
                        file.display(),
                        size / 1024,
                        last * 100.0
                    );
                    match postprocess::ensure_playable(&file, &resolved, &control) {
                        Ok((p, ext)) => {
                            postprocess::write_tags(
                                &p,
                                &postprocess::TagInfo {
                                    title: &track.title,
                                    artist: &track.artist,
                                    album: track.album.as_deref(),
                                    genre: None,
                                    album_id: track.album_id,
                                    cover_url: track.cover_url.as_deref(),
                                    thumbnail_url: None,
                                },
                            );
                            let meta = crate::audio_player::AudioPlayer::get_metadata_light(&p.to_string_lossy());
                            println!("  playable as .{}; tags: {:?}", ext, meta.map(|m| (m.title, m.artist, m.duration_secs)));
                        }
                        Err(e) => println!("  not playable: {:?} {}", e.kind, e.message),
                    }
                }
                Err(e) => println!("  FAILED: {:?} {}", e.kind, e.message),
            }
        }
        if let Ok(url) = std::env::var("LFM_LIVE_LINK") {
            match ytdlp::probe_link(&resolved, &net, &url, &control) {
                Ok((title, entries)) => println!("\nlink {:?}: {} entries, first {:?}", title, entries.len(), entries.first().map(|e| (&e.artist, &e.title, e.duration_secs))),
                Err(e) => println!("\nlink failed: {:?} {}", e.kind, e.message),
            }
        }
    }

    #[test]
    fn link_labels() {
        assert_eq!(link_label("https://music.youtube.com/watch?v=x"), "YouTube");
        assert_eq!(link_label("https://youtu.be/x"), "YouTube");
        assert_eq!(link_label("https://artist.bandcamp.com/track/y"), "Bandcamp");
        assert_eq!(link_label("https://vk.com/audio1"), "vk.com");
    }
}

