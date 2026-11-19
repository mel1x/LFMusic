use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{Monitor, WebviewWindow};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::sync::{Arc, Mutex};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::time::{Duration, Instant};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::{Manager, PhysicalPosition, PhysicalSize, Position, Size, WindowEvent};

pub const DEFAULT_WINDOW_WIDTH: u32 = 1460;
pub const DEFAULT_WINDOW_HEIGHT: u32 = 860;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MonitorSnapshot {
    pub name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SavedWindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
    pub maximized: bool,
    pub monitor: MonitorSnapshot,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedWindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

const WINDOW_STATE_FILE: &str = "window_state.json";

fn monitors_match(saved: &MonitorSnapshot, current: &MonitorSnapshot) -> bool {
    match (&saved.name, &current.name) {
        (Some(saved_name), Some(current_name)) => saved_name == current_name,
        _ => {
            saved.x == current.x
                && saved.y == current.y
                && saved.width == current.width
                && saved.height == current.height
                && (saved.scale_factor - current.scale_factor).abs() < f64::EPSILON
        }
    }
}

fn is_geometry_valid(x: i32, y: i32, width: u32, height: u32) -> bool {
    x != -32000 && y != -32000 && width >= 300 && height >= 300
}

pub fn resolve_window_geometry(
    saved: Option<&SavedWindowGeometry>,
    monitors: &[MonitorSnapshot],
    default_width: u32,
    default_height: u32,
) -> Option<ResolvedWindowGeometry> {
    if let Some(saved) = saved
        && is_geometry_valid(saved.x, saved.y, saved.width, saved.height)
            && monitors
                .iter()
                .any(|monitor| monitors_match(&saved.monitor, monitor))
            {
                return Some(ResolvedWindowGeometry {
                    x: saved.x,
                    y: saved.y,
                    width: saved.width,
                    height: saved.height,
                });
            }

    let first = monitors.first()?;
    let width = default_width.min(first.width);
    let height = default_height.min(first.height);
    let x = first.x + ((first.width.saturating_sub(width)) / 2) as i32;
    let y = first.y + ((first.height.saturating_sub(height)) / 2) as i32;

    Some(ResolvedWindowGeometry {
        x,
        y,
        width,
        height,
    })
}

fn state_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(WINDOW_STATE_FILE)
}

fn load(app_data_dir: &Path) -> Option<SavedWindowGeometry> {
    let path = state_path(app_data_dir);
    let bytes = fs::read(path).ok()?;
    let state: SavedWindowGeometry = serde_json::from_slice(&bytes).ok()?;
    if is_geometry_valid(state.x, state.y, state.width, state.height) {
        Some(state)
    } else {
        None
    }
}

fn save(app_data_dir: &Path, state: &SavedWindowGeometry) -> Result<(), String> {
    fs::create_dir_all(app_data_dir).map_err(|e| e.to_string())?;
    let path = state_path(app_data_dir);
    let json = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

fn monitor_snapshot(monitor: &Monitor) -> MonitorSnapshot {
    MonitorSnapshot {
        name: monitor.name().cloned(),
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
        scale_factor: monitor.scale_factor(),
    }
}

fn available_monitor_snapshots(window: &WebviewWindow) -> Vec<MonitorSnapshot> {
    window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(monitor_snapshot)
        .collect()
}

fn current_window_geometry(
    window: &WebviewWindow,
    previous: Option<SavedWindowGeometry>,
) -> Result<SavedWindowGeometry, String> {
    let fullscreen = window.is_fullscreen().map_err(|e| e.to_string())?;
    let maximized = window.is_maximized().map_err(|e| e.to_string())?;

    // Fullscreen/maximized: keep the previous normal geometry, only update flags and monitor.
    if (fullscreen || maximized)
        && let Some(mut state) = previous {
            state.fullscreen = fullscreen;
            state.maximized = maximized;
            if let Ok(Some(monitor)) = window.current_monitor() {
                state.monitor = monitor_snapshot(&monitor);
            }
            return Ok(state);
        }

    let position = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .map(|monitor| monitor_snapshot(&monitor))
        .or_else(|| available_monitor_snapshots(window).into_iter().next())
        .ok_or_else(|| "No monitor available".to_string())?;

    Ok(SavedWindowGeometry {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        fullscreen,
        maximized,
        monitor,
    })
}

fn save_current_window_geometry(window: &WebviewWindow, app_data_dir: &Path) -> Result<(), String> {
    let state = current_window_geometry(window, load(app_data_dir))?;
    if is_geometry_valid(state.x, state.y, state.width, state.height) {
        save(app_data_dir, &state)?;
    }
    Ok(())
}

/// Toggles fullscreen (F11). Maximized is cleared first: maximized -> fullscreen breaks window
/// state on Windows.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub fn toggle_window_fullscreen(window: WebviewWindow) -> Result<bool, String> {
    let is_fullscreen = window.is_fullscreen().map_err(|e| e.to_string())?;
    if is_fullscreen {
        window.set_fullscreen(false).map_err(|e| e.to_string())?;
        Ok(false)
    } else {
        if window.is_maximized().map_err(|e| e.to_string())? {
            let position = window.outer_position().map_err(|e| e.to_string())?;
            let size = window.outer_size().map_err(|e| e.to_string())?;
            window.unmaximize().map_err(|e| e.to_string())?;
            window
                .set_position(Position::Physical(PhysicalPosition::new(position.x, position.y)))
                .map_err(|e| e.to_string())?;
            window
                .set_size(Size::Physical(PhysicalSize::new(size.width, size.height)))
                .map_err(|e| e.to_string())?;
        }
        window.set_fullscreen(true).map_err(|e| e.to_string())?;
        Ok(true)
    }
}

/// Toggle main window maximize.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub fn toggle_window_maximize(window: WebviewWindow) -> Result<(), String> {
    if window.is_fullscreen().map_err(|e| e.to_string())? {
        window.set_fullscreen(false).map_err(|e| e.to_string())?;
        window.maximize().map_err(|e| e.to_string())?;
        return Ok(());
    }
    if window.is_maximized().map_err(|e| e.to_string())? {
        window.unmaximize().map_err(|e| e.to_string())?;
    } else {
        window.maximize().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub fn toggle_window_fullscreen(_window: WebviewWindow) -> Result<bool, String> {
    Ok(false)
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub fn toggle_window_maximize(_window: WebviewWindow) -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn restore_main_window_geometry(app: &tauri::App, app_data_dir: &Path) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let monitors = available_monitor_snapshots(&window);
    let saved_state = load(app_data_dir);
    if let Some(geometry) = resolve_window_geometry(
        saved_state.as_ref(),
        &monitors,
        DEFAULT_WINDOW_WIDTH,
        DEFAULT_WINDOW_HEIGHT,
    ) {
        let _ = window.set_size(Size::Physical(PhysicalSize::new(
            geometry.width,
            geometry.height,
        )));
        let _ = window.set_position(Position::Physical(PhysicalPosition::new(
            geometry.x, geometry.y,
        )));
    }
    // Mutually exclusive, fullscreen wins: setting both on Windows breaks window state.
    if saved_state.as_ref().is_some_and(|state| state.fullscreen) {
        let _ = window.set_fullscreen(true);
    } else if saved_state.as_ref().is_some_and(|state| state.maximized) {
        let _ = window.maximize();
    }

    let app_data_dir = app_data_dir.to_path_buf();
    let window_for_events = window.clone();
    let save_due: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { .. } => {
            *save_due.lock().unwrap_or_else(|e| e.into_inner()) = None;
            if let Err(e) = save_current_window_geometry(&window_for_events, &app_data_dir) {
                eprintln!("Window state save error: {}", e);
            }
        }
        WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            #[cfg(windows)]
            if matches!(event, WindowEvent::Resized(_)) {
                webview_memory::follow_minimized(&window_for_events);
            }
            schedule_save(&save_due, &window_for_events, &app_data_dir);
        }
        _ => {}
    });

    let _ = window.show();
}

/// Moves/resizes come dozens of times a second while dragging; the file is written once the window
/// is still.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const SAVE_DEBOUNCE: Duration = Duration::from_millis(600);

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn schedule_save(due: &Arc<Mutex<Option<Instant>>>, window: &WebviewWindow, app_data_dir: &Path) {
    let mut guard = due.lock().unwrap_or_else(|e| e.into_inner());
    let idle = guard.is_none();
    *guard = Some(Instant::now() + SAVE_DEBOUNCE);
    drop(guard);
    if !idle {
        return;
    }
    let due = Arc::clone(due);
    let window = window.clone();
    let app_data_dir = app_data_dir.to_path_buf();
    let _ = std::thread::Builder::new()
        .name("window-state-save".into())
        .spawn(move || {
            loop {
                let wait = {
                    let mut guard = due.lock().unwrap_or_else(|e| e.into_inner());
                    match *guard {
                        None => return,
                        Some(deadline) => {
                            let now = Instant::now();
                            if deadline <= now {
                                *guard = None;
                                break;
                            }
                            deadline - now
                        }
                    }
                };
                std::thread::sleep(wait);
            }
            if let Err(e) = save_current_window_geometry(&window, &app_data_dir) {
                eprintln!("Window state save error: {}", e);
            }
        });
}

/// WebView2 trims its memory while the window is minimized.
#[cfg(windows)]
mod webview_memory {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri::WebviewWindow;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    use windows::core::Interface;

    static LOW: AtomicBool = AtomicBool::new(false);

    pub fn follow_minimized(window: &WebviewWindow) {
        let minimized = window.is_minimized().unwrap_or(false);
        if LOW.swap(minimized, Ordering::Relaxed) == minimized {
            return;
        }
        let level = if minimized {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
        } else {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
        };
        let _ = window.with_webview(move |webview| unsafe {
            // ICoreWebView2_19 needs WebView2 Runtime 1.0.2210+; older runtimes skip this.
            if let Ok(core) = webview.controller().CoreWebView2()
                && let Ok(core19) = core.cast::<ICoreWebView2_19>()
            {
                let _ = core19.SetMemoryUsageTargetLevel(level);
            }
        });
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn restore_main_window_geometry(_app: &tauri::App, _app_data_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, x: i32, y: i32, width: u32, height: u32) -> MonitorSnapshot {
        MonitorSnapshot {
            name: Some(name.to_string()),
            x,
            y,
            width,
            height,
            scale_factor: 1.0,
        }
    }

    #[test]
    fn restores_saved_geometry_when_saved_monitor_is_available() {
        let saved = SavedWindowGeometry {
            x: 1960,
            y: 120,
            width: 1200,
            height: 720,
            fullscreen: false,
            maximized: false,
            monitor: monitor("DISPLAY2", 1920, 0, 2560, 1440),
        };
        let monitors = [
            monitor("DISPLAY1", 0, 0, 1920, 1080),
            monitor("DISPLAY2", 1920, 0, 2560, 1440),
        ];

        let resolved = resolve_window_geometry(Some(&saved), &monitors, 1460, 860).unwrap();

        assert_eq!(
            resolved,
            ResolvedWindowGeometry {
                x: 1960,
                y: 120,
                width: 1200,
                height: 720,
            }
        );
    }

    #[test]
    fn centers_default_size_on_first_monitor_when_saved_monitor_is_missing() {
        let saved = SavedWindowGeometry {
            x: 1960,
            y: 120,
            width: 1200,
            height: 720,
            fullscreen: false,
            maximized: false,
            monitor: monitor("DISPLAY2", 1920, 0, 2560, 1440),
        };
        let monitors = [monitor("DISPLAY1", 0, 0, 1920, 1080)];

        let resolved = resolve_window_geometry(Some(&saved), &monitors, 1460, 860).unwrap();

        assert_eq!(
            resolved,
            ResolvedWindowGeometry {
                x: 230,
                y: 110,
                width: 1460,
                height: 860,
            }
        );
    }

    #[test]
    fn serializes_fullscreen_state() {
        let saved = SavedWindowGeometry {
            x: 10,
            y: 20,
            width: 1200,
            height: 720,
            fullscreen: true,
            maximized: false,
            monitor: monitor("DISPLAY1", 0, 0, 1920, 1080),
        };

        let value = serde_json::to_value(saved).unwrap();

        assert_eq!(value["fullscreen"], true);
    }

    #[test]
    fn serializes_and_restores_maximized_state() {
        let saved = SavedWindowGeometry {
            x: 10,
            y: 20,
            width: 1200,
            height: 720,
            fullscreen: false,
            maximized: true,
            monitor: monitor("DISPLAY1", 0, 0, 1920, 1080),
        };

        let json = serde_json::to_string(&saved).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["maximized"], true);

        let restored: SavedWindowGeometry = serde_json::from_str(&json).unwrap();
        assert!(restored.maximized);
        assert!(!restored.fullscreen);
    }

    #[test]
    fn monitors_match_falls_back_to_position_when_name_present_only_on_one_side() {
        let saved = MonitorSnapshot {
            name: None,
            ..monitor("IGNORED", 0, 0, 1920, 1080)
        };
        let current = monitor("DISPLAY1", 0, 0, 1920, 1080);
        assert!(monitors_match(&saved, &current));
    }

    #[test]
    fn monitors_match_by_name_ignores_position_drift() {
        let saved = monitor("DISPLAY2", 1920, 0, 2560, 1440);
        let current = monitor("DISPLAY2", 0, 0, 2560, 1440);
        assert!(monitors_match(&saved, &current));
    }

    #[test]
    fn resolve_window_geometry_returns_none_without_monitors() {
        assert_eq!(resolve_window_geometry(None, &[], 1460, 860), None);
    }

    #[test]
    fn resolve_window_geometry_boundary_width_300_is_valid() {
        let saved = SavedWindowGeometry {
            x: 10,
            y: 10,
            width: 300,
            height: 300,
            fullscreen: false,
            maximized: false,
            monitor: monitor("DISPLAY1", 0, 0, 1920, 1080),
        };
        let monitors = [monitor("DISPLAY1", 0, 0, 1920, 1080)];
        let resolved = resolve_window_geometry(Some(&saved), &monitors, 1460, 860).unwrap();
        assert_eq!(resolved.width, 300);
        assert_eq!(resolved.height, 300);
    }

    #[test]
    fn resolve_window_geometry_boundary_width_299_is_invalid() {
        let saved = SavedWindowGeometry {
            x: 10,
            y: 10,
            width: 299,
            height: 300,
            fullscreen: false,
            maximized: false,
            monitor: monitor("DISPLAY1", 0, 0, 1920, 1080),
        };
        let monitors = [monitor("DISPLAY1", 0, 0, 1920, 1080)];
        let resolved = resolve_window_geometry(Some(&saved), &monitors, 1460, 860).unwrap();
        assert_eq!(resolved.width, 1460);
    }

    #[test]
    fn resolve_window_geometry_clamps_default_size_to_small_monitor() {
        let monitors = [monitor("DISPLAY1", 0, 0, 800, 600)];
        let resolved = resolve_window_geometry(None, &monitors, 1460, 860).unwrap();
        assert_eq!(resolved.width, 800);
        assert_eq!(resolved.height, 600);
        assert_eq!(resolved.x, 0);
        assert_eq!(resolved.y, 0);
    }

    #[test]
    fn ignores_invalid_geometry_minimized_coordinates() {
        let saved = SavedWindowGeometry {
            x: -32000,
            y: -32000,
            width: 200,
            height: 41,
            fullscreen: false,
            maximized: false,
            monitor: monitor("DISPLAY1", 0, 0, 1920, 1080),
        };
        let monitors = [monitor("DISPLAY1", 0, 0, 1920, 1080)];

        let resolved = resolve_window_geometry(Some(&saved), &monitors, 1460, 860).unwrap();

        assert_eq!(
            resolved,
            ResolvedWindowGeometry {
                x: 230,
                y: 110,
                width: 1460,
                height: 860,
            }
        );
    }
}
