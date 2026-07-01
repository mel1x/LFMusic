//! Resolver for external **yt-dlp** and **ffmpeg** binaries used by the download page.
//!
//! Resolve order: app bin dir (`<app_data_dir>/bin`) → system `PATH`. No silent
//! auto-download on resolve — install is explicit only, so the download worker
//! never pulls binaries from the network unnoticed (supply-chain).
//!
//! - **yt-dlp**: single binary, fetched from GitHub releases for the current platform.
//! - **ffmpeg**: optional (most audio downloads work without it); prebuilt from
//!   `yt-dlp/FFmpeg-Builds` on Windows/Linux, `evermeet.cx` on macOS.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;

/// Serializes tool installs: queue worker, settings "Install" button, and
/// update could all trigger a download at once and race on the same `.part`
/// file. A concurrent call just waits and finds the already-downloaded binary.
static ENSURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct ResolvedTools {
    pub ytdlp: PathBuf,
    pub ffmpeg: Option<PathBuf>,
    /// JS runtime for yt-dlp (YouTube nsig decryption); some formats unavailable without it.
    pub deno: Option<PathBuf>,
    pub cookies_file: Option<PathBuf>,
    /// `--proxy` value: `None` = don't pass the flag (system proxy/env), `Some("")` =
    /// direct connection, `Some(url)` = explicit proxy.
    pub proxy: Option<String>,
    /// PO-token provider for YouTube's bot-wall (script/CLI mode, no server).
    pub pot: Option<PotProvider>,
}

/// Resolved PO-token provider (bgutil-pot): a plain executable invoked fresh
/// per token by yt-dlp's own plugin — there is no persistent server/port to
/// manage here, matching how yt-dlp/ffmpeg/Deno are already just short-lived
/// subprocesses. `cli_path` becomes the plugin's `cli_path=` extractor-arg,
/// `plugin_dir` becomes yt-dlp's `--plugin-dirs` so the plugin is found at all.
#[derive(Debug, Clone)]
pub struct PotProvider {
    pub cli_path: PathBuf,
    pub plugin_dir: PathBuf,
}

/// Normalizes a path for passing through yt-dlp's Python arg parser, where a
/// literal backslash can be read as an escape char (forward slashes work fine
/// on Windows and the drive letter is kept as-is).
pub fn normalize_path_arg(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ToolInfo {
    pub installed: bool,
    /// Installed by the app (lives in the bin dir) — can be updated/removed.
    pub managed: bool,
    pub version: Option<String>,
    /// yt-dlp: computed even for a system (PATH) install, since an update lands
    /// in the app bin dir and becomes managed. ffmpeg: only for managed (can't
    /// reliably version-compare a system build).
    pub update_available: bool,
}

/// Deno installs/removes alongside yt-dlp, but is tracked as a full `ToolInfo`
/// (not a bare flag) so the UI can show its version and allow removing an
/// app-managed Deno even when yt-dlp itself comes from PATH.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ToolsInfo {
    pub ytdlp: ToolInfo,
    pub ffmpeg: ToolInfo,
    pub deno: ToolInfo,
    /// PO-token provider for YouTube's bot-wall — desktop-only, installed
    /// independently of the rest (not part of "install all").
    pub pot: ToolInfo,
}

/// `Command` without a flashing console window on Windows.
pub fn silent_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    // yt-dlp (frozen-exe) forces stdout encoding to the OS locale (cp1251 on
    // Windows) and ignores PYTHONIOENCODING/PYTHONUTF8. We don't fight it here;
    // downloads.rs does lossy decoding instead, since a strict UTF-8 reader
    // would abort on Cyrillic output and yt-dlp would fail with [Errno 22].
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn bin_dir(app_data_dir: &Path) -> PathBuf {
    let dir = app_data_dir.join("bin");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn ytdlp_local(app_data_dir: &Path) -> PathBuf {
    bin_dir(app_data_dir).join(if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    })
}

fn ffmpeg_local(app_data_dir: &Path) -> PathBuf {
    bin_dir(app_data_dir).join(if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    })
}

fn deno_local(app_data_dir: &Path) -> PathBuf {
    bin_dir(app_data_dir).join(if cfg!(windows) { "deno.exe" } else { "deno" })
}

fn pot_local(app_data_dir: &Path) -> PathBuf {
    bin_dir(app_data_dir).join(if cfg!(windows) {
        "bgutil-pot.exe"
    } else {
        "bgutil-pot"
    })
}

/// Directory passed to yt-dlp's `--plugin-dirs`; holds only the POT provider's
/// script/CLI plugin as a `yt_dlp_plugins/extractor/...` namespace package.
fn pot_plugin_dir(app_data_dir: &Path) -> PathBuf {
    bin_dir(app_data_dir).join("yt-dlp-plugins")
}

/// The script/CLI provider's files, relative to [`pot_plugin_dir`]. The
/// HTTP-server provider (`getpot_bgutil_http.py`) is deliberately not
/// installed — we only ever run bgutil-pot in one-shot CLI mode, and shipping
/// the HTTP provider too would make yt-dlp waste a request probing a server
/// that will never exist (it's tried at higher preference than the CLI one).
const POT_PLUGIN_FILES: &[&str] = &[
    "yt_dlp_plugins/extractor/getpot_bgutil.py",
    "yt_dlp_plugins/extractor/getpot_bgutil_cli.py",
];

fn pot_plugin_files_present(app_data_dir: &Path) -> bool {
    let dir = pot_plugin_dir(app_data_dir);
    POT_PLUGIN_FILES.iter().all(|rel| dir.join(rel).exists())
}

/// Sanity-checks that `program` runs by invoking it with a version argument.
fn runs(program: &Path, version_arg: &str) -> bool {
    silent_command(program)
        .arg(version_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn resolve_ytdlp(app_data_dir: &Path) -> Option<PathBuf> {
    let local = ytdlp_local(app_data_dir);
    if local.exists() && runs(&local, "--version") {
        return Some(local);
    }
    let on_path = PathBuf::from("yt-dlp");
    if runs(&on_path, "--version") {
        return Some(on_path);
    }
    None
}

pub fn resolve_ffmpeg(app_data_dir: &Path) -> Option<PathBuf> {
    let local = ffmpeg_local(app_data_dir);
    if local.exists() && runs(&local, "-version") {
        return Some(local);
    }
    let on_path = PathBuf::from("ffmpeg");
    if runs(&on_path, "-version") {
        return Some(on_path);
    }
    None
}

/// Deno is yt-dlp's JS runtime for YouTube nsig decryption; without it some
/// streams fail to download with `[Errno 22]`.
pub fn resolve_deno(app_data_dir: &Path) -> Option<PathBuf> {
    let local = deno_local(app_data_dir);
    if local.exists() && runs(&local, "--version") {
        return Some(local);
    }
    let on_path = PathBuf::from("deno");
    if runs(&on_path, "--version") {
        return Some(on_path);
    }
    None
}

/// PO-token provider for YouTube's bot-wall. `Some` only when both the binary
/// and its yt-dlp plugin files are present — installed as a unit by
/// [`install_pot`], so a half-finished install just resolves to `None`
/// (downloads fall back to today's behavior) rather than a broken state.
pub fn resolve_pot(app_data_dir: &Path) -> Option<PotProvider> {
    let local = pot_local(app_data_dir);
    if !local.exists() || !runs(&local, "--version") {
        return None;
    }
    if !pot_plugin_files_present(app_data_dir) {
        return None;
    }
    Some(PotProvider {
        cli_path: local,
        plugin_dir: pot_plugin_dir(app_data_dir),
    })
}

pub fn deno_js_runtime_arg(app_data_dir: &Path) -> Option<String> {
    let deno = resolve_deno(app_data_dir)?;
    Some(deno_runtime_value(&deno))
}

/// Backslashes are converted to forward slashes: the path passes through
/// yt-dlp's Python arg parser where `\` can be read as an escape char. Forward
/// slashes work fine on Windows and the drive letter (`C:/…`) is kept as-is
/// (not the MSYS `/c/…` form).
pub fn deno_runtime_value(deno: &Path) -> String {
    format!("deno:{}", normalize_path_arg(deno))
}

pub fn check(app_data_dir: &Path) -> (bool, bool) {
    (
        resolve_ytdlp(app_data_dir).is_some(),
        resolve_ffmpeg(app_data_dir).is_some(),
    )
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("LFMusic/0.1.0 (+https://github.com/mel1x/LFMusic)")
        // Binaries are large (tens of MB) — only bound the connect, not the whole transfer.
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

fn fmt_size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    format!("{:.1} МБ", mb)
}

fn fmt_progress(done: u64, total: Option<u64>) -> String {
    match total {
        Some(t) if t > 0 => format!("{} / {}", fmt_size(done), fmt_size(t)),
        _ => fmt_size(done),
    }
}

/// Downloads to `target` atomically (`.part` → rename), reporting progress.
fn download_to_file(
    url: &str,
    target: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let client = http_client()?;
    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let total = resp.content_length();

    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = target.with_extension("part");
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut downloaded = 0u64;
    loop {
        let n = resp.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;
        progress(downloaded, total);
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(())
}

fn ytdlp_asset_name() -> &'static str {
    #[cfg(windows)]
    {
        "yt-dlp.exe"
    }
    #[cfg(target_os = "macos")]
    {
        "yt-dlp_macos"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "yt-dlp"
    }
}

fn deno_asset_name() -> &'static str {
    let arm = std::env::consts::ARCH == "aarch64";
    #[cfg(windows)]
    {
        if arm {
            "deno-aarch64-pc-windows-msvc.zip"
        } else {
            "deno-x86_64-pc-windows-msvc.zip"
        }
    }
    #[cfg(target_os = "macos")]
    {
        if arm {
            "deno-aarch64-apple-darwin.zip"
        } else {
            "deno-x86_64-apple-darwin.zip"
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if arm {
            "deno-aarch64-unknown-linux-gnu.zip"
        } else {
            "deno-x86_64-unknown-linux-gnu.zip"
        }
    }
}

/// `None` on platforms/architectures with no published release asset (Android,
/// 32-bit, Windows on ARM64) — the tool is simply not offered there.
fn pot_asset_name() -> Option<&'static str> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    return Some("bgutil-pot-windows-x86_64.exe");
    #[cfg(target_os = "macos")]
    return Some(if std::env::consts::ARCH == "aarch64" {
        "bgutil-pot-macos-aarch64"
    } else {
        "bgutil-pot-macos-x86_64"
    });
    #[cfg(target_os = "linux")]
    return Some(if std::env::consts::ARCH == "aarch64" {
        "bgutil-pot-linux-aarch64"
    } else {
        "bgutil-pot-linux-x86_64"
    });
    #[allow(unreachable_code)]
    None
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

fn sha256_hex(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hex_encode(hasher.finalize().as_slice()))
}

/// Fetches the expected SHA256 from a specific yt-dlp release's checksums file,
/// pinned by tag so the checksum and binary come from the same release.
fn fetch_ytdlp_sha256(tag: &str, asset: &str) -> Option<String> {
    // The checksums asset is named `SHA2-256SUMS`, not `SHA256SUMS` — the wrong
    // name 404s and install_ytdlp would discard the binary as unverifiable.
    let url = format!(
        "https://github.com/yt-dlp/yt-dlp/releases/download/{}/SHA2-256SUMS",
        tag
    );
    let client = http_client().ok()?;
    let text = client
        .get(&url)
        .timeout(Duration::from_secs(20))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .text()
        .ok()?;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(hash), Some(name)) = (it.next(), it.next()) else {
            continue;
        };
        // sha256sum binary-mode entries may prefix the name with '*'.
        if name.trim_start_matches('*') == asset {
            return Some(hash.to_lowercase());
        }
    }
    None
}

#[cfg(unix)]
fn make_executable(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(p)
        .map_err(|e| e.to_string())?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(p, perms).map_err(|e| e.to_string())
}

/// Extracts the named binaries from a zip (matched by basename, ignoring
/// internal path) into `bin`, setting +x on Unix. Returns the path to `wanted[0]`.
fn extract_zip_binaries(zip_path: &Path, bin: &Path, wanted: &[&str]) -> Result<PathBuf, String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        let base = name.rsplit('/').next().unwrap_or("").to_string();
        if wanted.contains(&base.as_str()) {
            let out = bin.join(&base);
            let mut out_file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            make_executable(&out)?;
        }
    }
    let primary = bin.join(wanted[0]);
    if primary.exists() {
        Ok(primary)
    } else {
        Err(format!("{} не найден в архиве", wanted[0]))
    }
}

/// Extracts specific entries (matched by full in-archive path, not basename)
/// into `dest_root`, recreating their parent directories. Unlike
/// [`extract_zip_binaries`] this preserves the archive's internal layout —
/// needed for a Python namespace-package plugin, where the
/// `yt_dlp_plugins/extractor/` structure must stay intact for yt-dlp's
/// `--plugin-dirs` to discover it.
fn extract_zip_paths(zip_path: &Path, dest_root: &Path, wanted: &[&str]) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut found = vec![false; wanted.len()];
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        let Some(idx) = wanted.iter().position(|w| *w == name) else {
            continue;
        };
        let out = dest_root.join(&name);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out_file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
        found[idx] = true;
    }
    if let Some(pos) = found.iter().position(|f| !f) {
        return Err(format!("{} не найден в архиве", wanted[pos]));
    }
    Ok(())
}

#[cfg(windows)]
fn download_ffmpeg_impl(
    dest: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, String> {
    let url = if std::env::consts::ARCH == "aarch64" {
        "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-winarm64-gpl.zip"
    } else {
        "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl.zip"
    };
    let zip_path = dest.join("ffmpeg-download.zip");
    download_to_file(url, &zip_path, &mut progress)?;
    let result = extract_zip_binaries(&zip_path, dest, &["ffmpeg.exe", "ffprobe.exe"]);
    let _ = std::fs::remove_file(&zip_path);
    result
}

/// Linux ffmpeg: download .tar.xz, decompress to .tar with pure-Rust lzma
/// (no system liblzma dependency), then extract ffmpeg/ffprobe and chmod +x.
#[cfg(target_os = "linux")]
fn download_ffmpeg_impl(
    dest: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, String> {
    let bin = dest;
    let url = if std::env::consts::ARCH == "aarch64" {
        "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-linuxarm64-gpl.tar.xz"
    } else {
        "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-linux64-gpl.tar.xz"
    };
    let xz_path = bin.join("ffmpeg-download.tar.xz");
    let tar_path = bin.join("ffmpeg-download.tar");
    download_to_file(url, &xz_path, &mut progress)?;

    let result = (|| -> Result<PathBuf, String> {
        // Stream xz -> tar file-to-file rather than buffering ~150MB in memory.
        {
            let xz_file = std::fs::File::open(&xz_path).map_err(|e| e.to_string())?;
            let mut reader = std::io::BufReader::new(xz_file);
            let tar_file = std::fs::File::create(&tar_path).map_err(|e| e.to_string())?;
            let mut writer = std::io::BufWriter::new(tar_file);
            lzma_rs::xz_decompress(&mut reader, &mut writer)
                .map_err(|e| format!("xz decode: {:?}", e))?;
            writer.flush().map_err(|e| e.to_string())?;
        }

        let tar_file = std::fs::File::open(&tar_path).map_err(|e| e.to_string())?;
        let mut archive = tar::Archive::new(std::io::BufReader::new(tar_file));
        let wanted = ["ffmpeg", "ffprobe"];
        for entry in archive.entries().map_err(|e| e.to_string())? {
            let mut entry = entry.map_err(|e| e.to_string())?;
            let base = entry
                .path()
                .map_err(|e| e.to_string())?
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if wanted.contains(&base.as_str()) {
                let out = bin.join(&base);
                let mut out_file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
                make_executable(&out)?;
            }
        }
        let ffmpeg = bin.join("ffmpeg");
        if ffmpeg.exists() {
            Ok(ffmpeg)
        } else {
            Err("ffmpeg не найден в архиве".into())
        }
    })();

    let _ = std::fs::remove_file(&xz_path);
    let _ = std::fs::remove_file(&tar_path);
    result
}

/// macOS ffmpeg from evermeet.cx (Intel builds, run under Rosetta on Apple
/// Silicon). ffprobe is best-effort/optional.
#[cfg(target_os = "macos")]
fn download_ffmpeg_impl(
    dest: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, String> {
    let bin = dest;

    let ffmpeg_zip = bin.join("ffmpeg-download.zip");
    download_to_file(
        "https://evermeet.cx/ffmpeg/getrelease/ffmpeg/zip",
        &ffmpeg_zip,
        &mut progress,
    )?;
    let ffmpeg_res = extract_zip_binaries(&ffmpeg_zip, &bin, &["ffmpeg"]);
    let _ = std::fs::remove_file(&ffmpeg_zip);
    let ffmpeg = ffmpeg_res?;

    let ffprobe_zip = bin.join("ffprobe-download.zip");
    if download_to_file(
        "https://evermeet.cx/ffmpeg/getrelease/ffprobe/zip",
        &ffprobe_zip,
        |_, _| {},
    )
    .is_ok()
    {
        let _ = extract_zip_binaries(&ffprobe_zip, &bin, &["ffprobe"]);
    }
    let _ = std::fs::remove_file(&ffprobe_zip);

    Ok(ffmpeg)
}

fn run_capture(program: &Path, args: &[&str]) -> Option<String> {
    let out = silent_command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn ytdlp_version(path: &Path) -> Option<String> {
    run_capture(path, &["--version"]).filter(|s| !s.is_empty())
}

/// Parses "ffmpeg version <ver> Copyright …" (first token of first line).
fn ffmpeg_version(path: &Path) -> Option<String> {
    let out = run_capture(path, &["-version"])?;
    let first = out.lines().next()?;
    let rest = first.strip_prefix("ffmpeg version ").unwrap_or(first);
    let token = rest.split_whitespace().next()?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Parses "deno <ver> (release …)" (second token of first line).
fn deno_version(path: &Path) -> Option<String> {
    let out = run_capture(path, &["--version"])?;
    let first = out.lines().next()?;
    let token = first
        .strip_prefix("deno ")
        .unwrap_or(first)
        .split_whitespace()
        .next()?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Parses "bgutil-pot <ver>" (second token of first line).
fn pot_version(path: &Path) -> Option<String> {
    let out = run_capture(path, &["--version"])?;
    let first = out.lines().next()?;
    let token = first
        .strip_prefix("bgutil-pot ")
        .unwrap_or(first)
        .split_whitespace()
        .next()?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

pub fn ytdlp_info(app_data_dir: &Path) -> ToolInfo {
    let resolved = resolve_ytdlp(app_data_dir);
    let managed = ytdlp_local(app_data_dir).exists();
    let version = resolved.as_ref().and_then(|p| ytdlp_version(p));
    ToolInfo {
        installed: resolved.is_some(),
        managed,
        version,
        update_available: false,
    }
}

pub fn ffmpeg_info(app_data_dir: &Path) -> ToolInfo {
    let resolved = resolve_ffmpeg(app_data_dir);
    let managed = ffmpeg_local(app_data_dir).exists();
    let version = resolved.as_ref().and_then(|p| ffmpeg_version(p));
    ToolInfo {
        installed: resolved.is_some(),
        managed,
        version,
        update_available: false,
    }
}

/// `update_available` is never set for Deno: it always installs as "latest" alongside yt-dlp.
pub fn deno_info(app_data_dir: &Path) -> ToolInfo {
    let resolved = resolve_deno(app_data_dir);
    let managed = deno_local(app_data_dir).exists();
    let version = resolved.as_ref().and_then(|p| deno_version(p));
    ToolInfo {
        installed: resolved.is_some(),
        managed,
        version,
        update_available: false,
    }
}

/// `update_available` is filled in separately by `check_updates` (needs a
/// network round-trip); a plain `info()` call stays offline like the others.
pub fn pot_info(app_data_dir: &Path) -> ToolInfo {
    let resolved = resolve_pot(app_data_dir);
    let managed = pot_local(app_data_dir).exists();
    let version = resolved.as_ref().and_then(|p| pot_version(&p.cli_path));
    ToolInfo {
        installed: resolved.is_some(),
        managed,
        version,
        update_available: false,
    }
}

pub fn info(app_data_dir: &Path) -> ToolsInfo {
    ToolsInfo {
        ytdlp: ytdlp_info(app_data_dir),
        ffmpeg: ffmpeg_info(app_data_dir),
        deno: deno_info(app_data_dir),
        pot: pot_info(app_data_dir),
    }
}

pub fn check_updates(app_data_dir: &Path) -> ToolsInfo {
    let mut out = info(app_data_dir);

    // Versions are YYYY.MM.DD, so lexicographic compare matches chronological.
    // Checked for any installed yt-dlp (including system PATH), not just
    // managed: an update lands in the app bin dir and resolve then prefers it.
    if out.ytdlp.installed {
        if let (Some(cur), Some(latest)) = (out.ytdlp.version.clone(), fetch_ytdlp_latest()) {
            out.ytdlp.update_available = cur.as_str() < latest.as_str();
        }
    }

    // ffmpeg has no parseable version (rolling build), so we compare the
    // asset's updated_at marker saved at download time against the current one.
    if out.ffmpeg.managed {
        let stored = read_meta(app_data_dir)
            .get("ffmpeg")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let (Some(stored), Some(latest)) = (stored, fetch_ffmpeg_latest()) {
            out.ffmpeg.update_available = stored != latest;
        }
    }

    // Simple inequality (not `<`) to sidestep semver-ordering bugs: this repo's
    // tags aren't the sortable YYYY.MM.DD scheme yt-dlp uses.
    if out.pot.managed {
        let stored = read_meta(app_data_dir)
            .get("pot")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let (Some(stored), Some(latest)) = (stored, fetch_pot_latest()) {
            out.pot.update_available = stored != latest;
        }
    }

    out
}

fn fetch_json(url: &str) -> Option<serde_json::Value> {
    let client = http_client().ok()?;
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(15))
        .send()
        .ok()?
        .error_for_status()
        .ok()?;
    resp.json::<serde_json::Value>().ok()
}

fn fetch_ytdlp_latest() -> Option<String> {
    let v = fetch_json("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")?;
    v.get("tag_name")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn fetch_pot_latest() -> Option<String> {
    let v = fetch_json(
        "https://api.github.com/repos/jim60105/bgutil-ytdlp-pot-provider-rs/releases/latest",
    )?;
    v.get("tag_name")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Windows/Linux ffmpeg freshness marker: `updated_at` of the matching asset
/// in the yt-dlp/FFmpeg-Builds rolling "latest" release.
#[cfg(any(windows, target_os = "linux"))]
fn fetch_ffmpeg_latest() -> Option<String> {
    let want = {
        #[cfg(windows)]
        {
            if std::env::consts::ARCH == "aarch64" {
                "ffmpeg-master-latest-winarm64-gpl.zip"
            } else {
                "ffmpeg-master-latest-win64-gpl.zip"
            }
        }
        #[cfg(target_os = "linux")]
        {
            if std::env::consts::ARCH == "aarch64" {
                "ffmpeg-master-latest-linuxarm64-gpl.tar.xz"
            } else {
                "ffmpeg-master-latest-linux64-gpl.tar.xz"
            }
        }
    };
    let v = fetch_json("https://api.github.com/repos/yt-dlp/FFmpeg-Builds/releases/tags/latest")?;
    let assets = v.get("assets")?.as_array()?;
    for a in assets {
        if a.get("name").and_then(|n| n.as_str()) == Some(want) {
            return a
                .get("updated_at")
                .and_then(|u| u.as_str())
                .map(|s| s.to_string());
        }
    }
    None
}

/// macOS ffmpeg comes from evermeet.cx, not GitHub — freshness marker is the
/// `version` field from its public release info API.
#[cfg(not(any(windows, target_os = "linux")))]
fn fetch_ffmpeg_latest() -> Option<String> {
    let v = fetch_json("https://evermeet.cx/ffmpeg/info/ffmpeg/release")?;
    v.get("version")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Scratch subfolder under bin used for download+verify before the atomic
/// swap; wiped before use to avoid leftovers from a previous failed attempt.
fn staging_dir(bin: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = bin.join(format!(".staging-{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// RAII wrapper that removes the staging dir on any exit path (success, `Err`,
/// or panic) — manual cleanup at each early return used to leave orphaned
/// `.staging-*` dirs behind on a panic during download/extract. Final files
/// are moved out via `swap_in_files` before drop, so removing the rest is safe.
struct StagingDir {
    path: PathBuf,
}

impl StagingDir {
    fn new(bin: &Path, name: &str) -> Result<Self, String> {
        Ok(Self {
            path: staging_dir(bin, name)?,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn backup_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    target.with_file_name(format!("{}.bak", name))
}

/// Atomically installs already-verified staged files: existing `target` is
/// moved aside to `.bak`, then staged is renamed into place (same-filesystem
/// rename is atomic). Any failure rolls back all swaps done so far from their
/// backups, so an update either fully succeeds or leaves the install untouched.
fn swap_in_files(pairs: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    fn rollback(done: &[(PathBuf, Option<PathBuf>)]) {
        for (target, bak) in done.iter().rev() {
            let _ = std::fs::remove_file(target);
            if let Some(b) = bak {
                let _ = std::fs::rename(b, target);
            }
        }
    }

    let mut done: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    for (staged, target) in pairs {
        if !staged.exists() {
            rollback(&done);
            return Err(format!(
                "подготовленный файл отсутствует: {}",
                staged.display()
            ));
        }
        let bak = if target.exists() {
            let b = backup_path(target);
            let _ = std::fs::remove_file(&b);
            if let Err(e) = std::fs::rename(target, &b) {
                rollback(&done);
                return Err(format!("не удалось отвести старый файл: {}", e));
            }
            Some(b)
        } else {
            None
        };
        if let Err(e) = std::fs::rename(staged, target) {
            // Restore the backup we just moved aside, then roll back the rest.
            if let Some(b) = &bak {
                let _ = std::fs::rename(b, target);
            }
            rollback(&done);
            return Err(format!("не удалось установить новый файл: {}", e));
        }
        done.push((target.clone(), bak));
    }
    // Best-effort cleanup; a locked old exe will get removed later via retry.
    for (_, bak) in &done {
        if let Some(b) = bak {
            let _ = remove_file_retry(b);
        }
    }
    Ok(())
}

/// (Re)installs yt-dlp pinned to a specific release tag (not `latest/download`),
/// verified against that same release's SHA256 checksum. Download+verify happen
/// in staging; only a verified binary is atomically swapped into bin.
pub fn install_ytdlp(
    app_data_dir: &Path,
    report: &(dyn Fn(String) + Sync),
) -> Result<ToolInfo, String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tag =
        fetch_ytdlp_latest().ok_or_else(|| "Не удалось определить версию yt-dlp".to_string())?;
    let asset = ytdlp_asset_name();

    // Fetch the checksum before downloading the binary — no point pulling
    // down tens of MB if we can't verify it anyway.
    report("Получение контрольной суммы yt-dlp…".to_string());
    let expected = fetch_ytdlp_sha256(&tag, asset)
        .ok_or_else(|| "Не удалось получить контрольную сумму yt-dlp для проверки".to_string())?;

    let url = format!(
        "https://github.com/yt-dlp/yt-dlp/releases/download/{}/{}",
        tag, asset
    );
    let target = ytdlp_local(app_data_dir);
    let bin = bin_dir(app_data_dir);
    let staging = StagingDir::new(&bin, "ytdlp")?;
    let local_name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("yt-dlp")
        .to_string();
    let staged = staging.path().join(&local_name);

    report(format!("Скачивание yt-dlp {}…", tag));
    if let Err(e) = download_to_file(&url, &staged, |d, t| {
        report(format!("Скачивание yt-dlp {}… {}", tag, fmt_progress(d, t)))
    }) {
        return Err(format!("Не удалось скачать yt-dlp: {}", e));
    }

    report("Проверка контрольной суммы yt-dlp…".to_string());
    let actual = sha256_hex(&staged)
        .map_err(|e| format!("Не удалось посчитать контрольную сумму: {}", e))?;
    if actual != expected {
        return Err("Контрольная сумма yt-dlp не совпала — файл отклонён".to_string());
    }

    #[cfg(unix)]
    make_executable(&staged)?;
    if !runs(&staged, "--version") {
        return Err("yt-dlp скачан, но не запускается".to_string());
    }

    swap_in_files(&[(staged, target)])
        .map_err(|e| format!("Не удалось установить yt-dlp: {}", e))?;

    write_meta_field(app_data_dir, "ytdlp", &tag);
    Ok(ytdlp_info(app_data_dir))
}

/// (Re)installs ffmpeg into bin, verified by a successful run before the
/// atomic swap (see [`swap_in_files`]).
pub fn install_ffmpeg(
    app_data_dir: &Path,
    report: &(dyn Fn(String) + Sync),
) -> Result<ToolInfo, String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        let bin = bin_dir(app_data_dir);
        let staging = StagingDir::new(&bin, "ffmpeg")?;

        report("Скачивание ffmpeg…".to_string());
        let staged_ffmpeg = download_ffmpeg_impl(staging.path(), |d, t| {
            report(format!("Скачивание ffmpeg… {}", fmt_progress(d, t)))
        })
        .map_err(|e| format!("Не удалось скачать ffmpeg: {}", e))?;
        if !runs(&staged_ffmpeg, "-version") {
            return Err("ffmpeg скачан, но не запускается".to_string());
        }

        // ffmpeg and ffprobe (if present) swap in as a single transaction.
        let ff_name = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        let fp_name = if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        };
        let mut pairs = vec![(staging.path().join(ff_name), bin.join(ff_name))];
        if staging.path().join(fp_name).exists() {
            pairs.push((staging.path().join(fp_name), bin.join(fp_name)));
        }
        swap_in_files(&pairs).map_err(|e| format!("Не удалось установить ffmpeg: {}", e))?;

        if let Some(tag) = fetch_ffmpeg_latest() {
            write_meta_field(app_data_dir, "ffmpeg", &tag);
        }
        Ok(ffmpeg_info(app_data_dir))
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = report;
        Err("Автоматическая установка ffmpeg не поддерживается на этой платформе".to_string())
    }
}

/// On Windows a just-exited yt-dlp/Deno process (or one still checking cookies
/// in the background) can hold the .exe locked for a moment — `remove_file`
/// fails with os error 5/32. Retries for ~6s; a missing file counts as success.
fn remove_file_retry(path: &Path) -> std::io::Result<()> {
    let mut last_err: Option<std::io::Error> = None;
    for _ in 0..20 {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(Duration::from_millis(300));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "remove failed")))
}

pub fn delete_ytdlp(app_data_dir: &Path) -> Result<(), String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let p = ytdlp_local(app_data_dir);
    if p.exists() {
        remove_file_retry(&p).map_err(|e| {
            format!("Не удалось удалить yt-dlp: {} (файл занят — дождитесь окончания скачивания/проверки cookies)", e)
        })?;
    }
    Ok(())
}

pub fn delete_ffmpeg(app_data_dir: &Path) -> Result<(), String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let bin = bin_dir(app_data_dir);
    // Don't swallow delete errors: a locked binary would leave the tool
    // "partially installed" while the caller thinks removal succeeded.
    let mut errors: Vec<String> = Vec::new();
    for name in ["ffmpeg", "ffmpeg.exe", "ffprobe", "ffprobe.exe"] {
        let p = bin.join(name);
        if p.exists() {
            if let Err(e) = remove_file_retry(&p) {
                errors.push(format!("{}: {}", name, e));
            }
        }
    }
    if !errors.is_empty() {
        return Err(format!("Не удалось удалить ffmpeg: {}", errors.join("; ")));
    }
    remove_meta_field(app_data_dir, "ffmpeg");
    Ok(())
}

/// Idempotent: skips the download if a working Deno is already resolvable
/// (bin dir or system PATH). Lifecycle is tied to yt-dlp (installed/removed together).
pub fn install_deno(app_data_dir: &Path, report: &(dyn Fn(String) + Sync)) -> Result<(), String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    if resolve_deno(app_data_dir).is_some() {
        return Ok(());
    }

    let bin = bin_dir(app_data_dir);
    let staging = StagingDir::new(&bin, "deno")?;
    let asset = deno_asset_name();
    let url = format!(
        "https://github.com/denoland/deno/releases/latest/download/{}",
        asset
    );
    let zip_path = staging.path().join("deno-download.zip");

    report("Скачивание Deno (JS-движок для YouTube)…".to_string());
    if let Err(e) = download_to_file(&url, &zip_path, |d, t| {
        report(format!(
            "Скачивание Deno (JS-движок для YouTube)… {}",
            fmt_progress(d, t)
        ))
    }) {
        return Err(format!("Не удалось скачать Deno: {}", e));
    }

    let bin_name = if cfg!(windows) { "deno.exe" } else { "deno" };
    let staged = extract_zip_binaries(&zip_path, staging.path(), &[bin_name])
        .map_err(|e| format!("Не удалось распаковать Deno: {}", e))?;
    let _ = std::fs::remove_file(&zip_path);

    #[cfg(unix)]
    make_executable(&staged)?;
    if !runs(&staged, "--version") {
        return Err("Deno скачан, но не запускается".to_string());
    }

    swap_in_files(&[(staged, deno_local(app_data_dir))])
        .map_err(|e| format!("Не удалось установить Deno: {}", e))?;
    Ok(())
}

/// Only removes the app-managed copy in the bin dir; a system Deno on PATH is left alone.
pub fn delete_deno(app_data_dir: &Path) -> Result<(), String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let p = deno_local(app_data_dir);
    if p.exists() {
        remove_file_retry(&p).map_err(|e| {
            format!("Не удалось удалить Deno: {} (файл занят — дождитесь окончания скачивания/проверки cookies)", e)
        })?;
    }
    Ok(())
}

/// (Re)installs the PO-token provider (bgutil-pot) for YouTube's bot-wall —
/// script/CLI mode only, no persistent server (see [`PotProvider`]). Two
/// artifacts installed as a unit: the executable itself (staged, verified by
/// running, atomically swapped in like the other tools) and its yt-dlp plugin
/// (plain files, no lock-contention risk, written directly). No SHA256 is
/// published upstream for this repo — same trust level already accepted for
/// ffmpeg/Deno, verified by successfully running `--version` instead.
pub fn install_pot(app_data_dir: &Path, report: &(dyn Fn(String) + Sync)) -> Result<ToolInfo, String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let asset = pot_asset_name().ok_or_else(|| {
        "Обход бот-проверки YouTube не поддерживается на этой платформе".to_string()
    })?;

    report("Получение версии bgutil-pot…".to_string());
    let tag =
        fetch_pot_latest().ok_or_else(|| "Не удалось определить версию bgutil-pot".to_string())?;

    let bin = bin_dir(app_data_dir);
    let staging = StagingDir::new(&bin, "pot")?;

    let url = format!(
        "https://github.com/jim60105/bgutil-ytdlp-pot-provider-rs/releases/download/{}/{}",
        tag, asset
    );
    let local_name = pot_local(app_data_dir)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("bgutil-pot")
        .to_string();
    let staged = staging.path().join(&local_name);

    report(format!("Скачивание bgutil-pot {}…", tag));
    download_to_file(&url, &staged, |d, t| {
        report(format!(
            "Скачивание bgutil-pot {}… {}",
            tag,
            fmt_progress(d, t)
        ))
    })
    .map_err(|e| format!("Не удалось скачать bgutil-pot: {}", e))?;

    #[cfg(unix)]
    make_executable(&staged)?;
    if !runs(&staged, "--version") {
        return Err("bgutil-pot скачан, но не запускается".to_string());
    }

    report("Скачивание плагина yt-dlp…".to_string());
    let plugin_zip_url = format!(
        "https://github.com/jim60105/bgutil-ytdlp-pot-provider-rs/releases/download/{}/bgutil-ytdlp-pot-provider-rs.zip",
        tag
    );
    let plugin_zip = staging.path().join("plugin.zip");
    download_to_file(&plugin_zip_url, &plugin_zip, |_, _| {})
        .map_err(|e| format!("Не удалось скачать плагин yt-dlp: {}", e))?;
    extract_zip_paths(&plugin_zip, &pot_plugin_dir(app_data_dir), POT_PLUGIN_FILES)
        .map_err(|e| format!("Не удалось распаковать плагин yt-dlp: {}", e))?;

    // Only swap the live binary in once the plugin is confirmed in place too —
    // a plugin-side failure above leaves the previous (or no) exe untouched.
    swap_in_files(&[(staged, pot_local(app_data_dir))])
        .map_err(|e| format!("Не удалось установить bgutil-pot: {}", e))?;

    write_meta_field(app_data_dir, "pot", &tag);
    Ok(pot_info(app_data_dir))
}

/// Removes both the binary and its yt-dlp plugin directory — installed and
/// removed as a unit, like Deno is paired with yt-dlp.
pub fn delete_pot(app_data_dir: &Path) -> Result<(), String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let p = pot_local(app_data_dir);
    if p.exists() {
        remove_file_retry(&p).map_err(|e| format!("Не удалось удалить bgutil-pot: {}", e))?;
    }
    let plugin_dir = pot_plugin_dir(app_data_dir);
    if plugin_dir.exists() {
        std::fs::remove_dir_all(&plugin_dir)
            .map_err(|e| format!("Не удалось удалить плагин yt-dlp: {}", e))?;
    }
    remove_meta_field(app_data_dir, "pot");
    Ok(())
}

fn meta_path(app_data_dir: &Path) -> PathBuf {
    bin_dir(app_data_dir).join("tools_meta.json")
}

fn read_meta(app_data_dir: &Path) -> serde_json::Value {
    std::fs::read_to_string(meta_path(app_data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn write_meta_field(app_data_dir: &Path, key: &str, value: &str) {
    let mut m = read_meta(app_data_dir);
    if !m.is_object() {
        m = serde_json::json!({});
    }
    if let Some(obj) = m.as_object_mut() {
        obj.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
    let _ = std::fs::write(meta_path(app_data_dir), m.to_string());
}

fn remove_meta_field(app_data_dir: &Path, key: &str) {
    let mut m = read_meta(app_data_dir);
    if let Some(obj) = m.as_object_mut() {
        if obj.remove(key).is_some() {
            let _ = std::fs::write(meta_path(app_data_dir), m.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(test_name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("lfmusic_tools_{test_name}_{unique}"))
    }

    #[test]
    fn fmt_size_formats_megabytes_with_one_decimal() {
        assert_eq!(fmt_size(0), "0.0 МБ");
        assert_eq!(fmt_size(1024 * 1024), "1.0 МБ");
        assert_eq!(fmt_size(1024 * 1024 * 3 / 2), "1.5 МБ");
    }

    #[test]
    fn fmt_progress_shows_total_when_known_and_size_only_otherwise() {
        assert_eq!(fmt_progress(1024 * 1024, Some(2 * 1024 * 1024)), "1.0 МБ / 2.0 МБ");
        assert_eq!(fmt_progress(1024 * 1024, None), "1.0 МБ");
        assert_eq!(fmt_progress(1024 * 1024, Some(0)), "1.0 МБ");
    }

    #[test]
    fn hex_encode_matches_known_vectors() {
        assert_eq!(hex_encode(&[]), "");
        assert_eq!(hex_encode(&[0x00]), "00");
        assert_eq!(hex_encode(&[0xDE, 0xAD, 0xBE, 0xEF]), "deadbeef");
    }

    #[test]
    fn sha256_hex_matches_known_vector_for_empty_file() {
        let dir = temp_dir("sha256_empty");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("empty.bin");
        std::fs::write(&file, b"").unwrap();
        // SHA256("") — well-known test vector (FIPS 180 / NIST).
        assert_eq!(
            sha256_hex(&file).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_hex_matches_known_vector_for_abc() {
        let dir = temp_dir("sha256_abc");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("abc.bin");
        std::fs::write(&file, b"abc").unwrap();
        // SHA256("abc") — standard NIST test vector.
        assert_eq!(
            sha256_hex(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_hex_errors_for_missing_file() {
        let dir = temp_dir("sha256_missing");
        assert!(sha256_hex(&dir.join("nope.bin")).is_err());
    }

    #[test]
    fn deno_runtime_value_normalizes_windows_backslashes() {
        let path = Path::new("C:\\Users\\melix\\AppData\\bin\\deno.exe");
        assert_eq!(
            deno_runtime_value(path),
            "deno:C:/Users/melix/AppData/bin/deno.exe"
        );
    }

    #[test]
    fn deno_runtime_value_preserves_forward_slash_paths() {
        let path = Path::new("/home/user/bin/deno");
        assert_eq!(deno_runtime_value(path), "deno:/home/user/bin/deno");
    }

    #[test]
    fn backup_path_appends_bak_suffix_to_file_name() {
        let target = Path::new("C:/app/bin/yt-dlp.exe");
        assert_eq!(backup_path(target), PathBuf::from("C:/app/bin/yt-dlp.exe.bak"));
    }

    #[test]
    fn meta_round_trips_through_write_read_remove() {
        let dir = temp_dir("meta_roundtrip");
        write_meta_field(&dir, "ytdlp", "2025.06.09");
        let meta = read_meta(&dir);
        assert_eq!(meta["ytdlp"], "2025.06.09");

        write_meta_field(&dir, "ffmpeg", "2025-01-01T00:00:00Z");
        let meta2 = read_meta(&dir);
        assert_eq!(meta2["ytdlp"], "2025.06.09");
        assert_eq!(meta2["ffmpeg"], "2025-01-01T00:00:00Z");

        remove_meta_field(&dir, "ytdlp");
        let meta3 = read_meta(&dir);
        assert!(meta3.get("ytdlp").is_none());
        assert_eq!(meta3["ffmpeg"], "2025-01-01T00:00:00Z");

        let _ = std::fs::remove_dir_all(bin_dir(&dir));
    }

    #[test]
    fn read_meta_on_missing_file_returns_empty_object() {
        let dir = temp_dir("meta_missing");
        let meta = read_meta(&dir);
        assert!(meta.is_object());
        assert_eq!(meta.as_object().unwrap().len(), 0);
    }

    #[test]
    fn resolve_ytdlp_returns_none_when_nothing_installed_or_on_path_name_clash() {
        let dir = temp_dir("resolve_empty");
        // No local bin/yt-dlp.exe created. A system yt-dlp may or may not exist
        // depending on the machine, so we only assert no panic + a consistent Option.
        let _ = resolve_ytdlp(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ytdlp_info_reports_not_managed_when_bin_dir_is_empty() {
        let dir = temp_dir("info_empty");
        let info = ytdlp_info(&dir);
        assert!(!info.managed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swap_in_files_installs_new_file_when_target_absent() {
        let dir = temp_dir("swap_fresh");
        std::fs::create_dir_all(&dir).unwrap();
        let staged = dir.join("staged.txt");
        std::fs::write(&staged, b"new content").unwrap();
        let target = dir.join("target.txt");

        swap_in_files(&[(staged.clone(), target.clone())]).expect("swap should succeed");

        assert!(!staged.exists(), "staged файл должен быть перемещён");
        assert_eq!(std::fs::read(&target).unwrap(), b"new content");
        assert!(!backup_path(&target).exists(), "бэкап не нужен — старого файла не было");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swap_in_files_replaces_existing_target_and_cleans_up_backup() {
        let dir = temp_dir("swap_replace");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("target.txt");
        std::fs::write(&target, b"old content").unwrap();
        let staged = dir.join("staged.txt");
        std::fs::write(&staged, b"new content").unwrap();

        swap_in_files(&[(staged.clone(), target.clone())]).expect("swap should succeed");

        assert_eq!(std::fs::read(&target).unwrap(), b"new content");
        assert!(!backup_path(&target).exists(), "бэкап должен удаляться на успехе");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swap_in_files_rolls_back_all_pairs_when_one_staged_file_is_missing() {
        let dir = temp_dir("swap_rollback");
        std::fs::create_dir_all(&dir).unwrap();

        let target_a = dir.join("a.txt");
        std::fs::write(&target_a, b"old a").unwrap();
        let staged_a = dir.join("staged_a.txt");
        std::fs::write(&staged_a, b"new a").unwrap();

        let target_b = dir.join("b.txt");
        std::fs::write(&target_b, b"old b").unwrap();
        let staged_b = dir.join("staged_b_missing.txt"); // intentionally not created

        let result = swap_in_files(&[
            (staged_a.clone(), target_a.clone()),
            (staged_b, target_b.clone()),
        ]);

        assert!(result.is_err(), "должна вернуться ошибка из-за отсутствующего staged-файла");
        assert_eq!(
            std::fs::read(&target_a).unwrap(),
            b"old a",
            "после отката первый таргет должен вернуться к исходному содержимому"
        );
        assert_eq!(std::fs::read(&target_b).unwrap(), b"old b", "второй таргет не должен был тронуться");
        assert!(!backup_path(&target_a).exists(), "бэкап должен быть подчищен после отката");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn build_test_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            zip.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut zip, data).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn extract_zip_binaries_pulls_wanted_files_to_bin_dir() {
        let dir = temp_dir("zip_extract");
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("bundle.zip");
        build_test_zip(
            &zip_path,
            &[
                ("ffmpeg.exe", b"ffmpeg-bytes"),
                ("ffprobe.exe", b"ffprobe-bytes"),
                ("README.txt", b"ignored"),
            ],
        );

        let primary =
            extract_zip_binaries(&zip_path, &dir, &["ffmpeg.exe", "ffprobe.exe"]).expect("extract");
        assert_eq!(primary, dir.join("ffmpeg.exe"));
        assert_eq!(std::fs::read(dir.join("ffmpeg.exe")).unwrap(), b"ffmpeg-bytes");
        assert_eq!(std::fs::read(dir.join("ffprobe.exe")).unwrap(), b"ffprobe-bytes");
        assert!(!dir.join("README.txt").exists(), "не запрошенные файлы не извлекаются");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_zip_binaries_handles_nested_paths_by_basename() {
        let dir = temp_dir("zip_nested");
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("bundle.zip");
        build_test_zip(&zip_path, &[("ffmpeg-7.0/bin/ffmpeg.exe", b"nested-bytes")]);

        let primary = extract_zip_binaries(&zip_path, &dir, &["ffmpeg.exe"]).expect("extract");
        assert_eq!(primary, dir.join("ffmpeg.exe"));
        assert_eq!(std::fs::read(&primary).unwrap(), b"nested-bytes");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_zip_binaries_errors_when_primary_target_is_absent() {
        let dir = temp_dir("zip_missing_primary");
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("bundle.zip");
        build_test_zip(&zip_path, &[("unrelated.txt", b"x")]);

        let result = extract_zip_binaries(&zip_path, &dir, &["ffmpeg.exe"]);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
