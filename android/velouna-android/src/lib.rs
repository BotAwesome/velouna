//! NativeActivity entry point: hands the Android app over to Velouna.

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    velouna::run_android(app);
}
