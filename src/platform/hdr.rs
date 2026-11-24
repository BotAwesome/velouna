//! HDR output on Windows.
//!
//! The normal video path renders through libmpv's OpenGL interface into
//! egui, which can only show SDR. For HDR videos on an HDR screen mpv draws
//! itself with `gpu-next` (D3D11, HDR swap chain) into a window of its own
//! that lies over the video area of the Velouna window.
//!
//! Whatever Velouna draws over the video (bars in full screen, menus, OSD,
//! chapter strip) is read back from OpenGL after each paint and shown in a
//! third, layered window with per-pixel alpha on top of the video. OpenGL
//! windows can't be made see-through here (the driver presents them past
//! the window's alpha and colour key), a layered bitmap window always can.
//!
//! Both extra windows never take the focus; mouse input on them goes on to
//! the main window as if it had happened there.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

/// mpv's window and the layered window for the controls (0 = none yet).
static VIDEO_HWND: AtomicIsize = AtomicIsize::new(0);
static OVERLAY_HWND: AtomicIsize = AtomicIsize::new(0);
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
/// Video area in client pixels of the main window, None = hidden.
static AREA: Mutex<Option<[i32; 4]>> = Mutex::new(None);
static HOOKED: AtomicBool = AtomicBool::new(false);
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Hash of the overlay picture last handed to Windows.
static OVERLAY_HASH: AtomicU64 = AtomicU64::new(0);

fn h(v: isize) -> HWND {
    HWND(v as *mut _)
}

/// Creates the (hidden) windows once and returns mpv's `wid`.
pub fn video_window(main: isize) -> Option<isize> {
    MAIN_HWND.store(main, Ordering::Relaxed);
    let v = VIDEO_HWND.load(Ordering::Relaxed);
    if v != 0 {
        return Some(v);
    }
    unsafe {
        let inst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok()?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(input_proc),
            hInstance: inst.into(),
            lpszClassName: w!("VelounaHdrVideo"),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let wc2 = WNDCLASSW { lpszClassName: w!("VelounaHdrOverlay"), hbrBackground: HBRUSH::default(), ..wc };
        RegisterClassW(&wc2);
        // Owned by the main window: always right above it, hidden with it
        // when minimized, no taskbar entry, never activated.
        let video = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            w!("VelounaHdrVideo"),
            w!("Velouna HDR"),
            WS_POPUP | WS_CLIPCHILDREN,
            0,
            0,
            16,
            16,
            Some(h(main)),
            None,
            Some(inst.into()),
            None,
        )
        .ok()?;
        // Owned by the video window: always right above the video.
        let overlay = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            w!("VelounaHdrOverlay"),
            w!("Velouna HDR"),
            WS_POPUP,
            0,
            0,
            16,
            16,
            Some(video),
            None,
            Some(inst.into()),
            None,
        )
        .ok()?;
        VIDEO_HWND.store(video.0 as isize, Ordering::Relaxed);
        OVERLAY_HWND.store(overlay.0 as isize, Ordering::Relaxed);
        if !HOOKED.swap(true, Ordering::Relaxed) {
            let _ = windows::Win32::UI::Shell::SetWindowSubclass(h(main), Some(follow_proc), 0x7e2, 0);
        }
        Some(video.0 as isize)
    }
}

/// Mouse input on the video or the overlay goes on to the main window, as
/// if it had been there; these windows never take the focus.
unsafe extern "system" fn input_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let m = MAIN_HWND.load(Ordering::Relaxed);
    if m != 0 {
        match msg {
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            // Left these windows: tell the main window, unless the pointer
            // went onto it or onto the other one of ours.
            WM_MOUSELEAVE => {
                if !pointer_on_ours(true) {
                    unsafe {
                        let _ = PostMessageW(Some(h(m)), WM_MOUSELEAVE, WPARAM(0), LPARAM(0));
                    }
                }
                return LRESULT(0);
            }
            WM_SETCURSOR => return unsafe { SendMessageW(h(m), msg, Some(WPARAM(m as usize)), Some(lp)) },
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP
            | WM_RBUTTONDBLCLK | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_MBUTTONDBLCLK | WM_XBUTTONDOWN | WM_XBUTTONUP
            | WM_XBUTTONDBLCLK => {
                // Client position here -> client position in the main window.
                let mut p = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
                unsafe {
                    let _ = ClientToScreen(hwnd, &mut p);
                    let _ = ScreenToClient(h(m), &mut p);
                }
                let l = LPARAM((((p.y as u16) as isize) << 16) | (p.x as u16) as isize);
                if msg == WM_MOUSEMOVE {
                    use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
                    let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                }
                if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN) {
                    unsafe {
                        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(h(m)));
                    }
                }
                return unsafe { SendMessageW(h(m), msg, Some(wp), Some(l)) };
            }
            // Wheel positions are screen positions already.
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => return unsafe { SendMessageW(h(m), msg, Some(wp), Some(lp)) },
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// The pointer is over the video, the overlay (or mpv's window in it), and
/// with `main` also over the main window.
fn pointer_on_ours(main: bool) -> bool {
    let mut p = POINT::default();
    unsafe {
        if GetCursorPos(&mut p).is_err() {
            return false;
        }
        let w = WindowFromPoint(p);
        if w.is_invalid() {
            return false;
        }
        let root = GetAncestor(w, GA_ROOT).0 as isize;
        let ours = [VIDEO_HWND.load(Ordering::Relaxed), OVERLAY_HWND.load(Ordering::Relaxed)];
        ours.contains(&root) || (main && root == MAIN_HWND.load(Ordering::Relaxed))
    }
}

/// mpv's own window inside the video window must not take the mouse:
/// disabled, its clicks go to the video window and from there on.
fn calm_children(v: HWND) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled};
    unsafe {
        let mut c = GetWindow(v, GW_CHILD).ok();
        while let Some(w) = c.filter(|w| !w.is_invalid()) {
            if IsWindowEnabled(w).as_bool() {
                let _ = EnableWindow(w, false);
            }
            c = GetWindow(w, GW_HWNDNEXT).ok();
        }
    }
}

/// Shows the video over `area` (client pixels of the main window), or hides
/// it with None. Cheap when nothing changed.
pub fn place(area: Option<[i32; 4]>) {
    let changed = {
        let mut a = AREA.lock().unwrap();
        let c = *a != area;
        *a = area;
        c
    };
    if changed {
        if area.is_none() {
            OVERLAY_HASH.store(0, Ordering::Relaxed);
        }
        update();
    }
}

/// The video area, while HDR output is shown.
pub fn area() -> Option<[i32; 4]> {
    *AREA.lock().unwrap()
}

/// Moves both windows to where the video area of the main window is.
fn update() {
    let v = VIDEO_HWND.load(Ordering::Relaxed);
    let o = OVERLAY_HWND.load(Ordering::Relaxed);
    let m = MAIN_HWND.load(Ordering::Relaxed);
    if v == 0 || m == 0 {
        return;
    }
    let area = *AREA.lock().unwrap();
    unsafe {
        match area {
            Some([x, y, w, hh]) => {
                let mut p = POINT { x, y };
                let _ = ClientToScreen(h(m), &mut p);
                let f = SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW;
                let _ = SetWindowPos(h(v), None, p.x, p.y, w.max(1), hh.max(1), f);
                let _ = SetWindowPos(h(o), None, p.x, p.y, w.max(1), hh.max(1), f);
                calm_children(h(v));
            }
            None => {
                let _ = ShowWindow(h(o), SW_HIDE);
                let _ = ShowWindow(h(v), SW_HIDE);
            }
        }
    }
}

/// Keeps the windows over the video area while the main window moves.
unsafe extern "system" fn follow_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _d: usize) -> LRESULT {
    let active = AREA.lock().map(|a| a.is_some()).unwrap_or(false);
    // The pointer went onto the video: for Velouna it is still inside.
    if msg == WM_MOUSELEAVE && active && pointer_on_ours(false) {
        return LRESULT(0);
    }
    let r = unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wp, lp) };
    if msg == WM_WINDOWPOSCHANGED && AREA.lock().map(|a| a.is_some()).unwrap_or(false) {
        update();
    }
    r
}

/// Hands the controls drawn over the video to the overlay window.
/// `rgba` is premultiplied, top row first, `w`×`h` pixels.
pub fn overlay(rgba: &[u8], w: i32, hh: i32) {
    let o = OVERLAY_HWND.load(Ordering::Relaxed);
    if o == 0 || w <= 0 || hh <= 0 || rgba.len() < (w * hh * 4) as usize {
        return;
    }
    let Some([x, y, _, _]) = area() else { return };
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut k = std::collections::hash_map::DefaultHasher::new();
        (w, hh, x, y).hash(&mut k);
        rgba.hash(&mut k);
        k.finish() | 1
    };
    if OVERLAY_HASH.swap(hash, Ordering::Relaxed) == hash {
        return;
    }
    let m = MAIN_HWND.load(Ordering::Relaxed);
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -hh,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        if let Ok(bmp) = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * hh * 4) as usize);
            for (d, s) in dst.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = s[3];
            }
            let old = SelectObject(dc, bmp.into());
            let mut p = POINT { x, y };
            let _ = ClientToScreen(h(m), &mut p);
            let size = SIZE { cx: w, cy: hh };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(h(o), Some(screen), Some(&p), Some(&size), Some(dc), Some(&src), Default::default(), Some(&blend), ULW_ALPHA);
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
    }
}

/// The window's monitor shows HDR (Windows "Use HDR" is on there).
/// Asking DXGI takes a few ms, so the answer is kept for two seconds.
pub fn screen_hdr(main: isize) -> bool {
    static LAST: Mutex<Option<(Instant, isize, bool)>> = Mutex::new(None);
    let mon = unsafe { MonitorFromWindow(h(main), MONITOR_DEFAULTTONEAREST) };
    let mut l = LAST.lock().unwrap();
    if let Some((t, m, v)) = *l {
        if m == mon.0 as isize && t.elapsed() < Duration::from_secs(2) {
            return v;
        }
    }
    let v = monitor_hdr(mon).unwrap_or(false);
    if l.map(|x| x.2) != Some(v) {
        log::info!("screen HDR: {}", if v { "on" } else { "off" });
    }
    *l = Some((Instant::now(), mon.0 as isize, v));
    v
}

fn monitor_hdr(mon: HMONITOR) -> windows::core::Result<bool> {
    use windows::Win32::Graphics::Dxgi::Common::DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput6};
    use windows::core::Interface;
    unsafe {
        let f: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut a = 0;
        while let Ok(adapter) = f.EnumAdapters1(a) {
            let mut o = 0;
            while let Ok(out) = adapter.EnumOutputs(o) {
                if let Ok(o6) = out.cast::<IDXGIOutput6>() {
                    let d = o6.GetDesc1()?;
                    if d.Monitor == mon {
                        return Ok(d.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020);
                    }
                }
                o += 1;
            }
            a += 1;
        }
    }
    Ok(false)
}
