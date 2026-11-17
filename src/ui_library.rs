//! Library: compact side panel next to the video and the full library page (with home).

use crate::app::{App, Dialog, ThumbState, View};
use crate::config::{Pin, PinTarget};
use crate::library::{LibLoc, MusicView, SortMode, ViewMode, name_of};
use crate::media::{self, MediaTools};
use crate::theme;
use crate::ui_panels::{LibDrag, show_in_explorer};
use crate::widgets::{self, icon_button};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Layout, Rect, RichText, Sense, Stroke, StrokeKind, TextureId,
    Ui, pos2, vec2,
};
use egui_phosphor::regular as ph;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
enum Kind {
    Folder(PathBuf),
    File(String),
    Stream(usize),
    Playlist(usize),
    Go(LibLoc),
    NewStream,
    NewPlaylist,
    /// Public TV channel (index into the iptv catalog).
    Channel(usize),
    /// Starred TV channel (index into `data.tv_favorites`).
    Fav(usize),
    /// Radio station from the directory.
    Radio(Arc<crate::radio::Station>),
    /// Starred radio station (index into `data.radio_favorites`).
    RadioFav(usize),
    /// Status line, does nothing.
    Info,
    TvRetry,
}

/// "Jetzt: Tatort · bis 21:45 · <rest>" when the guide knows the channel.
fn with_now(guides: &HashMap<String, Arc<crate::epg::Guide>>, country: Option<&str>, keys: &[String], rest: &str) -> String {
    let now = crate::epg::now();
    let cur = country.and_then(|c| guides.get(c)).and_then(|g| g.lookup(keys)).and_then(|p| crate::epg::now_next(p, now));
    match cur {
        Some((p, _)) if !p.title.is_empty() => {
            let head = tf!("Jetzt: {} · bis {}", p.title, crate::epg::clock(p.stop));
            if rest.is_empty() { head } else { format!("{head} · {rest}") }
        }
        _ => rest.to_string(),
    }
}

/// The built-in iptv-org list opens the language browser instead of
/// loading thousands of channels into the queue.
/// Icon of a TV channel / radio station in the history.
fn live_icon(h: &crate::config::HistoryEntry) -> Option<&'static str> {
    h.live.then_some(if h.audio_only { ph::RADIO } else { ph::TELEVISION_SIMPLE })
}

/// "Radio · Live" / "TV · Live" for a channel in the history.
fn live_label(h: &crate::config::HistoryEntry) -> String {
    format!("{} · Live", if h.audio_only { "Radio" } else { "TV" })
}

fn is_iptv_org(url: &str) -> bool {
    url.starts_with("https://iptv-org.github.io/iptv/index")
}

#[derive(Clone)]
struct Entry {
    kind: Kind,
    title: String,
    sub: String,
    icon: &'static str,
}

enum Click {
    Open(usize),
    PlayAll,
    Menu(MenuAct, usize),
}

enum MenuAct {
    Play,
    PlayNext,
    Enqueue,
    AddToPlaylist,
    Pin,
    Watched(bool),
    Explorer,
    Remove,
    /// TV channel: star / unstar.
    Favorite,
    /// TV channel: program guide.
    Guide,
}

impl App {
    fn thumb_for(&mut self, ctx: &egui::Context, path: &str) -> Option<TextureId> {
        if media::is_url(path) || !self.tools.available() {
            return None;
        }
        let key = self.thumb_keys.entry(path.to_string()).or_insert_with(|| MediaTools::thumb_key(path)).clone();
        match self.thumbs.get(&key) {
            Some(ThumbState::Ready(t)) => Some(t.id()),
            Some(_) => None,
            None => {
                if let Some(rgba) = self.tools.request_thumb(path, &key) {
                    let img = egui::ColorImage::from_rgba_unmultiplied([media::THUMB_W, media::THUMB_H], &rgba);
                    let tex = ctx.load_texture(&key, img, egui::TextureOptions::LINEAR);
                    let id = tex.id();
                    self.thumbs.insert(key, ThumbState::Ready(tex));
                    Some(id)
                } else {
                    self.thumbs.insert(key, ThumbState::Loading);
                    None
                }
            }
        }
    }

    fn progress_of(&self, path: &str) -> Option<(f32, bool)> {
        self.data.history_for(path).map(|h| (h.progress(), h.finished))
    }

    fn files_entries(&mut self, files: Vec<String>) -> Vec<Entry> {
        let mut files = files;
        let hist = |p: &String| self.data.history_for(p).map(|h| (h.finished, h.progress())).unwrap_or((false, 0.0));
        match self.lib.sort {
            SortMode::TitleAz => files.sort_by(|a, b| media::natural_cmp(&media::display_name(a), &media::display_name(b))),
            SortMode::TitleZa => files.sort_by(|a, b| media::natural_cmp(&media::display_name(b), &media::display_name(a))),
            SortMode::Newest | SortMode::Oldest => {
                let mut m: Vec<(u64, String)> = files.into_iter().map(|f| (media::file_mtime(Path::new(&f)), f)).collect();
                m.sort_by(|a, b| a.0.cmp(&b.0));
                if self.lib.sort == SortMode::Newest {
                    m.reverse();
                }
                files = m.into_iter().map(|x| x.1).collect();
            }
            SortMode::UnwatchedFirst => files.sort_by(|a, b| hist(a).0.cmp(&hist(b).0).then(media::natural_cmp(a, b))),
            SortMode::WatchedFirst => files.sort_by(|a, b| hist(b).0.cmp(&hist(a).0).then(media::natural_cmp(a, b))),
        }
        files
            .into_iter()
            .map(|f| {
                let probe = self.tools.probe_cache.get(&f);
                let sub = probe.map(|p| media::fmt_time_short(p.duration)).unwrap_or_default();
                let title = if media::is_audio(&f) {
                    probe.and_then(|p| p.title.clone()).unwrap_or_else(|| media::display_name(&f))
                } else {
                    media::display_name(&f)
                };
                let icon = if media::is_audio(&f) { ph::MUSIC_NOTE } else if media::is_playlist(&f) { ph::PLAYLIST } else { ph::FILM_STRIP };
                Entry { kind: Kind::File(f), title, sub, icon }
            })
            .collect()
    }

    fn music_entries(&mut self, view: &MusicView) -> Vec<Entry> {
        let files: Vec<String> = self.lib.music_files.iter().map(|p| p.to_string_lossy().to_string()).collect();
        let tag = |app: &App, f: &str, which: u8| -> String {
            let p = app.tools.probe_cache.get(f);
            let v = match which {
                0 => p.and_then(|p| p.album.clone()),
                1 => p.and_then(|p| p.artist.clone()),
                _ => p.and_then(|p| p.genre.clone()),
            };
            v.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| t!("Unbekannt").into())
        };
        match view {
            MusicView::Songs => self.files_entries(files),
            MusicView::Albums | MusicView::Artists | MusicView::Genres => {
                let which = match view {
                    MusicView::Albums => 0,
                    MusicView::Artists => 1,
                    _ => 2,
                };
                let mut groups: std::collections::BTreeMap<String, usize> = Default::default();
                for f in &files {
                    *groups.entry(tag(self, f, which)).or_default() += 1;
                }
                groups
                    .into_iter()
                    .map(|(name, n)| Entry {
                        kind: Kind::Go(LibLoc::Music(match which {
                            0 => MusicView::Album(name.clone()),
                            1 => MusicView::Artist(name.clone()),
                            _ => MusicView::Genre(name.clone()),
                        })),
                        title: name,
                        sub: tf!("{n} Titel"),
                        icon: match which {
                            0 => ph::VINYL_RECORD,
                            1 => ph::USER,
                            _ => ph::TAG,
                        },
                    })
                    .collect()
            }
            MusicView::Album(a) | MusicView::Artist(a) | MusicView::Genre(a) => {
                let which = match view {
                    MusicView::Album(_) => 0,
                    MusicView::Artist(_) => 1,
                    _ => 2,
                };
                let mut sel: Vec<String> = files.into_iter().filter(|f| tag(self, f, which) == *a).collect();
                if which == 0 {
                    sel.sort_by_key(|f| self.tools.probe_cache.get(f).and_then(|p| p.track).unwrap_or(999));
                    let save = self.lib.sort;
                    self.lib.sort = SortMode::TitleAz;
                    let mut e = self.files_entries(sel.clone());
                    self.lib.sort = save;
                    // keep track order
                    e.sort_by_key(|x| match &x.kind {
                        Kind::File(f) => sel.iter().position(|s| s == f).unwrap_or(0),
                        _ => 0,
                    });
                    e
                } else {
                    self.files_entries(sel)
                }
            }
        }
    }

    fn entries(&mut self) -> Vec<Entry> {
        let loc = self.lib.loc.clone();
        let show_hidden = self.data.settings.show_hidden;
        let mut v = match &loc {
            LibLoc::Home | LibLoc::Plex(_) => vec![],
            LibLoc::Videos(None) => self
                .data
                .settings
                .video_folders
                .iter()
                .map(|f| Entry { kind: Kind::Folder(f.clone()), title: name_of(f), sub: f.to_string_lossy().to_string(), icon: ph::FOLDER_SIMPLE })
                .collect(),
            LibLoc::Videos(Some(dir)) | LibLoc::Folder(dir) | LibLoc::MusicFolder(Some(dir)) => {
                let l = self.lib.listing_for(dir, show_hidden);
                let folders = l.folders.clone();
                let files: Vec<String> = l.files.iter().map(|p| p.to_string_lossy().to_string()).collect();
                let mut e: Vec<Entry> = folders
                    .into_iter()
                    .map(|f| Entry { kind: Kind::Folder(f.clone()), title: name_of(&f), sub: String::new(), icon: ph::FOLDER_SIMPLE })
                    .collect();
                e.extend(self.files_entries(files));
                e
            }
            LibLoc::MusicFolder(None) => self
                .data
                .settings
                .music_folders
                .iter()
                .map(|f| Entry { kind: Kind::Folder(f.clone()), title: name_of(f), sub: f.to_string_lossy().to_string(), icon: ph::FOLDER_SIMPLE })
                .collect(),
            LibLoc::Music(view) => self.music_entries(view),
            LibLoc::Network => {
                let mut e = vec![Entry { kind: Kind::NewStream, title: t!("Netzwerkstream hinzufügen").into(), sub: String::new(), icon: ph::PLUS }];
                // TV and radio have their own pages.
                e.extend(self.data.streams.iter().enumerate().filter(|(_, s)| !is_iptv_org(&s.url) && !crate::radio::is_radio_browser(&s.url)).map(|(i, s)| Entry {
                    kind: Kind::Stream(i),
                    title: crate::i18n::tr(&s.name).to_string(),
                    sub: s.url.clone(),
                    icon: ph::BROADCAST,
                }));
                e
            }
            LibLoc::Playlists(None) => {
                let mut e = vec![Entry { kind: Kind::NewPlaylist, title: t!("Playlist erstellen").into(), sub: String::new(), icon: ph::PLUS }];
                e.extend(self.data.playlists.iter().enumerate().map(|(i, p)| Entry {
                    kind: Kind::Playlist(i),
                    title: p.name.clone(),
                    sub: tf!("{} Titel", p.items.len()),
                    icon: ph::PLAYLIST,
                }));
                e
            }
            LibLoc::Playlists(Some(i)) => {
                let items = self.data.playlists.get(*i).map(|p| p.items.clone()).unwrap_or_default();
                let save = self.lib.sort;
                self.lib.sort = SortMode::TitleAz;
                let mut e = self.files_entries(items.clone());
                self.lib.sort = save;
                e.sort_by_key(|x| match &x.kind {
                    Kind::File(f) => items.iter().position(|s| s == f).unwrap_or(0),
                    _ => 0,
                });
                e
            }
            LibLoc::Tv(sel) => self.tv_entries(sel.as_ref().map(|s| s.0.as_str())),
            LibLoc::Radio(sel) => self.radio_entries(sel.as_ref().map(|s| s.0.as_str())),
            LibLoc::Recent => {
                let mut h = self.data.history.clone();
                h.sort_by(|a, b| b.last_played.cmp(&a.last_played));
                h.into_iter()
                    .map(|h| Entry {
                        icon: live_icon(&h).unwrap_or(if h.audio_only { ph::MUSIC_NOTE } else if media::is_url(&h.path) { ph::BROADCAST } else { ph::FILM_STRIP }),
                        kind: Kind::File(h.path.clone()),
                        sub: if h.live {
                            live_label(&h)
                        } else if h.duration > 0.0 {
                            format!("{} / {}", media::fmt_time_short(h.position), media::fmt_time_short(h.duration))
                        } else {
                            String::new()
                        },
                        title: h.title,
                    })
                    .collect()
            }
        };
        let q = self.lib.search.trim().to_lowercase();
        if !q.is_empty() {
            v.retain(|e| e.title.to_lowercase().contains(&q) || matches!(e.kind, Kind::NewStream | Kind::NewPlaylist));
        }
        v
    }

    /// Languages (with a search over all channels), or one language's channels.
    fn tv_entries(&mut self, lang: Option<&str>) -> Vec<Entry> {
        use crate::iptv::{FAVORITES, TvState};
        // Favorites need no channel list.
        if lang == Some(FAVORITES) {
            let countries: Vec<String> = self.data.tv_favorites.iter().filter_map(|f| f.country.clone()).collect();
            let guides = self.tv_guides(countries, 8);
            return self
                .data
                .tv_favorites
                .iter()
                .enumerate()
                .map(|(i, f)| Entry {
                    kind: Kind::Fav(i),
                    title: f.title.clone(),
                    sub: with_now(&guides, f.country.as_deref(), &f.epg_keys, &f.sub),
                    icon: ph::STAR,
                })
                .collect();
        }
        let mut head = vec![];
        if lang.is_none() && !self.data.tv_favorites.is_empty() && self.lib.search.trim().is_empty() {
            let n = self.data.tv_favorites.len();
            head.push(Entry {
                kind: Kind::Go(LibLoc::Tv(Some((FAVORITES.into(), t!("TV-Favoriten").into())))),
                title: t!("TV-Favoriten").into(),
                sub: tf!("{n} Sender"),
                icon: ph::STAR,
            });
        }
        let info = |title: String, sub: String| Entry { kind: Kind::Info, title, sub, icon: ph::INFO };
        let cat = match self.tv.state() {
            TvState::Ready(c) => c.clone(),
            TvState::Failed(e) => {
                head.push(info(t!("Senderliste konnte nicht geladen werden").into(), e.clone()));
                head.push(Entry { kind: Kind::TvRetry, title: t!("Erneut versuchen").into(), sub: String::new(), icon: ph::ARROW_CLOCKWISE });
                return head;
            }
            _ => {
                head.push(info(t!("Senderliste wird geladen …").into(), "iptv-org".into()));
                return head;
            }
        };
        let q = self.lib.search.trim().to_lowercase();
        let ids: Vec<usize> = match lang {
            Some(code) => cat.by_lang.get(code).cloned().unwrap_or_default(),
            // Searching on the language page finds channels in every language.
            None if !q.is_empty() => {
                let mut seen = std::collections::HashSet::new();
                (0..cat.channels.len())
                    .filter(|&i| cat.channels[i].title.to_lowercase().contains(&q) && seen.insert(cat.channels[i].url.as_str()))
                    .take(500)
                    .collect()
            }
            None => vec![],
        };
        // Program guides of the countries most of these channels come from.
        let guides = self.tv_guides(ids.iter().filter_map(|&i| cat.channels[i].country.clone()).collect(), 3);
        let favs: std::collections::HashSet<&str> = self.data.tv_favorites.iter().map(|f| f.url.as_str()).collect();
        let channel = |i: usize| {
            let c = &cat.channels[i];
            Entry {
                kind: Kind::Channel(i),
                title: c.title.clone(),
                sub: with_now(&guides, c.country.as_deref(), &c.epg_keys, &cat.subtitle(c)),
                icon: if favs.contains(c.url.as_str()) { ph::STAR } else { ph::TELEVISION_SIMPLE },
            }
        };
        match lang {
            Some(_) => ids.into_iter().map(channel).collect(),
            None if !q.is_empty() => ids.into_iter().map(channel).collect(),
            None => head
                .into_iter()
                .chain(cat.languages().into_iter().map(|(code, label, n)| Entry {
                    kind: Kind::Go(LibLoc::Tv(Some((code, label.clone())))),
                    title: label,
                    sub: tf!("{n} Sender"),
                    icon: ph::TRANSLATE,
                }))
                .collect(),
        }
    }

    /// Radio: languages (with a search over all stations), one language's
    /// stations by popularity, or the favorites.
    fn radio_entries(&mut self, lang: Option<&str>) -> Vec<Entry> {
        use crate::radio::{FAVORITES, Load};
        if lang == Some(FAVORITES) {
            return self
                .data
                .radio_favorites
                .iter()
                .enumerate()
                .map(|(i, f)| Entry { kind: Kind::RadioFav(i), title: f.name.clone(), sub: f.sub.clone(), icon: ph::STAR })
                .collect();
        }
        let q = self.lib.search.trim().to_string();
        let info = |title: String, sub: String| Entry { kind: Kind::Info, title, sub, icon: ph::INFO };
        let failed = |e: String| {
            vec![
                info(t!("Senderliste konnte nicht geladen werden").into(), e),
                Entry { kind: Kind::TvRetry, title: t!("Erneut versuchen").into(), sub: String::new(), icon: ph::ARROW_CLOCKWISE },
            ]
        };
        let favs: std::collections::HashSet<String> = self.data.radio_favorites.iter().map(|f| f.uuid.clone()).collect();
        let station = |s: &Arc<crate::radio::Station>| Entry {
            kind: Kind::Radio(s.clone()),
            title: s.title(),
            sub: s.subtitle(),
            icon: if favs.contains(&s.stationuuid) { ph::STAR } else { ph::RADIO },
        };
        let list = |r: Load<crate::radio::Stations>, sub: &str| match r {
            Load::Ready(v) => v.iter().map(station).collect(),
            // The search filter keeps entries whose title matches: say what
            // is loading in the title.
            Load::Loading => vec![info(format!("{} {}", t!("Senderliste wird geladen …"), sub), "radio-browser.info".into())],
            Load::Failed(e) => failed(e),
        };
        match lang {
            Some(l) => list(self.radio.stations(l), ""),
            None if !q.is_empty() => list(self.radio.search(&q), &q),
            None => {
                let mut v = vec![];
                if !self.data.radio_favorites.is_empty() {
                    let n = self.data.radio_favorites.len();
                    v.push(Entry {
                        kind: Kind::Go(LibLoc::Radio(Some((FAVORITES.into(), t!("Radio-Favoriten").into())))),
                        title: t!("Radio-Favoriten").into(),
                        sub: tf!("{n} Radiosender"),
                        icon: ph::STAR,
                    });
                }
                match self.radio.languages() {
                    Load::Ready(langs) => v.extend(langs.iter().map(|l| {
                        let n = l.count;
                        Entry { kind: Kind::Go(LibLoc::Radio(Some((l.name.clone(), l.label.clone())))), title: l.label.clone(), sub: tf!("{n} Radiosender"), icon: ph::TRANSLATE }
                    })),
                    Load::Loading => v.push(info(t!("Senderliste wird geladen …").into(), "radio-browser.info".into())),
                    Load::Failed(e) => v.extend(failed(e)),
                }
                v
            }
        }
    }

    /// Loaded guides for the `max` most common of `countries` (starting
    /// downloads as needed).
    fn tv_guides(&mut self, countries: Vec<String>, max: usize) -> HashMap<String, Arc<crate::epg::Guide>> {
        let mut count: HashMap<String, usize> = HashMap::new();
        for c in countries {
            *count.entry(c).or_default() += 1;
        }
        let mut top: Vec<(String, usize)> = count.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut out = HashMap::new();
        for (c, _) in top.into_iter().take(max) {
            self.epg.want(&c);
            if let Some(g) = self.epg.guide(&c) {
                out.insert(c, g);
            }
        }
        out
    }

    /// (title, country, guide keys) of a channel or favorite entry.
    fn tv_guide_target(&self, kind: &Kind) -> Option<(String, String, Vec<String>)> {
        match kind {
            Kind::Channel(i) => self.tv.catalog().and_then(|c| {
                let ch = c.channels.get(*i)?;
                Some((ch.title.clone(), ch.country.clone()?, ch.epg_keys.clone()))
            }),
            Kind::Fav(i) => self.data.tv_favorites.get(*i).and_then(|f| Some((f.title.clone(), f.country.clone()?, f.epg_keys.clone()))),
            _ => None,
        }
    }

    /// (url, mpv options) of a channel or favorite entry.
    fn tv_item(&self, kind: &Kind) -> Option<(String, String)> {
        match kind {
            Kind::Channel(i) => self.tv.catalog().and_then(|c| c.channels.get(*i).map(|c| (c.url.clone(), c.mpv_options()))),
            Kind::Fav(i) => self.data.tv_favorites.get(*i).map(|f| (f.url.clone(), crate::iptv::favorite_options(f))),
            Kind::Radio(s) => Some((s.stream_url().to_string(), s.mpv_options())),
            Kind::RadioFav(i) => self.data.radio_favorites.get(*i).map(|f| (f.url.clone(), crate::radio::favorite_options(f))),
            _ => None,
        }
    }

    /// Notes logo and kind (radio) of a channel / station being queued.
    fn remember_stream(&mut self, kind: &Kind) {
        let Some((url, _)) = self.tv_item(kind) else { return };
        self.live_urls.insert(url.clone());
        if matches!(kind, Kind::Radio(_) | Kind::RadioFav(_)) {
            self.radio_urls.insert(url.clone());
        }
        if let Some(logo) = self.entry_logo(kind) {
            self.stream_logos.insert(url, logo);
        }
    }

    /// Logo URL of a radio station or TV channel entry.
    fn entry_logo(&self, kind: &Kind) -> Option<String> {
        let u = match kind {
            Kind::Radio(s) => s.favicon.clone(),
            Kind::RadioFav(i) => self.data.radio_favorites.get(*i)?.favicon.clone(),
            Kind::Channel(i) => self.tv.catalog()?.channels.get(*i)?.logo.clone()?,
            Kind::Fav(i) => self.data.tv_favorites.get(*i)?.logo.clone(),
            Kind::File(f) => self.data.history_for(f)?.logo.clone()?,
            Kind::Go(LibLoc::Tv(Some((code, _)))) => crate::flags::for_language(code)?,
            Kind::Go(LibLoc::Radio(Some((name, _)))) => crate::flags::for_language(self.radio.iso_of(name)?)?,
            _ => return None,
        };
        (u.starts_with("http://") || u.starts_with("https://")).then_some(u)
    }

    /// Draws a station logo fitted into `r`; false while it is not loaded.
    fn paint_logo(&mut self, ui: &Ui, url: &str, r: Rect) -> bool {
        let Some(img) = self.logos.get(url, || url.to_string()) else { return false };
        let s = img.size;
        let k = (r.width() / s.x).min(r.height() / s.y);
        let fit = Rect::from_center_size(r.center(), s * k);
        egui::Image::new((img.id, fit.size())).tint(Color32::from_white_alpha((img.alpha * 255.0) as u8)).corner_radius(CornerRadius::same(4)).paint_at(ui, fit);
        true
    }

    /// Whether a channel / station entry is starred.
    fn is_favorite(&self, kind: &Kind) -> bool {
        match kind {
            Kind::Fav(_) | Kind::RadioFav(_) => true,
            Kind::Radio(s) => self.data.radio_favorites.iter().any(|f| f.uuid == s.stationuuid),
            _ => self.tv_item(kind).is_some_and(|(url, _)| self.data.tv_favorites.iter().any(|f| f.url == url)),
        }
    }

    /// Stars or unstars a channel / favorite entry.
    fn toggle_tv_favorite(&mut self, kind: &Kind) {
        let radio = match kind {
            Kind::Radio(s) => Some(crate::config::RadioFavorite {
                name: s.title(),
                url: s.stream_url().to_string(),
                uuid: s.stationuuid.clone(),
                sub: s.subtitle(),
                favicon: s.favicon.clone(),
            }),
            Kind::RadioFav(i) => self.data.radio_favorites.get(*i).cloned(),
            _ => None,
        };
        if let Some(fav) = radio {
            if let Some(pos) = self.data.radio_favorites.iter().position(|f| f.uuid == fav.uuid && f.url == fav.url) {
                self.data.radio_favorites.remove(pos);
                self.osd(tf!("Aus Radio-Favoriten entfernt: {}", fav.name));
            } else {
                self.osd(tf!("Zu Radio-Favoriten hinzugefügt: {}", fav.name));
                self.data.radio_favorites.push(fav);
            }
            self.mark_dirty();
            return;
        }
        let fav = match kind {
            Kind::Channel(i) => match self.tv.catalog() {
                Some(c) if *i < c.channels.len() => c.favorite(&c.channels[*i]),
                _ => return,
            },
            Kind::Fav(i) => match self.data.tv_favorites.get(*i) {
                Some(f) => f.clone(),
                None => return,
            },
            _ => return,
        };
        if let Some(pos) = self.data.tv_favorites.iter().position(|f| f.url == fav.url) {
            self.data.tv_favorites.remove(pos);
            self.osd(tf!("Aus TV-Favoriten entfernt: {}", fav.title));
        } else {
            self.osd(tf!("Zu TV-Favoriten hinzugefügt: {}", fav.title));
            self.data.tv_favorites.push(fav);
        }
        self.mark_dirty();
    }

    /// Plays channels as a queue (the ones around `start`), each with its
    /// title, user agent and referrer.
    fn play_tv(&mut self, entries: &[Entry], idx: usize) {
        // Only the entries around the tapped one: thousands of loadfile
        // calls would stall, and a window is plenty to zap through.
        let lo = idx.saturating_sub(150);
        let hi = (idx + 350).min(entries.len());
        let mut items = vec![];
        let mut start = 0;
        for (n, e) in entries[lo..hi].iter().enumerate() {
            if let Some(it) = self.tv_item(&e.kind) {
                if lo + n == idx {
                    start = items.len();
                }
                items.push(it);
            }
        }
        if let Some(Kind::Radio(s)) = entries.get(idx).map(|e| &e.kind) {
            self.radio.clicked(&s.stationuuid);
        }
        for e in &entries[lo..hi] {
            self.remember_stream(&e.kind);
        }
        let Some(p) = &self.player else { return };
        if items.is_empty() {
            return;
        }
        for (n, (url, opts)) in items.iter().enumerate() {
            let mode = if n == 0 { "replace" } else { "append" };
            let _ = p.mpv.command(&["loadfile", url, mode, "-1", opts]);
        }
        let _ = p.mpv.set_i64("playlist-pos", start as i64);
        p.set_pause(false);
        // Radio plays on in the list (mini player); TV opens the player.
        if !matches!(entries.get(idx).map(|e| &e.kind), Some(Kind::Radio(_) | Kind::RadioFav(_))) {
            self.view = View::Player;
        }
    }

    fn entry_files(&self, e: &Entry) -> Vec<String> {
        match &e.kind {
            Kind::File(f) => vec![f.clone()],
            Kind::Folder(d) => media::list_dir(d, false).1.into_iter().map(|p| p.to_string_lossy().to_string()).collect(),
            Kind::Stream(i) => self.data.streams.get(*i).map(|s| vec![s.url.clone()]).unwrap_or_default(),
            Kind::Playlist(i) => self.data.playlists.get(*i).map(|p| p.items.clone()).unwrap_or_default(),
            _ => vec![],
        }
    }

    fn activate(&mut self, entries: &[Entry], idx: usize, stay_in_view: bool) {
        let Some(e) = entries.get(idx).cloned() else { return };
        let section = self.lib.loc.section();
        match e.kind {
            Kind::Folder(d) => {
                let loc = match section {
                    1 => LibLoc::Videos(Some(d)),
                    2 => LibLoc::MusicFolder(Some(d)),
                    _ => LibLoc::Folder(d),
                };
                self.lib.go(loc);
            }
            Kind::Go(l) => self.lib.go(l),
            Kind::NewStream => {
                self.dlg.url.clear();
                self.dlg.url_name.clear();
                self.dlg.url_save = true;
                self.dialog = Some(Dialog::Network);
            }
            Kind::NewPlaylist => {
                self.dlg.playlist_name.clear();
                self.dialog = Some(Dialog::NewPlaylist(vec![]));
            }
            Kind::Playlist(i) => self.lib.go(LibLoc::Playlists(Some(i))),
            Kind::Stream(i) => {
                if let Some(s) = self.data.streams.get(i).cloned() {
                    if is_iptv_org(&s.url) {
                        self.lib.go(LibLoc::Tv(None));
                    } else if crate::radio::is_radio_browser(&s.url) {
                        self.lib.go(LibLoc::Radio(None));
                    } else {
                        self.play_list(&[s.url], 0);
                    }
                }
            }
            Kind::Channel(_) | Kind::Fav(_) | Kind::Radio(_) | Kind::RadioFav(_) => self.play_tv(entries, idx),
            Kind::TvRetry if matches!(self.lib.loc, LibLoc::Radio(_)) => self.radio.reload(),
            Kind::TvRetry => self.tv.reload(),
            Kind::Info => {}
            Kind::File(f) => {
                let in_recent = self.lib.loc == LibLoc::Recent;
                if self.data.settings.library_add_folder_to_queue && !in_recent {
                    let files: Vec<String> = entries
                        .iter()
                        .filter_map(|x| match &x.kind {
                            Kind::File(f) if !media::is_playlist(f) => Some(f.clone()),
                            _ => None,
                        })
                        .collect();
                    let start = files.iter().position(|x| *x == f).unwrap_or(0);
                    if files.is_empty() {
                        self.play_list(&[f], 0);
                    } else {
                        self.play_list(&files, start);
                    }
                } else {
                    self.play_list(&[f], 0);
                }
                if stay_in_view {
                    self.view = View::Player;
                }
            }
        }
    }

    fn entry_menu(&self, ui: &mut Ui, e: &Entry, idx: usize, out: &mut Option<Click>) {
        let t = self.t;
        if matches!(e.kind, Kind::Channel(_) | Kind::Fav(_) | Kind::Radio(_) | Kind::RadioFav(_)) {
            let starred = self.is_favorite(&e.kind);
            let radio = matches!(e.kind, Kind::Radio(_) | Kind::RadioFav(_));
            let fav_label = match (radio, starred) {
                (false, false) => t!("Zu TV-Favoriten hinzufügen"),
                (false, true) => t!("Aus TV-Favoriten entfernen"),
                (true, false) => t!("Zu Radio-Favoriten hinzufügen"),
                (true, true) => t!("Aus Radio-Favoriten entfernen"),
            };
            let items = [
                (ph::PLAY, t!("Abspielen"), MenuAct::Play),
                (ph::ARROW_BEND_DOWN_RIGHT, t!("Als Nächstes abspielen"), MenuAct::PlayNext),
                (ph::QUEUE, t!("Zur Warteschlange hinzufügen"), MenuAct::Enqueue),
                (ph::STAR, fav_label, MenuAct::Favorite),
            ];
            for (icon, label, act) in items {
                if widgets::menu_item(ui, &t, icon, label, "", false).clicked() {
                    *out = Some(Click::Menu(act, idx));
                    ui.close();
                }
            }
            if self.tv_guide_target(&e.kind).is_some() && widgets::menu_item(ui, &t, ph::CALENDAR_BLANK, t!("Programm"), "", false).clicked() {
                *out = Some(Click::Menu(MenuAct::Guide, idx));
                ui.close();
            }
            return;
        }
        let playable = matches!(e.kind, Kind::File(_) | Kind::Folder(_) | Kind::Stream(_) | Kind::Playlist(_));
        if playable {
            if widgets::menu_item(ui, &t, ph::PLAY, t!("Abspielen"), "", false).clicked() {
                *out = Some(Click::Menu(MenuAct::Play, idx));
                ui.close();
            }
            if widgets::menu_item(ui, &t, ph::ARROW_BEND_DOWN_RIGHT, t!("Als Nächstes abspielen"), "", false).clicked() {
                *out = Some(Click::Menu(MenuAct::PlayNext, idx));
                ui.close();
            }
            if widgets::menu_item(ui, &t, ph::QUEUE, t!("Zur Warteschlange hinzufügen"), "", false).clicked() {
                *out = Some(Click::Menu(MenuAct::Enqueue, idx));
                ui.close();
            }
            if widgets::menu_item(ui, &t, ph::PLAYLIST, t!("Zu Playlist hinzufügen…"), "", false).clicked() {
                *out = Some(Click::Menu(MenuAct::AddToPlaylist, idx));
                ui.close();
            }
        }
        if matches!(e.kind, Kind::Folder(_) | Kind::Stream(_) | Kind::Playlist(_)) && widgets::menu_item(ui, &t, ph::PUSH_PIN, t!("Anheften"), "", false).clicked() {
            *out = Some(Click::Menu(MenuAct::Pin, idx));
            ui.close();
        }
        // Live channels have nothing to watch to the end.
        if let Kind::File(f) = &e.kind
            && !self.data.history_for(f).is_some_and(|h| h.live)
        {
            let watched = self.data.history_for(f).map(|h| h.finished).unwrap_or(false);
            if widgets::menu_item(ui, &t, if watched { ph::EYE_SLASH } else { ph::EYE }, if watched { t!("Als ungesehen markieren") } else { t!("Als gesehen markieren") }, "", false).clicked() {
                *out = Some(Click::Menu(MenuAct::Watched(!watched), idx));
                ui.close();
            }
        }
        // Phones have no Explorer to show a file in.
        if !crate::platform::MOBILE && (matches!(&e.kind, Kind::File(f) if !media::is_url(f)) || matches!(e.kind, Kind::Folder(_))) {
            if widgets::menu_item(ui, &t, ph::FOLDER_OPEN, crate::platform::show_in_folder_label(), "", false).clicked() {
                *out = Some(Click::Menu(MenuAct::Explorer, idx));
                ui.close();
            }
        }
        let removable = matches!(e.kind, Kind::Stream(_) | Kind::Playlist(_))
            || matches!(self.lib.loc, LibLoc::Playlists(Some(_)) | LibLoc::Recent)
            || (matches!(self.lib.loc, LibLoc::Videos(None) | LibLoc::MusicFolder(None)) && matches!(e.kind, Kind::Folder(_)));
        if removable && widgets::menu_item(ui, &t, ph::TRASH, t!("Entfernen"), "", false).clicked() {
            *out = Some(Click::Menu(MenuAct::Remove, idx));
            ui.close();
        }
    }

    fn tv_menu(&mut self, act: MenuAct, entries: &[Entry], idx: usize) {
        let kind = entries[idx].kind.clone();
        match act {
            MenuAct::Play => self.play_tv(entries, idx),
            MenuAct::PlayNext | MenuAct::Enqueue => {
                self.remember_stream(&kind);
                let Some((url, opts)) = self.tv_item(&kind) else { return };
                let Some(p) = &self.player else { return };
                let next = matches!(act, MenuAct::PlayNext);
                let mode = if next { "insert-next" } else if p.st.has_media() { "append" } else { "append-play" };
                let _ = p.mpv.command(&["loadfile", &url, mode, "-1", &opts]);
                let title = entries[idx].title.clone();
                self.osd(if next { t!("Wird als Nächstes abgespielt").to_string() } else { tf!("Zur Warteschlange hinzugefügt: {}", title) });
            }
            MenuAct::Favorite => self.toggle_tv_favorite(&kind),
            MenuAct::Guide => {
                if let Some((title, country, keys)) = self.tv_guide_target(&kind) {
                    self.epg.want(&country);
                    self.dialog = Some(Dialog::TvGuide { title, country, keys });
                }
            }
            _ => {}
        }
    }

    fn handle_click(&mut self, click: Click, entries: &[Entry], stay: bool) {
        match click {
            Click::Open(i) => self.activate(entries, i, stay),
            Click::PlayAll => {
                let files: Vec<String> = entries.iter().filter_map(|e| if let Kind::File(f) = &e.kind { Some(f.clone()) } else { None }).collect();
                if !files.is_empty() {
                    self.play_list(&files, 0);
                }
            }
            Click::Menu(act, i) => {
                let Some(e) = entries.get(i).cloned() else { return };
                if matches!(e.kind, Kind::Channel(_) | Kind::Fav(_) | Kind::Radio(_) | Kind::RadioFav(_)) {
                    self.tv_menu(act, entries, i);
                    return;
                }
                let files = self.entry_files(&e);
                match act {
                    MenuAct::Play => {
                        if !files.is_empty() {
                            self.play_list(&files, 0);
                        }
                    }
                    MenuAct::PlayNext => self.play_next(&files),
                    MenuAct::Enqueue => self.enqueue(&files),
                    MenuAct::AddToPlaylist => self.dialog = Some(Dialog::AddToPlaylist(files)),
                    MenuAct::Pin => {
                        let target = match &e.kind {
                            Kind::Folder(d) => PinTarget::Folder(d.clone()),
                            Kind::Stream(i) => PinTarget::Stream(self.data.streams[*i].url.clone()),
                            Kind::Playlist(i) => PinTarget::Playlist(self.data.playlists[*i].name.clone()),
                            _ => return,
                        };
                        if !self.data.pins.iter().any(|p| p.target == target) {
                            self.data.pins.push(Pin { name: e.title.clone(), target });
                            self.mark_dirty();
                        }
                    }
                    MenuAct::Watched(w) => {
                        if let Kind::File(f) = &e.kind {
                            self.mark_watched(f, w);
                        }
                    }
                    MenuAct::Explorer => match &e.kind {
                        Kind::File(f) => show_in_explorer(f),
                        Kind::Folder(d) => {
                            let _ = std::process::Command::new("explorer").arg(d).spawn();
                        }
                        _ => {}
                    },
                    MenuAct::Favorite | MenuAct::Guide => {}
                    MenuAct::Remove => match (&e.kind, self.lib.loc.clone()) {
                        (Kind::Stream(i), _) => {
                            self.data.streams.remove(*i);
                            self.mark_dirty();
                        }
                        (Kind::Playlist(i), _) => {
                            self.data.playlists.remove(*i);
                            self.mark_dirty();
                        }
                        (Kind::File(f), LibLoc::Playlists(Some(pi))) => {
                            if let Some(pl) = self.data.playlists.get_mut(pi) {
                                pl.items.retain(|x| x != f);
                            }
                            self.mark_dirty();
                        }
                        (Kind::File(f), LibLoc::Recent) => {
                            self.data.history.retain(|h| h.path != *f);
                            self.mark_dirty();
                        }
                        (Kind::Folder(d), LibLoc::Videos(None)) => {
                            self.data.settings.video_folders.retain(|x| x != d);
                            self.mark_dirty();
                        }
                        (Kind::Folder(d), LibLoc::MusicFolder(None)) => {
                            self.data.settings.music_folders.retain(|x| x != d);
                            self.mark_dirty();
                        }
                        _ => {}
                    },
                }
            }
        }
    }

    // ------------------------------------------------------------ toolbar

    fn lib_toolbar(&mut self, ui: &mut Ui, count: usize, compact: bool, entries: &[Entry], click: &mut Option<Click>) {
        let t = self.t;
        ui.horizontal(|ui| {
            ui.set_min_height(30.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.label(RichText::new(tf!("{count} Elemente")).size(12.5).color(t.text_dim));
            if matches!(self.lib.loc, LibLoc::Playlists(Some(_)) | LibLoc::Music(MusicView::Album(_)) | LibLoc::Music(MusicView::Artist(_)) | LibLoc::Music(MusicView::Genre(_)))
                || entries.iter().filter(|e| matches!(e.kind, Kind::File(_))).count() > 1
            {
                ui.add_space(6.0);
                if ui.add(egui::Button::new(RichText::new(tf!("{}  Alle abspielen", ph::PLAY)).size(12.0)).small()).clicked() {
                    *click = Some(Click::PlayAll);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let more = icon_button(ui, &t, ph::DOTS_THREE, "", false);
                egui::Popup::menu(&more).width(240.0).show(|ui| {
                    if widgets::menu_item(ui, &t, ph::FILE_PLUS, t!("Datei öffnen…"), "", false).clicked() {
                        ui.close();
                        self.dialog_open_files();
                    }
                    if widgets::menu_item(ui, &t, ph::FOLDER_PLUS, t!("Ordner zur Bibliothek hinzufügen…"), "", false).clicked() {
                        ui.close();
                        self.add_library_folder();
                    }
                    if widgets::menu_item(ui, &t, ph::BROADCAST, t!("Netzwerkstream hinzufügen…"), "", false).clicked() {
                        ui.close();
                        self.dialog = Some(Dialog::Network);
                    }
                    if widgets::menu_item(ui, &t, ph::PLAYLIST, t!("Playlist erstellen…"), "", false).clicked() {
                        ui.close();
                        self.dlg.playlist_name.clear();
                        self.dialog = Some(Dialog::NewPlaylist(vec![]));
                    }
                    if widgets::menu_item(ui, &t, ph::ARROWS_CLOCKWISE, t!("Aktualisieren"), "", false).clicked() {
                        ui.close();
                        if matches!(self.lib.loc, LibLoc::Radio(_)) {
                            self.radio.reload();
                        } else if matches!(self.lib.loc, LibLoc::Tv(_)) {
                            self.tv.reload();
                            self.epg.reload();
                        } else {
                            self.lib.refresh();
                            let ctx = ui.ctx().clone();
                            self.lib.start_music_scan(&self.data, &ctx);
                        }
                    }
                    if self.lib.loc == LibLoc::Recent && widgets::menu_item(ui, &t, ph::TRASH, t!("Verlauf leeren"), "", false).clicked() {
                        ui.close();
                        self.data.history.clear();
                        self.mark_dirty();
                    }
                });
                if icon_button(ui, &t, ph::MAGNIFYING_GLASS, t!("Suchen"), self.lib.search_open).clicked() {
                    self.lib.search_open = !self.lib.search_open;
                    if !self.lib.search_open {
                        self.lib.search.clear();
                    }
                }
                let side = compact && !crate::mobile::touch();
                let view = if side { self.lib.compact_view } else { self.lib.view };
                let vicon = if view == ViewMode::Grid { ph::SQUARES_FOUR } else { ph::LIST_BULLETS };
                if icon_button(ui, &t, vicon, t!("Ansicht: Raster / Liste"), false).clicked() {
                    let next = if view == ViewMode::Grid { ViewMode::List } else { ViewMode::Grid };
                    if side {
                        self.lib.compact_view = next;
                    } else {
                        self.lib.view = next;
                    }
                }
                // The TV list has its own order (channel count, then name).
                if !matches!(self.lib.loc, LibLoc::Tv(_) | LibLoc::Radio(_)) {
                    let sresp = icon_button(ui, &t, ph::ARROWS_DOWN_UP, t!("Sortieren"), false);
                    egui::Popup::menu(&sresp).width(220.0).show(|ui| {
                        for m in SortMode::ALL {
                            if widgets::menu_item(ui, &t, "", m.label(), "", self.lib.sort == m).clicked() {
                                self.lib.sort = m;
                                ui.close();
                            }
                        }
                    });
                }
                let _ = compact;
            });
        });
        if self.lib.search_open {
            ui.add_space(6.0);
            let id = egui::Id::new("library-search-field");
            let w = ui.available_width().min(560.0);
            let out = widgets::search_field(ui, &t, id, &mut self.lib.search, t!("In dieser Ansicht suchen…"), w);
            if self.lib.search.is_empty() && !out.cleared && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                // Esc on an empty field closes the search row.
                self.lib.search_open = false;
            } else if !out.focused && self.lib.search.is_empty() && !out.cleared {
                ui.memory_mut(|m| m.request_focus(id));
            }
        }
    }

    pub fn add_library_folder(&mut self) {
        let music = matches!(self.lib.loc, LibLoc::Music(_) | LibLoc::MusicFolder(_));
        self.pick(crate::pick::Pick::LibraryFolder { music, show: true });
    }

    fn header_nav(&mut self, ui: &mut Ui, compact: bool) {
        let t = self.t;
        ui.horizontal(|ui| {
            ui.set_min_height(34.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            let can_back = !self.lib.back.is_empty();
            let can_fwd = !self.lib.fwd.is_empty();
            if ui.add_enabled_ui(can_back, |ui| icon_button(ui, &t, ph::ARROW_LEFT, t!("Zurück"), false)).inner.clicked() {
                self.lib.go_back();
            }
            if ui.add_enabled_ui(can_fwd, |ui| icon_button(ui, &t, ph::ARROW_RIGHT, t!("Vorwärts"), false)).inner.clicked() {
                self.lib.go_fwd();
            }
            ui.add_space(4.0);
            let crumbs = self.lib.crumbs(&self.data);
            let n = crumbs.len();
            let start = if compact && n > 2 { n - 2 } else { 0 };
            for (i, (label, loc)) in crumbs.into_iter().enumerate().skip(start) {
                let last = i == n - 1;
                let rt = RichText::new(&label).font(if last { theme::semibold(if compact { 15.0 } else { 17.0 }) } else { FontId::proportional(if compact { 13.5 } else { 15.0 }) }).color(if last { t.text } else { t.text_dim });
                if ui.add(egui::Button::new(rt).frame(false)).on_hover_cursor(CursorIcon::PointingHand).clicked() && !last {
                    self.lib.go(loc);
                }
                if !last {
                    ui.label(RichText::new(ph::CARET_RIGHT).size(11.0).color(t.text_faint));
                }
            }
            if compact {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, &t, ph::ARROW_SQUARE_OUT, t!("Bibliothek im Vollbild"), false).clicked() {
                        self.view = View::Library;
                        self.lib.compact_open = false;
                    }
                });
            }
        });
    }

    // ------------------------------------------------------- compact panel

    pub fn library_compact(&mut self, ui: &mut Ui) {
        // The panel is exactly as wide as it was dragged: the content is laid
        // out in a child that does not grow the panel (a long title would
        // otherwise push the width back out after every drag).
        let full = ui.max_rect();
        ui.allocate_rect(full, Sense::hover());
        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(full).layout(*ui.layout()));
        inner.set_clip_rect(full.intersect(ui.clip_rect()));
        self.library_compact_inner(&mut inner);
    }

    fn library_compact_inner(&mut self, ui: &mut Ui) {
        let t = self.t;
        let full = ui.max_rect();
        let rail_w = 48.0;
        let rail = Rect::from_min_max(full.min, pos2(full.left() + rail_w, full.bottom()));
        ui.painter().rect_filled(rail, CornerRadius::ZERO, t.bg);
        ui.painter().line_segment([rail.right_top(), rail.right_bottom()], Stroke::new(1.0, t.border));
        let section = self.lib.loc.section();
        ui.scope_builder(egui::UiBuilder::new().max_rect(rail.shrink2(vec2(9.0, 8.0))), |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for (sec, icon, tip, loc) in [
                (0u8, ph::HOUSE, "Start", LibLoc::Home),
                (1, ph::FILM_STRIP, "Videos", LibLoc::Videos(None)),
                (2, ph::MUSIC_NOTES, t!("Musik"), LibLoc::Music(MusicView::Songs)),
                (7, ph::TELEVISION, t!("TV-Sender"), LibLoc::Tv(None)),
                (8, ph::RADIO, t!("Radiosender"), LibLoc::Radio(None)),
                (3, ph::BROADCAST, t!("Netzwerk"), LibLoc::Network),
                (4, ph::PLAYLIST, "Playlists", LibLoc::Playlists(None)),
                (5, ph::CLOCK_COUNTER_CLOCKWISE, t!("Zuletzt gespielt"), LibLoc::Recent),
                (6, ph::TELEVISION_SIMPLE, self.plex.backend().name(), LibLoc::Plex(crate::library::PlexLoc::Home)),
            ] {
                if !self.data.settings.nav_shown(nav_id(sec)) {
                    continue;
                }
                if icon_button(ui, &t, icon, tip, section == sec).clicked() {
                    self.lib.go(loc);
                }
            }
            let pins = if self.data.settings.nav_shown("pins") { self.data.pins.clone() } else { Vec::new() };
            if !pins.is_empty() {
                widgets::separator(ui, &t);
            }
            for p in pins {
                let icon = match p.target {
                    PinTarget::Folder(_) => ph::FOLDER_SIMPLE,
                    PinTarget::Playlist(_) => ph::PLAYLIST,
                    PinTarget::Stream(_) => ph::BROADCAST,
                    PinTarget::File(_) => ph::FILE_VIDEO,
                };
                if icon_button(ui, &t, icon, &p.name, false).clicked() {
                    self.open_pin(&p);
                }
            }
            if icon_button(ui, &t, ph::PLUS, t!("Ordner hinzufügen"), false).clicked() {
                self.add_library_folder();
            }
        });
        let content = Rect::from_min_max(pos2(rail.right() + 10.0, full.top() + 4.0), pos2(full.right() - 8.0, full.bottom()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            if self.lib.loc == LibLoc::Home {
                self.lib.loc = LibLoc::Videos(self.current_folder());
                if let LibLoc::Videos(None) = self.lib.loc {
                    self.lib.loc = LibLoc::Recent;
                }
            }
            if let LibLoc::Plex(p) = self.lib.loc.clone() {
                if self.plex.ready() {
                    self.header_nav(ui, true);
                }
                self.plex_page(ui, p, true);
                return;
            }
            self.header_nav(ui, true);
            let entries = self.entries();
            let mut click = None;
            self.lib_toolbar(ui, entries.iter().filter(|e| !matches!(e.kind, Kind::NewPlaylist | Kind::NewStream)).count(), true, &entries, &mut click);
            widgets::separator(ui, &t);
            self.entries_view(ui, &entries, true, &mut click);
            if let Some(c) = click {
                self.handle_click(c, &entries, true);
            }
        });
    }

    fn open_pin(&mut self, p: &Pin) {
        match &p.target {
            PinTarget::Folder(d) => {
                let in_videos = self.data.settings.video_folders.iter().any(|r| d.starts_with(r));
                self.lib.go(if in_videos { LibLoc::Videos(Some(d.clone())) } else { LibLoc::Folder(d.clone()) });
            }
            PinTarget::Playlist(name) => {
                if let Some(i) = self.data.playlists.iter().position(|x| x.name == *name) {
                    self.lib.go(LibLoc::Playlists(Some(i)));
                }
            }
            PinTarget::Stream(url) if is_iptv_org(url) => self.lib.go(LibLoc::Tv(None)),
            PinTarget::Stream(url) if crate::radio::is_radio_browser(url) => self.lib.go(LibLoc::Radio(None)),
            PinTarget::Stream(url) | PinTarget::File(url) => self.play_list(&[url.clone()], 0),
        }
    }

    // ------------------------------------------------------- entries view

    fn entries_view(&mut self, ui: &mut Ui, entries: &[Entry], compact: bool, click: &mut Option<Click>) {
        let t = self.t;
        let ctx = ui.ctx().clone();
        if entries.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                let msg = match &self.lib.loc {
                    LibLoc::Music(_) if self.lib.scanning => t!("Musikbibliothek wird durchsucht…"),
                    LibLoc::Music(_) => t!("Keine Musik gefunden. Füge über „…“ einen Musikordner hinzu."),
                    LibLoc::Recent => t!("Noch nichts abgespielt."),
                    _ if !self.lib.search.is_empty() => t!("Keine Treffer."),
                    _ => t!("Dieser Ordner enthält keine Medien."),
                };
                ui.label(RichText::new(msg).color(t.text_dim));
            });
            return;
        }
        let current = self.current_path();
        let music_tags = matches!(self.lib.loc, LibLoc::Music(_));
        // `compact` is the narrow desktop sidebar (its own view setting,
        // small tiles) or a touch screen, where tiles are smaller too.
        let touch = crate::mobile::touch();
        let side = compact && !touch;
        let grid = if side { self.lib.compact_view } else { self.lib.view } == ViewMode::Grid;
        if matches!(self.lib.loc, LibLoc::Music(MusicView::Albums) | LibLoc::Music(MusicView::Artists) | LibLoc::Music(MusicView::Genres)) {
            // Make sure all tags get read.
            let files: Vec<String> = self.lib.music_files.iter().map(|p| p.to_string_lossy().to_string()).collect();
            let mut pending = 0;
            for f in files.iter() {
                if self.tools.request_probe(f).is_none() {
                    pending += 1;
                }
            }
            if pending > 0 {
                ui.label(RichText::new(tf!("Metadaten werden gelesen… ({} von {})", files.len() - pending, files.len())).size(12.0).color(t.text_faint));
            }
        }
        // On touch a row that senses drags would swallow the swipe and the
        // list could not scroll; tap and long-press still work with clicks.
        let row_sense = if crate::mobile::touch() { Sense::click() } else { Sense::click_and_drag() };
        if grid {
            let card_w = if touch { 150.0 } else if side { 128.0 } else { 196.0 };
            let gap = 14.0;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let w = ui.available_width();
                let cols = ((w + gap) / (card_w + gap)).floor().max(1.0) as usize;
                let cw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
                let thumb_h = cw * 9.0 / 16.0;
                for (row_i, row) in entries.chunks(cols).enumerate() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for (ci, e) in row.iter().enumerate() {
                            let idx = row_i * cols + ci;
                            let (rect, resp) = ui.allocate_exact_size(vec2(cw, thumb_h + 46.0), row_sense);
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            let img = Rect::from_min_size(rect.min, vec2(cw, thumb_h));
                            let hovered = resp.hovered();
                            ui.painter().rect_filled(img, CornerRadius::same(theme::R_LG), if hovered { t.hover } else { t.surface });
                            let mut drew = false;
                            if let Kind::File(f) = &e.kind {
                                if let Some(tex) = self.thumb_for(&ctx, f) {
                                    egui::Image::new((tex, img.size())).corner_radius(CornerRadius::same(theme::R_LG)).paint_at(ui, img);
                                    drew = true;
                                }
                                if let Some((pr, fin)) = self.progress_of(f) {
                                    if fin {
                                        let b = Rect::from_min_size(img.right_top() + vec2(-28.0, 6.0), vec2(22.0, 22.0));
                                        ui.painter().rect_filled(b, CornerRadius::same(11), Color32::from_black_alpha(170));
                                        ui.painter().text(b.center(), Align2::CENTER_CENTER, ph::CHECK, theme::icon(13.0), t.accent);
                                    } else if pr > 0.01 {
                                        let pr_r = Rect::from_min_max(pos2(img.left(), img.bottom() - 4.0), img.max);
                                        widgets::progress_line(ui.painter(), pr_r, pr, &t);
                                    }
                                }
                            }
                            if !drew {
                                if let Some(logo) = self.entry_logo(&e.kind) {
                                    drew = self.paint_logo(ui, &logo, Rect::from_center_size(img.center(), vec2(thumb_h * 0.7, thumb_h * 0.7)));
                                }
                            }
                            if !drew {
                                let ic = if matches!(e.kind, Kind::Folder(_)) { ph::FOLDER_SIMPLE } else { e.icon };
                                ui.painter().text(img.center(), Align2::CENTER_CENTER, ic, theme::icon(thumb_h * 0.32), if matches!(e.kind, Kind::NewStream | Kind::NewPlaylist) { t.accent } else { t.text_faint });
                            }
                            if hovered {
                                ui.painter().rect_stroke(img, CornerRadius::same(theme::R_LG), Stroke::new(1.5, t.accent), StrokeKind::Inside);
                                if matches!(e.kind, Kind::File(_)) {
                                    let c = Rect::from_center_size(img.center(), vec2(42.0, 30.0));
                                    ui.painter().rect_filled(c, CornerRadius::same(4), t.accent);
                                    ui.painter().text(c.center(), Align2::CENTER_CENTER, ph::PLAY, theme::fill_icon(15.0), t.on_accent);
                                }
                            }
                            let is_cur = matches!(&e.kind, Kind::File(f) if *f == current);
                            widgets::painter_text_trunc(ui.painter(), pos2(rect.left() + 2.0, img.bottom() + 6.0), &e.title, FontId::proportional(13.0), if is_cur { t.accent } else { t.text }, cw - 4.0);
                            widgets::painter_text_trunc(ui.painter(), pos2(rect.left() + 2.0, img.bottom() + 25.0), &e.sub, FontId::proportional(11.5), t.text_faint, cw - 4.0);
                            self.item_interactions(ui, &resp, e, idx, click);
                        }
                    });
                    ui.add_space(gap);
                }
            });
        } else {
            let row_h = if music_tags || compact { 44.0 } else { 52.0 };
            // With video previews in the list, rows without a picture get a
            // box of the same size, so all titles start in one line.
            let thumb_column = entries.iter().any(|e| matches!(&e.kind, Kind::File(f) if media::is_video(f)));
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, entries.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for idx in range {
                    let e = &entries[idx];
                    let w = ui.available_width();
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, row_h), row_sense);
                    let is_cur = matches!(&e.kind, Kind::File(f) if *f == current);
                    if is_cur {
                        ui.painter().rect_filled(rect, CornerRadius::same(theme::R), t.accent_soft);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(theme::R), t.hover);
                    }
                    let th_w = (row_h - 10.0) * 16.0 / 9.0;
                    let mut x = rect.left() + 6.0;
                    let mut drew = false;
                    if let Kind::File(f) = &e.kind {
                        if !compact || media::is_video(f) {
                            if let Some(tex) = self.thumb_for(&ctx, f) {
                                let r = Rect::from_min_size(pos2(x, rect.top() + 5.0), vec2(th_w, row_h - 10.0));
                                egui::Image::new((tex, r.size())).corner_radius(CornerRadius::same(3)).paint_at(ui, r);
                                if let Some((pr, fin)) = self.progress_of(f) {
                                    if !fin && pr > 0.01 {
                                        widgets::progress_line(ui.painter(), Rect::from_min_max(pos2(r.left(), r.bottom() - 3.0), r.max), pr, &t);
                                    }
                                }
                                x += th_w + 10.0;
                                drew = true;
                            }
                        }
                    }
                    if !drew {
                        if let Some(logo) = self.entry_logo(&e.kind) {
                            let side = (row_h - 16.0).min(30.0);
                            if self.paint_logo(ui, &logo, Rect::from_center_size(pos2(x + 12.0, rect.center().y), vec2(side, side))) {
                                x += 32.0;
                                drew = true;
                            }
                        }
                    }
                    if !drew {
                        let col = if matches!(e.kind, Kind::NewStream | Kind::NewPlaylist) || is_cur { t.accent } else { t.text_dim };
                        if thumb_column {
                            let r = Rect::from_min_size(pos2(x, rect.top() + 5.0), vec2(th_w, row_h - 10.0));
                            ui.painter().rect_filled(r, CornerRadius::same(3), t.surface);
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, e.icon, theme::icon(18.0), col);
                            if let Kind::File(f) = &e.kind {
                                if let Some((pr, fin)) = self.progress_of(f) {
                                    if !fin && pr > 0.01 {
                                        widgets::progress_line(ui.painter(), Rect::from_min_max(pos2(r.left(), r.bottom() - 3.0), r.max), pr, &t);
                                    }
                                }
                            }
                            x += th_w + 10.0;
                        } else {
                            ui.painter().text(pos2(x + 12.0, rect.center().y), Align2::CENTER_CENTER, e.icon, theme::icon(18.0), col);
                            x += 32.0;
                        }
                    }
                    let watched = matches!(&e.kind, Kind::File(f) if self.progress_of(f).map(|p| p.1).unwrap_or(false));
                    let title_col = if is_cur { t.accent } else if watched { t.text_dim } else { t.text };
                    let right_w = if music_tags && !compact { w * 0.45 } else { 60.0 };
                    let avail = rect.right() - x - right_w;
                    if e.sub.is_empty() || music_tags {
                        widgets::painter_text_trunc(ui.painter(), pos2(x, rect.center().y - 9.0), &e.title, FontId::proportional(13.5), title_col, avail);
                    } else {
                        widgets::painter_text_trunc(ui.painter(), pos2(x, rect.center().y - 17.0), &e.title, FontId::proportional(13.5), title_col, avail);
                        widgets::painter_text_trunc(ui.painter(), pos2(x, rect.center().y + 2.0), &e.sub, FontId::proportional(11.5), t.text_faint, avail);
                    }
                    if music_tags {
                        if let Kind::File(f) = &e.kind {
                            let p = self.tools.request_probe(f).cloned();
                            if !compact {
                                let colw = right_w / 3.0;
                                let artist = p.as_ref().and_then(|p| p.artist.clone()).unwrap_or_default();
                                let album = p.as_ref().and_then(|p| p.album.clone()).unwrap_or_default();
                                widgets::painter_text_trunc(ui.painter(), pos2(rect.right() - right_w, rect.center().y - 9.0), &artist, FontId::proportional(12.5), t.text_dim, colw - 8.0);
                                widgets::painter_text_trunc(ui.painter(), pos2(rect.right() - right_w + colw, rect.center().y - 9.0), &album, FontId::proportional(12.5), t.text_dim, colw - 8.0);
                            }
                            let d = p.map(|p| media::fmt_time_short(p.duration)).unwrap_or_default();
                            ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, d, FontId::proportional(12.0), t.text_faint);
                        }
                    } else if let Kind::File(f) = &e.kind {
                        if watched {
                            ui.painter().text(pos2(rect.right() - 12.0, rect.center().y), Align2::CENTER_CENTER, ph::CHECK, theme::icon(14.0), t.accent);
                        } else if let Some(p) = self.tools.request_probe(f).filter(|_| e.sub.is_empty()) {
                            ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, media::fmt_time_short(p.duration), FontId::proportional(12.0), t.text_faint);
                        }
                    } else if matches!(e.kind, Kind::Folder(_) | Kind::Go(_) | Kind::Playlist(_)) {
                        ui.painter().text(pos2(rect.right() - 12.0, rect.center().y), Align2::CENTER_CENTER, ph::CARET_RIGHT, theme::icon(13.0), t.text_faint);
                    }
                    self.item_interactions(ui, &resp, e, idx, click);
                }
            });
        }
    }

    fn item_interactions(&mut self, _ui: &mut Ui, resp: &egui::Response, e: &Entry, idx: usize, click: &mut Option<Click>) {
        let resp = resp.clone().on_hover_cursor(CursorIcon::PointingHand);
        let single = matches!(e.kind, Kind::Folder(_) | Kind::Go(_) | Kind::Playlist(_) | Kind::NewStream | Kind::NewPlaylist | Kind::Info | Kind::TvRetry)
            || matches!(&e.kind, Kind::Stream(i) if self.data.streams.get(*i).is_some_and(|s| is_iptv_org(&s.url) || crate::radio::is_radio_browser(&s.url)));
        if (single && resp.clicked()) || (!single && (resp.double_clicked() || resp.clicked())) {
            *click = Some(Click::Open(idx));
        }
        if resp.drag_started() {
            let files = self.entry_files(e);
            if !files.is_empty() {
                resp.dnd_set_drag_payload(LibDrag(files));
            }
        }
        let mut out = None;
        // Entries without actions get no (empty) menu.
        if !matches!(e.kind, Kind::Go(_) | Kind::Info | Kind::TvRetry | Kind::NewStream | Kind::NewPlaylist) {
            resp.context_menu(|ui| {
                ui.set_min_width(240.0);
                self.entry_menu(ui, e, idx, &mut out);
            });
        }
        if out.is_some() {
            *click = out;
        }
    }

    // ------------------------------------------------------- full library

    pub fn library_full(&mut self, ui: &mut Ui) {
        let t = self.t;
        let full = ui.max_rect();
        let nav_w = 210.0;
        let nav = Rect::from_min_max(full.min, pos2(full.left() + nav_w, full.bottom()));
        ui.painter().rect_filled(nav, CornerRadius::ZERO, t.panel);
        ui.painter().line_segment([nav.right_top(), nav.right_bottom()], Stroke::new(1.0, t.border));
        let section = self.lib.loc.section();
        ui.scope_builder(egui::UiBuilder::new().max_rect(nav.shrink2(vec2(10.0, 12.0))), |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (sec, icon, label, loc) in [
                (0u8, ph::HOUSE, "Start", LibLoc::Home),
                (1, ph::FILM_STRIP, "Videos", LibLoc::Videos(None)),
                (2, ph::MUSIC_NOTES, t!("Musik"), LibLoc::Music(MusicView::Songs)),
                (7, ph::TELEVISION, t!("TV-Sender"), LibLoc::Tv(None)),
                (8, ph::RADIO, t!("Radiosender"), LibLoc::Radio(None)),
                (3, ph::BROADCAST, t!("Netzwerk"), LibLoc::Network),
                (4, ph::PLAYLIST, "Playlists", LibLoc::Playlists(None)),
                (5, ph::CLOCK_COUNTER_CLOCKWISE, t!("Zuletzt gespielt"), LibLoc::Recent),
            ] {
                if !self.data.settings.nav_shown(nav_id(sec)) {
                    continue;
                }
                if nav_item(ui, &t, icon, label, section == sec).clicked() {
                    self.lib.go(loc);
                }
            }
            if section == 2 && self.data.settings.nav_shown("music") {
                ui.add_space(4.0);
                for (label, loc) in [
                    (t!("Titel"), LibLoc::Music(MusicView::Songs)),
                    (t!("Alben"), LibLoc::Music(MusicView::Albums)),
                    (t!("Interpreten"), LibLoc::Music(MusicView::Artists)),
                    ("Genres", LibLoc::Music(MusicView::Genres)),
                    (t!("Ordner"), LibLoc::MusicFolder(None)),
                ] {
                    let sel = std::mem::discriminant(&self.lib.loc) == std::mem::discriminant(&loc)
                        && match (&self.lib.loc, &loc) {
                            (LibLoc::Music(a), LibLoc::Music(b)) => std::mem::discriminant(a) == std::mem::discriminant(b)
                                || matches!((a, b), (MusicView::Album(_), MusicView::Albums) | (MusicView::Artist(_), MusicView::Artists) | (MusicView::Genre(_), MusicView::Genres)),
                            _ => true,
                        };
                    if sub_nav_item(ui, &t, label, sel).clicked() {
                        self.lib.go(loc);
                    }
                }
            }
            if self.data.settings.nav_shown("servers") {
                self.plex_nav(ui, nav_item);
            }
            let show_pins = self.data.settings.nav_shown("pins");
            if show_pins {
                ui.add_space(14.0);
                ui.label(RichText::new(t!("ANGEHEFTET")).size(11.0).color(t.text_faint));
                ui.add_space(2.0);
            }
            let pins = if show_pins { self.data.pins.clone() } else { Vec::new() };
            let mut unpin = None;
            for (i, p) in pins.iter().enumerate() {
                let icon = match p.target {
                    PinTarget::Folder(_) => ph::FOLDER_SIMPLE,
                    PinTarget::Playlist(_) => ph::PLAYLIST,
                    PinTarget::Stream(_) => ph::BROADCAST,
                    PinTarget::File(_) => ph::FILE_VIDEO,
                };
                let r = nav_item(ui, &t, icon, &p.name, false);
                if r.clicked() {
                    self.open_pin(p);
                }
                r.context_menu(|ui| {
                    if widgets::menu_item(ui, &t, ph::PUSH_PIN_SLASH, t!("Lösen"), "", false).clicked() {
                        unpin = Some(i);
                        ui.close();
                    }
                });
            }
            if pins.is_empty() && show_pins {
                ui.label(RichText::new(t!("Rechtsklick auf einen Ordner → Anheften")).size(11.5).color(t.text_faint));
            }
            if let Some(i) = unpin {
                self.data.pins.remove(i);
                self.mark_dirty();
            }
            ui.add_space(8.0);
            if nav_item(ui, &t, ph::PLUS, t!("Ordner hinzufügen"), false).clicked() {
                self.add_library_folder();
            }
        });

        let content = Rect::from_min_max(pos2(nav.right() + 20.0, full.top() + 10.0), pos2(full.right() - 20.0, full.bottom()));
        let mini = self.has_media();
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| self.library_content(ui, false));

        if mini {
            self.mini_player(ui, full);
        }
    }

    /// The page for the current location. `touch`: phone layout (navigation
    /// and back button live in the header, narrow lists, no floating player).
    pub fn library_content(&mut self, ui: &mut Ui, touch: bool) {
        let t = self.t;
        let mini = self.has_media() && !touch;
        if self.lib.loc == LibLoc::Home {
            self.home(ui);
        } else if let LibLoc::Plex(p) = self.lib.loc.clone() {
            if self.plex.ready() && !touch {
                self.header_nav(ui, false);
                ui.add_space(4.0);
            }
            if mini {
                // (Not set_max_height: that puts the cursor back to the top,
                // over the header.)
                ui.scope_builder(egui::UiBuilder::new().max_rect(below_cursor(ui, 4.0)), |ui| self.plex_page(ui, p, touch));
            } else {
                self.plex_page(ui, p, touch);
            }
        } else {
            if !touch {
                self.header_nav(ui, false);
            }
            let entries = self.entries();
            let mut click = None;
            self.lib_toolbar(ui, entries.iter().filter(|e| !matches!(e.kind, Kind::NewPlaylist | Kind::NewStream)).count(), touch, &entries, &mut click);
            widgets::separator(ui, &t);
            if crate::platform::MOBILE && matches!(self.lib.loc, LibLoc::Videos(_) | LibLoc::Music(_) | LibLoc::MusicFolder(_)) {
                self.storage_card(ui);
            }
            if mini {
                ui.scope_builder(egui::UiBuilder::new().max_rect(below_cursor(ui, 4.0)), |ui| self.entries_view(ui, &entries, touch, &mut click));
            } else {
                self.entries_view(ui, &entries, touch, &mut click);
            }
            if let Some(c) = click {
                self.handle_click(c, &entries, touch);
            }
        }
    }

    /// Android without the storage permission: the library can't see any
    /// file, say why and ask.
    fn storage_card(&mut self, ui: &mut Ui) {
        if self.storage.media() {
            return;
        }
        let t = self.t;
        ui.add_space(10.0);
        egui::Frame::new().fill(t.elevated).corner_radius(CornerRadius::same(theme::R_LG)).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(ph::LOCK_SIMPLE).size(18.0).color(t.accent));
                ui.label(RichText::new(t!("Zugriff auf Videos und Musik")).font(theme::semibold(15.0)).color(t.text));
            });
            ui.add_space(4.0);
            ui.label(RichText::new(t!("Velour braucht deine Erlaubnis, um die Videos und die Musik auf diesem Gerät anzuzeigen.")).color(t.text_dim));
            ui.add_space(8.0);
            if widgets::accent_button(ui, &t, t!("Zugriff erlauben")).clicked() {
                crate::pick::request_storage();
            }
        });
        ui.add_space(6.0);
    }

    fn mini_player(&mut self, ui: &mut Ui, area: Rect) {
        let t = self.t;
        let Some(p) = &self.player else { return };
        let st = p.st.clone();
        let w = 300.0;
        let h = if st.has_video() { w * 9.0 / 16.0 } else { 64.0 };
        let r = Rect::from_min_size(pos2(area.right() - w - 18.0, area.bottom() - h - 44.0), vec2(w, h + 30.0));
        let resp = ui.interact(r, Id::new("mini_player"), Sense::click());
        ui.painter().rect_filled(r.expand(1.0), CornerRadius::same(theme::R_LG), t.border);
        ui.painter().rect_filled(r, CornerRadius::same(theme::R_LG), t.surface);
        let vr = Rect::from_min_size(r.min, vec2(w, h));
        if st.has_video() {
            if let Some(v) = &self.video {
                v.paint(ui.painter(), vr);
            }
        } else {
            ui.painter().rect_filled(vr, CornerRadius::same(theme::R_LG), t.elevated);
            widgets::draw_logo(ui.painter(), Rect::from_center_size(pos2(vr.left() + 32.0, vr.center().y), vec2(30.0, 30.0)), t.accent);
            widgets::painter_text_trunc(ui.painter(), pos2(vr.left() + 60.0, vr.center().y - 9.0), &st.media_title, FontId::proportional(13.0), t.text, w - 70.0);
        }
        widgets::painter_text_trunc(ui.painter(), pos2(r.left() + 10.0, vr.bottom() + 7.0), &st.media_title, FontId::proportional(12.0), t.text_dim, w - 20.0);
        // Radio is live: a full line instead of the buffer position.
        let radio = self.radio_urls.contains(&st.path);
        if radio || st.duration > 0.0 {
            let f = if radio { 1.0 } else { (st.time / st.duration) as f32 };
            widgets::progress_line(ui.painter(), Rect::from_min_max(pos2(r.left(), vr.bottom() - 3.0), pos2(r.right(), vr.bottom())), f, &t);
        }
        if resp.hovered() {
            ui.painter().text(vr.center(), Align2::CENTER_CENTER, ph::ARROWS_OUT_SIMPLE, theme::icon(26.0), Color32::from_white_alpha(220));
        }
        if resp.on_hover_text(t!("Zurück zum Video")).on_hover_cursor(CursorIcon::PointingHand).clicked() {
            self.view = View::Player;
        }
    }

    // ---------------------------------------------------------------- home

    fn home(&mut self, ui: &mut Ui) {
        let t = self.t;
        let ctx = ui.ctx().clone();
        let mut hist = self.data.history.clone();
        hist.sort_by(|a, b| b.last_played.cmp(&a.last_played));
        let cont: Vec<_> = hist.iter().filter(|h| h.resumable() && !media::is_url(&h.path) && Path::new(&h.path).exists()).take(8).cloned().collect();
        let recent: Vec<_> = hist.iter().take(10).cloned().collect();
        let empty = hist.is_empty() && self.data.playlists.is_empty();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let touch = crate::mobile::touch();
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                // The touch header already shows the logo.
                if !touch {
                    let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                    widgets::draw_logo(ui.painter(), r, t.accent);
                }
                let size = if touch { 21.0 } else { 24.0 };
                ui.label(RichText::new(if empty { t!("Willkommen bei Velour") } else { t!("Willkommen zurück") }).font(theme::semibold(size)).color(t.text));
            });
            ui.add_space(14.0);
            ui.horizontal_wrapped(|ui| {
                if widgets::icon_text_button(ui, &t, ph::FILE_PLUS, t!("Datei öffnen")).clicked() {
                    self.dialog_open_files();
                }
                if widgets::icon_text_button(ui, &t, ph::FOLDER_OPEN, t!("Ordner öffnen")).clicked() {
                    self.dialog_open_folder();
                }
                if widgets::icon_text_button(ui, &t, ph::BROADCAST, t!("Netzwerkstream")).clicked() {
                    self.dialog = Some(Dialog::Network);
                }
                if widgets::icon_text_button(ui, &t, ph::CLIPBOARD, t!("Aus Zwischenablage")).clicked() {
                    self.open_clipboard();
                }
            });
            ui.add_space(22.0);

            if empty {
                self.plex_home_shelf(ui);
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(120.0, 120.0), Sense::hover());
                    widgets::draw_logo(ui.painter(), r, t.accent.gamma_multiply(0.85));
                    ui.add_space(16.0);
                    if touch {
                        ui.label(RichText::new(t!("Noch nichts abgespielt")).font(theme::semibold(17.0)).color(t.text));
                        ui.label(RichText::new(t!("Starte etwas über Plex, Videos oder einen Netzwerkstream.")).color(t.text_dim));
                    } else {
                        ui.label(RichText::new(t!("Ziehe Videos, Musik oder Ordner hierher")).font(theme::semibold(17.0)).color(t.text));
                        ui.label(RichText::new(t!("oder durchstöbere links deine Bibliothek.")).color(t.text_dim));
                    }
                });
                return;
            }

            if !cont.is_empty() {
                section_title(ui, &t, t!("Weiterschauen"));
                let mut play: Option<(String, f64)> = None;
                egui::ScrollArea::horizontal().id_salt("cont").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        for h in &cont {
                            let cw = 240.0;
                            let th = cw * 9.0 / 16.0;
                            let (rect, resp) = ui.allocate_exact_size(vec2(cw, th + 48.0), Sense::click());
                            let img = Rect::from_min_size(rect.min, vec2(cw, th));
                            ui.painter().rect_filled(img, CornerRadius::same(theme::R_LG), t.surface);
                            if let Some(tex) = self.thumb_for(&ctx, &h.path) {
                                egui::Image::new((tex, img.size())).corner_radius(CornerRadius::same(theme::R_LG)).paint_at(ui, img);
                            } else {
                                ui.painter().text(img.center(), Align2::CENTER_CENTER, if h.audio_only { ph::MUSIC_NOTE } else { ph::FILM_STRIP }, theme::icon(36.0), t.text_faint);
                            }
                            widgets::progress_line(ui.painter(), Rect::from_min_max(pos2(img.left(), img.bottom() - 4.0), img.max), h.progress(), &t);
                            if resp.hovered() {
                                ui.painter().rect_stroke(img, CornerRadius::same(theme::R_LG), Stroke::new(1.5, t.accent), StrokeKind::Inside);
                                let c = Rect::from_center_size(img.center(), vec2(46.0, 32.0));
                                ui.painter().rect_filled(c, CornerRadius::same(4), t.accent);
                                ui.painter().text(c.center(), Align2::CENTER_CENTER, ph::PLAY, theme::fill_icon(16.0), t.on_accent);
                            }
                            widgets::painter_text_trunc(ui.painter(), pos2(rect.left() + 2.0, img.bottom() + 7.0), &h.title, FontId::proportional(13.0), t.text, cw - 4.0);
                            let folder = media::parent_folder_name(&h.path).unwrap_or_default();
                            widgets::painter_text_trunc(
                                ui.painter(),
                                pos2(rect.left() + 2.0, img.bottom() + 26.0),
                                &tf!("{} · noch {}", folder, media::fmt_time_short(h.duration - h.position)),
                                FontId::proportional(11.5),
                                t.text_faint,
                                cw - 4.0,
                            );
                            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                play = Some((h.path.clone(), h.position));
                            }
                        }
                    });
                });
                if let Some((path, _pos)) = play {
                    let prev = self.data.settings.resume;
                    self.data.settings.resume = crate::config::ResumeMode::Always;
                    self.open_paths(vec![path]);
                    self.pending_resume_restore = Some(prev);
                }
                ui.add_space(18.0);
            }
            self.plex_home_shelf(ui);

            if !self.data.playlists.is_empty() || !self.data.streams.is_empty() || !self.data.pins.is_empty() {
                section_title(ui, &t, t!("Sammlungen"));
                // Pins, playlists and streams: square tiles on touch (two
                // side by side on a phone), chips on the desktop.
                enum Open {
                    Pin(usize),
                    Playlist(usize),
                    Stream(String),
                }
                let mut items: Vec<(&str, String, &str, Open)> = vec![];
                for (i, p) in self.data.pins.iter().enumerate() {
                    items.push((ph::PUSH_PIN, p.name.clone(), t!("Angeheftet"), Open::Pin(i)));
                }
                for (i, p) in self.data.playlists.iter().enumerate() {
                    items.push((ph::PLAYLIST, p.name.clone(), t!("Wiedergabeliste"), Open::Playlist(i)));
                }
                for s in self.data.streams.iter().take(6) {
                    let (icon, kind) = if is_iptv_org(&s.url) {
                        (ph::TELEVISION_SIMPLE, t!("TV-Sender"))
                    } else if crate::radio::is_radio_browser(&s.url) {
                        (ph::RADIO, "Radio")
                    } else {
                        (ph::BROADCAST, "Stream")
                    };
                    items.push((icon, crate::i18n::tr(&s.name).to_string(), kind, Open::Stream(s.url.clone())));
                }
                let mut open = None;
                if touch {
                    let gap = 10.0;
                    let w = ui.available_width();
                    // Two per row on a phone (50/50), more side by side on
                    // wide screens; height stays compact.
                    let cols = ((w + gap) / 230.0).floor().max(2.0);
                    let size = ((w - (cols - 1.0) * gap) / cols).floor();
                    for row in items.chunks(cols as usize) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for (icon, name, kind, o) in row {
                                if collection_tile(ui, &t, icon, name, kind, size).clicked() {
                                    open = Some(o);
                                }
                            }
                        });
                        ui.add_space(gap - ui.spacing().item_spacing.y);
                    }
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                        for (icon, name, _, o) in &items {
                            if collection_chip(ui, &t, icon, name).clicked() {
                                open = Some(o);
                            }
                        }
                    });
                }
                match open {
                    Some(Open::Pin(i)) => {
                        if let Some(p) = self.data.pins.get(*i).cloned() {
                            self.open_pin(&p);
                        }
                    }
                    Some(Open::Playlist(i)) => self.lib.go(LibLoc::Playlists(Some(*i))),
                    Some(Open::Stream(url)) if is_iptv_org(url) => self.lib.go(LibLoc::Tv(None)),
                    Some(Open::Stream(url)) if crate::radio::is_radio_browser(url) => self.lib.go(LibLoc::Radio(None)),
                    Some(Open::Stream(url)) => self.play_list(&[url.clone()], 0),
                    None => {}
                }
                ui.add_space(18.0);
            }

            if !recent.is_empty() {
                section_title(ui, &t, t!("Zuletzt gespielt"));
                for h in &recent {
                    let w = ui.available_width();
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, if touch { 54.0 } else { 40.0 }), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(theme::R), t.hover);
                    }
                    let icon = live_icon(h).unwrap_or(if h.audio_only { ph::MUSIC_NOTE } else if media::is_url(&h.path) { ph::BROADCAST } else { ph::FILM_STRIP });
                    let logo_drawn = match &h.logo {
                        Some(l) => self.paint_logo(ui, l, Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(24.0, 24.0))),
                        None => false,
                    };
                    if !logo_drawn {
                        ui.painter().text(pos2(rect.left() + 18.0, rect.center().y), Align2::CENTER_CENTER, icon, theme::icon(16.0), t.text_dim);
                    }
                    let folder = if h.live { live_label(h) } else { media::parent_folder_name(&h.path).unwrap_or_else(|| "Stream".into()) };
                    let status = if h.live {
                        String::new()
                    } else if h.finished {
                        t!("gesehen").to_string()
                    } else if h.duration > 0.0 {
                        format!("{:.0} %", h.progress() * 100.0)
                    } else {
                        String::new()
                    };
                    let status_col = if h.finished { t.accent } else { t.text_faint };
                    if touch {
                        // Two lines: title, then folder · status.
                        let tw = w - 48.0;
                        widgets::painter_text_trunc(ui.painter(), pos2(rect.left() + 40.0, rect.center().y - 19.0), &h.title, FontId::proportional(14.0), t.text, tw);
                        let sg = ui.painter().layout_no_wrap(status.clone(), FontId::proportional(12.0), status_col);
                        let sw = if status.is_empty() { 0.0 } else { sg.size().x + 12.0 };
                        widgets::painter_text_trunc(ui.painter(), pos2(rect.left() + 40.0, rect.center().y + 1.0), &folder, FontId::proportional(12.0), t.text_faint, tw - sw);
                        ui.painter().text(pos2(rect.right() - 8.0, rect.center().y + 9.0), Align2::RIGHT_CENTER, status, FontId::proportional(12.0), status_col);
                    } else {
                        widgets::painter_text_trunc(ui.painter(), pos2(rect.left() + 40.0, rect.center().y - 9.0), &h.title, FontId::proportional(13.5), t.text, w * 0.55);
                        widgets::painter_text_trunc(ui.painter(), pos2(rect.left() + w * 0.62, rect.center().y - 9.0), &folder, FontId::proportional(12.5), t.text_faint, w * 0.25);
                        ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, status, FontId::proportional(12.0), status_col);
                    }
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        self.open_paths(vec![h.path.clone()]);
                    }
                }
            }
        });
    }
}

fn section_title(ui: &mut Ui, t: &theme::Theme, s: &str) {
    ui.label(RichText::new(s).font(theme::semibold(16.0)).color(t.text));
    ui.add_space(8.0);
}

/// Settings id (`config::NAV_ITEMS`) of a library section number.
fn nav_id(section: u8) -> &'static str {
    match section {
        1 => "videos",
        2 => "music",
        3 => "network",
        4 => "playlists",
        5 => "recent",
        6 => "servers",
        7 => "tv",
        8 => "radio",
        _ => "home",
    }
}

fn nav_item(ui: &mut Ui, t: &theme::Theme, icon: &str, label: &str, selected: bool) -> egui::Response {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
    if selected {
        ui.painter().rect_filled(r, CornerRadius::same(theme::R), t.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(theme::R), t.hover);
    }
    let col = if selected { t.accent } else { t.text_dim };
    ui.painter().text(pos2(r.left() + 16.0, r.center().y), Align2::CENTER_CENTER, icon, theme::icon(17.0), col);
    widgets::painter_text_trunc(ui.painter(), pos2(r.left() + 36.0, r.center().y - 9.0), label, FontId::proportional(13.5), if selected { t.text } else { t.text_dim }, w - 40.0);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn sub_nav_item(ui: &mut Ui, t: &theme::Theme, label: &str, selected: bool) -> egui::Response {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(theme::R), t.hover);
    }
    if selected {
        ui.painter().rect_filled(Rect::from_min_size(pos2(r.left() + 24.0, r.top() + 6.0), vec2(2.0, 16.0)), CornerRadius::ZERO, t.accent);
    }
    ui.painter().text(pos2(r.left() + 36.0, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), if selected { t.text } else { t.text_dim });
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Tile for the touch home page: icon and kind on top, name (up to two
/// lines) at the bottom. `w` wide, compact height.
fn collection_tile(ui: &mut Ui, t: &theme::Theme, icon: &str, label: &str, kind: &str, w: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(w, 94.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(theme::R_LG + 2), if resp.is_pointer_button_down_on() { t.hover } else { t.surface });
    let pad = 12.0;
    let badge = Rect::from_min_size(r.min + vec2(pad, pad), vec2(32.0, 32.0));
    p.rect_filled(badge, CornerRadius::same(9), t.accent_soft);
    p.text(badge.center(), Align2::CENTER_CENTER, icon, theme::icon(17.0), t.accent);
    p.text(pos2(r.right() - pad, badge.center().y), Align2::RIGHT_CENTER, kind, FontId::proportional(11.0), t.text_dim);
    let mut job = egui::text::LayoutJob::simple(label.to_string(), theme::semibold(13.0), t.text, w - pad * 2.0);
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = false;
    let g = p.layout_job(job);
    p.galley(pos2(r.left() + pad, r.bottom() - pad - g.size().y), g, t.text);
    resp
}

fn collection_chip(ui: &mut Ui, t: &theme::Theme, icon: &str, label: &str) -> egui::Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(13.0), t.text);
    let w = (g.size().x + 46.0).min(260.0);
    let (r, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
    ui.painter().rect_filled(r, CornerRadius::same(theme::R_LG), if resp.hovered() { t.hover } else { t.surface });
    ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::CENTER_CENTER, icon, theme::icon(16.0), t.accent);
    widgets::painter_text_trunc(ui.painter(), pos2(r.left() + 34.0, r.center().y - 9.0), label, FontId::proportional(13.0), t.text, w - 40.0);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// The space left below the cursor, `gap` short of the bottom.
fn below_cursor(ui: &Ui, gap: f32) -> Rect {
    let mut r = ui.available_rect_before_wrap();
    r.max.y = (r.max.y - gap).max(r.min.y);
    r
}
