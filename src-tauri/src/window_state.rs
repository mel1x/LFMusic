use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{
    Manager, Monitor, PhysicalPosition, PhysicalSize, Position, Size, WebviewWindow, WindowEvent,
};

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
    if let Some(saved) = saved {
        if is_geometry_valid(saved.x, saved.y, saved.width, saved.height) {
            if monitors
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
        }
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

    // В полноэкранном (F11) или развёрнутом режиме физические position/size
    // отражают размеры на весь экран — их нельзя сохранять как «нормальную»
    // геометрию, иначе при выходе из режима окну не к чему будет вернуться.
    // Поэтому сохраняем предыдущую нормальную геометрию, обновляя только флаги
    // и текущий монитор.
    if fullscreen || maximized {
        if let Some(mut state) = previous {
            state.fullscreen = fullscreen;
            state.maximized = maximized;
            if let Ok(Some(monitor)) = window.current_monitor() {
                state.monitor = monitor_snapshot(&monitor);
            }
            return Ok(state);
        }
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

/// Переключить полноэкранный режим главного окна (F11).
///
/// Прямой переход из развёрнутого (maximized) состояния в fullscreen на Windows
/// вызывает рассинхрон состояния окна (лаги, «сломанный» fullscreen), поэтому
/// перед входом снимаем maximized. При этом сохраняем тот же размер и позицию
/// окна, чтобы оно не «прыгало» к старой свёрнутой геометрии — после выхода из
/// fullscreen окно останется ровно там же. Делается на стороне Rust одним
/// вызовом, чтобы избежать нескольких медленных IPC-переходов.
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

/// Переключить развёрнутое состояние главного окна (клик по «развернуть» в шапке).
///
/// Если окно в полноэкранном режиме (F11) — выходим из него и разворачиваем окно
/// (выключение F11 + maximize). В остальных случаях — обычное переключение
/// maximized.
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
    // Режимы взаимоисключающие: fullscreen имеет приоритет. Одновременная
    // установка maximized + fullscreen на Windows ломает состояние окна.
    if saved_state.as_ref().is_some_and(|state| state.fullscreen) {
        let _ = window.set_fullscreen(true);
    } else if saved_state.as_ref().is_some_and(|state| state.maximized) {
        let _ = window.maximize();
    }

    let app_data_dir = app_data_dir.to_path_buf();
    let window_for_events = window.clone();
    window.on_window_event(move |event| {
        if matches!(
            event,
            WindowEvent::Moved(_)
                | WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. }
                | WindowEvent::CloseRequested { .. }
        ) {
            if let Err(e) = save_current_window_geometry(&window_for_events, &app_data_dir) {
                eprintln!("Window state save error: {}", e);
            }
        }
    });

    let _ = window.show();
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
        // (Some, None)/(None, Some) не попадает в ветку сравнения по имени
        // (нужны Some с обеих сторон) — должен сработать фолбэк по позиции.
        let saved = MonitorSnapshot {
            name: None,
            ..monitor("IGNORED", 0, 0, 1920, 1080)
        };
        let current = monitor("DISPLAY1", 0, 0, 1920, 1080);
        assert!(monitors_match(&saved, &current));
    }

    #[test]
    fn monitors_match_by_name_ignores_position_drift() {
        // С обеих сторон есть имя — сравнение идёт по имени, даже если позиция
        // монитора изменилась (например, пользователь переставил мониторы местами).
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
        // 299 невалидно -> должно откатиться на центрирование дефолтного размера.
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

        // Should center the default size on the first monitor instead of using saved geometry
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
