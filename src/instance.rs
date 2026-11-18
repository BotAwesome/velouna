//! One Velouna at a time (setting "Nur ein Velouna-Fenster", on by default).
//!
//! The first process listens on a loopback port derived from the data
//! folder (so a separate profile, `VELOUNA_DATA_DIR`, is a separate Velouna).
//! A later process hands its files over and quits; the running one opens
//! them and comes to the front. The handshake carries a random token kept
//! in the data folder, so only this user's Velouna is listened to, and a
//! foreign program that happens to hold the port is simply ignored.
//!
//! Explorer and file managers start one process per selected file, all at
//! once: files arriving within a short moment are opened as one list.

use eframe::egui;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAGIC: &str = "velouna-instance-1";
/// Files from several processes started together are collected this long.
const BATCH: Duration = Duration::from_millis(350);

#[derive(Default)]
struct Inbox {
    files: Vec<String>,
    /// A handover arrived (with or without files): come to the front.
    pending: bool,
    last: Option<Instant>,
}

static INBOX: Mutex<Inbox> = Mutex::new(Inbox { files: Vec::new(), pending: false, last: None });
static CTX: OnceLock<egui::Context> = OnceLock::new();
static LISTENING: Mutex<bool> = Mutex::new(false);

fn addr() -> SocketAddr {
    // FNV-1a of the data folder: a stable port per profile in 20000..30000,
    // below the ephemeral ranges of Windows (49152+) and Linux (32768+).
    let dir = crate::config::data_dir();
    let mut h: u32 = 0x811c_9dc5;
    for b in dir.to_string_lossy().as_bytes() {
        h = (h ^ *b as u32).wrapping_mul(0x0100_0193);
    }
    SocketAddr::from((Ipv4Addr::LOCALHOST, 20000 + (h % 10000) as u16))
}

fn token_path() -> PathBuf {
    crate::config::data_dir().join("instance.token")
}

fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    // RandomState is seeded from the OS; mixed with time and pid.
    let mut out = String::new();
    for i in 0..4u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// The setting, read straight from config.json before anything else loads
/// (a missing or older file means the default: on).
fn enabled() -> bool {
    std::fs::read_to_string(crate::config::data_dir().join("config.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["settings"]["single_instance"].as_bool())
        .unwrap_or(true)
}

/// Paths relative to this process's folder mean nothing to the running one.
fn absolute(files: &[String]) -> Vec<String> {
    files
        .iter()
        .map(|f| {
            if f.contains("://") {
                return f.clone();
            }
            std::path::absolute(f).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| f.clone())
        })
        .collect()
}

pub enum Start {
    /// The files went to the running Velouna: quit.
    HandedOver,
    /// Open the window; a note for the log, if something was odd.
    Run(Option<String>),
}

/// At startup, before the window opens and before the log file is created
/// (it would empty the running Velouna's log).
pub fn hand_over_or_listen(files: &[String]) -> Start {
    if !enabled() || listen() {
        return Start::Run(None);
    }
    // The port is taken: by our running Velouna (hand over) or by some other
    // program (then just run normally).
    match hand_over(&absolute(files)) {
        Ok(()) => Start::HandedOver,
        Err(e) => Start::Run(Some(format!("single instance: {e}; starting a separate window"))),
    }
}

/// Starts listening (startup, or when the setting is turned on). False when
/// the port is taken.
pub fn listen() -> bool {
    let mut on = LISTENING.lock().unwrap_or_else(|e| e.into_inner());
    if *on {
        return true;
    }
    let Ok(listener) = TcpListener::bind(addr()) else { return false };
    let token = new_token();
    if let Err(e) = std::fs::write(token_path(), &token) {
        log::warn!("single instance: token: {e}");
        return false;
    }
    *on = true;
    let _ = std::thread::Builder::new().name("velouna-instance".into()).spawn(move || {
        for conn in listener.incoming().flatten() {
            if let Err(e) = receive(conn, &token) {
                log::warn!("single instance: {e}");
            }
        }
    });
    true
}

fn receive(conn: TcpStream, token: &str) -> Result<(), String> {
    conn.set_read_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
    let mut out = conn.try_clone().map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(conn).lines();
    let mut next = || lines.next().and_then(Result::ok).unwrap_or_default();
    if next() != MAGIC {
        return Err("unknown client".into());
    }
    if next() != token {
        let _ = out.write_all(b"token\n");
        return Err("wrong token".into());
    }
    let files: Vec<String> = serde_json::from_str(&next()).unwrap_or_default();
    {
        let mut inbox = INBOX.lock().unwrap_or_else(|e| e.into_inner());
        inbox.files.extend(files);
        inbox.pending = true;
        inbox.last = Some(Instant::now());
    }
    let _ = out.write_all(b"ok\n");
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint_after(BATCH);
    }
    Ok(())
}

fn hand_over(files: &[String]) -> Result<(), String> {
    // Windows only lets the running Velouna come to the front if the process
    // the user just started allows it.
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn AllowSetForegroundWindow(pid: u32) -> i32;
        }
        const ASFW_ANY: u32 = u32::MAX;
        unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    }
    let payload = serde_json::to_string(files).map_err(|e| e.to_string())?;
    // The running Velouna may have bound the port a moment ago and not yet
    // written its token: retry briefly.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let r = (|| -> Result<String, String> {
            let token = std::fs::read_to_string(token_path()).map_err(|e| e.to_string())?;
            let mut s = TcpStream::connect_timeout(&addr(), Duration::from_secs(1)).map_err(|e| e.to_string())?;
            s.set_read_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
            s.write_all(format!("{MAGIC}\n{}\n{payload}\n", token.trim()).as_bytes()).map_err(|e| e.to_string())?;
            let mut reply = String::new();
            BufReader::new(s).read_line(&mut reply).map_err(|e| e.to_string())?;
            Ok(reply.trim().to_string())
        })();
        match r {
            Ok(reply) if reply == "ok" => return Ok(()),
            Ok(reply) if reply == "token" && Instant::now() < deadline => {}
            Ok(reply) => return Err(format!("no Velouna on the port (reply {reply:?})")),
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => {}
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Called every frame by the app: files handed over by other processes once
/// the batch is complete. `Some` (maybe empty) means: come to the front.
pub fn take(ctx: &egui::Context) -> Option<Vec<String>> {
    let _ = CTX.set(ctx.clone());
    let mut inbox = INBOX.lock().unwrap_or_else(|e| e.into_inner());
    if !inbox.pending {
        return None;
    }
    let waited = inbox.last.map(|t| t.elapsed()).unwrap_or(BATCH);
    if waited < BATCH {
        ctx.request_repaint_after(BATCH - waited);
        return None;
    }
    inbox.pending = false;
    Some(std::mem::take(&mut inbox.files))
}
