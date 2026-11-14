//! GPU video output: mpv renders into an offscreen OpenGL framebuffer which is
//! then blitted into egui's frame inside a paint callback.

use crate::mpv::{self, Mpv};
use eframe::egui;
use eframe::glow::{self, HasContext};
use std::ffi::{CString, c_char, c_void};
use std::sync::{Arc, Mutex};

#[cfg(windows)]
mod gl_loader {
    use std::ffi::{c_char, c_void};
    #[link(name = "opengl32")]
    unsafe extern "system" {
        fn wglGetProcAddress(name: *const c_char) -> *mut c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
        fn LoadLibraryA(name: *const c_char) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    pub unsafe extern "C" fn get_proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
        unsafe {
            let p = wglGetProcAddress(name);
            let bad = p.is_null() || matches!(p as isize, 1 | 2 | 3 | -1);
            if !bad {
                return p;
            }
            let mut m = GetModuleHandleA(c"opengl32.dll".as_ptr());
            if m.is_null() {
                m = LoadLibraryA(c"opengl32.dll".as_ptr());
            }
            if m.is_null() {
                return std::ptr::null_mut();
            }
            GetProcAddress(m, name)
        }
    }
}

#[cfg(target_os = "android")]
mod gl_loader {
    use std::ffi::{CStr, c_char, c_void};
    use std::sync::OnceLock;
    #[link(name = "EGL")]
    unsafe extern "C" {
        fn eglGetProcAddress(name: *const c_char) -> *mut c_void;
    }
    /// GLES entry points for mpv's OpenGL renderer: EGL first (extensions,
    /// and core functions on EGL 1.5), then the GLES library itself.
    pub unsafe extern "C" fn get_proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
        static GLES: OnceLock<Option<libloading::Library>> = OnceLock::new();
        unsafe {
            let p = eglGetProcAddress(name);
            if !p.is_null() {
                return p;
            }
            let lib = GLES.get_or_init(|| libloading::Library::new("libGLESv3.so").or_else(|_| libloading::Library::new("libGLESv2.so")).ok());
            let Some(lib) = lib else { return std::ptr::null_mut() };
            lib.get::<*mut c_void>(CStr::from_ptr(name).to_bytes_with_nul()).map(|s| *s).unwrap_or(std::ptr::null_mut())
        }
    }
}

#[cfg(target_os = "linux")]
mod gl_loader {
    use std::ffi::{CStr, c_char, c_void};
    use std::sync::OnceLock;
    type GetProc = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    type GetCurrent = unsafe extern "C" fn() -> *mut c_void;
    struct Loader {
        get: Option<GetProc>,
        lib: Option<libloading::Library>,
    }
    unsafe impl Send for Loader {}
    unsafe impl Sync for Loader {}
    fn sym<T: Copy>(lib: &libloading::Library, name: &[u8]) -> Option<T> {
        unsafe { lib.get::<T>(name).ok().map(|s| *s) }
    }
    /// eframe (glutin) uses EGL on Wayland and usually on X11, GLX on some
    /// X11 setups: ask whichever API holds the current context.
    fn loader() -> &'static Loader {
        static L: OnceLock<Loader> = OnceLock::new();
        L.get_or_init(|| unsafe {
            if let Ok(egl) = libloading::Library::new("libEGL.so.1") {
                let cur: Option<GetCurrent> = sym(&egl, b"eglGetCurrentContext\0");
                if cur.is_some_and(|f| !f().is_null()) {
                    let get: Option<GetProc> = sym(&egl, b"eglGetProcAddress\0");
                    return Loader { get, lib: Some(egl) };
                }
            }
            if let Ok(glx) = libloading::Library::new("libGLX.so.0").or_else(|_| libloading::Library::new("libGL.so.1")) {
                let get: Option<GetProc> = sym(&glx, b"glXGetProcAddressARB\0");
                return Loader { get, lib: Some(glx) };
            }
            Loader { get: None, lib: None }
        })
    }
    pub unsafe extern "C" fn get_proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
        static GL: OnceLock<Option<libloading::Library>> = OnceLock::new();
        unsafe {
            let l = loader();
            if let Some(get) = l.get {
                let p = get(name);
                if !p.is_null() {
                    return p;
                }
            }
            let _ = &l.lib;
            // Core functions straight from the GL library as a last resort.
            let gl = GL.get_or_init(|| libloading::Library::new("libGL.so.1").or_else(|_| libloading::Library::new("libOpenGL.so.0")).ok());
            let Some(gl) = gl else { return std::ptr::null_mut() };
            gl.get::<*mut c_void>(CStr::from_ptr(name).to_bytes_with_nul()).map(|s| *s).unwrap_or(std::ptr::null_mut())
        }
    }
}

struct Inner {
    mpv: Arc<Mpv>,
    ctx: *mut mpv::mpv_render_context,
    fbo: Option<glow::Framebuffer>,
    tex: Option<glow::Texture>,
    size: (i32, i32),
    failed: Option<String>,
    egui_ctx: egui::Context,
}

unsafe impl Send for Inner {}

#[derive(Clone)]
pub struct VideoRenderer {
    inner: Arc<Mutex<Inner>>,
}

impl VideoRenderer {
    pub fn new(mpv: Arc<Mpv>, egui_ctx: egui::Context) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                mpv,
                ctx: std::ptr::null_mut(),
                fbo: None,
                tex: None,
                size: (0, 0),
                failed: None,
                egui_ctx,
            })),
        }
    }

    pub fn error(&self) -> Option<String> {
        self.inner.lock().ok().and_then(|i| i.failed.clone())
    }

    /// mpv's render context exists (or could not be made). Without it mpv
    /// can't open a video ("No render context set"), so it has to exist
    /// before the first file loads, not only once a video is on screen.
    pub fn ready(&self) -> bool {
        self.inner.lock().map(|i| !i.ctx.is_null() || i.failed.is_some()).unwrap_or(true)
    }

    /// Adds a paint callback that draws the current video frame into `rect`.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect) {
        let inner = self.inner.clone();
        let cb = egui::PaintCallback {
            rect,
            callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, p| {
                if let Ok(mut i) = inner.lock() {
                    let vp = info.viewport_in_pixels();
                    unsafe { i.render(p.gl(), vp) };
                }
            })),
        };
        painter.add(cb);
    }

    /// Must be called with the GL context current (eframe `on_exit`).
    pub fn destroy(&self, gl: Option<&glow::Context>) {
        if let Ok(mut i) = self.inner.lock() {
            unsafe {
                if !i.ctx.is_null() {
                    (i.mpv.api.render_free)(i.ctx);
                    i.ctx = std::ptr::null_mut();
                }
                if let Some(gl) = gl {
                    if let Some(f) = i.fbo.take() {
                        gl.delete_framebuffer(f);
                    }
                    if let Some(t) = i.tex.take() {
                        gl.delete_texture(t);
                    }
                }
            }
        }
    }
}

impl Inner {
    unsafe fn init(&mut self) {
        unsafe extern "C" fn on_update(d: *mut c_void) {
            let ctx = unsafe { &*(d as *const egui::Context) };
            ctx.request_repaint();
        }
        let api_type = CString::new("opengl").unwrap();
        #[cfg(any(windows, target_os = "android", target_os = "linux"))]
        let gpa: Option<mpv::GetProcAddressFn> = Some(gl_loader::get_proc_address);
        #[cfg(not(any(windows, target_os = "android", target_os = "linux")))]
        let gpa: Option<mpv::GetProcAddressFn> = None;
        let mut init = mpv::OpenGlInitParams { get_proc_address: gpa, get_proc_address_ctx: std::ptr::null_mut() };
        let mut params = [
            mpv::RenderParam { ty: mpv::RENDER_PARAM_API_TYPE, data: api_type.as_ptr() as *mut c_void },
            mpv::RenderParam {
                ty: mpv::RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: &mut init as *mut _ as *mut c_void,
            },
            mpv::RenderParam { ty: mpv::RENDER_PARAM_INVALID, data: std::ptr::null_mut() },
        ];
        let mut ctx: *mut mpv::mpv_render_context = std::ptr::null_mut();
        let r = unsafe { (self.mpv.api.render_create)(&mut ctx, self.mpv.handle(), params.as_mut_ptr()) };
        if r < 0 || ctx.is_null() {
            self.failed = Some(tf!("mpv OpenGL-Renderer: {}", self.mpv.error_string(r)));
            return;
        }
        let boxed = Box::into_raw(Box::new(self.egui_ctx.clone())) as *mut c_void;
        unsafe { (self.mpv.api.render_set_update_callback)(ctx, Some(on_update), boxed) };
        self.ctx = ctx;
    }

    unsafe fn ensure_target(&mut self, gl: &glow::Context, w: i32, h: i32) {
        if self.fbo.is_some() && self.size == (w, h) {
            return;
        }
        unsafe {
            if self.tex.is_none() {
                self.tex = gl.create_texture().ok();
            }
            if self.fbo.is_none() {
                self.fbo = gl.create_framebuffer().ok();
            }
            let (Some(tex), Some(fbo)) = (self.tex, self.fbo) else { return };
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w,
                h,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        self.size = (w, h);
    }

    unsafe fn render(&mut self, gl: &glow::Context, vp: egui::epaint::ViewportInPixels) {
        if self.failed.is_some() {
            return;
        }
        if self.ctx.is_null() {
            unsafe { self.init() };
            if self.ctx.is_null() {
                return;
            }
        }
        let (w, h) = (vp.width_px.max(1), vp.height_px.max(1));
        unsafe {
            self.ensure_target(gl, w, h);
            let Some(fbo) = self.fbo else { return };
            let fbo_id = fbo.0.get() as i32;
            let mut target = mpv::OpenGlFbo { fbo: fbo_id, w, h, internal_format: 0 };
            let mut flip: i32 = 1;
            let mut block: i32 = 0;
            let mut params = [
                mpv::RenderParam { ty: mpv::RENDER_PARAM_OPENGL_FBO, data: &mut target as *mut _ as *mut c_void },
                mpv::RenderParam { ty: mpv::RENDER_PARAM_FLIP_Y, data: &mut flip as *mut _ as *mut c_void },
                mpv::RenderParam {
                    ty: mpv::RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                    data: &mut block as *mut _ as *mut c_void,
                },
                mpv::RenderParam { ty: mpv::RENDER_PARAM_INVALID, data: std::ptr::null_mut() },
            ];
            (self.mpv.api.render_update)(self.ctx);
            (self.mpv.api.render_render)(self.ctx, params.as_mut_ptr());

            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
            gl.disable(glow::SCISSOR_TEST);
            gl.blit_framebuffer(
                0,
                0,
                w,
                h,
                vp.left_px,
                vp.from_bottom_px,
                vp.left_px + w,
                vp.from_bottom_px + h,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            (self.mpv.api.render_report_swap)(self.ctx);
        }
    }
}

#[allow(dead_code)]
fn _assert_types(_: *const c_char) {}
