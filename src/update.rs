//! Updates from GitHub releases: the newest release's text is the changelog,
//! its assets are the Windows setup (`Velour-Setup-<v>.exe`) and the Android
//! APK. Downloaded into the cache, checked against GitHub's SHA-256 digest,
//! then handed to the installer (Windows: setup `--quiet --relaunch`;
//! Android: the system package installer; Linux: the .deb through
//! `pkexec apt-get`, the tar.gz through its `install.sh`, then a restart).
//! A release without a Linux file counts as "up to date" on Linux.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

pub const REPO: &str = "BotAwesome/velour";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// API root. For tests `VELOUR_UPDATE_API` or a file `update-api.txt` in
/// the data folder points it elsewhere (on a phone only `adb run-as` with a
/// debug build can put that file there).
fn api() -> String {
    if let Ok(v) = std::env::var("VELOUR_UPDATE_API") {
        return v;
    }
    if let Ok(v) = std::fs::read_to_string(crate::config::data_dir().join("update-api.txt")) {
        let v = v.trim();
        if !v.is_empty() {
            return v.to_string();
        }
    }
    "https://api.github.com".into()
}

pub fn releases_page() -> String {
    format!("https://github.com/{REPO}/releases")
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    /// The file for this platform, if the release has one.
    pub asset: Option<Asset>,
}

#[derive(Clone, Debug)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(Release),
    Ready(Release, PathBuf),
    Installing(Release),
    Failed(String),
}

enum Msg {
    Checked(Result<Option<Release>, String>),
    Downloaded(Result<PathBuf, String>),
    #[cfg(target_os = "linux")]
    Installed(Result<(), String>),
}

pub struct Updater {
    pub status: Status,
    /// The last check was started by the user (show "up to date" / errors).
    pub manual: bool,
    /// The automatic check of this session is done (or not due).
    pub auto_done: bool,
    /// A release was found in the background: show the dialog when the
    /// user is not watching something.
    pub prompt: bool,
    /// Linux: the update is installed; start the new version and quit.
    pub restart: bool,
    rx: Option<Receiver<Msg>>,
    pub done: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            manual: false,
            auto_done: false,
            prompt: false,
            restart: false,
            rx: None,
            done: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// "v1.2.3", "1.2", "1.2.3-beta" → [1, 2, 3].
pub fn parse_version(s: &str) -> [u64; 3] {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+', ' ']).next().unwrap_or("");
    let mut v = [0; 3];
    for (i, p) in core.split('.').take(3).enumerate() {
        v[i] = p.parse().unwrap_or(0);
    }
    v
}

pub fn newer(a: &str, than: &str) -> bool {
    parse_version(a) > parse_version(than)
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout_read(Duration::from_secs(30)).user_agent(&format!("Velour/{CURRENT}")).build()
}

/// Linux: installed from the .deb (in /opt/velour) rather than the per-user
/// tar.gz.
#[cfg(target_os = "linux")]
pub fn linux_deb() -> bool {
    std::env::current_exe().is_ok_and(|p| p.starts_with("/opt/velour"))
}

/// Whether a release file is the one for this platform and install type.
fn wanted(name: &str) -> bool {
    let n = name.to_lowercase();
    #[cfg(target_os = "linux")]
    {
        if linux_deb() { n.ends_with("_amd64.deb") } else { n.ends_with("-linux-x86_64.tar.gz") }
    }
    #[cfg(not(target_os = "linux"))]
    {
        if crate::platform::MOBILE { n.ends_with(".apk") } else { n.ends_with(".exe") && n.contains("setup") }
    }
}

fn pick_asset(assets: &[serde_json::Value]) -> Option<Asset> {
    let want = |name: &str| wanted(name);
    assets.iter().find(|a| a["name"].as_str().is_some_and(want)).map(|a| Asset {
        name: a["name"].as_str().unwrap_or_default().to_string(),
        url: a["browser_download_url"].as_str().unwrap_or_default().to_string(),
        size: a["size"].as_u64().unwrap_or(0),
        sha256: a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(|d| d.to_lowercase()),
    })
}

/// The newest published release, if it is newer than this build.
fn fetch_latest() -> Result<Option<Release>, String> {
    let url = format!("{}/repos/{REPO}/releases/latest", api());
    let resp = agent().get(&url).set("Accept", "application/vnd.github+json").call();
    let json: serde_json::Value = match resp {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        // No release published yet.
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(ureq::Error::Status(code, _)) => return Err(format!("GitHub: HTTP {code}")),
        Err(e) => return Err(e.to_string()),
    };
    let tag = json["tag_name"].as_str().unwrap_or_default();
    if tag.is_empty() || !newer(tag, CURRENT) {
        return Ok(None);
    }
    let assets = json["assets"].as_array().cloned().unwrap_or_default();
    let asset = pick_asset(&assets);
    // Releases are made for Windows and Android first; one without a Linux
    // package is nothing a Linux user can install.
    if cfg!(target_os = "linux") && asset.is_none() {
        return Ok(None);
    }
    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        notes: json["body"].as_str().unwrap_or_default().replace("\r\n", "\n"),
        page: json["html_url"].as_str().map(str::to_string).unwrap_or_else(releases_page),
        asset,
    }))
}

fn download(asset: &Asset, done: &AtomicU64, total: &AtomicU64, cancel: &AtomicBool, ctx: &eframe::egui::Context) -> Result<PathBuf, String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let dir = crate::config::cache_dir().join("updates");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Only a plain file name from the release, never a path.
    let name: String = asset.name.chars().filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c)).collect();
    let path = dir.join(if name.is_empty() { "update.bin".into() } else { name });
    let part = path.with_extension("part");
    let resp = agent().get(&asset.url).call().map_err(|e| e.to_string())?;
    let len = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(asset.size);
    total.store(len, Ordering::Relaxed);
    let mut reader = resp.into_reader();
    let mut file = std::io::BufWriter::new(std::fs::File::create(&part).map_err(|e| e.to_string())?);
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut n_total = 0u64;
    let mut last_paint = std::time::Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err(String::new());
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hash.update(&buf[..n]);
        n_total += n as u64;
        done.store(n_total, Ordering::Relaxed);
        if last_paint.elapsed() > Duration::from_millis(100) {
            last_paint = std::time::Instant::now();
            ctx.request_repaint();
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if let Some(want) = &asset.sha256 {
        if *want != got {
            let _ = std::fs::remove_file(&part);
            return Err(crate::i18n::tr("Die heruntergeladene Datei ist beschädigt (Prüfsumme stimmt nicht).").to_string());
        }
    }
    if asset.size > 0 && n_total != asset.size {
        let _ = std::fs::remove_file(&part);
        return Err(crate::i18n::tr("Der Download ist unvollständig.").to_string());
    }
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    log::info!("update {} downloaded, sha256 {got}", asset.name);
    Ok(path)
}

/// A downloaded update that should survive an app restart.
#[derive(serde::Serialize, serde::Deserialize)]
struct Pending {
    release: Release,
    path: PathBuf,
}

fn updates_dir() -> PathBuf {
    crate::config::cache_dir().join("updates")
}

impl Updater {
    /// Android kills and restarts the app when the user allows it to install
    /// apps: remember the downloaded update so the dialog comes back.
    pub fn remember_ready(&self) {
        if let Status::Ready(release, path) = &self.status {
            if let Ok(s) = serde_json::to_string(&Pending { release: release.clone(), path: path.clone() }) {
                let _ = std::fs::write(updates_dir().join("pending.json"), s);
            }
        }
    }

    /// At startup: picks up a remembered update (true = show the dialog);
    /// otherwise clears old downloads.
    pub fn restore(&mut self) -> bool {
        let dir = updates_dir();
        let marker = dir.join("pending.json");
        let pending: Option<Pending> = std::fs::read_to_string(&marker).ok().and_then(|s| serde_json::from_str(&s).ok());
        let _ = std::fs::remove_file(&marker);
        if let Some(p) = pending {
            let size_ok = std::fs::metadata(&p.path)
                .map(|m| p.release.asset.as_ref().is_none_or(|a| a.size == 0 || a.size == m.len()))
                .unwrap_or(false);
            if newer(&p.release.version, CURRENT) && p.path.starts_with(&dir) && size_ok {
                self.status = Status::Ready(p.release, p.path);
                return true;
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        false
    }

    pub fn busy(&self) -> bool {
        matches!(self.status, Status::Checking | Status::Downloading(_) | Status::Installing(_))
    }

    pub fn check(&mut self, ctx: &eframe::egui::Context, manual: bool) {
        if self.busy() {
            self.manual |= manual;
            return;
        }
        self.manual = manual;
        self.status = Status::Checking;
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = fetch_latest();
            if let Err(e) = &r {
                log::warn!("update check: {e}");
            }
            let _ = tx.send(Msg::Checked(r));
            ctx.request_repaint();
        });
    }

    pub fn download(&mut self, ctx: &eframe::egui::Context) {
        let Status::Available(rel) = &self.status else { return };
        let Some(asset) = rel.asset.clone() else { return };
        self.status = Status::Downloading(rel.clone());
        self.done.store(0, Ordering::Relaxed);
        self.total.store(asset.size, Ordering::Relaxed);
        self.cancel.store(false, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let (done, total, cancel, ctx) = (self.done.clone(), self.total.clone(), self.cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = download(&asset, &done, &total, &cancel, &ctx);
            let _ = tx.send(Msg::Downloaded(r));
            ctx.request_repaint();
        });
    }

    pub fn cancel_download(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Picks up finished background work; true when a new release was found.
    pub fn poll(&mut self) -> bool {
        #[cfg(target_os = "android")]
        if let Status::Installing(rel) = &self.status {
            // PackageInstaller: -1 running / waiting for the user, 0 done
            // (the app is replaced and restarted), 3 aborted by the user.
            if let Some((code, msg)) = crate::platform::android::install_status() {
                match code {
                    -1 | 0 => {}
                    3 => self.status = Status::Available(rel.clone()),
                    // STATUS_FAILURE_CONFLICT: signed with another key (0.1.0
                    // used the debug key) – only a fresh install helps.
                    5 => {
                        self.status = Status::Failed(
                            t!("Diese Version ist anders signiert und lässt sich nicht über die installierte Version installieren. Deinstalliere Velour und installiere die APK von GitHub neu.").to_string(),
                        )
                    }
                    _ => self.status = Status::Failed(if msg.is_empty() { format!("PackageInstaller {code}") } else { msg }),
                }
            }
        }
        let Some(rx) = &self.rx else { return false };
        let Ok(msg) = rx.try_recv() else { return false };
        self.rx = None;
        match msg {
            Msg::Checked(Ok(Some(rel))) => {
                self.status = Status::Available(rel);
                return true;
            }
            Msg::Checked(Ok(None)) => self.status = Status::UpToDate,
            Msg::Checked(Err(e)) => self.status = Status::Failed(e),
            Msg::Downloaded(Ok(path)) => {
                if let Status::Downloading(rel) = &self.status {
                    self.status = Status::Ready(rel.clone(), path);
                }
            }
            Msg::Downloaded(Err(e)) => {
                self.status = match (&self.status, e.is_empty()) {
                    // Cancelled.
                    (Status::Downloading(rel), true) => Status::Available(rel.clone()),
                    _ => Status::Failed(e),
                };
            }
            #[cfg(target_os = "linux")]
            Msg::Installed(Ok(())) => self.restart = true,
            #[cfg(target_os = "linux")]
            Msg::Installed(Err(e)) => self.status = Status::Failed(e),
        }
        false
    }

    /// Starts the installer. Windows: the app must quit right after (true).
    pub fn install(&mut self) -> bool {
        let Status::Ready(rel, path) = &self.status else { return false };
        let (rel, path) = (rel.clone(), path.clone());
        #[cfg(windows)]
        {
            match std::process::Command::new(&path).args(["--quiet", "--relaunch"]).spawn() {
                Ok(_) => {
                    self.status = Status::Installing(rel);
                    return true;
                }
                Err(e) => self.status = Status::Failed(e.to_string()),
            }
        }
        #[cfg(target_os = "android")]
        {
            crate::platform::android::install_apk(&path.to_string_lossy());
            self.status = Status::Installing(rel);
        }
        #[cfg(target_os = "linux")]
        {
            self.status = Status::Installing(rel);
            let (tx, rx) = channel();
            self.rx = Some(rx);
            std::thread::spawn(move || {
                let r = linux::install(&path);
                if let Err(e) = &r {
                    log::warn!("update install: {e}");
                }
                let _ = tx.send(Msg::Installed(r));
            });
        }
        #[cfg(not(any(windows, target_os = "android", target_os = "linux")))]
        {
            let _ = (rel, path);
        }
        false
    }
}

#[cfg(target_os = "linux")]
pub mod linux {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// The executable to start after the update, read at startup: once the
    /// package has replaced it, /proc/self/exe ends in " (deleted)".
    fn exe() -> PathBuf {
        static EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        EXE.get_or_init(|| std::env::current_exe().unwrap_or_else(|_| "velour".into())).clone()
    }

    /// Remembers the executable path before anything replaces it.
    pub fn init() {
        let _ = exe();
    }

    /// .deb: apt through pkexec (the desktop asks for the password);
    /// tar.gz: unpacked next to the download, then its install.sh.
    pub fn install(path: &Path) -> Result<(), String> {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if name.ends_with(".deb") {
            let st = Command::new("pkexec")
                .args(["apt-get", "install", "-y", "--allow-downgrades"])
                .arg(path)
                .status()
                .map_err(|e| format!("pkexec: {e}"))?;
            return match st.code() {
                Some(0) => Ok(()),
                // 126: the password dialog was closed, 127: not authorized.
                Some(126 | 127) => Err(crate::i18n::tr("Die Installation wurde abgebrochen.").to_string()),
                _ => Err(format!("{} sudo apt install {}", crate::i18n::tr("Die Installation ist fehlgeschlagen. Installiere das Paket von Hand mit:"), path.display())),
            };
        }
        let dir = path.with_extension("").with_extension("unpacked");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let st = Command::new("tar").arg("xzf").arg(path).arg("-C").arg(&dir).status().map_err(|e| format!("tar: {e}"))?;
        if !st.success() {
            return Err(format!("tar: {st}"));
        }
        let script = std::fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path().join("install.sh"))
            .find(|p| p.is_file())
            .ok_or_else(|| "install.sh missing".to_string())?;
        let st = Command::new("sh").arg(&script).status().map_err(|e| format!("install.sh: {e}"))?;
        if st.success() { Ok(()) } else { Err(format!("install.sh: {st}")) }
    }

    /// Starts the new version once this process has quit.
    pub fn restart() {
        let pid = std::process::id().to_string();
        let script = r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec "$0""#;
        let _ = Command::new("sh").arg("-c").arg(script).arg(exe()).arg(pid).spawn();
    }
}

// ------------------------------------------------------------ changelog

/// The changelog shipped with this build (shown after an update).
pub const CHANGELOG: &str = include_str!("../CHANGELOG.md");

/// Sections of CHANGELOG.md for versions newer than `since` and not newer
/// than this build, as markdown (headings included).
pub fn changes_since(since: &str) -> String {
    let mut out = String::new();
    let mut take = false;
    for line in CHANGELOG.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            let v = h.trim().trim_start_matches('[').split([']', ' ']).next().unwrap_or("");
            take = newer(v, since) && !newer(v, CURRENT);
        }
        if take {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("v0.1.0-beta", "0.1.0"));
        assert_eq!(parse_version("v1.2.3-rc1"), [1, 2, 3]);
    }
}
