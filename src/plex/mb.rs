//! Jellyfin and Emby, which share the "MediaBrowser" API.
//!
//! Velouna's server pages ask for Plex-style paths (`/hubs`, `/library/metadata/1/children`
//! …). This module answers those paths from a Jellyfin or Emby server and maps
//! the results onto the Plex model, so all three kinds of server share one UI,
//! one cache and one player path. Endpoint choices and the differences between
//! the two dialects follow Plezy's MediaBrowser client (GPL-3.0).

use super::Conn;
use super::api::{Http, url_encode};
use super::model::{Backend, Connection, Hub, Item, Kind, MediaVersion, Part, Rating, Role, Section, Server, StreamInfo};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

/// Fields for library lists. Emby leaves out year, rating and dates unless named.
const LIST_FIELDS: &str = "Overview,Genres,DateCreated,PremiereDate,ProductionYear,OfficialRating,SortName,OriginalTitle,ChildCount,RecursiveItemCount";
/// Fields for home rows: no counts (each one costs the server a query per row).
const HUB_FIELDS: &str = "Overview,Genres,DateCreated,PremiereDate,ProductionYear,OfficialRating,SortName";
/// Lists that are played from (episodes, tracks) also carry the files.
const PLAY_FIELDS: &str = "Overview,DateCreated,PremiereDate,ProductionYear,OfficialRating,SortName,ChildCount,RecursiveItemCount,MediaSources,Path";
const DETAIL_FIELDS: &str = "Overview,Genres,People,Studios,Taglines,DateCreated,PremiereDate,ProductionYear,OfficialRating,SortName,OriginalTitle,ChildCount,RecursiveItemCount,MediaSources,Path,ProviderIds";
const IMAGES: &str = "EnableImageTypes=Primary,Backdrop,Thumb&ImageTypeLimit=1&EnableUserData=true";

// ------------------------------------------------------------ identity

/// `MediaBrowser` Authorization header. Values are percent-encoded; both
/// servers decode them, which keeps names like "Bjørn's PC" sendable.
pub fn auth_header(http: &Http, device_id: &str, token: Option<&str>) -> String {
    let f = |k: &str, v: &str| format!("{k}=\"{}\"", url_encode(v));
    let mut parts = vec![
        f("Client", &format!("Velouna {}", crate::platform::PLEX_PLATFORM)),
        f("Device", http.device_name()),
        f("DeviceId", device_id),
        f("Version", env!("CARGO_PKG_VERSION")),
    ];
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        parts.push(f("Token", t));
    }
    format!("MediaBrowser {}", parts.join(", "))
}

/// Query parameter that carries the token in URLs (subtitles, ffmpeg).
/// Jellyfin 10.11+ may refuse the legacy `api_key`, which Emby requires.
pub fn token_param(b: Backend) -> &'static str {
    match b {
        Backend::Emby => "api_key",
        _ => "ApiKey",
    }
}

/// HTTP header lines for mpv (a comma-separated list).
pub fn stream_headers(token: &str) -> String {
    format!("Authorization: MediaBrowser Token=\"{token}\",X-Emby-Token: {token}")
}

// --------------------------------------------------------------- login

/// What the user may type: `jellyfin.example.com`, `192.168.1.5:8096`,
/// `https://host/jellyfin/web/#/home` … Returns base URLs to try, best first.
pub fn candidates(input: &str) -> Vec<String> {
    let mut s = input.trim().trim_end_matches('/').to_string();
    for cut in ["/web/", "/web#", "/#"] {
        if let Some(i) = s.find(cut) {
            s.truncate(i);
        }
    }
    let s = s.trim_end_matches("/web").trim_end_matches('/').to_string();
    if s.is_empty() {
        return vec![];
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return vec![s];
    }
    let host_has_port = s.split('/').next().is_some_and(|h| h.rsplit_once(':').is_some_and(|(_, p)| p.chars().all(|c| c.is_ascii_digit())));
    if host_has_port {
        vec![format!("http://{s}"), format!("https://{s}")]
    } else {
        let (host, path) = match s.find('/') {
            Some(i) => (&s[..i], &s[i..]),
            None => (s.as_str(), ""),
        };
        vec![format!("https://{s}"), format!("http://{host}:8096{path}"), format!("https://{host}:8920{path}"), format!("http://{s}")]
    }
}

struct Probe {
    base: String,
    id: String,
    name: String,
    backend: Backend,
}

fn probe(http: &Http, base: &str) -> Result<Probe, String> {
    let v = http.mb("GET", &format!("{base}/System/Info/Public"), "", None, true)?;
    let id = s(&v, "Id");
    if id.is_empty() {
        return Err(t!("Das ist kein Jellyfin- oder Emby-Server").into());
    }
    let product = s(&v, "ProductName").to_lowercase();
    // Emby 4.9 sends no product name but is the only one with RemoteAddresses.
    let backend = if product.contains("emby") || (product.is_empty() && v.get("RemoteAddresses").is_some()) { Backend::Emby } else { Backend::Jellyfin };
    let name = [s(&v, "ServerName"), backend.name().to_string()].into_iter().find(|n| !n.is_empty()).unwrap_or_default();
    Ok(Probe { base: base.to_string(), id, name, backend })
}

/// Finds the server behind `input` and signs in. Returns the server and the user's name.
pub fn login(http: &Http, input: &str, user: &str, password: &str, device_id: &str) -> Result<(Server, String), String> {
    let mut last = t!("Server nicht gefunden").to_string();
    let mut found = None;
    for base in candidates(input) {
        match probe(http, &base) {
            Ok(p) => {
                found = Some(p);
                break;
            }
            Err(e) => last = e,
        }
    }
    let p = found.ok_or(last)?;
    let auth = auth_header(http, device_id, None);
    let v = http
        .mb("POST", &format!("{}/Users/AuthenticateByName", p.base), &auth, Some(&json!({ "Username": user, "Pw": password })), false)
        .map_err(|e| if e.contains("401") || e.contains("403") { t!("Benutzername oder Passwort ist falsch").to_string() } else { e })?;
    let token = s(&v, "AccessToken");
    let u = v.get("User").cloned().unwrap_or(Value::Null);
    let user_id = s(&u, "Id");
    if token.is_empty() || user_id.is_empty() {
        return Err(t!("Unvollständige Antwort des Servers").into());
    }
    let server = Server {
        id: format!("{}/{user_id}", p.id),
        name: p.name,
        token,
        owned: true,
        owner: String::new(),
        presence: true,
        connections: vec![Connection { uri: p.base, ..Default::default() }],
        backend: p.backend,
        user_id,
        device_id: device_id.to_string(),
    };
    Ok((server, s(&u, "Name")))
}

/// Ends the session on the server (token is revoked). Best effort.
pub fn logout(http: &Http, c: &Conn) {
    let _ = http.mb("POST", &format!("{}/Sessions/Logout", c.base), &auth_header(http, &c.device_id, Some(&c.token)), None, true);
}

// ------------------------------------------------------------- connect

/// Library type (`movies`, `tvshows` …) per view id, for building queries.
static VIEWS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Kind and series of every item seen, so `/children` knows what to ask for.
static KINDS: LazyLock<Mutex<HashMap<String, (Kind, String)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Items with an open playback session (`base|id`).
static PLAYING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Checks the server and loads its libraries.
pub fn connect(http: &Http, c: &Conn) -> Result<Vec<Section>, String> {
    let x = Ctx::new(http, c);
    let v = x.get(&format!("/Users/{}/Views", x.uid()))?;
    let mut out = Vec::new();
    let mut views = VIEWS.lock().unwrap();
    for it in items_of(&v) {
        let ct = s(it, "CollectionType");
        let kind = match ct.as_str() {
            "movies" | "homevideos" | "musicvideos" | "" | "mixed" => "movie",
            "tvshows" => "show",
            "music" => "artist",
            _ => continue, // collections, playlists, live TV, books, photos
        };
        let id = s(it, "Id");
        views.insert(id.clone(), ct.clone());
        out.push(Section { key: id, kind: kind.into(), title: s(it, "Name"), });
    }
    Ok(out)
}

// ------------------------------------------------------------- context

struct Ctx<'a> {
    http: &'a Http,
    c: &'a Conn,
    auth: String,
}

impl<'a> Ctx<'a> {
    fn new(http: &'a Http, c: &'a Conn) -> Ctx<'a> {
        Ctx { http, c, auth: auth_header(http, &c.device_id, Some(&c.token)) }
    }
    fn uid(&self) -> String {
        url_encode(&self.c.user_id)
    }
    fn emby(&self) -> bool {
        self.c.backend == Backend::Emby
    }
    fn get(&self, path: &str) -> Result<Value, String> {
        self.http.mb("GET", &format!("{}{path}", self.c.base), &self.auth, None, false)
    }
    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<(), String> {
        self.http.mb(method, &format!("{}{path}", self.c.base), &self.auth, body, false).map(|_| ())
    }
    /// `/Users/{uid}/Items?…` (the spelling both dialects know).
    fn items(&self, query: &str) -> Result<(Vec<Item>, i64), String> {
        let v = self.get(&format!("/Users/{}/Items?{query}&{IMAGES}", self.uid()))?;
        Ok(list(&v))
    }
    fn item(&self, id: &str, fields: &str) -> Result<Item, String> {
        let v = self.get(&format!("/Users/{}/Items/{}?Fields={fields}&{IMAGES}", self.uid(), url_encode(id)))?;
        Ok(map(&v))
    }
    /// Kind and series id of an item, asking the server if it was never seen.
    fn kind(&self, id: &str) -> Result<(Kind, String), String> {
        if let Some(k) = KINDS.lock().unwrap().get(id).cloned() {
            return Ok(k);
        }
        let it = self.item(id, "SortName")?;
        Ok((it.kind, it.grandparent_key.clone()))
    }
}

// ---------------------------------------------------------------- paths

/// Splits a Plex-style path into its route and decoded query.
fn split(path: &str) -> (String, HashMap<String, String>) {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let q = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (url_decode(k), url_decode(v))
        })
        .collect();
    (route.trim_end_matches('/').to_string(), q)
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16) {
                    Ok(x) => {
                        out.push(x);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `a=1&b=2` with encoded values; empty values are left out.
fn qs(pairs: &[(&str, &str)]) -> String {
    pairs.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{k}={}", url_encode(v))).collect::<Vec<_>>().join("&")
}

/// Plex paging parameters (added by the paged lists) as (start, size).
fn paging(q: &HashMap<String, String>) -> Option<(usize, usize)> {
    let start = q.get("X-Plex-Container-Start")?.parse().ok()?;
    let size = q.get("X-Plex-Container-Size")?.parse().ok()?;
    Some((start, size))
}

fn page_args(p: Option<(usize, usize)>) -> String {
    match p {
        Some((start, size)) => format!("&StartIndex={start}&Limit={size}"),
        None => String::new(),
    }
}

/// For answers that cannot page: cuts the requested window out of the whole list.
fn window((items, total): (Vec<Item>, i64), p: Option<(usize, usize)>) -> (Vec<Item>, i64) {
    match p {
        Some((start, size)) => (items.into_iter().skip(start).take(size).collect(), total),
        None => (items, total),
    }
}

/// Item types listed by a library with Plex type number `ty`.
fn types_for(view: &str, ty: &str) -> &'static str {
    let ct = VIEWS.lock().unwrap().get(view).cloned().unwrap_or_default();
    match (ct.as_str(), ty) {
        (_, "2") => "Series",
        ("music", _) => "MusicAlbum",
        ("movies", _) => "Movie",
        ("tvshows", _) => "Series",
        ("homevideos", _) => "Video",
        ("musicvideos", _) => "MusicVideo",
        _ => "Movie,Series,Video",
    }
}

fn sort_for(plex: &str) -> (&'static str, &'static str) {
    let (field, dir) = plex.split_once(':').unwrap_or((plex, "asc"));
    let by = match field {
        "addedAt" => "DateCreated,SortName",
        "originallyAvailableAt" => "PremiereDate,ProductionYear,SortName",
        "audienceRating" => "CommunityRating,SortName",
        "lastViewedAt" => "DatePlayed,SortName",
        _ => "SortName",
    };
    (by, if dir == "desc" { "Descending" } else { "Ascending" })
}

// ---------------------------------------------------------------- fetch

/// Items for a Plex-style list path. `paged` is false when the whole list is wanted.
pub fn items(http: &Http, c: &Conn, path: &str) -> Result<(Vec<Item>, i64), String> {
    let x = Ctx::new(http, c);
    let (route, q) = split(path);
    let p = paging(&q);
    let parts: Vec<&str> = route.trim_start_matches('/').split('/').collect();
    let id = |n: usize| parts.get(n).map(|s| url_encode(&url_decode(s))).unwrap_or_default();
    let uid = x.uid();
    match parts.as_slice() {
        ["library", "sections", _, "all"] => {
            let view = url_decode(parts[2]);
            let ty = q.get("type").map(|s| s.as_str()).unwrap_or("1");
            let (by, order) = sort_for(q.get("sort").map(|s| s.as_str()).unwrap_or("titleSort"));
            let unwatched = q.contains_key("unwatched") || q.contains_key("unwatchedLeaves");
            let genre = q.get("genre").cloned().unwrap_or_default();
            let search = q.get("title").cloned().unwrap_or_default();
            let filters = if unwatched { "IsUnplayed" } else { "" };
            let args = qs(&[("ParentId", &view), ("SortBy", by), ("SortOrder", order), ("Filters", filters), ("GenreIds", &genre), ("SearchTerm", &search)]);
            if ty == "8" {
                // Album artists: matches Plex's artist libraries (and the tags shown elsewhere).
                let v = x.get(&format!("/Artists/AlbumArtists?userId={uid}&{args}&Fields={LIST_FIELDS}&{IMAGES}{}", page_args(p)))?;
                return Ok(list(&v));
            }
            x.items(&format!("{args}&Recursive=true&IncludeItemTypes={}&Fields={LIST_FIELDS}{}", types_for(&view, ty), page_args(p)))
        }
        ["library", "sections", _, "collections"] => x.items(&format!("IncludeItemTypes=BoxSet&Recursive=true&SortBy=SortName&Fields={LIST_FIELDS}{}", page_args(p))),
        ["playlists"] => {
            // Jellyfin filters playlists by media type; Emby drops the type filter
            // altogether when one is given, so it gets none.
            let media = match q.get("playlistType").map(|s| s.as_str()) {
                Some("audio") if !x.emby() => "&MediaTypes=Audio",
                Some("video") if !x.emby() => "&MediaTypes=Video",
                _ => "",
            };
            x.items(&format!("IncludeItemTypes=Playlist&Recursive=true&SortBy=SortName&Fields={LIST_FIELDS}{media}{}", page_args(p)))
        }
        ["playlists", _, "items"] => {
            let v = x.get(&format!("/Playlists/{}/Items?userId={uid}&Fields={PLAY_FIELDS}&{IMAGES}{}", id(1), page_args(p)))?;
            Ok(list(&v))
        }
        ["library", "collections", _, "children"] => x.items(&format!("ParentId={}&Fields={PLAY_FIELDS}{}", id(2), page_args(p))),
        ["library", "metadata", _, "children"] => {
            let item = url_decode(parts[2]);
            let (kind, series) = x.kind(&item)?;
            let e = url_encode(&item);
            match kind {
                Kind::Show => {
                    let v = x.get(&format!("/Shows/{e}/Seasons?userId={uid}&Fields={LIST_FIELDS}&{IMAGES}"))?;
                    Ok(window(list(&v), p))
                }
                Kind::Season if !series.is_empty() => {
                    let v = x.get(&format!("/Shows/{}/Episodes?userId={uid}&SeasonId={e}&Fields={PLAY_FIELDS}&{IMAGES}", url_encode(&series)))?;
                    Ok(window(list(&v), p))
                }
                Kind::Artist => x.items(&format!("AlbumArtistIds={e}&IncludeItemTypes=MusicAlbum&Recursive=true&SortBy=ProductionYear,SortName&SortOrder=Descending&Fields={LIST_FIELDS}{}", page_args(p))),
                Kind::Playlist => {
                    let v = x.get(&format!("/Playlists/{e}/Items?userId={uid}&Fields={PLAY_FIELDS}&{IMAGES}{}", page_args(p)))?;
                    Ok(list(&v))
                }
                // Files have no children (the server would answer with its root).
                k if k.playable() => Ok((vec![], 0)),
                _ => x.items(&format!("ParentId={e}&SortBy=ParentIndexNumber,IndexNumber,SortName&Fields={PLAY_FIELDS}{}", page_args(p))),
            }
        }
        ["library", "metadata", _, "allLeaves"] => {
            let item = url_decode(parts[2]);
            let e = url_encode(&item);
            match x.kind(&item)?.0 {
                Kind::Artist => x.items(&format!("ArtistIds={e}&IncludeItemTypes=Audio&Recursive=true&SortBy=Album,ParentIndexNumber,IndexNumber,SortName&Fields={PLAY_FIELDS}{}", page_args(p))),
                Kind::Show => {
                    let v = x.get(&format!("/Shows/{e}/Episodes?userId={uid}&Fields={PLAY_FIELDS}&{IMAGES}"))?;
                    Ok(window(list(&v), p))
                }
                k if k.playable() => Ok((vec![x.item(&item, PLAY_FIELDS)?], 1)),
                _ => x.items(&format!("ParentId={e}&Recursive=true&IsFolder=false&SortBy=ParentIndexNumber,IndexNumber,SortName&Fields={PLAY_FIELDS}{}", page_args(p))),
            }
        }
        ["library", "metadata", _] => {
            let it = x.item(&url_decode(parts[2]), DETAIL_FIELDS)?;
            Ok((vec![it], 1))
        }
        // Rows' "more" pages (keys of the hubs built below).
        ["mb", "continue"] => resume(&x, q.get("parent").map(|s| s.as_str()).unwrap_or(""), p, false),
        ["mb", "onDeck"] => next_up(&x, q.get("parent").map(|s| s.as_str()).unwrap_or(""), p),
        ["mb", "items"] => {
            let mut q = q.clone();
            q.remove("X-Plex-Container-Start");
            q.remove("X-Plex-Container-Size");
            let pairs: Vec<(&str, &str)> = q.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            x.items(&format!("{}&Fields={LIST_FIELDS}{}", qs(&pairs), page_args(p)))
        }
        _ => Err(tf!("Nicht unterstützt: {}", route)),
    }
}

pub fn meta(http: &Http, c: &Conn, id: &str) -> Result<Item, String> {
    let x = Ctx::new(http, c);
    let mut it = x.item(id, DETAIL_FIELDS)?;
    if it.kind == Kind::Show {
        // Plex's "On Deck" for a show: the next episode to watch.
        let v = x.get(&format!("/Shows/NextUp?userId={}&SeriesId={}&Limit=1&Fields={PLAY_FIELDS}&{IMAGES}", x.uid(), url_encode(id)));
        if let Ok(v) = v {
            it.on_deck = list(&v).0.into_iter().next().map(Box::new);
        }
    }
    Ok(it)
}

/// Genres of a library as `(name, id)`.
pub fn values(http: &Http, c: &Conn, path: &str) -> Result<Vec<(String, String)>, String> {
    let x = Ctx::new(http, c);
    let (route, _) = split(path);
    let parts: Vec<&str> = route.trim_start_matches('/').split('/').collect();
    match parts.as_slice() {
        ["library", "sections", view, "genre"] => {
            let v = x.get(&format!("/Genres?userId={}&ParentId={}&SortBy=SortName", x.uid(), url_encode(&url_decode(view))))?;
            Ok(items_of(&v).iter().map(|g| (s(g, "Name"), s(g, "Id"))).filter(|(n, _)| !n.is_empty()).collect())
        }
        _ => Ok(vec![]),
    }
}

pub fn hubs(http: &Http, c: &Conn, path: &str, sections: &[Section]) -> Result<Vec<Hub>, String> {
    let x = Ctx::new(http, c);
    let (route, q) = split(path);
    let parts: Vec<&str> = route.trim_start_matches('/').split('/').collect();
    let mut out = Vec::new();
    let push = |out: &mut Vec<Hub>, title: String, identifier: &str, key: String, r: Result<(Vec<Item>, i64), String>, limit: usize| {
        if let Ok((items, total)) = r {
            if !items.is_empty() {
                out.push(Hub { title, key, identifier: identifier.into(), more: total as usize > limit, items });
            }
        }
    };
    match parts.as_slice() {
        ["hubs"] => {
            // "onDeck" sorts first in the hero/side column, then "continue".
            let mut rows: Vec<Row> = vec![
                (t!("Weiterschauen").into(), "mb.onDeck.resume", "/mb/continue".into(), 16, Box::new(|| resume(&x, "", Some((0, 16)), false))),
                (t!("Als Nächstes").into(), "mb.continue.nextup", "/mb/onDeck".into(), 16, Box::new(|| next_up(&x, "", Some((0, 16))))),
            ];
            for sec in sections {
                let x = &x;
                rows.push((tf!("Neu in {}", sec.title), "mb.latest", latest_key(&sec.key), 15, Box::new(move || latest(x, &sec.key))));
            }
            out = rows_parallel(rows);
        }
        ["hubs", "sections", view] => {
            let view = url_decode(view);
            let ct = VIEWS.lock().unwrap().get(&view).cloned().unwrap_or_default();
            let v = view.as_str();
            let mut rows: Vec<Row> = vec![(
                t!("Weiterschauen").into(),
                "mb.resume",
                format!("/mb/continue?parent={}", url_encode(v)),
                16,
                Box::new(|| resume(&x, v, Some((0, 16)), ct == "music")),
            )];
            if ct == "tvshows" {
                rows.push((t!("Als Nächstes").into(), "mb.nextup", format!("/mb/onDeck?parent={}", url_encode(v)), 16, Box::new(|| next_up(&x, v, Some((0, 16))))));
            }
            rows.push((t!("Zuletzt hinzugefügt").into(), "mb.latest", latest_key(v), 15, Box::new(|| latest(&x, v))));
            let types = types_for(v, "1");
            for (title, args) in [
                (t!("Favoriten"), qs(&[("ParentId", v), ("Recursive", "true"), ("IncludeItemTypes", types), ("Filters", "IsFavorite"), ("SortBy", "SortName")])),
                (t!("Am besten bewertet"), qs(&[("ParentId", v), ("Recursive", "true"), ("IncludeItemTypes", types), ("SortBy", "CommunityRating,SortName"), ("SortOrder", "Descending")])),
                (t!("Noch nicht gesehen"), qs(&[("ParentId", v), ("Recursive", "true"), ("IncludeItemTypes", types), ("Filters", "IsUnplayed"), ("SortBy", "Random")])),
            ] {
                if types == "MusicAlbum" && title == t!("Noch nicht gesehen") {
                    continue;
                }
                let x = &x;
                let key = format!("/mb/items?{args}");
                rows.push((title.into(), "mb.items", key, 16, Box::new(move || x.items(&format!("{args}&Fields={HUB_FIELDS}&Limit=16")))));
            }
            out = rows_parallel(rows);
        }
        ["hubs", "search"] => {
            let query = q.get("query").cloned().unwrap_or_default();
            let args = qs(&[("SearchTerm", &query), ("Recursive", "true"), ("IncludeItemTypes", "Movie,Series,Episode,BoxSet,MusicArtist,MusicAlbum,Audio,Video,MusicVideo"), ("Limit", "80")]);
            let (found, _) = x.items(&format!("{args}&Fields={HUB_FIELDS}"))?;
            for (kind, title) in [
                (Kind::Movie, t!("Filme")),
                (Kind::Show, t!("Serien")),
                (Kind::Episode, t!("Episoden")),
                (Kind::Collection, t!("Sammlungen")),
                (Kind::Artist, t!("Interpreten")),
                (Kind::Album, t!("Alben")),
                (Kind::Track, t!("Titel")),
                (Kind::Clip, "Videos"),
            ] {
                let items: Vec<Item> = found.iter().filter(|i| i.kind == kind).take(20).cloned().collect();
                let n = items.len() as i64;
                push(&mut out, title.into(), "mb.search", String::new(), Ok((items, n)), 20);
            }
        }
        ["library", "metadata", id, "related"] => {
            let v = x.get(&format!("/Items/{}/Similar?userId={}&Limit=16&Fields={HUB_FIELDS}&{IMAGES}", url_encode(&url_decode(id)), x.uid()))?;
            push(&mut out, t!("Ähnliche Titel").into(), "mb.similar", String::new(), Ok(list(&v)), 16);
        }
        _ => return Err(tf!("Nicht unterstützt: {}", route)),
    }
    Ok(out)
}

/// One row of a page: title, identifier, "more" key, row size, loader.
type Row<'a> = (String, &'static str, String, usize, Box<dyn FnOnce() -> Result<(Vec<Item>, i64), String> + Send + 'a>);

/// Loads the rows of a page at the same time (a page is several requests)
/// and keeps the ones with items, in order.
fn rows_parallel(rows: Vec<Row>) -> Vec<Hub> {
    std::thread::scope(|sc| {
        let running: Vec<_> = rows.into_iter().map(|(title, id, key, limit, load)| (title, id, key, limit, sc.spawn(load))).collect();
        running
            .into_iter()
            .filter_map(|(title, identifier, key, limit, h)| {
                let (items, total) = h.join().ok()?.ok()?;
                (!items.is_empty()).then(|| Hub { title, key, identifier: identifier.into(), more: total as usize > limit, items })
            })
            .collect()
    })
}

fn latest_key(view: &str) -> String {
    let ct = VIEWS.lock().unwrap().get(view).cloned().unwrap_or_default();
    let types = if ct == "tvshows" { "Episode" } else { types_for(view, "1") };
    format!("/mb/items?{}", qs(&[("ParentId", view), ("Recursive", "true"), ("IncludeItemTypes", types), ("SortBy", "DateCreated,SortName"), ("SortOrder", "Descending")]))
}

/// Newest items of a library (TV: grouped by show, as on the server's own home).
fn latest(x: &Ctx, view: &str) -> Result<(Vec<Item>, i64), String> {
    let v = x.get(&format!("/Users/{}/Items/Latest?ParentId={}&Limit=16&Fields={HUB_FIELDS}&{IMAGES}", x.uid(), url_encode(view)))?;
    Ok(list(&v))
}

/// Started videos. Emby mixes the next episode of started shows into this
/// list (position 0); those belong to "next up" and are left out here.
fn resume(x: &Ctx, parent: &str, p: Option<(usize, usize)>, any_media: bool) -> Result<(Vec<Item>, i64), String> {
    let media = if any_media { "" } else { "&MediaTypes=Video" };
    let v = x.get(&format!("/Users/{}/Items/Resume?{}&Fields={HUB_FIELDS}&{IMAGES}{media}{}", x.uid(), qs(&[("ParentId", parent)]), page_args(p)))?;
    let (mut items, total) = list(&v);
    if x.emby() {
        items.retain(|i| i.view_offset_ms > 0);
    }
    Ok((items, total))
}

/// Next episodes to watch. Jellyfin computes them library-wide; Emby only per
/// show, so there they come from the zero-position rows of the resume list.
fn next_up(x: &Ctx, parent: &str, p: Option<(usize, usize)>) -> Result<(Vec<Item>, i64), String> {
    if x.emby() {
        let v = x.get(&format!("/Users/{}/Items/Resume?{}&Fields={HUB_FIELDS}&{IMAGES}&Limit=100", x.uid(), qs(&[("ParentId", parent)])))?;
        let items: Vec<Item> = list(&v).0.into_iter().filter(|i| i.kind == Kind::Episode && i.view_offset_ms == 0).collect();
        let n = items.len() as i64;
        return Ok(window((items, n), p));
    }
    let cutoff = iso_days_ago(365);
    let v = x.get(&format!(
        "/Shows/NextUp?userId={}&{}&Fields={HUB_FIELDS}&{IMAGES}&EnableResumable=false&NextUpDateCutoff={}{}",
        x.uid(),
        qs(&[("ParentId", parent)]),
        url_encode(&cutoff),
        page_args(p)
    ))?;
    Ok(list(&v))
}

// ----------------------------------------------------------------- send

/// Carries out a Plex-style write (`/:/scrobble`, `/:/unscrobble`, `/:/timeline`).
pub fn send(http: &Http, c: &Conn, path: &str) -> Result<(), String> {
    let x = Ctx::new(http, c);
    let (route, q) = split(path);
    let key = q.get("key").or_else(|| q.get("ratingKey")).cloned().unwrap_or_default();
    let key = key.rsplit('/').next().unwrap_or(&key).to_string();
    if key.is_empty() {
        return Ok(());
    }
    let played = if x.emby() {
        format!("/Users/{}/PlayedItems/{}", x.uid(), url_encode(&key))
    } else {
        format!("/UserPlayedItems/{}?userId={}", url_encode(&key), x.uid())
    };
    match route.as_str() {
        "/:/scrobble" => x.send("POST", &played, None),
        "/:/unscrobble" => x.send("DELETE", &played, None),
        "/:/timeline" => {
            let ms: i64 = q.get("time").and_then(|t| t.parse().ok()).unwrap_or(0);
            let state = q.get("state").map(|s| s.as_str()).unwrap_or("playing");
            let tag = format!("{}|{key}", c.base);
            let mut body = json!({ "ItemId": key, "PositionTicks": ms * 10_000 });
            if x.emby() {
                // Emby answers 400 without a play session id.
                body["PlaySessionId"] = json!(format!("velouna-{key}"));
            }
            if state == "stopped" {
                PLAYING.lock().unwrap().remove(&tag);
                return x.send("POST", "/Sessions/Playing/Stopped", Some(&body));
            }
            body["IsPaused"] = json!(state == "paused");
            body["CanSeek"] = json!(true);
            body["IsMuted"] = json!(false);
            body["PlayMethod"] = json!("DirectPlay");
            let started = PLAYING.lock().unwrap().insert(tag.clone());
            let r = x.send("POST", if started { "/Sessions/Playing" } else { "/Sessions/Playing/Progress" }, Some(&body));
            if r.is_err() && started {
                PLAYING.lock().unwrap().remove(&tag);
            }
            r
        }
        _ => Ok(()),
    }
}

// ------------------------------------------------------------------ map

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}
fn n(v: &Value, k: &str) -> i64 {
    match v.get(k) {
        Some(Value::Number(x)) => x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)).unwrap_or(0),
        _ => 0,
    }
}
fn on(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(|x| x.as_i64())
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or(false)
}
fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}
fn items_of(v: &Value) -> &[Value] {
    match v {
        Value::Array(a) => a,
        _ => arr(v, "Items"),
    }
}

fn list(v: &Value) -> (Vec<Item>, i64) {
    let items: Vec<Item> = items_of(v).iter().map(map).collect();
    let total = on(v, "TotalRecordCount").filter(|t| *t >= items.len() as i64).unwrap_or(items.len() as i64);
    (items, total)
}

fn kind_of(t: &str) -> Kind {
    match t {
        "Movie" => Kind::Movie,
        "Series" => Kind::Show,
        "Season" => Kind::Season,
        "Episode" => Kind::Episode,
        "BoxSet" => Kind::Collection,
        "Playlist" => Kind::Playlist,
        "MusicArtist" => Kind::Artist,
        "MusicAlbum" => Kind::Album,
        "Audio" => Kind::Track,
        "Video" | "MusicVideo" | "Trailer" => Kind::Clip,
        "Photo" => Kind::Photo,
        _ => Kind::Other,
    }
}

fn img(id: &str, kind: &str, tag: &str) -> String {
    if id.is_empty() || tag.is_empty() { String::new() } else { format!("/Items/{id}/Images/{kind}?tag={tag}") }
}

fn ticks_ms(t: i64) -> i64 {
    t / 10_000
}

fn iso_days_ago(days: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let z = now / 86_400 - days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y:04}-{m:02}-{d:02}T00:00:00Z")
}

fn resolution(h: i64, w: i64) -> String {
    match () {
        _ if h >= 2000 || w >= 3500 => "4k".into(),
        _ if h >= 1000 || w >= 1900 => "1080".into(),
        _ if h >= 700 || w >= 1260 => "720".into(),
        _ if h >= 560 => "576".into(),
        _ if h > 0 => "480".into(),
        _ => String::new(),
    }
}

fn sub_ext(codec: &str) -> Option<&'static str> {
    match codec.to_lowercase().as_str() {
        "subrip" | "srt" => Some("srt"),
        "ass" => Some("ass"),
        "ssa" => Some("ssa"),
        "webvtt" | "vtt" => Some("vtt"),
        "mov_text" | "text" => Some("srt"),
        _ => None,
    }
}

fn media(v: &Value, id: &str, audio: bool) -> Vec<MediaVersion> {
    let seg = if audio { "Audio" } else { "Videos" };
    let mut out: Vec<MediaVersion> = arr(v, "MediaSources")
        .iter()
        .map(|m| {
            let msid = s(m, "Id");
            let streams = arr(m, "MediaStreams");
            let video = streams.iter().find(|x| s(x, "Type") == "Video");
            let (w, h) = video.map(|x| (n(x, "Width"), n(x, "Height"))).unwrap_or((0, 0));
            let range = video.map(|x| s(x, "VideoRangeType")).unwrap_or_default();
            let hdr = match range.as_str() {
                r if r.starts_with("DOVI") => "Dolby Vision".to_string(),
                "HDR10" | "HDR10Plus" | "HLG" => range.clone(),
                _ if video.is_some_and(|x| s(x, "VideoRange") == "HDR") => "HDR".into(),
                _ => String::new(),
            };
            let stream_infos = streams
                .iter()
                .filter_map(|x| {
                    let st = match s(x, "Type").as_str() {
                        "Video" => 1,
                        "Audio" => 2,
                        "Subtitle" => 3,
                        _ => return None,
                    };
                    let codec = s(x, "Codec");
                    let external_key = if st == 3 && b(x, "IsExternal") {
                        sub_ext(&codec).map(|e| format!("/Videos/{id}/{msid}/Subtitles/{}/0/Stream.{e}", n(x, "Index"))).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    Some(StreamInfo {
                        stream_type: st,
                        language: s(x, "Language"),
                        title: s(x, "Title"),
                        display: s(x, "DisplayTitle"),
                        external_key,
                    })
                })
                .collect();
            MediaVersion {
                resolution: resolution(h, w),
                video_codec: video.map(|x| s(x, "Codec")).unwrap_or_default(),
                hdr,
                parts: vec![Part {
                    key: format!("/{seg}/{id}/stream?Static=true&MediaSourceId={}", url_encode(&msid)),
                    file: s(m, "Path"),
                    size: n(m, "Size"),
                    streams: stream_infos,
                }],
            }
        })
        .collect();
    if out.is_empty() {
        // Lists without file details still get a playable stream (the server's default version).
        out.push(MediaVersion {
            parts: vec![Part { key: format!("/{seg}/{id}/stream?Static=true"), ..Default::default() }],
            ..Default::default()
        });
    }
    out
}

/// One `BaseItemDto` as a Plex item.
pub fn map(v: &Value) -> Item {
    let ty = s(v, "Type");
    let kind = kind_of(&ty);
    let id = s(v, "Id");
    let tags = v.get("ImageTags").cloned().unwrap_or(Value::Null);
    let primary = img(&id, "Primary", &s(&tags, "Primary"));
    let backdrop = arr(v, "BackdropImageTags").first().and_then(|t| t.as_str()).map(|t| format!("/Items/{id}/Images/Backdrop/0?tag={t}")).unwrap_or_default();
    let parent_backdrop = match (s(v, "ParentBackdropItemId"), arr(v, "ParentBackdropImageTags").first().and_then(|t| t.as_str())) {
        (pid, Some(t)) if !pid.is_empty() => format!("/Items/{pid}/Images/Backdrop/0?tag={t}"),
        _ => String::new(),
    };
    let series_thumb = img(&s(v, "SeriesId"), "Primary", &s(v, "SeriesPrimaryImageTag"));
    let parent_primary = img(&s(v, "ParentPrimaryImageItemId"), "Primary", &s(v, "ParentPrimaryImageTag"));
    let album_thumb = img(&s(v, "AlbumId"), "Primary", &s(v, "AlbumPrimaryImageTag"));
    let ud = v.get("UserData").cloned().unwrap_or(Value::Null);
    let played = b(&ud, "Played");
    let unplayed = n(&ud, "UnplayedItemCount");
    let folder = matches!(kind, Kind::Show | Kind::Season);
    let (leaf, viewed) = match on(v, "RecursiveItemCount") {
        Some(r) if folder && r > 0 => (r, if played { r } else { (r - unplayed).max(0) }),
        _ if folder => {
            if played {
                (1, 1)
            } else {
                (unplayed, 0)
            }
        }
        Some(r) => (r, 0),
        None => (0, 0),
    };
    let first_artist = arr(v, "AlbumArtists").first().cloned().unwrap_or(Value::Null);
    let (mut parent_title, mut parent_key, mut grandparent_title, mut grandparent_key) = (String::new(), String::new(), String::new(), String::new());
    let (mut thumb, mut art, mut grandparent_thumb, mut grandparent_art) = (primary.clone(), backdrop.clone(), String::new(), String::new());
    match kind {
        Kind::Episode => {
            parent_title = s(v, "SeasonName");
            parent_key = s(v, "SeasonId");
            grandparent_title = s(v, "SeriesName");
            grandparent_key = s(v, "SeriesId");
            grandparent_thumb = series_thumb.clone();
            grandparent_art = parent_backdrop.clone();
            if thumb.is_empty() {
                thumb = series_thumb.clone();
            }
        }
        Kind::Season => {
            parent_title = s(v, "SeriesName");
            parent_key = s(v, "SeriesId");
            if thumb.is_empty() {
                thumb = if series_thumb.is_empty() { parent_primary.clone() } else { series_thumb.clone() };
            }
            if art.is_empty() {
                art = parent_backdrop.clone();
            }
        }
        Kind::Album => {
            parent_title = [s(v, "AlbumArtist"), s(&first_artist, "Name")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
            parent_key = s(&first_artist, "Id");
            if art.is_empty() {
                art = parent_backdrop.clone();
            }
        }
        Kind::Track => {
            parent_title = s(v, "Album");
            parent_key = s(v, "AlbumId");
            grandparent_title = [s(v, "AlbumArtist"), s(&first_artist, "Name")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
            grandparent_key = s(&first_artist, "Id");
            if thumb.is_empty() {
                thumb = album_thumb.clone();
            }
            if art.is_empty() {
                art = parent_backdrop.clone();
            }
        }
        _ => {
            if art.is_empty() {
                art = parent_backdrop.clone();
            }
        }
    }
    let people = arr(v, "People");
    let named = |t: &str| people.iter().filter(|p| s(p, "Type") == t).map(|p| s(p, "Name")).filter(|x| !x.is_empty()).collect::<Vec<_>>();
    let mut ratings = Vec::new();
    if let Some(r) = v.get("CommunityRating").and_then(|x| x.as_f64()).filter(|r| *r > 0.0) {
        ratings.push(Rating { source: "themoviedb://image.rating".into(), value: r, audience: true });
    }
    if let Some(r) = v.get("CriticRating").and_then(|x| x.as_f64()).filter(|r| *r > 0.0) {
        let ripe = if r >= 60.0 { "ripe" } else { "rotten" };
        ratings.push(Rating { source: format!("rottentomatoes://image.rating.{ripe}"), value: r, audience: false });
    }
    let duration_ms = ticks_ms(n(v, "RunTimeTicks"));
    let playable = kind.playable();
    {
        let series = if kind == Kind::Season || kind == Kind::Episode { s(v, "SeriesId") } else { String::new() };
        let mut k = KINDS.lock().unwrap();
        if k.len() > 50_000 {
            k.clear();
        }
        k.insert(id.clone(), (kind, series));
    }
    Item {
        rating_key: id.clone(),
        kind,
        title: s(v, "Name"),
        original_title: s(v, "OriginalTitle"),
        parent_title,
        grandparent_title,
        parent_key,
        grandparent_key,
        index: on(v, "IndexNumber"),
        parent_index: on(v, "ParentIndexNumber"),
        year: on(v, "ProductionYear").filter(|y| *y > 0),
        summary: s(v, "Overview"),
        tagline: arr(v, "Taglines").first().and_then(|t| t.as_str()).unwrap_or("").to_string(),
        studio: arr(v, "Studios").first().map(|x| s(x, "Name")).unwrap_or_default(),
        content_rating: s(v, "OfficialRating"),
        duration_ms,
        view_offset_ms: ticks_ms(n(&ud, "PlaybackPositionTicks")),
        view_count: if played { n(&ud, "PlayCount").max(1) } else { 0 },
        leaf_count: leaf,
        viewed_leaf_count: viewed,
        child_count: n(v, "ChildCount"),
        thumb,
        art,
        grandparent_thumb,
        grandparent_art,
        composite: String::new(),
        originally_available: s(v, "PremiereDate").get(..10).unwrap_or("").to_string(),
        ratings,
        genres: arr(v, "Genres").iter().filter_map(|g| g.as_str().map(String::from)).collect(),
        directors: named("Director"),
        writers: named("Writer"),
        roles: people
            .iter()
            .filter(|p| matches!(s(p, "Type").as_str(), "Actor" | "GuestStar"))
            .map(|p| Role { name: s(p, "Name"), role: s(p, "Role"), thumb: img(&s(p, "Id"), "Primary", &s(p, "PrimaryImageTag")) })
            .collect(),
        media: if playable { media(v, &id, kind == Kind::Track) } else { vec![] },
        library_section_id: String::new(),
        library_section_title: String::new(),
        on_deck: None,
    }
}

/// Artwork URL for an image path from [`map`], sized for a `w`×`h` box.
pub fn image_url(base: &str, backend: Backend, path: &str, w: u32, h: u32) -> String {
    let sep = if path.contains('?') { '&' } else { '?' };
    match backend {
        // Jellyfin scales to cover the box; Emby only knows the max sizes.
        Backend::Jellyfin => format!("{base}{path}{sep}fillWidth={w}&fillHeight={h}&quality=90"),
        _ => format!("{base}{path}{sep}maxWidth={}&quality=90", w.max(h * 2 / 3)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_people_type() {
        assert_eq!(candidates("jf.example.com/"), vec!["https://jf.example.com", "http://jf.example.com:8096", "https://jf.example.com:8920", "http://jf.example.com"]);
        assert_eq!(candidates("192.168.1.5:8096"), vec!["http://192.168.1.5:8096", "https://192.168.1.5:8096"]);
        assert_eq!(candidates("https://host/jellyfin/web/#/home"), vec!["https://host/jellyfin"]);
        assert_eq!(candidates("  "), Vec::<String>::new());
    }

    #[test]
    fn paths_and_dates() {
        let (r, q) = split("/library/sections/ab/all?type=1&sort=addedAt:desc&title=G%C3%BCnter%20X&X-Plex-Container-Start=60&X-Plex-Container-Size=60");
        assert_eq!(r, "/library/sections/ab/all");
        assert_eq!(q["title"], "Günter X");
        assert_eq!(paging(&q), Some((60, 60)));
        assert_eq!(sort_for("addedAt:desc"), ("DateCreated,SortName", "Descending"));
        assert_eq!(iso_days_ago(0).len(), 20);
        assert_eq!(url_decode("a%2"), "a%2");
    }

    /// Against a real server: `VELOUNA_MB_URL=… VELOUNA_MB_USER=… cargo test live_server -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_server() {
        let url = std::env::var("VELOUNA_MB_URL").expect("VELOUNA_MB_URL");
        let user = std::env::var("VELOUNA_MB_USER").unwrap_or_default();
        let pw = std::env::var("VELOUNA_MB_PASS").unwrap_or_default();
        let http = Http::new("velouna-test".into());
        let (srv, name) = login(&http, &url, &user, &pw, "velouna-test-device").expect("login");
        println!("signed in as {name} on {} ({:?})", srv.name, srv.backend);
        let c = Conn { base: srv.connections[0].uri.clone(), token: srv.token, backend: srv.backend, user_id: srv.user_id, device_id: srv.device_id };
        let secs = connect(&http, &c).expect("views");
        for s in &secs {
            println!("section {} {}", s.title, s.kind);
        }
        let home = hubs(&http, &c, "/hubs?count=16", &secs).expect("home");
        for h in &home {
            println!("hub {} [{}] {} items, more={} key={}", h.title, h.identifier, h.items.len(), h.more, h.key);
        }
        for s in &secs {
            let ty = match s.kind.as_str() { "show" => 2, "artist" => 8, _ => 1 };
            let path = format!("/library/sections/{}/all?type={ty}&sort=titleSort:asc&X-Plex-Container-Start=0&X-Plex-Container-Size=60", s.key);
            let (list, total) = items(&http, &c, &path).expect("all");
            println!("{}: {total} total, first: {:?}", s.title, list.iter().take(3).map(|i| (i.title.clone(), i.kind, i.leaf_count, i.viewed_leaf_count)).collect::<Vec<_>>());
            let g = values(&http, &c, &format!("/library/sections/{}/genre?type={ty}", s.key)).expect("genres");
            println!("  genres: {}", g.len());
            let sh = hubs(&http, &c, &format!("/hubs/sections/{}?count=16", s.key), &secs).expect("section hubs");
            println!("  section hubs: {:?}", sh.iter().map(|h| (h.title.clone(), h.items.len())).collect::<Vec<_>>());
            if let Some(first) = list.first() {
                let kids = items(&http, &c, &format!("/library/metadata/{}/children", first.rating_key)).expect("children");
                println!("  children of {}: {:?}", first.title, kids.0.iter().take(4).map(|i| (i.title.clone(), i.kind)).collect::<Vec<_>>());
                if let Some(k) = kids.0.first() {
                    let gk = items(&http, &c, &format!("/library/metadata/{}/children", k.rating_key)).expect("grandchildren");
                    println!("  children of {}: {:?}", k.title, gk.0.iter().take(3).map(|i| (i.title.clone(), i.kind, i.first_part().map(|p| p.key.clone()))).collect::<Vec<_>>());
                }
                let leaves = items(&http, &c, &format!("/library/metadata/{}/allLeaves", first.rating_key)).expect("leaves");
                println!("  leaves: {}", leaves.0.len());
                let m = meta(&http, &c, &first.rating_key).expect("meta");
                println!("  meta: {} {:?} roles={} genres={:?} media={} deck={:?}", m.title, m.year, m.roles.len(), m.genres, m.media.len(), m.on_deck.as_ref().map(|d| d.title.clone()));
            }
        }
        let found = hubs(&http, &c, "/hubs/search?query=the&limit=20", &secs).expect("search");
        println!("search: {:?}", found.iter().map(|h| (h.title.clone(), h.items.len())).collect::<Vec<_>>());
        for h in home.iter().filter(|h| h.more) {
            let (it, total) = items(&http, &c, &format!("{}{}X-Plex-Container-Start=0&X-Plex-Container-Size=60", h.key, if h.key.contains('?') { '&' } else { '?' })).expect("more");
            println!("more {}: {} of {total}", h.title, it.len());
        }
    }

    #[test]
    fn maps_an_episode() {
        let v: Value = serde_json::from_str(
            r#"{"Name":"Earthfall","Id":"e1","Type":"Episode","IndexNumber":1,"ParentIndexNumber":1,"SeriesName":"Pioneer One","SeriesId":"s1","SeasonId":"se1",
            "SeriesPrimaryImageTag":"t2","ImageTags":{"Primary":"t1"},"RunTimeTicks":18963580000,"UserData":{"PlaybackPositionTicks":600000000,"Played":false},
            "MediaSources":[{"Id":"m1","Container":"mkv","Path":"/x.mkv","MediaStreams":[{"Type":"Video","Codec":"hevc","Width":3840,"Height":2160,"VideoRangeType":"DOVIWithHDR10"},
            {"Type":"Subtitle","Codec":"subrip","Index":3,"IsExternal":true}]}]}"#,
        )
        .unwrap();
        let it = map(&v);
        assert_eq!(it.kind, Kind::Episode);
        assert_eq!(it.episode_code(), "S1 E1");
        assert_eq!(it.grandparent_key, "s1");
        assert_eq!(it.parent_key, "se1");
        assert_eq!(it.view_offset_ms, 60_000);
        assert_eq!(it.thumb, "/Items/e1/Images/Primary?tag=t1");
        assert_eq!(it.grandparent_thumb, "/Items/s1/Images/Primary?tag=t2");
        let m = &it.media[0];
        assert_eq!((m.resolution.as_str(), m.hdr.as_str()), ("4k", "Dolby Vision"));
        assert_eq!(m.parts[0].key, "/Videos/e1/stream?Static=true&MediaSourceId=m1");
        assert_eq!(m.parts[0].streams[1].external_key, "/Videos/e1/m1/Subtitles/3/0/Stream.srt");
    }
}
