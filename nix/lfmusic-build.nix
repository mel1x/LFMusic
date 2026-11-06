# NixOS module: system-wide tools and libraries to build LFMusic with a plain
# `pnpm install && pnpm tauri build --no-bundle`, no nix develop / nix-shell.
# Copy it next to your configuration.nix, add it to `imports`, rebuild, re-login.
{ pkgs, lib, ... }:

let
  libs = with pkgs; [
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

  # .pc files of these libraries require their propagated dependencies too
  # (gtk3 -> pango -> harfbuzz -> ...), so collect the whole closure.
  propagated = p: builtins.filter lib.isDerivation
    (lib.flatten ((p.propagatedBuildInputs or [ ]) ++ (p.propagatedNativeBuildInputs or [ ])));
  allLibs = map (x: x.pkg) (builtins.genericClosure {
    startSet = map (p: { key = p.outPath; pkg = p; }) libs;
    operator = x: map (p: { key = p.outPath; pkg = p; }) (propagated x.pkg);
  });

  pcDirs = (map lib.getDev allLibs) ++ allLibs;
in
{
  environment.systemPackages = with pkgs; [
    gcc
    pkg-config
    rustc
    cargo
    nodejs
    pnpm
    # Downloader tools from PATH: binaries the app downloads itself
    # do not run on NixOS without nix-ld
    yt-dlp
    ffmpeg
    deno
  ];

  environment.sessionVariables = {
    PKG_CONFIG_PATH = lib.concatStringsSep ":" [
      (lib.makeSearchPath "lib/pkgconfig" pcDirs)
      (lib.makeSearchPath "share/pkgconfig" pcDirs)
    ];
    # A binary built by plain cargo has no rpath to /nix/store libraries
    LD_LIBRARY_PATH = lib.makeLibraryPath allLibs;
    GST_PLUGIN_SYSTEM_PATH_1_0 = lib.makeSearchPath "lib/gstreamer-1.0" allLibs;
    # Blank window on NVIDIA / some Wayland setups
    WEBKIT_DISABLE_DMABUF_RENDERER = "1";
  };
}
