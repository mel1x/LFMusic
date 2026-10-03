//! yt-dlp invocations: candidate search, link probing and the audio download
//! itself, plus classification of its error output into [`ErrorKind`].

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use super::matching::Candidate;
use super::tools::{self, Resolved};
use super::{ErrorKind, JobError};

/// Where to look for a catalog track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    YouTube,
    SoundCloud,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::YouTube => "YouTube",
            Source::SoundCloud => "SoundCloud",
        }
    }
}

/// Network/auth options shared by every invocation.
#[derive(Debug, Clone, Default)]
pub struct NetOptions {
    /// `None` = let yt-dlp use system/env proxy, `Some("")` = direct, `Some(url)` = explicit.
    pub proxy: Option<String>,
    pub cookies: Option<Cookies>,
}

#[derive(Debug, Clone)]
pub enum Cookies {
    /// `--cookies-from-browser <name>`
    Browser(String),
    /// `--cookies <netscape file>`
    File(PathBuf),
}

/// Cancellation handle for one job: the running process (if any) and a flag.
#[derive(Default)]
pub struct Control {
    cancelled: AtomicBool,
    pid: Mutex<Option<u32>>,
}

impl Control {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(pid) = *self.pid.lock().unwrap_or_else(|e| e.into_inner()) {
            kill_tree(pid);
        }
    }

    pub fn check(&self) -> Result<(), JobError> {
        if self.is_cancelled() {
            Err(JobError::cancelled())
        } else {
            Ok(())
        }
    }

    /// Sleeps in short slices so cancellation stays responsive.
    pub fn sleep(&self, d: Duration) -> Result<(), JobError> {
        let end = std::time::Instant::now() + d;
        while std::time::Instant::now() < end {
            self.check()?;
            std::thread::sleep(Duration::from_millis(100));
        }
        self.check()
    }
}

/// Kills a process together with its children. The PyInstaller-built yt-dlp
/// is a bootloader process plus the real interpreter, and it spawns ffmpeg —
/// killing only the top PID would leave those running.
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = tools::command(Path::new("taskkill"))
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // Processes are started in their own group (see `spawn`), so the
        // negative PID targets the whole group.
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{}", pid))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn base_command(tools: &Resolved, net: &NetOptions) -> std::process::Command {
    let mut cmd = tools::command(&tools.ytdlp);
    cmd.args([
        "--ignore-config",
        "--no-color",
        "--encoding",
        "utf-8",
        "--socket-timeout",
        "20",
        "--retries",
        "5",
        "--fragment-retries",
        "5",
        "--extractor-retries",
        "3",
    ]);
    if let Some(deno) = &tools.deno {
        cmd.arg("--js-runtimes")
            .arg(format!("deno:{}", tools::path_arg(deno)));
    }
    match &net.proxy {
        Some(p) => {
            cmd.arg("--proxy").arg(p);
        }
        None => {}
    }
    match &net.cookies {
        Some(Cookies::Browser(name)) => {
            cmd.arg("--cookies-from-browser").arg(name);
        }
        Some(Cookies::File(path)) => {
            cmd.arg("--cookies").arg(path);
        }
        None => {}
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd
}

fn spawn(mut cmd: std::process::Command, control: &Control) -> Result<Child, JobError> {
    control.check()?;
    let child = cmd.spawn().map_err(|e| {
        JobError::new(ErrorKind::ToolMissing, format!("Не удалось запустить yt-dlp: {}", e))
    })?;
    *control.pid.lock().unwrap_or_else(|e| e.into_inner()) = Some(child.id());
    // A cancel that raced with the spawn wouldn't have seen the PID.
    if control.is_cancelled() {
        kill_tree(child.id());
    }
    Ok(child)
}

fn finish(mut child: Child, control: &Control) -> Result<std::process::ExitStatus, JobError> {
    let status = child.wait();
    *control.pid.lock().unwrap_or_else(|e| e.into_inner()) = None;
    control.check()?;
    status.map_err(|e| JobError::new(ErrorKind::Unknown, e.to_string()))
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// Runs yt-dlp to completion, returning stdout or a classified error.
fn run_capture(cmd: std::process::Command, control: &Control) -> Result<String, JobError> {
    let mut child = spawn(cmd, control)?;
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let status = finish(child, control)?;
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if !status.success() {
        return Err(classify(&stderr));
    }
    Ok(stdout)
}

// ── Search ──────────────────────────────────────────────────────────────────

fn str_field(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(str::to_string)
}

fn entry_url(v: &Value) -> Option<String> {
    str_field(v, &["webpage_url", "original_url", "url"]).filter(|u| u.starts_with("http"))
}

fn entry_to_candidate(v: &Value) -> Option<Candidate> {
    Some(Candidate {
        url: entry_url(v)?,
        title: str_field(v, &["title"])?,
        uploader: str_field(v, &["channel", "uploader", "artist"]).unwrap_or_default(),
        duration_secs: v.get("duration").and_then(Value::as_f64),
    })
}

/// Searches `source` for `query`, returning up to `count` candidates.
///
/// YouTube uses `--flat-playlist`: one request to the search page, which
/// already carries title/channel/duration and never touches the (bot-walled)
/// player endpoint. SoundCloud's flat entries lack durations, so its results
/// are fully extracted instead (a few cheap API calls).
pub fn search(
    tools: &Resolved,
    net: &NetOptions,
    source: Source,
    query: &str,
    count: u32,
    control: &Control,
) -> Result<Vec<Candidate>, JobError> {
    let mut cmd = base_command(tools, net);
    cmd.arg("--dump-json").arg("--ignore-errors");
    match source {
        Source::YouTube => {
            cmd.arg("--flat-playlist")
                .arg(format!("ytsearch{}:{}", count, query));
        }
        Source::SoundCloud => {
            cmd.arg(format!("scsearch{}:{}", count, query));
        }
    }
    // --ignore-errors makes yt-dlp exit non-zero if any single entry failed,
    // even though the others were printed — keep whatever parsed.
    let mut child = spawn(cmd, control)?;
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let status = finish(child, control)?;
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    let found: Vec<Candidate> = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .filter_map(|v| entry_to_candidate(&v))
        .collect();
    if found.is_empty() && !status.success() {
        return Err(classify(&stderr));
    }
    Ok(found)
}

// ── Link probing ────────────────────────────────────────────────────────────

/// Metadata for one entry behind a pasted link.
#[derive(Debug, Clone)]
pub struct LinkEntry {
    pub url: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_secs: Option<u32>,
    pub thumbnail: Option<String>,
}

/// Uploader names that carry no artist information.
fn clean_uploader(s: &str) -> String {
    s.trim()
        .trim_end_matches(" - Topic")
        .trim_end_matches("VEVO")
        .trim()
        .to_string()
}

/// "Artist - Title (Official Video)" → ("Artist", "Title").
pub fn split_artist_title(raw: &str) -> Option<(String, String)> {
    for sep in [" - ", " – ", " — ", " | "] {
        if let Some((a, t)) = raw.split_once(sep) {
            let (a, t) = (a.trim(), strip_video_noise(t));
            if !a.is_empty() && !t.is_empty() {
                return Some((a.to_string(), t));
            }
        }
    }
    None
}

/// Drops "(Official Video)", "[Lyrics]", "(Audio)" and similar decorations
/// that describe the upload rather than the song.
pub fn strip_video_noise(title: &str) -> String {
    const NOISE: &[&str] = &[
        "official", "video", "audio", "lyric", "lyrics", "visualizer", "music video", "mv", "hd",
        "4k", "клип", "премьера", "official music video",
    ];
    let mut out = String::new();
    let mut rest = title;
    while let Some(start) = rest.find(|c| c == '(' || c == '[') {
        let close = if rest[start..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[start..].find(close) else { break };
        let inner = rest[start + 1..start + len].to_lowercase();
        out.push_str(&rest[..start]);
        if !NOISE.iter().any(|n| inner.contains(n)) {
            out.push_str(&rest[start..=start + len]);
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn best_thumbnail(v: &Value) -> Option<String> {
    if let Some(list) = v.get("thumbnails").and_then(Value::as_array) {
        if let Some(url) = list
            .iter()
            .rev()
            .filter_map(|t| t.get("url").and_then(Value::as_str))
            .find(|u| u.starts_with("http"))
        {
            return Some(url.to_string());
        }
    }
    str_field(v, &["thumbnail"])
}

fn entry_to_link(v: &Value) -> Option<LinkEntry> {
    let url = entry_url(v)?;
    let raw_title = str_field(v, &["title"])?;
    // YouTube Music / SoundCloud expose proper music metadata — prefer it.
    let (artist, title) = match (str_field(v, &["artist", "creator"]), str_field(v, &["track"])) {
        (Some(a), Some(t)) => (a, t),
        _ => split_artist_title(&raw_title).unwrap_or_else(|| {
            let uploader = str_field(v, &["channel", "uploader"]).unwrap_or_default();
            (clean_uploader(&uploader), strip_video_noise(&raw_title))
        }),
    };
    Some(LinkEntry {
        url,
        title,
        artist,
        album: str_field(v, &["album"]),
        duration_secs: v
            .get("duration")
            .and_then(Value::as_f64)
            .map(|d| d.round() as u32),
        thumbnail: best_thumbnail(v),
    })
}

/// Probes a link without downloading. A playlist/album link yields all of its
/// entries (capped), a single video yields one.
pub fn probe_link(
    tools: &Resolved,
    net: &NetOptions,
    url: &str,
    control: &Control,
) -> Result<(Option<String>, Vec<LinkEntry>), JobError> {
    let mut cmd = base_command(tools, net);
    cmd.args(["--dump-single-json", "--flat-playlist", "--playlist-end", "300"])
        .arg(url);
    let stdout = run_capture(cmd, control)?;
    let root: Value = serde_json::from_str(stdout.trim())
        .map_err(|_| JobError::new(ErrorKind::Unknown, "yt-dlp вернул неожиданный ответ"))?;
    let (collection, entries): (Option<String>, Vec<LinkEntry>) =
        match root.get("entries").and_then(Value::as_array) {
            Some(list) => (str_field(&root, &["title"]), list.iter().filter_map(entry_to_link).collect()),
            None => (None, entry_to_link(&root).into_iter().collect()),
        };
    if entries.is_empty() {
        return Err(JobError::new(ErrorKind::NotFound, "По ссылке не найдено аудио"));
    }
    Ok((collection, entries))
}

// ── Download ────────────────────────────────────────────────────────────────

/// Progress update from a running download.
pub struct Progress {
    pub fraction: Option<f64>,
    pub speed_bps: Option<f64>,
    pub eta_secs: Option<u64>,
}

/// Formats the player can decode as-is (symphonia: no Opus).
const FORMAT_SELECTOR: &str = "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio[ext=mp3]/\
                               bestaudio[acodec=vorbis]/bestaudio/best";

const PROGRESS_TAG: &str = "[lfm-progress]";
const FILE_TAG: &str = "[lfm-file]";

fn parse_num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)
}

fn parse_progress(line: &str) -> Option<Progress> {
    let rest = line.trim().strip_prefix(PROGRESS_TAG)?;
    let parts: Vec<&str> = rest.split('|').collect();
    if parts.len() < 5 {
        return None;
    }
    let done = parse_num(parts[0]);
    let total = parse_num(parts[1]).or_else(|| parse_num(parts[2]));
    let fraction = match (done, total) {
        (Some(d), Some(t)) if t > 0.0 => Some((d / t).clamp(0.0, 1.0)),
        _ => None,
    };
    Some(Progress {
        fraction,
        speed_bps: parse_num(parts[3]),
        eta_secs: parse_num(parts[4]).map(|v| v as u64),
    })
}

/// Downloads `url`'s audio into `dir` (an empty per-job folder) and returns
/// the produced file.
pub fn download(
    tools: &Resolved,
    net: &NetOptions,
    url: &str,
    dir: &Path,
    control: &Control,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<PathBuf, JobError> {
    let mut cmd = base_command(tools, net);
    cmd.args(["--no-playlist", "--no-mtime", "--newline", "--progress", "--no-part"])
        .arg("--progress-template")
        .arg(format!(
            "download:{}%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s",
            PROGRESS_TAG
        ))
        .arg("--print")
        .arg(format!("after_move:{}%(filepath)s", FILE_TAG))
        .arg("-f")
        .arg(FORMAT_SELECTOR)
        .arg("-o")
        .arg(dir.join("audio.%(ext)s"));
    if let Some(ff) = &tools.ffmpeg {
        if let Some(parent) = ff.parent().filter(|p| !p.as_os_str().is_empty()) {
            cmd.arg("--ffmpeg-location").arg(parent);
        }
    }
    cmd.arg("--").arg(url);

    let mut child = spawn(cmd, control)?;
    let err = drain(child.stderr.take());
    let mut reported: Option<PathBuf> = None;
    if let Some(out) = child.stdout.take() {
        let mut reader = BufReader::new(out);
        let mut raw = Vec::new();
        loop {
            raw.clear();
            match reader.read_until(b'\n', &mut raw) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = String::from_utf8_lossy(&raw);
            if let Some(p) = parse_progress(&line) {
                on_progress(p);
            } else if let Some(path) = line.trim().strip_prefix(FILE_TAG) {
                let path = path.trim();
                if !path.is_empty() {
                    reported = Some(PathBuf::from(path));
                }
            }
        }
    }
    let status = finish(child, control)?;
    let stderr = err.join().unwrap_or_default();
    if !status.success() {
        return Err(classify(&stderr));
    }
    reported
        .filter(|p| p.is_file())
        .or_else(|| {
            // Fallback: the only non-temp file in the job folder.
            std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("audio.") && !n.ends_with(".part") && !n.ends_with(".ytdl"))
                        .unwrap_or(false)
            })
        })
        .ok_or_else(|| JobError::new(ErrorKind::Unknown, "yt-dlp не сохранил файл"))
}

// ── Error classification ────────────────────────────────────────────────────

/// Last `ERROR:` line, without yt-dlp's `[extractor] id:` prefix noise.
fn error_line(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("yt-dlp завершился с ошибкой")
        .trim();
    let line = line.trim_start_matches("ERROR:").trim();
    let line = match line.find("]") {
        Some(i) if line.starts_with('[') => line[i + 1..].trim(),
        _ => line,
    };
    let mut msg: String = line.chars().take(300).collect();
    if msg.is_empty() {
        msg = "yt-dlp завершился с ошибкой".into();
    }
    msg
}

pub fn classify(stderr: &str) -> JobError {
    let s = stderr.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| s.contains(n));
    // Order matters: YouTube's age/bot messages also mention cookies, so the
    // specific cases go first and the cookie check only matches yt-dlp's own
    // cookie-loading failures.
    let kind = if has(&["not a bot", "confirm you're not", "confirm you’re not"]) {
        ErrorKind::BotCheck
    } else if has(&[
        "cookies database",
        "failed to decrypt",
        "could not copy chrome cookie",
        "netscape format cookies",
        "unsupported browser",
        "cookies file",
    ]) {
        ErrorKind::Cookies
    } else if has(&[
        "unable to extract",
        "nsig extraction failed",
        "signature extraction failed",
        "please report this issue",
        "update to the latest version",
        "confirm you are on the latest version",
    ]) {
        ErrorKind::Outdated
    } else if has(&["http error 429", "too many requests", "rate-limit", "rate limit"]) {
        ErrorKind::RateLimited
    } else if has(&[
        "your age",
        "age-restricted",
        "inappropriate for some users",
        "members-only",
        "join this channel",
        "requires payment",
        "login required",
        "sign in",
    ]) {
        ErrorKind::AuthRequired
    } else if has(&[
        "video unavailable",
        "has been removed",
        "not available in your country",
        "geo restrict",
        "private video",
        "copyright",
        "this track is not available",
        "http error 404",
        "is not a valid url",
        "unsupported url",
    ]) {
        ErrorKind::Unavailable
    } else if has(&["requested format is not available", "no video formats", "no formats found"]) {
        ErrorKind::Format
    } else if has(&["no space left", "errno 28", "disk full"]) {
        ErrorKind::Disk
    } else if has(&[
        "timed out",
        "timeout",
        "unable to download webpage",
        "connection",
        "getaddrinfo",
        "name or service not known",
        "network is unreachable",
        "ssl",
        "remote end closed",
        "proxy",
        "errno 11001",
        "temporary failure",
        "http error 5",
    ]) {
        ErrorKind::Network
    } else {
        ErrorKind::Unknown
    };
    JobError::new(kind, error_line(stderr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_failures() {
        let k = |s: &str| classify(s).kind;
        assert_eq!(
            k("ERROR: [youtube] abc: Sign in to confirm you’re not a bot. Use --cookies"),
            ErrorKind::BotCheck
        );
        assert_eq!(
            k("ERROR: [youtube] abc: Sign in to confirm your age. This video may be inappropriate"),
            ErrorKind::AuthRequired
        );
        assert_eq!(k("ERROR: [youtube] abc: Video unavailable"), ErrorKind::Unavailable);
        assert_eq!(
            k("ERROR: Unable to download webpage: <urlopen error [Errno 11001] getaddrinfo failed>"),
            ErrorKind::Network
        );
        assert_eq!(k("ERROR: HTTP Error 429: Too Many Requests"), ErrorKind::RateLimited);
        assert_eq!(
            k("ERROR: Could not find firefox cookies database in C:\\..."),
            ErrorKind::Cookies
        );
        assert_eq!(k("ERROR: something odd"), ErrorKind::Unknown);
    }

    #[test]
    fn error_line_strips_prefixes() {
        assert_eq!(
            error_line("WARNING: x\nERROR: [youtube] abc123: Video unavailable\n"),
            "abc123: Video unavailable"
        );
    }

    #[test]
    fn parses_progress_lines() {
        let p = parse_progress("[lfm-progress]512|1024|NA|2048.5|3").unwrap();
        assert_eq!(p.fraction, Some(0.5));
        assert_eq!(p.eta_secs, Some(3));
        let p = parse_progress("[lfm-progress]512|NA|2048|NA|NA").unwrap();
        assert_eq!(p.fraction, Some(0.25));
        assert!(p.speed_bps.is_none());
        assert!(parse_progress("[download] 50%").is_none());
    }

    #[test]
    fn splits_video_titles() {
        assert_eq!(
            split_artist_title("Daft Punk - Get Lucky (Official Audio) ft. Pharrell"),
            Some(("Daft Punk".into(), "Get Lucky ft. Pharrell".into()))
        );
        assert_eq!(strip_video_noise("Song [Lyrics] (Remix)"), "Song (Remix)");
        assert!(split_artist_title("Just a title").is_none());
    }
}
