//! Public radio: the community directory of radio-browser.info
//! (https://api.radio-browser.info). Languages, stations per language by
//! popularity, and a name search over all stations.

use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// API servers, tried in order.
const MIRRORS: &[&str] = &["https://de1.api.radio-browser.info", "https://de2.api.radio-browser.info"];
const MAX_AGE: Duration = Duration::from_secs(24 * 3600);
/// Most popular stations loaded per language.
const PER_LANGUAGE: usize = 1500;
/// Location code of the favorites list in `LibLoc::Radio`.
pub const FAVORITES: &str = "fav";
/// The home tile / network stream that opens the radio browser.
pub const HOME_URL: &str = "https://www.radio-browser.info/";

pub fn is_radio_browser(url: &str) -> bool {
    url.starts_with("https://www.radio-browser.info")
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Station {
    pub stationuuid: String,
    pub name: String,
    pub url: String,
    pub url_resolved: String,
    pub tags: String,
    pub country: String,
    pub countrycode: String,
    pub codec: String,
    pub bitrate: u32,
    /// Logo URL (may be empty).
    pub favicon: String,
}

impl Station {
    pub fn stream_url(&self) -> &str {
        if self.url_resolved.is_empty() { &self.url } else { &self.url_resolved }
    }

    pub fn title(&self) -> String {
        let n = self.name.trim();
        if n.is_empty() { self.stream_url().to_string() } else { n.to_string() }
    }

    /// "Deutschland · news, pop · MP3 128 kbps"
    pub fn subtitle(&self) -> String {
        let mut parts: Vec<String> = vec![];
        let de = if crate::i18n::is_en() { None } else { crate::iptv::country_de(&self.countrycode.to_uppercase()) };
        match de {
            Some(n) => parts.push(n.to_string()),
            None if !self.country.is_empty() => parts.push(self.country.clone()),
            None => {}
        }
        let tags: Vec<&str> = self.tags.split(',').map(str::trim).filter(|t| !t.is_empty()).take(3).collect();
        if !tags.is_empty() {
            parts.push(tags.join(", "));
        }
        match (self.codec.trim(), self.bitrate) {
            ("", 0) => {}
            (c, 0) => parts.push(c.to_string()),
            ("", b) => parts.push(format!("{b} kbps")),
            (c, b) => parts.push(format!("{c} {b} kbps")),
        }
        parts.join(" · ")
    }

    pub fn mpv_options(&self) -> String {
        let t = self.title();
        format!("force-media-title=%{}%{}", t.len(), t)
    }
}

pub fn favorite_options(f: &crate::config::RadioFavorite) -> String {
    format!("force-media-title=%{}%{}", f.name.len(), f.name)
}

#[derive(Deserialize)]
struct ApiLanguage {
    name: String,
    iso_639: Option<String>,
    stationcount: usize,
}

#[derive(Clone)]
pub struct Language {
    /// Exact API name ("german"), used to fetch its stations.
    pub name: String,
    pub label: String,
    pub count: usize,
    /// ISO 639 code (for the flag).
    pub iso: String,
}

pub type Stations = Arc<Vec<Arc<Station>>>;

#[derive(Clone)]
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

enum Msg {
    Languages(Result<Vec<Language>, String>),
    Stations(String, Result<Stations, String>),
    Search(String, Result<Stations, String>),
}

pub struct Radio {
    languages: Option<Load<Arc<Vec<Language>>>>,
    stations: HashMap<String, Load<Stations>>,
    /// Query of the last search started, and its result.
    search: Option<(String, Load<Stations>)>,
    /// Query being typed and since when (searches start after a pause).
    pending: Option<(String, Instant)>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: eframe::egui::Context,
}

impl Radio {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        let (tx, rx) = channel();
        Self { languages: None, stations: HashMap::new(), search: None, pending: None, tx, rx, ctx }
    }

    fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Languages(r) => self.languages = Some(r.map(|v| Load::Ready(Arc::new(v))).unwrap_or_else(Load::Failed)),
                Msg::Stations(l, r) => {
                    self.stations.insert(l, r.map(Load::Ready).unwrap_or_else(Load::Failed));
                }
                Msg::Search(q, r) => {
                    if self.search.as_ref().is_some_and(|s| s.0 == q) {
                        self.search = Some((q, r.map(Load::Ready).unwrap_or_else(Load::Failed)));
                    }
                }
            }
        }
    }

    fn spawn(&self, f: impl FnOnce() -> Msg + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
    }

    /// ISO 639 code of a loaded language (by API name).
    pub fn iso_of(&self, name: &str) -> Option<&str> {
        match &self.languages {
            Some(Load::Ready(v)) => v.iter().find(|l| l.name == name).map(|l| l.iso.as_str()),
            _ => None,
        }
    }

    pub fn languages(&mut self) -> Load<Arc<Vec<Language>>> {
        self.poll();
        if self.languages.is_none() {
            self.languages = Some(Load::Loading);
            self.spawn(|| Msg::Languages(load_languages(false)));
        }
        self.languages.clone().unwrap_or(Load::Loading)
    }

    pub fn stations(&mut self, lang: &str) -> Load<Stations> {
        self.poll();
        if !self.stations.contains_key(lang) {
            self.stations.insert(lang.to_string(), Load::Loading);
            let l = lang.to_string();
            self.spawn(move || {
                let r = load_stations(&l, false);
                Msg::Stations(l, r)
            });
        }
        self.stations.get(lang).cloned().unwrap_or(Load::Loading)
    }

    /// Stations whose name contains `q`, over all languages. Starts the
    /// request once typing has paused for a moment.
    pub fn search(&mut self, q: &str) -> Load<Stations> {
        self.poll();
        if let Some((cur, r)) = &self.search {
            if cur == q {
                return r.clone();
            }
        }
        let since = match &self.pending {
            Some((p, t)) if p == q => *t,
            _ => {
                self.pending = Some((q.to_string(), Instant::now()));
                Instant::now()
            }
        };
        let wait = Duration::from_millis(450);
        if since.elapsed() < wait {
            self.ctx.request_repaint_after(wait - since.elapsed());
            return Load::Loading;
        }
        self.search = Some((q.to_string(), Load::Loading));
        let q = q.to_string();
        self.spawn(move || {
            let r = search(&q);
            Msg::Search(q, r)
        });
        Load::Loading
    }

    /// Loads everything again from the network.
    pub fn reload(&mut self) {
        self.poll();
        self.languages = Some(Load::Loading);
        self.stations.clear();
        self.search = None;
        self.spawn(|| Msg::Languages(load_languages(true)));
    }

    /// Tells the directory a station was played (its popularity count).
    pub fn clicked(&self, uuid: &str) {
        if uuid.is_empty() {
            return;
        }
        let path = format!("/json/url/{uuid}");
        std::thread::spawn(move || {
            let _ = get(&path);
        });
    }
}

// ------------------------------------------------------------- network

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(40))
        .user_agent(&format!("Velour/{} (+https://github.com/BotAwesome/velour)", env!("CARGO_PKG_VERSION")))
        .build()
}

/// GET `path` from the first mirror that answers.
fn get(path: &str) -> Result<String, String> {
    let mut last = String::from("no server");
    for m in MIRRORS {
        match agent().get(&format!("{m}{path}")).call() {
            Ok(r) => {
                let mut s = String::new();
                r.into_reader().take(64 << 20).read_to_string(&mut s).map_err(|e| e.to_string())?;
                return Ok(s);
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

fn cache_path(key: &str) -> PathBuf {
    let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    crate::config::cache_dir().join("radio").join(format!("{safe}.json"))
}

/// `get` with a day-long cache; an old copy is used when offline.
fn cached(key: &str, path: &str, force: bool) -> Result<String, String> {
    let p = cache_path(key);
    let fresh = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok()).is_some_and(|a| a < MAX_AGE);
    if fresh && !force {
        if let Ok(s) = std::fs::read_to_string(&p) {
            return Ok(s);
        }
    }
    match get(path) {
        Ok(s) => {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(&p, &s);
            Ok(s)
        }
        Err(e) => std::fs::read_to_string(&p).map_err(|_| e),
    }
}

fn stations_from(s: &str) -> Result<Stations, String> {
    let v: Vec<Station> = serde_json::from_str(s).map_err(|e| e.to_string())?;
    Ok(Arc::new(v.into_iter().filter(|s| !s.stream_url().is_empty()).map(Arc::new).collect()))
}

fn load_languages(force: bool) -> Result<Vec<Language>, String> {
    let s = cached("languages", "/json/languages?hidebroken=true&order=stationcount&reverse=true", force)?;
    let v: Vec<ApiLanguage> = serde_json::from_str(&s).map_err(|e| e.to_string())?;
    // Only the entries with a language code: the free-text variants
    // ("american english", "español mexico" …) would clutter the list.
    let mut out: Vec<Language> = v
        .into_iter()
        .filter(|l| l.iso_639.as_deref().is_some_and(|c| !c.is_empty()) && l.stationcount > 0)
        .map(|l| {
            let iso = l.iso_639.unwrap_or_default();
            Language { label: label(&l.name, &iso), name: l.name, count: l.stationcount, iso }
        })
        .collect();
    let pinned: &[&str] = if crate::i18n::is_en() { &["english"] } else { &["german", "english"] };
    let rank = |n: &str| pinned.iter().position(|p| *p == n).unwrap_or(pinned.len());
    out.sort_by(|a, b| rank(&a.name).cmp(&rank(&b.name)).then(b.count.cmp(&a.count)).then_with(|| a.label.cmp(&b.label)));
    if force {
        for l in &out {
            let _ = std::fs::remove_file(cache_path(&format!("lang_{}", l.name)));
        }
    }
    Ok(out)
}

fn load_stations(lang: &str, force: bool) -> Result<Stations, String> {
    let path = format!("/json/stations/bylanguageexact/{}?hidebroken=true&order=clickcount&reverse=true&limit={PER_LANGUAGE}", enc(lang));
    stations_from(&cached(&format!("lang_{lang}"), &path, force)?)
}

fn search(q: &str) -> Result<Stations, String> {
    stations_from(&get(&format!("/json/stations/search?name={}&hidebroken=true&order=clickcount&reverse=true&limit=300", enc(q)))?)
}

/// Percent-encoding for a URL path segment / query value.
fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// German name of a language (ISO 639-1), else the API name capitalized.
fn label(name: &str, iso: &str) -> String {
    if !crate::i18n::is_en() {
        let de = match iso {
            "de" => "Deutsch",
            "en" => "Englisch",
            "es" => "Spanisch",
            "fr" => "Französisch",
            "zh" => "Chinesisch",
            "ru" => "Russisch",
            "it" => "Italienisch",
            "el" => "Griechisch",
            "pt" => "Portugiesisch",
            "nl" => "Niederländisch",
            "pl" => "Polnisch",
            "ar" => "Arabisch",
            "hi" => "Hindi",
            "ro" => "Rumänisch",
            "sr" => "Serbisch",
            "tr" => "Türkisch",
            "hu" => "Ungarisch",
            "ta" => "Tamil",
            "cs" => "Tschechisch",
            "hr" => "Kroatisch",
            "uk" => "Ukrainisch",
            "id" => "Indonesisch",
            "bg" => "Bulgarisch",
            "ne" => "Nepalesisch",
            "sv" => "Schwedisch",
            "ja" => "Japanisch",
            "da" => "Dänisch",
            "ca" => "Katalanisch",
            "bs" => "Bosnisch",
            "ko" => "Koreanisch",
            "sk" => "Slowakisch",
            "lv" => "Lettisch",
            "sq" => "Albanisch",
            "fi" => "Finnisch",
            "no" | "nb" => "Norwegisch",
            "lt" => "Litauisch",
            "et" => "Estnisch",
            "sl" => "Slowenisch",
            "mk" => "Mazedonisch",
            "he" => "Hebräisch",
            "fa" => "Persisch",
            "ur" => "Urdu",
            "bn" => "Bengalisch",
            "th" => "Thailändisch",
            "vi" => "Vietnamesisch",
            "ms" => "Malaiisch",
            "tl" => "Tagalog",
            "sw" => "Suaheli",
            "af" => "Afrikaans",
            "ga" => "Irisch",
            "cy" => "Walisisch",
            "eu" => "Baskisch",
            "gl" => "Galicisch",
            "is" => "Isländisch",
            "lb" => "Luxemburgisch",
            "mt" => "Maltesisch",
            "ka" => "Georgisch",
            "hy" => "Armenisch",
            "az" => "Aserbaidschanisch",
            "kk" => "Kasachisch",
            "be" => "Belarussisch",
            "la" => "Latein",
            "eo" => "Esperanto",
            _ => "",
        };
        if !de.is_empty() {
            return de.to_string();
        }
    }
    let mut c = name.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes() {
        assert_eq!(enc("german"), "german");
        assert_eq!(enc("español mexico"), "espa%C3%B1ol%20mexico");
    }

    /// Real requests; `cargo test radio -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live() {
        let langs = load_languages(false).unwrap();
        println!("{} languages: {:?}", langs.len(), langs.iter().take(6).map(|l| (&l.label, l.count)).collect::<Vec<_>>());
        let de = load_stations("german", false).unwrap();
        for s in de.iter().take(5) {
            println!("{} | {} | {}", s.title(), s.subtitle(), s.stream_url());
        }
        assert!(de.len() > 500);
        let r = search("swr3").unwrap();
        assert!(r.iter().any(|s| s.name.to_lowercase().contains("swr3")));
    }
}
