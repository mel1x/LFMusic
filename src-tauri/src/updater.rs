//! In-app updates from GitHub releases. Windows only: the release carries signed
//! NSIS/MSI installers and `latest.json` (see `tauri.windows.conf.json`).
//! Other platforms report `supported: false`.

use serde::Serialize;
use tauri::AppHandle;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    supported: bool,
    /// Portable exe (not installed): updating means downloading it again.
    portable: bool,
    current: String,
    /// Newer version, `None` when up to date.
    version: Option<String>,
}

#[cfg(windows)]
mod imp {
    use super::UpdateCheck;
    use std::sync::Mutex;
    use tauri::utils::{config::BundleType, platform::bundle_type};
    use tauri::{AppHandle, Emitter, Manager};
    use tauri_plugin_updater::{Update, UpdaterExt};

    #[derive(Default)]
    pub struct PendingUpdate(Mutex<Option<Update>>);

    /// NSIS installs put `uninstall.exe` next to the app; MSI builds are patched
    /// with their bundle type. Anything else is a copied (portable) exe.
    fn is_portable() -> bool {
        let nsis_installed = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("uninstall.exe").exists()))
            .unwrap_or(false);
        !nsis_installed && bundle_type() != Some(BundleType::Msi)
    }

    pub async fn check(app: AppHandle) -> Result<UpdateCheck, String> {
        let update = app
            .updater()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| e.to_string())?;
        let version = update.as_ref().map(|u| u.version.clone());
        *app.state::<PendingUpdate>().0.lock().map_err(|_| "Lock error")? = update;
        Ok(UpdateCheck {
            supported: true,
            portable: is_portable(),
            current: app.package_info().version.to_string(),
            version,
        })
    }

    /// Downloads and runs the installer (passive mode). The installer closes the
    /// app itself; the restart is a fallback.
    pub async fn install(app: AppHandle) -> Result<(), String> {
        let update = app
            .state::<PendingUpdate>()
            .0
            .lock()
            .map_err(|_| "Lock error")?
            .clone()
            .ok_or("No update available")?;

        let emitter = app.clone();
        let mut downloaded: u64 = 0;
        let mut last_percent: i64 = -1;
        update
            .download_and_install(
                move |chunk, total| {
                    downloaded += chunk as u64;
                    let percent = total
                        .filter(|t| *t > 0)
                        .map(|t| (downloaded * 100 / t) as i64)
                        .unwrap_or(0);
                    if percent != last_percent {
                        last_percent = percent;
                        let _ = emitter.emit("update-progress", percent);
                    }
                },
                || {},
            )
            .await
            .map_err(|e| e.to_string())?;
        app.restart();
    }
}

#[cfg(windows)]
pub use imp::PendingUpdate;

#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<UpdateCheck, String> {
    #[cfg(windows)]
    {
        imp::check(app).await
    }
    #[cfg(not(windows))]
    {
        Ok(UpdateCheck {
            supported: false,
            portable: false,
            current: app.package_info().version.to_string(),
            version: None,
        })
    }
}

#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        imp::install(app).await
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("Updates are not supported on this platform".into())
    }
}
