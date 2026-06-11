//! Интеграция с панелью задач Windows:
//!  * кнопки-миниатюры (shuffle / prev / play-pause / next / repeat) через ITaskbarList3;
//!  * замена «живого» превью окна на обложки треков (предыдущий | текущий | следующий)
//!    через DWM iconic bitmap.
//!
//! Весь модуль — no-op на не-Windows платформах (см. заглушки в конце файла).

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

    use base64::{engine::general_purpose::STANDARD, Engine as _};
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
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateIconIndirect, RegisterWindowMessageW, HICON, ICONINFO, WM_COMMAND,
    };

    // ── Идентификаторы кнопок ────────────────────────────────────────────────
    const ID_SHUFFLE: u32 = 1;
    const ID_PREV: u32 = 2;
    const ID_PLAYPAUSE: u32 = 3;
    const ID_NEXT: u32 = 4;
    const ID_REPEAT: u32 = 5;

    // ── Сообщения окна, не объявленные в крейте ──────────────────────────────
    const WM_DWMSENDICONICTHUMBNAIL: u32 = 0x0323;
    const WM_DWMSENDICONICLIVEPREVIEW: u32 = 0x0326;
    const THBN_CLICKED: u32 = 0x1800;
    const SUBCLASS_ID: usize = 0x4C46_4D31; // "LFM1"
    const TRANS_MS: f32 = 300.0; // длительность кроссфейда/слайда обложек
    const SCALE_MS: f32 = 320.0; // длительность анимации размера при play/pause
    const PLAY_SCALE: f32 = 1.10; // во сколько раз центральная обложка крупнее при игре

    // ── Глифы Segoe MDL2 Assets ──────────────────────────────────────────────
    const GLYPH_SHUFFLE: u16 = 0xE8B1;
    const GLYPH_PREV: u16 = 0xE892;
    const GLYPH_PLAY: u16 = 0xE768;
    const GLYPH_PAUSE: u16 = 0xE769;
    const GLYPH_NEXT: u16 = 0xE893;
    const GLYPH_REPEAT_ALL: u16 = 0xE8EE;
    const GLYPH_REPEAT_ONE: u16 = 0xE8ED;

    const ICON_SIZE: i32 = 32;
    const LEVEL_BRIGHT: u8 = 255;
    const LEVEL_DIM: u8 = 160;
    // Рабочий размер обложек в превью — кадры рендерятся из него, не из оригинала.
    const WORK_SIZE: u32 = 256;

    // ── Глобальное состояние (только Send-данные) ────────────────────────────
    static APP: OnceLock<AppHandle> = OnceLock::new();
    static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
    static PLAYING: AtomicBool = AtomicBool::new(false);
    static SHUFFLED: AtomicBool = AtomicBool::new(false);
    static REPEAT_TRACK: AtomicBool = AtomicBool::new(false);
    static TASKBAR_MSG: AtomicU32 = AtomicU32::new(0);

    // Готовый «мастер» обложки: WORK_SIZE×WORK_SIZE, углы уже скруглены.
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
    // Предыдущий набор обложек — для кроссфейда/слайда.
    static OLD_COVERS: Mutex<Covers> = Mutex::new(Covers {
        prev: None,
        current: None,
        next: None,
    });

    // Тип перехода между наборами обложек.
    #[derive(Clone, Copy, PartialEq)]
    enum Anim {
        None,
        Fade,
        SlideFwd,
        SlideBack,
    }
    // (тип перехода, время старта).
    static TRANSITION: Mutex<(Anim, Option<Instant>)> = Mutex::new((Anim::None, None));

    // Анимация размера центральной обложки (play/pause): (старт, от, до).
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

    // Габариты последнего запрошенного DWM эскиза — чтобы пушить кадры анимации.
    static LAST_W: AtomicI32 = AtomicI32::new(0);
    static LAST_H: AtomicI32 = AtomicI32::new(0);

    // Пробуждение выделенного потока-аниматора.
    static WAKE: (Mutex<()>, Condvar) = (Mutex::new(()), Condvar::new());

    // COM-интерфейс и HICON не Send — держим в TLS главного потока.
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

    // ── Рендер глифа шрифта в HICON (32bpp с альфой) ─────────────────────────
    // `level` управляет яркостью (255 — белый, меньше — приглушённый серый).
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

            // Альфа из яркости (белый текст на чёрном фоне), цвет — приглушённый
            // серый по `level`, premultiplied.
            let px = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
            for chunk in px.chunks_exact_mut(4) {
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
                        prev: render_glyph_icon(GLYPH_PREV, LEVEL_BRIGHT)?,
                        play: render_glyph_icon(GLYPH_PLAY, LEVEL_BRIGHT)?,
                        pause: render_glyph_icon(GLYPH_PAUSE, LEVEL_BRIGHT)?,
                        next: render_glyph_icon(GLYPH_NEXT, LEVEL_BRIGHT)?,
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

    /// Собирает 5 кнопок по текущему состоянию (play/pause, shuffle, repeat).
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

    /// (Пере)создаёт набор кнопок-миниатюр. Главный поток.
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

    /// Обновляет иконки/подсказки кнопок без пересоздания. Главный поток.
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

    // ── Обложки как iconic bitmap ────────────────────────────────────────────

    /// Скругляет углы квадратной обложки (alpha-маска по SDF rounded-box, с AA).
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

    /// Масштабирует готовый мастер до квадрата `side`. Скругление углов уже
    /// запечено в мастере (альфа масштабируется вместе с картинкой), поэтому
    /// на кадр анимации остаётся только один дешёвый ресайз.
    fn prep_cover(img: &Img, side: u32) -> RgbaImage {
        image::imageops::resize(img, side, side, image::imageops::FilterType::Triangle)
    }

    /// Плавное замедление на концах (smoothstep).
    fn ease(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Геометрия раскладки: (gap, сторона базовой обложки `s`, шаг между центрами).
    fn layout(ch: u32) -> (f32, f32, f32) {
        let gap = ((ch as f32) * 0.06).max(2.0);
        let s = ((ch as f32) - 2.0 * gap).max(1.0);
        (gap, s, s + gap)
    }

    /// Рисует одну обложку в позиции слота `slot` (0 — центр, ±1 — соседи).
    /// Чем ближе к центру, тем крупнее (до `play_scale`) — индикатор воспроизведения.
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
        let size = (s * scale).round().max(1.0) as u32;
        let img = prep_cover(src, size);
        let xc = cw as f32 / 2.0 + slot * step;
        let x = (xc - size as f32 / 2.0).round() as i64;
        let y = ((ch as f32 - size as f32) / 2.0).round() as i64;
        image::imageops::overlay(canvas, &img, x, y);
    }

    /// Статичная раскладка «предыдущий | текущий | следующий».
    fn compose(c: &Covers, cw: u32, ch: u32, play_scale: f32) -> RgbaImage {
        let mut canvas = RgbaImage::from_pixel(cw, ch, image::Rgba([0, 0, 0, 0]));
        let (_, s, step) = layout(ch);
        // Соседи под центром (центр может быть крупнее) — рисуем его последним.
        draw_slot(&mut canvas, &c.prev, -1.0, step, s, cw, ch, play_scale);
        draw_slot(&mut canvas, &c.next, 1.0, step, s, cw, ch, play_scale);
        draw_slot(&mut canvas, &c.current, 0.0, step, s, cw, ch, play_scale);
        canvas
    }

    /// Раскладка во время слайда: обложки сдвигаются влево (вперёд) или вправо
    /// (назад) на один шаг; обложка, проходящая центр, плавно укрупняется.
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
        let (_, s, step) = layout(ch);
        let shift = if forward { -t } else { t };

        // (обложка, слот при t=0). Четвёртая обложка въезжает с края.
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

        // Рисуем от дальних к ближним, чтобы центральная была сверху.
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

    /// Текущий переход обложек: (тип, t∈[0;1]).
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

    /// Текущее значение масштаба центральной обложки (с учётом анимации).
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

    /// Строит превью с учётом перехода обложек и масштаба центральной.
    fn make_thumbnail(max_w: i32, max_h: i32) -> Option<HBITMAP> {
        let cw = max_w.max(1) as u32;
        let ch = max_h.max(1) as u32;
        let ps = play_scale_now();
        let (kind, raw_t) = transition_state();
        let t = ease(raw_t);

        match kind {
            Anim::SlideFwd | Anim::SlideBack => {
                let new = COVERS.lock().ok()?;
                if new.current.is_none() {
                    return None;
                }
                let old = OLD_COVERS.lock().ok()?;
                let forward = kind == Anim::SlideFwd;
                let img = compose_slide(&old, &new, forward, t, cw, ch, ps);
                unsafe { blend_to_hbitmap(&img, None, 1.0) }
            }
            Anim::Fade => {
                let new = COVERS.lock().ok()?;
                if new.current.is_none() {
                    return None;
                }
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
                if new.current.is_none() {
                    return None;
                }
                let img = compose(&new, cw, ch, ps);
                unsafe { blend_to_hbitmap(&img, None, 1.0) }
            }
        }
    }

    /// Создаёт HBITMAP (premultiplied BGRA) из нового кадра, при наличии старого —
    /// смешивая их по `t` в premultiplied-пространстве (корректный кроссфейд).
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
        // premultiply одного пикселя RGBA → (b, g, r, a) с весом w∈[0;1].
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

    // ── Оконная subclass-процедура ───────────────────────────────────────────
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
                if let Some(hbm) = make_thumbnail(640, 400) {
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

    // ── Поток-аниматор ───────────────────────────────────────────────────────
    // Рендерит и пушит кадры напрямую в DWM (~60fps), минуя цикл сообщений tao
    // и главный поток (иначе WM_TIMER пампится медленно — отсюда «5 fps»).

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
            // Спим на condvar, пока нет активной анимации.
            {
                let mut g = WAKE.0.lock().unwrap();
                while !any_anim_active() {
                    // С таймаутом — на случай пропущенного уведомления.
                    g = WAKE.1.wait_timeout(g, Duration::from_millis(250)).unwrap().0;
                }
            }
            // На время анимации поднимаем точность системного таймера: иначе
            // sleep квантуется по ~15.6 мс и вместо 60 fps выходит ~30 и рвано.
            unsafe {
                let _ = timeBeginPeriod(1);
            }
            // Кадры по абсолютным дедлайнам — джиттер сна не накапливается.
            let frame = Duration::from_micros(16_667);
            let mut next = Instant::now();
            while any_anim_active() {
                push_frame();
                next += frame;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now; // отстали — пропускаем, не догоняем пачкой кадров
                }
            }
            // Финальный (устоявшийся) кадр.
            push_frame();
            unsafe {
                let _ = timeEndPeriod(1);
            }
        });
    }

    /// Будит поток-аниматор (после изменения состояния анимации).
    fn wake_animator() {
        WAKE.1.notify_one();
    }

    // ── Публичный API ─────────────────────────────────────────────────────────

    pub fn init(app: &AppHandle, hwnd_raw: isize) {
        let _ = APP.set(app.clone());
        HWND_RAW.store(hwnd_raw, Ordering::Relaxed);
        let h = HWND(hwnd_raw as *mut c_void);
        unsafe {
            let msg = RegisterWindowMessageW(w!("TaskbarButtonCreated"));
            TASKBAR_MSG.store(msg, Ordering::Relaxed);

            let _ = SetWindowSubclass(h, Some(subclass_proc), SUBCLASS_ID, 0);

            // Включаем поддержку iconic bitmap (обложки вместо превью).
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

        // Анимация размера центральной обложки: больше при игре, обратно на паузе.
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

    // Сигнатура последнего набора обложек — чтобы не перезапускать декод и
    // анимацию, когда фронтенд шлёт тот же набор повторно.
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
            h.finish().max(1) // 0 зарезервирован как «ещё не было вызовов»
        };
        if LAST_COVERS_SIG.swap(sig, Ordering::Relaxed) == sig {
            return;
        }

        let decode = |url: Option<String>| -> Option<Img> {
            let url = url?;
            let b64 = url.split(',').nth(1)?;
            let bytes = STANDARD.decode(b64).ok()?;
            let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
            // Готовим «мастер» один раз: рабочий размер + скруглённые углы.
            // Кадры анимации дальше только масштабируют его — иначе покадровый
            // ресайз из полного разрешения и SDF-скругление съедают весь кадр.
            let mut img = image::imageops::resize(
                &img,
                WORK_SIZE,
                WORK_SIZE,
                image::imageops::FilterType::Triangle,
            );
            round_corners(&mut img, WORK_SIZE as f32 * 0.14);
            Some(img)
        };
        let new = Covers {
            prev: decode(prev),
            current: decode(current),
            next: decode(next),
        };
        let has_current = new.current.is_some();
        let kind = match transition.as_str() {
            "forward" => Anim::SlideFwd,
            "backward" => Anim::SlideBack,
            "fade" => Anim::Fade,
            _ => Anim::None,
        };

        // Текущий набор уходит в «старый», запускаем переход, если было от чего.
        let mut animate = false;
        if let (Ok(mut cur), Ok(mut old), Ok(mut tr)) =
            (COVERS.lock(), OLD_COVERS.lock(), TRANSITION.lock())
        {
            let had_current = cur.current.is_some();
            *old = std::mem::replace(&mut *cur, new);
            if had_current && has_current && kind != Anim::None {
                *tr = (kind, Some(Instant::now()));
                animate = true;
            } else {
                *tr = (Anim::None, None);
            }
        }

        if let Some(window) = app.get_webview_window("main") {
            let _ = window.run_on_main_thread(move || {
                set_force_iconic(has_current);
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
}

// ── Заглушки для не-Windows платформ ─────────────────────────────────────────
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
