# NixOS dev/build environment: `nix-shell`, then `pnpm install` and `pnpm tauri build --no-bundle`.
{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
    gobject-introspection
    cargo
    rustc
    nodejs
    pnpm

    # Downloader tools from PATH: binaries the app downloads itself
    # do not run on NixOS without nix-ld
    yt-dlp
    ffmpeg
    deno
  ];

  buildInputs = with pkgs; [
    # Tauri 2 / WebKitGTK
    at-spi2-atk
    atkmm
    cairo
    gdk-pixbuf
    glib
    gtk3
    harfbuzz
    librsvg
    libsoup_3
    pango
    webkitgtk_4_1

    # Audio output (cpal via rodio)
    alsa-lib

    # Video playback inside the webview (onboarding videos)
    gst_all_1.gstreamer
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
  ];

  shellHook = ''
    # GSettings schemas, otherwise the folder picker dialog crashes
    export XDG_DATA_DIRS=${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:$XDG_DATA_DIRS
    # Blank window on NVIDIA / some Wayland setups
    export WEBKIT_DISABLE_DMABUF_RENDERER=1
  '';
}
