//! Android glue: logging, app folders and calls into the Java framework.

use jni::JavaVM;
use jni::objects::{JObject, JValue};
use std::sync::OnceLock;
use winit::platform::android::activity::AndroidApp;

static APP: OnceLock<AndroidApp> = OnceLock::new();

pub fn init(app: &AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("Velour"));
    if let Some(files) = app.internal_data_path() {
        let cache = files.parent().map(|p| p.join("cache")).unwrap_or_else(|| files.join("cache"));
        // Set before any other thread starts.
        unsafe {
            std::env::set_var("VELOUR_DATA_DIR", &files);
            std::env::set_var("VELOUR_CACHE_DIR", &cache);
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

/// Calls a `void name(boolean)` method of `VelourActivity` (runs on the UI
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

/// The user allowed Velour to install apps (needed for updates).
pub fn can_install_packages() -> bool {
    with_activity(|env, activity| env.call_method(activity, "canInstallPackages", "()Z", &[])?.z()).unwrap_or(true)
}

/// Opens Android's "install unknown apps" page for Velour.
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
    with_activity(|env, activity| {
        let window = env.call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
        let decor = env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()?;
        let wi = env.call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])?.l()?;
        if wi.is_null() {
            return Ok([0.0; 4]);
        }
        let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
        if sdk >= 30 {
            let bars = env.call_static_method("android/view/WindowInsets$Type", "systemBars", "()I", &[])?.i()?;
            let cut = env.call_static_method("android/view/WindowInsets$Type", "displayCutout", "()I", &[])?.i()?;
            // The soft keyboard too, so the layout shrinks above it.
            let ime = env.call_static_method("android/view/WindowInsets$Type", "ime", "()I", &[])?.i()?;
            let ins = env.call_method(&wi, "getInsets", "(I)Landroid/graphics/Insets;", &[JValue::Int(bars | cut | ime)])?.l()?;
            let mut v = [0.0; 4];
            for (i, f) in ["left", "top", "right", "bottom"].iter().enumerate() {
                v[i] = env.get_field(&ins, *f, "I")?.i()? as f32;
            }
            Ok(v)
        } else {
            let mut v = [0.0; 4];
            for (i, m) in ["getSystemWindowInsetLeft", "getSystemWindowInsetTop", "getSystemWindowInsetRight", "getSystemWindowInsetBottom"].iter().enumerate() {
                v[i] = env.call_method(&wi, *m, "()I", &[])?.i()? as f32;
            }
            Ok(v)
        }
    })
}
