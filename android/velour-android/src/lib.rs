//! NativeActivity entry point: hands the Android app over to Velour.

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    velour::run_android(app);
}
