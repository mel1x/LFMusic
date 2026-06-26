//! Резолвер внешних утилит **yt-dlp** и **ffmpeg** для страницы «Скачивание».
//!
//! Резолв (`resolve_ytdlp`/`resolve_ffmpeg`): bin-папка приложения
//! (`<app_data_dir>/bin`) → системный `PATH`. Автоскачивания при резолве нет —
//! установка только явная, со страницы настроек (`install_ytdlp`/`install_ffmpeg`,
//! у yt-dlp с привязкой к версии и проверкой контрольной суммы). Так воркер
//! скачивания не тянет бинари из сети молча (supply-chain).
//!
//! - **yt-dlp** распространяется одним бинарём — качается напрямую с GitHub
//!   releases под текущую платформу (обязателен для скачивания треков).
//! - **ffmpeg** ставится из готовых сборок под ОС: Windows/Linux —
//!   `yt-dlp/FFmpeg-Builds` (zip / `tar.xz`), macOS — `evermeet.cx` (zip). Он
//!   опционален: большинство аудиопотоков скачиваются и без него (с ним — лучший
//!   выбор форматов и ремукс в поддерживаемый библиотекой контейнер).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;

/// Сериализуем подготовку утилит: воркер очереди, кнопка «Установить» на странице
/// настроек и обновление могут вызвать скачивание одновременно — без блокировки
/// они писали бы в один `.part` и портили бинарь. Конкурентный вызов дождётся и
/// найдёт уже скачанное.
static ENSURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Резолвнутые пути к утилитам. `ffmpeg`/`deno` опциональны.
pub struct ResolvedTools {
    pub ytdlp: PathBuf,
    pub ffmpeg: Option<PathBuf>,
    /// JS-движок для yt-dlp (расшифровка nsig YouTube). Без него часть форматов
    /// недоступна.
    pub deno: Option<PathBuf>,
    /// Файл cookies (Netscape) для yt-dlp (`--cookies`), если пользователь
    /// авторизовался в YouTube.
    pub cookies_file: Option<PathBuf>,
}

/// Информация об одной внешней утилите (для страницы настроек/уведомлений).
#[derive(Debug, Clone, Serialize, Default)]
pub struct ToolInfo {
    /// Утилита доступна (в bin-папке приложения или в системном `PATH`).
    pub installed: bool,
    /// Установлена приложением (лежит в bin-папке) — её можно обновлять/удалять.
    pub managed: bool,
    /// Версия установленной утилиты (человекочитаемая), если удалось получить.
    pub version: Option<String>,
    /// Доступна более новая версия. Для yt-dlp определяется и для системного
    /// (PATH) — обновление скачается как управляемое приложением; для ffmpeg —
    /// только для managed (версию системной сборки надёжно не сравнить).
    pub update_available: bool,
}

/// Сведения об утилитах. Deno ставится/удаляется вместе с yt-dlp, но статус несём
/// полноценным `ToolInfo` (а не голым флагом): UI должен видеть `managed`, чтобы
/// разрешить удаление app-managed Deno в сценарии «yt-dlp из PATH + Deno от
/// приложения», а также показать версию.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ToolsInfo {
    pub ytdlp: ToolInfo,
    pub ffmpeg: ToolInfo,
    /// JS-движок Deno (нужен yt-dlp для YouTube).
    pub deno: ToolInfo,
}

/// Создаёт `Command` без всплывающего консольного окна на Windows.
pub fn silent_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    // Примечание: yt-dlp (frozen-exe) принудительно ставит кодировку stdout по
    // локали (на Windows — cp1251) и игнорирует PYTHONIOENCODING/PYTHONUTF8.
    // Поэтому кодировку не навязываем здесь, а делаем чтение его stdout/stderr
    // устойчивым к не-UTF-8 (потерьное декодирование в downloads.rs) — иначе
    // строгий UTF-8-ридер обрывался бы на кириллице и yt-dlp падал с [Errno 22].
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW — чтобы при каждом запуске yt-dlp/ffmpeg не мигала
        // консоль (приложение с decorations:false, окна быть не должно).
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
    bin_dir(app_data_dir).join(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" })
}

fn ffmpeg_local(app_data_dir: &Path) -> PathBuf {
    bin_dir(app_data_dir).join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" })
}

fn deno_local(app_data_dir: &Path) -> PathBuf {
    bin_dir(app_data_dir).join(if cfg!(windows) { "deno.exe" } else { "deno" })
}

/// Запускается ли программа с аргументом версии (проверка работоспособности).
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

/// Находит рабочий yt-dlp (bin-папка → PATH), без скачивания.
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

/// Находит рабочий ffmpeg (bin-папка → PATH), без скачивания.
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

/// Находит рабочий Deno (bin-папка → PATH), без скачивания. Deno — JS-движок для
/// yt-dlp (расшифровка nsig YouTube); без него часть форматов недоступна и для
/// некоторых потоков скачивание падает с `[Errno 22]`.
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

/// Аргумент `--js-runtimes deno:<путь>` для yt-dlp, если Deno доступен. Наш бинарь
/// лежит в bin-папке (не в PATH), поэтому передаём абсолютный путь явно.
pub fn deno_js_runtime_arg(app_data_dir: &Path) -> Option<String> {
    let deno = resolve_deno(app_data_dir)?;
    Some(deno_runtime_value(&deno))
}

/// Значение `deno:<путь>` для `--js-runtimes`. Обратные слэши Windows заменяем на
/// прямые: внутри yt-dlp путь проходит через парсер аргументов Python, где `\`
/// может быть истолкован как экранирующий символ. Прямые слэши Windows принимает
/// штатно, а драйв-литера (`C:/…`) сохраняется (это НЕ MSYS-форма `/c/…`).
pub fn deno_runtime_value(deno: &Path) -> String {
    format!("deno:{}", deno.to_string_lossy().replace('\\', "/"))
}

/// Текущая доступность утилит (без скачивания) — для статуса на странице.
pub fn check(app_data_dir: &Path) -> (bool, bool) {
    (
        resolve_ytdlp(app_data_dir).is_some(),
        resolve_ffmpeg(app_data_dir).is_some(),
    )
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("LFMusic/0.1.0 (+https://github.com/mel1x/LFMusic)")
        // Бинари крупные (десятки МБ) — общий таймаут не ставим, только на коннект.
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

/// Качает файл по URL в `target` (атомарно: `.part` → rename), сообщая прогресс.
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

/// Имя ассета yt-dlp под текущую платформу (как в релизе на GitHub).
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

/// Имя zip-ассета Deno под текущую платформу (как в релизе на GitHub).
fn deno_asset_name() -> &'static str {
    let arm = std::env::consts::ARCH == "aarch64";
    #[cfg(windows)]
    {
        if arm { "deno-aarch64-pc-windows-msvc.zip" } else { "deno-x86_64-pc-windows-msvc.zip" }
    }
    #[cfg(target_os = "macos")]
    {
        if arm { "deno-aarch64-apple-darwin.zip" } else { "deno-x86_64-apple-darwin.zip" }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if arm { "deno-aarch64-unknown-linux-gnu.zip" } else { "deno-x86_64-unknown-linux-gnu.zip" }
    }
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

/// SHA256 файла в hex (нижний регистр).
fn sha256_hex(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hex_encode(hasher.finalize().as_slice()))
}

/// Ожидаемый SHA256 ассета из файла `SHA256SUMS` конкретного релиза yt-dlp.
/// Привязка к версии (tag) гарантирует, что сумма и бинарь — из одного релиза.
fn fetch_ytdlp_sha256(tag: &str, asset: &str) -> Option<String> {
    // ВАЖНО: ассет с суммами называется `SHA2-256SUMS` (а НЕ `SHA256SUMS`).
    // С неверным именем GitHub отдавал 404 — сумма не находилась, и install_ytdlp
    // удалял уже скачанный бинарь, считая его непроверяемым.
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
        // Формат sha256sum может пометить бинарный режим звёздочкой перед именем.
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

/// Извлекает указанные бинарники из zip-архива (по базовому имени файла) в
/// bin-папку. На Unix проставляет +x. Возвращает путь к первому из `wanted`.
/// Доступно на всех десктоп-платформах: ffmpeg-zip (Windows/macOS) и Deno-zip
/// (Windows/macOS/Linux) распаковываются одинаково.
fn extract_zip_binaries(
    zip_path: &Path,
    bin: &Path,
    wanted: &[&str],
) -> Result<PathBuf, String> {
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

/// Качает ffmpeg (Windows: zip из yt-dlp/FFmpeg-Builds) и распаковывает
/// `ffmpeg.exe`/`ffprobe.exe` в bin-папку.
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

/// Качает ffmpeg (Linux: tar.xz из yt-dlp/FFmpeg-Builds). Поток: скачиваем .tar.xz
/// → распаковываем xz на диск во .tar (чистый Rust, без системного liblzma) →
/// извлекаем из tar `ffmpeg`/`ffprobe` в bin-папку и ставим +x.
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
        // xz → tar (стримим файл→файл, чтобы не держать ~150 МБ в памяти).
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

/// Качает ffmpeg (macOS: zip с evermeet.cx — по одиночному бинарю на архив).
/// Сборки Intel; на Apple Silicon запускаются через Rosetta. ffprobe — опционально.
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

    // ffprobe — best-effort (для нашего сценария не критичен).
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

// ──────────────────────────────────────────────────────────────────────────────
// Версии / статус утилит (страница настроек, уведомления)
// ──────────────────────────────────────────────────────────────────────────────

/// Запускает программу и возвращает её stdout (trim), если завершилась успешно.
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

/// Версия yt-dlp (`--version` → строка вида `2025.06.09`).
fn ytdlp_version(path: &Path) -> Option<String> {
    run_capture(path, &["--version"]).filter(|s| !s.is_empty())
}

/// Версия ffmpeg: первый токен первой строки `-version`
/// («ffmpeg version <ver> Copyright …»).
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

/// Версия Deno: второй токен первой строки `--version` («deno <ver> (release …)»).
fn deno_version(path: &Path) -> Option<String> {
    let out = run_capture(path, &["--version"])?;
    let first = out.lines().next()?;
    let token = first.strip_prefix("deno ").unwrap_or(first).split_whitespace().next()?;
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Локальные сведения о yt-dlp (без обращения к сети).
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

/// Локальные сведения о ffmpeg (без обращения к сети).
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

/// Локальные сведения о Deno (без обращения к сети). `update_available` для Deno
/// не вычисляем: он ставится по принципу «latest» вместе с yt-dlp.
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

/// Локальный статус всех утилит (быстро, без сети) — для первичной отрисовки.
pub fn info(app_data_dir: &Path) -> ToolsInfo {
    ToolsInfo {
        ytdlp: ytdlp_info(app_data_dir),
        ffmpeg: ffmpeg_info(app_data_dir),
        deno: deno_info(app_data_dir),
    }
}

/// Статус утилит с проверкой обновлений (обращается к GitHub).
pub fn check_updates(app_data_dir: &Path) -> ToolsInfo {
    let mut out = info(app_data_dir);

    // yt-dlp: сравниваем установленную версию с последним релизом. Версии вида
    // YYYY.MM.DD — лексикографическое сравнение совпадает с хронологическим.
    // Проверяем для ЛЮБОГО установленного yt-dlp (в т.ч. системного из PATH), а не
    // только managed: обновление скачается в bin-папку приложения, после чего
    // resolve начнёт предпочитать его — устаревший системный станет управляемым.
    if out.ytdlp.installed {
        if let (Some(cur), Some(latest)) = (out.ytdlp.version.clone(), fetch_ytdlp_latest()) {
            out.ytdlp.update_available = cur.as_str() < latest.as_str();
        }
    }

    // ffmpeg: сравниваем сохранённый при скачивании маркер (updated_at ассета)
    // с актуальным. Парсить версию rolling-сборки ненадёжно, поэтому маркер.
    if out.ffmpeg.managed {
        let stored = read_meta(app_data_dir)
            .get("ffmpeg")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let (Some(stored), Some(latest)) = (stored, fetch_ffmpeg_latest()) {
            out.ffmpeg.update_available = stored != latest;
        }
    }

    out
}

// ──────────────────────────────────────────────────────────────────────────────
// Сеть: последние доступные версии
// ──────────────────────────────────────────────────────────────────────────────

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

/// Последний релиз yt-dlp (`tag_name`, например `2025.06.09`).
fn fetch_ytdlp_latest() -> Option<String> {
    let v = fetch_json("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest")?;
    v.get("tag_name")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Маркер актуальности ffmpeg-сборки (updated_at нужного ассета rolling-релиза
/// `latest` репозитория yt-dlp/FFmpeg-Builds). На Windows/Linux.
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

/// macOS качает ffmpeg с evermeet.cx (не GitHub). Маркер актуальности — поле
/// `version` из публичного info-API текущего релиза. Без него macOS-пользователи
/// никогда не увидели бы уведомление об обновлении ffmpeg.
#[cfg(not(any(windows, target_os = "linux")))]
fn fetch_ffmpeg_latest() -> Option<String> {
    let v = fetch_json("https://evermeet.cx/ffmpeg/info/ffmpeg/release")?;
    v.get("version")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// ──────────────────────────────────────────────────────────────────────────────
// Установка / обновление / удаление (управляемые приложением бинари)
// ──────────────────────────────────────────────────────────────────────────────

// ──────────────────────────────────────────────────────────────────────────────
// Атомарная установка: staging → проверка → атомарная подмена с backup/rollback
// ──────────────────────────────────────────────────────────────────────────────

/// Временная подпапка staging внутри bin (скачиваем и проверяем здесь, затем
/// переносим в bin). Чистится перед использованием, чтобы не тащить хвосты
/// прошлой неудачной попытки.
fn staging_dir(bin: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = bin.join(format!(".staging-{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// RAII-обёртка над staging-папкой: удаляет её при выходе из области видимости —
/// при успехе, на любой ветке `Err` И при панике. Раньше чистка делалась вручную
/// на каждом раннем выходе, и непредвиденная паника (в скачивании/распаковке)
/// оставила бы скрытую `.staging-*` висеть на диске. Итоговые файлы выносятся из
/// staging до выхода (`swap_in_files`), поэтому удалять пустой остаток безопасно.
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

/// Путь резервной копии для атомарной подмены (`<target>.bak`).
fn backup_path(target: &Path) -> PathBuf {
    let name = target.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    target.with_file_name(format!("{}.bak", name))
}

/// Атомарно ставит уже проверенные файлы из staging на их конечные места:
/// существующий `target` отводится в `.bak`, затем staged переносится на его
/// место (rename в пределах одной ФС атомарен). При ЛЮБОМ сбое — откат всех уже
/// выполненных подмен из бэкапов (старые версии возвращаются на место). На успехе
/// бэкапы удаляются. Так обновление либо проходит целиком, либо не трогает
/// работающую установку.
fn swap_in_files(pairs: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    // (target, бэкап-или-None) для уже выполненных подмен — для отката.
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
            return Err(format!("подготовленный файл отсутствует: {}", staged.display()));
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
            // Не получилось поставить новый — возвращаем только что отведённый
            // бэкап и откатываем всё остальное.
            if let Some(b) = &bak {
                let _ = std::fs::rename(b, target);
            }
            rollback(&done);
            return Err(format!("не удалось установить новый файл: {}", e));
        }
        done.push((target.clone(), bak));
    }
    // Успех — удаляем бэкапы (best-effort; занятый старый exe уберётся позже).
    for (_, bak) in &done {
        if let Some(b) = bak {
            let _ = remove_file_retry(b);
        }
    }
    Ok(())
}

/// (Пере)скачать yt-dlp. Версия фиксируется конкретным релизом (не
/// `latest/download`), целостность проверяется по SHA256 из `SHA2-256SUMS` того
/// же релиза. Скачивание и проверки идут в staging-папке; в bin бинарь попадает
/// только после успешной проверки — атомарной подменой с backup/rollback, чтобы
/// неудачное обновление не сломало уже работающую установку.
pub fn install_ytdlp(
    app_data_dir: &Path,
    report: &(dyn Fn(String) + Sync),
) -> Result<ToolInfo, String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Резолвим конкретную версию: бинарь и его контрольная сумма берутся из
    // одного неизменяемого релиза (а не из подвижного latest-редиректа).
    let tag = fetch_ytdlp_latest()
        .ok_or_else(|| "Не удалось определить версию yt-dlp".to_string())?;
    let asset = ytdlp_asset_name();

    // Сперва получаем ожидаемую контрольную сумму — ДО скачивания бинаря. Если
    // суммы недоступны, незачем качать впустую десятки МБ; фейлимся сразу.
    report("Получение контрольной суммы yt-dlp…".to_string());
    let expected = fetch_ytdlp_sha256(&tag, asset)
        .ok_or_else(|| "Не удалось получить контрольную сумму yt-dlp для проверки".to_string())?;

    let url = format!(
        "https://github.com/yt-dlp/yt-dlp/releases/download/{}/{}",
        tag, asset
    );
    let target = ytdlp_local(app_data_dir);
    let bin = bin_dir(app_data_dir);
    // RAII: staging удалится на любом выходе (Err/успех/паника) — см. StagingDir.
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

    // Проверка целостности по контрольной сумме релиза (на staged-файле).
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

    // Всё проверено — атомарно подменяем рабочий бинарь.
    swap_in_files(&[(staged, target)])
        .map_err(|e| format!("Не удалось установить yt-dlp: {}", e))?;

    write_meta_field(app_data_dir, "ytdlp", &tag);
    Ok(ytdlp_info(app_data_dir))
}

/// (Пере)скачать ffmpeg. Скачивание и распаковка идут в staging-папке; в bin
/// бинари (`ffmpeg` + `ffprobe`) попадают только после проверки запуска —
/// атомарной подменой с backup/rollback (см. [`swap_in_files`]).
pub fn install_ffmpeg(
    app_data_dir: &Path,
    report: &(dyn Fn(String) + Sync),
) -> Result<ToolInfo, String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        let bin = bin_dir(app_data_dir);
        // RAII: staging удалится на любом выходе (Err/успех/паника) — см. StagingDir.
        let staging = StagingDir::new(&bin, "ffmpeg")?;

        report("Скачивание ffmpeg…".to_string());
        let staged_ffmpeg = download_ffmpeg_impl(staging.path(), |d, t| {
            report(format!("Скачивание ffmpeg… {}", fmt_progress(d, t)))
        })
        .map_err(|e| format!("Не удалось скачать ffmpeg: {}", e))?;
        if !runs(&staged_ffmpeg, "-version") {
            return Err("ffmpeg скачан, но не запускается".to_string());
        }

        // Подменяем ffmpeg и (если есть) ffprobe одной транзакцией.
        let ff_name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
        let fp_name = if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" };
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

/// Удалить файл с повторными попытками. На Windows только что отработавший
/// (или фоном проверяющий cookies) yt-dlp/Deno может держать .exe ещё доли
/// секунды — `remove_file` падает с `os error 5` (Отказано в доступе) /
/// `os error 32` (занят). Отсутствие файла считаем успехом. ~6 с попыток.
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

/// Удалить скачанный приложением yt-dlp (только из bin-папки).
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

/// Удалить скачанный приложением ffmpeg (ffmpeg + ffprobe из bin-папки) и маркер.
pub fn delete_ffmpeg(app_data_dir: &Path) -> Result<(), String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let bin = bin_dir(app_data_dir);
    // Не глотаем ошибки удаления: занятый/заблокированный бинарь оставил бы
    // утилиту «частично установленной», а вызывающий считал бы удаление успешным.
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

/// Установить Deno (JS-движок для yt-dlp) в bin-папку. Идемпотентно: если рабочий
/// Deno уже доступен (в bin-папке или системном PATH) — пропускаем скачивание.
/// Скачивается одним zip с GitHub-релиза denoland/deno под текущую платформу.
/// Привязан к жизненному циклу yt-dlp (ставится/удаляется вместе с ним).
pub fn install_deno(
    app_data_dir: &Path,
    report: &(dyn Fn(String) + Sync),
) -> Result<(), String> {
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Уже есть рабочий Deno (наш или системный) — не качаем повторно.
    if resolve_deno(app_data_dir).is_some() {
        return Ok(());
    }

    let bin = bin_dir(app_data_dir);
    // RAII: staging удалится на любом выходе (Err/успех/паника) — см. StagingDir.
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

/// Удалить скачанный приложением Deno (только из bin-папки). Системный Deno из
/// PATH не трогаем. Best-effort: ошибка удаления возвращается вызывающему.
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

// ──────────────────────────────────────────────────────────────────────────────
// Метаданные утилит (bin/tools_meta.json): маркеры версий для проверки обновлений
// ──────────────────────────────────────────────────────────────────────────────

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
        obj.insert(key.to_string(), serde_json::Value::String(value.to_string()));
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
