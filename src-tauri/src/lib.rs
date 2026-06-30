mod audio_features;
mod audio_player;
mod catalog_search;
mod covers;
mod discord_rpc;
mod downloads;
mod external;
mod library;
mod mpris;
mod pins;
mod player_state;
mod settings;
mod spectrum;
pub mod statistics;
mod taskbar;
mod thumbnails;
mod tools;
mod track_covers;
mod window_state;

use audio_player::{AudioPlayer, EqSettings, PlayerStatus, TrackMetadata};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use library::{AudioFeatureProgressEmitter, FolderInfo, GenreCount, MusicLibrary, PlaylistInfo};
use player_state::SavedPlayerState;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::Instant;
use tauri::{Emitter, Manager, State};

#[derive(Clone, serde::Serialize)]
struct ScanProgress {
    current: usize,
    total: usize,
    percent: f64,
}

#[derive(Clone, serde::Serialize)]
struct AudioFeatureProgress {
    current: usize,
    total: usize,
    percent: f64,
    active: bool,
    /// Текущая активная фаза подготовки: "analysis" (анализ треков для
    /// рекомендаций), "external" (получение данных из интернета), "download"
    /// (скачивание треков) или "done". Фронт по ней выбирает текст прогресса.
    phase: &'static str,
}

const SCAN_PROGRESS_EMIT_MS: u64 = 120;
const AUDIO_FEATURE_PROGRESS_EMIT_MS: u64 = 250;

fn emit_scan_progress(app: &tauri::AppHandle, current: usize, total: usize) {
    let percent = if total > 0 {
        (current as f64 / total as f64) * 100.0
    } else {
        100.0
    };
    let _ = app.emit(
        "scan_progress",
        ScanProgress {
            current,
            total,
            percent,
        },
    );
}

fn scan_progress_emitter(app: tauri::AppHandle) -> impl Fn(usize, usize) + Send + Sync {
    let started = Instant::now();
    let last_emit_ms = Arc::new(AtomicU64::new(0));

    move |current, total| {
        let done = total == 0 || current >= total;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let last = last_emit_ms.load(Ordering::Relaxed);
        if !done && elapsed_ms.saturating_sub(last) < SCAN_PROGRESS_EMIT_MS {
            return;
        }

        if done
            || last_emit_ms
                .compare_exchange(last, elapsed_ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            emit_scan_progress(&app, current, total);
        }
    }
}

fn emit_audio_feature_progress(
    app: &tauri::AppHandle,
    current: usize,
    total: usize,
    percent: f64,
    phase: &'static str,
) {
    let active = total > 0 && current < total;
    let _ = app.emit(
        "audio_feature_progress",
        AudioFeatureProgress {
            current,
            total,
            percent,
            active,
            phase,
        },
    );
}

/// Объединённый прогресс «подготовки»: фаза анализа аудио-фич (`f_*`), фаза
/// подготовки внешних источников (`e_*`) и фаза скачивания треков (`d_*`)
/// складываются в один прогресс-бар (TitleBar.vue). Внешние шаги дописываются к
/// анализу, скачивание — последней фазой (см. выбор `phase` ниже).
#[derive(Clone, Copy)]
struct PrepCounts {
    f_done: usize,
    f_total: usize,
    e_done: usize,
    e_total: usize,
    d_done: usize,
    d_total: usize,
    /// Доля прогресса текущего скачиваемого трека (0..1). Прибавляется к числу
    /// готовых только при расчёте процента — чтобы бар в шапке шёл плавно, а не
    /// прыгал «потреково». На счётчик «готово/всего» (current/total) не влияет.
    d_frac: f64,
}

static PREP_COUNTS: Mutex<PrepCounts> = Mutex::new(PrepCounts {
    f_done: 0,
    f_total: 0,
    e_done: 0,
    e_total: 0,
    d_done: 0,
    d_total: 0,
    d_frac: 0.0,
});
static PREP_LAST_EMIT_MS: AtomicU64 = AtomicU64::new(0);

fn prep_clock() -> &'static Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now)
}

/// Применить изменение к счётчикам и (с троттлингом) эмитнуть объединённый
/// прогресс. По достижении 100% счётчики сбрасываются — следующий проход
/// (новый анализ/подготовка) начинается с чистого листа.
fn report_prep_progress(app: &tauri::AppHandle, mutate: impl FnOnce(&mut PrepCounts)) {
    let (current, total, percent, force_done, phase) = {
        let mut p = PREP_COUNTS.lock().unwrap_or_else(|e| e.into_inner());
        mutate(&mut p);
        let current = p.f_done + p.e_done + p.d_done;
        let total = p.f_total + p.e_total + p.d_total;
        let done = total == 0 || current >= total;
        // Фаза для текста прогресса: получение из интернета ("external")
        // приоритетнее анализа ("analysis"); скачивание ("download") показывается
        // последним — только когда анализ и внешние источники уже завершены.
        let phase = if p.e_total > 0 && p.e_done < p.e_total {
            "external"
        } else if p.f_total > 0 && p.f_done < p.f_total {
            "analysis"
        } else if p.d_total > 0 && p.d_done < p.d_total {
            "download"
        } else {
            "done"
        };
        // Плавный процент: к числу готовых добавляем долю текущего скачиваемого
        // трека (d_frac ∈ [0,1)). Так бар не прыгает с 0 сразу на «трек готов».
        let percent = if total > 0 {
            ((current as f64 + p.d_frac.clamp(0.0, 1.0)) / total as f64 * 100.0).clamp(0.0, 100.0)
        } else {
            100.0
        };
        if done && total > 0 {
            *p = PrepCounts {
                f_done: 0,
                f_total: 0,
                e_done: 0,
                e_total: 0,
                d_done: 0,
                d_total: 0,
                d_frac: 0.0,
            };
        }
        (current, total, percent, done, phase)
    };

    let elapsed_ms = prep_clock().elapsed().as_millis() as u64;
    let last = PREP_LAST_EMIT_MS.load(Ordering::Relaxed);
    if !force_done && elapsed_ms.saturating_sub(last) < AUDIO_FEATURE_PROGRESS_EMIT_MS {
        return;
    }
    if force_done
        || PREP_LAST_EMIT_MS
            .compare_exchange(last, elapsed_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        emit_audio_feature_progress(app, current, total, percent, phase);
    }
}

fn audio_feature_progress_emitter(app: tauri::AppHandle) -> AudioFeatureProgressEmitter {
    Arc::new(move |current, total| {
        report_prep_progress(&app, |p| {
            p.f_total = total;
            p.f_done = current.min(total);
        });
    })
}

/// Обновить «внешнюю» фазу объединённого прогресса.
fn report_external_progress(app: &tauri::AppHandle, done: usize, total: usize) {
    report_prep_progress(app, |p| {
        p.e_total = total;
        p.e_done = done.min(total);
    });
}

/// Обновить фазу «скачивание треков» объединённого прогресса (вызывается из
/// менеджера загрузок). Эта фаза показывается последней — после анализа и
/// внешних источников.
pub(crate) fn report_download_progress(app: &tauri::AppHandle, done: usize, total: usize) {
    report_prep_progress(app, |p| {
        p.d_total = total;
        p.d_done = done.min(total);
        // Целочисленное обновление (трек поставлен/завершён/убран) — сбрасываем
        // дробь текущего трека, чтобы она не «висела» от предыдущего скачивания.
        p.d_frac = 0.0;
    });
}

/// То же, но с долей прогресса текущего скачиваемого трека (`frac` ∈ [0,1]) — для
/// плавного процента в шапке во время загрузки. `done`/`total` остаются числом
/// готовых/всего треков (счётчик в тексте прогресса не дробится).
pub(crate) fn report_download_progress_frac(
    app: &tauri::AppHandle,
    done: usize,
    total: usize,
    frac: f64,
) {
    report_prep_progress(app, |p| {
        p.d_total = total;
        p.d_done = done.min(total);
        p.d_frac = frac.clamp(0.0, 1.0);
    });
}

// ──────────────────────────────────────────────────────────────────────────────
// State types
// ──────────────────────────────────────────────────────────────────────────────

pub struct PlayerState(pub RwLock<AudioPlayer>);
pub struct LibraryState(pub Mutex<MusicLibrary>);
pub struct SettingsState(pub Mutex<(settings::AppSettings, PathBuf)>);

// ──────────────────────────────────────────────────────────────────────────────
// Команды плеера
// ──────────────────────────────────────────────────────────────────────────────

/// Применить кэшированный accent_color из БД или сохранить новый,
/// а также наложить пользовательские переопределения трека
/// (название, исполнитель, обложка) — чтобы при воспроизведении плеер
/// показывал отредактированные пользователем значения.
fn apply_accent_color(
    library: &State<LibraryState>,
    file_path: &str,
    metadata: &mut TrackMetadata,
) {
    let lib = match library.0.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };

    // Переопределения title/artist/cover
    if let Some(t) = lib.get_track(file_path) {
        if let Some(ref title) = t.title {
            metadata.title = Some(title.clone());
        }
        if let Some(ref artist) = t.artist {
            metadata.artist = Some(artist.clone());
        }
        metadata.bpm = t.bpm;
        metadata.loudness = t.loudness;
        metadata.energy = t.energy;
        metadata.danceability = t.danceability;
        metadata.spectral_centroid = t.spectral_centroid;
        metadata.key = t.key.clone();
        metadata.audio_fingerprint = t.audio_fingerprint.clone();
        // Пользовательская обложка имеет приоритет; встроенная уже проставлена
        // миниатюрой-URL в get_metadata (см. get_metadata_inner).
        if let Some(sig) = lib.get_override_cover_sig(file_path) {
            metadata.cover_art = Some(thumbnails::cover_protocol_url(file_path, &sig));
        }
    }

    // Проверяем кэшированный accent_color
    match lib.get_accent_color(file_path) {
        Some(cached) => {
            metadata.accent_color = Some(cached);
        }
        None => {
            // Сохраняем accent_color из метаданных, если он есть
            if let Some(ref color) = metadata.accent_color {
                let _ = lib.set_accent_color(file_path, color);
            }
        }
    }
}

#[tauri::command]
fn player_play(
    state: State<PlayerState>,
    library: State<LibraryState>,
    file_path: String,
) -> Result<TrackMetadata, String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    let mut metadata = player.play(&file_path)?;
    drop(player);
    apply_accent_color(&library, &file_path, &mut metadata);
    Ok(metadata)
}

#[tauri::command]
fn player_play_crossfade(
    state: State<PlayerState>,
    library: State<LibraryState>,
    file_path: String,
    crossfade_secs: f64,
) -> Result<TrackMetadata, String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    let mut metadata = player.play_crossfade(&file_path, crossfade_secs)?;
    drop(player);
    apply_accent_color(&library, &file_path, &mut metadata);
    Ok(metadata)
}

#[tauri::command]
fn player_play_paused(
    state: State<PlayerState>,
    library: State<LibraryState>,
    file_path: String,
    position_secs: f64,
) -> Result<TrackMetadata, String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    let mut metadata = player.play_paused(&file_path, position_secs)?;
    drop(player);
    apply_accent_color(&library, &file_path, &mut metadata);
    Ok(metadata)
}

#[tauri::command]
fn player_pause(state: State<PlayerState>) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.pause();
    Ok(())
}

#[tauri::command]
fn player_resume(state: State<PlayerState>) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.resume();
    Ok(())
}

#[tauri::command]
fn player_stop(state: State<PlayerState>) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.stop();
    Ok(())
}

#[tauri::command]
fn player_set_volume(state: State<PlayerState>, volume: f32) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.set_volume(volume);
    Ok(())
}

#[tauri::command]
fn player_get_volume(state: State<PlayerState>) -> Result<f32, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_volume())
}

#[tauri::command]
fn player_seek(state: State<PlayerState>, position_secs: f64) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.seek(position_secs)
}

#[tauri::command]
fn player_seek_crossfade(
    state: State<PlayerState>,
    position_secs: f64,
    crossfade_secs: f64,
) -> Result<(), String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    player.seek_crossfade(position_secs, crossfade_secs)
}

#[tauri::command]
fn player_get_status(state: State<PlayerState>) -> Result<PlayerStatus, String> {
    let mut player = state.0.write().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_status())
}

/// Частотные полосы спектра текущего звука (0..1) для визуализатора.
/// Опрашивается фронтендом на кадрах rAF (30/60 fps).
#[tauri::command]
fn player_get_spectrum(state: State<PlayerState>, bars: usize) -> Result<Vec<f32>, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.spectrum_bars(bars))
}

/// Включить/выключить снятие спектра — фронтенд синхронизирует с видимостью
/// блока-визуализатора, чтобы при выключенном визуализаторе не было лишней работы.
#[tauri::command]
fn player_set_spectrum_enabled(state: State<PlayerState>, enabled: bool) -> Result<(), String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    player.set_spectrum_enabled(enabled);
    Ok(())
}

// Тяжёлые команды помечены (async): выполняются в пуле потоков, а не на
// главном потоке — иначе чтение тегов/обложек блокирует событийный цикл окна.
#[tauri::command(async)]
fn get_track_metadata(
    library: State<LibraryState>,
    file_path: String,
) -> Result<TrackMetadata, String> {
    let mut metadata = AudioPlayer::get_metadata(&file_path)?;
    apply_accent_color(&library, &file_path, &mut metadata);
    Ok(metadata)
}

/// Вернуть URL обложки трека (кастомный протокол `lfmusic-cover`, миниатюра)
/// или `null`, если обложки нет. Дёшево: lookup override в БД + проверка тега;
/// сама миниатюра генерируется лениво обработчиком протокола при показе.
#[tauri::command(async)]
fn get_cover_art(
    library: State<LibraryState>,
    file_path: String,
) -> Result<Option<String>, String> {
    let lib = library.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(thumbnails::cover_url(&lib, &file_path))
}

/// Прочитать оригинальную обложку из файла, игнорируя пользовательские
/// переопределения. Нужно для превью в модалке редактирования: при клике
/// «сбросить обложку» показываем то, что лежит в тегах файла.
#[tauri::command(async)]
fn get_original_cover_art(file_path: String) -> Result<Option<String>, String> {
    Ok(AudioPlayer::extract_cover_art(&file_path))
}

/// Прочитать оригинальные метаданные трека (как они лежат в тегах файла),
/// без применения пользовательских переопределений.
/// Используется кнопкой «Сбросить» в модалке редактирования: туда подставляются
/// оригинальные название/исполнитель/обложка для предпросмотра до сохранения.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OriginalTrack {
    title: Option<String>,
    artist: Option<String>,
    cover_art: Option<String>,
    has_override_cover: bool,
}

#[tauri::command(async)]
fn get_original_track(
    library: State<LibraryState>,
    file_path: String,
) -> Result<OriginalTrack, String> {
    let metadata = AudioPlayer::get_metadata(&file_path)?;
    let has_override_cover = library
        .0
        .lock()
        .map(|lib| lib.has_override_cover(&file_path))
        .unwrap_or(false);
    Ok(OriginalTrack {
        title: metadata.title,
        artist: metadata.artist,
        cover_art: metadata.cover_art,
        has_override_cover,
    })
}

#[tauri::command]
fn get_supported_formats() -> Vec<String> {
    AudioPlayer::supported_formats()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды библиотеки (новые)
// ──────────────────────────────────────────────────────────────────────────────

/// Получить список папок с количеством треков в каждой
#[tauri::command]
fn library_get_folders(state: State<LibraryState>) -> Result<Vec<FolderInfo>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_folders())
}

/// Добавить папку, просканировать
#[tauri::command]
async fn library_add_folder(app: tauri::AppHandle, folder_path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let separators = settings::get_artist_separators(&app.state::<SettingsState>());
        let state = app.state::<LibraryState>();
        let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
        library.add_folder(
            folder_path,
            &separators,
            Some(audio_feature_progress_emitter(app.clone())),
            scan_progress_emitter(app.clone()),
        )?;
        Ok(())
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

fn android_music_folder_candidates() -> Vec<String> {
    let mut seen = HashSet::new();
    let mut folders = Vec::new();
    for candidate in [
        "/storage/emulated/0/Music",
        "/storage/emulated/0/Download",
        "/storage/emulated/0/Downloads",
        "/storage/emulated/0/Podcasts",
        "/storage/emulated/0/Audio",
        "/storage/emulated/0/Sounds",
        "/storage/emulated/0/Recordings",
        "/sdcard/Music",
        "/sdcard/Download",
        "/sdcard/Downloads",
        "/sdcard/Podcasts",
        "/sdcard/Audio",
        "/sdcard/Sounds",
        "/sdcard/Recordings",
    ] {
        let path = PathBuf::from(candidate);
        if !path.is_dir() {
            continue;
        }

        let key = std::fs::canonicalize(&path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        if seen.insert(key) {
            folders.push(candidate.to_string());
        }
    }
    folders
}

#[tauri::command]
async fn library_add_android_music_folders(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let candidates = android_music_folder_candidates();
        if candidates.is_empty() {
            return Err("Не удалось найти стандартные папки с аудио. Разрешите доступ к аудио и проверьте, что на устройстве есть музыкальные файлы.".to_string());
        }

        let separators = settings::get_artist_separators(&app.state::<SettingsState>());
        let state = app.state::<LibraryState>();
        let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut added = 0usize;
        let mut last_error: Option<String> = None;

        for folder_path in candidates {
            match library.add_folder(
                folder_path,
                &separators,
                Some(audio_feature_progress_emitter(app.clone())),
                scan_progress_emitter(app.clone()),
            ) {
                Ok(_) => {
                    added += 1;
                }
                Err(e) => {
                    last_error = Some(e);
                }
            }
        }

        if added == 0 {
            return Err(last_error.unwrap_or_else(|| "Не удалось просканировать стандартные папки с аудио.".to_string()));
        }

        Ok(())
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

/// Удалить папку и все её треки из кэша
#[tauri::command(async)]
fn library_remove_folder(
    app: tauri::AppHandle,
    library_state: State<LibraryState>,
    settings_state: State<SettingsState>,
    folder_path: String,
) -> Result<(), String> {
    let separators = settings::get_artist_separators(&settings_state);
    let mut library = library_state
        .0
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    library.remove_folder(
        folder_path,
        &separators,
        Some(audio_feature_progress_emitter(app)),
    )
}

/// Получить все треки из кэша (без пересканирования)
#[tauri::command(async)]
fn library_get_tracks(state: State<LibraryState>) -> Result<Vec<TrackMetadata>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_all_tracks())
}

/// Упорядоченные пути каталога с фильтром по жанру и/или поисковому запросу.
/// Возвращает только `file_path` (лёгкий список идентичности для виртуального
/// скролла и источника воспроизведения каталога).
#[tauri::command(async)]
fn library_get_catalog_paths(
    state: State<LibraryState>,
    genre: Option<String>,
    search: Option<String>,
) -> Result<Vec<String>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_catalog_paths(genre.as_deref(), search.as_deref()))
}

/// Пакетная материализация метаданных по списку путей (видимое окно каталога,
/// дозагрузка промахов кэша). Порядок результата совпадает с порядком `paths`.
#[tauri::command(async)]
fn library_get_tracks_by_paths(
    state: State<LibraryState>,
    paths: Vec<String>,
) -> Result<Vec<TrackMetadata>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_tracks_by_paths(&paths))
}

/// Список жанров с количеством треков (для фильтр-чипов каталога).
#[tauri::command(async)]
fn library_list_genres(state: State<LibraryState>) -> Result<Vec<GenreCount>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.list_genres())
}

/// Ранжированный поиск треков по каталогу (секция «Треки» страницы поиска).
/// Сохраняет поведение фронтендового searchRanked (точное → нечёткое →
/// раскладка → формат).
#[tauri::command(async)]
fn library_search(
    state: State<LibraryState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<TrackMetadata>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.search_tracks(q, limit))
}

#[tauri::command]
fn library_resume_audio_features(
    app: tauri::AppHandle,
    state: State<LibraryState>,
) -> Result<usize, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.resume_pending_audio_feature_analysis(Some(audio_feature_progress_emitter(app)))
}

#[tauri::command]
fn library_audio_features_status(
    state: State<LibraryState>,
) -> Result<AudioFeatureProgress, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    let (current, total, active) = library.audio_feature_status()?;
    let percent = if total > 0 {
        (current as f64 / total as f64) * 100.0
    } else {
        100.0
    };
    Ok(AudioFeatureProgress {
        current,
        total,
        percent,
        active,
        phase: if active { "analysis" } else { "done" },
    })
}

/// Поднять трек на первое место в каталоге (навсегда)
#[tauri::command]
fn library_move_track_to_top(file_path: String, state: State<LibraryState>) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.move_track_to_top(&file_path)
}

/// Удалить из кэша треки, файлы которых больше не существуют на диске
#[tauri::command(async)]
fn library_cleanup(state: State<LibraryState>) -> Result<usize, String> {
    let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.cleanup_missing_tracks()
}

/// Пересканировать все папки: обновить изменённые файлы, удалить несуществующие
#[tauri::command]
async fn library_rescan(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let separators = settings::get_artist_separators(&app.state::<SettingsState>());
        let state = app.state::<LibraryState>();
        let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
        library.rescan_all(
            &separators,
            Some(audio_feature_progress_emitter(app.clone())),
            scan_progress_emitter(app.clone()),
        )?;
        Ok(())
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

/// Пересобрать плейлисты исполнителей (вызывать после смены разделителей)
#[tauri::command(async)]
fn library_rebuild_artists(
    library_state: State<LibraryState>,
    settings_state: State<SettingsState>,
) -> Result<(), String> {
    let separators = settings::get_artist_separators(&settings_state);
    let mut library = library_state
        .0
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    library.rebuild_artist_playlists(&separators)
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды плейлистов
// ──────────────────────────────────────────────────────────────────────────────

/// Получить все плейлисты
#[tauri::command]
fn playlist_get_all(state: State<LibraryState>) -> Result<Vec<PlaylistInfo>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_playlists())
}

/// Получить треки плейлиста
#[tauri::command(async)]
fn playlist_get_tracks(
    state: State<LibraryState>,
    playlist_id: i64,
) -> Result<Vec<TrackMetadata>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_playlist_tracks(playlist_id))
}

/// Создать пользовательский плейлист
#[tauri::command]
fn playlist_create(state: State<LibraryState>, name: String) -> Result<PlaylistInfo, String> {
    let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.create_playlist(name)
}

/// Переименовать плейлист
#[tauri::command]
fn playlist_rename(
    state: State<LibraryState>,
    playlist_id: i64,
    name: String,
) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.rename_playlist(playlist_id, name)
}

/// Удалить пользовательский плейлист
#[tauri::command]
fn playlist_delete(state: State<LibraryState>, playlist_id: i64) -> Result<(), String> {
    let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.delete_playlist(playlist_id)
}

/// Добавить трек в плейлист
#[tauri::command]
fn playlist_add_track(
    state: State<LibraryState>,
    playlist_id: i64,
    file_path: String,
) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.add_track_to_playlist(playlist_id, &file_path)
}

/// Удалить трек из плейлиста
#[tauri::command]
fn playlist_remove_track(
    state: State<LibraryState>,
    playlist_id: i64,
    file_path: String,
) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.remove_track_from_playlist(playlist_id, &file_path)
}

/// Поднять трек на первое место в плейлисте
#[tauri::command]
fn playlist_move_track_to_top(
    state: State<LibraryState>,
    playlist_id: i64,
    file_path: String,
) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.move_playlist_track_to_top(playlist_id, &file_path)
}

/// Получить id пользовательских плейлистов, в которых уже есть указанный трек
#[tauri::command]
fn playlist_get_ids_for_track(
    state: State<LibraryState>,
    file_path: String,
) -> Result<Vec<i64>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_user_playlist_ids_for_track(&file_path))
}

// ──────────────────────────────────────────────────────────────────────────────
// Порядок треков в альбоме
// ──────────────────────────────────────────────────────────────────────────────

#[tauri::command]
fn album_get_track_order(
    state: State<LibraryState>,
    album_name: String,
) -> Result<Option<Vec<String>>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.get_album_track_order(&album_name)
}

#[tauri::command]
fn album_set_track_order(
    state: State<LibraryState>,
    album_name: String,
    track_paths: Vec<String>,
) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.set_album_track_order(&album_name, &track_paths)
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды внешних источников
// ──────────────────────────────────────────────────────────────────────────────

/// Получить макет альбома по данным внешних источников: канонический порядок
/// треков + «фантомные» (отсутствующие в библиотеке) треки. Результат
/// кэшируется в БД, поэтому повторные открытия не ходят в сеть.
///
/// Сетевой запрос выполняется на blocking-пуле (spawn_blocking), чтобы не
/// занимать tokio-воркеры и не блокировать главный поток UI.
#[tauri::command]
async fn external_album_layout(
    app: tauri::AppHandle,
    album_name: String,
    refresh: bool,
) -> Result<external::AlbumLayout, String> {
    tauri::async_runtime::spawn_blocking(move || run_album_layout(&app, &album_name, refresh))
        .await
        .map_err(|e| format!("Task error: {}", e))?
}

/// Синхронное ядро [`external_album_layout`] (используется и командой через
/// spawn_blocking, и батч-проходом подготовки). Блокировку библиотеки НЕ держим
/// во время сетевого запроса. Кэш в БД бессрочный — повторно в сеть не ходим.
fn run_album_layout(
    app: &tauri::AppHandle,
    album_name: &str,
    refresh: bool,
) -> Result<external::AlbumLayout, String> {
    let state = app.state::<LibraryState>();

    let none_layout = || external::AlbumLayout {
        status: "none".into(),
        source: None,
        entries: Vec::new(),
    };

    let now = external::unix_secs();

    // Фаза 1 (под блокировкой): читаем локальные треки и кэш, решаем, нужна сеть.
    let (local, query_artist, album_key, decision) = {
        let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
        let local = library.get_album_local_tracks(album_name);
        if local.is_empty() {
            return Ok(none_layout());
        }

        // Самый частый исполнитель среди локальных треков — для запроса.
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (_, _, artist) in &local {
            if !artist.is_empty() {
                *counts.entry(artist.clone()).or_default() += 1;
            }
        }
        let query_artist = counts
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .map(|(a, _)| a)
            .unwrap_or_default();

        // Ключ кэша зависит и от названия, и от артиста — два разных
        // одноимённых альбома от разных исполнителей не путаются.
        let album_key = external::album_cache_key(album_name, &query_artist);

        // 0 = кэш; 1 = сеть; 2 = none из (бессрочного) негативного кэша.
        let decision: (u8, Vec<external::ExternalTrack>, Option<String>) =
            match library.get_external_album(&album_key) {
                Some((src, status, _, tracks)) if !refresh && status == "ok" => (0, tracks, src),
                Some((_, status, _, _)) if !refresh && status == "none" => (2, Vec::new(), None),
                _ => (1, Vec::new(), None),
            };
        (local, query_artist, album_key, decision)
    };

    let (mode, cached_tracks, cached_source) = decision;
    if mode == 2 {
        return Ok(none_layout());
    }

    let local_for_match: Vec<(String, String)> =
        local.into_iter().map(|(p, t, _)| (p, t)).collect();

    if mode == 0 {
        if cached_tracks.is_empty() {
            return Ok(none_layout());
        }
        let entries = external::build_layout(&cached_tracks, &local_for_match);
        return Ok(external::AlbumLayout {
            status: "ok".into(),
            source: cached_source,
            entries,
        });
    }

    // Фаза 2 (без блокировки): сеть.
    let fetched = external::fetch_album_tracklist(&query_artist, album_name);

    // Фаза 3 (под блокировкой): сохраняем результат в кэш.
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    match fetched {
        external::FetchOutcome::Found(album) if !album.tracks.is_empty() => {
            let _ = library.set_external_album(
                &album_key,
                Some(&album.source),
                "ok",
                &album.tracks,
                now,
            );
            let entries = external::build_layout(&album.tracks, &local_for_match);
            Ok(external::AlbumLayout {
                status: "ok".into(),
                source: Some(album.source),
                entries,
            })
        }
        // Все источники реально ответили «нет» (или вернули пустой список) —
        // кэшируем негативный результат, чтобы не ходить в сеть повторно.
        external::FetchOutcome::Found(_) | external::FetchOutcome::NotFound => {
            let _ = library.set_external_album(&album_key, None, "none", &[], now);
            Ok(none_layout())
        }
        // Временная ошибка провайдера (сеть/парсинг/rate-limit) — НЕ кэшируем
        // «нет», иначе альбом навсегда останется без внешнего порядка.
        external::FetchOutcome::TemporaryError => Ok(none_layout()),
    }
}

/// Получить аватарку исполнителя из внешнего источника (с дисковым кэшем).
#[tauri::command]
async fn external_artist_image(
    app: tauri::AppHandle,
    artist_name: String,
    refresh: bool,
) -> Result<external::ArtistImageResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = {
            let state = app.state::<SettingsState>();
            let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
            settings.1.clone()
        };
        Ok(external::get_or_fetch_artist_image(
            &dir,
            &artist_name,
            refresh,
        ))
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

/// Получить обложку для трека без встроенной обложки (по альбому/названию).
#[tauri::command]
async fn external_track_cover(
    app: tauri::AppHandle,
    artist: String,
    album: String,
    title: String,
    refresh: bool,
) -> Result<external::ArtistImageResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = {
            let state = app.state::<SettingsState>();
            let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
            settings.1.clone()
        };
        Ok(external::get_or_fetch_track_cover(
            &dir, &artist, &album, &title, refresh,
        ))
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

/// Загрузить все уже закэшированные аватарки исполнителей (для быстрой
/// отрисовки на старте без обращения к сети).
#[tauri::command]
async fn external_load_cached_artist_images(
    app: tauri::AppHandle,
) -> Result<Vec<external::CachedArtistImage>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = {
            let state = app.state::<SettingsState>();
            let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
            settings.1.clone()
        };
        Ok(external::load_cached_artist_images(&dir))
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

// ──────────────────────────────────────────────────────────────────────────────
// Батч-подготовка внешних источников (как анализ: один проход, бессрочный кэш)
// ──────────────────────────────────────────────────────────────────────────────

/// Поколение воркера подготовки — повторный вызов команды отменяет предыдущий
/// проход (например, при переключении настроек).
static EXTERNAL_PREP_GENERATION: AtomicU64 = AtomicU64::new(0);

fn external_prep_current(generation: u64) -> bool {
    EXTERNAL_PREP_GENERATION.load(Ordering::SeqCst) == generation
}

/// Запустить (или перезапустить) фоновую подготовку внешних источников: один
/// проход по библиотеке заранее тянет и кэширует обложки/аватарки/порядок
/// альбомов для включённых источников. Прогресс дописывается к прогрессу
/// анализа треков. Уже обработанное (ok/none) в сеть повторно не ходит.
/// Безопасно вызывать на старте и при любом переключении внешних настроек.
#[tauri::command]
fn external_prepare(app: tauri::AppHandle) -> Result<(), String> {
    let generation = EXTERNAL_PREP_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    let (dir, covers_on, avatars_on, albums_on) = {
        let state = app.state::<SettingsState>();
        let s = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
        (
            s.1.clone(),
            settings::option_bool(&s.0, "external_track_covers", false),
            settings::option_bool(&s.0, "external_artist_avatars", false),
            settings::option_bool(&s.0, "external_album_order", false)
                || settings::option_bool(&s.0, "external_supplement_missing", false),
        )
    };

    // Сбрасываем внешнюю фазу прогресса (актуально при выключении источников —
    // прогресс-бар спрячется, если анализ тоже завершён).
    report_external_progress(&app, 0, 0);

    if !covers_on && !avatars_on && !albums_on {
        return Ok(());
    }

    let _ = std::thread::Builder::new()
        .name("external-prepare".into())
        .spawn(move || {
            run_external_preparation(app, dir, covers_on, avatars_on, albums_on, generation)
        });
    Ok(())
}

/// Группа треков, делящих один ключ кэша обложки (как правило — один альбом).
struct CoverGroup {
    artist: String,
    album: String,
    title: String,
    paths: Vec<String>,
}

/// Одно задание фоновой подготовки внешних источников (для пула воркеров).
enum PrepTask {
    /// Аватарка исполнителя (по имени).
    Artist(String),
    /// Внешняя обложка для группы треков (как правило — альбом).
    Cover(CoverGroup),
    /// Канонический порядок треков альбома + «фантомы» (по названию альбома).
    Album(String),
}

/// Степень параллелизма фоновой подготовки. Задания бьют по разным хостам
/// (Deezer/iTunes/MusicBrainz/CAA/Wikipedia), а пер-хост-лимитер
/// (`external::send_throttled`) сам разводит запросы к одному хосту во времени —
/// поэтому несколько потоков безопасны для rate-limit и дают кратный выигрыш:
/// пропускная способность упирается в интервал хоста, а не в сумму сетевых задержек.
/// Манифесты кэша сериализуются отдельными мьютексами (см. external.rs).
const EXTERNAL_PREP_CONCURRENCY: usize = 6;

/// Тело воркера пула подготовки: тянет задания по общему курсору, пока они есть
/// (или пока проход не отменён сменой поколения). Прогресс — по общему атомарному
/// счётчику готовых.
#[allow(clippy::too_many_arguments)]
fn external_prep_worker(
    app: tauri::AppHandle,
    tasks: Arc<Vec<PrepTask>>,
    cursor: Arc<AtomicUsize>,
    done: Arc<AtomicUsize>,
    none_keys: Arc<Mutex<Vec<String>>>,
    override_paths: Arc<HashSet<String>>,
    dir: Arc<PathBuf>,
    total: usize,
    generation: u64,
) {
    loop {
        if !external_prep_current(generation) {
            return;
        }
        let idx = cursor.fetch_add(1, Ordering::SeqCst);
        if idx >= tasks.len() {
            return;
        }
        match &tasks[idx] {
            PrepTask::Artist(name) => {
                let _ = external::get_or_fetch_artist_image(&dir, name, false);
            }
            PrepTask::Cover(g) => {
                // Альбом нуждается во внешней обложке, если хотя бы у одного трека
                // нет ни override-, ни встроенной обложки.
                let needs_external = g
                    .paths
                    .iter()
                    .any(|p| !override_paths.contains(p) && !AudioPlayer::has_embedded_cover(p));
                if needs_external {
                    let _ =
                        external::get_or_fetch_track_cover(&dir, &g.artist, &g.album, &g.title, false);
                } else {
                    // Локально покрыт — помечаем «нет внешней» (одним сохранением в
                    // конце), чтобы при следующем старте не перечитывать теги альбома.
                    none_keys
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(external::track_cover_key(&g.artist, &g.album, &g.title));
                }
            }
            PrepTask::Album(name) => {
                let _ = run_album_layout(&app, name, false);
            }
        }
        let d = done.fetch_add(1, Ordering::SeqCst) + 1;
        report_external_progress(&app, d, total);
    }
}

fn run_external_preparation(
    app: tauri::AppHandle,
    dir: PathBuf,
    covers_on: bool,
    avatars_on: bool,
    albums_on: bool,
    generation: u64,
) {
    // ── Сбор заданий (короткие блокировки библиотеки) ──
    let artists: Vec<String> = if avatars_on {
        let state = app.state::<LibraryState>();
        let names = match state.0.lock() {
            Ok(lib) => lib.get_artists().into_iter().map(|p| p.name).collect(),
            Err(_) => Vec::new(),
        };
        names
    } else {
        Vec::new()
    };

    let albums: Vec<String> = if albums_on {
        let state = app.state::<LibraryState>();
        let names = match state.0.lock() {
            Ok(lib) => lib.pending_external_album_names(),
            Err(_) => Vec::new(),
        };
        names
    } else {
        Vec::new()
    };

    let (cover_groups, override_paths): (Vec<CoverGroup>, HashSet<String>) = if covers_on {
        let state = app.state::<LibraryState>();
        let built = match state.0.lock() {
            Ok(lib) => {
                let overrides = lib.override_cover_paths();
                let mut by_key: std::collections::HashMap<String, CoverGroup> =
                    std::collections::HashMap::new();
                for t in lib.get_all_tracks_light() {
                    let artist = t.artist.unwrap_or_default();
                    let album = t.album.unwrap_or_default();
                    let title = t.title.unwrap_or_default();
                    let key = external::track_cover_key(&artist, &album, &title);
                    if key.is_empty() {
                        continue;
                    }
                    by_key
                        .entry(key)
                        .or_insert_with(|| CoverGroup {
                            artist,
                            album,
                            title,
                            paths: Vec::new(),
                        })
                        .paths
                        .push(t.file_path);
                }
                (by_key.into_values().collect(), overrides)
            }
            Err(_) => (Vec::new(), HashSet::new()),
        };
        built
    } else {
        (Vec::new(), HashSet::new())
    };

    // ── Отфильтровываем уже обработанное, считаем общий объём ──
    let artist_settled = if avatars_on {
        external::artist_settled_keys(&dir)
    } else {
        HashSet::new()
    };
    let pending_artists: Vec<String> = artists
        .into_iter()
        .filter(|a| {
            !a.trim().is_empty() && !artist_settled.contains(&external::artist_image_key(a))
        })
        .collect();

    let cover_settled = if covers_on {
        external::cover_settled_keys(&dir)
    } else {
        HashSet::new()
    };
    let pending_covers: Vec<CoverGroup> = cover_groups
        .into_iter()
        .filter(|g| {
            !cover_settled.contains(&external::track_cover_key(&g.artist, &g.album, &g.title))
        })
        .collect();

    let total = pending_artists.len() + pending_covers.len() + albums.len();
    if total == 0 {
        report_external_progress(&app, 0, 0);
        return;
    }

    report_external_progress(&app, 0, total);

    // Единый список заданий, ЧЕРЕДУЯ типы (аватарка/обложка/альбом): так в каждый
    // момент пул бьёт по разным хостам (аватарки → Deezer, обложки/альбомы → iTunes…),
    // а не упирается всеми потоками в один хост.
    let mut tasks: Vec<PrepTask> = Vec::with_capacity(total);
    {
        let mut ai = pending_artists.into_iter();
        let mut ci = pending_covers.into_iter();
        let mut bi = albums.into_iter();
        loop {
            let mut pushed = false;
            if let Some(a) = ai.next() {
                tasks.push(PrepTask::Artist(a));
                pushed = true;
            }
            if let Some(c) = ci.next() {
                tasks.push(PrepTask::Cover(c));
                pushed = true;
            }
            if let Some(b) = bi.next() {
                tasks.push(PrepTask::Album(b));
                pushed = true;
            }
            if !pushed {
                break;
            }
        }
    }

    let tasks = Arc::new(tasks);
    let cursor = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicUsize::new(0));
    let none_keys = Arc::new(Mutex::new(Vec::<String>::new()));
    let override_paths = Arc::new(override_paths);
    let dir = Arc::new(dir);

    // Пул воркеров: N−1 фоновых потоков + текущий поток как ещё один воркер (так
    // прогресс гарантирован, даже если часть spawn не удалась, и поток подготовки
    // не простаивает). Каждый воркер тянет задания по общему курсору.
    let worker_count = EXTERNAL_PREP_CONCURRENCY.min(tasks.len()).max(1);
    let mut handles = Vec::with_capacity(worker_count.saturating_sub(1));
    for i in 1..worker_count {
        let app = app.clone();
        let tasks = Arc::clone(&tasks);
        let cursor = Arc::clone(&cursor);
        let done = Arc::clone(&done);
        let none_keys = Arc::clone(&none_keys);
        let override_paths = Arc::clone(&override_paths);
        let dir = Arc::clone(&dir);
        if let Ok(h) = std::thread::Builder::new()
            .name(format!("external-prep-{}", i))
            .spawn(move || {
                external_prep_worker(
                    app,
                    tasks,
                    cursor,
                    done,
                    none_keys,
                    override_paths,
                    dir,
                    total,
                    generation,
                )
            })
        {
            handles.push(h);
        }
    }
    external_prep_worker(
        app.clone(),
        Arc::clone(&tasks),
        Arc::clone(&cursor),
        Arc::clone(&done),
        Arc::clone(&none_keys),
        Arc::clone(&override_paths),
        Arc::clone(&dir),
        total,
        generation,
    );
    for h in handles {
        let _ = h.join();
    }

    // Локально покрытые обложки помечаем «нет внешней» одним сохранением — после
    // того как все воркеры закончили собирать их ключи.
    if external_prep_current(generation) {
        let keys = none_keys.lock().unwrap_or_else(|e| e.into_inner());
        external::mark_track_covers_none(&dir, &keys);
    }
}

#[tauri::command]
fn statistics_add_track_delta(
    state: State<LibraryState>,
    delta: statistics::TrackStatisticsDelta,
) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.add_track_statistics_delta(delta)
}

#[tauri::command]
fn statistics_get_track(
    state: State<LibraryState>,
    file_path: String,
) -> Result<statistics::TrackStatistics, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.get_track_statistics(&file_path)
}

#[tauri::command(async)]
fn statistics_sum_listen_count(
    state: State<LibraryState>,
    file_paths: Vec<String>,
) -> Result<i64, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.sum_track_listen_count(&file_paths)
}

#[tauri::command]
fn statistics_add_collection_full_listen(
    state: State<LibraryState>,
    kind: statistics::CollectionKind,
    key: String,
) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.add_collection_full_listen(kind, &key)
}

#[tauri::command(async)]
fn statistics_get_recently_played(
    state: State<LibraryState>,
    limit: usize,
) -> Result<Vec<audio_player::TrackMetadata>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_recently_played(limit))
}

#[tauri::command(async)]
fn statistics_get_most_played(
    state: State<LibraryState>,
    limit: usize,
) -> Result<Vec<audio_player::TrackMetadata>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_most_played(limit))
}

#[tauri::command(async)]
fn statistics_get_top_artists(
    state: State<LibraryState>,
    limit: usize,
) -> Result<Vec<library::TopArtist>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_top_artists(limit))
}

#[tauri::command(async)]
fn statistics_get_top_albums(
    state: State<LibraryState>,
    limit: usize,
) -> Result<Vec<library::TopAlbum>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_top_albums(limit))
}

#[tauri::command(async)]
fn statistics_get_summary_stats(
    state: State<LibraryState>,
) -> Result<library::SummaryStats, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_summary_stats())
}

#[tauri::command(async)]
fn statistics_get_recommendations(
    state: State<LibraryState>,
    mood: String,
    familiarity: Option<String>,
    lossless_only: Option<bool>,
    limit: usize,
) -> Result<Vec<audio_player::TrackMetadata>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_recommendations_by_mood(
        &mood,
        familiarity.as_deref().unwrap_or("balanced"),
        lossless_only.unwrap_or(false),
        limit,
    ))
}

#[tauri::command(async)]
fn statistics_get_taste_profile(
    state: State<LibraryState>,
) -> Result<library::TasteProfile, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.taste_profile())
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды эквалайзера
// ──────────────────────────────────────────────────────────────────────────────

/// Сохранить EQ в общий settings.json
fn eq_persist(settings_state: &State<SettingsState>, eq: &EqSettings) -> Result<(), String> {
    let mut state = settings_state
        .0
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    state.0.eq_settings = eq.clone();
    settings::save(&state.1, &state.0)
}

#[tauri::command]
fn eq_set_bands(
    player_state: State<PlayerState>,
    settings_state: State<SettingsState>,
    bands: Vec<f32>,
) -> Result<(), String> {
    let player = player_state
        .0
        .read()
        .map_err(|e| format!("Lock error: {}", e))?;
    player.set_eq_bands(bands);
    let eq = player.get_eq_settings();
    eq_persist(&settings_state, &eq)
}

#[tauri::command]
fn eq_get_bands(state: State<PlayerState>) -> Result<Vec<f32>, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_eq_bands())
}

#[tauri::command]
fn eq_set_gain(
    player_state: State<PlayerState>,
    settings_state: State<SettingsState>,
    gain: f32,
) -> Result<(), String> {
    let player = player_state
        .0
        .read()
        .map_err(|e| format!("Lock error: {}", e))?;
    player.set_eq_gain(gain);
    let eq = player.get_eq_settings();
    eq_persist(&settings_state, &eq)
}

#[tauri::command]
fn eq_get_gain(state: State<PlayerState>) -> Result<f32, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_eq_gain())
}

#[tauri::command]
fn eq_set_enabled(
    player_state: State<PlayerState>,
    settings_state: State<SettingsState>,
    enabled: bool,
) -> Result<(), String> {
    let player = player_state
        .0
        .read()
        .map_err(|e| format!("Lock error: {}", e))?;
    player.set_eq_enabled(enabled);
    let eq = player.get_eq_settings();
    eq_persist(&settings_state, &eq)
}

#[tauri::command]
fn eq_get_enabled(state: State<PlayerState>) -> Result<bool, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_eq_enabled())
}

#[tauri::command]
fn eq_get_settings(state: State<PlayerState>) -> Result<EqSettings, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_eq_settings())
}

#[tauri::command]
fn eq_set_settings(
    player_state: State<PlayerState>,
    settings_state: State<SettingsState>,
    eq_settings: EqSettings,
) -> Result<(), String> {
    let player = player_state
        .0
        .read()
        .map_err(|e| format!("Lock error: {}", e))?;
    player.set_eq_settings(eq_settings);
    let eq = player.get_eq_settings();
    eq_persist(&settings_state, &eq)
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды настроек
// ──────────────────────────────────────────────────────────────────────────────

/// Фронтенд может хранить опции разных типов (булевые, строковые и т.д.)
#[tauri::command]
fn get_settings(
    state: State<SettingsState>,
) -> Result<std::collections::HashMap<String, serde_json::Value>, String> {
    let state = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(state.0.options.clone())
}

#[tauri::command]
fn save_settings(
    state: State<SettingsState>,
    settings: std::collections::HashMap<String, serde_json::Value>,
) -> Result<(), String> {
    let mut state = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    state.0.options = settings;
    settings::save(&state.1, &state.0)
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды состояния плеера
// ──────────────────────────────────────────────────────────────────────────────

#[tauri::command]
fn save_player_state(
    state: State<SettingsState>,
    player_state: SavedPlayerState,
) -> Result<(), String> {
    let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    player_state::save(&settings.1, &player_state)
}

#[tauri::command]
fn load_player_state(state: State<SettingsState>) -> Result<SavedPlayerState, String> {
    let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player_state::load(&settings.1))
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды закреплений
// ──────────────────────────────────────────────────────────────────────────────

#[tauri::command]
fn pins_load(state: State<SettingsState>) -> Result<pins::PinsState, String> {
    let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(pins::load(&settings.1))
}

#[tauri::command]
fn pins_save(state: State<SettingsState>, pins_state: pins::PinsState) -> Result<(), String> {
    let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    pins::save(&settings.1, &pins_state)
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды переопределения метаданных трека
// ──────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverInput {
    /// Путь к выбранному файлу-изображению на диске пользователя
    source_path: Option<String>,
    /// Сбросить обложку и accent — вернуть оригинал из тегов файла
    reset: Option<bool>,
}

fn read_cover_as_data_url(source_path: &str) -> Result<(String, Option<String>), String> {
    let bytes =
        std::fs::read(source_path).map_err(|e| format!("Не удалось прочитать обложку: {}", e))?;
    let mime = match std::path::Path::new(source_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        _ => "image/jpeg",
    };
    let data_url = format!("data:{};base64,{}", mime, STANDARD.encode(&bytes));
    let accent = audio_player::extract_accent_color(&bytes);
    Ok((data_url, accent))
}

/// Сохранить пользовательское переопределение трека (название, исполнитель, обложка).
/// Параметры:
///  - title/artist: пустая строка означает «вернуть оригинал»
///  - cover.source_path: путь к новому изображению. Если задан — обложка обновится.
///  - cover.reset = true: сбросить обложку (вернуть оригинал из тегов файла).
///  - если cover не задан — текущая обложка не меняется.
#[tauri::command]
fn track_set_override(
    library: State<LibraryState>,
    file_path: String,
    title: Option<String>,
    artist: Option<String>,
    cover: Option<CoverInput>,
) -> Result<TrackMetadata, String> {
    let lib = library.0.lock().map_err(|e| format!("Lock error: {}", e))?;

    let (cover_data, accent_color, keep_existing_cover) = match cover {
        Some(c) if c.reset.unwrap_or(false) => (None, None, false),
        Some(c) if c.source_path.as_deref().is_some() => {
            let (url, accent) = read_cover_as_data_url(c.source_path.as_deref().unwrap())?;
            (Some(url), accent, false)
        }
        _ => (None, None, true),
    };

    lib.set_track_override(
        &file_path,
        title.as_deref(),
        artist.as_deref(),
        cover_data.as_deref(),
        accent_color.as_deref(),
        keep_existing_cover,
    )?;
    drop(lib);

    // Возвращаем актуальные метаданные трека (с применёнными переопределениями)
    fetch_track_with_override(&library, &file_path)
}

/// Прочитать локальный файл изображения и вернуть data URL
/// (нужно фронту, чтобы показать превью выбранной обложки до сохранения).
#[tauri::command]
fn read_image_as_data_url(source_path: String) -> Result<String, String> {
    let (data_url, _) = read_cover_as_data_url(&source_path)?;
    Ok(data_url)
}

/// Полностью сбросить ВСЕ пользовательские переопределения треков сразу.
/// Используется кнопкой «Сбросить редактирование» в настройках.
#[tauri::command]
fn track_clear_all_overrides(library: State<LibraryState>) -> Result<usize, String> {
    let lib = library.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    lib.clear_all_track_overrides()
}

/// Полностью сбросить пользовательское переопределение трека
#[tauri::command]
fn track_clear_override(
    library: State<LibraryState>,
    file_path: String,
) -> Result<TrackMetadata, String> {
    {
        let lib = library.0.lock().map_err(|e| format!("Lock error: {}", e))?;
        lib.clear_track_override(&file_path)?;
    }
    fetch_track_with_override(&library, &file_path)
}

fn fetch_track_with_override(
    library: &State<LibraryState>,
    file_path: &str,
) -> Result<TrackMetadata, String> {
    // Сначала пытаемся отдать трек из библиотеки (он содержит COALESCE-применённые
    // переопределения). Если трека нет в библиотеке (редко — например, играем файл
    // вне сохранённых папок), фолбэчимся на чтение тегов с диска.
    let from_lib = {
        let lib = library.0.lock().map_err(|e| format!("Lock error: {}", e))?;
        lib.get_track(file_path)
    };
    if let Some(mut t) = from_lib {
        // Маппер вернул URL только для override-обложки; для встроенной — None,
        // подставляем её миниатюру-URL.
        if t.cover_art.is_none() && AudioPlayer::has_embedded_cover(file_path) {
            t.cover_art = Some(thumbnails::cover_protocol_url(
                file_path,
                &thumbnails::embedded_sig(file_path),
            ));
        }
        return Ok(t);
    }

    let mut metadata = AudioPlayer::get_metadata(file_path)?;
    let lib = library.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    if let Some(sig) = lib.get_override_cover_sig(file_path) {
        metadata.cover_art = Some(thumbnails::cover_protocol_url(file_path, &sig));
    }
    if let Some(accent) = lib.get_accent_color(file_path) {
        metadata.accent_color = Some(accent);
    }
    Ok(metadata)
}

// ──────────────────────────────────────────────────────────────────────────────
// Команды кастомных обложек
// ──────────────────────────────────────────────────────────────────────────────

#[tauri::command]
fn cover_set_custom(
    state: State<SettingsState>,
    kind: String,
    key: String,
    source_path: String,
) -> Result<String, String> {
    let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    covers::set_custom_cover(&settings.1, &kind, &key, &source_path)
}

#[tauri::command]
fn cover_remove_custom(
    state: State<SettingsState>,
    kind: String,
    key: String,
) -> Result<(), String> {
    let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    covers::remove_custom_cover(&settings.1, &kind, &key)
}

#[tauri::command]
fn cover_load_all(state: State<SettingsState>) -> Result<Vec<covers::CustomCoverEntry>, String> {
    let settings = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(covers::load_all(&settings.1))
}

// ──────────────────────────────────────────────────────────────────────────────
// Точка входа
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn set_windows_priority_class() {
    unsafe {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn SetPriorityClass(h_process: *mut std::ffi::c_void, dw_priority_class: u32) -> i32;
        }
        // ABOVE_NORMAL_PRIORITY_CLASS = 0x00008000
        let _ = SetPriorityClass(GetCurrentProcess(), 0x00008000);
    }
}

#[tauri::command]
fn taskbar_set_playing(app: tauri::AppHandle, playing: bool) {
    taskbar::set_playing(&app, playing);
}

#[tauri::command]
fn taskbar_set_modes(app: tauri::AppHandle, shuffled: bool, repeat_track: bool) {
    taskbar::set_modes(&app, shuffled, repeat_track);
}

#[tauri::command]
fn taskbar_set_covers(
    app: tauri::AppHandle,
    prev: Option<String>,
    current: Option<String>,
    next: Option<String>,
    transition: String,
) {
    taskbar::set_covers(&app, prev, current, next, transition);
}

// ── Системное медиа-управление (MPRIS, Linux) ────────────────────────────────

#[tauri::command]
fn mpris_update_metadata(
    app: tauri::AppHandle,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    file_path: Option<String>,
    duration_secs: f64,
) {
    mpris::update_metadata(&app, title, artist, album, file_path, duration_secs);
}

#[tauri::command]
fn mpris_update_playback(state: String, position_secs: f64) {
    mpris::update_playback(&state, position_secs);
}

// ── Discord Rich Presence ─────────────────────────────────────────────────────

// async — IPC с клиентом Discord (пайп/сокет) не должен блокировать main thread.
#[tauri::command(async)]
fn discord_update_presence(payload: discord_rpc::PresencePayload) -> bool {
    discord_rpc::update_presence(payload)
}

#[tauri::command(async)]
fn discord_clear_presence() {
    discord_rpc::clear_presence();
}

// ──────────────────────────────────────────────────────────────────────────────
// Скачивание треков (страница «Скачивание»)
// ──────────────────────────────────────────────────────────────────────────────

/// Поиск треков во внешних источниках (кандидаты на скачивание), постранично.
/// Помечает `in_library` для треков, уже присутствующих в локальной библиотеке.
#[tauri::command]
async fn downloads_search(
    app: tauri::AppHandle,
    query: String,
    // Точные исполнитель/название (клик по «фантому» в альбоме) — включают точный
    // поиск по полям Deezer; для обычного поиска из строки отсутствуют.
    artist: Option<String>,
    title: Option<String>,
    index: Option<u32>,
    limit: Option<u32>,
) -> Result<external::ExternalSearchPage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut page = external::search_tracks(
            &query,
            artist.as_deref(),
            title.as_deref(),
            index.unwrap_or(0),
            limit.unwrap_or(30),
        );
        mark_tracks_in_library(&app, &mut page.tracks);
        page
    })
    .await
    .map_err(|e| format!("Task error: {}", e))
}

/// Проставляет `in_library` внешним трекам, у которых в локальной библиотеке есть
/// совпадающая версия: название совпало (без шума), есть хотя бы один общий артист
/// (поддержка мультиартистных треков) И альбом совместим. Альбом совместим, когда:
/// у внешнего трека альбома нет, ИЛИ у локальной версии тег альбома пуст (не с чем
/// сверять — не блокируем), ИЛИ ключи альбомов совпадают. Так одноимённый трек
/// того же исполнителя из ДРУГОГО альбома не помечается «скачанным» — только
/// реально скачанная версия. Общий помощник для поиска/альбома/исполнителя.
fn mark_tracks_in_library(app: &tauri::AppHandle, tracks: &mut [external::ExternalSearchTrack]) {
    if tracks.is_empty() {
        return;
    }
    // Индекс локального каталога — краткие блокировки настроек/библиотеки.
    let separators = settings::get_artist_separators(&app.state::<SettingsState>());
    let lib_index = app
        .state::<LibraryState>()
        .0
        .lock()
        .map(|lib| lib.catalog_artist_title_index(&separators))
        .unwrap_or_default();

    // Набор разделителей для нечёткого сопоставления готовим ОДИН раз до цикла:
    // иначе на каждый трек заново аллоцировался и сортировался их список
    // (см. external::prepare_artist_separators).
    let prepared_seps = external::prepare_artist_separators(&separators);

    for t in tracks.iter_mut() {
        let tkey = external::title_match_key(&t.title);
        let Some(local_versions) = lib_index.get(&tkey) else {
            continue;
        };
        let track_artists = external::split_artists_for_match_prepared(&t.artist, &prepared_seps);
        let ext_album_key = t
            .album
            .as_deref()
            .map(external::album_match_key)
            .unwrap_or_default();
        let matched = local_versions.iter().any(|v| {
            let artist_match = track_artists.iter().any(|a| v.artists.contains(a));
            let album_match =
                ext_album_key.is_empty() || v.album_key.is_empty() || v.album_key == ext_album_key;
            artist_match && album_match
        });
        if matched {
            t.in_library = true;
        }
    }
}

/// Детальная карточка альбома (Deezer) для страницы скачивания альбома.
/// Треки помечаются `in_library`, чтобы кнопки сразу отражали наличие в каталоге.
#[tauri::command]
async fn downloads_album(
    app: tauri::AppHandle,
    album_id: u64,
) -> Result<Option<external::ExternalAlbumDetail>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut detail = external::album_detail(album_id)?;
        if let Some(d) = detail.as_mut() {
            mark_tracks_in_library(&app, &mut d.tracks);
        }
        Ok(detail)
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

/// Детальная карточка исполнителя (Deezer): его альбомы и популярные треки.
/// Популярные треки помечаются `in_library`.
#[tauri::command]
async fn downloads_artist(
    app: tauri::AppHandle,
    artist_id: u64,
) -> Result<Option<external::ExternalArtistDetail>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut detail = external::artist_detail(artist_id)?;
        if let Some(d) = detail.as_mut() {
            mark_tracks_in_library(&app, &mut d.top_tracks);
        }
        Ok(detail)
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

/// Поставить трек в очередь скачивания. Возвращает id элемента очереди.
#[tauri::command]
fn downloads_enqueue(
    app: tauri::AppHandle,
    request: downloads::DownloadRequest,
) -> Result<u64, String> {
    Ok(downloads::enqueue(&app, request))
}

/// Поставить пачку треков в очередь (кнопка «Скачать все треки» у альбома).
/// Возвращает id элементов очереди в порядке запросов.
#[tauri::command]
fn downloads_enqueue_many(
    app: tauri::AppHandle,
    requests: Vec<downloads::DownloadRequest>,
) -> Result<Vec<u64>, String> {
    Ok(downloads::enqueue_many(&app, requests))
}

/// Текущий снимок очереди + статус утилит (для первичной загрузки страницы).
#[tauri::command]
fn downloads_snapshot(app: tauri::AppHandle) -> downloads::DownloadSnapshot {
    downloads::snapshot(&app)
}

/// Убрать (или отменить активный) элемент очереди.
#[tauri::command(async)]
fn downloads_remove(app: tauri::AppHandle, id: u64) {
    downloads::remove(&app, id);
}

/// Повторить скачивание ошибочного элемента очереди.
#[tauri::command(async)]
fn downloads_retry(app: tauri::AppHandle, id: u64) {
    downloads::retry(&app, id);
}

/// Очистить завершённые/ошибочные элементы.
#[tauri::command]
fn downloads_clear_finished(app: tauri::AppHandle) {
    downloads::clear_finished(&app);
}

/// Путь папки скачиваний (фронт по нему определяет, скачан ли трек приложением).
#[tauri::command]
fn downloads_dir_path(app: tauri::AppHandle) -> String {
    downloads::downloads_dir_path(&app)
}

/// Сменить папку для скачиваемых треков. `folder_path` = None/пусто — сброс на
/// стандартную папку «LFMusic Downloads» в системной музыкальной директории.
/// При смене старая папка скачиваний убирается из сканируемых, а новая
/// добавляется. Если новая папка вложена в уже сканируемую — её не добавляем
/// отдельно (иначе двойное сканирование; треки и так попадут в накрывающую
/// папку). При любой ошибке подготовки папки — fallback на стандартную.
/// Возвращает фактически применённый путь.
#[tauri::command]
async fn downloads_set_dir(
    app: tauri::AppHandle,
    folder_path: Option<String>,
) -> Result<String, String> {
    // Сканирование новой папки может быть долгим (если выбрали существующую папку
    // с музыкой) — уводим всю работу в blocking-пул, как library_add_folder.
    tauri::async_runtime::spawn_blocking(move || {
        let (old_dir, new_dir) = downloads::apply_downloads_dir(&app, folder_path);

        // Папка не изменилась (тот же путь или fallback вернул прежнюю) — библиотеку
        // не трогаем.
        if !downloads::same_path(&old_dir, &new_dir) {
            let separators = settings::get_artist_separators(&app.state::<SettingsState>());
            let old = old_dir.to_string_lossy().to_string();
            let new = new_dir.to_string_lossy().to_string();

            // Перерегистрацию делаем под одним локом библиотеки (проверка
            // вложенности + мутации атомарны).
            let state = app.state::<LibraryState>();
            let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;

            // 1) Убрать старую папку из сканируемых — ТОЛЬКО если она была отдельной
            //    записью в БД. Если старая папка была вложена в пользовательскую
            //    (covering_folder вернул другой путь) — своей записи у неё нет,
            //    удалять нечего, иначе снесли бы папку пользователя вместе с треками.
            if library.covering_folder(&old).as_deref() == Some(old.as_str()) {
                library.remove_folder(
                    old,
                    &separators,
                    Some(audio_feature_progress_emitter(app.clone())),
                )?;
            }

            // 2) Добавить новую папку — если она НЕ вложена в уже сканируемую (иначе
            //    двойное сканирование; вложенные треки попадут в накрывающую папку).
            if library.covering_folder(&new).is_none() {
                library.add_folder(
                    new,
                    &separators,
                    Some(audio_feature_progress_emitter(app.clone())),
                    scan_progress_emitter(app.clone()),
                )?;
            }
            drop(library);

            // Набор папок/каталог изменился — фронт перезагрузит библиотеку и список папок.
            let _ = app.emit("downloads_library_updated", ());
        }

        Ok(new_dir.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

/// Открыть папку в системном файловом менеджере (клик по иконке папки в списке
/// «Локальные папки» настроек).
///
/// На Windows открываем через shell-ассоциацию (`ShellExecuteW` с verb `"open"`),
/// а НЕ прямым вызовом `explorer.exe`: иначе игнорировался бы зарегистрированный
/// пользователем обработчик открытия папок (сторонний файловый менеджер). macOS
/// `open` и Linux `xdg-open` и так уважают дефолтное приложение.
#[tauri::command]
fn open_folder_in_explorer(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).is_dir() {
        return Err("Папка недоступна".to_string());
    }
    #[cfg(windows)]
    {
        use windows::core::{w, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        // Широкая строка с нуль-терминатором; держим её живой на время вызова.
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let hinst = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecute возвращает HINSTANCE > 32 при успехе (легаси-семантика).
        if hinst.0 as isize <= 32 {
            return Err("Не удалось открыть папку".to_string());
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "macos")]
        let mut cmd = std::process::Command::new("open");
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(&path);
        cmd.spawn()
            .map_err(|e| format!("Не удалось открыть папку: {}", e))?;
        return Ok(());
    }
}

/// Удалить скачанный приложением трек (из библиотеки и файл с диска).
#[tauri::command(async)]
fn downloads_delete_track(app: tauri::AppHandle, file_path: String) -> Result<(), String> {
    downloads::delete_downloaded_track(&app, &file_path)
}

/// Проверить доступность yt-dlp/ffmpeg/deno (без скачивания).
#[tauri::command]
async fn downloads_check_tools(app: tauri::AppHandle) -> Result<downloads::ToolsStatus, String> {
    tauri::async_runtime::spawn_blocking(move || downloads::refresh_tools_status(&app))
        .await
        .map_err(|e| format!("Task error: {}", e))
}

/// Открыть окно входа в YouTube. После авторизации cookies сохраняются и
/// автоматически проверяются (статус приходит снапшотом `download_queue`).
#[tauri::command]
fn downloads_youtube_login(app: tauri::AppHandle) {
    downloads::open_login(&app);
}

/// Запустить проверку валидности сохранённых cookies YouTube (фоновый сетевой
/// тест). Результат приходит снапшотом `download_queue`.
#[tauri::command]
fn downloads_validate_cookies(app: tauri::AppHandle) {
    downloads::validate_youtube_cookies(&app);
}

/// Выйти из аккаунта YouTube (удалить сохранённые cookies).
///
/// `(async)` — ОБЯЗАТЕЛЬНО: logout чистит webview cookie store
/// (`cookies_for_url`/`delete_cookie`), а эти вызовы WebView2 синхронно ждут, пока
/// главный поток прокрутит цикл сообщений. На обычной (синхронной) команде тело
/// исполняется НА главном потоке — он блокируется сам на себе (дедлок): зависает
/// шапка/окно, приложение перестаёт закрываться. `(async)` уносит команду в
/// отдельный поток, главный поток свободен и обслуживает запрос cookies (тот же
/// приём, что у `login_watch_loop`, который читает cookies из фонового потока).
#[tauri::command(async)]
fn downloads_youtube_logout(app: tauri::AppHandle) {
    downloads::logout(&app);
}

/// Текущая конфигурация прокси для yt-dlp (режим + URL).
#[tauri::command]
fn downloads_get_proxy(app: tauri::AppHandle) -> downloads::ProxyConfig {
    downloads::get_proxy(&app)
}

/// Сохранить конфигурацию прокси для yt-dlp. `mode`: "system" | "none" | "custom";
/// `url` нужен только для "custom" (напр. socks5://127.0.0.1:1080 для ByeDPI).
#[tauri::command]
fn downloads_set_proxy(app: tauri::AppHandle, mode: String, url: String) {
    downloads::set_proxy(&app, mode, url);
}

// ──────────────────────────────────────────────────────────────────────────────
// Управление утилитами (yt-dlp/ffmpeg) — страница настроек «Внешние источники»
// ──────────────────────────────────────────────────────────────────────────────

fn tools_app_data_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Прогресс установки/обновления утилиты (событие `tools_progress`).
#[derive(Clone, serde::Serialize)]
struct ToolsProgress {
    /// "ytdlp" | "ffmpeg".
    tool: String,
    message: Option<String>,
    active: bool,
}

/// Локальная информация об утилитах (версии/managed) — без обращения к сети.
#[tauri::command]
async fn tools_info(app: tauri::AppHandle) -> Result<tools::ToolsInfo, String> {
    tauri::async_runtime::spawn_blocking(move || tools::info(&tools_app_data_dir(&app)))
        .await
        .map_err(|e| format!("Task error: {}", e))
}

/// Проверка обновлений утилит (обращается к GitHub). Возвращает ту же структуру
/// с заполненным `update_available`.
#[tauri::command]
async fn tools_check_updates(app: tauri::AppHandle) -> Result<tools::ToolsInfo, String> {
    tauri::async_runtime::spawn_blocking(move || tools::check_updates(&tools_app_data_dir(&app)))
        .await
        .map_err(|e| format!("Task error: {}", e))
}

/// (Пере)установить/обновить утилиту (`tool` = "ytdlp" | "ffmpeg"). Прогресс
/// стримится событием `tools_progress`; по завершении эмитится `tools_changed`.
#[tauri::command]
async fn tools_install(app: tauri::AppHandle, tool: String) -> Result<tools::ToolsInfo, String> {
    let app_for_task = app.clone();
    let info = tauri::async_runtime::spawn_blocking(move || {
        let dir = tools_app_data_dir(&app_for_task);
        let app_for_report = app_for_task.clone();
        let tool_for_report = tool.clone();
        let report = move |msg: String| {
            let _ = app_for_report.emit(
                "tools_progress",
                ToolsProgress {
                    tool: tool_for_report.clone(),
                    message: Some(msg),
                    active: true,
                },
            );
        };
        let result = match tool.as_str() {
            // Deno (JS-движок для yt-dlp) ставится вместе с yt-dlp. Best-effort:
            // при ошибке Deno не откатываем yt-dlp (он работает и без Deno, но в
            // деградированном режиме) — только сообщаем в прогрессе.
            "ytdlp" => {
                let r = tools::install_ytdlp(&dir, &report);
                if r.is_ok() {
                    if let Err(e) = tools::install_deno(&dir, &report) {
                        report(format!("yt-dlp установлен; Deno не установлен: {}", e));
                    }
                }
                r
            }
            "ffmpeg" => tools::install_ffmpeg(&dir, &report),
            _ => Err("Неизвестная утилита".to_string()),
        };
        // Снимаем индикатор прогресса независимо от исхода.
        let _ = app_for_task.emit(
            "tools_progress",
            ToolsProgress {
                tool: tool.clone(),
                message: None,
                active: false,
            },
        );
        result?;
        Ok::<_, String>(tools::info(&dir))
    })
    .await;
    // Статус обновляем ВСЕГДА — даже если установка вернула ошибку ИЛИ задача
    // упала (паника): иначе UI остался бы устаревшим. Эмитим ДО разбора исхода,
    // чтобы ранний выход по ошибке join не пропустил событие (refreshToolsInfo).
    let _ = app.emit("tools_changed", ());
    info.map_err(|e| format!("Task error: {}", e))?
}

/// Доустановить Deno (JS-движок для yt-dlp), если он отсутствует. Нужно для
/// пользователей, поставивших yt-dlp до появления связки с Deno. Прогресс — под
/// ключом "ytdlp" (Deno показывается в карточке yt-dlp).
#[tauri::command]
async fn tools_install_deno(app: tauri::AppHandle) -> Result<tools::ToolsInfo, String> {
    let app_for_task = app.clone();
    let info = tauri::async_runtime::spawn_blocking(move || {
        let dir = tools_app_data_dir(&app_for_task);
        let app_for_report = app_for_task.clone();
        let report = move |msg: String| {
            let _ = app_for_report.emit(
                "tools_progress",
                ToolsProgress {
                    tool: "ytdlp".to_string(),
                    message: Some(msg),
                    active: true,
                },
            );
        };
        let result = tools::install_deno(&dir, &report);
        let _ = app_for_task.emit(
            "tools_progress",
            ToolsProgress {
                tool: "ytdlp".to_string(),
                message: None,
                active: false,
            },
        );
        result?;
        Ok::<_, String>(tools::info(&dir))
    })
    .await;
    // Статус обновляем ВСЕГДА (даже при ошибке/панике задачи), чтобы UI не остался
    // устаревшим. Эмитим ДО разбора исхода join.
    let _ = app.emit("tools_changed", ());
    info.map_err(|e| format!("Task error: {}", e))?
}

/// Установить сразу все утилиты одним действием: yt-dlp (+ Deno) и ffmpeg.
/// Прогресс по каждой стримится событием `tools_progress` под её ключом. yt-dlp
/// обязателен — его сбой возвращается ошибкой; ffmpeg опционален (его сбой не
/// фатален: качество/ремукс ограничены, но скачивание работает).
#[tauri::command]
async fn tools_install_all(app: tauri::AppHandle) -> Result<tools::ToolsInfo, String> {
    let app_for_task = app.clone();
    let info = tauri::async_runtime::spawn_blocking(move || {
        let dir = tools_app_data_dir(&app_for_task);

        let make_report = |tool: &'static str| {
            let app_for_report = app_for_task.clone();
            move |msg: String| {
                let _ = app_for_report.emit(
                    "tools_progress",
                    ToolsProgress {
                        tool: tool.to_string(),
                        message: Some(msg),
                        active: true,
                    },
                );
            }
        };
        let clear_progress = |tool: &'static str| {
            let _ = app_for_task.emit(
                "tools_progress",
                ToolsProgress {
                    tool: tool.to_string(),
                    message: None,
                    active: false,
                },
            );
        };

        // Ставим только ОТСУТСТВУЮЩЕЕ: «Установить необходимое» не должно
        // перекачивать уже установленные бинари (обновление — отдельная кнопка).
        // yt-dlp обязателен; Deno докидываем рядом (install_deno сам идемпотентен —
        // вернёт Ok, если Deno уже есть).
        let report_ytdlp = make_report("ytdlp");
        let ytdlp_result: Result<(), String> = if tools::resolve_ytdlp(&dir).is_some() {
            Ok(())
        } else {
            tools::install_ytdlp(&dir, &report_ytdlp).map(|_| ())
        };
        if ytdlp_result.is_ok() {
            if let Err(e) = tools::install_deno(&dir, &report_ytdlp) {
                report_ytdlp(format!("Deno не установлен: {}", e));
            }
        }
        clear_progress("ytdlp");

        // ffmpeg опционален — ставим, только если его ещё нет; сбой не валит всё.
        let report_ffmpeg = make_report("ffmpeg");
        if tools::resolve_ffmpeg(&dir).is_none() {
            if let Err(e) = tools::install_ffmpeg(&dir, &report_ffmpeg) {
                report_ffmpeg(format!("ffmpeg не установлен: {}", e));
            }
        }
        clear_progress("ffmpeg");

        ytdlp_result?;
        Ok::<_, String>(tools::info(&dir))
    })
    .await;
    // Статус обновляем ВСЕГДА: при частичном успехе (ffmpeg встал, а yt-dlp нет)
    // ИЛИ панике задачи событие триггерит refreshToolsInfo, и UI отразит
    // фактическое состояние. Эмитим ДО разбора исхода join.
    let _ = app.emit("tools_changed", ());
    info.map_err(|e| format!("Task error: {}", e))?
}

/// Удалить скачанную приложением утилиту (`tool` = "ytdlp" | "ffmpeg").
#[tauri::command]
async fn tools_delete(app: tauri::AppHandle, tool: String) -> Result<tools::ToolsInfo, String> {
    // Утилитой может пользоваться активный воркер скачивания (yt-dlp/ffmpeg уже
    // резолвнуты и качают очередь). Удаление выдернуло бы бинарь из-под него —
    // не даём, пока в очереди есть незавершённые элементы.
    if downloads::has_active_jobs(&app) {
        return Err(
            "Нельзя удалить, пока есть треки в очереди скачивания. Дождитесь завершения или очистите очередь."
                .to_string(),
        );
    }
    let app_for_task = app.clone();
    let info = tauri::async_runtime::spawn_blocking(move || {
        let dir = tools_app_data_dir(&app_for_task);
        match tool.as_str() {
            // Deno удаляется вместе с yt-dlp (ставились вместе).
            "ytdlp" => {
                tools::delete_ytdlp(&dir)?;
                tools::delete_deno(&dir)?;
            }
            "ffmpeg" => tools::delete_ffmpeg(&dir)?,
            _ => return Err("Неизвестная утилита".to_string()),
        }
        Ok::<_, String>(tools::info(&dir))
    })
    .await
    .map_err(|e| format!("Task error: {}", e))??;
    let _ = app.emit("tools_changed", ());
    Ok(info)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "windows")]
    set_windows_priority_class();

    let player = AudioPlayer::new().expect("Не удалось инициализировать аудиоплеер");

    tauri::Builder::default()
        // Кастомный протокол обложек: отдаёт уменьшенные WebP-миниатюры по
        // file_path (генерируются лениво и кэшируются на диск), без base64 в IPC.
        .register_asynchronous_uri_scheme_protocol("lfmusic-cover", |ctx, request, responder| {
            thumbnails::protocol_handler(ctx, request, responder);
        })
        // Кастомный протокол внешних картинок (аватарки/обложки): отдаёт
        // кэшированные WebP по lfmusic-external://{artist|cover}/<hash>, без
        // base64 в IPC — по образцу lfmusic-cover.
        .register_asynchronous_uri_scheme_protocol("lfmusic-external", |ctx, request, responder| {
            external::protocol_handler(ctx, request, responder);
        })
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Не удалось получить директорию данных приложения");

            window_state::restore_main_window_geometry(app, &app_data_dir);

            // Интеграция с панелью задач Windows (кнопки + обложка-превью).
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                if let Ok(hwnd) = window.hwnd() {
                    taskbar::init(&app.handle().clone(), hwnd.0 as isize);
                }
            }

            // Системное медиа-управление Linux (MPRIS): отображение трека и кнопки.
            #[cfg(target_os = "linux")]
            {
                let cover_dir = app
                    .path()
                    .app_cache_dir()
                    .unwrap_or_else(|_| app_data_dir.clone());
                let _ = std::fs::create_dir_all(&cover_dir);
                mpris::init(&app.handle().clone(), cover_dir);
            }

            let app_settings = settings::load(&app_data_dir);

            // Загружаем сохранённые настройки эквалайзера из settings.json
            let player_state = app.state::<PlayerState>();
            let player = player_state.0.read().expect("Lock error");
            player.set_eq_settings(app_settings.eq_settings.clone());
            drop(player);

            app.manage(SettingsState(Mutex::new((
                app_settings,
                app_data_dir.clone(),
            ))));

            let external_marker_dir = app_data_dir.clone();
            let mut library =
                MusicLibrary::new(app_data_dir).expect("Не удалось инициализировать библиотеку");

            // Один раз при запуске пересобираем плейлисты исполнителей,
            // чтобы существующие библиотеки получили записи без ожидания рескана.
            let separators = {
                let s = app.state::<SettingsState>();
                settings::get_artist_separators(&s)
            };
            if let Err(e) = library.rebuild_artist_playlists(&separators) {
                eprintln!("Ошибка инициализации плейлистов исполнителей: {}", e);
            }

            // Одноразовая миграция: сбрасываем негативный кэш внешних альбомов —
            // старые «нет» могли быть закэшированы из-за временных ошибок (теперь
            // такие не кэшируются). Гейт — файл-маркер версии.
            {
                let marker = external_marker_dir.join("external_album.version");
                let current = std::fs::read_to_string(&marker)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok())
                    .unwrap_or(0);
                const EXTERNAL_ALBUM_CACHE_VERSION: u32 = 1;
                if current < EXTERNAL_ALBUM_CACHE_VERSION {
                    let _ = library.clear_external_album_negative_cache();
                    let _ = std::fs::write(&marker, EXTERNAL_ALBUM_CACHE_VERSION.to_string());
                }
            }

            app.manage(LibraryState(Mutex::new(library)));

            Ok(())
        })
        .manage(PlayerState(RwLock::new(player)))
        .manage(downloads::DownloadsState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            // Плеер
            player_play,
            player_play_crossfade,
            player_play_paused,
            player_pause,
            player_resume,
            player_stop,
            player_set_volume,
            player_get_volume,
            player_seek,
            player_seek_crossfade,
            player_get_status,
            player_get_spectrum,
            player_set_spectrum_enabled,
            get_track_metadata,
            get_cover_art,
            get_original_cover_art,
            get_supported_formats,
            // Библиотека
            library_get_folders,
            library_add_folder,
            library_add_android_music_folders,
            library_remove_folder,
            library_get_tracks,
            library_get_catalog_paths,
            library_get_tracks_by_paths,
            library_list_genres,
            library_search,
            library_resume_audio_features,
            library_audio_features_status,
            library_move_track_to_top,
            library_cleanup,
            library_rescan,
            library_rebuild_artists,
            // Плейлисты
            playlist_get_all,
            playlist_get_tracks,
            playlist_create,
            playlist_rename,
            playlist_delete,
            playlist_add_track,
            playlist_remove_track,
            playlist_move_track_to_top,
            playlist_get_ids_for_track,
            album_get_track_order,
            album_set_track_order,
            // Внешние источники
            external_album_layout,
            external_artist_image,
            external_track_cover,
            external_load_cached_artist_images,
            external_prepare,
            // Скачивание треков
            downloads_search,
            downloads_album,
            downloads_artist,
            downloads_enqueue,
            downloads_enqueue_many,
            downloads_snapshot,
            downloads_remove,
            downloads_retry,
            downloads_clear_finished,
            downloads_dir_path,
            downloads_set_dir,
            downloads_delete_track,
            downloads_check_tools,
            downloads_youtube_login,
            downloads_validate_cookies,
            downloads_youtube_logout,
            downloads_get_proxy,
            downloads_set_proxy,
            tools_info,
            tools_check_updates,
            tools_install,
            tools_install_deno,
            tools_install_all,
            tools_delete,
            statistics_add_track_delta,
            statistics_get_track,
            statistics_sum_listen_count,
            statistics_add_collection_full_listen,
            statistics_get_recently_played,
            statistics_get_most_played,
            statistics_get_top_artists,
            statistics_get_top_albums,
            statistics_get_summary_stats,
            statistics_get_recommendations,
            statistics_get_taste_profile,
            // Эквалайзер
            eq_set_bands,
            eq_get_bands,
            eq_set_gain,
            eq_get_gain,
            eq_set_enabled,
            eq_get_enabled,
            eq_get_settings,
            eq_set_settings,
            // Настройки
            get_settings,
            save_settings,
            // Состояние плеера
            save_player_state,
            load_player_state,
            // Закрепления
            pins_load,
            pins_save,
            // Кастомные обложки
            cover_set_custom,
            cover_remove_custom,
            cover_load_all,
            // Переопределения метаданных трека
            track_set_override,
            track_clear_override,
            track_clear_all_overrides,
            get_original_track,
            read_image_as_data_url,
            // Окно
            window_state::toggle_window_fullscreen,
            window_state::toggle_window_maximize,
            open_folder_in_explorer,
            // Панель задач (Windows)
            taskbar_set_playing,
            taskbar_set_modes,
            taskbar_set_covers,
            // Системное медиа-управление (MPRIS, Linux)
            mpris_update_metadata,
            mpris_update_playback,
            // Discord Rich Presence
            discord_update_presence,
            discord_clear_presence,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                // Сначала добиваем активный yt-dlp — иначе при закрытии во время
                // скачивания он остался бы процессом-зомби в фоне ОС.
                downloads::cleanup_on_exit(app_handle);
                if let Some(state) = app_handle.try_state::<LibraryState>() {
                    if let Ok(library) = state.0.lock() {
                        if let Err(e) = library.shutdown() {
                            eprintln!("Library shutdown error: {}", e);
                        }
                    }
                }
            }
        });
}
