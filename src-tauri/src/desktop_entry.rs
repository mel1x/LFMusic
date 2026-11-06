//! Linux: registers the app in the user's desktop (`~/.local/share`) so panels
//! and taskbars show the LFMusic logo. On Wayland the compositor only knows the
//! window's app_id (`lfmusic`, the binary name) and the panel resolves the icon
//! through a matching `.desktop` file; the window icon set by GTK is ignored.
//! A plain binary (`tauri build --no-bundle`, a copied release) has no such
//! file, so it would get a generic icon. Packaged installs (.deb/.rpm under
//! /usr, /opt) ship their own entry and are left alone.

#[cfg(target_os = "linux")]
mod imp {
    use std::fs;
    use std::path::{Path, PathBuf};

    const APP_ID: &str = "lfmusic";

    const ICONS: &[(&str, &[u8])] = &[
        ("128x128", include_bytes!("../icons/128x128.png")),
        ("256x256", include_bytes!("../icons/128x128@2x.png")),
        ("512x512", include_bytes!("../icons/icon.png")),
    ];

    /// Writes `contents` only if the file is missing or differs, so a regular
    /// start touches nothing (and doesn't wake panels watching these dirs).
    fn write_if_changed(path: &Path, contents: &[u8]) -> std::io::Result<()> {
        if fs::read(path).ok().as_deref() == Some(contents) {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, contents)
    }

    fn quote_exec(path: &str) -> String {
        // Desktop Entry spec: quoted args escape `"`, `` ` ``, `$` and `\`.
        let mut out = String::from("\"");
        for c in path.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    }

    pub fn register(data_dir: PathBuf) {
        // AppImage runs from a temporary mount; the stable path is $APPIMAGE.
        let exe = match std::env::var_os("APPIMAGE") {
            Some(p) => PathBuf::from(p),
            None => match std::env::current_exe() {
                Ok(p) => p,
                Err(_) => return,
            },
        };
        if exe.starts_with("/usr") || exe.starts_with("/opt") || exe.starts_with("/nix/store") {
            return;
        }

        let entry_path = data_dir.join("applications").join(format!("{APP_ID}.desktop"));
        // `tauri dev` must not repoint an existing entry at the debug binary.
        if cfg!(debug_assertions) && entry_path.exists() {
            return;
        }

        let icons_dir = data_dir.join("icons").join("hicolor");
        for (size, png) in ICONS {
            let path = icons_dir.join(size).join("apps").join(format!("{APP_ID}.png"));
            if let Err(e) = write_if_changed(&path, png) {
                eprintln!("[desktop_entry] icon {}: {e}", path.display());
            }
        }

        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=LFMusic\n\
             Comment=Music player for your local library\n\
             Exec={}\n\
             Icon={APP_ID}\n\
             StartupWMClass={APP_ID}\n\
             Categories=AudioVideo;Audio;Player;\n\
             Terminal=false\n",
            quote_exec(&exe.to_string_lossy()),
        );
        if let Err(e) = write_if_changed(&entry_path, entry.as_bytes()) {
            eprintln!("[desktop_entry] {}: {e}", entry_path.display());
        }
    }
}

#[cfg(target_os = "linux")]
pub use imp::register;
