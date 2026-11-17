//! Library navigation state and background scanning.

use crate::config::Data;
use crate::media;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

#[derive(Clone, PartialEq, Debug)]
pub enum MusicView {
    Songs,
    Albums,
    Artists,
    Genres,
    Album(String),
    Artist(String),
    Genre(String),
}

#[derive(Clone, PartialEq, Debug)]
pub enum LibLoc {
    Home,
    Videos(Option<PathBuf>),
    Music(MusicView),
    MusicFolder(Option<PathBuf>),
    Network,
    Playlists(Option<usize>),
    Recent,
    Folder(PathBuf),
    Plex(PlexLoc),
    /// Public TV (iptv-org): languages, or the channels of one language
    /// (code, label).
    Tv(Option<(String, String)>),
    /// Public radio (radio-browser.info): languages, or the stations of one
    /// language (API name, label).
    Radio(Option<(String, String)>),
}

/// Places inside the Plex part of the library.
#[derive(Clone, PartialEq, Debug)]
pub enum PlexLoc {
    /// Server overview (or the sign-in page while not connected).
    Home,
    Section { key: String, title: String, tab: PlexTab },
    /// Detail page of a movie, show, collection, playlist …
    Item { key: String, title: String, section: Option<(String, String)> },
    /// Full list behind a hub ("Alle anzeigen").
    Hub { key: String, title: String },
    Search(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlexTab {
    Recommended,
    Library,
    Collections,
    Playlists,
}

impl LibLoc {
    pub fn section(&self) -> u8 {
        match self {
            LibLoc::Home => 0,
            LibLoc::Videos(_) | LibLoc::Folder(_) => 1,
            LibLoc::Music(_) | LibLoc::MusicFolder(_) => 2,
            LibLoc::Network => 3,
            LibLoc::Playlists(_) => 4,
            LibLoc::Recent => 5,
            LibLoc::Plex(_) => 6,
            LibLoc::Tv(_) => 7,
            LibLoc::Radio(_) => 8,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortMode {
    UnwatchedFirst,
    WatchedFirst,
    Newest,
    Oldest,
    TitleAz,
    TitleZa,
}

impl SortMode {
    pub const ALL: [SortMode; 6] =
        [SortMode::UnwatchedFirst, SortMode::WatchedFirst, SortMode::Newest, SortMode::Oldest, SortMode::TitleAz, SortMode::TitleZa];
    pub fn label(self) -> &'static str {
        match self {
            SortMode::UnwatchedFirst => t!("Ungesehene zuerst"),
            SortMode::WatchedFirst => t!("Gesehene zuerst"),
            SortMode::Newest => t!("Neueste zuerst"),
            SortMode::Oldest => t!("Älteste zuerst"),
            SortMode::TitleAz => t!("Titel A–Z"),
            SortMode::TitleZa => t!("Titel Z–A"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    Grid,
    List,
}

pub struct Listing {
    pub dir: PathBuf,
    pub folders: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
}

pub struct LibraryState {
    pub loc: LibLoc,
    pub back: Vec<LibLoc>,
    pub fwd: Vec<LibLoc>,
    pub search: String,
    pub search_open: bool,
    pub sort: SortMode,
    pub view: ViewMode,
    /// The narrow library next to the player has its own view (list by default).
    pub compact_view: ViewMode,
    pub compact_open: bool,
    pub listing: Option<Listing>,
    pub music_files: Vec<PathBuf>,
    pub scanning: bool,
    scan_rx: Option<Receiver<Vec<PathBuf>>>,
}

impl LibraryState {
    pub fn new() -> Self {
        Self {
            loc: LibLoc::Home,
            back: vec![],
            fwd: vec![],
            search: String::new(),
            search_open: false,
            sort: SortMode::TitleAz,
            view: ViewMode::Grid,
            compact_view: ViewMode::List,
            compact_open: false,
            listing: None,
            music_files: vec![],
            scanning: false,
            scan_rx: None,
        }
    }

    pub fn go(&mut self, loc: LibLoc) {
        if loc != self.loc {
            self.back.push(std::mem::replace(&mut self.loc, loc));
            self.fwd.clear();
            self.search.clear();
        }
    }
    /// Opens a top-level page with a fresh history (phone tabs).
    pub fn go_root(&mut self, loc: LibLoc) {
        self.go(loc);
        self.back.clear();
        self.fwd.clear();
    }
    pub fn go_back(&mut self) {
        if let Some(l) = self.back.pop() {
            self.fwd.push(std::mem::replace(&mut self.loc, l));
        }
    }
    pub fn go_fwd(&mut self) {
        if let Some(l) = self.fwd.pop() {
            self.back.push(std::mem::replace(&mut self.loc, l));
        }
    }

    pub fn listing_for(&mut self, dir: &Path, show_hidden: bool) -> &Listing {
        let stale = self.listing.as_ref().map(|l| l.dir != dir).unwrap_or(true);
        if stale {
            let (folders, files) = media::list_dir(dir, show_hidden);
            self.listing = Some(Listing { dir: dir.to_path_buf(), folders, files });
        }
        self.listing.as_ref().unwrap()
    }

    pub fn refresh(&mut self) {
        self.listing = None;
    }

    pub fn start_music_scan(&mut self, data: &Data, ctx: &eframe::egui::Context) {
        if self.scanning {
            return;
        }
        let roots = data.settings.music_folders.clone();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        self.scanning = true;
        self.scan_rx = Some(rx);
        std::thread::spawn(move || {
            let mut out = Vec::new();
            for r in roots {
                media::scan_recursive(&r, media::is_audio, &mut out, 0);
            }
            out.sort_by(|a, b| media::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
            out.dedup();
            let _ = tx.send(out);
            ctx.request_repaint();
        });
    }

    pub fn poll_scan(&mut self) {
        if let Some(rx) = &self.scan_rx {
            if let Ok(v) = rx.try_recv() {
                self.music_files = v;
                self.scanning = false;
                self.scan_rx = None;
            }
        }
    }

    pub fn title(&self, data: &Data) -> String {
        match &self.loc {
            LibLoc::Home => "Start".into(),
            LibLoc::Videos(None) => "Videos".into(),
            LibLoc::Videos(Some(p)) | LibLoc::Folder(p) | LibLoc::MusicFolder(Some(p)) => {
                p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string_lossy().to_string())
            }
            LibLoc::MusicFolder(None) => t!("Musik · Ordner").into(),
            LibLoc::Music(v) => match v {
                MusicView::Songs => t!("Musik").into(),
                MusicView::Albums => t!("Alben").into(),
                MusicView::Artists => t!("Interpreten").into(),
                MusicView::Genres => "Genres".into(),
                MusicView::Album(a) | MusicView::Artist(a) | MusicView::Genre(a) => a.clone(),
            },
            LibLoc::Network => t!("Netzwerk").into(),
            LibLoc::Tv(None) => t!("TV-Sender").into(),
            LibLoc::Tv(Some((_, label))) => label.clone(),
            LibLoc::Radio(None) => t!("Radiosender").into(),
            LibLoc::Radio(Some((_, label))) => label.clone(),
            LibLoc::Playlists(None) => "Playlists".into(),
            LibLoc::Playlists(Some(i)) => data.playlists.get(*i).map(|p| p.name.clone()).unwrap_or_default(),
            LibLoc::Recent => t!("Zuletzt gespielt").into(),
            LibLoc::Plex(p) => match p {
                PlexLoc::Home => plex_root(data),
                PlexLoc::Section { title, .. } | PlexLoc::Item { title, .. } | PlexLoc::Hub { title, .. } => title.clone(),
                PlexLoc::Search(q) => tf!("Suche: {q}"),
            },
        }
    }

    /// Breadcrumb trail: (label, location).
    pub fn crumbs(&self, data: &Data) -> Vec<(String, LibLoc)> {
        let mut v = Vec::new();
        match &self.loc {
            LibLoc::Videos(Some(p)) => {
                v.push(("Videos".into(), LibLoc::Videos(None)));
                let root = data.settings.video_folders.iter().find(|r| p.starts_with(r)).cloned();
                if let Some(root) = root {
                    let mut cur = root.clone();
                    v.push((name_of(&root), LibLoc::Videos(Some(root.clone()))));
                    if let Ok(rel) = p.strip_prefix(&root) {
                        for c in rel.components() {
                            cur = cur.join(c);
                            v.push((name_of(&cur), LibLoc::Videos(Some(cur.clone()))));
                        }
                    }
                } else {
                    v.push((name_of(p), self.loc.clone()));
                }
            }
            LibLoc::MusicFolder(Some(p)) => {
                v.push((t!("Musik").into(), LibLoc::Music(MusicView::Songs)));
                v.push((name_of(p), self.loc.clone()));
            }
            LibLoc::Music(MusicView::Album(_)) => {
                v.push((t!("Alben").into(), LibLoc::Music(MusicView::Albums)));
                v.push((self.title(data), self.loc.clone()));
            }
            LibLoc::Music(MusicView::Artist(_)) => {
                v.push((t!("Interpreten").into(), LibLoc::Music(MusicView::Artists)));
                v.push((self.title(data), self.loc.clone()));
            }
            LibLoc::Music(MusicView::Genre(_)) => {
                v.push(("Genres".into(), LibLoc::Music(MusicView::Genres)));
                v.push((self.title(data), self.loc.clone()));
            }
            LibLoc::Playlists(Some(_)) => {
                v.push(("Playlists".into(), LibLoc::Playlists(None)));
                v.push((self.title(data), self.loc.clone()));
            }
            LibLoc::Tv(sel) => {
                v.push((t!("TV-Sender").into(), LibLoc::Tv(None)));
                if sel.is_some() {
                    v.push((self.title(data), self.loc.clone()));
                }
            }
            LibLoc::Radio(sel) => {
                v.push((t!("Radiosender").into(), LibLoc::Radio(None)));
                if sel.is_some() {
                    v.push((self.title(data), self.loc.clone()));
                }
            }
            LibLoc::Folder(p) => {
                if let Some(parent) = p.parent() {
                    v.push((name_of(parent), LibLoc::Folder(parent.to_path_buf())));
                }
                v.push((name_of(p), self.loc.clone()));
            }
            LibLoc::Plex(p) => {
                let root = LibLoc::Plex(PlexLoc::Home);
                v.push((plex_root(data), root.clone()));
                match p {
                    PlexLoc::Home => {}
                    PlexLoc::Item { section: Some((key, title)), .. } => {
                        v.push((title.clone(), LibLoc::Plex(PlexLoc::Section { key: key.clone(), title: title.clone(), tab: PlexTab::Library })));
                        v.push((self.title(data), self.loc.clone()));
                    }
                    _ => v.push((self.title(data), self.loc.clone())),
                }
            }
            _ => v.push((self.title(data), self.loc.clone())),
        }
        v
    }
}

fn plex_root(data: &Data) -> String {
    if data.plex.server_name.is_empty() { "Plex".into() } else { data.plex.server_name.clone() }
}

pub fn name_of(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string_lossy().to_string())
}
