//! Plugin repos (velouna-plugins/docs/repos.md): a signed `repo.json` on any static
//! web address, the packages it lists, and building such a folder.
//!
//! Trust works like SSH host keys: the key a repo names is shown when it is
//! added and kept from then on. Every fetch checks the signature of
//! `repo.json` with the kept key, every download its size and SHA-256.

use super::{Kind, Manifest, Text};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The official repo; it comes with Velouna and can be hidden, not removed.
pub const OFFICIAL: &str = "https://github.com/BotAwesome/velouna-plugins";
/// Its key, so not even the first fetch has to be trusted blindly.
pub const OFFICIAL_KEY: &str = "ed25519:NIsx6FGSN40zF7+GKzJEvPPPDkYEO+pDE7xY9+drOIQ=";
/// repo.json format this Velouna reads.
pub const FORMAT: u32 = 1;
const MAX_INDEX: u64 = 4 << 20;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Index {
    pub format: u32,
    pub name: String,
    #[serde(default)]
    pub maintainer: String,
    #[serde(default)]
    pub homepage: String,
    /// "ed25519:<base64>" of the key that signs this file.
    pub key: String,
    pub plugins: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: Text,
    #[serde(default)]
    pub description: Text,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// Picture, relative to repo.json.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preview: String,
    /// Newest first.
    pub versions: Vec<Version>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Version {
    pub version: String,
    pub api: u32,
    #[serde(default)]
    pub velouna: String,
    /// Package, relative to repo.json.
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub changes: Text,
}

impl Version {
    /// Runs on this Velouna (interface and minimum version).
    pub fn fits(&self) -> bool {
        let api = self.api <= super::API && self.api + 1 >= super::API;
        let min = self.velouna.strip_prefix(">=").is_none_or(|m| !super::version_lt(env!("CARGO_PKG_VERSION"), m.trim()));
        api && min
    }
}

impl Entry {
    /// The newest version that runs here.
    pub fn best(&self) -> Option<&Version> {
        self.versions.iter().filter(|v| v.fits()).max_by(|a, b| {
            if super::version_lt(&a.version, &b.version) { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater }
        })
    }
}

/// A repo the user added, as kept in config.json.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Repo {
    /// Address of repo.json.
    pub url: String,
    pub name: String,
    /// The key seen when it was added; later fetches must match it.
    pub key: String,
    /// Left out of the catalog (the official one can't be removed).
    pub hidden: bool,
    /// Added by the link of single plugin folders: only these plugins of
    /// the repo are shown. Empty: all of them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
}

impl Repo {
    pub fn official() -> Repo {
        Repo { url: resolve(OFFICIAL).unwrap_or_default(), name: "Velouna".into(), key: OFFICIAL_KEY.into(), hidden: false, only: Vec::new() }
    }

    pub fn is_official(&self) -> bool {
        self.url == Repo::official().url
    }

    /// Whether plugin `id` of this repo is shown (see `only`).
    pub fn shows(&self, id: &str) -> bool {
        self.only.is_empty() || self.only.iter().any(|o| o == id)
    }
}

/// A repo.json fetched and checked.
#[derive(Clone, Debug, PartialEq)]
pub struct Fetched {
    pub url: String,
    pub index: Index,
    /// The link was to this one plugin of the repo, not to the repo.
    pub only: Option<String>,
}

/// The address of repo.json for what the user typed: the file itself, the
/// folder it is in, or a GitHub repo (its default branch).
pub fn resolve(input: &str) -> Result<String, String> {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err(t!("Bitte eine Adresse eingeben").into());
    }
    let s = if s.contains("://") { s.to_string() } else { format!("https://{s}") };
    let gh = s.strip_prefix("https://github.com/").or_else(|| s.strip_prefix("https://www.github.com/"));
    let s = match gh {
        Some(rest) => {
            let parts: Vec<&str> = rest.split('/').collect();
            if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
                return Err(t!("GitHub-Adresse ohne Repo").into());
            }
            let repo = parts[1].trim_end_matches(".git");
            format!("https://raw.githubusercontent.com/{}/{repo}/HEAD/repo.json", parts[0])
        }
        None if s.ends_with(".json") => s,
        None => format!("{s}/repo.json"),
    };
    if !(s.starts_with("https://") || is_local_http(&s)) {
        return Err(tf!("Nur https-Adressen sind erlaubt: {}", s.escape_debug()));
    }
    Ok(s)
}

/// http only on this machine (for building and trying a repo).
fn is_local_http(url: &str) -> bool {
    ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|p| url.strip_prefix(p).is_some_and(|r| r.is_empty() || r.starts_with(':') || r.starts_with('/')))
}

/// `rel` (from repo.json) next to the index; nothing outside its folder.
pub fn join(index_url: &str, rel: &str) -> Result<String, String> {
    if rel.contains("://") || rel.starts_with('/') || rel.split('/').any(|p| p == ".." || p == ".") || rel.contains('\\') {
        return Err(tf!("Unzulässiger Pfad im Repo: {}", rel));
    }
    let base = &index_url[..index_url.rfind('/').map(|i| i + 1).unwrap_or(index_url.len())];
    Ok(format!("{base}{rel}"))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .redirects(5)
        .user_agent(&format!("Velouna/{}", env!("CARGO_PKG_VERSION")))
        .build()
}

fn get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    get_opt(url, limit)?.ok_or_else(|| tf!("Nicht gefunden: {}", url))
}

/// Like `get`, but `None` when the server says there is no such file.
fn get_opt(url: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
    let resp = match agent().get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(ureq::Error::Status(c, _)) => return Err(tf!("Server antwortet mit {}", c)),
        Err(ureq::Error::Transport(_)) => return Err(t!("Repo nicht erreichbar").to_string()),
    };
    let mut buf = Vec::new();
    resp.into_reader().take(limit + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if buf.len() as u64 > limit {
        return Err(t!("Datei ist zu groß").into());
    }
    Ok(Some(buf))
}

fn key_bytes(key: &str) -> Result<VerifyingKey, String> {
    let raw = key.strip_prefix("ed25519:").and_then(|k| B64.decode(k.trim()).ok()).ok_or_else(|| t!("Ungültiger Repo-Schlüssel").to_string())?;
    let arr: [u8; 32] = raw.try_into().map_err(|_| t!("Ungültiger Repo-Schlüssel").to_string())?;
    VerifyingKey::from_bytes(&arr).map_err(|_| t!("Ungültiger Repo-Schlüssel").to_string())
}

/// Short, readable fingerprint of a key for the "add repo" dialog.
pub fn fingerprint(key: &str) -> String {
    let h = Sha256::digest(key.as_bytes());
    h[..8].chunks(2).map(|c| format!("{:02X}{:02X}", c[0], c[1])).collect::<Vec<_>>().join(" ")
}

/// Checks `sig` (base64) over `msg` with `key`.
pub fn verify(key: &str, msg: &[u8], sig: &[u8]) -> Result<(), String> {
    let vk = key_bytes(key)?;
    let raw = B64.decode(String::from_utf8_lossy(sig).trim()).map_err(|_| t!("Signatur des Repos ist ungültig").to_string())?;
    let sig = Signature::from_slice(&raw).map_err(|_| t!("Signatur des Repos ist ungültig").to_string())?;
    vk.verify(msg, &sig).map_err(|_| t!("Signatur des Repos stimmt nicht: repo.json wurde verändert").to_string())
}

/// Parses and checks a repo.json against its signature and, once the repo
/// was added, against the key kept then.
pub fn check(bytes: &[u8], sig: &[u8], pinned: Option<&str>) -> Result<Index, String> {
    let index: Index = serde_json::from_slice(bytes).map_err(|e| tf!("repo.json: {}", e))?;
    if index.format != FORMAT {
        return Err(tf!("Repo-Format {} wird nicht unterstützt", index.format));
    }
    if let Some(p) = pinned.filter(|p| !p.is_empty()) {
        if p != index.key {
            return Err(t!("Der Schlüssel dieses Repos hat sich geändert. Updates daraus sind gesperrt.").into());
        }
    }
    verify(&index.key, bytes, sig)?;
    for e in &index.plugins {
        for v in &e.versions {
            join("https://x/repo.json", &v.url)?;
        }
    }
    Ok(index)
}

/// Downloads and checks a repo.json (blocking; run it off the UI thread).
pub fn fetch(url: &str, pinned: Option<&str>) -> Result<Fetched, String> {
    let bytes = get(url, MAX_INDEX)?;
    let sig = get(&format!("{url}.sig"), 1024).map_err(|e| tf!("Signatur fehlt ({})", e))?;
    Ok(Fetched { url: url.to_string(), index: check(&bytes, &sig, pinned)?, only: None })
}

/// Like `fetch`, but `None` when there is no repo.json at `url`.
fn fetch_opt(url: &str) -> Result<Option<Fetched>, String> {
    let Some(bytes) = get_opt(url, MAX_INDEX)? else { return Ok(None) };
    let sig = get(&format!("{url}.sig"), 1024).map_err(|e| tf!("Signatur fehlt ({})", e))?;
    Ok(Some(Fetched { url: url.to_string(), index: check(&bytes, &sig, None)?, only: None }))
}

/// Where a link points: a repo.json, or a folder below `root` (which may be
/// a repo's folder or one plugin's folder).
#[derive(Debug, PartialEq)]
enum Place {
    Index(String),
    Folder { root: String, path: String },
}

fn place(input: &str) -> Result<Place, String> {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err(t!("Bitte eine Adresse eingeben").into());
    }
    let s = if s.contains("://") { s.to_string() } else { format!("https://{s}") };
    if !(s.starts_with("https://") || is_local_http(&s)) {
        return Err(tf!("Nur https-Adressen sind erlaubt: {}", s.escape_debug()));
    }
    let parts = |rest: &str| -> Vec<String> { rest.split('/').filter(|p| !p.is_empty()).map(String::from).collect() };
    if let Some(rest) = s.strip_prefix("https://github.com/").or_else(|| s.strip_prefix("https://www.github.com/")) {
        let p = parts(rest);
        if p.len() < 2 {
            return Err(t!("GitHub-Adresse ohne Repo").into());
        }
        let (user, repo) = (&p[0], p[1].trim_end_matches(".git"));
        // github.com/u/r/tree/<branch>/<folder> or /blob/<branch>/<file>
        if p.len() >= 4 && (p[2] == "tree" || p[2] == "blob") {
            return Ok(Place::Folder { root: format!("https://raw.githubusercontent.com/{user}/{repo}/{}", p[3]), path: p[4..].join("/") });
        }
        return Ok(Place::Index(format!("https://raw.githubusercontent.com/{user}/{repo}/HEAD/repo.json")));
    }
    if let Some(rest) = s.strip_prefix("https://raw.githubusercontent.com/") {
        let p = parts(rest);
        if p.len() >= 3 {
            return Ok(Place::Folder { root: format!("https://raw.githubusercontent.com/{}/{}/{}", p[0], p[1], p[2]), path: p[3..].join("/") });
        }
    }
    let host_end = s.find("://").map(|i| i + 3).and_then(|i| s[i..].find('/').map(|j| i + j)).unwrap_or(s.len());
    Ok(Place::Folder { root: s[..host_end].to_string(), path: s[host_end..].trim_matches('/').to_string() })
}

fn under(root: &str, dir: &str, file: &str) -> String {
    if dir.is_empty() { format!("{root}/{file}") } else { format!("{root}/{dir}/{file}") }
}

/// Fetches what the user linked (blocking): a repo (its repo.json, its
/// folder or its GitHub page), or one plugin folder inside a repo. For a
/// plugin folder, the repo.json is looked up in the folders above, and the
/// result names the plugin in `only`.
pub fn probe(input: &str) -> Result<Fetched, String> {
    let (root, path) = match place(input)? {
        Place::Index(url) => return fetch(&url, None),
        Place::Folder { root, path } => (root, path),
    };
    if path.ends_with(".json") && !path.ends_with("plugin.json") {
        return fetch(&format!("{root}/{path}"), None);
    }
    let (folder, file_link) = match path.strip_suffix("plugin.json") {
        Some(f) => (f.trim_end_matches('/').to_string(), true),
        None => (path.clone(), false),
    };
    if !file_link {
        if let Some(f) = fetch_opt(&under(&root, &folder, "repo.json"))? {
            return Ok(f);
        }
    }
    let Some(manifest) = get_opt(&under(&root, &folder, "plugin.json"), 1 << 20)? else {
        return Err(t!("Unter dieser Adresse ist weder ein Repo noch ein Plugin").into());
    };
    let id = serde_json::from_slice::<serde_json::Value>(&manifest)
        .ok()
        .and_then(|v| v.get("id")?.as_str().map(String::from))
        .ok_or_else(|| t!("Die plugin.json in diesem Ordner hat keine id").to_string())?;
    // the repo is the nearest folder above with a repo.json
    let mut dir = folder.as_str();
    while !dir.is_empty() {
        dir = dir.rfind('/').map(|i| &dir[..i]).unwrap_or("");
        if let Some(mut f) = fetch_opt(&under(&root, dir, "repo.json"))? {
            if !f.index.plugins.iter().any(|e| e.id == id) {
                return Err(tf!("Das Repo „{}“ darüber enthält dieses Plugin nicht", f.index.name));
            }
            f.only = Some(id);
            return Ok(f);
        }
    }
    Err(t!("Zu diesem Plugin-Ordner wurde kein Repo gefunden").into())
}

/// Downloads one package into the cache and checks size and SHA-256
/// against the signed index (blocking).
pub fn download(index_url: &str, v: &Version) -> Result<PathBuf, String> {
    let url = join(index_url, &v.url)?;
    let bytes = get(&url, v.size.max(1))?;
    if bytes.len() as u64 != v.size {
        return Err(t!("Paket hat nicht die angegebene Größe").into());
    }
    let sum = hex(&Sha256::digest(&bytes));
    if !sum.eq_ignore_ascii_case(&v.sha256) {
        return Err(t!("Prüfsumme des Pakets stimmt nicht").into());
    }
    let dir = crate::config::cache_dir().join("plugin-downloads");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{sum}.{}", super::EXT));
    std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
    Ok(file)
}

/// Package and index must ask for the same permissions.
pub fn same_permissions(index: &[String], m: &Manifest) -> bool {
    let mut a: Vec<&String> = index.iter().collect();
    let mut b: Vec<&String> = m.permissions.iter().collect();
    a.sort();
    b.sort();
    a.dedup();
    b.dedup();
    a == b
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ------------------------------------------------------------- building

/// A new signing key: (secret for the key file, public "ed25519:…").
pub fn keygen() -> Result<(String, String), String> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| e.to_string())?;
    let sk = SigningKey::from_bytes(&seed);
    Ok((B64.encode(seed), public_key(&sk)))
}

pub fn public_key(sk: &SigningKey) -> String {
    format!("ed25519:{}", B64.encode(sk.verifying_key().to_bytes()))
}

pub fn load_key(file: &Path) -> Result<SigningKey, String> {
    let s = std::fs::read_to_string(file).map_err(|e| tf!("Schlüsseldatei {}: {}", file.display(), e))?;
    let raw = B64.decode(s.trim()).map_err(|_| t!("Schlüsseldatei ist ungültig").to_string())?;
    let seed: [u8; 32] = raw.try_into().map_err(|_| t!("Schlüsseldatei ist ungültig").to_string())?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn sign(sk: &SigningKey, msg: &[u8]) -> String {
    B64.encode(sk.sign(msg).to_bytes())
}

/// Zips a plugin folder (same bytes for the same files: fixed order and
/// times, so rebuilding an unchanged plugin gives the same checksum).
pub fn pack(src: &Path) -> Result<(Manifest, Vec<u8>), String> {
    let m = super::read_manifest(src)?;
    let mut files = Vec::new();
    walk(src, src, &mut files)?;
    files.sort();
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut out);
        let o = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        for rel in files {
            let data = std::fs::read(src.join(&rel)).map_err(|e| e.to_string())?;
            z.start_file(rel.as_str(), o).map_err(|e| e.to_string())?;
            z.write_all(&data).map_err(|e| e.to_string())?;
        }
        z.finish().map_err(|e| e.to_string())?;
    }
    Ok((m, out.into_inner()))
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for e in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            walk(root, &p, out)?;
        } else {
            let rel = p.strip_prefix(root).map_err(|e| e.to_string())?;
            out.push(rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"));
        }
    }
    Ok(())
}

/// What the repo is called, written into repo.json.
pub struct RepoInfo {
    pub name: String,
    pub maintainer: String,
    pub homepage: String,
}

/// Builds or updates a repo folder from plugin folders under `src`:
/// packs each plugin, keeps older versions already in `out`, writes
/// repo.json and its signature. Returns one line per plugin.
pub fn build(src: &Path, out: &Path, sk: &SigningKey, info: &RepoInfo) -> Result<Vec<String>, String> {
    let old: Option<Index> = std::fs::read(out.join("repo.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let mut plugins: Vec<Entry> = Vec::new();
    let mut report = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).filter(|p| p.join("plugin.json").is_file()).collect();
    dirs.sort();
    for dir in dirs {
        let (m, bytes) = pack(&dir)?;
        let rel_dir = format!("packages/{}", m.id);
        std::fs::create_dir_all(out.join(&rel_dir)).map_err(|e| e.to_string())?;
        let rel = format!("{rel_dir}/{}-{}.{}", m.id, m.version, super::EXT);
        let sum = hex(&Sha256::digest(&bytes));
        // A published version never changes: same number, other bytes is a mistake.
        let prev = old.as_ref().and_then(|o| o.plugins.iter().find(|e| e.id == m.id));
        if let Some(v) = prev.and_then(|e| e.versions.iter().find(|v| v.version == m.version)) {
            if v.sha256 != sum {
                return Err(tf!("{} {} ist schon veröffentlicht, aber geändert: bitte die Version erhöhen", m.id, m.version));
            }
        }
        std::fs::write(out.join(&rel), &bytes).map_err(|e| e.to_string())?;
        let preview = ["preview.webp", "preview.png", "preview.jpg"].iter().find(|f| dir.join(f).is_file()).map(|f| {
            let to = format!("{rel_dir}/{f}");
            let _ = std::fs::copy(dir.join(f), out.join(&to));
            to
        });
        let changes = std::fs::read(dir.join("changes.json")).ok().and_then(|b| serde_json::from_slice::<Text>(&b).ok()).unwrap_or_default();
        let mut versions = vec![Version {
            version: m.version.clone(),
            api: m.api,
            velouna: m.velouna.clone(),
            url: rel.clone(),
            sha256: sum,
            size: bytes.len() as u64,
            permissions: m.permissions.clone(),
            changes,
        }];
        // older versions stay listed while their files are there
        for v in prev.map(|e| e.versions.clone()).unwrap_or_default() {
            if v.version != m.version && out.join(&v.url).is_file() {
                versions.push(v);
            }
        }
        versions.sort_by(|a, b| if super::version_lt(&a.version, &b.version) { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less });
        report.push(format!("{} {} ({} KB)", m.id, m.version, bytes.len() / 1024));
        plugins.push(Entry {
            id: m.id.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            kind: m.kind,
            author: m.author.clone(),
            preview: preview.unwrap_or_default(),
            versions,
        });
    }
    let index = Index { format: FORMAT, name: info.name.clone(), maintainer: info.maintainer.clone(), homepage: info.homepage.clone(), key: public_key(sk), plugins };
    let mut json = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
    json.push(b'\n');
    std::fs::write(out.join("repo.json"), &json).map_err(|e| e.to_string())?;
    std::fs::write(out.join("repo.json.sig"), format!("{}\n", sign(sk, &json))).map_err(|e| e.to_string())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(resolve("github.com/BotAwesome/velouna-plugins").unwrap(), "https://raw.githubusercontent.com/BotAwesome/velouna-plugins/HEAD/repo.json");
        assert_eq!(resolve("https://github.com/a/b.git/").unwrap(), "https://raw.githubusercontent.com/a/b/HEAD/repo.json");
        assert_eq!(resolve("example.org/repo").unwrap(), "https://example.org/repo/repo.json");
        assert_eq!(resolve("https://example.org/x/repo.json").unwrap(), "https://example.org/x/repo.json");
        assert!(resolve("http://example.org/repo.json").is_err());
        assert!(resolve("http://127.0.0.1:8765").is_ok());
        assert!(resolve("http://127.0.0.1:87655").is_ok(), "{:?}", resolve("http://127.0.0.1:87655"));
        assert!(resolve("http://localhost.evil.org/repo.json").is_err());
        assert_eq!(join("https://e.org/r/repo.json", "packages/a/a-1.velounaplugin").unwrap(), "https://e.org/r/packages/a/a-1.velounaplugin");
        assert!(join("https://e.org/r/repo.json", "../x").is_err());
        assert!(join("https://e.org/r/repo.json", "https://evil.org/x").is_err());
        // links to one plugin's folder
        let f = |root: &str, path: &str| Place::Folder { root: root.into(), path: path.into() };
        let gh = "https://raw.githubusercontent.com/me/plugs/main";
        assert_eq!(place("https://github.com/me/plugs/tree/main/plugins/neon").unwrap(), f(gh, "plugins/neon"));
        assert_eq!(place("github.com/me/plugs/blob/main/plugins/neon/plugin.json").unwrap(), f(gh, "plugins/neon/plugin.json"));
        assert_eq!(place("https://raw.githubusercontent.com/me/plugs/main/plugins/neon/").unwrap(), f(gh, "plugins/neon"));
        assert_eq!(place("github.com/me/plugs").unwrap(), Place::Index("https://raw.githubusercontent.com/me/plugs/HEAD/repo.json".into()));
        assert_eq!(place("example.org/r/plugins/neon").unwrap(), f("https://example.org", "r/plugins/neon"));
        assert_eq!(place("http://127.0.0.1:8765/plugins/x").unwrap(), f("http://127.0.0.1:8765", "plugins/x"));
        assert_eq!(place("http://127.0.0.1:8765").unwrap(), f("http://127.0.0.1:8765", ""));
        assert!(place("http://example.org/plugins/x").is_err());
    }

    /// Builds a repo from a plugin folder, then checks it like Velouna does.
    #[test]
    fn build_and_check() {
        let dir = std::env::temp_dir().join(format!("velouna-repo-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src/viz");
        std::fs::create_dir_all(src.join("shaders")).unwrap();
        std::fs::write(src.join("plugin.json"), r#"{"id":"dev.test.viz","name":"Test","version":"1.0.0","kind":"visualizer","api":1}"#).unwrap();
        std::fs::write(src.join("shaders/a.glsl"), "void main(){}").unwrap();
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let (secret, public) = keygen().unwrap();
        let kf = dir.join("k");
        std::fs::write(&kf, &secret).unwrap();
        let sk = load_key(&kf).unwrap();
        assert_eq!(public_key(&sk), public);
        let info = RepoInfo { name: "Test".into(), maintainer: "me".into(), homepage: String::new() };
        build(&dir.join("src"), &out, &sk, &info).unwrap();
        let json = std::fs::read(out.join("repo.json")).unwrap();
        let sig = std::fs::read(out.join("repo.json.sig")).unwrap();
        let idx = check(&json, &sig, None).unwrap();
        assert_eq!(idx.plugins[0].versions[0].size, std::fs::metadata(out.join(&idx.plugins[0].versions[0].url)).unwrap().len());
        // same files, same package
        build(&dir.join("src"), &out, &sk, &info).unwrap();
        assert_eq!(std::fs::read(out.join("repo.json")).unwrap(), json);
        // pinned key must match
        assert!(check(&json, &sig, Some(&public)).is_ok());
        let (_, other) = keygen().unwrap();
        assert!(check(&json, &sig, Some(&other)).is_err());
        // a changed index fails the signature
        let mut bad = json.clone();
        let i = bad.iter().position(|b| *b == b'T').unwrap();
        bad[i] = b'X';
        assert!(check(&bad, &sig, None).is_err());
        // changed files under the same version are refused
        std::fs::write(src.join("shaders/a.glsl"), "void main(){ }").unwrap();
        assert!(build(&dir.join("src"), &out, &sk, &info).is_err());
        // a new version keeps the old one listed
        std::fs::write(src.join("plugin.json"), r#"{"id":"dev.test.viz","name":"Test","version":"1.1.0","kind":"visualizer","api":1}"#).unwrap();
        build(&dir.join("src"), &out, &sk, &info).unwrap();
        let idx = check(&std::fs::read(out.join("repo.json")).unwrap(), &std::fs::read(out.join("repo.json.sig")).unwrap(), Some(&public)).unwrap();
        let vs: Vec<&str> = idx.plugins[0].versions.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(vs, ["1.1.0", "1.0.0"]);
        assert_eq!(idx.plugins[0].best().unwrap().version, "1.1.0");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
