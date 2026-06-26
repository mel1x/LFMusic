//! Менеджер очереди скачивания треков (страница «Скачивание»).
//!
//! Пользователь ищет треки во внешних источниках ([`crate::external::search_tracks`])
//! и ставит их в очередь. Один фоновый воркер последовательно скачивает их через
//! **yt-dlp** в папку «LFMusic Downloads» (в системной музыкальной директории),
//! проставляет теги через `lofty`, после чего папка регистрируется как папка
//! библиотеки и пересканируется — треки появляются в каталоге.
//!
//! Состояние очереди стримится на фронт событием `download_queue`
//! ([`DownloadSnapshot`]). Грубый прогресс очереди (готово/всего) дополнительно
//! уходит в общий «блок статуса обработки» в шапке как фаза `download` (через
//! [`crate::report_download_progress`]) — последней после анализа и внешних
//! источников.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::tools;

/// Запрос на скачивание трека (с фронтенда).
#[derive(Debug, Clone, Deserialize)]
pub struct DownloadRequest {
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    /// Явный поисковый запрос для yt-dlp; по умолчанию «исполнитель название».
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub cover_url: Option<String>,
    /// Принудительно скачать, даже если такой трек уже в очереди/скачан
    /// (повторное скачивание из модалки «уже в библиотеке»).
    #[serde(default)]
    pub force: Option<bool>,
}

/// Элемент очереди скачивания (отдаётся на фронт).
#[derive(Debug, Clone, Serialize)]
pub struct DownloadItem {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub query: String,
    pub cover_url: Option<String>,
    /// "queued" | "downloading" | "done" | "error".
    pub status: String,
    /// Прогресс текущего элемента, 0..100.
    pub progress: f64,
    pub error: Option<String>,
    pub file_path: Option<String>,
}

/// Доступность внешних утилит (для статуса и блокировки кнопок на странице).
#[derive(Debug, Clone, Serialize)]
pub struct ToolsStatus {
    pub ytdlp: bool,
    pub ffmpeg: bool,
    /// JS-движок Deno доступен (ставится/удаляется вместе с yt-dlp).
    pub deno: bool,
    /// Идёт автоскачивание/проверка утилит.
    pub preparing: bool,
    pub message: Option<String>,
    /// Есть сохранённые cookies (пользователь авторизовался в YouTube).
    pub cookies_authorized: bool,
    /// Cookies прошли проверку — скачивание разблокировано.
    pub cookies_valid: bool,
    /// Идёт проверка cookies (сетевой тест извлечения).
    pub cookies_checking: bool,
    /// Открыто окно входа в YouTube (идёт авторизация).
    pub cookies_login_open: bool,
    /// Сообщение об ошибке проверки/авторизации cookies.
    pub cookies_message: Option<String>,
}

/// Полный снимок состояния для фронта (событие `download_queue`).
#[derive(Debug, Clone, Serialize)]
pub struct DownloadSnapshot {
    pub items: Vec<DownloadItem>,
    pub tools: ToolsStatus,
}

/// Менеджер очереди — за глобальным мьютексом ([`DownloadsState`]).
pub struct DownloadManager {
    items: Vec<DownloadItem>,
    next_id: u64,
    worker_running: bool,
    /// Счётчики текущего «прогона» для фазы прогресса в шапке.
    run_total: usize,
    run_done: usize,
    tools_preparing: bool,
    tools_message: Option<String>,
    ytdlp_ready: bool,
    ffmpeg_ready: bool,
    deno_ready: bool,
    /// Авторизация в YouTube (см. [`open_login`], [`validate_youtube_cookies`]).
    cookies_authorized: bool,
    cookies_valid: bool,
    cookies_checking: bool,
    cookies_login_open: bool,
    cookies_message: Option<String>,
    /// Состояние cookies уже подтянуто с диска (один раз за запуск). До этого
    /// поля cookies_* нулевые и не отражают реальный сохранённый вход.
    cookies_seeded: bool,
    /// Дочерний процесс активного скачивания (для отмены).
    current_child: Arc<Mutex<Option<Child>>>,
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
            cookies_authorized: false,
            cookies_valid: false,
            cookies_checking: false,
            cookies_login_open: false,
            cookies_message: None,
            cookies_seeded: false,
            current_child: Arc::new(Mutex::new(None)),
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
                preparing: self.tools_preparing,
                message: self.tools_message.clone(),
                cookies_authorized: self.cookies_authorized,
                cookies_valid: self.cookies_valid,
                cookies_checking: self.cookies_checking,
                cookies_login_open: self.cookies_login_open,
                cookies_message: self.cookies_message.clone(),
            },
        }
    }
}

/// Обёртка для [`tauri::Manager::manage`].
pub struct DownloadsState(pub Mutex<DownloadManager>);

impl Default for DownloadsState {
    fn default() -> Self {
        Self(Mutex::new(DownloadManager::default()))
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Доступ к состоянию + эмиссия событий
// ──────────────────────────────────────────────────────────────────────────────

fn with_manager<R>(app: &AppHandle, f: impl FnOnce(&mut DownloadManager) -> R) -> R {
    let state = app.state::<DownloadsState>();
    let mut guard = state.0.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

fn emit_snapshot(app: &AppHandle) {
    let snap = with_manager(app, |m| m.snapshot());
    let _ = app.emit("download_queue", snap);
}

/// Сообщить общему прогресс-бару в шапке текущее состояние фазы скачивания.
fn report_titlebar(app: &AppHandle) {
    let (done, total) = with_manager(app, |m| (m.run_done, m.run_total));
    crate::report_download_progress(app, done, total);
}

/// То же, но с долей прогресса текущего скачиваемого трека — для плавного
/// процента в шапке (бар не прыгает «потреково» с 0 на 100).
fn report_titlebar_frac(app: &AppHandle, frac: f64) {
    let (done, total) = with_manager(app, |m| (m.run_done, m.run_total));
    crate::report_download_progress_frac(app, done, total, frac);
}

// ──────────────────────────────────────────────────────────────────────────────
// Публичный API (вызывается из команд lib.rs)
// ──────────────────────────────────────────────────────────────────────────────

pub fn snapshot(app: &AppHandle) -> DownloadSnapshot {
    ensure_cookie_seed(app);
    with_manager(app, |m| m.snapshot())
}

/// Завершить активный дочерний процесс скачивания (yt-dlp) при выходе из
/// приложения. Без этого закрытие окна во время скачивания оставило бы yt-dlp
/// висеть фоновым процессом-зомби: Rust не убивает запущенные `Child` при
/// завершении родителя. Подписан на `RunEvent::Exit` в lib.rs.
pub fn cleanup_on_exit(app: &AppHandle) {
    let child = with_manager(app, |m| {
        m.current_child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    });
    if let Some(mut c) = child {
        kill_child_tree(&mut c);
    }
}

/// Прибить всё дерево процессов скачивания (yt-dlp + порождённые им ffmpeg/deno).
/// Один `child.kill()` убил бы только yt-dlp, а потомки могли бы остаться держать
/// файл/пайпы. Windows — `taskkill /T` (по дереву PID); Unix — сигнал всей группе
/// процессов (yt-dlp — её лидер, см. `process_group(0)` при запуске).
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
        // Негативный PID = вся группа процессов.
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{}", pid))
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Есть ли элементы очереди в работе (queued/downloading). Используется как
/// backend-guard для удаления утилит: воркер мог уже резолвнуть yt-dlp/ffmpeg и
/// качать очередь — удаление выдернуло бы бинарь из-под него.
pub fn has_active_jobs(app: &AppHandle) -> bool {
    with_manager(app, |m| {
        m.items
            .iter()
            .any(|i| i.status == "queued" || i.status == "downloading")
    })
}

/// Абсолютный путь папки скачиваний (фронт определяет по нему, скачан ли трек
/// приложением — для пункта «Удалить трек» в контекстном меню).
pub fn downloads_dir_path(app: &AppHandle) -> String {
    downloads_dir(app).to_string_lossy().to_string()
}

/// Лежит ли `file` внутри папки скачиваний `dir` — проверка контейнмента
/// безопасно (по компонентам пути). Наивный строковый `starts_with` пропустил бы
/// соседнюю папку с тем же префиксом (`…/LFMusic Downloads-old/x.mp3` прошёл бы
/// проверку против `…/LFMusic Downloads`), поэтому каноникализируем оба пути и
/// сравниваем компонентно (заодно гасит `..` и симлинки). Если файла уже нет на
/// диске (каноникализация падает) — лексический фолбэк с обязательной границей-
/// разделителем: этого достаточно, чтобы решить, можно ли вычистить осиротевшую
/// запись из БД.
fn is_within_downloads(dir: &Path, file: &Path) -> bool {
    if let (Ok(dir_c), Ok(file_c)) = (dir.canonicalize(), file.canonicalize()) {
        return file_c.starts_with(&dir_c);
    }
    // Фолбэк (файла уже нет на диске → canonicalize падает): ОБЯЗАТЕЛЬНО лексически
    // резолвим `.`/`..` без обращения к диску — иначе путь вида
    // `…/LFMusic Downloads/../../Desktop/x` прошёл бы строковую проверку префикса
    // и позволил удалить файл вне папки скачиваний (path traversal).
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

/// Лексически нормализует путь: схлопывает `.` и резолвит `..` БЕЗ обращения к
/// файловой системе (в отличие от `canonicalize`, который требует существования
/// пути). Используется для безопасной проверки контейнмента, когда файла уже нет.
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

/// Удалить скачанный приложением трек: удалить файл с диска и убрать из библиотеки.
/// Из соображений безопасности удаляет только файлы ВНУТРИ папки скачиваний
/// (произвольные файлы библиотеки этим путём не трогаются).
pub fn delete_downloaded_track(app: &AppHandle, file_path: &str) -> Result<(), String> {
    let dir = downloads_dir(app);
    if !is_within_downloads(&dir, Path::new(file_path)) {
        return Err("Можно удалять только треки, скачанные приложением".to_string());
    }

    // Если этот трек сейчас играет — останавливаем плеер: декодер (symphonia)
    // держит хэндл файла, иначе удаление на Windows упрётся в «файл занят».
    if let Ok(mut player) = app.state::<crate::PlayerState>().0.write() {
        if player.current_file_path().as_deref() == Some(file_path) {
            player.stop();
        }
    }

    // Карантин: сначала ПЕРЕМЕЩАЕМ файл во временное имя (rename в той же папке —
    // атомарен). Так удаление обратимо: если убрать запись из БД не выйдет, файл
    // вернём на место. Если перемещение не удалось (файл занят/нет прав) — БД не
    // трогаем, состояние согласовано. Ретраи перекрывают асинхронное освобождение
    // хэндла аудио-потоком после stop().
    let path = PathBuf::from(file_path);
    let quarantine: Option<PathBuf> = if path.exists() {
        let q = quarantine_path(&path);
        rename_with_retries(&path, &q)
            .map_err(|e| format!("Не удалось удалить файл: {}", e))?;
        Some(q)
    } else {
        None
    };

    // Убираем из библиотеки. При ошибке БД — ВОЗВРАЩАЕМ файл из карантина, чтобы
    // не потерять его при сохранившейся записи (никакого рассинхрона).
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
            let _ = std::fs::rename(q, &path); // откат
        }
        return Err(e);
    }

    // БД обновлена — физически удаляем файл из карантина. Best-effort: даже если не
    // выйдет, состояние согласовано (записи в библиотеке уже нет), а хвост подчистит
    // `cleanup_orphan_temp_files` при следующем запуске воркера.
    if let Some(q) = quarantine {
        let _ = remove_file_with_retries(&q);
    }
    let _ = app.emit("downloads_library_updated", ());
    Ok(())
}

/// Имя файла-карантина рядом с оригиналом (та же папка → rename атомарен и быстр).
/// Префикс `.lfm-trash-` распознаётся как временный и чистится при старте воркера.
fn quarantine_path(path: &Path) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("track");
    path.with_file_name(format!(".lfm-trash-{}-{}", ts, name))
}

/// Переименование с повторами: только что игравший файл может быть занят доли
/// секунды, пока аудио-поток освобождает хэндл после stop() (Windows).
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

/// Удаление файла с повторами: только что игравший файл может быть занят ещё
/// доли секунды, пока плеер его освобождает после остановки.
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

/// Поставить трек в очередь и (при необходимости) запустить воркер. Дубликаты
/// (тот же нормализованный «исполнитель + название + альбом», ещё не ошибочные)
/// не добавляются повторно — защита от двойного клика/повторного invoke/
/// устаревшего UI; возвращается id уже существующего элемента. Одноимённый трек
/// того же исполнителя из ДРУГОГО альбома дублем не считается — качается отдельно.
pub fn enqueue(app: &AppHandle, req: DownloadRequest) -> u64 {
    let query = req
        .query
        .filter(|q| !q.trim().is_empty())
        .unwrap_or_else(|| format!("{} {}", req.artist, req.title).trim().to_string());
    let key = crate::external::track_identity_key(&req.artist, &req.title, req.album.as_deref());
    let force = req.force.unwrap_or(false);

    let (id, is_new) = with_manager(app, |m| {
        // Уже в очереди/качается/скачан — не дублируем (ошибочные можно повторить).
        // force обходит проверку (осознанное повторное скачивание из модалки).
        if !force {
            if let Some(existing) = m.items.iter().find(|i| {
                i.status != "error"
                    && crate::external::track_identity_key(&i.artist, &i.title, i.album.as_deref())
                        == key
            }) {
                return (existing.id, false);
            }
        }
        let id = m.next_id;
        m.next_id += 1;
        m.items.push(DownloadItem {
            id,
            title: req.title,
            artist: req.artist,
            album: req.album,
            query,
            cover_url: req.cover_url,
            status: "queued".into(),
            progress: 0.0,
            error: None,
            file_path: None,
        });
        m.run_total += 1;
        (id, true)
    });

    if is_new {
        emit_snapshot(app);
        report_titlebar(app);
        ensure_worker(app);
    }
    id
}

/// Убрать элемент из очереди. Активный (скачивающийся) — отменяется (kill).
pub fn remove(app: &AppHandle, id: u64) {
    let child_to_kill = with_manager(app, |m| {
        let Some(pos) = m.items.iter().position(|i| i.id == id) else {
            return None;
        };
        let status = m.items[pos].status.clone();
        match status.as_str() {
            "queued" => {
                m.items.remove(pos);
                m.run_total = m.run_total.saturating_sub(1);
                None
            }
            "downloading" => {
                // Удаляем из списка; воркер увидит пропажу и зачтёт run_done.
                m.items.remove(pos);
                m.current_child.lock().unwrap_or_else(|e| e.into_inner()).take()
            }
            _ => {
                // done / error — просто убрать из истории.
                m.items.remove(pos);
                None
            }
        }
    });

    if let Some(mut child) = child_to_kill {
        kill_child_tree(&mut child);
    }

    emit_snapshot(app);
    report_titlebar(app);
}

/// Повторить скачивание ошибочного элемента: вернуть его статус из «error»
/// обратно в очередь (queued), очистить ошибку/прогресс и поднять воркер.
/// Метаданные и поисковый запрос сохраняются — повторно ничего вводить не нужно.
pub fn retry(app: &AppHandle, id: u64) {
    let restarted = with_manager(app, |m| {
        let Some(item) = m.items.iter_mut().find(|i| i.id == id && i.status == "error") else {
            return false;
        };
        item.status = "queued".into();
        item.progress = 0.0;
        item.error = None;
        item.file_path = None;
        // Добавляем в текущий прогон, чтобы прогресс-бар в шапке учёл повтор.
        m.run_total += 1;
        true
    });
    if restarted {
        emit_snapshot(app);
        report_titlebar(app);
        ensure_worker(app);
    }
}

/// Очистить завершённые/ошибочные элементы из списка.
pub fn clear_finished(app: &AppHandle) {
    with_manager(app, |m| {
        m.items
            .retain(|i| i.status == "queued" || i.status == "downloading");
    });
    emit_snapshot(app);
}

/// Проверить доступность утилит (без скачивания), закэшировать и вернуть статус.
pub fn refresh_tools_status(app: &AppHandle) -> ToolsStatus {
    let app_data_dir = app_data_dir(app);
    let (ytdlp, ffmpeg) = tools::check(&app_data_dir);
    let deno = tools::resolve_deno(&app_data_dir).is_some();
    let authorized = is_authorized(app);
    let invalid = cookies_invalid_marker(app).exists();
    with_manager(app, |m| {
        m.ytdlp_ready = ytdlp;
        m.ffmpeg_ready = ffmpeg;
        m.deno_ready = deno;
        m.cookies_authorized = authorized;
        // Валиден = есть cookies И не помечены протухшими (вход помнится с диска).
        m.cookies_valid = authorized && !invalid;
        m.cookies_seeded = true;
        m.snapshot().tools
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// Авторизация в YouTube (вход в собственном webview-окне → cookies.txt)
//
// `--cookies-from-browser` сломан на Windows (Chrome/Edge с 2024 шифруют cookies
// App-Bound Encryption, yt-dlp падает на DPAPI). Поэтому открываем своё окно с
// формой входа Google, после авторизации читаем cookies сессии (включая
// HttpOnly-авторизационные — их даёт webview cookie store Tauri, недоступный из
// document.cookie) и пишем их в Netscape-файл `cookies.txt`, который yt-dlp
// принимает через `--cookies`. Окно делит хранилище cookies с основным webview,
// поэтому сессия переживает перезапуск приложения.
// ──────────────────────────────────────────────────────────────────────────────

const LOGIN_LABEL: &str = "youtube-login";
const LOGIN_URL: &str =
    "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2F";
/// UA обычного десктоп-Chrome. Google отказывается отображать форму входа во
/// «встроенных» webview (по умолчанию WebView2 шлёт UA с `Edg/...` + признаки
/// embedded) — отдаёт пустую/заблокированную страницу. Под видом обычного Chrome
/// страница входа открывается нормально.
const LOGIN_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// Стабильное видео для проверки cookies (популярный, не удаляемый ролик).
const COOKIE_CHECK_URL: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

/// Путь к файлу cookies (Netscape-формат) для yt-dlp.
fn cookies_file(app: &AppHandle) -> PathBuf {
    app_data_dir(app).join("youtube_cookies.txt")
}

/// Авторизованы ли мы (есть сохранённый файл cookies).
pub fn is_authorized(app: &AppHandle) -> bool {
    cookies_file(app).exists()
}

/// Маркер «cookies протухли». Его НАЛИЧИЕ означает, что вход больше не валиден
/// (проверка провалилась или скачивание упёрлось в бот-стену при наличии cookies).
/// Отсутствие при наличии cookies-файла = вход считаем валидным. Так успешный
/// вход «помнится» между запусками: статус показывается сразу, без повторной
/// сетевой проверки, и сбрасывается только когда cookies реально перестали работать.
fn cookies_invalid_marker(app: &AppHandle) -> PathBuf {
    app_data_dir(app).join("youtube_cookies_invalid")
}

/// Пометить cookies как протухшие (создать маркер) + сбросить флаг валидности.
fn mark_cookies_invalid(app: &AppHandle) {
    let _ = std::fs::write(cookies_invalid_marker(app), b"1");
    with_manager(app, |m| {
        m.cookies_valid = false;
        m.cookies_seeded = true;
    });
}

/// Снять пометку «протухли» (после успешного входа/проверки).
fn clear_cookies_invalid(app: &AppHandle) {
    let _ = std::fs::remove_file(cookies_invalid_marker(app));
}

/// Подтянуть сохранённое состояние входа с диска один раз за запуск. Валиден =
/// есть cookies-файл И нет маркера «протухли». До первого вызова поля cookies_*
/// в менеджере нулевые (Default) и не отражают реальный сохранённый вход.
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

/// Открыть окно входа в YouTube и запустить ожидание авторизации. Идемпотентно:
/// если окно уже открыто — просто фокусируем его.
pub fn open_login(app: &AppHandle) {
    eprintln!("[youtube-login] open_login: запрошен вход");
    if let Some(win) = app.get_webview_window(LOGIN_LABEL) {
        let _ = win.set_focus();
        return;
    }
    with_manager(app, |m| {
        m.cookies_login_open = true;
        m.cookies_message = None;
    });
    emit_snapshot(app);

    // ВАЖНО: окно создаём в фоновом потоке, а НЕ через run_on_main_thread.
    // `WebviewWindowBuilder::build()` сам диспетчеризует создание в event loop и
    // блокирует вызывающий поток до готовности окна. Если вызвать его на главном
    // потоке — event loop не может допумпить создание webview → дедлок (белое
    // окно + зависшее приложение). Из фонового потока всё работает штатно.
    let app2 = app.clone();
    if std::thread::Builder::new()
        .name("youtube-login".into())
        .spawn(move || {
            if let Err(e) = build_login_window(&app2) {
                with_manager(&app2, |m| {
                    m.cookies_login_open = false;
                    m.cookies_message = Some(format!("Не удалось открыть окно входа: {}", e));
                });
                emit_snapshot(&app2);
                return;
            }
            login_watch_loop(app2);
        })
        .is_err()
    {
        // Поток не создался — снимаем флаг «окно входа открыто», иначе UI завис
        // бы в состоянии ожидания входа навсегда.
        with_manager(app, |m| {
            m.cookies_login_open = false;
            m.cookies_message = Some("Не удалось запустить вход".to_string());
        });
        emit_snapshot(app);
    }
}

fn build_login_window(app: &AppHandle) -> Result<(), String> {
    let url = tauri::Url::parse(LOGIN_URL).map_err(|e| e.to_string())?;
    let win = tauri::WebviewWindowBuilder::new(app, LOGIN_LABEL, tauri::WebviewUrl::External(url))
        .title("Вход в YouTube — LFMusic")
        .inner_size(520.0, 720.0)
        .min_inner_size(420.0, 560.0)
        .center()
        .focused(true)
        .user_agent(LOGIN_UA)
        // Логи в stderr (виден в терминале `tauri dev`) — диагностика пустого окна.
        .on_navigation(|url| {
            eprintln!("[youtube-login] navigate → {}", url);
            true
        })
        .on_page_load(|_w, payload| {
            eprintln!("[youtube-login] page_load → {}", payload.url());
        })
        .build()
        .map_err(|e| {
            eprintln!("[youtube-login] ошибка создания окна: {}", e);
            e.to_string()
        })?;
    eprintln!("[youtube-login] окно создано, грузим {}", LOGIN_URL);
    // Иногда созданное в рантайме окно WebView2 остаётся белым до первого события
    // фокуса/ресайза — подталкиваем его.
    let _ = win.set_focus();
    Ok(())
}

/// Имена авторизационных cookies YouTube (их появление = пользователь вошёл).
const AUTH_COOKIE_NAMES: &[&str] = &["__Secure-3PSID", "__Secure-1PSID", "LOGIN_INFO", "SID"];

/// Прочитать cookies Google/YouTube из webview cookie store (общий для окон).
/// Берём из окна входа (пока открыто) либо из основного окна — хранилище одно.
fn read_youtube_cookies(app: &AppHandle) -> Vec<tauri::webview::cookie::Cookie<'static>> {
    let Some(win) = app
        .get_webview_window(LOGIN_LABEL)
        .or_else(|| app.get_webview_window("main"))
    else {
        return Vec::new();
    };
    // `cookies_for_url` надёжнее, чем `cookies()` (полный дамп) на всех бэкендах;
    // собираем по ключевым доменам и дедупим по (имя, домен).
    let urls = [
        "https://www.youtube.com/",
        "https://accounts.google.com/",
        "https://www.google.com/",
    ];
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for u in urls {
        let Ok(url) = tauri::Url::parse(u) else { continue };
        if let Ok(cookies) = win.cookies_for_url(url) {
            for c in cookies {
                let key = format!("{}|{}", c.name(), c.domain().unwrap_or(""));
                if seen.insert(key) {
                    out.push(c);
                }
            }
        }
    }
    out
}

/// Есть ли в текущей сессии авторизационные cookies YouTube.
fn has_youtube_auth(app: &AppHandle) -> bool {
    read_youtube_cookies(app).iter().any(|c| {
        let domain = c.domain().unwrap_or("").to_ascii_lowercase();
        domain.contains("youtube.com") && AUTH_COOKIE_NAMES.contains(&c.name())
    })
}

/// Удалить cookies YouTube/Google из webview-хранилища (общего для всех окон).
/// Нужно при «Выйти»: сам файл cookies.txt мы удаляем, но без очистки webview
/// старая сессия осталась бы — следующий вход тут же её подхватил бы (watcher
/// видит авторизационные cookies и сразу фиксирует тот же аккаунт), и сменить
/// аккаунт стало бы невозможно. Удаляем точечно только cookies наших доменов
/// (не трогая localStorage/прочие данные основного окна — `clear_all_browsing_data`
/// был бы слишком грубым и снёс бы состояние приложения).
fn clear_webview_youtube_cookies(app: &AppHandle) {
    let Some(win) = app
        .get_webview_window(LOGIN_LABEL)
        .or_else(|| app.get_webview_window("main"))
    else {
        return;
    };
    for c in read_youtube_cookies(app) {
        let _ = win.delete_cookie(c);
    }
}

/// Записать cookies в Netscape-файл для yt-dlp. Возвращает число записанных строк.
fn write_cookies_file(app: &AppHandle, cookies: &[tauri::webview::cookie::Cookie<'static>]) -> Result<usize, String> {
    let mut out = String::from("# Netscape HTTP Cookie File\n# Сгенерировано LFMusic\n\n");
    let mut count = 0usize;
    for c in cookies {
        let domain = match c.domain() {
            Some(d) if !d.is_empty() => d,
            _ => continue,
        };
        let dl = domain.to_ascii_lowercase();
        if !(dl.contains("youtube.com") || dl.contains("google.com") || dl.contains("googlevideo.com")) {
            continue;
        }
        // HttpOnly помечается префиксом домена `#HttpOnly_` (его понимает yt-dlp).
        let http_only = c.http_only().unwrap_or(false);
        let domain_field = if http_only {
            format!("#HttpOnly_{}", domain)
        } else {
            domain.to_string()
        };
        let include_sub = if domain.starts_with('.') { "TRUE" } else { "FALSE" };
        let path = c.path().unwrap_or("/");
        let secure = if c.secure().unwrap_or(false) { "TRUE" } else { "FALSE" };
        // Session-cookies (без даты) → 0.
        let expiry = c
            .expires_datetime()
            .map(|d| d.unix_timestamp())
            .filter(|&t| t > 0)
            .unwrap_or(0);
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            domain_field, include_sub, path, secure, expiry, c.name(), c.value()
        ));
        count += 1;
    }
    if count == 0 {
        return Err("Cookies YouTube не найдены — войдите в аккаунт.".to_string());
    }
    write_private_atomic(&cookies_file(app), out.as_bytes())?;
    Ok(count)
}

/// Атомарно и приватно записать файл. Cookies содержат активную сессию, поэтому:
/// (1) пишем во временный сосед и атомарно переименовываем — не оставляем
/// частично записанный файл при сбое; (2) ставим права 0600 на Unix (иначе umask
/// мог бы оставить файл доступным для чтения другим пользователям системы).
fn write_private_atomic(target: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = target.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    // rename атомарно заменяет существующий target на обеих платформах. Но target
    // может быть КРАТКО занят: параллельный yt-dlp читает cookies во время проверки
    // авторизации/скачивания, и тогда замена временно падает (sharing violation на
    // Windows). Это и приводило к «вход актуальный, а cookies не обновились» —
    // повторяем несколько раз, прежде чем сдаться.
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

/// Зафиксировать вход: записать cookies.txt и сразу считать вход валидным.
/// Сетевую проверку НЕ запускаем автоматически (раньше она давала мелькание
/// «вход → проверка» при каждом входе) — вход помнится с диска, а реальную
/// негодность cookies выявит либо ручная «Проверить», либо бот-стена при
/// скачивании (тогда выставится маркер «протухли»).
fn finalize_login(app: &AppHandle) {
    let cookies = read_youtube_cookies(app);
    match write_cookies_file(app, &cookies) {
        Ok(_) => {
            clear_cookies_invalid(app);
            with_manager(app, |m| {
                m.cookies_authorized = true;
                m.cookies_valid = true;
                m.cookies_checking = false;
                m.cookies_message = None;
                m.cookies_seeded = true;
            });
            emit_snapshot(app);
        }
        Err(e) => {
            let authorized = is_authorized(app);
            with_manager(app, |m| {
                m.cookies_authorized = authorized;
                m.cookies_valid = false;
                m.cookies_message = Some(e);
            });
            emit_snapshot(app);
        }
    }
}

/// Фоновое ожидание авторизации в окне входа. Опрашивает cookie store; при
/// появлении авторизационных cookies фиксирует вход и закрывает окно. Сдаётся,
/// если пользователь закрыл окно сам или прошло 5 минут.
fn login_watch_loop(app: AppHandle) {
    let start = Instant::now();
    let mut window_seen = false;
    loop {
        std::thread::sleep(Duration::from_millis(1200));

        if app.get_webview_window(LOGIN_LABEL).is_some() {
            window_seen = true;
        } else {
            // Окно закрылось. Если успели его увидеть — пробуем зафиксировать вход
            // (cookies в общем хранилище переживают закрытие окна).
            if window_seen {
                finalize_login(&app);
            }
            with_manager(&app, |m| m.cookies_login_open = false);
            emit_snapshot(&app);
            return;
        }

        if start.elapsed() > Duration::from_secs(300) {
            if let Some(w) = app.get_webview_window(LOGIN_LABEL) {
                let _ = w.close();
            }
            with_manager(&app, |m| m.cookies_login_open = false);
            emit_snapshot(&app);
            return;
        }

        if has_youtube_auth(&app) {
            // Сначала фиксируем (окно ещё открыто), затем закрываем.
            finalize_login(&app);
            if let Some(w) = app.get_webview_window(LOGIN_LABEL) {
                let _ = w.close();
            }
            with_manager(&app, |m| m.cookies_login_open = false);
            emit_snapshot(&app);
            return;
        }
    }
}

/// Выйти из аккаунта: удалить cookies.txt, очистить webview-сессию YouTube/Google
/// и сбросить статус. Очистка webview нужна, чтобы при следующем входе можно было
/// выбрать другой аккаунт (иначе старая сессия подхватилась бы автоматически).
pub fn logout(app: &AppHandle) {
    let _ = std::fs::remove_file(cookies_file(app));
    clear_webview_youtube_cookies(app);
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

/// Проверить валидность сохранённых cookies (фоновый сетевой тест: пробное
/// `--simulate`-извлечение стабильного ролика). Результат кэшируется в менеджере
/// и стримится снапшотом; обязательная фаза разблокировки скачивания.
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
            // Успех снимает маркер «протухли» и запоминается между запусками.
            // Провал НЕ пишем на диск: он мог быть из-за отсутствия Deno или
            // временной сети — не делаем вход «невалидным» навсегда. Реальную
            // негодность фиксирует только бот-стена при скачивании.
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

/// Запускает yt-dlp в режиме `--simulate` с сохранёнными cookies и определяет,
/// годятся ли они для скачивания. Возвращает `(валидны, сообщение_об_ошибке)`.
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
        return (false, Some("Нет cookies — авторизуйтесь в YouTube.".to_string()));
    }

    // Без JS-движка YouTube не отдаёт аудиоформаты (signature/n-challenge не
    // решаются) — скачивание всё равно не сработает. Сообщаем явно, а не отдаём
    // загадочное «Requested format is not available».
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
        .arg(deno_rt);
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

// ──────────────────────────────────────────────────────────────────────────────
// Воркер
// ──────────────────────────────────────────────────────────────────────────────

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
        // Поток не создался — иначе worker_running остался бы навсегда true и
        // очередь зависла бы без воркера. Сбрасываем флаг, чтобы следующий
        // enqueue смог поднять воркер заново.
        with_manager(app, |m| m.worker_running = false);
    }
}

fn worker_loop(app: AppHandle) {
    let app_data_dir = app_data_dir(&app);
    let dl_dir = downloads_dir(&app);

    // Подчищаем недокачанные хвосты от прошлой сессии (краш/обрыв/перезагрузка во
    // время загрузки): `.part`, `.ytdl`, фрагменты. Делаем ДО цикла — активных
    // загрузок ещё нет, так что свой `.part` мы не заденем.
    cleanup_orphan_temp_files(&dl_dir);

    // ── Резолв утилит без автоскачивания (supply-chain): используем только уже
    // установленные бинари. Установка — только явная, со страницы настроек
    // (с фиксированной версией и проверкой контрольной суммы). yt-dlp обязателен;
    // ffmpeg опционален (без него выбор форматов ограничен). ──
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
    let cookies_path = cookies_file(&app);
    let cookies_file_opt = cookies_path.exists().then_some(cookies_path);
    with_manager(&app, |m| {
        m.tools_preparing = false;
        m.tools_message = None;
        m.ytdlp_ready = true;
        m.ffmpeg_ready = ffmpeg.is_some();
        m.deno_ready = deno.is_some();
    });
    emit_snapshot(&app);
    let resolved = tools::ResolvedTools {
        ytdlp,
        ffmpeg,
        deno,
        cookies_file: cookies_file_opt,
    };

    let child_slot = with_manager(&app, |m| m.current_child.clone());

    loop {
        // Берём следующий элемент в очереди.
        let job = with_manager(&app, |m| {
            let pos = m.items.iter().position(|i| i.status == "queued")?;
            m.items[pos].status = "downloading".into();
            m.items[pos].progress = 0.0;
            Some(m.items[pos].clone())
        });
        let Some(job) = job else {
            break;
        };
        emit_snapshot(&app);

        let result = download_one(&app, &resolved, &dl_dir, &job, &child_slot);

        // Импорт в библиотеку выполняем СРАЗУ после успешного скачивания и до
        // выставления финального статуса: иначе при провале сканирования UI
        // показал бы «готово», хотя трека в каталоге нет. Ошибку импорта
        // показываем отдельно (файл скачан, но не добавлен).
        let import_err: Option<String> = match &result {
            Ok(path) => import_track(&app, &dl_dir, path).err(),
            Err(_) => None,
        };

        with_manager(&app, |m| {
            m.run_done += 1;
            if let Some(item) = m.items.iter_mut().find(|i| i.id == job.id) {
                match &result {
                    Ok(path) => {
                        item.file_path = Some(path.to_string_lossy().to_string());
                        if let Some(err) = &import_err {
                            item.status = "error".into();
                            item.progress = 100.0;
                            item.error = Some(format!(
                                "Трек скачан, но не добавлен в библиотеку: {}",
                                err
                            ));
                        } else {
                            item.status = "done".into();
                            item.progress = 100.0;
                            item.error = None;
                        }
                    }
                    Err(e) => {
                        item.status = "error".into();
                        item.error = Some(e.clone());
                    }
                }
            }
        });
        emit_snapshot(&app);
        report_titlebar(&app);
    }

    // Анализ «настроения» новых треков запускаем ОДИН раз по завершении пачки, а
    // не на каждый трек: иначе очередь из N треков N раз отменяла бы и
    // перезапускала фоновый воркер, мешая ему досчитывать.
    restart_audio_feature_analysis(&app);

    finish_run(&app);
}

/// Перезапустить фоновый анализ аудио-фич, чтобы он подхватил только что
/// добавленные треки (их job-лист фиксируется при старте воркера). Best-effort:
/// если библиотека занята — пропускаем (досчитает при следующем запуске).
fn restart_audio_feature_analysis(app: &AppHandle) {
    if let Ok(lib) = app.state::<crate::LibraryState>().0.lock() {
        let _ = lib.resume_pending_audio_feature_analysis(None);
    }
}

/// Завершить прогон: сбросить счётчики, снять флаг воркера, дотолкнуть прогресс
/// шапки до «готово» и запустить воркер заново, если что-то добавили в гонке.
fn finish_run(app: &AppHandle) {
    let (done, total) = with_manager(app, |m| {
        let r = (m.run_done, m.run_total);
        m.run_done = 0;
        m.run_total = 0;
        m.worker_running = false;
        r
    });
    // Равные done/total → прогресс-бар отметит фазу завершённой и спрячется.
    crate::report_download_progress(app, total.max(done), total.max(done));
    emit_snapshot(app);

    // Если за время прогона добавили новые элементы — поднимаем воркер снова.
    ensure_worker(app);
}

/// Текст-маркер «бот-стены» YouTube в сообщении об ошибке (по нему фронт
/// показывает мягкую подсказку войти в аккаунт). Держать в синхроне с фронтом
/// (`useDownloads.ts` → `isBotWallError`).
pub const BOT_WALL_HINT: &str = "YouTube требует вход";

/// Признаки «бот-стены» в stderr yt-dlp (требуется авторизация для обхода).
fn is_bot_wall(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("confirm you")
        || s.contains("not a bot")
        || s.contains("sign in to confirm")
        || s.contains("sign in to prove")
        || (s.contains("cookies") && s.contains("authentication") && s.contains("required"))
}

/// Пользователь отменил скачивание (kill процесса) — повтор с другим источником
/// не нужен.
fn is_cancel_error(e: &str) -> bool {
    e.contains("отменено")
}

/// Поисковые цели для yt-dlp по порядку надёжности. Сначала — YouTube Music
/// (чистые аудиорелизы без видео-заставок), затем обычный `ytsearch1`
/// (устойчивый встроенный экстрактор-фолбэк, когда music-страница не отдалась).
fn search_targets(query: &str) -> Vec<String> {
    vec![
        format!(
            "https://music.youtube.com/search?q={}#songs",
            percent_encode_query(query)
        ),
        format!("ytsearch1:{}", query),
    ]
}

/// Скачать один трек через yt-dlp, проставить теги. Возвращает путь к файлу.
/// Пробует поисковые цели по очереди (см. [`search_targets`]) — это страхует от
/// нестабильного music-экстрактора. На отмене/«бот-стене» дальше не пробуем.
fn download_one(
    app: &AppHandle,
    resolved: &tools::ResolvedTools,
    dir: &Path,
    job: &DownloadItem,
    child_slot: &Arc<Mutex<Option<Child>>>,
) -> Result<PathBuf, String> {
    let stem = unique_stem(dir, &job.artist, &job.title);
    let targets = search_targets(&job.query);

    // Бот-стена YouTube часто транзиентна — те же cookies проходят со 2–3 попытки
    // (ровно то, что пользователь делал руками, переоткрывая скачивание). Поэтому
    // при бот-стене не сдаёмся сразу: повторяем несколько раз с короткой паузой, и
    // помечаем вход устаревшим ТОЛЬКО если ВСЕ попытки упёрлись в бот-стену (тогда
    // cookies, скорее всего, реально протухли). Обычные ошибки ретраем не лечатся.
    const BOT_WALL_ATTEMPTS: usize = 3;
    let mut last_err = "yt-dlp завершился с ошибкой".to_string();
    let mut bot_wall_seen = false;

    for attempt in 0..BOT_WALL_ATTEMPTS {
        let mut hit_bot_wall = false;
        for target in &targets {
            let result =
                download_one_with_stem(app, resolved, dir, job, child_slot, &stem, target);
            // Уборка хвостов yt-dlp (фрагменты, `.part`, `.ytdl`) при любом исходе.
            // `stem` уникален для задания: при ошибке безопасно удалить все `stem.*`,
            // при успехе — всё, кроме итогового файла.
            match result {
                Ok(final_path) => {
                    remove_stem_files(dir, &stem, Some(&final_path));
                    return Ok(final_path);
                }
                Err(e) => {
                    remove_stem_files(dir, &stem, None);
                    // Отмена пользователем — прекращаем немедленно.
                    if is_cancel_error(&e) {
                        return Err(e);
                    }
                    if e.contains(BOT_WALL_HINT) {
                        hit_bot_wall = true;
                        bot_wall_seen = true;
                    }
                    last_err = e;
                }
            }
        }
        // Повторяем только транзиентную бот-стену; обычная ошибка — выходим сразу.
        if !hit_bot_wall {
            break;
        }
        if attempt + 1 < BOT_WALL_ATTEMPTS {
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    // Бот-стена пережила все повторы → cookies, вероятно, реально протухли. Только
    // теперь помечаем вход устаревшим (статус сбросится и переживёт перезапуск).
    if bot_wall_seen {
        if resolved.cookies_file.is_some() {
            mark_cookies_invalid(app);
            emit_snapshot(app);
            return Err(format!(
                "{} - войдите заново в «Внешние источники»",
                BOT_WALL_HINT
            ));
        }
        return Err(format!(
            "{} - войдите в YouTube в настройках «Внешние источники»",
            BOT_WALL_HINT
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
) -> Result<PathBuf, String> {
    let out_template = dir.join(format!("{}.%(ext)s", stem));

    // Библиотека сканирует только определённые контейнеры (m4a/opus/ogg/mp3/…),
    // а YouTube обычно отдаёт лучшее аудио в .webm, который НЕ поддерживается.
    // Поэтому: с ffmpeg берём абсолютно лучшее аудио и ремуксируем webm/mkv в
    // .opus (копирование потока, без перекодирования; именно `.opus`, а не
    // `.ogg`, — иначе по расширению теги читаются как Vorbis и не пишутся). Без
    // ffmpeg жёстко ограничиваем выбор поддерживаемыми контейнерами — без
    // запасного `best`, чтобы при отсутствии подходящего потока yt-dlp вернул
    // ошибку, а не скачал .webm.
    let ffmpeg_available = resolved.ffmpeg.is_some();
    let format = if ffmpeg_available {
        "bestaudio/best"
    } else {
        "bestaudio[ext=m4a]/bestaudio[ext=mp3]/bestaudio[ext=opus]/\
         bestaudio[ext=ogg]/bestaudio[ext=aac]/bestaudio[ext=flac]/bestaudio[ext=wav]"
    };

    let mut cmd = tools::silent_command(&resolved.ytdlp);
    cmd.arg("--playlist-items")
        .arg("1")
        .arg("--no-warnings")
        .arg("--newline")
        // Сетевые подстраховки: больше повторов и таймаут сокета, чтобы единичный
        // обрыв/таймаут не валил весь трек (главная причина «нестабильности»).
        .arg("--retries")
        .arg("10")
        .arg("--fragment-retries")
        .arg("10")
        .arg("--extractor-retries")
        .arg("3")
        .arg("--socket-timeout")
        .arg("30")
        // Качаем последовательно (без --concurrent-fragments): параллельная
        // загрузка фрагментов пишет в выходной файл через seek/предвыделение, и
        // на Windows для части YouTube-потоков (SABR/фрагментированный DASH) это
        // даёт `OSError: [Errno 22] Invalid argument` в downloader. Для одиночных
        // аудиопотоков (обычные треки) параллелизм всё равно не даёт выигрыша.
        .arg("-f")
        .arg(format)
        .arg("-o")
        .arg(&out_template)
        .arg(search_target);
    if let Some(ff) = &resolved.ffmpeg {
        cmd.arg("--remux-video").arg("webm>opus/mkv>opus/mka>opus");
        // --ffmpeg-location указываем только для нашего скачанного бинаря
        // (абсолютный путь). Для ffmpeg из PATH (голое имя) parent() == "" —
        // флаг сломал бы вызов, поэтому полагаемся на PATH.
        if ff.is_absolute() {
            if let Some(ff_dir) = ff.parent() {
                cmd.arg("--ffmpeg-location").arg(ff_dir);
            }
        }
    }
    // Cookies авторизованной сессии YouTube — обход бот-стены («Sign in to confirm
    // you're not a bot»). Авторизация необязательна: если cookies есть — передаём
    // их, если нет — пробуем без (большинство треков качаются и так).
    if let Some(cookies) = &resolved.cookies_file {
        cmd.arg("--cookies").arg(cookies);
    }
    // JS-движок Deno — корректное извлечение форматов YouTube (расшифровка nsig);
    // уводит от деградированных SABR-потоков, дающих `[Errno 22]` на Windows.
    if let Some(deno) = &resolved.deno {
        cmd.arg("--js-runtimes")
            .arg(tools::deno_runtime_value(deno));
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // На Unix кладём yt-dlp в собственную группу процессов, чтобы при отмене
    // прибить всё дерево (yt-dlp + порождённые им ffmpeg/deno) сигналом группе.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Не удалось запустить yt-dlp: {}", e))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Сохраняем процесс для возможной отмены.
    {
        let mut slot = child_slot.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(child);
    }

    // Стдерр сливаем в отдельном потоке (иначе буфер может заблокировать). Читаем
    // байтами + потерьное декодирование: локальная кодировка yt-dlp не должна
    // ломать сбор stderr (read_to_string упал бы на невалидном UTF-8 и потерял
    // бы весь вывод, включая текст ошибки).
    let stderr_handle = stderr.map(|err| {
        std::thread::spawn(move || {
            let mut bytes: Vec<u8> = Vec::new();
            let _ = BufReader::new(err).read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    });

    // Парсим прогресс из stdout (yt-dlp с --newline пишет проценты построчно).
    // ВАЖНО: читаем сырые байты и декодируем потерьно (from_utf8_lossy), а НЕ
    // через BufRead::lines() (строгий UTF-8). yt-dlp может писать stdout в
    // локальной кодировке (cp1251 на Windows): строка с кириллицей —
    // невалидный UTF-8, lines() оборвал бы чтение, пайп бы закрылся, и yt-dlp
    // упал бы с `OSError: [Errno 22]` при следующей записи в stdout.
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
                    // Плавный общий процент в шапке: доля текущего трека.
                    report_titlebar_frac(app, pct / 100.0);
                }
            }
        }
    }

    // Забираем процесс назад и ждём завершения.
    let status = {
        let taken = child_slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        match taken {
            Some(mut c) => c.wait().map_err(|e| e.to_string())?,
            // Процесс уже забрали (отмена) — считаем отменённым.
            None => return Err("Скачивание отменено".to_string()),
        }
    };

    let stderr_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    if !status.success() {
        // Полный stderr — в лог приложения: пользовательское сообщение сжато до
        // осмысленной строки, но для диагностики «загадочных» ошибок yt-dlp
        // (например, `[Errno 22]`) нужен весь вывод с контекстом.
        eprintln!(
            "[downloads] yt-dlp ошибка (запрос «{}»):\n{}",
            job.query,
            stderr_text.trim()
        );
        // Бот-стена YouTube — возвращаем только маркер. Вход НЕ инвалидируем здесь:
        // бот-стена часто транзиентна (те же cookies проходят со 2–3 попытки).
        // Решение о повторах и об устаревании cookies принимает download_one.
        if is_bot_wall(&stderr_text) {
            return Err(BOT_WALL_HINT.to_string());
        }
        return Err(extract_ytdlp_error(&stderr_text));
    }

    let file = find_output(dir, stem)
        .ok_or_else(|| "Файл не найден после скачивания".to_string())?;

    // Страховка: если итоговый контейнер библиотека не сканирует (например, без
    // ffmpeg прорвался .webm), не выдаём ложное «готово» — удаляем и сообщаем.
    let ext = file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !is_supported_audio_ext(&ext) {
        let _ = std::fs::remove_file(&file);
        return Err(format!(
            "формат {} не поддерживается (нужен ffmpeg для конвертации)",
            if ext.is_empty() { "файла" } else { &ext }
        ));
    }

    // Теги — best-effort: даже если не записались, файл уже скачан и играбелен.
    write_tags(&file, &job.title, &job.artist, job.album.as_deref(), job.cover_url.as_deref());

    Ok(file)
}

// ──────────────────────────────────────────────────────────────────────────────
// Утилиты файловой системы / парсинга
// ──────────────────────────────────────────────────────────────────────────────

/// Расширения, которые сканирует библиотека (см. `AudioPlayer::supported_formats`).
const SUPPORTED_AUDIO_EXTS: &[&str] = &[
    "m4a", "opus", "ogg", "mp3", "flac", "wav", "aac", "mp4", "aiff", "aif",
];

fn is_supported_audio_ext(ext: &str) -> bool {
    SUPPORTED_AUDIO_EXTS.contains(&ext)
}

/// Percent-encode значения query-параметра URL (UTF-8 байты → %XX). Достаточно
/// для построения адреса поиска music.youtube.com из произвольного запроса.
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

/// Зарезервированные в Windows имена устройств: файл `CON.ogg`, `NUL` и т.п.
/// создать нельзя. Проверяем по базовому имени без учёта регистра.
fn is_reserved_windows_name(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0')
}

fn app_data_dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Папка для скачанных треков: «LFMusic Downloads» в системной музыкальной
/// директории (фолбэк — папка данных приложения).
fn downloads_dir(app: &AppHandle) -> PathBuf {
    let base = app
        .path()
        .audio_dir()
        .ok()
        .unwrap_or_else(|| app_data_dir(app));
    let dir = base.join("LFMusic Downloads");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Убирает недопустимые в имени файла символы (Windows-набор) и схлопывает пробелы.
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

/// Обрезает базовое имя так, чтобы полный путь уложился в лимит Windows
/// (`MAX_PATH` = 260). yt-dlp создаёт рядом с итоговым файлом временные
/// (`{stem}.f251.webm.part`, `{stem}.<ext>.ytdl`, фрагменты), поэтому даже
/// валидное по символам, но длинное «Исполнитель - Название» приводит к
/// `OSError: [Errno 22] Invalid argument` в `sanitize_open` (см. yt-dlp #11251).
/// Режем базу с запасом под разделитель, худший временной суффикс и « (NN)».
fn clamp_stem_for_path(dir: &Path, base: String) -> String {
    // Держимся ниже 260 с запасом; SUFFIX_RESERVE ≈ ".f251.webm.part" + " (12)".
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
    // После обрезки не оставляем хвостовых пробелов/точек (Windows их не любит).
    let trimmed = truncated.trim_matches(|c| c == '.' || c == ' ').to_string();
    if trimmed.is_empty() {
        "track".to_string()
    } else {
        trimmed
    }
}

/// Имя файла без расширения: «Исполнитель - Название», уникализированное суффиксом.
/// Содержимое папки читается один раз в `HashSet`, коллизии проверяются в памяти
/// за O(1) (а не повторным `read_dir` на каждой итерации).
fn unique_stem(dir: &Path, artist: &str, title: &str) -> String {
    let a = sanitize(artist);
    let t = sanitize(title);
    // Любая из частей может полностью стереться при санитарии (название из одних
    // спецсимволов) — тогда не оставляем висящих « - » и не получаем пустой stem.
    let base = match (a.is_empty(), t.is_empty()) {
        (false, false) => format!("{} - {}", a, t),
        (false, true) => a,
        (true, false) => t,
        (true, true) => "track".to_string(),
    };
    // Ограничиваем длину под лимит пути Windows ещё до уникализации.
    let mut base = clamp_stem_for_path(dir, base);
    // Зарезервированные имена Windows (CON, NUL, COM1…) — добавляем подчёркивание.
    if is_reserved_windows_name(&base) {
        base.push('_');
    }

    // Существующие имена файлов (без расширения) — один проход по директории.
    let existing: HashSet<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|e| e.path().file_stem().and_then(|s| s.to_str()).map(String::from))
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

/// Удаляет файлы `stem.*` в папке (хвосты yt-dlp: фрагменты, `.part`, `.ytdl`).
/// `keep` — итоговый файл, который нужно сохранить (при успешном скачивании).
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
        // Файлы этого задания: «stem» либо «stem.<что-то>» (включая stem.f140.m4a).
        let name = match p.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name == stem || name.starts_with(&format!("{}.", stem)) {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// Удаляет осиротевшие временные файлы yt-dlp в папке скачиваний (верхний уровень):
/// `.part`/`.ytdl`/фрагменты, оставшиеся после краша или обрыва загрузки. Готовые
/// аудиофайлы (`.opus`/`.mp3`/`.m4a`/…) не трогает — фильтр строго по temp-маркерам.
fn cleanup_orphan_temp_files(dir: &Path) {
    // Трогаем только ДАВНО не изменявшиеся файлы. Параллельная сессия (вторая копия
    // приложения или внешний загрузчик в ту же папку) могла бы прямо сейчас писать
    // свой `.part` — а удаление оборвало бы её загрузку. Активный `.part` постоянно
    // обновляет mtime, поэтому «не трогали сутки» = почти наверняка хвост от краша,
    // а не живая загрузка. При любой неопределённости с возрастом — НЕ удаляем.
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
        // `.part` (частичный файл), `.ytdl` (состояние загрузки), `.part-Frag…`
        // и прочие `.part-*` (фрагменты DASH/HLS), а также брошенные файлы-карантины
        // удаления (`.lfm-trash-…`).
        let is_temp = lower.ends_with(".part")
            || lower.ends_with(".ytdl")
            || lower.contains(".part-")
            || lower.starts_with(".lfm-trash-");
        if !is_temp {
            continue;
        }
        // Свежие (возможно, активные) файлы пропускаем. Любая ошибка чтения времени
        // → считаем файл свежим и НЕ удаляем (перестраховка против гонок).
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
    // Предпочитаем поддерживаемое аудио-расширение (на случай, если рядом остался
    // промежуточный файл ремукса), иначе — первый подходящий.
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

/// Извлекает осмысленное сообщение об ошибке из stderr yt-dlp. Предпочитает
/// собственную ошибку yt-dlp (последняя строка `ERROR: …`); для «голого» Python-
/// трейсбэка (например `OSError: [Errno 22] Invalid argument`) возвращает
/// последние непустые строки — с контекстом, где именно упало, — а не одну
/// загадочную строку исключения без указания операции/файла.
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

/// Извлекает процент из строки прогресса yt-dlp вида `[download]  12.3% of ...`.
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

// ──────────────────────────────────────────────────────────────────────────────
// Теги (lofty) + регистрация в библиотеке
// ──────────────────────────────────────────────────────────────────────────────

/// Общий HTTP-клиент для обложек (инициализация пула/сертификатов дорогая —
/// держим один экземпляр на процесс).
fn cover_client() -> &'static reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            // Без User-Agent часть CDN (в т.ч. Apple/iTunes) отвечает 403 — тогда
            // обложки скачанных треков периодически не подгружались бы.
            .user_agent("LFMusic/0.1.0 (+https://github.com/mel1x/LFMusic)")
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    })
}

fn fetch_cover_bytes(url: &str) -> Option<Vec<u8>> {
    const MAX: usize = 8 * 1024 * 1024;
    let resp = cover_client().get(url).send().ok()?.error_for_status().ok()?;
    // Отсекаем заведомо большие картинки по Content-Length, не загружая их целиком.
    if let Some(len) = resp.content_length() {
        if len > MAX as u64 {
            return None;
        }
    }
    // Читаем не более MAX+1 байт — страховка от слишком большого/битого ответа без
    // корректного Content-Length (chunked): держим в памяти только верхнюю границу.
    let mut bytes: Vec<u8> = Vec::new();
    resp.take((MAX + 1) as u64).read_to_end(&mut bytes).ok()?;
    if bytes.len() < 1000 || bytes.len() > MAX {
        return None;
    }
    Some(bytes)
}

/// Определяет MIME обложки по сигнатуре (jpeg/png/webp/gif/bmp). lofty не должен
/// помечать PNG/WebP как Jpeg — иначе обложка может не отобразиться.
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

/// Проставляет теги (название/исполнитель/альбом + обложку) в скачанный файл.
/// Best-effort: ошибки лишь логируются (контейнер вроде webm может не писаться).
fn write_tags(path: &Path, title: &str, artist: &str, album: Option<&str>, cover_url: Option<&str>) {
    use lofty::config::WriteOptions;
    use lofty::picture::{Picture, PictureType};
    use lofty::prelude::*;
    use lofty::tag::Tag;

    // ВАЖНО: тип файла определяем по содержимому (см. `read_tagged_file`), а не по
    // расширению. yt-dlp ремуксит Opus-аудио в контейнер, у которого lofty по
    // расширению ждёт Vorbis; без определения по магическим байтам открытие
    // падает с «File missing magic signature», и теги/обложка не записываются.
    let mut tagged = match crate::audio_player::AudioPlayer::read_tagged_file(path) {
        Some(t) => t,
        None => {
            eprintln!("[downloads] не удалось открыть для тегирования: {}", path.display());
            return;
        }
    };

    let tag_type = tagged.primary_tag_type();
    if tagged.primary_tag().is_none() {
        tagged.insert_tag(Tag::new(tag_type));
    }
    let Some(tag) = tagged.primary_tag_mut() else {
        return;
    };

    tag.set_title(title.to_string());
    tag.set_artist(artist.to_string());
    if let Some(al) = album {
        if !al.is_empty() {
            tag.set_album(al.to_string());
        }
    }
    // Обложка: сначала прямой URL из результата поиска (Deezer/iTunes CDN), а если
    // его нет или скачать не вышло — добираем через внешние провайдеры по
    // исполнителю/альбому/названию (тот же механизм, что и для локальных треков).
    if let Some(bytes) = resolve_cover_bytes(cover_url, artist, album.unwrap_or(""), title) {
        let mime = detect_image_mime(&bytes);
        let pic = Picture::new_unchecked(PictureType::CoverFront, Some(mime), None, bytes);
        tag.push_picture(pic);
    }

    if let Err(e) = tag.save_to_path(path, WriteOptions::default()) {
        eprintln!("[downloads] не удалось записать теги: {}", e);
    }
}

/// Достаёт байты обложки: пробует прямой `cover_url`, при неудаче — внешние
/// источники (iTunes/Deezer/Cover Art Archive) по метаданным трека. Любой провал
/// — `None` (тегирование без обложки).
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

/// Регистрирует папку скачиваний как библиотечную и пересканирует её, чтобы
/// новые треки появились в каталоге.
/// Точечно добавляет ОДИН только что скачанный файл в библиотеку (без обхода
/// всей папки — путь известен) и возвращает исход. На успехе эмитит
/// `downloads_library_updated` (фронт обновит каталог); при ошибке НЕ эмитит —
/// воркер покажет её как ошибку импорта трека. `dir` — папка скачиваний (нужна
/// для определения `folder_path` и дедупа с уже добавленными папками).
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
    use std::path::Path;

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
        // Даже с уникализацией и худшим временным суффиксом yt-dlp путь обязан
        // уложиться в Windows MAX_PATH (260) — иначе sanitize_open даёт [Errno 22].
        let worst = format!("{}\\{} (12).f251.webm.part", dir.display(), stem);
        assert!(worst.chars().count() < 260, "len = {}", worst.chars().count());
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
}
