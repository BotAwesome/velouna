//! Minimal, dynamically loaded binding to libmpv (client + render API).
//!
//! The DLL is loaded at runtime with `libloading`, so no import library is
//! needed at build time. Search order: `VELOUR_LIBMPV` env var, next to the
//! executable, `./lib`, the working directory, then the system search path.

#![allow(non_camel_case_types, dead_code)]

use libloading::Library;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::PathBuf;

#[repr(C)]
pub struct mpv_handle {
    _p: [u8; 0],
}
#[repr(C)]
pub struct mpv_render_context {
    _p: [u8; 0],
}

pub const FMT_NONE: c_int = 0;
pub const FMT_STRING: c_int = 1;
pub const FMT_OSD_STRING: c_int = 2;
pub const FMT_FLAG: c_int = 3;
pub const FMT_INT64: c_int = 4;
pub const FMT_DOUBLE: c_int = 5;
pub const FMT_NODE: c_int = 6;
pub const FMT_NODE_ARRAY: c_int = 7;
pub const FMT_NODE_MAP: c_int = 8;
pub const FMT_BYTE_ARRAY: c_int = 9;

pub const RENDER_PARAM_INVALID: c_int = 0;
pub const RENDER_PARAM_API_TYPE: c_int = 1;
pub const RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
pub const RENDER_PARAM_OPENGL_FBO: c_int = 3;
pub const RENDER_PARAM_FLIP_Y: c_int = 4;
pub const RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;

#[repr(C)]
#[derive(Copy, Clone)]
pub union NodeU {
    pub string: *mut c_char,
    pub flag: c_int,
    pub int64: i64,
    pub double: f64,
    pub list: *mut NodeList,
    pub ba: *mut c_void,
}

#[repr(C)]
pub struct RawNode {
    pub u: NodeU,
    pub format: c_int,
}

#[repr(C)]
pub struct NodeList {
    pub num: c_int,
    pub values: *mut RawNode,
    pub keys: *mut *mut c_char,
}

#[repr(C)]
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct RawEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct RawEventEndFile {
    reason: c_int,
    error: c_int,
    playlist_entry_id: i64,
    playlist_insert_id: i64,
    playlist_insert_num_entries: c_int,
}

#[repr(C)]
pub struct RenderParam {
    pub ty: c_int,
    pub data: *mut c_void,
}

pub type GetProcAddressFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;

#[repr(C)]
pub struct OpenGlInitParams {
    pub get_proc_address: Option<GetProcAddressFn>,
    pub get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
pub struct OpenGlFbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

type FnCreate = unsafe extern "C" fn() -> *mut mpv_handle;
type FnInitialize = unsafe extern "C" fn(*mut mpv_handle) -> c_int;
type FnTerminateDestroy = unsafe extern "C" fn(*mut mpv_handle);
type FnSetOptionString = unsafe extern "C" fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int;
type FnCommand = unsafe extern "C" fn(*mut mpv_handle, *mut *const c_char) -> c_int;
type FnCommandAsync = unsafe extern "C" fn(*mut mpv_handle, u64, *mut *const c_char) -> c_int;
type FnSetProperty = unsafe extern "C" fn(*mut mpv_handle, *const c_char, c_int, *mut c_void) -> c_int;
type FnSetPropertyString = unsafe extern "C" fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int;
type FnGetProperty = unsafe extern "C" fn(*mut mpv_handle, *const c_char, c_int, *mut c_void) -> c_int;
type FnObserveProperty = unsafe extern "C" fn(*mut mpv_handle, u64, *const c_char, c_int) -> c_int;
type FnWaitEvent = unsafe extern "C" fn(*mut mpv_handle, f64) -> *mut RawEvent;
type FnSetWakeupCallback =
    unsafe extern "C" fn(*mut mpv_handle, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void);
type FnFree = unsafe extern "C" fn(*mut c_void);
type FnFreeNodeContents = unsafe extern "C" fn(*mut RawNode);
type FnErrorString = unsafe extern "C" fn(c_int) -> *const c_char;
type FnClientApiVersion = unsafe extern "C" fn() -> std::ffi::c_ulong;

type FnRenderCreate =
    unsafe extern "C" fn(*mut *mut mpv_render_context, *mut mpv_handle, *mut RenderParam) -> c_int;
type FnRenderRender = unsafe extern "C" fn(*mut mpv_render_context, *mut RenderParam) -> c_int;
type FnRenderSetUpdateCallback = unsafe extern "C" fn(
    *mut mpv_render_context,
    Option<unsafe extern "C" fn(*mut c_void)>,
    *mut c_void,
);
type FnRenderUpdate = unsafe extern "C" fn(*mut mpv_render_context) -> u64;
type FnRenderFree = unsafe extern "C" fn(*mut mpv_render_context);
type FnRenderReportSwap = unsafe extern "C" fn(*mut mpv_render_context);

pub struct Api {
    _lib: Library,
    pub path: PathBuf,
    create: FnCreate,
    initialize: FnInitialize,
    terminate_destroy: FnTerminateDestroy,
    set_option_string: FnSetOptionString,
    command: FnCommand,
    command_async: FnCommandAsync,
    set_property: FnSetProperty,
    set_property_string: FnSetPropertyString,
    get_property: FnGetProperty,
    observe_property: FnObserveProperty,
    wait_event: FnWaitEvent,
    set_wakeup_callback: FnSetWakeupCallback,
    free: FnFree,
    free_node_contents: FnFreeNodeContents,
    error_string: FnErrorString,
    client_api_version: FnClientApiVersion,
    pub render_create: FnRenderCreate,
    pub render_render: FnRenderRender,
    pub render_set_update_callback: FnRenderSetUpdateCallback,
    pub render_update: FnRenderUpdate,
    pub render_free: FnRenderFree,
    pub render_report_swap: FnRenderReportSwap,
}

fn candidate_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "android")]
    let names = ["libmpv.so"];
    #[cfg(not(target_os = "android"))]
    let names = ["libmpv-2.dll", "mpv-2.dll", "mpv-1.dll"];
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("VELOUR_LIBMPV").or_else(|_| std::env::var("VLB_LIBMPV")) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return vec![p];
        }
        dirs.push(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            dirs.push(d.to_path_buf());
            dirs.push(d.join("lib"));
            // target/debug -> project root/lib during development
            if let Some(root) = d.parent().and_then(|p| p.parent()) {
                dirs.push(root.join("lib"));
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("lib"));
        dirs.push(cwd);
    }
    let mut out = Vec::new();
    for d in &dirs {
        for n in names {
            let p = d.join(n);
            if p.is_file() {
                out.push(p);
            }
        }
    }
    // Finally let the OS search PATH.
    for n in names {
        out.push(PathBuf::from(n));
    }
    out
}

macro_rules! sym {
    ($lib:expr, $name:literal) => {
        unsafe {
            *$lib
                .get(concat!($name, "\0").as_bytes())
                .map_err(|e| tf!("Symbol {} fehlt: {e}", $name))?
        }
    };
}

impl Api {
    pub fn load() -> Result<Api, String> {
        let mut last_err = String::from(t!("libmpv-2.dll nicht gefunden"));
        for p in candidate_paths() {
            match unsafe { Library::new(&p) } {
                Ok(lib) => return Self::from_lib(lib, p),
                Err(e) => last_err = format!("{}: {e}", p.display()),
            }
        }
        Err(last_err)
    }

    fn from_lib(lib: Library, path: PathBuf) -> Result<Api, String> {
        let create: FnCreate = sym!(lib, "mpv_create");
        let initialize: FnInitialize = sym!(lib, "mpv_initialize");
        let terminate_destroy: FnTerminateDestroy = sym!(lib, "mpv_terminate_destroy");
        let set_option_string: FnSetOptionString = sym!(lib, "mpv_set_option_string");
        let command: FnCommand = sym!(lib, "mpv_command");
        let command_async: FnCommandAsync = sym!(lib, "mpv_command_async");
        let set_property: FnSetProperty = sym!(lib, "mpv_set_property");
        let set_property_string: FnSetPropertyString = sym!(lib, "mpv_set_property_string");
        let get_property: FnGetProperty = sym!(lib, "mpv_get_property");
        let observe_property: FnObserveProperty = sym!(lib, "mpv_observe_property");
        let wait_event: FnWaitEvent = sym!(lib, "mpv_wait_event");
        let set_wakeup_callback: FnSetWakeupCallback = sym!(lib, "mpv_set_wakeup_callback");
        let free: FnFree = sym!(lib, "mpv_free");
        let free_node_contents: FnFreeNodeContents = sym!(lib, "mpv_free_node_contents");
        let error_string: FnErrorString = sym!(lib, "mpv_error_string");
        let client_api_version: FnClientApiVersion = sym!(lib, "mpv_client_api_version");
        let render_create: FnRenderCreate = sym!(lib, "mpv_render_context_create");
        let render_render: FnRenderRender = sym!(lib, "mpv_render_context_render");
        let render_set_update_callback: FnRenderSetUpdateCallback =
            sym!(lib, "mpv_render_context_set_update_callback");
        let render_update: FnRenderUpdate = sym!(lib, "mpv_render_context_update");
        let render_free: FnRenderFree = sym!(lib, "mpv_render_context_free");
        let render_report_swap: FnRenderReportSwap = sym!(lib, "mpv_render_context_report_swap");
        Ok(Api {
            _lib: lib,
            path,
            create,
            initialize,
            terminate_destroy,
            set_option_string,
            command,
            command_async,
            set_property,
            set_property_string,
            get_property,
            observe_property,
            wait_event,
            set_wakeup_callback,
            free,
            free_node_contents,
            error_string,
            client_api_version,
            render_create,
            render_render,
            render_set_update_callback,
            render_update,
            render_free,
            render_report_swap,
        })
    }
}

/// Owned, Rust-side copy of an `mpv_node`.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    None,
    Str(String),
    Flag(bool),
    Int(i64),
    Double(f64),
    Array(Vec<Value>),
    Map(Vec<(String, Value)>),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Double(d) => Some(*d),
            Value::Int(i) => Some(*i as f64),
            Value::Flag(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::Str(s) => s.parse().ok(),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Double(d) => Some(*d as i64),
            Value::Flag(b) => Some(*b as i64),
            Value::Str(s) => s.parse().ok(),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Flag(b) => Some(*b),
            Value::Int(i) => Some(*i != 0),
            Value::Str(s) => Some(s == "yes" || s == "true"),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_array(&self) -> &[Value] {
        match self {
            Value::Array(a) => a,
            _ => &[],
        }
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn str_of(&self, key: &str) -> Option<String> {
        self.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
    }
    pub fn f64_of(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|v| v.as_f64())
    }
    pub fn i64_of(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(|v| v.as_i64())
    }
    pub fn bool_of(&self, key: &str) -> bool {
        self.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
    }
}

unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

unsafe fn node_to_value(n: &RawNode) -> Value {
    unsafe {
        match n.format {
            FMT_STRING | FMT_OSD_STRING => Value::Str(cstr(n.u.string)),
            FMT_FLAG => Value::Flag(n.u.flag != 0),
            FMT_INT64 => Value::Int(n.u.int64),
            FMT_DOUBLE => Value::Double(n.u.double),
            FMT_NODE_ARRAY => {
                let list = &*n.u.list;
                let mut v = Vec::with_capacity(list.num.max(0) as usize);
                for i in 0..list.num.max(0) as usize {
                    v.push(node_to_value(&*list.values.add(i)));
                }
                Value::Array(v)
            }
            FMT_NODE_MAP => {
                let list = &*n.u.list;
                let mut v = Vec::with_capacity(list.num.max(0) as usize);
                for i in 0..list.num.max(0) as usize {
                    let k = cstr(*list.keys.add(i));
                    v.push((k, node_to_value(&*list.values.add(i))));
                }
                Value::Map(v)
            }
            _ => Value::None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Shutdown,
    StartFile,
    EndFile { reason: i32, error: i32 },
    FileLoaded,
    VideoReconfig,
    AudioReconfig,
    Seek,
    PlaybackRestart,
    PropertyChange { name: String, value: Value },
    CommandReply { userdata: u64, error: i32 },
    Other(i32),
}

pub const END_EOF: i32 = 0;
pub const END_STOP: i32 = 2;
pub const END_ERROR: i32 = 4;

pub struct Mpv {
    pub api: &'static Api,
    handle: *mut mpv_handle,
}

unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

struct ArgV {
    _owned: Vec<CString>,
    ptrs: Vec<*const c_char>,
}

fn argv(args: &[&str]) -> ArgV {
    let owned: Vec<CString> = args
        .iter()
        .map(|a| CString::new(a.replace('\0', "")).unwrap_or_default())
        .collect();
    let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    ArgV { _owned: owned, ptrs }
}

fn c(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

impl Mpv {
    /// Creates (but does not initialize) an mpv instance.
    pub fn create() -> Result<Mpv, String> {
        let api: &'static Api = Box::leak(Box::new(Api::load()?));
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err(t!("mpv_create() fehlgeschlagen").into());
        }
        Ok(Mpv { api, handle })
    }

    pub fn handle(&self) -> *mut mpv_handle {
        self.handle
    }

    pub fn initialize(&self) -> Result<(), String> {
        let r = unsafe { (self.api.initialize)(self.handle) };
        self.check(r)
    }

    pub fn error_string(&self, code: i32) -> String {
        unsafe { cstr((self.api.error_string)(code)) }
    }

    fn check(&self, r: c_int) -> Result<(), String> {
        if r < 0 { Err(self.error_string(r)) } else { Ok(()) }
    }

    pub fn api_version(&self) -> (u32, u32) {
        let v = unsafe { (self.api.client_api_version)() } as u64;
        ((v >> 16) as u32, (v & 0xffff) as u32)
    }

    pub fn set_option(&self, name: &str, value: &str) -> Result<(), String> {
        let (n, v) = (c(name), c(value));
        let r = unsafe { (self.api.set_option_string)(self.handle, n.as_ptr(), v.as_ptr()) };
        self.check(r)
    }

    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let mut a = argv(args);
        let r = unsafe { (self.api.command)(self.handle, a.ptrs.as_mut_ptr()) };
        self.check(r)
    }

    pub fn command_async(&self, userdata: u64, args: &[&str]) -> Result<(), String> {
        let mut a = argv(args);
        let r = unsafe { (self.api.command_async)(self.handle, userdata, a.ptrs.as_mut_ptr()) };
        self.check(r)
    }

    pub fn set_str(&self, name: &str, value: &str) -> Result<(), String> {
        let (n, v) = (c(name), c(value));
        let r = unsafe { (self.api.set_property_string)(self.handle, n.as_ptr(), v.as_ptr()) };
        self.check(r)
    }

    pub fn set_f64(&self, name: &str, value: f64) -> Result<(), String> {
        let n = c(name);
        let mut v = value;
        let r = unsafe {
            (self.api.set_property)(self.handle, n.as_ptr(), FMT_DOUBLE, &mut v as *mut f64 as *mut c_void)
        };
        self.check(r)
    }

    pub fn set_i64(&self, name: &str, value: i64) -> Result<(), String> {
        let n = c(name);
        let mut v = value;
        let r = unsafe {
            (self.api.set_property)(self.handle, n.as_ptr(), FMT_INT64, &mut v as *mut i64 as *mut c_void)
        };
        self.check(r)
    }

    pub fn set_flag(&self, name: &str, value: bool) -> Result<(), String> {
        let n = c(name);
        let mut v: c_int = value as c_int;
        let r = unsafe {
            (self.api.set_property)(self.handle, n.as_ptr(), FMT_FLAG, &mut v as *mut c_int as *mut c_void)
        };
        self.check(r)
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        let n = c(name);
        let mut node = RawNode { u: NodeU { int64: 0 }, format: FMT_NONE };
        let r = unsafe {
            (self.api.get_property)(self.handle, n.as_ptr(), FMT_NODE, &mut node as *mut RawNode as *mut c_void)
        };
        if r < 0 {
            return None;
        }
        let v = unsafe { node_to_value(&node) };
        unsafe { (self.api.free_node_contents)(&mut node) };
        Some(v)
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.get(name).and_then(|v| v.as_f64())
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        let n = c(name);
        let mut p: *mut c_char = std::ptr::null_mut();
        let r = unsafe {
            (self.api.get_property)(self.handle, n.as_ptr(), FMT_STRING, &mut p as *mut *mut c_char as *mut c_void)
        };
        if r < 0 || p.is_null() {
            return None;
        }
        let s = unsafe { cstr(p) };
        unsafe { (self.api.free)(p as *mut c_void) };
        Some(s)
    }

    pub fn observe(&self, id: u64, name: &str) {
        let n = c(name);
        unsafe { (self.api.observe_property)(self.handle, id, n.as_ptr(), FMT_NODE) };
    }

    /// Non-blocking poll for the next event.
    pub fn poll_event(&self) -> Option<Event> {
        let ev = unsafe { &*(self.api.wait_event)(self.handle, 0.0) };
        let e = match ev.event_id {
            0 => return None,
            1 => Event::Shutdown,
            5 => Event::CommandReply { userdata: ev.reply_userdata, error: ev.error },
            6 => Event::StartFile,
            7 => {
                let d = unsafe { &*(ev.data as *const RawEventEndFile) };
                Event::EndFile { reason: d.reason, error: d.error }
            }
            8 => Event::FileLoaded,
            17 => Event::VideoReconfig,
            18 => Event::AudioReconfig,
            20 => Event::Seek,
            21 => Event::PlaybackRestart,
            22 => {
                let p = unsafe { &*(ev.data as *const RawEventProperty) };
                let name = unsafe { cstr(p.name) };
                let value = if p.format == FMT_NODE && !p.data.is_null() {
                    unsafe { node_to_value(&*(p.data as *const RawNode)) }
                } else {
                    Value::None
                };
                Event::PropertyChange { name, value }
            }
            other => Event::Other(other),
        };
        Some(e)
    }

    /// Calls `f` from an mpv thread whenever new events are available.
    pub fn set_wakeup_callback(&self, f: Box<dyn Fn() + Send + Sync>) {
        unsafe extern "C" fn tramp(d: *mut c_void) {
            let f = unsafe { &*(d as *const Box<dyn Fn() + Send + Sync>) };
            f();
        }
        let raw = Box::into_raw(Box::new(f)) as *mut c_void;
        unsafe { (self.api.set_wakeup_callback)(self.handle, Some(tramp), raw) };
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        unsafe { (self.api.terminate_destroy)(self.handle) };
    }
}
