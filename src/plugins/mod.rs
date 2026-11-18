//! Extensions (plugin docs: github.com/BotAwesome/velouna-plugins, docs/): the manifest, the installed list,
//! installing a `.velounaplugin` file and switching plugins on and off.
//! What a plugin may do is decided by its kind and its granted permissions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub mod repo;

/// Plugin interface version this Velouna speaks (the previous one is still loaded).
pub const API: u32 = 1;
/// File extension of a plugin package (a zip).
pub const EXT: &str = "velounaplugin";
const MAX_FILES: usize = 512;
const MAX_BYTES: u64 = 64 << 20;

/// A text in several languages ("de", "en"), or one for all.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Text {
    One(String),
    Lang(BTreeMap<String, String>),
}

impl Default for Text {
    fn default() -> Self {
        Text::One(String::new())
    }
}

impl Text {
    pub fn get(&self) -> String {
        match self {
            Text::One(s) => s.clone(),
            Text::Lang(m) => m
                .get(crate::i18n::code())
                .or_else(|| m.get("en"))
                .or_else(|| m.values().next())
                .cloned()
                .unwrap_or_default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Visualizer,
    Lua,
    Theme,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Visualizer => t!("Visualizer"),
            Kind::Lua => t!("Erweiterung"),
            Kind::Theme => t!("Design"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: Text,
    #[serde(default)]
    pub description: Text,
    pub version: String,
    pub kind: Kind,
    pub api: u32,
    /// Minimum Velouna version, e.g. ">=0.3.0".
    #[serde(default)]
    pub velouna: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Kind-specific part (passes, params, palettes of a visualizer).
    #[serde(default)]
    pub visualizer: Option<serde_json::Value>,
}

impl Manifest {
    fn check(&self) -> Result<(), String> {
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 100
            && self.id.contains('.')
            && self.id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            && !self.id.starts_with('.')
            && !self.id.contains("..");
        if !id_ok {
            return Err(tf!("Ungültige Kennung „{}“", self.id));
        }
        if self.api > API || self.api + 1 < API {
            return Err(tf!("Braucht Schnittstelle {}, Velouna kann {}", self.api, API));
        }
        if let Some(min) = self.velouna.strip_prefix(">=") {
            if version_lt(env!("CARGO_PKG_VERSION"), min.trim()) {
                return Err(tf!("Braucht Velouna {} oder neuer", min.trim()));
            }
        }
        if self.kind == Kind::Visualizer && !self.permissions.is_empty() {
            return Err(t!("Visualizer dürfen keine Berechtigungen verlangen").into());
        }
        Ok(())
    }
}

/// `a < b` for dotted version numbers.
pub fn version_lt(a: &str, b: &str) -> bool {
    let p = |s: &str| s.split(['.', '-']).map(|x| x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    p(a) < p(b)
}

/// What Velouna keeps per plugin in config.json.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct State {
    pub enabled: bool,
    /// Permissions agreed to, and for which version.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub granted: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub granted_version: String,
    /// The plugin's own settings (declared in its manifest).
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub settings: serde_json::Map<String, serde_json::Value>,
    /// repo.json of the repo it came from; updates only come from there.
    /// Empty when it was installed from a file.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub repo: String,
}

pub struct Installed {
    pub manifest: Manifest,
    pub dir: PathBuf,
    /// Set when the plugin was stopped (load or script error).
    pub error: Option<String>,
}

/// A folder under plugins/ that is no plugin (broken manifest).
pub struct Broken {
    pub dir: PathBuf,
    pub error: String,
}

#[derive(Default)]
pub struct Plugins {
    pub list: Vec<Installed>,
    pub broken: Vec<Broken>,
    scanned: bool,
}

/// A plugin package file, by its name.
pub fn is_package(path: &str) -> bool {
    Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case(EXT))
}

pub fn root() -> PathBuf {
    crate::config::data_dir().join("plugins")
}

pub fn data_root() -> PathBuf {
    crate::config::data_dir().join("plugin-data")
}

impl Plugins {
    /// Reads the installed plugins once (cheap: one small JSON file each).
    pub fn ensure(&mut self) {
        if !self.scanned {
            self.rescan();
        }
    }

    pub fn rescan(&mut self) {
        self.scanned = true;
        self.list.clear();
        self.broken.clear();
        let Ok(rd) = std::fs::read_dir(root()) else { return };
        for e in rd.flatten() {
            let dir = e.path();
            if !dir.is_dir() || dir.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                continue;
            }
            match read_manifest(&dir) {
                Ok(m) => self.list.push(Installed { manifest: m, dir, error: None }),
                Err(error) => self.broken.push(Broken { dir, error }),
            }
        }
        self.list.sort_by_key(|p| p.manifest.name.get().to_lowercase());
    }

    pub fn get(&self, id: &str) -> Option<&Installed> {
        self.list.iter().find(|p| p.manifest.id == id)
    }

    /// Enabled plugins of one kind.
    pub fn enabled<'a>(&'a self, kind: Kind, states: &'a BTreeMap<String, State>) -> impl Iterator<Item = &'a Installed> + 'a {
        self.list
            .iter()
            .filter(move |p| p.manifest.kind == kind && p.error.is_none() && states.get(&p.manifest.id).is_some_and(|s| s.enabled))
    }
}

pub(crate) fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    let s = std::fs::read(dir.join("plugin.json")).map_err(|_| t!("plugin.json fehlt").to_string())?;
    let m: Manifest = serde_json::from_slice(&s).map_err(|e| tf!("plugin.json: {}", e))?;
    m.check()?;
    Ok(m)
}

/// A package file read and checked, waiting for the user's yes.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub file: PathBuf,
    pub manifest: Manifest,
    /// Version installed now, if this is an update.
    pub installed: Option<String>,
    /// Permissions the installed version did not have.
    pub new_permissions: Vec<String>,
    /// The repo it comes from (none: a file).
    pub origin: Option<repo::Repo>,
}

/// Opens a `.velounaplugin` file and reads its manifest (nothing is written).
pub fn inspect(file: &Path, plugins: &Plugins) -> Result<Package, String> {
    let mut zip = open_zip(file)?;
    let mut s = Vec::new();
    zip.by_name("plugin.json")
        .map_err(|_| t!("Die Datei enthält keine plugin.json").to_string())?
        .take(1 << 20)
        .read_to_end(&mut s)
        .map_err(|e| e.to_string())?;
    let manifest: Manifest = serde_json::from_slice(&s).map_err(|e| tf!("plugin.json: {}", e))?;
    manifest.check()?;
    let old = plugins.get(&manifest.id);
    let new_permissions = manifest
        .permissions
        .iter()
        .filter(|p| old.is_none_or(|o| !o.manifest.permissions.contains(p)))
        .cloned()
        .collect();
    Ok(Package { file: file.to_path_buf(), installed: old.map(|o| o.manifest.version.clone()), manifest, new_permissions, origin: None })
}

fn open_zip(file: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(f).map_err(|_| t!("Keine gültige Plugin-Datei").to_string())
}

/// Unpacks a checked package into plugins/<id>/ (replacing an older version
/// only once the new one is completely on disk).
pub fn install(pkg: &Package) -> Result<(), String> {
    install_into(pkg, &root())
}

fn install_into(pkg: &Package, root: &Path) -> Result<(), String> {
    let mut zip = open_zip(&pkg.file)?;
    if zip.len() > MAX_FILES {
        return Err(t!("Zu viele Dateien im Paket").into());
    }
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let id = &pkg.manifest.id;
    let tmp = root.join(format!(".{id}.new"));
    let _ = std::fs::remove_dir_all(&tmp);
    let res = (|| -> Result<(), String> {
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut e = zip.by_index(i).map_err(|e| e.to_string())?;
            // Only plain relative paths: no "..", no absolute or drive paths.
            let Some(rel) = e.enclosed_name() else { return Err(tf!("Unzulässiger Pfad im Paket: {}", e.name())) };
            if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
                return Err(tf!("Unzulässiger Pfad im Paket: {}", e.name()));
            }
            let out = tmp.join(&rel);
            if e.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                continue;
            }
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let mut buf = Vec::new();
            (&mut e).take(MAX_BYTES - total + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
            total += buf.len() as u64;
            if total > MAX_BYTES {
                return Err(t!("Paket ist zu groß").into());
            }
            std::fs::write(&out, buf).map_err(|e| e.to_string())?;
        }
        let m = read_manifest(&tmp)?;
        if m != pkg.manifest {
            return Err(t!("Paket hat sich beim Installieren verändert").into());
        }
        Ok(())
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let dest = root.join(id);
    let old = root.join(format!(".{id}.old"));
    let _ = std::fs::remove_dir_all(&old);
    if dest.exists() {
        std::fs::rename(&dest, &old).map_err(|e| e.to_string())?;
    }
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::rename(&old, &dest);
        return Err(e.to_string());
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Removes the program part; `keep_data` leaves plugin-data/<id>/ in place.
pub fn uninstall(p: &Installed, keep_data: bool) -> Result<(), String> {
    std::fs::remove_dir_all(&p.dir).map_err(|e| e.to_string())?;
    if !keep_data {
        let _ = std::fs::remove_dir_all(data_root().join(&p.manifest.id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_runs() {
        let lua = mlua::Lua::new();
        let v: i64 = lua.load("local s = 0 for i = 1, 10 do s = s + i end return s").eval().unwrap();
        assert_eq!(v, 55);
    }

    /// Installs a zip into a scratch root: new install, update, bad path.
    #[test]
    fn install_roundtrip() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("velouna-plugin-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let make = |name: &str, version: &str, extra: &[(&str, &str)]| -> PathBuf {
            let f = dir.join(name);
            let mut z = zip::ZipWriter::new(std::fs::File::create(&f).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            let m = format!(r#"{{"id":"dev.test.viz","name":{{"de":"Test","en":"Test"}},"version":"{version}","kind":"visualizer","api":1}}"#);
            z.start_file("plugin.json", o).unwrap();
            z.write_all(m.as_bytes()).unwrap();
            for (n, c) in extra {
                z.start_file(*n, o).unwrap();
                z.write_all(c.as_bytes()).unwrap();
            }
            z.finish().unwrap();
            f
        };
        let root = dir.join("plugins");
        let mut ps = Plugins::default();
        let pkg = inspect(&make("a.velounaplugin", "1.0.0", &[("shaders/a.glsl", "void main(){}")]), &ps).unwrap();
        assert_eq!(pkg.installed, None);
        install_into(&pkg, &root).unwrap();
        assert!(root.join("dev.test.viz/shaders/a.glsl").is_file());
        ps.list.push(Installed { manifest: pkg.manifest.clone(), dir: root.join("dev.test.viz"), error: None });
        let up = inspect(&make("b.velounaplugin", "1.1.0", &[]), &ps).unwrap();
        assert_eq!(up.installed.as_deref(), Some("1.0.0"));
        install_into(&up, &root).unwrap();
        assert!(!root.join("dev.test.viz/shaders/a.glsl").exists(), "update replaces the old files");
        assert!(read_manifest(&root.join("dev.test.viz")).unwrap().version == "1.1.0");
        let bad = inspect(&make("c.velounaplugin", "1.2.0", &[("../evil.txt", "x")]), &ps).unwrap();
        assert!(install_into(&bad, &root).is_err());
        assert!(!dir.join("evil.txt").exists());
        assert!(read_manifest(&root.join("dev.test.viz")).unwrap().version == "1.1.0", "failed update keeps the old version");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_checks() {
        let m = |id: &str, perms: &[&str]| Manifest {
            id: id.into(),
            name: Text::One("x".into()),
            description: Text::default(),
            version: "1.0.0".into(),
            kind: Kind::Visualizer,
            api: API,
            velouna: String::new(),
            author: String::new(),
            license: String::new(),
            permissions: perms.iter().map(|s| s.to_string()).collect(),
            visualizer: None,
        };
        assert!(m("dev.example.ink", &[]).check().is_ok());
        assert!(m("ink", &[]).check().is_err());
        assert!(m("../evil.x", &[]).check().is_err());
        assert!(m("dev.x/y", &[]).check().is_err());
        assert!(m("dev.example.ink", &["network:*"]).check().is_err());
        assert!(version_lt("0.2.1", "0.10.0"));
        assert!(!version_lt("1.0.0", "1.0.0"));
    }
}
