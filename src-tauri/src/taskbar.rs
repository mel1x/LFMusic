//! Windows taskbar integration:
//!  * thumbnail buttons (shuffle / prev / play-pause / next / repeat) via ITaskbarList3;
//!  * replacing the live window preview with track covers (prev | current | next)
//!    via DWM iconic bitmap.
//!
//! No-op on non-Windows platforms (see stubs at the end of the file).

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::collections::hash_map::DefaultHasher;
    use std::ffi::c_void;
    use std::hash::{Hash, Hasher};
    use std::sync::atomic::{
        AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicU64, Ordering,
    };
    use std::sync::{Condvar, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use image::RgbaImage;
    use tauri::{AppHandle, Emitter, Manager};

    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{
        DwmInvalidateIconicBitmaps, DwmSetIconicLivePreviewBitmap, DwmSetIconicThumbnail,
        DwmSetWindowAttribute, DWMWA_FORCE_ICONIC_REPRESENTATION, DWMWA_HAS_ICONIC_BITMAP,
    };
    use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};

    use windows::Win32::Graphics::Gdi::{
        CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW, DeleteDC,
        DeleteObject, DrawTextW, GdiFlush, GetDC, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
        BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY, DIB_RGB_COLORS, DT_CENTER, DT_NOPREFIX,
        DT_SINGLELINE, DT_VCENTER, HBITMAP, HGDIOBJ, LOGFONTW, TRANSPARENT,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        DefSubclassProc, ITaskbarList3, SetWindowSubclass, TaskbarList, THB_FLAGS, THB_ICON,
        THB_TOOLTIP, THBF_ENABLED, THUMBBUTTON, THUMBBUTTONMASK,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIconIndirect, GetClientRect, IsIconic, RegisterWindowMessageW, HICON, ICONINFO,
        PW_RENDERFULLCONTENT, WM_COMMAND,
    };

    const ID_SHUFFLE: u32 = 1;
    const ID_PREV: u32 = 2;
    const ID_PLAYPAUSE: u32 = 3;
    const ID_NEXT: u32 = 4;
    const ID_REPEAT: u32 = 5;

    // Window messages not exposed by the `windows` crate.
    const WM_DWMSENDICONICTHUMBNAIL: u32 = 0x0323;
    const WM_DWMSENDICONICLIVEPREVIEW: u32 = 0x0326;
    const THBN_CLICKED: u32 = 0x1800;
    const SUBCLASS_ID: usize = 0x4C46_4D31; // "LFM1"
    const TRANS_MS: f32 = 170.0; // cover crossfade/slide duration
    const SCALE_MS: f32 = 180.0; // play/pause size-animation duration
    // Bottom padding of the preview (fraction of height). All three covers are
    // top-aligned (y=0); this leaves a small gap at the bottom.
    const BOTTOM_PAD_FRAC: f32 = 0.07;
    // While playing, the center cover grows by exactly the bottom pad to fill
    // the preview height (s × PLAY_SCALE = ch); on pause it matches the side covers.
    const PLAY_SCALE: f32 = 1.0 / (1.0 - BOTTOM_PAD_FRAC);

    // Segoe MDL2 Assets glyphs.
    const GLYPH_SHUFFLE: u16 = 0xE8B1;
    const GLYPH_REPEAT_ALL: u16 = 0xE8EE;
    const GLYPH_REPEAT_ONE: u16 = 0xE8ED;

    const ICON_SIZE: i32 = 32;
    const LEVEL_BRIGHT: u8 = 255;
    const LEVEL_DIM: u8 = 160;
    // Working cover resolution frames are rendered from (not the original).
    // High enough that even the enlarged center cover (×PLAY_SCALE) downscales
    // on large thumbnails instead of upscaling and blurring.
    const WORK_SIZE: u32 = 384;

    // Fallback cover for tracks without art, matching the frontend's default
    // (src/assets/trackCover.webp). Keeps neighbor slots (e.g. "prev" wrapping
    // to the last track) from going empty and the preview from falling back to
    // a plain window when the current track has no cover.
    const DEFAULT_COVER: &[u8] = include_bytes!("../../src/assets/trackCover.webp");

    static APP: OnceLock<AppHandle> = OnceLock::new();
    static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
    static PLAYING: AtomicBool = AtomicBool::new(false);
    static SHUFFLED: AtomicBool = AtomicBool::new(false);
    static REPEAT_TRACK: AtomicBool = AtomicBool::new(false);
    static TASKBAR_MSG: AtomicU32 = AtomicU32::new(0);

    // Prepared "master" cover: WORK_SIZE×WORK_SIZE, corners already rounded.
    type Img = RgbaImage;
    struct Covers {
        prev: Option<Img>,
        current: Option<Img>,
        next: Option<Img>,
    }
    static COVERS: Mutex<Covers> = Mutex::new(Covers {
        prev: None,
        current: None,
        next: None,
    });
    // Previous cover set, kept for crossfade/slide transitions.
    static OLD_COVERS: Mutex<Covers> = Mutex::new(Covers {
        prev: None,
        current: None,
        next: None,
    });

    #[derive(Clone, Copy, PartialEq)]
    enum Anim {
        None,
        Fade,
        SlideFwd,
        SlideBack,
    }
    // (transition kind, start time).
    static TRANSITION: Mutex<(Anim, Option<Instant>)> = Mutex::new((Anim::None, None));

    // Center-cover size animation for play/pause: (start, from, to).
    struct ScaleAnim {
        start: Option<Instant>,
        from: f32,
        to: f32,
    }
    static SCALE: Mutex<ScaleAnim> = Mutex::new(ScaleAnim {
        start: None,
        from: 1.0,
        to: 1.0,
    });

    // Size of the last DWM-requested thumbnail, so animation frames can be pushed at the same size.
    static LAST_W: AtomicI32 = AtomicI32::new(0);
    static LAST_H: AtomicI32 = AtomicI32::new(0);

    static WAKE: (Mutex<()>, Condvar) = (Mutex::new(()), Condvar::new());

    // COM interface and HICON aren't Send — kept in main-thread TLS.
    struct Icons {
        shuffle_dim: HICON,
        shuffle_on: HICON,
        prev: HICON,
        play: HICON,
        pause: HICON,
        next: HICON,
        repeat_all: HICON,
        repeat_one: HICON,
    }
    thread_local! {
        static TASKBAR: RefCell<Option<ITaskbarList3>> = const { RefCell::new(None) };
        static ICONS: RefCell<Option<Icons>> = const { RefCell::new(None) };
    }

    fn hwnd() -> HWND {
        HWND(HWND_RAW.load(Ordering::Relaxed) as *mut c_void)
    }

    #[derive(Clone, Copy, Debug)]
    enum SolidIcon {
        Prev,
        Play,
        Pause,
        Next,
    }

    #[derive(Clone, Copy)]
    struct Point {
        x: f32,
        y: f32,
    }

    fn rect_contains(p: Point, left: f32, top: f32, right: f32, bottom: f32) -> bool {
        p.x >= left && p.x <= right && p.y >= top && p.y <= bottom
    }

    fn triangle_contains(p: Point, a: Point, b: Point, c: Point) -> bool {
        let edge = |p1: Point, p2: Point, p3: Point| {
            (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y)
        };
        let d1 = edge(p, a, b);
        let d2 = edge(p, b, c);
        let d3 = edge(p, c, a);
        let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(has_neg && has_pos)
    }

    fn solid_transport_contains(kind: SolidIcon, p: Point) -> bool {
        const D: f32 = 32.0;
        let pt = |x: f32, y: f32| Point { x: x / D, y: y / D };

        match kind {
            SolidIcon::Play => {
                triangle_contains(p, pt(10.0, 7.0), pt(10.0, 25.0), pt(24.0, 16.0))
            }
            SolidIcon::Pause => {
                rect_contains(p, 10.0 / D, 7.0 / D, 14.0 / D, 25.0 / D)
                    || rect_contains(p, 18.0 / D, 7.0 / D, 22.0 / D, 25.0 / D)
            }
            SolidIcon::Next => {
                triangle_contains(p, pt(8.0, 7.0), pt(8.0, 25.0), pt(20.0, 16.0))
                    || rect_contains(p, 22.0 / D, 7.0 / D, 25.0 / D, 25.0 / D)
            }
            SolidIcon::Prev => {
                triangle_contains(p, pt(24.0, 7.0), pt(24.0, 25.0), pt(12.0, 16.0))
                    || rect_contains(p, 7.0 / D, 7.0 / D, 10.0 / D, 25.0 / D)
            }
        }
    }

    fn rasterize_solid_transport_icon(kind: SolidIcon, level: u8, size: i32) -> Vec<u8> {
        if size <= 0 {
            return Vec::new();
        }

        const AA: i32 = 4;
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        let size_f = size as f32;

        for y in 0..size {
            for x in 0..size {
                let mut covered = 0;
                for sy in 0..AA {
                    for sx in 0..AA {
                        let p = Point {
                            x: (x as f32 + (sx as f32 + 0.5) / AA as f32) / size_f,
                            y: (y as f32 + (sy as f32 + 0.5) / AA as f32) / size_f,
                        };
                        if solid_transport_contains(kind, p) {
                            covered += 1;
                        }
                    }
                }

                let alpha = (covered * 255 / (AA * AA)) as u8;
                let color = (alpha as u16 * level as u16 / 255) as u8;
                let offset = ((y * size + x) * 4) as usize;
                pixels[offset] = color;
                pixels[offset + 1] = color;
                pixels[offset + 2] = color;
                pixels[offset + 3] = alpha;
            }
        }

        pixels
    }

    unsafe fn create_icon_from_bgra(size: i32, pixels: &[u8]) -> Option<HICON> {
        if size <= 0 || pixels.len() != (size * size * 4) as usize {
            return None;
        }

        let memdc = CreateCompatibleDC(None);
        if memdc.is_invalid() {
            return None;
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size, // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        let hbm = CreateDIBSection(Some(memdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok();
        let result = (|| {
            let hbm = hbm?;
            if bits.is_null() {
                return None;
            }

            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
            let mask = CreateBitmap(size, size, 1, 1, None);
            let ii = ICONINFO {
                fIcon: true.into(),
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask,
                hbmColor: hbm,
            };
            let icon = CreateIconIndirect(&ii).ok();
            let _ = DeleteObject(HGDIOBJ(mask.0));
            icon
        })();

        if let Some(hbm) = hbm {
            let _ = DeleteObject(HGDIOBJ(hbm.0));
        }
        let _ = DeleteDC(memdc);
        result
    }

    unsafe fn render_solid_transport_icon(kind: SolidIcon, level: u8) -> Option<HICON> {
        let pixels = rasterize_solid_transport_icon(kind, level, ICON_SIZE);
        create_icon_from_bgra(ICON_SIZE, &pixels)
    }

    /// Renders a font glyph to an HICON (32bpp with alpha). `level` controls
    /// brightness (255 = white, lower = dimmed gray).
    unsafe fn render_glyph_icon(glyph: u16, level: u8) -> Option<HICON> {
        let size = ICON_SIZE;
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let memdc = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let hbm = CreateDIBSection(Some(memdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok();
        let result = (|| {
            let hbm = hbm?;
            if bits.is_null() {
                return None;
            }
            let old_bmp = SelectObject(memdc, HGDIOBJ(hbm.0));

            let mut lf = LOGFONTW {
                lfHeight: -(size * 7 / 10),
                lfWeight: 400,
                lfQuality: CLEARTYPE_QUALITY,
                ..Default::default()
            };
            let face: Vec<u16> = "Segoe MDL2 Assets".encode_utf16().collect();
            for (i, c) in face.iter().enumerate().take(lf.lfFaceName.len() - 1) {
                lf.lfFaceName[i] = *c;
            }
            let font = CreateFontIndirectW(&lf);
            let old_font = SelectObject(memdc, HGDIOBJ(font.0));

            SetBkMode(memdc, TRANSPARENT);
            let _ = SetTextColor(memdc, COLORREF(0x00FF_FFFF));
            let mut rect = RECT { left: 0, top: 0, right: size, bottom: size };
            let mut text = [glyph];
            DrawTextW(
                memdc,
                &mut text,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
            let _ = GdiFlush();

            // Alpha from brightness (white text on black bg); color dimmed by
            // `level`, premultiplied.
            let px = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
            for chunk in px.as_chunks_mut::<4>().0 {
                let a = chunk[0].max(chunk[1]).max(chunk[2]); // BGRA
                let c = (a as u16 * level as u16 / 255) as u8;
                chunk[0] = c;
                chunk[1] = c;
                chunk[2] = c;
                chunk[3] = a;
            }

            let mask = CreateBitmap(size, size, 1, 1, None);
            let ii = ICONINFO {
                fIcon: true.into(),
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask,
                hbmColor: hbm,
            };
            let icon = CreateIconIndirect(&ii).ok();

            SelectObject(memdc, old_font);
            SelectObject(memdc, old_bmp);
            let _ = DeleteObject(HGDIOBJ(font.0));
            let _ = DeleteObject(HGDIOBJ(mask.0));
            icon
        })();

        if let Some(hbm) = hbm {
            let _ = DeleteObject(HGDIOBJ(hbm.0));
        }
        let _ = DeleteDC(memdc);
        ReleaseDC(None, screen);
        result
    }

    fn with_icons<R>(f: impl FnOnce(&Icons) -> R) -> Option<R> {
        ICONS.with(|cell| {
            if cell.borrow().is_none() {
                let icons = unsafe {
                    Some(Icons {
                        shuffle_dim: render_glyph_icon(GLYPH_SHUFFLE, LEVEL_DIM)?,
                        shuffle_on: render_glyph_icon(GLYPH_SHUFFLE, LEVEL_BRIGHT)?,
                        prev: render_solid_transport_icon(SolidIcon::Prev, LEVEL_BRIGHT)?,
                        play: render_solid_transport_icon(SolidIcon::Play, LEVEL_BRIGHT)?,
                        pause: render_solid_transport_icon(SolidIcon::Pause, LEVEL_BRIGHT)?,
                        next: render_solid_transport_icon(SolidIcon::Next, LEVEL_BRIGHT)?,
                        repeat_all: render_glyph_icon(GLYPH_REPEAT_ALL, LEVEL_DIM)?,
                        repeat_one: render_glyph_icon(GLYPH_REPEAT_ONE, LEVEL_BRIGHT)?,
                    })
                };
                *cell.borrow_mut() = icons;
            }
            cell.borrow().as_ref().map(f)
        })
    }

    fn with_taskbar<R>(f: impl FnOnce(&ITaskbarList3) -> R) -> Option<R> {
        TASKBAR.with(|cell| {
            if cell.borrow().is_none() {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                    let tb: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_ALL).ok()?;
                    tb.HrInit().ok()?;
                    *cell.borrow_mut() = Some(tb);
                }
            }
            cell.borrow().as_ref().map(f)
        })
    }

    fn make_button(id: u32, icon: HICON, tip: &str) -> THUMBBUTTON {
        let mut btn = THUMBBUTTON {
            dwMask: THUMBBUTTONMASK(THB_ICON.0 | THB_TOOLTIP.0 | THB_FLAGS.0),
            iId: id,
            hIcon: icon,
            dwFlags: THBF_ENABLED,
            ..Default::default()
        };
        for (i, c) in tip.encode_utf16().enumerate().take(btn.szTip.len() - 1) {
            btn.szTip[i] = c;
        }
        btn
    }

    /// Builds the 5 buttons from current state (play/pause, shuffle, repeat).
    fn build_buttons() -> Option<[THUMBBUTTON; 5]> {
        let playing = PLAYING.load(Ordering::Relaxed);
        let shuffled = SHUFFLED.load(Ordering::Relaxed);
        let repeat_track = REPEAT_TRACK.load(Ordering::Relaxed);
        with_icons(|icons| {
            let pp = if playing { icons.pause } else { icons.play };
            let sh = if shuffled { icons.shuffle_on } else { icons.shuffle_dim };
            let rp = if repeat_track { icons.repeat_one } else { icons.repeat_all };
            [
                make_button(ID_SHUFFLE, sh, "Перемешать"),
                make_button(ID_PREV, icons.prev, "Предыдущий"),
                make_button(ID_PLAYPAUSE, pp, if playing { "Пауза" } else { "Воспроизвести" }),
                make_button(ID_NEXT, icons.next, "Следующий"),
                make_button(
                    ID_REPEAT,
                    rp,
                    if repeat_track { "Повтор трека" } else { "Повтор плейлиста" },
                ),
            ]
        })
    }

    /// (Re)creates the thumbnail button set. Must run on the main thread.
    fn setup_thumbbar() {
        let h = hwnd();
        if h.0.is_null() {
            return;
        }
        let Some(buttons) = build_buttons() else { return };
        with_taskbar(|tb| unsafe {
            let _ = tb.ThumbBarAddButtons(h, &buttons);
        });
    }

    /// Updates button icons/tooltips without recreating them. Main thread only.
    fn update_buttons() {
        let h = hwnd();
        if h.0.is_null() {
            return;
        }
        let Some(buttons) = build_buttons() else { return };
        with_taskbar(|tb| unsafe {
            let _ = tb.ThumbBarUpdateButtons(h, &buttons);
        });
    }

    /// Rounds the corners of a square cover (SDF rounded-box alpha mask, antialiased).
    fn round_corners(img: &mut RgbaImage, r: f32) {
        let w = img.width() as f32;
        let h = img.height() as f32;
        let (hw, hh) = (w / 2.0, h / 2.0);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let qx = (x as f32 + 0.5 - hw).abs() - (hw - r);
            let qy = (y as f32 + 0.5 - hh).abs() - (hh - r);
            let dx = qx.max(0.0);
            let dy = qy.max(0.0);
            let sdf = (dx * dx + dy * dy).sqrt() + qx.max(qy).min(0.0) - r;
            let cov = (0.5 - sdf).clamp(0.0, 1.0);
            if cov < 1.0 {
                p.0[3] = (p.0[3] as f32 * cov).round() as u8;
            }
        }
    }

    /// Resizes the master cover to a `side`×`side` square. Corner rounding is
    /// already baked into the master, so each animation frame is just one cheap resize.
    fn prep_cover(img: &Img, side: u32) -> RgbaImage {
        image::imageops::resize(img, side, side, image::imageops::FilterType::CatmullRom)
    }

    /// Smoothstep ease (slows at both ends).
    fn ease(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Layout geometry: (base cover side `s`, step between slot centers).
    fn layout(ch: u32) -> (f32, f32) {
        let s = ((ch as f32) * (1.0 - BOTTOM_PAD_FRAC)).max(1.0);
        let hgap = ((ch as f32) * 0.1).max(2.0);
        (s, s + hgap)
    }

    /// Draws one cover at slot position `slot` (0 = center, ±1 = neighbors).
    /// Covers scale up toward the center (up to `play_scale`) as a play indicator.
    fn draw_slot(
        canvas: &mut RgbaImage,
        cover: &Option<Img>,
        slot: f32,
        step: f32,
        s: f32,
        cw: u32,
        ch: u32,
        play_scale: f32,
    ) {
        let Some(src) = cover else { return };
        let closeness = (1.0 - slot.abs()).clamp(0.0, 1.0);
        let scale = 1.0 + (play_scale - 1.0) * closeness;
        let size = (s * scale).round().clamp(1.0, ch as f32) as u32;
        let img = prep_cover(src, size);
        let xc = cw as f32 / 2.0 + slot * step;
        let x = (xc - size as f32 / 2.0).round() as i64;
        image::imageops::overlay(canvas, &img, x, 0);
    }

    /// Static "prev | current | next" layout.
    fn compose(c: &Covers, cw: u32, ch: u32, play_scale: f32) -> RgbaImage {
        let mut canvas = RgbaImage::from_pixel(cw, ch, image::Rgba([0, 0, 0, 0]));
        let (s, step) = layout(ch);
        // Draw neighbors first so the (possibly larger) center cover ends up on top.
        draw_slot(&mut canvas, &c.prev, -1.0, step, s, cw, ch, play_scale);
        draw_slot(&mut canvas, &c.next, 1.0, step, s, cw, ch, play_scale);
        draw_slot(&mut canvas, &c.current, 0.0, step, s, cw, ch, play_scale);
        canvas
    }

    /// Slide transition: covers shift left (forward) or right (backward) by
    /// one step; the cover passing through center scales up smoothly.
    fn compose_slide(
        old: &Covers,
        new: &Covers,
        forward: bool,
        t: f32,
        cw: u32,
        ch: u32,
        play_scale: f32,
    ) -> RgbaImage {
        let mut canvas = RgbaImage::from_pixel(cw, ch, image::Rgba([0, 0, 0, 0]));
        let (s, step) = layout(ch);
        let shift = if forward { -t } else { t };

        // (cover, slot at t=0). The 4th cover enters from off-screen.
        let items: [(&Option<Img>, f32); 4] = if forward {
            [
                (&old.prev, -1.0),
                (&old.current, 0.0),
                (&old.next, 1.0),
                (&new.next, 2.0),
            ]
        } else {
            [
                (&new.prev, -2.0),
                (&old.prev, -1.0),
                (&old.current, 0.0),
                (&old.next, 1.0),
            ]
        };

        // Draw farthest-to-nearest so the center cover ends up on top.
        let mut order: [usize; 4] = [0, 1, 2, 3];
        order.sort_by(|&a, &b| {
            let pa = (items[a].1 + shift).abs();
            let pb = (items[b].1 + shift).abs();
            pb.partial_cmp(&pa).unwrap_or(std::cmp::Ordering::Equal)
        });
        for &i in &order {
            let (cover, slot0) = items[i];
            draw_slot(&mut canvas, cover, slot0 + shift, step, s, cw, ch, play_scale);
        }
        canvas
    }

    /// Current transition: (kind, t∈[0;1]).
    fn transition_state() -> (Anim, f32) {
        if let Ok(g) = TRANSITION.lock() {
            let (kind, start) = *g;
            if let Some(t0) = start {
                let e = t0.elapsed().as_millis() as f32;
                if e < TRANS_MS {
                    return (kind, (e / TRANS_MS).clamp(0.0, 1.0));
                }
            }
        }
        (Anim::None, 1.0)
    }

    fn transition_active() -> bool {
        !matches!(transition_state().0, Anim::None)
    }

    fn scale_value(s: &ScaleAnim) -> f32 {
        if let Some(t0) = s.start {
            let e = t0.elapsed().as_millis() as f32;
            if e < SCALE_MS {
                return s.from + (s.to - s.from) * ease(e / SCALE_MS);
            }
        }
        s.to
    }

    fn play_scale_now() -> f32 {
        if let Ok(s) = SCALE.lock() {
            scale_value(&s)
        } else {
            1.0
        }
    }

    fn scale_active() -> bool {
        if let Ok(s) = SCALE.lock() {
            if let Some(t0) = s.start {
                return (t0.elapsed().as_millis() as f32) < SCALE_MS;
            }
        }
        false
    }

    fn any_anim_active() -> bool {
        transition_active() || scale_active()
    }

    /// Builds a thumbnail frame accounting for the active transition and center-cover scale.
    fn make_thumbnail(max_w: i32, max_h: i32) -> Option<HBITMAP> {
        let cw = max_w.max(1) as u32;
        let ch = max_h.max(1) as u32;
        let ps = play_scale_now();
        let (kind, raw_t) = transition_state();
        let t = ease(raw_t);

        match kind {
            Anim::SlideFwd | Anim::SlideBack => {
                let new = COVERS.lock().ok()?;
                new.current.as_ref()?;
                let old = OLD_COVERS.lock().ok()?;
                let forward = kind == Anim::SlideFwd;
                let img = compose_slide(&old, &new, forward, t, cw, ch, ps);
                unsafe { blend_to_hbitmap(&img, None, 1.0) }
            }
            Anim::Fade => {
                let new = COVERS.lock().ok()?;
                new.current.as_ref()?;
                let ni = compose(&new, cw, ch, ps);
                let old = OLD_COVERS.lock().ok()?;
                let oi = if old.current.is_some() {
                    Some(compose(&old, cw, ch, ps))
                } else {
                    None
                };
                unsafe { blend_to_hbitmap(&ni, oi.as_ref(), t) }
            }
            Anim::None => {
                let new = COVERS.lock().ok()?;
                new.current.as_ref()?;
                let img = compose(&new, cw, ch, ps);
                unsafe { blend_to_hbitmap(&img, None, 1.0) }
            }
        }
    }

    /// Builds an HBITMAP (premultiplied BGRA), blending with the old frame by
    /// `t` in premultiplied space (for a correct crossfade) if given.
    unsafe fn blend_to_hbitmap(newi: &RgbaImage, oldi: Option<&RgbaImage>, t: f32) -> Option<HBITMAP> {
        let (w, h) = (newi.width() as i32, newi.height() as i32);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let hbm = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        if bits.is_null() {
            let _ = DeleteObject(HGDIOBJ(hbm.0));
            return None;
        }
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
        let np = newi.as_raw();
        // Premultiplies one RGBA pixel to (b, g, r, a) with weight w∈[0;1].
        let pm = |s: &[u8], o: usize, weight: f32| -> (f32, f32, f32, f32) {
            let a = s[o + 3] as f32 / 255.0;
            (
                s[o + 2] as f32 * a * weight, // B
                s[o + 1] as f32 * a * weight, // G
                s[o] as f32 * a * weight,     // R
                s[o + 3] as f32 * weight,     // A
            )
        };
        for i in 0..(w * h) as usize {
            let o = i * 4;
            let (b, g, r, a) = if let Some(oldi) = oldi {
                let op = oldi.as_raw();
                let (nb, ng, nr, na) = pm(np, o, t);
                let (ob, og, or_, oa) = pm(op, o, 1.0 - t);
                (nb + ob, ng + og, nr + or_, na + oa)
            } else {
                pm(np, o, 1.0)
            };
            dst[o] = b as u8;
            dst[o + 1] = g as u8;
            dst[o + 2] = r as u8;
            dst[o + 3] = a as u8;
        }
        Some(hbm)
    }

    fn set_force_iconic(on: bool) {
        let h = hwnd();
        if h.0.is_null() {
            return;
        }
        let val: i32 = if on { 1 } else { 0 };
        unsafe {
            let _ = DwmSetWindowAttribute(
                h,
                DWMWA_FORCE_ICONIC_REPRESENTATION,
                &val as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }
    }

    /// Captures the real window (incl. WebView2 content) for Aero Peek via
    /// `PrintWindow(PW_RENDERFULLCONTENT)`. Returns `None` if minimized or the
    /// capture fails, so the caller falls back to covers.
    unsafe fn capture_window(h: HWND) -> Option<HBITMAP> {
        if IsIconic(h).as_bool() {
            return None; // a minimized window has no content to capture
        }
        let mut rc = RECT::default();
        GetClientRect(h, &mut rc).ok()?;
        let w = (rc.right - rc.left).max(1);
        let ht = (rc.bottom - rc.top).max(1);

        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let memdc = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -ht, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let hbm = CreateDIBSection(Some(memdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok();
        let result = (|| {
            let hbm = hbm?;
            if bits.is_null() {
                return None;
            }
            let old = SelectObject(memdc, HGDIOBJ(hbm.0));
            // PW_RENDERFULLCONTENT also captures WebView2 (DComp) content.
            let ok = PrintWindow(h, memdc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
            SelectObject(memdc, old);
            if !ok {
                return None;
            }
            // GDI doesn't set alpha — force opaque or DWM may show a transparent/black frame.
            let px = std::slice::from_raw_parts_mut(bits as *mut u8, (w * ht * 4) as usize);
            for chunk in px.as_chunks_mut::<4>().0 {
                chunk[3] = 255;
            }
            Some(hbm)
        })();

        if result.is_none() {
            if let Some(hbm) = hbm {
                let _ = DeleteObject(HGDIOBJ(hbm.0));
            }
        }
        let _ = DeleteDC(memdc);
        ReleaseDC(None, screen);
        result
    }

    unsafe extern "system" fn subclass_proc(
        h: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        match msg {
            WM_COMMAND => {
                let notify = ((wparam.0 >> 16) & 0xFFFF) as u32;
                if notify == THBN_CLICKED {
                    let id = (wparam.0 & 0xFFFF) as u32;
                    let action = match id {
                        ID_SHUFFLE => Some("shuffle"),
                        ID_PREV => Some("prev"),
                        ID_PLAYPAUSE => Some("playpause"),
                        ID_NEXT => Some("next"),
                        ID_REPEAT => Some("repeat"),
                        _ => None,
                    };
                    if let Some(action) = action {
                        if let Some(app) = APP.get() {
                            let _ = app.emit("taskbar_media", action);
                        }
                        return LRESULT(0);
                    }
                }
            }
            WM_DWMSENDICONICTHUMBNAIL => {
                let max_w = ((lparam.0 >> 16) & 0xFFFF) as i32;
                let max_h = (lparam.0 & 0xFFFF) as i32;
                LAST_W.store(max_w, Ordering::Relaxed);
                LAST_H.store(max_h, Ordering::Relaxed);
                if let Some(hbm) = make_thumbnail(max_w, max_h) {
                    let _ = DwmSetIconicThumbnail(h, hbm, Default::default());
                    let _ = DeleteObject(HGDIOBJ(hbm.0));
                }
                return LRESULT(0);
            }
            WM_DWMSENDICONICLIVEPREVIEW => {
                // Aero Peek shows the real window, not enlarged covers; fall
                // back to covers if minimized or capture fails.
                let hbm = capture_window(h).or_else(|| make_thumbnail(640, 400));
                if let Some(hbm) = hbm {
                    let _ = DwmSetIconicLivePreviewBitmap(h, hbm, None, Default::default());
                    let _ = DeleteObject(HGDIOBJ(hbm.0));
                }
                return LRESULT(0);
            }
            _ => {
                if msg == TASKBAR_MSG.load(Ordering::Relaxed) && msg != 0 {
                    setup_thumbbar();
                }
            }
        }
        DefSubclassProc(h, msg, wparam, lparam)
    }

    // Renders and pushes frames directly to DWM (~60fps), bypassing tao's
    // message loop and the main thread — pumping via WM_TIMER was capped at ~5fps.

    fn push_frame() {
        let (w, h) = (LAST_W.load(Ordering::Relaxed), LAST_H.load(Ordering::Relaxed));
        if w <= 0 || h <= 0 {
            return;
        }
        let hwndv = hwnd();
        if hwndv.0.is_null() {
            return;
        }
        unsafe {
            if let Some(hbm) = make_thumbnail(w, h) {
                let _ = DwmSetIconicThumbnail(hwndv, hbm, Default::default());
                let _ = DeleteObject(HGDIOBJ(hbm.0));
            }
        }
    }

    fn spawn_animator() {
        std::thread::spawn(|| loop {
            // Sleep on the condvar until an animation starts.
            {
                let mut g = WAKE.0.lock().unwrap();
                while !any_anim_active() {
                    // Timeout guards against a missed notify.
                    g = WAKE.1.wait_timeout(g, Duration::from_millis(250)).unwrap().0;
                }
            }
            // Raise timer resolution during animation: default sleep quantizes
            // to ~15.6ms, giving ~30fps and jitter instead of a smooth 60fps.
            unsafe {
                let _ = timeBeginPeriod(1);
            }
            // Frame pacing uses absolute deadlines so sleep jitter doesn't accumulate.
            let frame = Duration::from_micros(16_667);
            let mut next = Instant::now();
            while any_anim_active() {
                push_frame();
                next += frame;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now; // fell behind — skip ahead instead of bursting frames
                }
            }
            push_frame(); // final settled frame
            unsafe {
                let _ = timeEndPeriod(1);
            }
        });
    }

    fn wake_animator() {
        WAKE.1.notify_one();
    }

    pub fn init(app: &AppHandle, hwnd_raw: isize) {
        let _ = APP.set(app.clone());
        HWND_RAW.store(hwnd_raw, Ordering::Relaxed);
        let h = HWND(hwnd_raw as *mut c_void);
        unsafe {
            let msg = RegisterWindowMessageW(w!("TaskbarButtonCreated"));
            TASKBAR_MSG.store(msg, Ordering::Relaxed);

            let _ = SetWindowSubclass(h, Some(subclass_proc), SUBCLASS_ID, 0);

            // Enables iconic bitmap support (covers instead of the default preview).
            let yes: i32 = 1;
            let _ = DwmSetWindowAttribute(
                h,
                DWMWA_HAS_ICONIC_BITMAP,
                &yes as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }
        setup_thumbbar();
        spawn_animator();
    }

    pub fn set_playing(app: &AppHandle, playing: bool) {
        PLAYING.store(playing, Ordering::Relaxed);

        let target = if playing { PLAY_SCALE } else { 1.0 };
        let mut animate = false;
        if let Ok(mut sc) = SCALE.lock() {
            if (sc.to - target).abs() > 0.001 {
                sc.from = scale_value(&sc);
                sc.to = target;
                sc.start = Some(Instant::now());
                animate = true;
            }
        }

        if let Some(window) = app.get_webview_window("main") {
            let _ = window.run_on_main_thread(update_buttons);
        }
        if animate {
            wake_animator();
        }
    }

    pub fn set_modes(app: &AppHandle, shuffled: bool, repeat_track: bool) {
        SHUFFLED.store(shuffled, Ordering::Relaxed);
        REPEAT_TRACK.store(repeat_track, Ordering::Relaxed);
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.run_on_main_thread(update_buttons);
        }
    }

    // Signature of the last cover set, to skip re-decoding/re-animating when
    // the frontend resends the same set.
    static LAST_COVERS_SIG: AtomicU64 = AtomicU64::new(0);

    pub fn set_covers(
        app: &AppHandle,
        prev: Option<String>,
        current: Option<String>,
        next: Option<String>,
        transition: String,
    ) {
        let sig = {
            let mut h = DefaultHasher::new();
            prev.hash(&mut h);
            current.hash(&mut h);
            next.hash(&mut h);
            h.finish().max(1) // 0 is reserved for "no calls yet"
        };
        if LAST_COVERS_SIG.swap(sig, Ordering::Relaxed) == sig {
            return;
        }

        let decode = |file_path: Option<String>| -> Option<Img> {
            let file_path = file_path?;
            // Covers come from the WebP thumbnail cache (generated on demand),
            // avoiding base64 over IPC; fall back to the default image if none.
            let bytes = crate::thumbnails::cover_bytes(app, &file_path, 512)
                .unwrap_or_else(|| DEFAULT_COVER.to_vec());
            let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
            // Prepare the "master" once (working size + rounded corners); animation
            // frames only resize it, since full-res resize + SDF rounding per frame is too slow.
            let mut img = image::imageops::resize(
                &img,
                WORK_SIZE,
                WORK_SIZE,
                image::imageops::FilterType::Lanczos3,
            );
            round_corners(&mut img, WORK_SIZE as f32 * 0.09);
            Some(img)
        };
        // Whether a track is playing at all (regardless of cover), for force-iconic.
        let has_current_track = current.is_some();
        let new = Covers {
            prev: decode(prev),
            current: decode(current),
            next: decode(next),
        };
        let has_current_cover = new.current.is_some();
        let kind = match transition.as_str() {
            "forward" => Anim::SlideFwd,
            "backward" => Anim::SlideBack,
            "fade" => Anim::Fade,
            _ => Anim::None,
        };

        // Current set becomes "old"; start a transition if there was something to transition from.
        let mut animate = false;
        if let (Ok(mut cur), Ok(mut old), Ok(mut tr)) =
            (COVERS.lock(), OLD_COVERS.lock(), TRANSITION.lock())
        {
            let had_current = cur.current.is_some();
            *old = std::mem::replace(&mut *cur, new);
            if had_current && has_current_cover && kind != Anim::None {
                *tr = (kind, Some(Instant::now()));
                animate = true;
            } else {
                *tr = (Anim::None, None);
            }
        }

        if let Some(window) = app.get_webview_window("main") {
            let _ = window.run_on_main_thread(move || {
                set_force_iconic(has_current_track);
                let h = hwnd();
                if !h.0.is_null() {
                    unsafe {
                        let _ = DwmInvalidateIconicBitmaps(h);
                    }
                }
            });
        }
        if animate {
            wake_animator();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn transport_icon_masks_are_solid_white_shapes() {
            for kind in [
                SolidIcon::Prev,
                SolidIcon::Play,
                SolidIcon::Pause,
                SolidIcon::Next,
            ] {
                let pixels = rasterize_solid_transport_icon(kind, LEVEL_BRIGHT, ICON_SIZE);
                let mut opaque = 0usize;
                let mut bright_opaque = 0usize;
                let mut translucent = 0usize;

                for px in pixels.as_chunks::<4>().0 {
                    let alpha = px[3];
                    if alpha == 255 {
                        opaque += 1;
                        if px[0] == 255 && px[1] == 255 && px[2] == 255 {
                            bright_opaque += 1;
                        }
                    } else if alpha > 0 {
                        translucent += 1;
                    }
                }

                assert!(opaque > 80, "{kind:?} should have a filled body");
                assert_eq!(
                    bright_opaque, opaque,
                    "{kind:?} opaque pixels should be solid white"
                );
                assert!(
                    translucent < opaque / 2,
                    "{kind:?} should not render as a mostly outlined glyph"
                );
            }
        }

        #[test]
        fn rect_contains_includes_boundary_and_excludes_outside() {
            assert!(rect_contains(Point { x: 0.0, y: 0.0 }, 0.0, 0.0, 1.0, 1.0));
            assert!(rect_contains(Point { x: 1.0, y: 1.0 }, 0.0, 0.0, 1.0, 1.0));
            assert!(rect_contains(Point { x: 0.5, y: 0.5 }, 0.0, 0.0, 1.0, 1.0));
            assert!(!rect_contains(Point { x: 1.01, y: 0.5 }, 0.0, 0.0, 1.0, 1.0));
            assert!(!rect_contains(Point { x: -0.01, y: 0.5 }, 0.0, 0.0, 1.0, 1.0));
        }

        #[test]
        fn triangle_contains_vertices_and_centroid_but_not_far_outside() {
            let a = Point { x: 0.0, y: 0.0 };
            let b = Point { x: 4.0, y: 0.0 };
            let c = Point { x: 0.0, y: 4.0 };
            let centroid = Point { x: 4.0 / 3.0, y: 4.0 / 3.0 };
            assert!(triangle_contains(centroid, a, b, c));
            assert!(triangle_contains(a, a, b, c), "вершина считается внутри");
            assert!(!triangle_contains(Point { x: 10.0, y: 10.0 }, a, b, c));
        }

        #[test]
        fn ease_is_clamped_and_passes_through_endpoints() {
            assert_eq!(ease(0.0), 0.0);
            assert_eq!(ease(1.0), 1.0);
            assert_eq!(ease(-5.0), 0.0, "должен клэмпиться снизу");
            assert_eq!(ease(5.0), 1.0, "должен клэмпиться сверху");
            // Smoothstep is symmetric around t=0.5: ease(0.5) == 0.5.
            assert!((ease(0.5) - 0.5).abs() < 1e-6);
        }

        #[test]
        fn ease_is_monotonically_non_decreasing_on_unit_interval() {
            let mut prev = ease(0.0);
            for i in 1..=20 {
                let t = i as f32 / 20.0;
                let v = ease(t);
                assert!(v >= prev - 1e-6, "ease должен быть неубывающим: {prev} -> {v} at t={t}");
                prev = v;
            }
        }

        #[test]
        fn layout_step_is_always_larger_than_base_size() {
            for ch in [1u32, 32, 96, 384, 1000] {
                let (s, step) = layout(ch);
                assert!(s > 0.0, "сторона базовой обложки должна быть положительной для ch={ch}");
                assert!(step > s, "между обложками всегда должен быть зазор для ch={ch}");
            }
        }

        #[test]
        fn layout_base_side_matches_bottom_pad_formula() {
            let (s, _) = layout(384);
            let expected = 384.0 * (1.0 - BOTTOM_PAD_FRAC);
            assert!((s - expected).abs() < 0.001);
        }

        #[test]
        fn round_corners_keeps_center_opaque_and_clips_corner() {
            let mut img = RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]));
            round_corners(&mut img, 12.0);

            let center = img.get_pixel(32, 32);
            assert_eq!(center[3], 255, "центр не должен обрезаться скруглением");

            let corner = img.get_pixel(0, 0);
            assert!(corner[3] < 255, "угол должен стать частично/полностью прозрачным");
        }

        #[test]
        fn prep_cover_resizes_to_requested_square_side() {
            let img = RgbaImage::from_pixel(100, 100, image::Rgba([10, 20, 30, 255]));
            let resized = prep_cover(&img, 40);
            assert_eq!(resized.width(), 40);
            assert_eq!(resized.height(), 40);
        }

        #[test]
        fn compose_with_no_covers_is_fully_transparent() {
            let covers = Covers { prev: None, current: None, next: None };
            let canvas = compose(&covers, 200, 100, PLAY_SCALE);
            assert!(canvas.pixels().all(|p| p[3] == 0));
        }

        #[test]
        fn compose_draws_something_when_current_cover_is_set() {
            let solid = RgbaImage::from_pixel(WORK_SIZE, WORK_SIZE, image::Rgba([200, 50, 50, 255]));
            let covers = Covers { prev: None, current: Some(solid), next: None };
            let canvas = compose(&covers, 200, 100, PLAY_SCALE);
            assert!(canvas.pixels().any(|p| p[3] > 0), "текущая обложка должна что-то нарисовать");
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use tauri::AppHandle;
    pub fn init(_app: &AppHandle, _hwnd_raw: isize) {}
    pub fn set_playing(_app: &AppHandle, _playing: bool) {}
    pub fn set_modes(_app: &AppHandle, _shuffled: bool, _repeat_track: bool) {}
    pub fn set_covers(
        _app: &AppHandle,
        _prev: Option<String>,
        _current: Option<String>,
        _next: Option<String>,
        _transition: String,
    ) {
    }
}

pub use imp::{init, set_covers, set_modes, set_playing};
