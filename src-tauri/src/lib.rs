mod audio_features;
mod audio_player;
mod browser_bridge;
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

use audio_player::{AudioPlayer, EqSettings, PlayerStatus, SinkEndSignal, TrackMetadata};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use library::{AudioFeatureProgressEmitter, FolderInfo, GenreCount, MusicLibrary, PlaylistInfo};
use player_state::SavedPlayerState;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
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
    /// Current phase: "analysis" | "external" | "download" | "done" — frontend picks progress text by this.
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

/// Combined "prep" progress bar (TitleBar.vue): audio-feature analysis (`f_*`),
/// external-source fetch (`e_*`), and downloads (`d_*`) phases in one bar.
#[derive(Clone, Copy)]
struct PrepCounts {
    f_done: usize,
    f_total: usize,
    e_done: usize,
    e_total: usize,
    d_done: usize,
    d_total: usize,
    /// Fractional progress (0..1) of the track currently downloading; only affects the
    /// displayed percent (not the done/total count), so the bar moves smoothly instead of jumping per-track.
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

/// Apply a mutation to the counters and (throttled) emit combined progress.
fn report_prep_progress(app: &tauri::AppHandle, mutate: impl FnOnce(&mut PrepCounts)) {
    let (current, total, percent, force_done, phase) = {
        let mut p = PREP_COUNTS.lock().unwrap_or_else(|e| e.into_inner());
        mutate(&mut p);

        let current = p.f_done + p.e_done + p.d_done;
        let total = p.f_total + p.e_total + p.d_total;
        let done = total == 0 || current >= total;
        // Phase priority: external > analysis > download (download shows last,
        // only once analysis and external sources are done).
        let phase = if p.e_total > 0 && p.e_done < p.e_total {
            "external"
        } else if p.f_total > 0 && p.f_done < p.f_total {
            "analysis"
        } else if p.d_total > 0 && p.d_done < p.d_total {
            "download"
        } else {
            "done"
        };
        // Add the current track's fractional progress so the bar doesn't jump 0 -> done per track.
        let percent = if total > 0 {
            ((current as f64 + p.d_frac.clamp(0.0, 1.0)) / total as f64 * 100.0).clamp(0.0, 100.0)
        } else {
            100.0
        };

        // Each phase clears only its OWN counters once it individually reaches
        // its own 100%, instead of a combined reset gated on the summed total.
        // The old combined reset only fired once every *concurrently active*
        // phase happened to be done at once — a phase that finished earlier
        // (e.g. external prep) while another was still running (e.g. analysis)
        // stayed at "100/100" indefinitely, sitting in the sum. With analysis/
        // downloads now finishing faster, phases interleave and hand off to each
        // other far more often, so a finished-but-not-yet-cleared phase's stale
        // total kept getting added back into the combined bar on every update
        // from an unrelated phase — the "jumps between unrelated numbers" bug.
        if p.f_total > 0 && p.f_done >= p.f_total {
            p.f_total = 0;
            p.f_done = 0;
        }
        if p.e_total > 0 && p.e_done >= p.e_total {
            p.e_total = 0;
            p.e_done = 0;
        }
        if p.d_total > 0 && p.d_done >= p.d_total {
            p.d_total = 0;
            p.d_done = 0;
            p.d_frac = 0.0;
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

fn report_external_progress(app: &tauri::AppHandle, done: usize, total: usize) {
    report_prep_progress(app, |p| {
        p.e_total = total;
        p.e_done = done.min(total);
    });
}

/// Called by the download manager; shown as the last phase after analysis/external.
pub(crate) fn report_download_progress(app: &tauri::AppHandle, done: usize, total: usize) {
    report_prep_progress(app, |p| {
        p.d_total = total;
        p.d_done = done.min(total);
        // Reset stale fraction from a previous download.
        p.d_frac = 0.0;
    });
}

/// Same as above but with the current track's fractional progress (`frac` in [0,1])
/// for a smooth percent while downloading; done/total stay whole track counts.
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

// ── State types ──────────────────────────────────────────────────────────────

pub struct PlayerState(pub RwLock<AudioPlayer>);
pub struct LibraryState(pub Mutex<MusicLibrary>);
pub struct SettingsState(pub Mutex<(settings::AppSettings, PathBuf)>);

// ── Player commands ──────────────────────────────────────────────────────────

/// Apply cached accent_color (or save a newly computed one) and overlay user
/// track overrides (title/artist/cover) onto the metadata.
fn apply_accent_color(
    library: &State<LibraryState>,
    file_path: &str,
    metadata: &mut TrackMetadata,
) {
    let lib = match library.0.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };

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
        // Override cover takes priority over the embedded one.
        if let Some(sig) = lib.get_override_cover_sig(file_path) {
            metadata.cover_art = Some(thumbnails::cover_protocol_url(file_path, &sig));
        }
    }

    match lib.get_accent_color(file_path) {
        Some(cached) => {
            metadata.accent_color = Some(cached);
        }
        None => {
            if let Some(ref color) = metadata.accent_color {
                let _ = lib.set_accent_color(file_path, color);
            }
        }
    }
}

// (async): opening the file, probing tags, and extracting/decoding the embedded
// cover for accent-color computation can take tens to hundreds of ms on a slow
// disk or large image — off the main/event-loop thread so window interaction
// doesn't stutter on every track change. Tauri runs a non-`async fn` command
// marked `(async)` via spawn_blocking; the RwLock<AudioPlayer> locking is unchanged.
#[tauri::command(async)]
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

#[tauri::command(async)]
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

#[tauri::command(async)]
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

/// Spectrum bars (0..1) for the visualizer; polled by the frontend on rAF frames.
#[tauri::command]
fn player_get_spectrum(state: State<PlayerState>, bars: usize) -> Result<Vec<f32>, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.spectrum_bars(bars))
}

/// Frontend toggles this with visualizer visibility to skip unnecessary FFT work when hidden.
#[tauri::command]
fn player_set_spectrum_enabled(state: State<PlayerState>, enabled: bool) -> Result<(), String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    player.set_spectrum_enabled(enabled);
    Ok(())
}

// (async): runs on the thread pool so tag/cover reads don't block the window's event loop.
#[tauri::command(async)]
fn get_track_metadata(
    library: State<LibraryState>,
    file_path: String,
) -> Result<TrackMetadata, String> {
    let mut metadata = AudioPlayer::get_metadata(&file_path)?;
    apply_accent_color(&library, &file_path, &mut metadata);
    Ok(metadata)
}

/// Cover URL (`lfmusic-cover` protocol) or null; the thumbnail itself is generated
/// lazily by the protocol handler on display, so this is a cheap DB/tag lookup.
#[tauri::command(async)]
fn get_cover_art(
    library: State<LibraryState>,
    file_path: String,
) -> Result<Option<String>, String> {
    let lib = library.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(thumbnails::cover_url(&lib, &file_path))
}

/// Reads the cover straight from file tags, ignoring user overrides (used by the
/// edit modal's "reset cover" preview).
#[tauri::command(async)]
fn get_original_cover_art(file_path: String) -> Result<Option<String>, String> {
    Ok(AudioPlayer::extract_cover_art(&file_path))
}

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

// ── Library commands ─────────────────────────────────────────────────────────

#[tauri::command]
fn library_get_folders(state: State<LibraryState>) -> Result<Vec<FolderInfo>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_folders())
}

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

/// Ordered catalog paths filtered by genre/search; returns just `file_path` for
/// virtual scroll + playback ordering (lightweight identity list).
#[tauri::command(async)]
fn library_get_catalog_paths(
    state: State<LibraryState>,
    genre: Option<String>,
    search: Option<String>,
) -> Result<Vec<String>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_catalog_paths(genre.as_deref(), search.as_deref()))
}

/// Batch-materializes metadata for the given paths (catalog viewport / cache-miss
/// fetch); result order matches `paths`.
#[tauri::command(async)]
fn library_get_tracks_by_paths(
    state: State<LibraryState>,
    paths: Vec<String>,
) -> Result<Vec<TrackMetadata>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_tracks_by_paths(&paths))
}

#[tauri::command(async)]
fn library_list_genres(state: State<LibraryState>) -> Result<Vec<GenreCount>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.list_genres())
}

/// Ranked catalog search: exact -> fuzzy -> layout-swap -> format keyword.
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

#[tauri::command]
fn library_move_track_to_top(file_path: String, state: State<LibraryState>) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.move_track_to_top(&file_path)
}

#[tauri::command(async)]
fn library_cleanup(state: State<LibraryState>) -> Result<usize, String> {
    let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.cleanup_missing_tracks()
}

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

/// Rebuild artist playlists (call after changing artist separators).
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

// ── Playlist commands ────────────────────────────────────────────────────────

#[tauri::command]
fn playlist_get_all(state: State<LibraryState>) -> Result<Vec<PlaylistInfo>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_playlists())
}

#[tauri::command(async)]
fn playlist_get_tracks(
    state: State<LibraryState>,
    playlist_id: i64,
) -> Result<Vec<TrackMetadata>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_playlist_tracks(playlist_id))
}

#[tauri::command]
fn playlist_create(state: State<LibraryState>, name: String) -> Result<PlaylistInfo, String> {
    let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.create_playlist(name)
}

#[tauri::command]
fn playlist_rename(
    state: State<LibraryState>,
    playlist_id: i64,
    name: String,
) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.rename_playlist(playlist_id, name)
}

#[tauri::command]
fn playlist_delete(state: State<LibraryState>, playlist_id: i64) -> Result<(), String> {
    let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.delete_playlist(playlist_id)
}

#[tauri::command]
fn playlist_add_track(
    state: State<LibraryState>,
    playlist_id: i64,
    file_path: String,
) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.add_track_to_playlist(playlist_id, &file_path)
}

#[tauri::command]
fn playlist_remove_track(
    state: State<LibraryState>,
    playlist_id: i64,
    file_path: String,
) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.remove_track_from_playlist(playlist_id, &file_path)
}

#[tauri::command]
fn playlist_move_track_to_top(
    state: State<LibraryState>,
    playlist_id: i64,
    file_path: String,
) -> Result<(), String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    library.move_playlist_track_to_top(playlist_id, &file_path)
}

#[tauri::command]
fn playlist_get_ids_for_track(
    state: State<LibraryState>,
    file_path: String,
) -> Result<Vec<i64>, String> {
    let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    Ok(library.get_user_playlist_ids_for_track(&file_path))
}

// ── Album track order ────────────────────────────────────────────────────────

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

// ── External-source commands ─────────────────────────────────────────────────

/// Canonical track order + "phantom" (not-in-library) tracks for an album,
/// sourced externally and cached in the DB (repeat opens skip the network).
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

/// Sync core of [`external_album_layout`], shared with the batch prep pass.
/// Library lock is NOT held during the network request; DB cache never expires.
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

    // Phase 1 (locked): read local tracks + cache, decide whether the network is needed.
    let (local, query_artist, album_key, decision) = {
        let library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;
        let local = library.get_album_local_tracks(album_name);
        if local.is_empty() {
            return Ok(none_layout());
        }

        // Most common artist among local tracks is used for the query.
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

        // Cache key includes the artist too, so same-titled albums by different
        // artists don't collide.
        let album_key = external::album_cache_key(album_name, &query_artist);

        // 0 = cache hit; 1 = need network; 2 = negative cache (permanent).
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

    // Phase 2 (unlocked): network.
    let fetched = external::fetch_album_tracklist(&query_artist, album_name);

    // Phase 3 (locked): persist result to cache.
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
        // All sources genuinely returned nothing — cache negative to avoid retrying.
        external::FetchOutcome::Found(_) | external::FetchOutcome::NotFound => {
            let _ = library.set_external_album(&album_key, None, "none", &[], now);
            Ok(none_layout())
        }
        // Transient provider error (network/parse/rate-limit) — don't cache "none",
        // or the album would be stuck without external ordering forever.
        external::FetchOutcome::TemporaryError => Ok(none_layout()),
    }
}

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

/// Loads all already-cached artist images for fast render on startup (no network).
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

// ── Batch prep of external sources (one pass, permanent cache) ──────────────

/// Bumped on every call so a new prep pass cancels any in-flight one (e.g. on
/// settings toggle).
static EXTERNAL_PREP_GENERATION: AtomicU64 = AtomicU64::new(0);

fn external_prep_current(generation: u64) -> bool {
    EXTERNAL_PREP_GENERATION.load(Ordering::SeqCst) == generation
}

/// (Re)start background prep: one pass over the library that pre-fetches and
/// caches covers/avatars/album order for enabled sources. Already-settled
/// entries (ok/none) aren't retried. Safe to call on startup or any settings toggle.
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

/// Tracks sharing one cover cache key (usually one album).
struct CoverGroup {
    artist: String,
    album: String,
    title: String,
    paths: Vec<String>,
}

/// One unit of work for the prep worker pool.
enum PrepTask {
    Artist(String),
    Cover(CoverGroup),
    Album(String),
}

/// Tasks hit different hosts (Deezer/iTunes/MusicBrainz/CAA/Wikipedia) and each
/// host is separately rate-limited (`external::send_throttled`), so concurrent
/// threads are safe and multiply throughput instead of summing latencies.
const EXTERNAL_PREP_CONCURRENCY: usize = 6;

/// Pulls tasks from a shared cursor until exhausted or the pass is cancelled
/// (generation changed).
#[allow(clippy::too_many_arguments)]
fn external_prep_worker(
    app: tauri::AppHandle,
    tasks: Arc<Vec<PrepTask>>,
    cursor: Arc<AtomicUsize>,
    done: Arc<AtomicUsize>,
    none_keys: Arc<Mutex<Vec<String>>>,
    override_paths: Arc<HashSet<String>>,
    cover_present_paths: Arc<HashSet<String>>,
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
                // Needs an external cover if at least one track lacks both override and
                // embedded art. cover_present_paths is a precomputed snapshot (see
                // run_external_preparation) instead of re-parsing tags per path here.
                let needs_external = g
                    .paths
                    .iter()
                    .any(|p| !override_paths.contains(p) && !cover_present_paths.contains(p));
                if needs_external {
                    let _ =
                        external::get_or_fetch_track_cover(&dir, &g.artist, &g.album, &g.title, false);
                } else {
                    // Fully covered locally — mark "none" (batched) so next startup skips re-reading tags.
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
    // ── Collect tasks (short library locks) ──
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

    let (cover_groups, override_paths, cover_present_paths): (
        Vec<CoverGroup>,
        HashSet<String>,
        HashSet<String>,
    ) = if covers_on {
        let state = app.state::<LibraryState>();
        let built = match state.0.lock() {
            Ok(lib) => {
                let overrides = lib.override_cover_paths();
                // Snapshot once up front instead of re-parsing tags per path inside
                // the worker pool (see external_prep_worker's needs_external check).
                let present = lib.cover_present_paths();
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
                (by_key.into_values().collect(), overrides, present)
            }
            Err(_) => (Vec::new(), HashSet::new(), HashSet::new()),
        };
        built
    } else {
        (Vec::new(), HashSet::new(), HashSet::new())
    };

    // ── Filter out already-settled entries, compute total ──
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

    // Interleave task types (artist/cover/album) so the pool hits different hosts
    // at once instead of piling onto one.
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
    let cover_present_paths = Arc::new(cover_present_paths);
    let dir = Arc::new(dir);

    // N-1 background threads + current thread as one more worker, so progress
    // continues even if some spawns fail.
    let worker_count = EXTERNAL_PREP_CONCURRENCY.min(tasks.len()).max(1);
    let mut handles = Vec::with_capacity(worker_count.saturating_sub(1));
    for i in 1..worker_count {
        let app = app.clone();
        let tasks = Arc::clone(&tasks);
        let cursor = Arc::clone(&cursor);
        let done = Arc::clone(&done);
        let none_keys = Arc::clone(&none_keys);
        let override_paths = Arc::clone(&override_paths);
        let cover_present_paths = Arc::clone(&cover_present_paths);
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
                    cover_present_paths,
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
        Arc::clone(&cover_present_paths),
        Arc::clone(&dir),
        total,
        generation,
    );
    for h in handles {
        let _ = h.join();
    }

    // Batch-save "no external cover" markers once all workers finished collecting keys.
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

// ── Equalizer commands ───────────────────────────────────────────────────────

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
fn eq_get_settings(state: State<PlayerState>) -> Result<EqSettings, String> {
    let player = state.0.read().map_err(|e| format!("Lock error: {}", e))?;
    Ok(player.get_eq_settings())
}

// ── Settings commands ────────────────────────────────────────────────────────

/// Options are freeform (bool/string/etc.), stored as JSON values.
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

// ── Player state persistence ─────────────────────────────────────────────────

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

// ── Pins commands ─────────────────────────────────────────────────────────────

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

// ── Track metadata override commands ─────────────────────────────────────────

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverInput {
    source_path: Option<String>,
    /// true = reset cover + accent to the original from file tags.
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

/// title/artist: empty string means "revert to original". cover: None leaves the
/// current cover untouched; `reset: true` reverts to the file's tagged cover.
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

    fetch_track_with_override(&library, &file_path)
}

#[tauri::command]
fn read_image_as_data_url(source_path: String) -> Result<String, String> {
    let (data_url, _) = read_cover_as_data_url(&source_path)?;
    Ok(data_url)
}

/// Clears all track overrides at once ("Reset edits" button in settings).
#[tauri::command]
fn track_clear_all_overrides(library: State<LibraryState>) -> Result<usize, String> {
    let lib = library.0.lock().map_err(|e| format!("Lock error: {}", e))?;
    lib.clear_all_track_overrides()
}

fn fetch_track_with_override(
    library: &State<LibraryState>,
    file_path: &str,
) -> Result<TrackMetadata, String> {
    // Prefer the library row (overrides already applied); fall back to reading
    // tags from disk if the track isn't in the library (e.g. playing an unsaved file).
    let from_lib = {
        let lib = library.0.lock().map_err(|e| format!("Lock error: {}", e))?;
        lib.get_track(file_path).map(|mut t| {
            // Mapper only fills cover_art for override covers; backfill embedded cover URL.
            if t.cover_art.is_none() && lib.has_cover(file_path) {
                t.cover_art = Some(thumbnails::cover_protocol_url(
                    file_path,
                    &thumbnails::embedded_sig(file_path),
                ));
            }
            t
        })
    };
    if let Some(t) = from_lib {
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

// ── Custom cover commands ─────────────────────────────────────────────────────

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

// ── Entry point ───────────────────────────────────────────────────────────────

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

// ── System media integration (MPRIS, Linux) ──────────────────────────────────

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

// async — Discord client IPC (pipe/socket) must not block the main thread.
#[tauri::command(async)]
fn discord_update_presence(payload: discord_rpc::PresencePayload) -> bool {
    discord_rpc::update_presence(payload)
}

#[tauri::command(async)]
fn discord_clear_presence() {
    discord_rpc::clear_presence();
}

// ── Track downloads ("Downloads" page) ───────────────────────────────────────

/// Paginated search for downloadable tracks; marks `in_library` for tracks already present locally.
#[tauri::command]
async fn downloads_search(
    app: tauri::AppHandle,
    query: String,
    // Exact artist/title (clicking a "phantom" track in an album) enable exact
    // Deezer field search; absent for a plain string search.
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

/// Resolves metadata for a direct-link download (the "paste a link" flow —
/// downloads a specific yt-dlp-supported URL instead of searching). The
/// frontend shows the result like any other search hit and hands it straight
/// back to `downloads_enqueue`, which downloads that exact URL (see
/// `ExternalSearchTrack::source_url` / `DownloadRequest::direct_url`).
#[tauri::command]
async fn downloads_resolve_url(
    app: tauri::AppHandle,
    url: String,
) -> Result<external::ExternalSearchTrack, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut track = downloads::resolve_url_metadata(&app, &url)?;
        mark_tracks_in_library(&app, std::slice::from_mut(&mut track));
        Ok(track)
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

/// Marks `in_library` when title matches, at least one artist overlaps (multi-artist
/// aware), and album is compatible (either side has no album tag, or album keys match) —
/// so a same-titled track from a DIFFERENT album by the same artist isn't falsely marked.
fn mark_tracks_in_library(app: &tauri::AppHandle, tracks: &mut [external::ExternalSearchTrack]) {
    if tracks.is_empty() {
        return;
    }
    let separators = settings::get_artist_separators(&app.state::<SettingsState>());
    let lib_index = app
        .state::<LibraryState>()
        .0
        .lock()
        .map(|lib| lib.catalog_artist_title_index(&separators))
        .unwrap_or_default();

    // Prepare separators once outside the loop (avoids realloc/sort per track).
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

#[tauri::command]
fn downloads_enqueue(
    app: tauri::AppHandle,
    request: downloads::DownloadRequest,
) -> Result<u64, String> {
    Ok(downloads::enqueue(&app, request))
}

#[tauri::command]
fn downloads_enqueue_many(
    app: tauri::AppHandle,
    requests: Vec<downloads::DownloadRequest>,
) -> Result<Vec<u64>, String> {
    Ok(downloads::enqueue_many(&app, requests))
}

#[tauri::command]
fn downloads_snapshot(app: tauri::AppHandle) -> downloads::DownloadSnapshot {
    downloads::snapshot(&app)
}

/// Removes a queued item, cancelling it first if active.
#[tauri::command(async)]
fn downloads_remove(app: tauri::AppHandle, id: u64) {
    downloads::remove(&app, id);
}

#[tauri::command(async)]
fn downloads_retry(app: tauri::AppHandle, id: u64) {
    downloads::retry(&app, id);
}

#[tauri::command]
fn downloads_clear_finished(app: tauri::AppHandle) {
    downloads::clear_finished(&app);
}

/// Frontend uses this path to tell whether a track was downloaded by the app.
#[tauri::command]
fn downloads_dir_path(app: tauri::AppHandle) -> String {
    downloads::downloads_dir_path(&app)
}

/// `folder_path` = None/empty resets to the default "LFMusic Downloads" folder.
/// If the new folder is already nested inside a scanned folder, it's not added
/// separately (avoids double-scanning). Any prep error falls back to the default.
/// Returns the path actually applied.
#[tauri::command]
async fn downloads_set_dir(
    app: tauri::AppHandle,
    folder_path: Option<String>,
) -> Result<String, String> {
    // Scanning the new folder can be slow, so this runs on the blocking pool like library_add_folder.
    tauri::async_runtime::spawn_blocking(move || {
        let (old_dir, new_dir) = downloads::apply_downloads_dir(&app, folder_path);

        if !downloads::same_path(&old_dir, &new_dir) {
            let separators = settings::get_artist_separators(&app.state::<SettingsState>());
            let old = old_dir.to_string_lossy().to_string();
            let new = new_dir.to_string_lossy().to_string();

            // Registration happens under one library lock so the nesting check + mutation are atomic.
            let state = app.state::<LibraryState>();
            let mut library = state.0.lock().map_err(|e| format!("Lock error: {}", e))?;

            // Only remove the old folder if it had its own DB entry — if it was
            // nested inside a user folder, removing it would delete the user's tracks too.
            if library.covering_folder(&old).as_deref() == Some(old.as_str()) {
                library.remove_folder(
                    old,
                    &separators,
                    Some(audio_feature_progress_emitter(app.clone())),
                )?;
            }

            // Skip adding if nested inside an already-scanned folder (avoids double-scanning).
            if library.covering_folder(&new).is_none() {
                library.add_folder(
                    new,
                    &separators,
                    Some(audio_feature_progress_emitter(app.clone())),
                    scan_progress_emitter(app.clone()),
                )?;
            }
            drop(library);

            let _ = app.emit("downloads_library_updated", ());
        }

        Ok(new_dir.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

/// Windows: opens via `ShellExecuteW` (verb "open") rather than calling `explorer.exe`
/// directly, so a user-registered third-party file manager isn't bypassed.
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
        // Null-terminated wide string; kept alive for the duration of the call.
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
        // ShellExecute returns HINSTANCE > 32 on success (legacy API semantics).
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

#[tauri::command(async)]
fn downloads_delete_track(app: tauri::AppHandle, file_path: String) -> Result<(), String> {
    downloads::delete_downloaded_track(&app, &file_path)
}

#[tauri::command]
async fn downloads_check_tools(app: tauri::AppHandle) -> Result<downloads::ToolsStatus, String> {
    tauri::async_runtime::spawn_blocking(move || downloads::refresh_tools_status(&app))
        .await
        .map_err(|e| format!("Task error: {}", e))
}

/// Background network check of the imported YouTube cookies; result arrives via `download_queue` snapshot.
#[tauri::command]
fn downloads_validate_cookies(app: tauri::AppHandle) {
    downloads::validate_youtube_cookies(&app);
}

/// Forgets the imported session (cookies.txt + status), e.g. to sign out.
#[tauri::command]
fn downloads_clear_youtube_cookies(app: tauri::AppHandle) {
    downloads::clear_bridge_cookies(&app);
}

/// Writes the bundled browser-extension source to `<app_data_dir>/browser-extension/`
/// (on demand, not at startup) and returns that folder's path so the UI can
/// open it for "Load unpacked".
#[tauri::command]
fn browser_bridge_prepare_extension(app: tauri::AppHandle) -> Result<String, String> {
    browser_bridge::prepare_extension_bundle(&app)
}

#[tauri::command]
fn downloads_get_proxy(app: tauri::AppHandle) -> downloads::ProxyConfig {
    downloads::get_proxy(&app)
}

/// `mode`: "system" | "none" | "custom"; `url` only used for "custom" (e.g. socks5://127.0.0.1:1080).
#[tauri::command]
fn downloads_set_proxy(app: tauri::AppHandle, mode: String, url: String) {
    downloads::set_proxy(&app, mode, url);
}

// ── Tool management (yt-dlp/ffmpeg) — Settings "External sources" page ───────

fn tools_app_data_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[derive(Clone, serde::Serialize)]
struct ToolsProgress {
    /// "ytdlp" | "ffmpeg".
    tool: String,
    message: Option<String>,
    active: bool,
}

/// Local-only info (versions/managed); no network access.
#[tauri::command]
async fn tools_info(app: tauri::AppHandle) -> Result<tools::ToolsInfo, String> {
    tauri::async_runtime::spawn_blocking(move || tools::info(&tools_app_data_dir(&app)))
        .await
        .map_err(|e| format!("Task error: {}", e))
}

/// Checks GitHub for updates; same shape as [`tools_info`] with `update_available` filled in.
#[tauri::command]
async fn tools_check_updates(app: tauri::AppHandle) -> Result<tools::ToolsInfo, String> {
    tauri::async_runtime::spawn_blocking(move || tools::check_updates(&tools_app_data_dir(&app)))
        .await
        .map_err(|e| format!("Task error: {}", e))
}

/// Progress streams via `tools_progress`; `tools_changed` emits on completion.
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
            // Deno installs alongside yt-dlp, best-effort: a Deno failure doesn't roll
            // back yt-dlp (it still works, degraded) — just reported in progress.
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
            "pot" => tools::install_pot(&dir, &report),
            _ => Err("Неизвестная утилита".to_string()),
        };
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
    // Emit tools_changed unconditionally (even on error/panic) and before
    // inspecting the join result, so the UI never stays stale.
    let _ = app.emit("tools_changed", ());
    info.map_err(|e| format!("Task error: {}", e))?
}

/// Installs Deno if missing, for users who installed yt-dlp before the Deno
/// pairing existed. Progress reports under the "ytdlp" key.
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
    let _ = app.emit("tools_changed", ());
    info.map_err(|e| format!("Task error: {}", e))?
}

/// Installs everything in one go: yt-dlp (+ Deno) is required and fails the
/// call on error; ffmpeg is optional (its failure isn't fatal — downloads still work).
#[tauri::command]
async fn tools_install_all(app: tauri::AppHandle) -> Result<tools::ToolsInfo, String> {
    let app_for_task = app.clone();
    let info = tauri::async_runtime::spawn_blocking(move || {
        fn make_report(app: tauri::AppHandle, tool: &'static str) -> impl Fn(String) {
            move |msg: String| {
                let _ = app.emit(
                    "tools_progress",
                    ToolsProgress {
                        tool: tool.to_string(),
                        message: Some(msg),
                        active: true,
                    },
                );
            }
        }
        fn clear_progress(app: &tauri::AppHandle, tool: &'static str) {
            let _ = app.emit(
                "tools_progress",
                ToolsProgress {
                    tool: tool.to_string(),
                    message: None,
                    active: false,
                },
            );
        }

        let dir = tools_app_data_dir(&app_for_task);

        // yt-dlp+Deno and ffmpeg are independent downloads from different hosts;
        // installing them on separate threads instead of back-to-back bounds
        // first-run setup time by the larger of the two instead of their sum.
        // Only installs what's missing — updating is a separate button.
        let ffmpeg_app = app_for_task.clone();
        let ffmpeg_dir = dir.clone();
        let ytdlp_result = std::thread::scope(|scope| {
            let ffmpeg_handle = scope.spawn(move || {
                let report_ffmpeg = make_report(ffmpeg_app.clone(), "ffmpeg");
                if tools::resolve_ffmpeg(&ffmpeg_dir).is_none() {
                    if let Err(e) = tools::install_ffmpeg(&ffmpeg_dir, &report_ffmpeg) {
                        report_ffmpeg(format!("ffmpeg не установлен: {}", e));
                    }
                }
                clear_progress(&ffmpeg_app, "ffmpeg");
            });

            // install_deno is idempotent (Ok if Deno already present).
            let report_ytdlp = make_report(app_for_task.clone(), "ytdlp");
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
            clear_progress(&app_for_task, "ytdlp");

            let _ = ffmpeg_handle.join();
            ytdlp_result
        });

        ytdlp_result?;
        Ok::<_, String>(tools::info(&dir))
    })
    .await;
    let _ = app.emit("tools_changed", ());
    info.map_err(|e| format!("Task error: {}", e))?
}

#[tauri::command]
async fn tools_delete(app: tauri::AppHandle, tool: String) -> Result<tools::ToolsInfo, String> {
    // Refuse while a download job may be actively using the resolved binary —
    // deleting it out from under an active worker would break the job.
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
            // Deno is removed together with yt-dlp (installed as a pair).
            "ytdlp" => {
                tools::delete_ytdlp(&dir)?;
                tools::delete_deno(&dir)?;
            }
            "ffmpeg" => tools::delete_ffmpeg(&dir)?,
            "pot" => tools::delete_pot(&dir)?,
            _ => return Err("Неизвестная утилита".to_string()),
        }
        Ok::<_, String>(tools::info(&dir))
    })
    .await
    .map_err(|e| format!("Task error: {}", e))??;
    let _ = app.emit("tools_changed", ());
    Ok(info)
}

/// Blocks on `signal` (near-instant wakeup on natural EOF, 500ms safety-net
/// fallback otherwise) and emits `track_ended` with the finished file's path —
/// replaces relying on the frontend's 500ms status poll to notice playback
/// stopped, which added up to half a second of dead air between tracks.
fn spawn_track_end_watcher(app: tauri::AppHandle, signal: Arc<SinkEndSignal>) {
    std::thread::Builder::new()
        .name("track-end-watcher".into())
        .spawn(move || loop {
            signal.wait(Duration::from_millis(500));
            let player_state = app.state::<PlayerState>();
            let finished_path = match player_state.0.write() {
                Ok(mut player) => player.poll_finished(),
                Err(_) => None,
            };
            if let Some(path) = finished_path {
                let _ = app.emit("track_ended", path);
            }
        })
        .expect("Не удалось запустить поток отслеживания окончания трека");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "windows")]
    set_windows_priority_class();

    let player = AudioPlayer::new().expect("Не удалось инициализировать аудиоплеер");
    let track_end_signal = player.sink_end_signal();

    tauri::Builder::default()
        // Serves lazily-generated, disk-cached WebP thumbnails by file_path, avoiding base64 over IPC.
        .register_asynchronous_uri_scheme_protocol("lfmusic-cover", |ctx, request, responder| {
            thumbnails::protocol_handler(ctx, request, responder);
        })
        // Same pattern as lfmusic-cover, for cached external artist/cover images
        // at lfmusic-external://{artist|cover}/<hash>.
        .register_asynchronous_uri_scheme_protocol("lfmusic-external", |ctx, request, responder| {
            external::protocol_handler(ctx, request, responder);
        })
        .setup(move |app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Не удалось получить директорию данных приложения");

            window_state::restore_main_window_geometry(app, &app_data_dir);

            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                if let Ok(hwnd) = window.hwnd() {
                    taskbar::init(&app.handle().clone(), hwnd.0 as isize);
                }
            }

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

            let player_state = app.state::<PlayerState>();
            let player = player_state.0.read().expect("Lock error");
            player.set_eq_settings(app_settings.eq_settings.clone());
            drop(player);

            // Must be managed before code below calls app.state::<SettingsState>().
            app.manage(SettingsState(Mutex::new((
                app_settings,
                app_data_dir.clone(),
            ))));

            let external_marker_dir = app_data_dir.clone();
            let mut library =
                MusicLibrary::new(app_data_dir).expect("Не удалось инициализировать библиотеку");

            // Rebuild once on startup so existing libraries get artist playlists without waiting for a rescan.
            let separators = {
                let s = app.state::<SettingsState>();
                settings::get_artist_separators(&s)
            };
            if let Err(e) = library.rebuild_artist_playlists(&separators) {
                eprintln!("Ошибка инициализации плейлистов исполнителей: {}", e);
            }

            // One-time migration: clear the external-album negative cache, since old
            // "none" entries could have been cached from transient errors (no longer the case).
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

            browser_bridge::start(app.handle().clone());
            spawn_track_end_watcher(app.handle().clone(), track_end_signal.clone());

            Ok(())
        })
        .manage(PlayerState(RwLock::new(player)))
        .manage(downloads::DownloadsState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            // Player
            player_play,
            player_play_crossfade,
            player_play_paused,
            player_pause,
            player_resume,
            player_stop,
            player_set_volume,
            player_seek,
            player_seek_crossfade,
            player_get_status,
            player_get_spectrum,
            player_set_spectrum_enabled,
            get_track_metadata,
            get_cover_art,
            get_original_cover_art,
            // Library
            library_get_folders,
            library_add_folder,
            library_add_android_music_folders,
            library_remove_folder,
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
            // Playlists
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
            // External sources
            external_album_layout,
            external_artist_image,
            external_track_cover,
            external_load_cached_artist_images,
            external_prepare,
            // Downloads
            downloads_search,
            downloads_resolve_url,
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
            downloads_validate_cookies,
            downloads_clear_youtube_cookies,
            browser_bridge_prepare_extension,
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
            // Equalizer
            eq_set_bands,
            eq_set_gain,
            eq_set_enabled,
            eq_get_settings,
            // Settings
            get_settings,
            save_settings,
            // Player state
            save_player_state,
            load_player_state,
            // Pins
            pins_load,
            pins_save,
            // Custom covers
            cover_set_custom,
            cover_remove_custom,
            cover_load_all,
            // Track metadata overrides
            track_set_override,
            track_clear_all_overrides,
            get_original_track,
            read_image_as_data_url,
            // Window
            window_state::toggle_window_fullscreen,
            window_state::toggle_window_maximize,
            open_folder_in_explorer,
            // Taskbar (Windows)
            taskbar_set_playing,
            taskbar_set_modes,
            taskbar_set_covers,
            // System media integration (MPRIS, Linux)
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
                // Kill any active yt-dlp first, or it'd linger as a zombie process.
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
