//! Program guide (EPG) for public TV: XMLTV files per country from
//! epgshare01.online, matched to iptv-org channels by name.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BASE: &str = "https://epgshare01.online/epgshare01/";
/// Guides are refreshed twice a day.
const MAX_AGE: Duration = Duration::from_secs(12 * 3600);
/// Programmes kept around "now" (seconds).
const KEEP_BEFORE: i64 = 3 * 3600;
const KEEP_AFTER: i64 = 36 * 3600;

#[derive(Clone, Debug)]
pub struct Prog {
    /// Unix seconds.
    pub start: i64,
    pub stop: i64,
    pub title: String,
    pub desc: String,
}

/// Programmes of one country's guide, looked up by normalized channel name.
#[derive(Default)]
pub struct Guide {
    by_key: HashMap<String, usize>,
    progs: Vec<Vec<Prog>>,
}

impl Guide {
    pub fn lookup(&self, keys: &[String]) -> Option<&[Prog]> {
        keys.iter().find_map(|k| self.by_key.get(k)).map(|&i| self.progs[i].as_slice())
    }
}

/// Lowercase letters and digits only, a trailing "hd" dropped:
/// "Das Erste HD" and "Das.Erste.de" both become "daserste".
pub fn norm(s: &str) -> String {
    let mut n: String = s.chars().flat_map(|c| c.to_lowercase()).filter(|c| c.is_alphanumeric()).collect();
    if n.len() > 3 && n.ends_with("hd") {
        n.truncate(n.len() - 2);
    }
    n
}

/// Keys an iptv-org channel can be found under: its name and its id
/// without the country part ("DasErste.de" -> "daserste").
pub fn keys(name: &str, id: &str) -> Vec<String> {
    let mut v = vec![];
    for k in [norm(name), norm(id.split('.').next().unwrap_or(""))] {
        if !k.is_empty() && !v.contains(&k) {
            v.push(k);
        }
    }
    v
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// "20:15" in local time.
pub fn clock(t: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(t, 0) {
        chrono::LocalResult::Single(d) | chrono::LocalResult::Ambiguous(d, _) => d.format("%H:%M").to_string(),
        chrono::LocalResult::None => String::new(),
    }
}

/// Local calendar day of a time, for grouping ("Heute" / "Morgen").
pub fn day_of(t: i64) -> i64 {
    use chrono::{Offset, TimeZone};
    let off = match chrono::Local.timestamp_opt(t, 0) {
        chrono::LocalResult::Single(d) | chrono::LocalResult::Ambiguous(d, _) => d.offset().fix().local_minus_utc() as i64,
        chrono::LocalResult::None => 0,
    };
    (t + off).div_euclid(86_400)
}

enum Slot {
    Loading,
    Ready(Arc<Guide>),
    /// No guide for this country (or the download failed).
    Missing,
}

pub struct Epg {
    slots: HashMap<String, Slot>,
    tx: Sender<(String, Option<Guide>)>,
    rx: Receiver<(String, Option<Guide>)>,
    ctx: eframe::egui::Context,
}

impl Epg {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        let (tx, rx) = channel();
        Self { slots: HashMap::new(), tx, rx, ctx }
    }

    fn poll(&mut self) {
        while let Ok((cc, g)) = self.rx.try_recv() {
            self.slots.insert(cc, g.map(|g| Slot::Ready(Arc::new(g))).unwrap_or(Slot::Missing));
        }
    }

    /// Starts loading the guide of a country (iptv-org code, e.g. "DE").
    pub fn want(&mut self, country: &str) {
        self.poll();
        if self.slots.contains_key(country) {
            return;
        }
        self.slots.insert(country.to_string(), Slot::Loading);
        let (cc, tx, ctx) = (country.to_string(), self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let g = load(&cc).map_err(|e| log::warn!("epg {cc}: {e}")).ok();
            let _ = tx.send((cc, g));
            ctx.request_repaint();
        });
    }

    pub fn loading(&self) -> bool {
        self.slots.values().any(|s| matches!(s, Slot::Loading))
    }

    pub fn guide(&mut self, country: &str) -> Option<Arc<Guide>> {
        self.poll();
        match self.slots.get(country) {
            Some(Slot::Ready(g)) => Some(g.clone()),
            _ => None,
        }
    }

    /// Forgets loaded guides so they are fetched again.
    pub fn reload(&mut self) {
        self.poll();
        self.slots.retain(|_, s| matches!(s, Slot::Loading));
        let _ = std::fs::remove_dir_all(cache_dir());
    }
}

/// The running programme and the next one.
pub fn now_next(progs: &[Prog], t: i64) -> Option<(&Prog, Option<&Prog>)> {
    let i = progs.iter().position(|p| p.start <= t && t < p.stop)?;
    Some((&progs[i], progs.get(i + 1)))
}

// ------------------------------------------------------------- loading

fn cache_dir() -> PathBuf {
    crate::config::cache_dir().join("epg")
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .user_agent(&format!("Velouna/{} (+https://github.com/BotAwesome/velouna)", env!("CARGO_PKG_VERSION")))
        .build()
}

fn fresh(p: &PathBuf) -> bool {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok()).is_some_and(|age| age < MAX_AGE)
}

/// Downloads `name` into the cache (unless fresh) and returns its bytes;
/// an old copy is used when the network fails.
fn fetch(name: &str) -> Result<Vec<u8>, String> {
    let p = cache_dir().join(name);
    if fresh(&p) {
        if let Ok(b) = std::fs::read(&p) {
            return Ok(b);
        }
    }
    // index.html stands for the server's file list.
    let url = if name == "index.html" { BASE.to_string() } else { format!("{BASE}{name}") };
    let net = agent().get(&url).call().map_err(|e| e.to_string()).and_then(|r| {
        let mut b = Vec::new();
        r.into_reader().take(200 << 20).read_to_end(&mut b).map_err(|e| e.to_string())?;
        Ok(b)
    });
    match net {
        Ok(b) => {
            let _ = std::fs::create_dir_all(cache_dir());
            let _ = std::fs::write(&p, &b);
            Ok(b)
        }
        Err(e) => std::fs::read(&p).map_err(|_| e),
    }
}

/// Guide files of a country, from the server's file list
/// ("epg_ripper_DE1.xml.gz", …). iptv-org's "UK" is also "UK" there.
fn files_for(country: &str) -> Result<Vec<String>, String> {
    let index = String::from_utf8_lossy(&fetch("index.html")?).to_string();
    let mut v: Vec<String> = vec![];
    let pre = format!("epg_ripper_{country}");
    let mut rest = index.as_str();
    while let Some(i) = rest.find(&pre) {
        let tail = &rest[i + pre.len()..];
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && tail[digits.len()..].starts_with(".xml.gz") {
            let name = format!("{pre}{digits}.xml.gz");
            if !v.contains(&name) {
                v.push(name);
            }
        }
        rest = tail;
    }
    Ok(v)
}

fn load(country: &str) -> Result<Guide, String> {
    let files = files_for(country)?;
    if files.is_empty() {
        return Err("no guide".into());
    }
    let mut g = Guide::default();
    let t = now();
    for f in files {
        let gz = fetch(&f)?;
        let mut xml = String::new();
        flate2::read::GzDecoder::new(gz.as_slice()).read_to_string(&mut xml).map_err(|e| format!("{f}: {e}"))?;
        parse_into(&xml, t, &mut g);
    }
    for p in &mut g.progs {
        p.sort_by_key(|x| x.start);
        p.dedup_by_key(|x| x.start);
    }
    Ok(g)
}

// ------------------------------------------------------------- XMLTV

/// Value of `name="…"` inside a start tag.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let i = tag.find(&pat)? + pat.len();
    let j = tag[i..].find('"')?;
    Some(&tag[i..i + j])
}

/// Text of the first `<name …>…</name>` element in `s`.
fn element<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let i = s.find(&open)?;
    let gt = i + s[i..].find('>')?;
    if s[..gt].ends_with('/') {
        return None;
    }
    let close = format!("</{name}>");
    let j = s[gt + 1..].find(&close)?;
    Some(&s[gt + 1..gt + 1 + j])
}

fn unescape(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("<![CDATA[").and_then(|x| x.strip_suffix("]]>")).unwrap_or(s);
    if !s.contains('&') {
        return s.replace("\\n", "\n");
    }
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'").replace("&amp;", "&").replace("\\n", "\n")
}

/// "20261112040000 +0000" -> unix seconds.
fn xmltv_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let d = s.get(..14)?;
    let n = |a: usize, b: usize| d.get(a..b)?.parse::<i64>().ok();
    let (y, mo, da, h, mi, se) = (n(0, 4)?, n(4, 6)?, n(6, 8)?, n(8, 10)?, n(10, 12)?, n(12, 14)?);
    let mut t = days_from_civil(y, mo, da) * 86_400 + h * 3600 + mi * 60 + se;
    let tz = s[14..].trim();
    if tz.len() == 5 {
        let sign = if tz.starts_with('-') { -1 } else { 1 };
        let off = tz[1..3].parse::<i64>().ok()? * 3600 + tz[3..5].parse::<i64>().ok()? * 60;
        t -= sign * off;
    }
    Some(t)
}

/// Days since 1970-01-01 (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_into(xml: &str, t: i64, g: &mut Guide) {
    // Channel ids of this file -> slot in g.progs.
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<channel ") {
        let block_end = rest[i..].find("</channel>").map(|j| i + j).unwrap_or(rest.len());
        let block = &rest[i..block_end];
        if let Some(id) = block.find('>').and_then(|gt| attr(&block[..gt], "id")) {
            let id = unescape(id);
            let slot = *ids.entry(id.clone()).or_insert_with(|| {
                g.progs.push(vec![]);
                g.progs.len() - 1
            });
            // The id without its country suffix, then every display name.
            let mut ks = vec![norm(id.rsplit_once('.').map(|x| x.0).unwrap_or(&id))];
            let mut b = block;
            while let Some(name) = element(b, "display-name") {
                ks.push(norm(&unescape(name)));
                let k = b.find("</display-name>").map(|k| k + 15).unwrap_or(b.len());
                b = &b[k..];
            }
            for k in ks.into_iter().filter(|k| !k.is_empty()) {
                g.by_key.entry(k).or_insert(slot);
            }
        }
        rest = &rest[block_end..];
    }
    let mut rest = xml;
    while let Some(i) = rest.find("<programme ") {
        let end = rest[i..].find("</programme>").map(|j| i + j + 12).unwrap_or(rest.len());
        let block = &rest[i..end];
        rest = &rest[end..];
        let Some(gt) = block.find('>') else { continue };
        let tag = &block[..gt];
        let (Some(start), Some(stop), Some(ch)) = (attr(tag, "start").and_then(xmltv_time), attr(tag, "stop").and_then(xmltv_time), attr(tag, "channel")) else {
            continue;
        };
        if stop < t - KEEP_BEFORE || start > t + KEEP_AFTER {
            continue;
        }
        let Some(&slot) = ids.get(&unescape(ch)) else { continue };
        let title = element(block, "title").map(unescape).unwrap_or_default();
        let desc = element(block, "desc").map(unescape).unwrap_or_default();
        g.progs[slot].push(Prog { start, stop, title, desc });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(xmltv_time("19700101000000 +0000"), Some(0));
        assert_eq!(xmltv_time("20261112040000 +0100"), xmltv_time("20261112030000 +0000"));
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }

    #[test]
    fn parses_xmltv() {
        let t = xmltv_time("20261112200000 +0000").unwrap();
        let xml = r#"<tv><channel id="Das.Erste.de"><display-name lang="de">Das Erste</display-name></channel>
<programme start="20261112194500 +0000" stop="20261112211500 +0000" channel="Das.Erste.de"><title lang="de">Tatort &amp; mehr</title><desc lang="de">Krimi</desc></programme>
<programme start="20261112211500 +0000" stop="20261112214500 +0000" channel="Das.Erste.de"><title>Tagesthemen</title></programme></tv>"#;
        let mut g = Guide::default();
        parse_into(xml, t, &mut g);
        let p = g.lookup(&keys("Das Erste HD", "DasErste.de")).unwrap();
        let (cur, next) = now_next(p, t).unwrap();
        assert_eq!(cur.title, "Tatort & mehr");
        assert_eq!(cur.desc, "Krimi");
        assert_eq!(next.unwrap().title, "Tagesthemen");
    }

    /// Real download; `cargo test epg -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_de() {
        let start = std::time::Instant::now();
        let g = load("DE").expect("load");
        println!("loaded in {:?}, {} channels", start.elapsed(), g.progs.len());
        for (name, id) in [("3sat", "3sat.de"), ("Das Erste", "DasErste.de"), ("ZDF", "ZDF.de"), ("KiKA", "KiKA.de")] {
            let p = g.lookup(&keys(name, id)).expect(name);
            let (cur, next) = now_next(p, now()).expect("now");
            println!("{name}: {} {} -> {} {}", clock(cur.start), cur.title, next.map(|n| clock(n.start)).unwrap_or_default(), next.map(|n| n.title.as_str()).unwrap_or(""));
        }
        assert!(files_for("CA").unwrap().iter().any(|f| f.contains("CA2")));
        assert!(files_for("XX").unwrap().is_empty());
    }
}
