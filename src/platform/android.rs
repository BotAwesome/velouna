//! Android glue: logging, app folders and calls into the Java framework.

use jni::JavaVM;
use jni::objects::{JObject, JValue};
use std::sync::OnceLock;
use winit::platform::android::activity::AndroidApp;

static APP: OnceLock<AndroidApp> = OnceLock::new();

pub fn init(app: &AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("Velouna"));
    if let Some(files) = app.internal_data_path() {
        let cache = files.parent().map(|p| p.join("cache")).unwrap_or_else(|| files.join("cache"));
        // Set before any other thread starts.
        unsafe {
            std::env::set_var("VELOUNA_DATA_DIR", &files);
            std::env::set_var("VELOUNA_CACHE_DIR", &cache);
        }
    }
    let _ = APP.set(app.clone());
    enable_mediacodec(app);
}

/// Runs `f` with a JNI env attached to the current thread and the activity.
fn with_activity<R>(f: impl FnOnce(&mut jni::JNIEnv, &JObject) -> jni::errors::Result<R>) -> Option<R> {
    let app = APP.get()?;
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut _) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    // The activity is a global ref owned by android-activity; don't delete it.
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
    let r = f(&mut env, &activity);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("jni: {e}");
            None
        }
    }
}

pub fn open_url(url: &str) {
    with_activity(|env, activity| {
        let s = env.new_string(url)?;
        let uri = env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&s)])?.l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V", &[JValue::Object(&action), JValue::Object(&uri)])?;
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(0x1000_0000)])?; // FLAG_ACTIVITY_NEW_TASK
        env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        Ok(())
    });
}

pub fn clipboard_text() -> Option<String> {
    with_activity(|env, activity| {
        let name = env.new_string("clipboard")?;
        let cm = env.call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)])?.l()?;
        let clip = env.call_method(&cm, "getPrimaryClip", "()Landroid/content/ClipData;", &[])?.l()?;
        if clip.is_null() {
            return Ok(String::new());
        }
        let item = env.call_method(&clip, "getItemAt", "(I)Landroid/content/ClipData$Item;", &[JValue::Int(0)])?.l()?;
        let text = env.call_method(&item, "coerceToText", "(Landroid/content/Context;)Ljava/lang/CharSequence;", &[JValue::Object(activity)])?.l()?;
        let s = env.call_method(&text, "toString", "()Ljava/lang/String;", &[])?.l()?;
        Ok(env.get_string(&s.into())?.into())
    })
}

/// `Activity.setRequestedOrientation`: -1 unspecified, 6 sensor landscape,
/// 7 sensor portrait.
pub fn set_orientation(o: i32) {
    log::info!("orientation -> {o}");
    with_activity(|env, activity| {
        env.call_method(activity, "setOrientation", "(I)V", &[JValue::Int(o)])?;
        Ok(())
    });
}

/// Calls a `void name(boolean)` method of `VelounaActivity` (runs on the UI
/// thread there). Only when the value changes.
fn activity_flag(state: &std::sync::atomic::AtomicU8, name: &str, on: bool) {
    use std::sync::atomic::Ordering;
    if state.swap(on as u8, Ordering::Relaxed) == on as u8 {
        return;
    }
    with_activity(|env, activity| {
        env.call_method(activity, name, "(Z)V", &[JValue::Bool(on as u8)])?;
        Ok(())
    });
}

/// FLAG_KEEP_SCREEN_ON while playing.
pub fn keep_screen_on(on: bool) {
    static STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);
    activity_flag(&STATE, "setKeepScreenOn", on);
}

/// Window brightness 0..1 for this app only; negative = system setting.
pub fn set_brightness(v: f32) {
    with_activity(|env, activity| {
        env.call_method(activity, "setBrightness", "(F)V", &[JValue::Float(v)])?;
        Ok(())
    });
}

/// Current brightness 0..1 (the window's own, else the system's).
pub fn brightness() -> Option<f32> {
    with_activity(|env, activity| Ok(env.call_method(activity, "getBrightness", "()F", &[])?.f()?)).filter(|v| *v >= 0.0)
}

/// Hides status and navigation bar (player).
pub fn immersive(on: bool) {
    static STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);
    activity_flag(&STATE, "setImmersive", on);
}

/// Lets FFmpeg reach MediaCodec (hardware decoding). Must run before mpv
/// opens a decoder; libmpv.so later links against this same libavcodec.
fn enable_mediacodec(app: &AndroidApp) {
    type SetVm = unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> i32;
    unsafe {
        match libloading::Library::new("libavcodec.so") {
            Ok(lib) => {
                if let Ok(f) = lib.get::<SetVm>(b"av_jni_set_java_vm\0") {
                    let r = f(app.vm_as_ptr(), std::ptr::null_mut());
                    log::info!("av_jni_set_java_vm: {r}");
                }
                // Keep it loaded for the life of the process.
                std::mem::forget(lib);
            }
            Err(e) => log::warn!("libavcodec.so: {e}"),
        }
    }
}

/// Back on the start page: leave like other apps do (keeps playing state).
pub fn move_to_back() {
    with_activity(|env, activity| {
        env.call_method(activity, "moveTaskToBack", "(Z)Z", &[JValue::Bool(1)])?;
        Ok(())
    });
}

/// Hands a downloaded APK to the system package installer.
pub fn install_apk(path: &str) {
    with_activity(|env, activity| {
        let s = env.new_string(path)?;
        env.call_method(activity, "installApk", "(Ljava/lang/String;)V", &[JValue::Object(&s)])?;
        Ok(())
    });
}

/// The user allowed Velouna to install apps (needed for updates).
pub fn can_install_packages() -> bool {
    with_activity(|env, activity| env.call_method(activity, "canInstallPackages", "()Z", &[])?.z()).unwrap_or(true)
}

/// Opens Android's "install unknown apps" page for Velouna.
pub fn open_install_permission() {
    with_activity(|env, activity| {
        env.call_method(activity, "openInstallPermission", "()V", &[])?;
        Ok(())
    });
}

/// Install result: (-1 running, 0 success, else PackageInstaller status; message).
pub fn install_status() -> Option<(i32, String)> {
    with_activity(|env, activity| {
        let code = env.call_method(activity, "getInstallStatus", "()I", &[])?.i()?;
        let msg = env.call_method(activity, "getInstallMessage", "()Ljava/lang/String;", &[])?.l()?;
        let msg: String = if msg.is_null() { String::new() } else { env.get_string(&msg.into())?.into() };
        Ok((code, msg))
    })
}

/// Left and right back-gesture strip widths in px (0 = button navigation).
pub fn gesture_edges_px() -> Option<[f32; 2]> {
    with_activity(|env, activity| {
        let arr = env.call_method(activity, "getGestureEdges", "()[I", &[])?.l()?;
        let arr = jni::objects::JIntArray::from(arr);
        let mut v = [0i32; 2];
        env.get_int_array_region(&arr, 0, &mut v)?;
        Ok([v[0] as f32, v[1] as f32])
    })
}

/// Reserves a screen-edge rectangle (px) from the system back gesture;
/// None releases it.
pub fn set_gesture_exclusion(r: Option<[i32; 4]>) {
    let [x, y, w, h] = r.unwrap_or([0; 4]);
    with_activity(|env, activity| {
        env.call_method(activity, "setGestureExclusion", "(IIII)V", &[JValue::Int(x), JValue::Int(y), JValue::Int(w), JValue::Int(h)])?;
        Ok(())
    });
}

/// System bars, display cutout and soft keyboard in pixels:
/// [left, top, right, bottom].
pub fn system_insets_px() -> Option<[f32; 4]> {
    // VelounaActivity.getSafeInsets: window metrics, so a rotation shows up
    // right away (the keyboard comes from the decor view's insets).
    with_activity(|env, activity| {
        let arr = env.call_method(activity, "getSafeInsets", "()[I", &[])?.l()?;
        let arr = jni::objects::JIntArray::from(arr);
        let mut v = [0i32; 4];
        env.get_int_array_region(&arr, 0, &mut v)?;
        Ok(v.map(|x| x as f32))
    })
}

// ------------------------------------------------------------ storage

/// What Velouna may read on the device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Storage {
    pub videos: bool,
    pub music: bool,
    /// Every file (subtitles and playlists next to the media too).
    pub all_files: bool,
}

/// Current storage permissions (None if the activity can't be asked).
pub fn storage() -> Option<Storage> {
    let bits = with_activity(|env, activity| env.call_method(activity, "storageAccess", "()I", &[])?.i())?;
    Some(Storage { videos: bits & 1 != 0, music: bits & 2 != 0, all_files: bits & 4 != 0 })
}

/// Velouna asked for the storage permission before.
pub fn storage_asked() -> bool {
    with_activity(|env, activity| env.call_method(activity, "storageAsked", "()Z", &[])?.z()).unwrap_or(true)
}

/// Shows Android's permission dialog (or the app's system settings page
/// once Android won't show the dialog again).
pub fn request_storage() {
    with_activity(|env, activity| {
        env.call_method(activity, "requestStorage", "()V", &[])?;
        Ok(())
    });
}

/// Opens the "all files access" page for Velouna.
pub fn open_all_files_access() {
    with_activity(|env, activity| {
        env.call_method(activity, "openAllFilesAccess", "()V", &[])?;
        Ok(())
    });
}

// ------------------------------------------------------------ picker

/// What the system picker should offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickMode {
    Media = 0,
    Subtitles = 1,
    Folder = 2,
}

/// Opens Android's file / folder picker; the answer comes from `take_pick`.
pub fn pick(mode: PickMode, multiple: bool) {
    with_activity(|env, activity| {
        env.call_method(activity, "pick", "(IZ)V", &[JValue::Int(mode as i32), JValue::Bool(multiple as u8)])?;
        Ok(())
    });
}

/// The picker's answer once it is closed: paths (`fdclose://N` for files
/// without one), "!name" for items that can't be opened; empty when
/// cancelled. None while the picker is still open.
pub fn take_pick() -> Option<Vec<String>> {
    with_activity(|env, activity| {
        let arr = env.call_method(activity, "takePick", "()[Ljava/lang/String;", &[])?.l()?;
        if arr.is_null() {
            return Ok(None);
        }
        let arr = jni::objects::JObjectArray::from(arr);
        let n = env.get_array_length(&arr)?;
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let s = env.get_object_array_element(&arr, i)?;
            if !s.is_null() {
                out.push(env.get_string(&s.into())?.into());
            }
        }
        Ok(Some(out))
    })
    .flatten()
}

// ------------------------------------------------------------ media session

static MEDIA_MPV: OnceLock<std::sync::Arc<crate::mpv::Mpv>> = OnceLock::new();
static MEDIA_WAKE: (std::sync::Mutex<bool>, std::sync::Condvar) = (std::sync::Mutex::new(false), std::sync::Condvar::new());
/// Image shown in the notification (station logo, cover), set by the app.
static MEDIA_ART: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Notification / lock screen / headset button -> mpv. Runs on a Java
/// thread, also while the app's UI is in the background.
extern "system" fn native_media_action(_env: jni::JNIEnv, _class: jni::objects::JClass, action: jni::sys::jint, arg: jni::sys::jlong) {
    let Some(m) = MEDIA_MPV.get() else { return };
    let r = match action {
        0 => m.command(&["cycle", "pause"]),
        1 => m.set_flag("pause", false),
        2 => m.set_flag("pause", true),
        3 => m.command(&["playlist-next"]),
        // Like the player's own button: back to the start first.
        4 if m.get_f64("time-pos").unwrap_or(0.0) > 3.0 => m.command(&["seek", "0", "absolute"]),
        4 => m.command(&["playlist-prev"]),
        5 => m.command(&["seek", &format!("{:.3}", arg as f64 / 1000.0), "absolute"]),
        _ => Ok(()),
    };
    if let Err(e) = r {
        log::info!("media action {action}: {e}");
    }
    wake_media();
}

fn wake_media() {
    if let Ok(mut w) = MEDIA_WAKE.0.lock() {
        *w = true;
        MEDIA_WAKE.1.notify_all();
    }
}

/// Picture for the notification (a local image file), None for none.
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

/// Starts the media session: registers the native callback and follows
/// mpv in a thread of its own (the UI loop doesn't run in the background).
pub fn media_init(mpv: std::sync::Arc<crate::mpv::Mpv>, labels: [&str; 5]) {
    if MEDIA_MPV.set(mpv.clone()).is_err() {
        return;
    }
    with_activity(|env, activity| {
        let class = env.get_object_class(activity)?;
        env.register_native_methods(
            &class,
            &[jni::NativeMethod { name: "nativeMediaAction".into(), sig: "(IJ)V".into(), fn_ptr: native_media_action as *mut std::ffi::c_void }],
        )?;
        let string_class = env.find_class("java/lang/String")?;
        let arr = env.new_object_array(5, &string_class, JObject::null())?;
        for (i, l) in labels.iter().enumerate() {
            let s = env.new_string(l)?;
            env.set_object_array_element(&arr, i as i32, &s)?;
        }
        env.call_method(activity, "setMediaLabels", "([Ljava/lang/String;)V", &[JValue::Object(&arr)])?;
        Ok(())
    });
    std::thread::Builder::new().name("media-session".into()).spawn(move || media_loop(mpv)).ok();
}

#[derive(PartialEq)]
struct Shown {
    title: String,
    text: String,
    playing: bool,
    duration_ms: i64,
    art: Option<String>,
}

fn media_loop(m: std::sync::Arc<crate::mpv::Mpv>) {
    use std::time::{Duration, Instant};
    let mut shown: Option<Shown> = None;
    // Position, time and speed of the last update, to spot seeks.
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
                with_activity(|env, activity| {
                    env.call_method(activity, "stopMedia", "()V", &[])?;
                    Ok(())
                });
            }
            continue;
        }
        let title = m.get_string("media-title").unwrap_or_default();
        let meta = |k: &str| m.get_string(&format!("metadata/by-key/{k}")).map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && s != "-");
        // Radio: the song on top, the station below.
        let (title, text) = match meta("icy-title").filter(|s| *s != title.trim()) {
            Some(song) => (song, title),
            None => (title, meta("artist").or_else(|| meta("album_artist")).unwrap_or_else(|| "Velouna".into())),
        };
        let playing = !m.get("pause").and_then(|v| v.as_bool()).unwrap_or(true);
        let pos = m.get_f64("time-pos").unwrap_or(0.0);
        let dur = m.get_f64("duration").unwrap_or(0.0);
        let speed = m.get_f64("speed").unwrap_or(1.0);
        let art = MEDIA_ART.lock().ok().and_then(|a| a.clone());
        let now = Shown { title, text, playing, duration_ms: (dur * 1000.0) as i64, art };
        let expected = sent.0 + if playing { sent.1.elapsed().as_secs_f64() * sent.2 } else { 0.0 };
        let jumped = (pos - expected).abs() > 2.0;
        if shown.as_ref() == Some(&now) && !jumped && sent.1.elapsed() < Duration::from_secs(30) {
            continue;
        }
        sent = (pos, Instant::now(), if playing { speed } else { 1.0 });
        with_activity(|env, activity| {
            let t = env.new_string(&now.title)?;
            let x = env.new_string(&now.text)?;
            let a = env.new_string(now.art.as_deref().unwrap_or(""))?;
            env.call_method(
                activity,
                "updateMedia",
                "(Ljava/lang/String;Ljava/lang/String;ZJJFLjava/lang/String;)V",
                &[
                    JValue::Object(&t),
                    JValue::Object(&x),
                    JValue::Bool(now.playing as u8),
                    JValue::Long((pos * 1000.0) as i64),
                    JValue::Long(now.duration_ms),
                    JValue::Float(speed as f32),
                    JValue::Object(&a),
                ],
            )?;
            Ok(())
        });
        shown = Some(now);
    }
}
