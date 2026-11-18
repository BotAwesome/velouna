//! Public TV channels from the iptv-org API (github.com/iptv-org/api,
//! Unlicense): every stream with its channel's languages, country and
//! categories. The JSON files are cached for a day.

use crate::config::TvFavorite;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

const API: &str = "https://iptv-org.github.io/api";
const MAX_AGE: Duration = Duration::from_secs(24 * 3600);
/// Language key for streams without a known language.
pub const UNKNOWN: &str = "und";

#[derive(Deserialize)]
struct ApiChannel {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    is_nsfw: bool,
    #[serde(default)]
    closed: Option<String>,
}

#[derive(Deserialize)]
struct ApiFeed {
    channel: String,
    id: String,
    #[serde(default)]
    is_main: bool,
    #[serde(default)]
    languages: Vec<String>,
}

#[derive(Deserialize)]
struct ApiStream {
    channel: Option<String>,
    feed: Option<String>,
    #[serde(default)]
    title: String,
    url: String,
    referrer: Option<String>,
    user_agent: Option<String>,
    quality: Option<String>,
    #[serde(default)]
    labels: Vec<String>,
}

#[derive(Deserialize)]
struct ApiLogo {
    channel: String,
    feed: Option<String>,
    #[serde(default = "yes")]
    in_use: bool,
    format: Option<String>,
    url: String,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct ApiNamed {
    #[serde(alias = "id")]
    code: String,
    name: String,
    #[serde(default)]
    flag: Option<String>,
}

pub struct TvChannel {
    pub title: String,
    pub url: String,
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
    pub quality: Option<String>,
    pub labels: Vec<String>,
    pub country: Option<String>,
    pub categories: Vec<String>,
    /// Names to find it in a program guide (see `epg::keys`).
    pub epg_keys: Vec<String>,
    /// Logo URL (raster formats only).
    pub logo: Option<String>,
}

pub struct Catalog {
    pub channels: Vec<TvChannel>,
    /// Language code → channel indices, sorted by title.
    pub by_lang: HashMap<String, Vec<usize>>,
    lang_names: HashMap<String, String>,
    countries: HashMap<String, (String, String)>,
    categories: HashMap<String, String>,
}

impl Catalog {
    /// Languages with their channel counts: the UI language (and English)
    /// first, then by channel count (ties by name), "unknown" last.
    pub fn languages(&self) -> Vec<(String, String, usize)> {
        let pinned: &[&str] = if crate::i18n::is_en() { &["eng"] } else { &["deu", "eng"] };
        let mut v: Vec<(String, String, usize)> = self.by_lang.iter().map(|(c, ids)| (c.clone(), self.lang_label(c), ids.len())).collect();
        let rank = |c: &str| pinned.iter().position(|p| *p == c).unwrap_or(if c == UNKNOWN { usize::MAX } else { pinned.len() });
        v.sort_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then(b.2.cmp(&a.2)).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
        v
    }

    pub fn lang_label(&self, code: &str) -> String {
        if code == UNKNOWN {
            return t!("Unbekannte Sprache").to_string();
        }
        if !crate::i18n::is_en() {
            if let Some(de) = lang_de(code) {
                return de.to_string();
            }
        }
        self.lang_names.get(code).cloned().unwrap_or_else(|| code.to_string())
    }

    /// "Deutschland · Nachrichten · 720p · Geo-blockiert" (no flag emoji:
    /// the UI fonts have no flag glyphs).
    pub fn subtitle(&self, c: &TvChannel) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(cc) = &c.country {
            let de = if crate::i18n::is_en() { None } else { country_de(cc) };
            match (de, self.countries.get(cc)) {
                (Some(n), _) => parts.push(n.to_string()),
                (None, Some((name, _))) => parts.push(name.clone()),
                _ => {}
            }
        }
        let cats: Vec<String> = c.categories.iter().take(2).map(|id| category_label(id, self.categories.get(id))).collect();
        if !cats.is_empty() {
            parts.push(cats.join(", "));
        }
        if let Some(q) = &c.quality {
            parts.push(q.clone());
        }
        for l in &c.labels {
            parts.push(match l.as_str() {
                "Geo-blocked" => t!("Geo-blockiert").to_string(),
                "Not 24/7" => t!("Nicht rund um die Uhr").to_string(),
                other => other.to_string(),
            });
        }
        parts.join(" · ")
    }

    /// What a favorite keeps of a channel.
    pub fn favorite(&self, c: &TvChannel) -> TvFavorite {
        TvFavorite {
            title: c.title.clone(),
            url: c.url.clone(),
            user_agent: c.user_agent.clone(),
            referrer: c.referrer.clone(),
            sub: self.subtitle(c),
            country: c.country.clone(),
            epg_keys: c.epg_keys.clone(),
            logo: c.logo.clone().unwrap_or_default(),
        }
    }
}

impl TvChannel {
    pub fn mpv_options(&self) -> String {
        mpv_options(&self.title, self.user_agent.as_deref(), self.referrer.as_deref())
    }
}

/// Favorites: location code of the list in `LibLoc::Tv`.
pub const FAVORITES: &str = "fav";

pub fn favorite_options(f: &TvFavorite) -> String {
    mpv_options(&f.title, f.user_agent.as_deref(), f.referrer.as_deref())
}

/// mpv per-file options: title, user agent and referrer from the list.
fn mpv_options(title: &str, user_agent: Option<&str>, referrer: Option<&str>) -> String {
    let q = |s: &str| format!("%{}%{}", s.len(), s);
    let mut o = format!("force-media-title={}", q(title));
    if let Some(ua) = user_agent.filter(|s| !s.is_empty()) {
        o.push_str(&format!(",user-agent={}", q(ua)));
    }
    if let Some(r) = referrer.filter(|s| !s.is_empty()) {
        o.push_str(&format!(",referrer={}", q(r)));
    }
    o
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .user_agent(&format!("Velouna/{} (+https://github.com/BotAwesome/velouna)", env!("CARGO_PKG_VERSION")))
        .build()
}

fn cache_path(name: &str) -> PathBuf {
    crate::config::cache_dir().join("iptv").join(format!("{name}.json"))
}

/// One API file: from the cache while it's younger than a day, else from
/// the network (falling back to an older cached copy when offline).
fn fetch(name: &str, force: bool) -> Result<String, String> {
    let path = cache_path(name);
    let fresh = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < MAX_AGE);
    if fresh && !force {
        if let Ok(s) = std::fs::read_to_string(&path) {
            return Ok(s);
        }
    }
    let net = agent().get(&format!("{API}/{name}.json")).call().map_err(|e| e.to_string()).and_then(|r| {
        let mut s = String::new();
        r.into_reader().read_to_string(&mut s).map_err(|e| e.to_string())?;
        Ok(s)
    });
    match net {
        Ok(s) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, &s);
            Ok(s)
        }
        Err(e) => std::fs::read_to_string(&path).map_err(|_| e),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(name: &str, force: bool) -> Result<Vec<T>, String> {
    let s = fetch(name, force)?;
    serde_json::from_str(&s).map_err(|e| format!("{name}.json: {e}"))
}

fn load(force: bool) -> Result<Catalog, String> {
    let channels: Vec<ApiChannel> = parse("channels", force)?;
    let feeds: Vec<ApiFeed> = parse("feeds", force)?;
    let streams: Vec<ApiStream> = parse("streams", force)?;
    let languages: Vec<ApiNamed> = parse("languages", force).unwrap_or_default();
    let countries: Vec<ApiNamed> = parse("countries", force).unwrap_or_default();
    let categories: Vec<ApiNamed> = parse("categories", force).unwrap_or_default();
    let logos: Vec<ApiLogo> = parse("logos", force).unwrap_or_default();
    // (channel, feed or "") -> logo; formats the image decoder can read,
    // current logos first.
    let mut logo_of: HashMap<(&str, &str), &str> = HashMap::new();
    let raster = |f: &Option<String>| matches!(f.as_deref(), Some("PNG" | "JPEG" | "WebP" | "GIF"));
    for pass in [true, false] {
        for l in logos.iter().filter(|l| l.in_use == pass && raster(&l.format)) {
            logo_of.entry((l.channel.as_str(), l.feed.as_deref().unwrap_or(""))).or_insert(l.url.as_str());
        }
    }

    let chans: HashMap<&str, &ApiChannel> = channels.iter().map(|c| (c.id.as_str(), c)).collect();
    let feed_langs: HashMap<(&str, &str), &Vec<String>> = feeds.iter().map(|f| ((f.channel.as_str(), f.id.as_str()), &f.languages)).collect();
    let main_langs: HashMap<&str, &Vec<String>> = feeds.iter().filter(|f| f.is_main).map(|f| (f.channel.as_str(), &f.languages)).collect();

    let mut out = Vec::with_capacity(streams.len());
    let mut by_lang: HashMap<String, Vec<usize>> = HashMap::new();
    for s in streams {
        let ch = s.channel.as_deref().and_then(|id| chans.get(id).copied());
        // Adult and closed channels stay out.
        if ch.is_some_and(|c| c.is_nsfw || c.closed.is_some()) || s.url.is_empty() {
            continue;
        }
        let langs: Vec<String> = match (s.channel.as_deref(), s.feed.as_deref()) {
            (Some(c), Some(f)) => feed_langs.get(&(c, f)).map(|v| (*v).clone()),
            (Some(c), None) => main_langs.get(c).map(|v| (*v).clone()),
            _ => None,
        }
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![UNKNOWN.to_string()]);
        let idx = out.len();
        for l in &langs {
            by_lang.entry(l.clone()).or_default().push(idx);
        }
        out.push(TvChannel {
            title: if s.title.is_empty() { s.url.clone() } else { s.title },
            url: s.url,
            user_agent: s.user_agent,
            referrer: s.referrer,
            quality: s.quality,
            labels: s.labels,
            country: ch.and_then(|c| c.country.clone()),
            categories: ch.map(|c| c.categories.clone()).unwrap_or_default(),
            epg_keys: ch.map(|c| crate::epg::keys(&c.name, &c.id)).unwrap_or_default(),
            logo: s.channel.as_deref().and_then(|c| {
                s.feed.as_deref().and_then(|f| logo_of.get(&(c, f))).or_else(|| logo_of.get(&(c, ""))).map(|u| u.to_string())
            }),
        });
    }
    for ids in by_lang.values_mut() {
        ids.sort_by(|a, b| crate::media::natural_cmp(&out[*a].title.to_lowercase(), &out[*b].title.to_lowercase()));
    }
    Ok(Catalog {
        channels: out,
        by_lang,
        lang_names: languages.into_iter().map(|l| (l.code, l.name)).collect(),
        countries: countries.into_iter().map(|c| (c.code, (c.name, c.flag.unwrap_or_default()))).collect(),
        categories: categories.into_iter().map(|c| (c.code, c.name)).collect(),
    })
}

pub enum TvState {
    Idle,
    Loading,
    Ready(Arc<Catalog>),
    Failed(String),
}

pub struct Tv {
    state: TvState,
    rx: Option<Receiver<Result<Catalog, String>>>,
    ctx: eframe::egui::Context,
}

impl Tv {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        Self { state: TvState::Idle, rx: None, ctx }
    }

    /// Current state; starts loading the first time it's asked for.
    pub fn state(&mut self) -> &TvState {
        if let Some(rx) = &self.rx {
            if let Ok(r) = rx.try_recv() {
                self.rx = None;
                self.state = match r {
                    Ok(c) => TvState::Ready(Arc::new(c)),
                    Err(e) => TvState::Failed(e),
                };
            }
        }
        if matches!(self.state, TvState::Idle) {
            self.start(false);
        }
        &self.state
    }

    /// The loaded list (no polling: `state()` runs every frame the TV page shows).
    pub fn catalog(&self) -> Option<Arc<Catalog>> {
        match &self.state {
            TvState::Ready(c) => Some(c.clone()),
            _ => None,
        }
    }

    /// Loads again from the network.
    pub fn reload(&mut self) {
        self.start(true);
    }

    fn start(&mut self, force: bool) {
        if self.rx.is_some() {
            return;
        }
        self.state = TvState::Loading;
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let r = load(force);
            if let Err(e) = &r {
                log::warn!("iptv: {e}");
            }
            let _ = tx.send(r);
            ctx.request_repaint();
        });
    }
}

/// German names of the languages with the most channels.
fn lang_de(code: &str) -> Option<&'static str> {
    Some(match code {
        "deu" => "Deutsch",
        "eng" => "Englisch",
        "spa" => "Spanisch",
        "rus" => "Russisch",
        "hin" => "Hindi",
        "por" => "Portugiesisch",
        "fra" => "Französisch",
        "ara" => "Arabisch",
        "ita" => "Italienisch",
        "zho" => "Chinesisch",
        "nld" => "Niederländisch",
        "fas" => "Persisch",
        "ron" => "Rumänisch",
        "ukr" => "Ukrainisch",
        "tur" => "Türkisch",
        "ind" => "Indonesisch",
        "hun" => "Ungarisch",
        "dan" => "Dänisch",
        "pol" => "Polnisch",
        "swe" => "Schwedisch",
        "tam" => "Tamil",
        "nor" | "nob" => "Norwegisch",
        "ben" => "Bengalisch",
        "vie" => "Vietnamesisch",
        "ces" => "Tschechisch",
        "ell" => "Griechisch",
        "kor" => "Koreanisch",
        "jpn" => "Japanisch",
        "tha" => "Thailändisch",
        "srp" => "Serbisch",
        "hrv" => "Kroatisch",
        "bos" => "Bosnisch",
        "bul" => "Bulgarisch",
        "slk" => "Slowakisch",
        "slv" => "Slowenisch",
        "fin" => "Finnisch",
        "heb" => "Hebräisch",
        "urd" => "Urdu",
        "msa" => "Malaiisch",
        "tgl" | "fil" => "Filipino",
        "cat" => "Katalanisch",
        "eus" => "Baskisch",
        "glg" => "Galicisch",
        "sqi" => "Albanisch",
        "mkd" => "Mazedonisch",
        "lit" => "Litauisch",
        "lav" => "Lettisch",
        "est" => "Estnisch",
        "kat" => "Georgisch",
        "hye" => "Armenisch",
        "aze" => "Aserbaidschanisch",
        "kaz" => "Kasachisch",
        "uzb" => "Usbekisch",
        "kur" => "Kurdisch",
        "pus" => "Paschtu",
        "swa" => "Suaheli",
        "afr" => "Afrikaans",
        "isl" => "Isländisch",
        "gle" => "Irisch",
        "cym" => "Walisisch",
        "ltz" => "Luxemburgisch",
        "mlt" => "Maltesisch",
        "bel" => "Weißrussisch",
        "mon" => "Mongolisch",
        "yue" => "Kantonesisch",
        "pan" => "Panjabi",
        "tel" => "Telugu",
        "mal" => "Malayalam",
        "kan" => "Kannada",
        "mar" => "Marathi",
        "guj" => "Gujarati",
        "nep" => "Nepali",
        "sin" => "Singhalesisch",
        "amh" => "Amharisch",
        "som" => "Somali",
        "hau" => "Hausa",
        "yor" => "Yoruba",
        "lat" => "Latein",
        _ => return None,
    })
}

fn category_label(id: &str, api_name: Option<&String>) -> String {
    if !crate::i18n::is_en() {
        let de = match id {
            "animation" => Some("Animation"),
            "auto" => Some("Auto"),
            "business" => Some("Wirtschaft"),
            "classic" => Some("Klassiker"),
            "comedy" => Some("Comedy"),
            "cooking" => Some("Kochen"),
            "culture" => Some("Kultur"),
            "documentary" => Some("Doku"),
            "education" => Some("Bildung"),
            "entertainment" => Some("Unterhaltung"),
            "family" => Some("Familie"),
            "general" => Some("Allgemein"),
            "kids" => Some("Kinder"),
            "legislative" => Some("Politik"),
            "lifestyle" => Some("Lifestyle"),
            "movies" => Some("Filme"),
            "music" => Some("Musik"),
            "news" => Some("Nachrichten"),
            "outdoor" => Some("Natur"),
            "public" => Some("Öffentlich"),
            "relax" => Some("Entspannung"),
            "religious" => Some("Religion"),
            "science" => Some("Wissenschaft"),
            "series" => Some("Serien"),
            "shop" => Some("Shopping"),
            "sports" => Some("Sport"),
            "travel" => Some("Reisen"),
            "weather" => Some("Wetter"),
            _ => None,
        };
        if let Some(de) = de {
            return de.to_string();
        }
    }
    api_name.cloned().unwrap_or_else(|| id.to_string())
}

/// German country names (iptv-org only has English ones).
pub fn country_de(code: &str) -> Option<&'static str> {
    Some(match code {
        "AD" => "Andorra",
        "AE" => "Vereinigte Arabische Emirate",
        "AF" => "Afghanistan",
        "AL" => "Albanien",
        "AM" => "Armenien",
        "AR" => "Argentinien",
        "AT" => "Österreich",
        "AU" => "Australien",
        "AW" => "Aruba",
        "AZ" => "Aserbaidschan",
        "BA" => "Bosnien und Herzegowina",
        "BB" => "Barbados",
        "BD" => "Bangladesch",
        "BE" => "Belgien",
        "BG" => "Bulgarien",
        "BH" => "Bahrain",
        "BJ" => "Benin",
        "BO" => "Bolivien",
        "BR" => "Brasilien",
        "BS" => "Bahamas",
        "BY" => "Belarus",
        "CA" => "Kanada",
        "CD" => "Demokratische Republik Kongo",
        "CF" => "Zentralafrikanische Republik",
        "CG" => "Republik Kongo",
        "CH" => "Schweiz",
        "CI" => "Elfenbeinküste",
        "CL" => "Chile",
        "CM" => "Kamerun",
        "CN" => "China",
        "CO" => "Kolumbien",
        "CR" => "Costa Rica",
        "CU" => "Kuba",
        "CV" => "Kap Verde",
        "CY" => "Zypern",
        "CZ" => "Tschechien",
        "DE" => "Deutschland",
        "DK" => "Dänemark",
        "DO" => "Dominikanische Republik",
        "DZ" => "Algerien",
        "EC" => "Ecuador",
        "EE" => "Estland",
        "EG" => "Ägypten",
        "ES" => "Spanien",
        "ET" => "Äthiopien",
        "FI" => "Finnland",
        "FJ" => "Fidschi",
        "FO" => "Färöer",
        "FR" => "Frankreich",
        "GA" => "Gabun",
        "GE" => "Georgien",
        "GH" => "Ghana",
        "GL" => "Grönland",
        "GQ" => "Äquatorialguinea",
        "GR" => "Griechenland",
        "GT" => "Guatemala",
        "HK" => "Hongkong",
        "HN" => "Honduras",
        "HR" => "Kroatien",
        "HT" => "Haiti",
        "HU" => "Ungarn",
        "ID" => "Indonesien",
        "IE" => "Irland",
        "IL" => "Israel",
        "IN" => "Indien",
        "IQ" => "Irak",
        "IR" => "Iran",
        "IS" => "Island",
        "IT" => "Italien",
        "JM" => "Jamaika",
        "JO" => "Jordanien",
        "JP" => "Japan",
        "KE" => "Kenia",
        "KG" => "Kirgisistan",
        "KH" => "Kambodscha",
        "KP" => "Nordkorea",
        "KR" => "Südkorea",
        "KW" => "Kuwait",
        "KZ" => "Kasachstan",
        "LA" => "Laos",
        "LB" => "Libanon",
        "LI" => "Liechtenstein",
        "LK" => "Sri Lanka",
        "LT" => "Litauen",
        "LU" => "Luxemburg",
        "LV" => "Lettland",
        "LY" => "Libyen",
        "MA" => "Marokko",
        "MC" => "Monaco",
        "MD" => "Moldau",
        "ME" => "Montenegro",
        "MG" => "Madagaskar",
        "MK" => "Nordmazedonien",
        "MM" => "Myanmar",
        "MN" => "Mongolei",
        "MO" => "Macau",
        "MR" => "Mauretanien",
        "MT" => "Malta",
        "MU" => "Mauritius",
        "MV" => "Malediven",
        "MX" => "Mexiko",
        "MY" => "Malaysia",
        "MZ" => "Mosambik",
        "NA" => "Namibia",
        "NG" => "Nigeria",
        "NI" => "Nicaragua",
        "NL" => "Niederlande",
        "NO" => "Norwegen",
        "NP" => "Nepal",
        "NZ" => "Neuseeland",
        "OM" => "Oman",
        "PA" => "Panama",
        "PE" => "Peru",
        "PG" => "Papua-Neuguinea",
        "PH" => "Philippinen",
        "PK" => "Pakistan",
        "PL" => "Polen",
        "PR" => "Puerto Rico",
        "PS" => "Palästina",
        "PT" => "Portugal",
        "PY" => "Paraguay",
        "QA" => "Katar",
        "RO" => "Rumänien",
        "RS" => "Serbien",
        "RU" => "Russland",
        "RW" => "Ruanda",
        "SA" => "Saudi-Arabien",
        "SC" => "Seychellen",
        "SD" => "Sudan",
        "SE" => "Schweden",
        "SG" => "Singapur",
        "SI" => "Slowenien",
        "SK" => "Slowakei",
        "SM" => "San Marino",
        "SN" => "Senegal",
        "SO" => "Somalia",
        "SS" => "Südsudan",
        "SV" => "El Salvador",
        "SX" => "Sint Maarten",
        "SY" => "Syrien",
        "TD" => "Tschad",
        "TG" => "Togo",
        "TH" => "Thailand",
        "TJ" => "Tadschikistan",
        "TM" => "Turkmenistan",
        "TN" => "Tunesien",
        "TR" => "Türkei",
        "TT" => "Trinidad und Tobago",
        "TW" => "Taiwan",
        "TZ" => "Tansania",
        "UA" => "Ukraine",
        "UG" => "Uganda",
        "UK" => "Großbritannien",
        "US" => "USA",
        "UY" => "Uruguay",
        "UZ" => "Usbekistan",
        "VA" => "Vatikanstadt",
        "VE" => "Venezuela",
        "VN" => "Vietnam",
        "XK" => "Kosovo",
        "YE" => "Jemen",
        "ZA" => "Südafrika",
        "ZM" => "Sambia",
        "ZW" => "Simbabwe",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    /// Fetches the real lists; run with `cargo test iptv -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_catalog() {
        let c = super::load(false).expect("load");
        let langs = c.languages();
        assert!(c.channels.len() > 10_000, "{}", c.channels.len());
        assert_eq!(langs[0].0, "deu");
        assert_eq!(langs.last().unwrap().0, super::UNKNOWN);
        for (code, label, n) in langs.iter().take(8) {
            println!("{code} {label} {n}");
        }
        let de = &c.by_lang["deu"];
        for &i in de.iter().take(5) {
            println!("{} | {}", c.channels[i].title, c.subtitle(&c.channels[i]));
        }
    }
}
