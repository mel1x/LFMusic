//! In-app updates from GitHub releases.
//! Windows: tauri-plugin-updater with signed NSIS/MSI installers (see `tauri.windows.conf.json`).
//! Android: the same `latest.json` carries an `android` entry with the APK url; the APK is
//! downloaded to the cache dir and handed to the system installer by the JS bridge
//! (`LFMusicAndroid.installApk`). Other platforms report `supported: false`.

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
    pub async fn install(app: AppHandle) -> Result<Option<String>, String> {
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
                    let percent = super::percent(downloaded, total);
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

#[cfg(target_os = "android")]
mod imp {
    use super::UpdateCheck;
    use std::io::Write;
    use std::sync::Mutex;
    use tauri::{AppHandle, Emitter, Manager};

    const LATEST_URL: &str = "https://github.com/mel1x/LFMusic/releases/latest/download/latest.json";

    /// APK url of the newer release found by the last check.
    #[derive(Default)]
    pub struct PendingUpdate(Mutex<Option<String>>);

    fn parse_version(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|part| {
                let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse().unwrap_or(0)
            })
            .collect()
    }

    pub async fn check(app: AppHandle) -> Result<UpdateCheck, String> {
        let current = app.package_info().version.to_string();
        let latest: serde_json::Value = reqwest::Client::new()
            .get(LATEST_URL)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;

        let latest_version = latest["version"].as_str().unwrap_or_default();
        let apk_url = latest["platforms"]["android"]["url"].as_str();
        let found = match apk_url {
            Some(url) if parse_version(latest_version) > parse_version(&current) => {
                Some((latest_version.to_string(), url.to_string()))
            }
            _ => None,
        };

        *app.state::<PendingUpdate>().0.lock().map_err(|_| "Lock error")? =
            found.as_ref().map(|(_, url)| url.clone());
        Ok(UpdateCheck {
            supported: true,
            portable: false,
            current,
            version: found.map(|(version, _)| version),
        })
    }

    /// Downloads the APK into the cache dir (shared through the FileProvider
    /// `cache-path`) and returns its path for the JS bridge to install.
    pub async fn install(app: AppHandle) -> Result<Option<String>, String> {
        let url = app
            .state::<PendingUpdate>()
            .0
            .lock()
            .map_err(|_| "Lock error")?
            .clone()
            .ok_or("No update available")?;

        let dir = app
            .path()
            .app_cache_dir()
            .map_err(|e| e.to_string())?
            .join("updates");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("lfmusic-update.apk");

        let mut response = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?;
        let total = response.content_length();
        let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let mut downloaded: u64 = 0;
        let mut last_percent: i64 = -1;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            file.write_all(&chunk).map_err(|e| e.to_string())?;
            downloaded += chunk.len() as u64;
            let percent = super::percent(downloaded, total);
            if percent != last_percent {
                last_percent = percent;
                let _ = app.emit("update-progress", percent);
            }
        }
        file.flush().map_err(|e| e.to_string())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    }
}

#[cfg(any(windows, target_os = "android"))]
pub use imp::PendingUpdate;

#[cfg(any(windows, target_os = "android"))]
fn percent(downloaded: u64, total: Option<u64>) -> i64 {
    total
        .filter(|t| *t > 0)
        .map(|t| (downloaded * 100 / t) as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<UpdateCheck, String> {
    #[cfg(any(windows, target_os = "android"))]
    {
        imp::check(app).await
    }
    #[cfg(not(any(windows, target_os = "android")))]
    {
        Ok(UpdateCheck {
            supported: false,
            portable: false,
            current: app.package_info().version.to_string(),
            version: None,
        })
    }
}

/// Windows: installs and restarts (returns only on error).
/// Android: returns the downloaded APK path for `LFMusicAndroid.installApk`.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<Option<String>, String> {
    #[cfg(any(windows, target_os = "android"))]
    {
        imp::install(app).await
    }
    #[cfg(not(any(windows, target_os = "android")))]
    {
        let _ = app;
        Err("Updates are not supported on this platform".into())
    }
}
