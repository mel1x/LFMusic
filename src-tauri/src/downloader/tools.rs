//! External binaries the downloader shells out to: **yt-dlp** (required),
//! **Deno** (yt-dlp's JS runtime — YouTube's signature challenge can't be
//! solved without one) and **ffmpeg** (optional; converts streams the player
//! can't decode, e.g. Opus, into m4a).
//!
//! Everything lives in `<app_data_dir>/bin`. A binary on the system `PATH`
//! is used when there is no app-managed copy. Installs download into a staging
//! file, verify it (SHA256 for yt-dlp/ffmpeg, `--version` for all), then swap
//! it in with a rename, so a failed install never breaks a working tool.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Ytdlp,
    Deno,
    Ffmpeg,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Ytdlp, Tool::Deno, Tool::Ffmpeg];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Ytdlp => "yt-dlp",
            Tool::Deno => "Deno",
            Tool::Ffmpeg => "ffmpeg",
        }
    }

    fn exe_name(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Tool::Ytdlp, true) => "yt-dlp.exe",
            (Tool::Ytdlp, false) => "yt-dlp",
            (Tool::Deno, true) => "deno.exe",
            (Tool::Deno, false) => "deno",
            (Tool::Ffmpeg, true) => "ffmpeg.exe",
            (Tool::Ffmpeg, false) => "ffmpeg",
        }
    }

    fn version_arg(self) -> &'static str {
        match self {
            Tool::Ffmpeg => "-version",
            _ => "--version",
        }
    }

    /// Whether a newer release can be detected. ffmpeg comes from a rolling
    /// build without comparable versions, so it is only ever installed/removed.
    fn tracks_updates(self) -> bool {
        !matches!(self, Tool::Ffmpeg)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ToolInfo {
    pub installed: bool,
    /// App-managed copy in the bin dir (can be updated/removed by the app).
    pub managed: bool,
    pub version: Option<String>,
    /// Latest known release, from the last update check (if any).
    pub latest: Option<String>,
    pub update_available: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ToolsInfo {
    pub ytdlp: ToolInfo,
    pub deno: ToolInfo,
    pub ffmpeg: ToolInfo,
    /// Not running on a platform where the tools exist (Android).
    pub unsupported: bool,
}

impl ToolsInfo {
    pub fn get_mut(&mut self, tool: Tool) -> &mut ToolInfo {
        match tool {
            Tool::Ytdlp => &mut self.ytdlp,
            Tool::Deno => &mut self.deno,
            Tool::Ffmpeg => &mut self.ffmpeg,
        }
    }
}

/// Paths of the tools as they'll be passed to a job.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub ytdlp: PathBuf,
    pub deno: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
}

/// Only one install/update at a time (button spam, auto-update racing a manual one).
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

pub fn platform_supported() -> bool {
    cfg!(not(any(target_os = "android", target_os = "ios")))
}

/// `Command` that doesn't flash a console window on Windows.
pub fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// yt-dlp parses some path arguments itself, where a backslash may be eaten
/// as an escape — forward slashes work everywhere, including Windows.
pub fn path_arg(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub fn bin_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("bin")
}

fn managed_path(data_dir: &Path, tool: Tool) -> PathBuf {
    bin_dir(data_dir).join(tool.exe_name())
}

/// An app-installed copy exists (cheap file check, doesn't run it).
pub fn is_managed(data_dir: &Path, tool: Tool) -> bool {
    managed_path(data_dir, tool).is_file()
}

fn version_of(program: &Path, tool: Tool) -> Option<String> {
    let out = command(program)
        .arg(tool.version_arg())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?.trim();
    // "2026.09.01" | "deno 2.5.1 (stable, …)" | "ffmpeg version N-1234-g… Copyright …"
    let v = match tool {
        Tool::Ytdlp => first.to_string(),
        Tool::Deno => first.trim_start_matches("deno ").split_whitespace().next()?.to_string(),
        Tool::Ffmpeg => first
            .trim_start_matches("ffmpeg version ")
            .split_whitespace()
            .next()?
            .to_string(),
    };
    (!v.is_empty()).then_some(v)
}

/// App-managed binary first, then `PATH`. Returns the path and its version.
fn locate(data_dir: &Path, tool: Tool) -> Option<(PathBuf, bool, String)> {
    if !platform_supported() {
        return None;
    }
    let managed = managed_path(data_dir, tool);
    if managed.is_file() {
        if let Some(v) = version_of(&managed, tool) {
            return Some((managed, true, v));
        }
    }
    let system = PathBuf::from(tool.exe_name());
    version_of(&system, tool).map(|v| (system, false, v))
}

pub fn resolve(data_dir: &Path) -> Option<Resolved> {
    let (ytdlp, _, _) = locate(data_dir, Tool::Ytdlp)?;
    Some(Resolved {
        ytdlp,
        deno: locate(data_dir, Tool::Deno).map(|t| t.0),
        ffmpeg: locate(data_dir, Tool::Ffmpeg).map(|t| t.0),
    })
}

/// Local status (no network). `latest` values from the last update check are
/// merged in from the metadata file.
pub fn info(data_dir: &Path) -> ToolsInfo {
    let mut out = ToolsInfo {
        unsupported: !platform_supported(),
        ..Default::default()
    };
    let meta = Meta::load(data_dir);
    for tool in Tool::ALL {
        let slot = out.get_mut(tool);
        if let Some((_, managed, version)) = locate(data_dir, tool) {
            slot.installed = true;
            slot.managed = managed;
            slot.version = Some(version);
        }
        slot.latest = meta.latest(tool).map(str::to_string);
        slot.update_available = slot.installed
            && tool.tracks_updates()
            && match (&slot.version, &slot.latest) {
                (Some(v), Some(l)) => normalize_version(v) != normalize_version(l),
                _ => false,
            };
    }
    out
}

fn normalize_version(v: &str) -> &str {
    v.trim().trim_start_matches('v')
}

// ── Metadata: last update check ─────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct Meta {
    #[serde(default)]
    ytdlp_latest: Option<String>,
    #[serde(default)]
    deno_latest: Option<String>,
    /// Unix seconds of the last successful update check.
    #[serde(default)]
    checked_at: u64,
}

impl Meta {
    fn path(data_dir: &Path) -> PathBuf {
        bin_dir(data_dir).join("tools.json")
    }
    fn load(data_dir: &Path) -> Meta {
        std::fs::read_to_string(Self::path(data_dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save(&self, data_dir: &Path) {
        let _ = std::fs::create_dir_all(bin_dir(data_dir));
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(data_dir), s);
        }
    }
    fn latest(&self, tool: Tool) -> Option<&str> {
        match tool {
            Tool::Ytdlp => self.ytdlp_latest.as_deref(),
            Tool::Deno => self.deno_latest.as_deref(),
            Tool::Ffmpeg => None,
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Seconds since the last successful update check.
pub fn secs_since_update_check(data_dir: &Path) -> u64 {
    now_secs().saturating_sub(Meta::load(data_dir).checked_at)
}

// ── Network ─────────────────────────────────────────────────────────────────

fn client(follow_redirects: bool) -> Result<reqwest::blocking::Client, String> {
    let mut b = reqwest::blocking::Client::builder()
        .user_agent("LFMusic (+https://github.com/mel1x/LFMusic)")
        .connect_timeout(Duration::from_secs(20));
    if !follow_redirects {
        b = b.redirect(reqwest::redirect::Policy::none());
    }
    b.build().map_err(|e| e.to_string())
}

/// GitHub intermittently answers 503/429 under load; retry with backoff
/// before giving up.
fn retrying<T>(mut f: impl FnMut() -> Result<T, String>) -> Result<T, String> {
    let mut last = String::new();
    for attempt in 0..4u32 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(800 * 2u64.pow(attempt - 1)));
        }
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Latest release tag of a GitHub repo. The `/releases/latest` redirect is
/// tried first (no rate limit), the REST API (60 req/h) as a fallback.
fn latest_github_tag(repo: &str) -> Result<String, String> {
    let via_redirect = || -> Result<String, String> {
        let resp = client(false)?
            .get(format!("https://github.com/{}/releases/latest", repo))
            .timeout(Duration::from_secs(20))
            .send()
            .map_err(|e| e.to_string())?;
        resp.headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|loc| loc.rsplit("/tag/").next())
            .filter(|t| !t.is_empty() && !t.contains('/') && !t.contains(':'))
            .map(str::to_string)
            .ok_or_else(|| format!("GitHub ответил {}", resp.status()))
    };
    let via_api = || -> Result<String, String> {
        let v: serde_json::Value = client(true)?
            .get(format!("https://api.github.com/repos/{}/releases/latest", repo))
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .timeout(Duration::from_secs(20))
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
            .map_err(|e| e.to_string())?;
        v.get("tag_name")
            .and_then(|t| t.as_str())
            .map(str::to_string)
            .ok_or_else(|| "GitHub API не вернул версию".to_string())
    };
    retrying(|| via_redirect().or_else(|_| via_api()))
        .map_err(|e| format!("Не удалось узнать последнюю версию {}: {}", repo, e))
}

fn fetch_text(url: &str) -> Result<String, String> {
    retrying(|| {
        client(true)?
            .get(url)
            .timeout(Duration::from_secs(30))
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.text())
            .map_err(|e| e.to_string())
    })
}

/// Streams `url` into `target`, reporting `(downloaded, total)`.
fn download(url: &str, target: &Path, progress: &dyn Fn(u64, Option<u64>)) -> Result<(), String> {
    let mut resp = retrying(|| {
        client(true)?
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("Ошибка загрузки: {}", e))
    })?;
    let total = resp.content_length();
    let mut file = std::fs::File::create(target).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 128 * 1024];
    let mut done = 0u64;
    loop {
        let n = resp.read(&mut buf).map_err(|e| format!("Обрыв загрузки: {}", e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(done, total);
    }
    file.flush().map_err(|e| e.to_string())?;
    if let Some(t) = total {
        if done != t {
            return Err("Файл скачался не полностью".into());
        }
    }
    Ok(())
}

fn sha256_hex(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect())
}

/// Finds `asset`'s hash in a `sha256sum`-style listing.
fn hash_from_listing(listing: &str, asset: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let mut it = line.split_whitespace();
        let hash = it.next()?;
        let name = it.next()?.trim_start_matches('*');
        (name == asset && hash.len() == 64).then(|| hash.to_lowercase())
    })
}

fn verify_sha256(file: &Path, expected: &str) -> Result<(), String> {
    let actual = sha256_hex(file)?;
    if actual != expected {
        return Err("Контрольная сумма не совпала - файл повреждён или подменён".into());
    }
    Ok(())
}

// ── Release assets per platform ─────────────────────────────────────────────

fn arm64() -> bool {
    std::env::consts::ARCH == "aarch64"
}

fn ytdlp_asset() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if arm64() {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp_linux"
    }
}

fn deno_asset() -> &'static str {
    match (cfg!(windows), cfg!(target_os = "macos"), arm64()) {
        (true, _, true) => "deno-aarch64-pc-windows-msvc.zip",
        (true, _, false) => "deno-x86_64-pc-windows-msvc.zip",
        (_, true, true) => "deno-aarch64-apple-darwin.zip",
        (_, true, false) => "deno-x86_64-apple-darwin.zip",
        (_, _, true) => "deno-aarch64-unknown-linux-gnu.zip",
        _ => "deno-x86_64-unknown-linux-gnu.zip",
    }
}

/// yt-dlp/FFmpeg-Builds asset (Windows/Linux). macOS uses evermeet.cx.
fn ffmpeg_asset() -> &'static str {
    match (cfg!(windows), arm64()) {
        (true, true) => "ffmpeg-master-latest-winarm64-gpl.zip",
        (true, false) => "ffmpeg-master-latest-win64-gpl.zip",
        (false, true) => "ffmpeg-master-latest-linuxarm64-gpl.tar.xz",
        (false, false) => "ffmpeg-master-latest-linux64-gpl.tar.xz",
    }
}

// ── Archive extraction ──────────────────────────────────────────────────────

/// Extracts the entry whose file name is `name` (anywhere in the archive) to `out`.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn extract_from_zip(archive: &Path, name: &str, out: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Архив повреждён: {}", e))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let entry_name = entry.name().replace('\\', "/");
        if !entry.is_dir() && entry_name.rsplit('/').next() == Some(name) {
            let mut dst = std::fs::File::create(out).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut dst).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err(format!("{} не найден в архиве", name))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn extract_from_zip(_archive: &Path, _name: &str, _out: &Path) -> Result<(), String> {
    Err("Не поддерживается на этой платформе".into())
}

#[cfg(target_os = "linux")]
fn extract_from_tar_xz(archive: &Path, name: &str, out: &Path) -> Result<(), String> {
    let xz = std::io::BufReader::new(std::fs::File::open(archive).map_err(|e| e.to_string())?);
    let tar_path = archive.with_extension("tar");
    {
        let mut tar_out = std::io::BufWriter::new(std::fs::File::create(&tar_path).map_err(|e| e.to_string())?);
        let mut xz = xz;
        lzma_rs::xz_decompress(&mut xz, &mut tar_out).map_err(|e| format!("Архив повреждён: {:?}", e))?;
        tar_out.flush().map_err(|e| e.to_string())?;
    }
    let result = (|| {
        let file = std::fs::File::open(&tar_path).map_err(|e| e.to_string())?;
        let mut tar = tar::Archive::new(std::io::BufReader::new(file));
        for entry in tar.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let is_match = entry
                .path()
                .ok()
                .and_then(|p| p.file_name().map(|n| n == name))
                .unwrap_or(false);
            if is_match {
                let mut dst = std::fs::File::create(out).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut dst).map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
        Err(format!("{} не найден в архиве", name))
    })();
    let _ = std::fs::remove_file(&tar_path);
    result
}

#[cfg(unix)]
fn make_executable(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn make_executable(_p: &Path) -> Result<(), String> {
    Ok(())
}

// ── Install / remove ────────────────────────────────────────────────────────

/// Progress callback: (tool, human-readable message, fraction 0..1 if known).
pub type Progress<'a> = &'a (dyn Fn(Tool, String, Option<f64>) + Sync);

fn size_mb(bytes: u64) -> String {
    format!("{:.1} МБ", bytes as f64 / 1_048_576.0)
}

/// Scratch folder for one install, removed on drop.
struct Staging(PathBuf);

impl Staging {
    fn new(data_dir: &Path, tool: Tool) -> Result<Staging, String> {
        let dir = bin_dir(data_dir).join(format!(".staging-{}", tool.label().to_lowercase()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Не удалось создать папку: {}", e))?;
        Ok(Staging(dir))
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Moves `new` over `target`. A running executable can't be deleted on
/// Windows but can be renamed, so the old copy is moved aside first and only
/// then removed (best-effort; leftovers are swept on the next install).
fn swap_in(new: &Path, target: &Path) -> Result<(), String> {
    let old = target.with_extension("old");
    let _ = std::fs::remove_file(&old);
    let had_old = target.exists();
    if had_old {
        std::fs::rename(target, &old).map_err(|e| format!("Не удалось заменить файл: {}", e))?;
    }
    if let Err(e) = std::fs::rename(new, target) {
        if had_old {
            let _ = std::fs::rename(&old, target);
        }
        return Err(format!("Не удалось установить файл: {}", e));
    }
    let _ = std::fs::remove_file(&old);
    Ok(())
}

/// Fetches the tool into staging, verifies it and swaps it in. Returns the
/// installed version.
pub fn install(data_dir: &Path, tool: Tool, progress: Progress) -> Result<String, String> {
    if !platform_supported() {
        return Err("Скачивание не поддерживается на этой платформе".into());
    }
    let _guard = INSTALL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::create_dir_all(bin_dir(data_dir)).map_err(|e| e.to_string())?;
    let staging = Staging::new(data_dir, tool)?;
    let staged_exe = staging.file(tool.exe_name());
    let report = |done: u64, total: Option<u64>| {
        let msg = match total {
            Some(t) if t > 0 => format!("Загрузка {}: {} из {}", tool.label(), size_mb(done), size_mb(t)),
            _ => format!("Загрузка {}: {}", tool.label(), size_mb(done)),
        };
        progress(tool, msg, total.filter(|t| *t > 0).map(|t| done as f64 / t as f64));
    };
    progress(tool, format!("Подготовка {}…", tool.label()), None);

    let mut latest_tag: Option<String> = None;
    match tool {
        Tool::Ytdlp => {
            let tag = latest_github_tag("yt-dlp/yt-dlp")?;
            let base = format!("https://github.com/yt-dlp/yt-dlp/releases/download/{}", tag);
            let sums = fetch_text(&format!("{}/SHA2-256SUMS", base))?;
            let expected = hash_from_listing(&sums, ytdlp_asset())
                .ok_or("Не найдена контрольная сумма yt-dlp")?;
            download(&format!("{}/{}", base, ytdlp_asset()), &staged_exe, &report)?;
            verify_sha256(&staged_exe, &expected)?;
            latest_tag = Some(tag);
        }
        Tool::Deno => {
            let tag = latest_github_tag("denoland/deno")?;
            let url = format!(
                "https://github.com/denoland/deno/releases/download/{}/{}",
                tag,
                deno_asset()
            );
            let archive = staging.file("deno.zip");
            download(&url, &archive, &report)?;
            progress(tool, "Распаковка Deno…".into(), None);
            extract_from_zip(&archive, tool.exe_name(), &staged_exe)?;
            latest_tag = Some(tag);
        }
        Tool::Ffmpeg => install_ffmpeg_into(&staging, &staged_exe, &report, progress)?,
    }

    make_executable(&staged_exe)?;
    let version = version_of(&staged_exe, tool)
        .ok_or_else(|| format!("{} не запускается на этой системе", tool.label()))?;
    swap_in(&staged_exe, &managed_path(data_dir, tool))?;

    if let Some(tag) = latest_tag {
        let mut meta = Meta::load(data_dir);
        match tool {
            Tool::Ytdlp => meta.ytdlp_latest = Some(tag),
            Tool::Deno => meta.deno_latest = Some(tag),
            Tool::Ffmpeg => {}
        }
        meta.save(data_dir);
    }
    Ok(version)
}

#[cfg(not(target_os = "macos"))]
fn install_ffmpeg_into(
    staging: &Staging,
    staged_exe: &Path,
    report: &dyn Fn(u64, Option<u64>),
    progress: Progress,
) -> Result<(), String> {
    let base = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest";
    let asset = ffmpeg_asset();
    // Checksums are published alongside the rolling build; verify when present.
    let expected = fetch_text(&format!("{}/checksums.sha256", base))
        .ok()
        .and_then(|s| hash_from_listing(&s, asset));
    let archive = staging.file(asset);
    download(&format!("{}/{}", base, asset), &archive, report)?;
    if let Some(hash) = expected {
        verify_sha256(&archive, &hash)?;
    }
    progress(Tool::Ffmpeg, "Распаковка ffmpeg…".into(), None);
    #[cfg(target_os = "linux")]
    return extract_from_tar_xz(&archive, Tool::Ffmpeg.exe_name(), staged_exe);
    #[cfg(not(target_os = "linux"))]
    return extract_from_zip(&archive, Tool::Ffmpeg.exe_name(), staged_exe);
}

#[cfg(target_os = "macos")]
fn install_ffmpeg_into(
    staging: &Staging,
    staged_exe: &Path,
    report: &dyn Fn(u64, Option<u64>),
    progress: Progress,
) -> Result<(), String> {
    let _ = ffmpeg_asset;
    let archive = staging.file("ffmpeg.zip");
    download("https://evermeet.cx/ffmpeg/getrelease/ffmpeg/zip", &archive, report)?;
    progress(Tool::Ffmpeg, "Распаковка ffmpeg…".into(), None);
    extract_from_zip(&archive, Tool::Ffmpeg.exe_name(), staged_exe)
}

pub fn remove(data_dir: &Path, tool: Tool) -> Result<(), String> {
    let _guard = INSTALL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = managed_path(data_dir, tool);
    for p in [path.clone(), path.with_extension("old")] {
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Не удалось удалить {}: {}", tool.label(), e)),
        }
    }
    Ok(())
}

/// Network check for newer yt-dlp / Deno releases; stores the result so the
/// local [`info`] can report `update_available` without the network.
pub fn check_updates(data_dir: &Path) -> Result<ToolsInfo, String> {
    let ytdlp = latest_github_tag("yt-dlp/yt-dlp")?;
    let deno = latest_github_tag("denoland/deno").ok();
    let mut meta = Meta::load(data_dir);
    meta.ytdlp_latest = Some(ytdlp);
    if deno.is_some() {
        meta.deno_latest = deno;
    }
    meta.checked_at = now_secs();
    meta.save(data_dir);
    Ok(info(data_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_checksum_listing() {
        let listing = "aaaa  other\n\
            0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  yt-dlp.exe\n";
        assert_eq!(
            hash_from_listing(listing, "yt-dlp.exe").as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert!(hash_from_listing(listing, "yt-dlp").is_none());
    }

    #[test]
    fn version_normalization() {
        assert_eq!(normalize_version("v2.5.1"), normalize_version("2.5.1"));
    }
}
