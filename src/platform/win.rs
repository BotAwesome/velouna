//! Windows glue: media keys and the Windows media overlay (System Media
//! Transport Controls), taskbar buttons and progress, the jump list, and
//! thumbnails and tags from the shell.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs, SystemMediaTransportControls,
    SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs, SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::StorageFile;
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
use windows::core::HSTRING;

/// Picture for the overlay (a local image file), set by the app.
static MEDIA_ART: Mutex<Option<String>> = Mutex::new(None);
static MEDIA_WAKE: (Mutex<bool>, std::sync::Condvar) = (Mutex::new(false), std::sync::Condvar::new());
static HWND_RAW: OnceLock<isize> = OnceLock::new();

fn wake_media() {
    if let Ok(mut w) = MEDIA_WAKE.0.lock() {
        *w = true;
        MEDIA_WAKE.1.notify_all();
    }
}

/// The window, for the taskbar and the media overlay.
pub fn set_window(hwnd: isize) {
    let _ = HWND_RAW.set(hwnd);
}

/// The main window's handle.
pub fn main_window() -> Option<isize> {
    HWND_RAW.get().copied()
}

fn hwnd() -> Option<HWND> {
    HWND_RAW.get().map(|h| HWND(*h as *mut _))
}

/// Picture for the media overlay (a local image file), None for none.
pub fn set_media_art(path: Option<String>) {
    let changed = match MEDIA_ART.lock() {
        Ok(mut a) if *a != path => {
            *a = path;
            true
        }
        _ => false,
    };
    if changed {
        wake_media();
    }
}

// ------------------------------------------------------------ media overlay

/// Connects the media keys and the Windows media overlay to mpv and keeps
/// the overlay up to date from a thread of its own.
pub fn media_init(mpv: Arc<crate::mpv::Mpv>) {
    let _ = MEDIA_MPV.set(mpv.clone());
    let Some(h) = hwnd() else { return };
    let smtc = match smtc_for(h) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("media overlay: {e}");
            return;
        }
    };
    let m = mpv.clone();
    let pressed = TypedEventHandler::<SystemMediaTransportControls, SystemMediaTransportControlsButtonPressedEventArgs>::new(move |_, args| {
        if let Some(a) = args.as_ref() {
            let r = match a.Button()? {
                SystemMediaTransportControlsButton::Play => m.set_flag("pause", false),
                SystemMediaTransportControlsButton::Pause => m.set_flag("pause", true),
                SystemMediaTransportControlsButton::Stop => m.command(&["stop"]),
                SystemMediaTransportControlsButton::Next => m.command(&["playlist-next"]),
                // Like the player's own button: back to the start first.
                SystemMediaTransportControlsButton::Previous if m.get_f64("time-pos").unwrap_or(0.0) > 3.0 => m.command(&["seek", "0", "absolute"]),
                SystemMediaTransportControlsButton::Previous => m.command(&["playlist-prev"]),
                _ => Ok(()),
            };
            if let Err(e) = r {
                log::info!("media key: {e}");
            }
            wake_media();
        }
        Ok(())
    });
    let m = mpv.clone();
    let seek = TypedEventHandler::<SystemMediaTransportControls, PlaybackPositionChangeRequestedEventArgs>::new(move |_, args| {
        if let Some(a) = args.as_ref() {
            let t = a.RequestedPlaybackPosition()?.Duration as f64 / 1e7;
            let _ = m.command(&["seek", &format!("{t:.3}"), "absolute"]);
            wake_media();
        }
        Ok(())
    });
    let ok = (|| -> windows::core::Result<()> {
        smtc.ButtonPressed(&pressed)?;
        smtc.PlaybackPositionChangeRequested(&seek)?;
        smtc.SetIsPlayEnabled(true)?;
        smtc.SetIsPauseEnabled(true)?;
        smtc.SetIsStopEnabled(true)?;
        smtc.SetIsNextEnabled(true)?;
        smtc.SetIsPreviousEnabled(true)?;
        Ok(())
    })();
    if let Err(e) = ok {
        log::warn!("media overlay: {e}");
        return;
    }
    std::thread::Builder::new().name("media-overlay".into()).spawn(move || media_loop(mpv, smtc)).ok();
}

fn smtc_for(h: HWND) -> windows::core::Result<SystemMediaTransportControls> {
    let interop = windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
    unsafe { interop.GetForWindow(h) }
}

#[derive(PartialEq)]
struct Shown {
    title: String,
    text: String,
    video: bool,
    playing: bool,
    duration: f64,
    art: Option<String>,
}

fn media_loop(m: Arc<crate::mpv::Mpv>, smtc: SystemMediaTransportControls) {
    let mut shown: Option<Shown> = None;
    let mut sent = (0.0f64, Instant::now(), 1.0f64);
    loop {
        if let Ok(w) = MEDIA_WAKE.0.lock() {
            let (mut w, _) = MEDIA_WAKE.1.wait_timeout_while(w, Duration::from_secs(1), |w| !*w).unwrap_or_else(|e| e.into_inner());
            *w = false;
        }
        let idle = m.get("idle-active").and_then(|v| v.as_bool()).unwrap_or(true);
        let path = m.get_string("path").unwrap_or_default();
        if idle || path.is_empty() {
            if shown.take().is_some() {
                let _ = smtc.SetPlaybackStatus(MediaPlaybackStatus::Closed);
                let _ = smtc.SetIsEnabled(false);
            }
            continue;
        }
        let title = m.get_string("media-title").unwrap_or_default();
        let meta = |k: &str| m.get_string(&format!("metadata/by-key/{k}")).map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && s != "-");
        // Radio: the song on top, the station below.
        let (title, text) = match meta("icy-title").filter(|s| *s != title.trim()) {
            Some(song) => (song, title),
            None => (title, meta("artist").or_else(|| meta("album_artist")).unwrap_or_default()),
        };
        let playing = !m.get("pause").and_then(|v| v.as_bool()).unwrap_or(true);
        let pos = m.get_f64("time-pos").unwrap_or(0.0);
        let duration = m.get_f64("duration").unwrap_or(0.0);
        let speed = m.get_f64("speed").unwrap_or(1.0);
        // a cover counts as a picture track too ("image": yes)
        let video = m.get_string("current-tracks/video/image").as_deref() == Some("no");
        let art = MEDIA_ART.lock().ok().and_then(|a| a.clone());
        let now = Shown { title, text, video, playing, duration, art };
        let expected = sent.0 + if playing { sent.1.elapsed().as_secs_f64() * sent.2 } else { 0.0 };
        let jumped = (pos - expected).abs() > 2.0;
        if shown.as_ref() == Some(&now) && !jumped && sent.1.elapsed() < Duration::from_secs(10) {
            continue;
        }
        sent = (pos, Instant::now(), if playing { speed } else { 1.0 });
        if let Err(e) = show(&smtc, &now, pos, shown.as_ref()) {
            log::info!("media overlay: {e}");
        }
        shown = Some(now);
    }
}

fn span(secs: f64) -> TimeSpan {
    TimeSpan { Duration: (secs.max(0.0) * 1e7) as i64 }
}

fn show(smtc: &SystemMediaTransportControls, now: &Shown, pos: f64, before: Option<&Shown>) -> windows::core::Result<()> {
    smtc.SetIsEnabled(true)?;
    smtc.SetPlaybackStatus(if now.playing { MediaPlaybackStatus::Playing } else { MediaPlaybackStatus::Paused })?;
    let changed = before.is_none_or(|b| b.title != now.title || b.text != now.text || b.art != now.art || b.video != now.video);
    if changed {
        let u = smtc.DisplayUpdater()?;
        u.ClearAll()?;
        if now.video {
            u.SetType(MediaPlaybackType::Video)?;
            let v = u.VideoProperties()?;
            v.SetTitle(&HSTRING::from(&now.title))?;
            v.SetSubtitle(&HSTRING::from(&now.text))?;
        } else {
            u.SetType(MediaPlaybackType::Music)?;
            let mp = u.MusicProperties()?;
            mp.SetTitle(&HSTRING::from(&now.title))?;
            mp.SetArtist(&HSTRING::from(&now.text))?;
        }
        if let Some(a) = &now.art {
            if let Ok(f) = StorageFile::GetFileFromPathAsync(&HSTRING::from(a.as_str())).and_then(|op| op.get()) {
                u.SetThumbnail(&RandomAccessStreamReference::CreateFromFile(&f)?)?;
            }
        }
        u.Update()?;
    }
    let tl = SystemMediaTransportControlsTimelineProperties::new()?;
    if now.duration > 0.0 {
        tl.SetStartTime(span(0.0))?;
        tl.SetMinSeekTime(span(0.0))?;
        tl.SetEndTime(span(now.duration))?;
        tl.SetMaxSeekTime(span(now.duration))?;
        tl.SetPosition(span(pos))?;
    }
    smtc.UpdateTimelineProperties(&tl)?;
    Ok(())
}

// ------------------------------------------------------------ taskbar

static MEDIA_MPV: OnceLock<Arc<crate::mpv::Mpv>> = OnceLock::new();
/// The taskbar button was (re)created: the thumbnail buttons need adding.
static TASKBAR_RESET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static TASKBAR_MSG: OnceLock<u32> = OnceLock::new();

const BTN_PREV: u32 = 1;
const BTN_PLAY: u32 = 2;
const BTN_NEXT: u32 = 3;

struct Taskbar {
    list: windows::Win32::UI::Shell::ITaskbarList3,
    icons: [windows::Win32::UI::WindowsAndMessaging::HICON; 4],
    added: bool,
    playing: Option<bool>,
    progress: Option<u32>,
}

thread_local! {
    static TASKBAR: std::cell::RefCell<Option<Taskbar>> = const { std::cell::RefCell::new(None) };
}

/// White glyphs for the taskbar buttons, 32x32 BGRA: previous, play, pause, next.
fn glyph(kind: u8) -> Vec<u8> {
    let n = 32usize;
    let mut px = vec![0u8; n * n * 4];
    // triangle with its base at x = ax (y 8..24) and its tip at x = bx (y 16)
    let tri = |x: f32, y: f32, ax: f32, bx: f32| {
        let t = (x - ax) / (bx - ax);
        (0.0..=1.0).contains(&t) && (y - 16.0).abs() <= 8.0 * (1.0 - t)
    };
    for yi in 0..n {
        for xi in 0..n {
            let (x, y) = (xi as f32 + 0.5, yi as f32 + 0.5);
            let bar = |a: f32, b: f32| (a..b).contains(&x) && (8.0..24.0).contains(&y);
            let on = match kind {
                0 => bar(8.0, 11.0) || tri(x, y, 24.0, 11.0),
                1 => tri(x, y, 10.0, 24.0),
                2 => bar(10.0, 14.0) || bar(18.0, 22.0),
                _ => bar(21.0, 24.0) || tri(x, y, 8.0, 21.0),
            };
            if on {
                let i = (yi * n + xi) * 4;
                px[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    px
}

fn make_icon(bgra: &[u8]) -> windows::core::Result<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};
    unsafe {
        let color = CreateBitmap(32, 32, 1, 32, Some(bgra.as_ptr() as *const _));
        let mask_bits = vec![0u8; 32 * 32 / 8];
        let mask = CreateBitmap(32, 32, 1, 1, Some(mask_bits.as_ptr() as *const _));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

fn thumb_button(id: u32, icon: windows::Win32::UI::WindowsAndMessaging::HICON, tip: &str, enabled: bool) -> windows::Win32::UI::Shell::THUMBBUTTON {
    use windows::Win32::UI::Shell::{THB_FLAGS, THB_ICON, THB_TOOLTIP, THBF_DISABLED, THBF_ENABLED, THUMBBUTTON};
    let mut b = THUMBBUTTON {
        dwMask: THB_FLAGS | THB_ICON | THB_TOOLTIP,
        iId: id,
        hIcon: icon,
        dwFlags: if enabled { THBF_ENABLED } else { THBF_DISABLED },
        ..Default::default()
    };
    for (i, c) in tip.encode_utf16().take(259).enumerate() {
        b.szTip[i] = c;
    }
    b
}

/// Window procedure hook: clicks on the taskbar buttons, and the taskbar
/// telling us its button is (re)created.
unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::WM_COMMAND;
    const THBN_CLICKED: usize = 0x1800;
    if msg == WM_COMMAND && (wparam.0 >> 16) & 0xffff == THBN_CLICKED {
        if let Some(m) = MEDIA_MPV.get() {
            let _ = match (wparam.0 & 0xffff) as u32 {
                BTN_PREV if m.get_f64("time-pos").unwrap_or(0.0) > 3.0 => m.command(&["seek", "0", "absolute"]),
                BTN_PREV => m.command(&["playlist-prev"]),
                BTN_PLAY => m.command(&["cycle", "pause"]),
                BTN_NEXT => m.command(&["playlist-next"]),
                _ => Ok(()),
            };
            wake_media();
        }
    }
    if TASKBAR_MSG.get() == Some(&msg) {
        TASKBAR_RESET.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Taskbar buttons (previous, play/pause, next) in the window preview, and
/// a progress bar in the taskbar icon. `playing` is None without media.
/// Call every frame from the UI thread; it only talks to Windows on changes.
pub fn taskbar(playing: Option<bool>, progress: Option<f32>, labels: [&str; 4]) {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Shell::{ITaskbarList3, SetWindowSubclass, TBPF_NOPROGRESS, TBPF_NORMAL, TaskbarList};
    let Some(h) = hwnd() else { return };
    TASKBAR.with(|tb| {
        let mut tb = tb.borrow_mut();
        if tb.is_none() {
            let r = (|| -> windows::core::Result<Taskbar> {
                let list: ITaskbarList3 = unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)? };
                unsafe { list.HrInit()? };
                let mut icons: [windows::Win32::UI::WindowsAndMessaging::HICON; 4] = Default::default();
                for (k, icon) in icons.iter_mut().enumerate() {
                    *icon = make_icon(&glyph(k as u8))?;
                }
                let _ = TASKBAR_MSG.set(unsafe { windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(windows::core::w!("TaskbarButtonCreated")) });
                unsafe {
                    let _ = SetWindowSubclass(h, Some(subclass_proc), 0x7e1, 0);
                }
                Ok(Taskbar { list, icons, added: false, playing: None, progress: None })
            })();
            match r {
                Ok(t) => *tb = Some(t),
                Err(e) => {
                    log::warn!("taskbar: {e}");
                    return;
                }
            }
        }
        let Some(t) = tb.as_mut() else { return };
        if TASKBAR_RESET.swap(false, std::sync::atomic::Ordering::Relaxed) {
            t.added = false;
            t.progress = None;
        }
        let buttons = |t: &Taskbar| {
            let on = playing.is_some();
            [
                thumb_button(BTN_PREV, t.icons[0], labels[0], on),
                if playing == Some(true) { thumb_button(BTN_PLAY, t.icons[2], labels[2], on) } else { thumb_button(BTN_PLAY, t.icons[1], labels[1], on) },
                thumb_button(BTN_NEXT, t.icons[3], labels[3], on),
            ]
        };
        if !t.added {
            if unsafe { t.list.ThumbBarAddButtons(h, &buttons(t)) }.is_ok() {
                log::info!("taskbar buttons added");
                t.added = true;
                t.playing = playing;
            }
        } else if t.playing != playing && unsafe { t.list.ThumbBarUpdateButtons(h, &buttons(t)) }.is_ok() {
            t.playing = playing;
        }
        let p = progress.map(|f| (f.clamp(0.0, 1.0) * 1000.0) as u32);
        if t.progress != p {
            unsafe {
                match p {
                    Some(v) => {
                        let _ = t.list.SetProgressState(h, TBPF_NORMAL);
                        let _ = t.list.SetProgressValue(h, v as u64, 1000);
                    }
                    None => {
                        let _ = t.list.SetProgressState(h, TBPF_NOPROGRESS);
                    }
                }
            }
            t.progress = p;
        }
    });
}

// ------------------------------------------------------------ jump list

/// Right click on the taskbar icon: lists of files (heading, [(path,
/// title)]) that open in Velouna.
pub fn jump_list(lists: &[(String, Vec<(String, String)>)]) {
    use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink};
    use windows::core::Interface;
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = HSTRING::from(exe.as_os_str());
    let r = (|| -> windows::core::Result<()> {
        let list: ICustomDestinationList = unsafe { CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)? };
        let mut slots = 0u32;
        let removed: IObjectArray = unsafe { list.BeginList(&mut slots)? };
        // entries the user removed from the list must not come back
        let mut gone = std::collections::HashSet::new();
        for i in 0..unsafe { removed.GetCount()? } {
            if let Ok(l) = unsafe { removed.GetAt::<IShellLinkW>(i) } {
                let mut buf = [0u16; 1024];
                if unsafe { l.GetArguments(&mut buf) }.is_ok() {
                    let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
                    gone.insert(String::from_utf16_lossy(&buf[..n]));
                }
            }
        }
        let mut left = slots.max(4) as usize;
        for (head, items) in lists {
            let coll: IObjectCollection = unsafe { CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)? };
            let mut n = 0;
            for (path, title) in items {
                if left == 0 {
                    break;
                }
                let args = format!("\"{path}\"");
                if gone.contains(&args) {
                    continue;
                }
                let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)? };
                unsafe {
                    link.SetPath(&exe)?;
                    link.SetArguments(&HSTRING::from(args))?;
                    link.SetIconLocation(&exe, 0)?;
                    link.SetDescription(&HSTRING::from(path.as_str()))?;
                    let ps: IPropertyStore = link.cast()?;
                    ps.SetValue(&PKEY_Title, &PROPVARIANT::from(title.as_str()))?;
                    ps.Commit()?;
                    coll.AddObject(&link)?;
                }
                n += 1;
                left -= 1;
            }
            if n > 0 {
                let arr: IObjectArray = coll.cast()?;
                unsafe { list.AppendCategory(&HSTRING::from(head.as_str()), &arr)? };
            }
        }
        unsafe { list.CommitList()? };
        Ok(())
    })();
    if let Err(e) = r {
        log::info!("jump list: {e}");
    }
}

// ------------------------------------------------------------ shell

fn com_mta() {
    thread_local! {
        static INIT: () = unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        };
    }
    INIT.with(|_| {});
}

/// The thumbnail Windows shows in Explorer (its cache, or its own
/// thumbnail handler) as straight RGBA, with its width and height.
pub fn shell_thumb(path: &str, size: i32) -> Option<(Vec<u8>, usize, usize)> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, GetObjectW, HGDIOBJ};
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY};
    com_mta();
    unsafe {
        let f: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let hbm = f.GetImage(SIZE { cx: size, cy: size }, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let mut bm = BITMAP::default();
        GetObjectW(HGDIOBJ(hbm.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _));
        let (w, h) = (bm.bmWidth.max(0) as usize, bm.bmHeight.max(0) as usize);
        let mut out = None;
        if w > 0 && h > 0 {
            let mut info = BITMAPINFO::default();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };
            let mut px = vec![0u8; w * h * 4];
            let dc = CreateCompatibleDC(None);
            let lines = GetDIBits(dc, hbm, 0, h as u32, Some(px.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
            let _ = DeleteDC(dc);
            if lines > 0 {
                // thumbnails without alpha come with alpha 0 everywhere
                let no_alpha = px.chunks_exact(4).all(|p| p[3] == 0);
                for p in px.chunks_exact_mut(4) {
                    p.swap(0, 2);
                    if no_alpha {
                        p[3] = 255;
                    }
                }
                out = Some((px, w, h));
            }
        }
        let _ = DeleteObject(hbm.into());
        out
    }
}

/// Tags, length and picture size from the Windows property system (what
/// Explorer shows in its columns), instead of one ffprobe per file.
pub fn shell_props(path: &str) -> Option<crate::media::ProbeInfo> {
    use windows::Win32::Storage::EnhancedStorage::{
        PKEY_Media_Duration, PKEY_Music_AlbumArtist, PKEY_Music_AlbumTitle, PKEY_Music_Artist, PKEY_Music_Genre, PKEY_Music_TrackNumber, PKEY_Title,
        PKEY_Video_FrameHeight, PKEY_Video_FrameWidth,
    };
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::System::Com::StructuredStorage::{PropVariantToStringAlloc, PropVariantToUInt32, PropVariantToUInt64};
    use windows::Win32::UI::Shell::PropertiesSystem::{GPS_DEFAULT, IPropertyStore, SHGetPropertyStoreFromParsingName};
    com_mta();
    unsafe {
        let ps: IPropertyStore = SHGetPropertyStoreFromParsingName(&HSTRING::from(path), None, GPS_DEFAULT).ok()?;
        let s = |k: &PROPERTYKEY| -> Option<String> {
            let v = ps.GetValue(k).ok()?;
            let p = PropVariantToStringAlloc(&v).ok()?;
            let out = p.to_string().ok();
            windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
            out.map(|x| x.trim().to_string()).filter(|x| !x.is_empty())
        };
        let u32_of = |k: &PROPERTYKEY| ps.GetValue(k).ok().and_then(|v| PropVariantToUInt32(&v).ok());
        let dur = ps.GetValue(&PKEY_Media_Duration).ok().and_then(|v| PropVariantToUInt64(&v).ok()).unwrap_or(0);
        let width = u32_of(&PKEY_Video_FrameWidth).unwrap_or(0);
        let height = u32_of(&PKEY_Video_FrameHeight).unwrap_or(0);
        Some(crate::media::ProbeInfo {
            duration: dur as f64 / 1e7,
            title: s(&PKEY_Title),
            artist: s(&PKEY_Music_Artist).or_else(|| s(&PKEY_Music_AlbumArtist)),
            album: s(&PKEY_Music_AlbumTitle),
            genre: s(&PKEY_Music_Genre),
            track: u32_of(&PKEY_Music_TrackNumber).filter(|n| *n > 0),
            has_video: width > 0,
            has_cover: false,
            width,
            height,
            mtime: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    /// Needs a video at VELOUNA_TEST_VIDEO (skipped without).
    #[test]
    fn shell_thumb_and_props() {
        let Ok(v) = std::env::var("VELOUNA_TEST_VIDEO") else { return };
        let t = std::time::Instant::now();
        let (px, w, h) = super::shell_thumb(&v, 320).expect("thumbnail");
        eprintln!("shell thumb {w}x{h} in {:?}", t.elapsed());
        assert_eq!(px.len(), w * h * 4);
        let p = super::shell_props(&v).expect("props");
        eprintln!("props: {:?} {}x{} {}s", p.title, p.width, p.height, p.duration);
        assert!(p.duration > 0.0);
    }
}
