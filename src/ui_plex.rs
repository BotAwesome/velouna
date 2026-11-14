//! Plex views in Velour's design language: server overview, library browser
//! (recommended / library / collections / playlists), detail pages and the
//! sign-in flow. Data comes from `crate::plex`; everything here is drawing and
//! turning clicks into navigation or playback.

use crate::app::{App, ResumePrompt, View};
use crate::config::ResumeMode;
use crate::library::{LibLoc, PlexLoc, PlexTab};
use crate::plex::{self, Hub, Item, Kind, PlayMode, PlayReq, St, Status};
use crate::theme::{self, Theme};
use crate::widgets::{self, icon_button};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2,
    text::{LayoutJob, TextFormat, TextWrapping},
    vec2,
};
use egui_phosphor::regular as ph;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ------------------------------------------------------------ ui state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Title,
    Added,
    Released,
    Rating,
    Viewed,
}

impl SortKey {
    const ALL: [SortKey; 5] = [SortKey::Title, SortKey::Added, SortKey::Released, SortKey::Rating, SortKey::Viewed];
    fn label(self) -> &'static str {
        match self {
            SortKey::Title => t!("Titel"),
            SortKey::Added => t!("Zuletzt hinzugefügt"),
            SortKey::Released => t!("Erscheinungsdatum"),
            SortKey::Rating => t!("Bewertung"),
            SortKey::Viewed => t!("Zuletzt angesehen"),
        }
    }
    fn field(self) -> &'static str {
        match self {
            SortKey::Title => "titleSort",
            SortKey::Added => "addedAt",
            SortKey::Released => "originallyAvailableAt",
            SortKey::Rating => "audienceRating",
            SortKey::Viewed => "lastViewedAt",
        }
    }
    fn natural_desc(self) -> bool {
        self != SortKey::Title
    }
}

#[derive(Clone, Debug)]
pub struct Browse {
    pub sort: SortKey,
    pub desc: bool,
    pub unwatched: bool,
    pub genre: Option<(String, String)>,
    pub search_open: bool,
    pub search: String,
    pub applied: String,
    pub typed_at: Option<Instant>,
}

impl Default for Browse {
    fn default() -> Self {
        Browse { sort: SortKey::Title, desc: false, unwatched: false, genre: None, search_open: false, search: String::new(), applied: String::new(), typed_at: None }
    }
}

#[derive(Default)]
struct ShelfState {
    target: f32,
    animating: bool,
}

#[derive(Default)]
pub struct PlexUi {
    pub browse: HashMap<String, Browse>,
    /// show rating key -> selected season rating key
    pub season: HashMap<String, String>,
    pub home_search: String,
    jump: Option<(String, f32)>,
    shelves: HashMap<Id, ShelfState>,
    hero_idx: usize,
    hero_changed: Option<Instant>,
    /// (url, position) to offer resuming after the file loaded.
    pub resume_hint: Option<(String, f64)>,
    subs_done: HashSet<String>,
    expanded: HashSet<String>,
    /// Set by `plex_item` in side layout: the column `detail_header` draws into.
    side_col: Option<Rect>,
    /// Debounce for the live server search.
    search_typed: Option<Instant>,
    /// Query the search field last mirrored from the page location.
    search_synced: Option<String>,
}

/// Side column width for the side layout, or None when the page is too narrow
/// (or the user chose the banner layout) and content should stack.
fn side_width(total: f32, side_layout: bool, compact: bool) -> Option<f32> {
    (side_layout && !compact && total >= 860.0).then(|| (total * 0.3).clamp(330.0, 430.0))
}

// ------------------------------------------------------------- actions

enum Act {
    Go(LibLoc),
    Open(Item),
    Play { item: Item, start_at: Option<f64>, mode: PlayMode, shuffle: bool },
    Watched(Item, bool),
    ToShow(Item),
    Season(String, String),
}

#[derive(Clone, Copy, PartialEq)]
enum CardShape {
    Poster,
    Wide,
    Square,
}

impl CardShape {
    fn for_item(i: &Item) -> CardShape {
        if i.kind.landscape() {
            CardShape::Wide
        } else if i.kind.square() {
            CardShape::Square
        } else {
            CardShape::Poster
        }
    }
    fn ratio(self) -> f32 {
        match self {
            CardShape::Poster => 1.5,
            CardShape::Wide => 9.0 / 16.0,
            CardShape::Square => 1.0,
        }
    }
}

// ------------------------------------------------------------- helpers

fn paint_image(ui: &Ui, rect: Rect, img: &plex::images::Shown, r: CornerRadius) {
    let (ia, ra) = (img.size.x / img.size.y.max(1.0), rect.width() / rect.height().max(1.0));
    let uv = if ia > ra {
        let w = ra / ia;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = ia / ra;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    };
    egui::Image::new((img.id, rect.size()))
        .uv(uv)
        .corner_radius(r)
        .tint(Color32::from_white_alpha((img.alpha * 255.0) as u8))
        .paint_at(ui, rect);
}

/// Four-corner gradient (colors are premultiplied).
fn gradient(p: &egui::Painter, r: Rect, tl: Color32, tr: Color32, br: Color32, bl: Color32) {
    let mut m = egui::Mesh::default();
    m.colored_vertex(r.left_top(), tl);
    m.colored_vertex(r.right_top(), tr);
    m.colored_vertex(r.right_bottom(), br);
    m.colored_vertex(r.left_bottom(), bl);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    p.add(egui::Shape::mesh(m));
}

fn fade(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

pub fn fmt_minutes(secs: f64) -> String {
    let m = (secs / 60.0).round() as i64;
    if m >= 60 {
        let (h, mm) = (m / 60, m % 60);
        if mm == 0 { tf!("{h} Std.") } else { tf!("{h} Std. {mm} Min.") }
    } else {
        tf!("{} Min.", m.max(1))
    }
}

fn fmt_date(s: &str) -> String {
    const M: [&str; 12] = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];
    let p: Vec<&str> = s.split('-').collect();
    if p.len() == 3 {
        if let (Ok(y), Ok(m), Ok(d)) = (p[0].parse::<i32>(), p[1].parse::<usize>(), p[2].parse::<u32>()) {
            if (1..=12).contains(&m) {
                let month = crate::i18n::tr(M[m - 1]);
                return if crate::i18n::is_en() { format!("{month} {d}, {y}") } else { format!("{d}. {month} {y}") };
            }
        }
    }
    s.to_string()
}

fn fmt_size(bytes: i64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        let v = format!("{:.1} GB", b / 1e9);
        if crate::i18n::is_en() { v } else { v.replace('.', ",") }
    } else { format!("{:.0} MB", b / 1e6) }
}

fn content_rating(s: &str) -> String {
    let r = s.rsplit('/').next().unwrap_or(s).trim().to_string();
    if r.chars().all(|c| c.is_ascii_digit()) && !r.is_empty() { format!("FSK {r}") } else { r }
}

fn rating_label(r: &plex::model::Rating) -> Option<(String, &'static str)> {
    let src = r.source.to_lowercase();
    if src.contains("imdb") {
        Some((format!("IMDb {}", format!("{:.1}", r.value).replace('.', ",")), ph::STAR))
    } else if src.contains("rottentomatoes") {
        let pct = if r.value <= 10.0 { r.value * 10.0 } else { r.value };
        Some((format!("{} {:.0} %", if r.audience { t!("Publikum") } else { t!("Kritiker") }, pct), if r.audience { ph::POPCORN } else { ph::FILM_REEL }))
    } else if src.contains("themoviedb") || src.contains("tmdb") {
        Some((format!("TMDB {:.0} %", if r.value <= 10.0 { r.value * 10.0 } else { r.value }), ph::STAR_HALF))
    } else if src.contains("thetvdb") || src.contains("tvdb") {
        Some((format!("TVDB {}", format!("{:.1}", r.value).replace('.', ",")), ph::STAR_HALF))
    } else {
        None
    }
}

fn wrapped(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32, rows: usize) -> Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(text.to_string(), TextFormat { font_id: font, color, ..Default::default() });
    job.wrap = TextWrapping { max_width: width, max_rows: rows, break_anywhere: false, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}

fn placeholder(ui: &Ui, t: &Theme, rect: Rect, item: &Item, r: CornerRadius) {
    ui.painter().rect_filled(rect, r, t.elevated);
    let icon = match item.kind {
        Kind::Show | Kind::Season | Kind::Episode => ph::TELEVISION_SIMPLE,
        Kind::Collection => ph::STACK,
        Kind::Playlist => ph::PLAYLIST,
        Kind::Artist | Kind::Album | Kind::Track => ph::MUSIC_NOTES,
        _ => ph::FILM_SLATE,
    };
    ui.painter().text(rect.center() - vec2(0.0, 8.0), Align2::CENTER_CENTER, icon, theme::icon((rect.width() * 0.22).clamp(16.0, 40.0)), t.text_faint);
    let g = wrapped(ui, &item.card_title(), FontId::proportional(11.5), t.text_faint, rect.width() - 16.0, 2);
    let pos = pos2(rect.center().x - g.size().x / 2.0, rect.center().y + rect.width() * 0.12);
    ui.painter().galley(pos, g, t.text_faint);
}

fn spinner(ui: &mut Ui, t: &Theme) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.add(egui::Spinner::new().size(26.0).color(t.accent));
    });
}

fn empty_state(ui: &mut Ui, t: &Theme, icon: &str, title: &str, text: &str) {
    ui.add_space(50.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(icon).font(theme::icon(40.0)).color(t.text_faint));
        ui.add_space(8.0);
        ui.label(RichText::new(title).font(theme::semibold(16.0)).color(t.text));
        if !text.is_empty() {
            ui.add_space(2.0);
            ui.label(RichText::new(text).color(t.text_dim));
        }
    });
}

/// Rounded square icon button on an elevated background (detail page actions).
fn action_icon(ui: &mut Ui, t: &Theme, icon: &str, tip: &str, active: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
    ui.painter().rect_filled(r, CornerRadius::same(theme::R), if resp.hovered() { t.hover } else { t.elevated });
    ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, theme::icon(18.0), if active { t.accent } else if resp.hovered() { t.text } else { t.text_dim });
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip)
}

/// Accent button with icon, label and an optional dim second line.
fn play_pill(ui: &mut Ui, t: &Theme, label: &str, sub: &str) -> egui::Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), theme::semibold(14.0), t.on_accent);
    let sg = (!sub.is_empty()).then(|| ui.painter().layout_no_wrap(sub.to_string(), FontId::proportional(12.0), t.on_accent));
    let text_w = g.size().x + sg.as_ref().map(|s| s.size().x + 10.0).unwrap_or(0.0);
    let (r, resp) = ui.allocate_exact_size(vec2(text_w + 54.0, 36.0), Sense::click());
    ui.painter().rect_filled(r, CornerRadius::same(theme::R), if resp.hovered() { t.accent.gamma_multiply(1.1) } else { t.accent });
    ui.painter().text(pos2(r.left() + 20.0, r.center().y), Align2::CENTER_CENTER, ph::PLAY, theme::fill_icon(15.0), t.on_accent);
    let x = r.left() + 36.0;
    let gh = g.size().y;
    ui.painter().galley(pos2(x, r.center().y - gh / 2.0), g.clone(), t.on_accent);
    if let Some(sg) = sg {
        let sh = sg.size().y;
        ui.painter().galley(pos2(x + g.size().x + 10.0, r.center().y - sh / 2.0), sg, fade(t.on_accent, 0.75));
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn chip(ui: &mut Ui, t: &Theme, icon: &str, text: &str) {
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(12.5), t.text);
    let iw = if icon.is_empty() { 0.0 } else { 18.0 };
    let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 18.0 + iw, 24.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(theme::R), t.elevated);
    ui.painter().rect_stroke(r, CornerRadius::same(theme::R), Stroke::new(1.0, t.border), StrokeKind::Inside);
    if !icon.is_empty() {
        ui.painter().text(pos2(r.left() + 15.0, r.center().y), Align2::CENTER_CENTER, icon, theme::icon(13.0), t.accent);
    }
    let gh = g.size().y;
    ui.painter().galley(pos2(r.left() + 9.0 + iw, r.center().y - gh / 2.0), g, t.text);
}

/// Small dropdown chip for toolbar filters (icon, label, caret).
fn tool_chip(ui: &mut Ui, t: &Theme, icon: &str, text: &str, active: bool) -> egui::Response {
    let col = if active { t.accent } else { t.text_dim };
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(12.5), col);
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 54.0, 28.0), Sense::click());
    let bg = if active { t.accent_soft } else if resp.hovered() { t.hover } else { t.elevated };
    ui.painter().rect_filled(r, CornerRadius::same(theme::R), bg);
    ui.painter().text(pos2(r.left() + 15.0, r.center().y), Align2::CENTER_CENTER, icon, theme::icon(14.0), col);
    let gh = g.size().y;
    ui.painter().galley(pos2(r.left() + 28.0, r.center().y - gh / 2.0), g, col);
    ui.painter().text(pos2(r.right() - 13.0, r.center().y), Align2::CENTER_CENTER, ph::CARET_DOWN, theme::icon(11.0), t.text_faint);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn pill(ui: &mut Ui, t: &Theme, text: &str, selected: bool) -> egui::Response {
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(13.0), if selected { t.accent } else { t.text_dim });
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 26.0, 30.0), Sense::click());
    let bg = if selected { t.accent_soft } else if resp.hovered() { t.hover } else { t.elevated };
    ui.painter().rect_filled(r, CornerRadius::same(theme::R), bg);
    let gs = g.size();
    ui.painter().galley(r.center() - gs / 2.0, g, t.text);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn link(ui: &mut Ui, t: &Theme, text: &str) -> egui::Response {
    let r = ui.add(egui::Button::new(RichText::new(text).size(12.5).color(t.text_dim)).frame(false));
    if r.hovered() {
        ui.painter().text(r.rect.left_center(), Align2::LEFT_CENTER, text, FontId::proportional(12.5), t.accent);
    }
    r.on_hover_cursor(CursorIcon::PointingHand)
}

/// The same movie often lives in several libraries (HD, 4K, 3D …): show it once.
/// What to continue: Plex's "On Deck" ("Aktuell": started items and the
/// next episode of a show being watched, most recent first), then
/// "Continue Watching" (started items only) for anything On Deck misses.
/// Only the second one left out the next episode after finishing one.
fn continue_items(hubs: &[Hub]) -> Vec<Item> {
    let on_deck = |h: &Hub| h.identifier.contains("ondeck") || h.identifier.contains("onDeck");
    let mut all: Vec<Item> = hubs.iter().filter(|h| on_deck(h)).flat_map(|h| h.items.iter().cloned()).collect();
    all.extend(hubs.iter().filter(|h| h.identifier.contains("continue")).flat_map(|h| h.items.iter().cloned()));
    dedup(&all)
}

fn dedup(items: &[Item]) -> Vec<Item> {
    let mut seen = HashSet::new();
    items
        .iter()
        .filter(|i| {
            let key = match i.kind {
                Kind::Movie | Kind::Show => format!("{:?}|{}|{}", i.kind, i.title.to_lowercase(), i.year.unwrap_or(0)),
                Kind::Episode => format!("ep|{}|{}", i.grandparent_title.to_lowercase(), i.episode_code()),
                _ => i.rating_key.clone(),
            };
            seen.insert(key)
        })
        .cloned()
        .collect()
}

fn shuffle<T>(v: &mut [T]) {
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1;
    for i in (1..v.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        v.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

/// mpv's length-prefixed quoting for loadfile option values.
fn q(s: &str) -> String {
    format!("%{}%{}", s.len(), s)
}

// ---------------------------------------------------------------- impl

impl App {
    /// Per-frame Plex work: results, pending playback, progress reporting, subtitles.
    pub fn plex_frame(&mut self, ctx: &egui::Context) {
        let local_only = self.data.settings.plex_local_only;
        if self.plex.poll(ctx, &mut self.data.plex, local_only) {
            self.mark_dirty();
        }
        if let Some(req) = self.plex.pending_play.clone() {
            if self.plex_execute(&req) {
                self.plex.pending_play = None;
            }
        }
        let (path, time, dur, paused, active) = match &self.player {
            Some(p) => (p.st.path.clone(), p.st.time, p.st.duration, p.st.paused, p.st.has_media()),
            None => (String::new(), 0.0, 0.0, true, false),
        };
        let sync = self.data.settings.plex_sync_progress;
        // While "Dort weitermachen?" is open the stream sits near 0:00; reporting
        // now would overwrite the server's resume point before the user chose.
        let deciding = self.resume_prompt.as_ref().is_some_and(|r| r.path == path);
        if !deciding {
            self.plex.report(&path, time, dur, paused, active, sync);
        }
        // External subtitle files of the current Plex item.
        if active && !self.px.subs_done.contains(&path) {
            if let Some(r) = plex::remote(&path) {
                match self.plex.meta(&r.rating_key) {
                    St::Ready(item) => {
                        self.px.subs_done.insert(path.clone());
                        if let (Some(part), Some(p)) = (item.first_part(), &self.player) {
                            for s in part.streams.iter().filter(|s| s.stream_type == 3 && !s.external_key.is_empty()) {
                                let url = format!("{}{}?X-Plex-Token={}", r.base, s.external_key, plex::api::url_encode(&r.token));
                                let title = if s.display.is_empty() { s.title.clone() } else { s.display.clone() };
                                let _ = p.mpv.command(&["sub-add", &url, "auto", &title, &s.language]);
                            }
                        }
                    }
                    St::Failed(_) => {
                        self.px.subs_done.insert(path.clone());
                    }
                    St::Loading => {}
                }
            }
        }
    }

    /// Called from `on_file_loaded` for Plex streams.
    pub fn plex_file_loaded(&mut self, path: &str) {
        let Some((hint_path, pos)) = self.px.resume_hint.take() else { return };
        if hint_path == path && pos > 5.0 {
            self.resume_prompt = Some(ResumePrompt { path: path.to_string(), pos, until: Instant::now() + Duration::from_secs(10) });
        }
    }

    // ---------------------------------------------------------- playback

    fn plex_play(&mut self, item: &Item, start_at: Option<f64>, mode: PlayMode, shuffle: bool) {
        let rk = &item.rating_key;
        let (list, start_key) = match item.kind {
            Kind::Movie | Kind::Clip => (format!("/library/metadata/{rk}"), Some(rk.clone())),
            Kind::Episode | Kind::Track if !item.parent_key.is_empty() && !shuffle => (format!("/library/metadata/{}/children", item.parent_key), Some(rk.clone())),
            Kind::Episode | Kind::Track => (format!("/library/metadata/{rk}"), Some(rk.clone())),
            Kind::Show | Kind::Artist => (format!("/library/metadata/{rk}/allLeaves"), None),
            Kind::Season | Kind::Album => (format!("/library/metadata/{rk}/children"), None),
            Kind::Collection => (format!("/library/collections/{rk}/children"), None),
            Kind::Playlist => (format!("/playlists/{rk}/items"), None),
            _ => (format!("/library/metadata/{rk}/children"), None),
        };
        self.plex.pending_play = Some(PlayReq { list, start_key, start_at, shuffle, mode });
        if mode == PlayMode::Play {
            self.osd(t!("Plex wird geladen…"));
        }
    }

    /// Runs a play request once its list is loaded. Returns true when done.
    fn plex_execute(&mut self, req: &PlayReq) -> bool {
        let items = match self.plex.items(&req.list) {
            St::Ready(v) => v,
            St::Loading => return false,
            St::Failed(e) => {
                self.osd(format!("Plex: {e}"));
                return true;
            }
        };
        let mut list: Vec<Item> = items.iter().filter(|i| i.kind.playable() && i.first_part().is_some()).cloned().collect();
        if list.is_empty() {
            self.osd(t!("Nichts Abspielbares gefunden"));
            return true;
        }
        let mut start = 0;
        if req.shuffle {
            shuffle(&mut list);
        } else if let Some(k) = &req.start_key {
            start = list.iter().position(|i| &i.rating_key == k).unwrap_or(0);
        } else {
            start = list
                .iter()
                .position(|i| i.progress().is_some() && !i.watched())
                .or_else(|| list.iter().position(|i| !i.watched()))
                .unwrap_or(0);
        }
        let list: Vec<Item> = list.into_iter().skip(start).take(120).collect();
        let Some(token) = self.plex.token_for_playback() else { return true };
        let base = self.plex.server.as_ref().map(|s| s.base.clone()).unwrap_or_default();
        let mut entries: Vec<(String, String)> = Vec::new();
        for (n, it) in list.iter().enumerate() {
            let Some(part) = it.first_part() else { continue };
            let Some(url) = self.plex.stream_url(&part.key) else { continue };
            plex::register(
                &url,
                plex::Remote { rating_key: it.rating_key.clone(), title: it.play_title(), duration: it.duration(), base: base.clone(), token: token.clone(), kind: it.kind },
            );
            let mut opts = format!("force-media-title={},http-header-fields={}", q(&it.play_title()), q(&format!("X-Plex-Token: {token}")));
            if n == 0 && req.mode == PlayMode::Play {
                let offset = it.view_offset();
                let resumable = offset > 30.0 && it.progress().map(|p| p < 0.95).unwrap_or(false);
                let at = match req.start_at {
                    Some(t) => t,
                    None if resumable => match self.data.settings.resume {
                        ResumeMode::Always => offset,
                        ResumeMode::Ask => {
                            self.px.resume_hint = Some((url.clone(), offset));
                            0.0
                        }
                        ResumeMode::Never => 0.0,
                    },
                    None => 0.0,
                };
                if at > 1.0 {
                    opts.push_str(&format!(",start={at:.3}"));
                }
            }
            entries.push((url, opts));
        }
        let Some(p) = &self.player else { return true };
        match req.mode {
            PlayMode::Play => {
                for (i, (url, opts)) in entries.iter().enumerate() {
                    let _ = p.mpv.command(&["loadfile", url, if i == 0 { "replace" } else { "append" }, "-1", opts]);
                }
                p.set_pause(!self.data.settings.autoplay_on_open);
                // Music stays in the library (mini player).
                if list.first().is_none_or(|it| it.kind != plex::model::Kind::Track) {
                    self.view = View::Player;
                }
            }
            PlayMode::Next => {
                for (url, opts) in entries.iter().rev() {
                    let _ = p.mpv.command(&["loadfile", url, "insert-next", "-1", opts]);
                }
                self.osd(t!("Wird als Nächstes abgespielt"));
            }
            PlayMode::Enqueue => {
                let idle = !p.st.has_media();
                for (i, (url, opts)) in entries.iter().enumerate() {
                    let _ = p.mpv.command(&["loadfile", url, if idle && i == 0 { "append-play" } else { "append" }, "-1", opts]);
                }
                let n = entries.len();
                self.osd(if n == 1 { t!("Zur Warteschlange hinzugefügt").to_string() } else { tf!("{n} Titel zur Warteschlange hinzugefügt") });
            }
        }
        true
    }

    fn plex_handle(&mut self, acts: Vec<Act>, section: Option<(String, String)>) {
        for a in acts {
            match a {
                Act::Go(l) => self.lib.go(l),
                Act::Open(item) => match item.kind {
                    Kind::Episode | Kind::Track | Kind::Clip => self.plex_play(&item, None, PlayMode::Play, false),
                    Kind::Season => {
                        self.px.season.insert(item.parent_key.clone(), item.rating_key.clone());
                        let title = if item.parent_title.is_empty() { item.title.clone() } else { item.parent_title.clone() };
                        self.lib.go(LibLoc::Plex(PlexLoc::Item { key: item.parent_key.clone(), title, section: section.clone() }));
                    }
                    _ => {
                        let sec = section.clone().or_else(|| {
                            (!item.library_section_id.is_empty() && !item.library_section_title.is_empty())
                                .then(|| (item.library_section_id.clone(), item.library_section_title.clone()))
                        });
                        self.lib.go(LibLoc::Plex(PlexLoc::Item { key: item.rating_key.clone(), title: item.title.clone(), section: sec }));
                    }
                },
                Act::Play { item, start_at, mode, shuffle } => self.plex_play(&item, start_at, mode, shuffle),
                Act::Watched(item, w) => {
                    self.plex.set_watched(&item, w);
                    self.osd(if w { t!("Als gesehen markiert") } else { t!("Als ungesehen markiert") });
                }
                Act::ToShow(item) => {
                    let key = if item.kind == Kind::Season { item.parent_key.clone() } else { item.grandparent_key.clone() };
                    if item.kind == Kind::Episode && !item.parent_key.is_empty() {
                        self.px.season.insert(key.clone(), item.parent_key.clone());
                    }
                    let title = if item.grandparent_title.is_empty() { item.parent_title.clone() } else { item.grandparent_title.clone() };
                    self.lib.go(LibLoc::Plex(PlexLoc::Item { key, title, section: section.clone() }));
                }
                Act::Season(show, season) => {
                    self.px.season.insert(show, season);
                }
            }
        }
    }

    // ------------------------------------------------------------- menus

    fn item_menu(&self, ui: &mut Ui, item: &Item, acts: &mut Vec<Act>) {
        let t = self.t;
        let playable = item.kind.playable() || matches!(item.kind, Kind::Show | Kind::Season | Kind::Collection | Kind::Playlist | Kind::Album | Kind::Artist);
        if playable {
            let resume = item.progress().filter(|_| !item.watched()).is_some();
            if widgets::menu_item(ui, &t, ph::PLAY, if resume { t!("Fortsetzen") } else { t!("Abspielen") }, "", false).clicked() {
                acts.push(Act::Play { item: item.clone(), start_at: resume.then(|| item.view_offset()), mode: PlayMode::Play, shuffle: false });
                ui.close();
            }
            if resume && widgets::menu_item(ui, &t, ph::SKIP_BACK, t!("Von vorne abspielen"), "", false).clicked() {
                acts.push(Act::Play { item: item.clone(), start_at: Some(0.0), mode: PlayMode::Play, shuffle: false });
                ui.close();
            }
            if !item.kind.playable() && widgets::menu_item(ui, &t, ph::SHUFFLE, t!("Zufallswiedergabe"), "", false).clicked() {
                acts.push(Act::Play { item: item.clone(), start_at: Some(0.0), mode: PlayMode::Play, shuffle: true });
                ui.close();
            }
            if widgets::menu_item(ui, &t, ph::ARROW_BEND_DOWN_RIGHT, t!("Als Nächstes abspielen"), "", false).clicked() {
                acts.push(Act::Play { item: item.clone(), start_at: None, mode: PlayMode::Next, shuffle: false });
                ui.close();
            }
            if widgets::menu_item(ui, &t, ph::QUEUE, t!("Zur Warteschlange hinzufügen"), "", false).clicked() {
                acts.push(Act::Play { item: item.clone(), start_at: None, mode: PlayMode::Enqueue, shuffle: false });
                ui.close();
            }
        }
        if matches!(item.kind, Kind::Movie | Kind::Show | Kind::Season | Kind::Episode) {
            let w = item.watched();
            if widgets::menu_item(ui, &t, if w { ph::EYE_SLASH } else { ph::EYE }, if w { t!("Als ungesehen markieren") } else { t!("Als gesehen markieren") }, "", false).clicked() {
                acts.push(Act::Watched(item.clone(), !w));
                ui.close();
            }
            // Started items can also be reset, like in the official Plex apps.
            if !w && item.progress().is_some() && widgets::menu_item(ui, &t, ph::EYE_SLASH, t!("Als ungesehen markieren"), "", false).clicked() {
                acts.push(Act::Watched(item.clone(), false));
                ui.close();
            }
        }
        if matches!(item.kind, Kind::Episode | Kind::Season) && widgets::menu_item(ui, &t, ph::TELEVISION_SIMPLE, t!("Zur Serie"), "", false).clicked() {
            acts.push(Act::ToShow(item.clone()));
            ui.close();
        }
        if matches!(item.kind, Kind::Movie | Kind::Episode) && widgets::menu_item(ui, &t, ph::INFO, "Details", "", false).clicked() {
            acts.push(Act::Go(LibLoc::Plex(PlexLoc::Item { key: item.rating_key.clone(), title: item.title.clone(), section: None })));
            ui.close();
        }
    }

    // ------------------------------------------------------------- cards

    /// Draws a card into `rect` (image + two text lines) and records clicks.
    fn plex_card(&mut self, ui: &mut Ui, rect: Rect, item: &Item, shape: CardShape, acts: &mut Vec<Act>, id: Id) {
        let t = self.t;
        let resp = ui.interact(rect, id, Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let img = Rect::from_min_size(rect.min, vec2(rect.width(), rect.width() * shape.ratio()));
        let cr = CornerRadius::same(theme::R_LG);
        let path = match shape {
            CardShape::Wide => item.still().to_string(),
            _ => item.poster().to_string(),
        };
        match self.plex.image(&path, img.width(), img.height()) {
            Some(s) => {
                ui.painter().rect_filled(img, cr, t.surface);
                paint_image(ui, img, &s, cr);
            }
            None => placeholder(ui, &t, img, item, cr),
        }
        let hovered = resp.hovered();
        // badges
        if matches!(item.kind, Kind::Show | Kind::Season) && item.leaf_count > 0 {
            if item.watched() {
                check_badge(ui, &t, img);
            } else if item.unwatched_leaves() > 0 {
                let txt = item.unwatched_leaves().to_string();
                let g = ui.painter().layout_no_wrap(txt, theme::semibold(11.5), t.on_accent);
                let bw = (g.size().x + 12.0).max(22.0);
                let b = Rect::from_min_size(pos2(img.right() - bw - 6.0, img.top() + 6.0), vec2(bw, 20.0));
                ui.painter().rect_filled(b, CornerRadius::same(10), t.accent);
                let gs = g.size();
                ui.painter().galley(b.center() - gs / 2.0, g, t.on_accent);
            }
        } else if item.kind.playable() && item.watched() {
            check_badge(ui, &t, img);
        }
        if let Some(p) = item.progress().filter(|_| !item.watched()) {
            let bar = Rect::from_min_max(pos2(img.left() + 8.0, img.bottom() - 9.0), pos2(img.right() - 8.0, img.bottom() - 5.0));
            ui.painter().rect_filled(bar, CornerRadius::same(2), Color32::from_black_alpha(150));
            let f = Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * p, bar.bottom()));
            ui.painter().rect_filled(f, CornerRadius::same(2), t.accent);
        }
        if hovered {
            ui.painter().rect_stroke(img, cr, Stroke::new(1.5, t.accent), StrokeKind::Inside);
            if item.kind.playable() {
                let c = Rect::from_center_size(img.center(), vec2(44.0, 32.0));
                ui.painter().rect_filled(c, CornerRadius::same(4), t.accent);
                ui.painter().text(c.center(), Align2::CENTER_CENTER, ph::PLAY, theme::fill_icon(16.0), t.on_accent);
            }
        }
        let tw = rect.width() - 2.0;
        widgets::painter_text_trunc(ui.painter(), pos2(rect.left() + 1.0, img.bottom() + 7.0), &item.card_title(), FontId::proportional(13.0), t.text, tw);
        widgets::painter_text_trunc(ui.painter(), pos2(rect.left() + 1.0, img.bottom() + 25.0), &item.card_sub(), FontId::proportional(11.5), t.text_faint, tw);
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked() {
            acts.push(Act::Open(item.clone()));
        }
        resp.context_menu(|ui| {
            ui.set_min_width(230.0);
            self.item_menu(ui, item, acts);
        });
    }

    /// Horizontal row of cards with a title and scroll arrows.
    fn shelf(&mut self, ui: &mut Ui, key: &str, title: &str, items: &[Item], more: Option<LibLoc>, acts: &mut Vec<Act>, compact: bool) {
        if items.is_empty() {
            return;
        }
        let items = &dedup(items)[..];
        let t = self.t;
        let wide = items.iter().filter(|i| i.kind.landscape()).count() * 2 > items.len();
        let shape = if wide { CardShape::Wide } else if items.iter().all(|i| i.kind.square()) { CardShape::Square } else { CardShape::Poster };
        let avail = ui.available_width();
        let base: f32 = match shape {
            CardShape::Wide => if compact { 200.0 } else { 268.0 },
            CardShape::Square => if compact { 120.0 } else { 160.0 },
            CardShape::Poster => if compact { 112.0 } else { 148.0 },
        };
        let cw = base.min(avail * 0.8);
        let gap = 14.0;
        let ch = cw * shape.ratio() + 46.0;
        let sid = Id::new(("plex-shelf", key));
        let content_w = items.len() as f32 * (cw + gap) - gap;
        let cur = ui.ctx().data(|d| d.get_temp::<f32>(sid.with("off"))).unwrap_or(0.0);
        let max_off = (content_w - avail).max(0.0);
        let mut scroll_to: Option<f32> = None;
        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            ui.label(RichText::new(title).font(theme::semibold(16.0)).color(t.text));
            if let Some(loc) = &more {
                ui.add_space(2.0);
                if link(ui, &t, &tf!("Alle anzeigen {}", ph::CARET_RIGHT)).clicked() {
                    acts.push(Act::Go(loc.clone()));
                }
            }
            if max_off > 1.0 && !crate::mobile::touch() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if ui.add_enabled_ui(cur < max_off - 1.0, |ui| icon_button(ui, &t, ph::CARET_RIGHT, "", false)).inner.clicked() {
                        scroll_to = Some((cur + avail * 0.85).min(max_off));
                    }
                    if ui.add_enabled_ui(cur > 1.0, |ui| icon_button(ui, &t, ph::CARET_LEFT, "", false)).inner.clicked() {
                        scroll_to = Some((cur - avail * 0.85).max(0.0));
                    }
                });
            }
        });
        ui.add_space(4.0);
        let st = self.px.shelves.entry(sid).or_default();
        if let Some(x) = scroll_to {
            st.target = x;
            st.animating = true;
        }
        let mut sa = egui::ScrollArea::horizontal().id_salt(sid).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).auto_shrink([false, true]);
        if st.animating {
            let x = ui.ctx().animate_value_with_time(sid.with("anim"), st.target, 0.28);
            if (x - st.target).abs() < 0.5 {
                st.animating = false;
            }
            sa = sa.horizontal_scroll_offset(x);
        } else {
            ui.ctx().animate_value_with_time(sid.with("anim"), cur, 0.0);
        }
        let out = sa.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (i, it) in items.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(vec2(cw, ch), Sense::hover());
                    self.plex_card(ui, r, it, shape, acts, sid.with(i));
                }
            });
        });
        let off = out.state.offset.x;
        ui.ctx().data_mut(|d| d.insert_temp(sid.with("off"), off));
        ui.add_space(14.0);
    }

    /// Wrapping grid for a fully loaded list.
    fn item_grid(&mut self, ui: &mut Ui, id: &str, items: &[Item], acts: &mut Vec<Act>, compact: bool) {
        let shape = items.first().map(CardShape::for_item).unwrap_or(CardShape::Poster);
        let (cols, cw, gap) = grid_metrics(ui.available_width(), shape, compact);
        let ch = cw * shape.ratio() + 46.0;
        for (ri, row) in items.chunks(cols).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (ci, it) in row.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(vec2(cw, ch), Sense::hover());
                    self.plex_card(ui, r, it, shape, acts, Id::new(("plex-grid", id, ri * cols + ci)));
                }
            });
            ui.add_space(gap * 0.6);
        }
    }

    /// Virtualized grid over a paged server list, with an optional A–Z bar.
    fn paged_grid(&mut self, ui: &mut Ui, path: &str, shape: CardShape, chars: Option<&[(String, i64)]>, acts: &mut Vec<Act>, compact: bool) {
        let t = self.t;
        let total = match self.plex.paged_total(path) {
            St::Ready(n) => n.max(0) as usize,
            St::Loading => return spinner(ui, &t),
            St::Failed(e) => {
                self.plex_error(ui, &e, Some(path));
                return;
            }
        };
        if total == 0 {
            empty_state(ui, &t, ph::MAGNIFYING_GLASS, t!("Keine Einträge"), t!("Mit den aktuellen Filtern wurde nichts gefunden."));
            return;
        }
        let bar_w = if chars.is_some() && !compact { 22.0 } else { 0.0 };
        let full = ui.available_rect_before_wrap();
        let grid_w = full.width() - bar_w - if bar_w > 0.0 { 6.0 } else { 0.0 };
        let (cols, cw, gap) = grid_metrics(grid_w - 10.0, shape, compact);
        let row_h = cw * shape.ratio() + 46.0 + gap * 0.6;
        let rows = total.div_ceil(cols);
        let sid = Id::new(("plex-paged", path));
        let mut sa = egui::ScrollArea::vertical().id_salt(sid).auto_shrink([false, false]);
        if let Some((p, y)) = self.px.jump.take() {
            if p == path {
                sa = sa.vertical_scroll_offset(y);
            } else {
                self.px.jump = Some((p, y));
            }
        }
        let grid_rect = Rect::from_min_size(full.min, vec2(grid_w, full.height()));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(grid_rect));
        let mut first_visible_row = 0usize;
        sa.show_viewport(&mut child, |ui, vp| {
            ui.set_height(rows as f32 * row_h);
            ui.set_width(grid_w - 10.0);
            let origin = ui.min_rect().min;
            let r0 = (vp.min.y / row_h).floor().max(0.0) as usize;
            let r1 = ((vp.max.y / row_h).ceil() as usize + 1).min(rows);
            first_visible_row = r0;
            for r in r0..r1 {
                for c in 0..cols {
                    let idx = r * cols + c;
                    if idx >= total {
                        break;
                    }
                    let rect = Rect::from_min_size(origin + vec2(c as f32 * (cw + gap), r as f32 * row_h), vec2(cw, row_h - gap * 0.6));
                    match self.plex.paged_item(path, idx) {
                        Some(it) => self.plex_card(ui, rect, &it, shape, acts, Id::new(("plex-paged-card", path, idx))),
                        None => {
                            let img = Rect::from_min_size(rect.min, vec2(cw, cw * shape.ratio()));
                            ui.painter().rect_filled(img, CornerRadius::same(theme::R_LG), t.surface);
                        }
                    }
                }
            }
            // Prefetch the page after the visible one.
            let _ = self.plex.paged_item(path, ((r1 + 2) * cols).min(total.saturating_sub(1)));
        });
        ui.advance_cursor_after_rect(grid_rect);
        // A–Z bar
        if let (Some(chars), true) = (chars, bar_w > 0.0) {
            let bar = Rect::from_min_size(pos2(full.right() - bar_w, full.top()), vec2(bar_w, full.height()));
            // Offsets must count every group, but only letters the font can show get a button.
            let shown = |l: &str| l.chars().all(|c| c.is_ascii_alphanumeric() || c == '#' || "ÄÖÜ".contains(c));
            let letters: Vec<&(String, i64)> = chars.iter().filter(|c| c.1 > 0).collect();
            let visible = letters.iter().filter(|c| shown(&c.0)).count();
            if visible > 0 {
                let step = (bar.height() / visible as f32).min(22.0);
                let mut acc = 0i64;
                let mut slot = 0usize;
                let current_idx = first_visible_row * cols;
                for (i, (label, n)) in letters.iter().enumerate() {
                    if !shown(label) {
                        acc += *n;
                        continue;
                    }
                    let r = Rect::from_min_size(pos2(bar.left(), bar.top() + slot as f32 * step), vec2(bar_w, step));
                    slot += 1;
                    let resp = ui.interact(r, Id::new(("az", path, i)), Sense::click());
                    let start = acc as usize;
                    let active = current_idx >= start && current_idx < start + *n as usize;
                    let col = if resp.hovered() || active { t.accent } else { t.text_faint };
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(11.0), col);
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        let row = start / cols;
                        self.px.jump = Some((path.to_string(), row as f32 * row_h));
                    }
                    acc += *n;
                }
            }
        }
    }

    fn plex_error(&mut self, ui: &mut Ui, e: &str, retry_list: Option<&str>) {
        let t = self.t;
        empty_state(ui, &t, ph::WARNING_CIRCLE, t!("Laden fehlgeschlagen"), e);
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            if widgets::ghost_button(ui, &t, t!("Erneut versuchen")).clicked() {
                if let Some(p) = retry_list {
                    self.plex.retry_list(p);
                }
                self.plex.clear_cache();
            }
        });
    }

    // ------------------------------------------------------------- pages

    /// Entry point from the library: draws the Plex page for `loc`.
    pub fn plex_page(&mut self, ui: &mut Ui, loc: PlexLoc, compact: bool) {
        if !self.plex.ready() {
            self.plex_connect_page(ui);
            return;
        }
        if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1)) {
            self.lib.go_back();
        }
        if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra2)) {
            self.lib.go_fwd();
        }
        let mut acts = Vec::new();
        let mut section_ctx = None;
        match &loc {
            PlexLoc::Home => self.plex_home(ui, &mut acts, compact),
            PlexLoc::Section { key, title, tab } => {
                section_ctx = Some((key.clone(), title.clone()));
                self.plex_section(ui, key, title, *tab, &mut acts, compact);
            }
            PlexLoc::Item { key, section, .. } => {
                section_ctx = section.clone();
                self.plex_item(ui, key, &mut acts, compact);
            }
            PlexLoc::Hub { key, .. } => {
                let shape = if key.contains("type=4") || key.contains("continue") || key.contains("onDeck") || key.contains("ondeck") { CardShape::Wide } else { CardShape::Poster };
                self.paged_grid(ui, key, shape, None, &mut acts, compact);
            }
            PlexLoc::Search(q) => self.plex_search(ui, q, &mut acts, compact),
        }
        self.plex_handle(acts, section_ctx);
    }

    fn plex_connect_page(&mut self, ui: &mut Ui) {
        let t = self.t;
        let status = self.plex.status.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space((ui.available_height() * 0.12).clamp(20.0, 90.0));
            ui.vertical_centered(|ui| {
                ui.set_max_width(440.0);
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(16), t.accent_soft);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, ph::TELEVISION_SIMPLE, theme::icon(32.0), t.accent);
                ui.add_space(14.0);
                match &status {
                    Status::SignedOut => {
                        ui.label(RichText::new(t!("Plex verbinden")).font(theme::semibold(22.0)).color(t.text));
                        ui.add_space(6.0);
                        ui.label(RichText::new(t!("Durchstöbere deine Plex-Mediatheken direkt in Velour – mit Sammlungen, Serien, Fortschritt und allem, was Velour beim Abspielen kann.")).color(t.text_dim));
                        ui.add_space(4.0);
                        ui.label(RichText::new(t!("Die Anmeldung läuft über plex.tv in deinem Browser. Velour sieht dein Passwort nie.")).size(12.5).color(t.text_faint));
                        ui.add_space(18.0);
                        if widgets::accent_button(ui, &t, t!("Mit Plex anmelden")).clicked() {
                            self.plex.start_login();
                        }
                    }
                    Status::Pin { url, .. } => {
                        ui.label(RichText::new(t!("Im Browser bestätigen")).font(theme::semibold(22.0)).color(t.text));
                        ui.add_space(6.0);
                        ui.label(RichText::new(t!("In deinem Browser hat sich plex.tv geöffnet. Melde dich dort an – Velour verbindet sich danach automatisch.")).color(t.text_dim));
                        ui.add_space(16.0);
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(16.0).color(t.accent));
                            ui.label(RichText::new(t!("Warte auf Bestätigung…")).color(t.text_dim));
                        });
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            if widgets::ghost_button(ui, &t, t!("Browser erneut öffnen")).clicked() {
                                plex::open_browser(url);
                            }
                            if widgets::ghost_button(ui, &t, t!("Abbrechen")).clicked() {
                                self.plex.cancel_login();
                            }
                        });
                    }
                    Status::Connecting(msg) => {
                        ui.label(RichText::new(msg).font(theme::semibold(18.0)).color(t.text));
                        ui.add_space(14.0);
                        ui.add(egui::Spinner::new().size(24.0).color(t.accent));
                    }
                    Status::Error(e) => {
                        ui.label(RichText::new(t!("Plex ist nicht erreichbar")).font(theme::semibold(20.0)).color(t.text));
                        ui.add_space(6.0);
                        ui.label(RichText::new(e).color(t.text_dim));
                        ui.add_space(16.0);
                        ui.horizontal(|ui| {
                            if self.plex.signed_in() {
                                if widgets::accent_button(ui, &t, t!("Erneut versuchen")).clicked() {
                                    let cfg = self.data.plex.clone();
                                    self.plex.retry(&cfg, self.data.settings.plex_local_only);
                                }
                                if widgets::ghost_button(ui, &t, t!("Abmelden")).clicked() {
                                    self.plex.sign_out(&mut self.data.plex);
                                    self.mark_dirty();
                                }
                            } else if widgets::accent_button(ui, &t, t!("Mit Plex anmelden")).clicked() {
                                self.plex.start_login();
                            }
                        });
                        if self.plex.servers.len() > 1 {
                            ui.add_space(18.0);
                            ui.label(RichText::new(t!("Anderen Server wählen")).size(12.0).color(t.text_faint));
                            for s in self.plex.servers.clone() {
                                let label = format!("{}{}", s.name, if s.owned { String::new() } else { tf!(" · geteilt von {}", s.owner) });
                                if widgets::menu_item(ui, &t, ph::HARD_DRIVES, &label, if s.presence { "online" } else { "offline" }, false).clicked() {
                                    self.plex.select_server(&s.id, self.data.settings.plex_local_only);
                                }
                            }
                        }
                    }
                    Status::Ready => {}
                }
                if self.plex.token_store_failed {
                    ui.add_space(14.0);
                    let note = if crate::platform::MOBILE { t!("Hinweis: Die Anmeldung konnte nicht gespeichert werden und gilt nur bis zum Beenden.") } else if cfg!(windows) { t!("Hinweis: Die Anmeldung konnte nicht im Windows-Anmeldespeicher gesichert werden und gilt nur bis zum Beenden.") } else { t!("Hinweis: Die Anmeldung konnte nicht im Schlüsselbund gesichert werden und gilt nur bis zum Beenden. Läuft GNOME Keyring oder KWallet?") };
                    ui.label(RichText::new(note).size(12.0).color(t.text_faint));
                }
            });
        });
    }

    fn plex_home(&mut self, ui: &mut Ui, acts: &mut Vec<Act>, compact: bool) {
        let t = self.t;
        let hubs = self.plex.hubs("/hubs?count=16&includeLibraryPlaylists=0");
        self.plex_search_bar(ui, acts, None, compact);
        let hubs = match hubs {
            St::Ready(h) => h,
            St::Loading => return spinner(ui, &t),
            St::Failed(e) => return self.plex_error(ui, &e, None),
        };
        let is_continue = |h: &Hub| h.identifier.contains("continue") || h.identifier.contains("ondeck") || h.identifier.contains("onDeck");
        let continue_items: Vec<Item> = continue_items(&hubs);

        // Side layout: what to continue lives in a column on the left, the
        // shelves scroll on the right – the first screen shows far more.
        if let Some(sw) = side_width(ui.available_width(), self.data.settings.plex_side_layout, compact).filter(|_| !continue_items.is_empty()) {
            let full = ui.available_rect_before_wrap();
            let left = Rect::from_min_size(full.min, vec2(sw, full.height()));
            let right = Rect::from_min_max(pos2(full.left() + sw + 28.0, full.top()), full.max);
            let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(left).layout(Layout::top_down(Align::Min)));
            self.plex_feature_column(&mut lui, &continue_items.iter().take(12).cloned().collect::<Vec<_>>(), acts);
            let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(Layout::top_down(Align::Min)));
            egui::ScrollArea::vertical().id_salt("plex-home-side").auto_shrink([false, false]).show(&mut rui, |ui| {
                ui.set_width(ui.available_width() - 8.0);
                for (i, h) in hubs.iter().enumerate().filter(|(_, h)| !is_continue(h)) {
                    let more = (h.more && !h.key.is_empty()).then(|| LibLoc::Plex(PlexLoc::Hub { key: h.key.clone(), title: h.title.clone() }));
                    self.shelf(ui, &format!("home{i}{}", h.identifier), &h.title, &h.items, more, acts, compact);
                }
            });
            ui.advance_cursor_after_rect(full);
            return;
        }

        egui::ScrollArea::vertical().id_salt("plex-home").auto_shrink([false, false]).show(ui, |ui| {
            if hubs.is_empty() {
                empty_state(ui, &t, ph::FILM_SLATE, t!("Noch nichts zu sehen"), t!("Sobald du etwas auf Plex schaust oder hinzufügst, erscheint es hier."));
            }
            // Hero: what to continue.
            let hero_items: Vec<Item> = continue_items.iter().take(6).cloned().collect();
            if !hero_items.is_empty() && !compact {
                self.plex_hero(ui, &hero_items, acts);
                ui.add_space(18.0);
            }
            // Sections as quick chips (phones: in the swipe-in drawer).
            if !self.plex.sections.is_empty() && !compact {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    for s in self.plex.sections.clone() {
                        if section_chip(ui, &t, s.icon(), &s.title).clicked() {
                            acts.push(Act::Go(LibLoc::Plex(PlexLoc::Section { key: s.key.clone(), title: s.title.clone(), tab: PlexTab::Recommended })));
                        }
                    }
                });
                ui.add_space(18.0);
            }
            for (i, h) in hubs.iter().enumerate() {
                let more = (h.more && !h.key.is_empty()).then(|| LibLoc::Plex(PlexLoc::Hub { key: h.key.clone(), title: h.title.clone() }));
                self.shelf(ui, &format!("home{i}{}", h.identifier), &h.title, &h.items, more, acts, compact);
            }
        });
    }

    fn plex_hero(&mut self, ui: &mut Ui, items: &[Item], acts: &mut Vec<Act>) {
        let t = self.t;
        let w = ui.available_width();
        let h = (w * 0.32).clamp(220.0, 380.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
        let n = items.len();
        let hovered = resp.hovered() || ui.rect_contains_pointer(rect);
        let changed = *self.px.hero_changed.get_or_insert_with(Instant::now);
        if !hovered && changed.elapsed() > Duration::from_secs(9) && n > 1 {
            self.px.hero_idx = (self.px.hero_idx + 1) % n;
            self.px.hero_changed = Some(Instant::now());
        }
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        let idx = self.px.hero_idx % n;
        let it = &items[idx];
        let cr = CornerRadius::same(theme::R_LG + 2);
        ui.painter().rect_filled(rect, cr, t.surface);
        let art = if it.kind == Kind::Episode && !it.grandparent_art.is_empty() { it.grandparent_art.clone() } else { it.backdrop().to_string() };
        if let Some(mut s) = self.plex.image(&art, w, h) {
            let since = self.px.hero_changed.map(|c| c.elapsed().as_secs_f32()).unwrap_or(1.0);
            s.alpha = s.alpha.min((since / 0.35).min(1.0));
            if s.alpha < 1.0 {
                ui.ctx().request_repaint();
            }
            paint_image(ui, rect, &s, cr);
        }
        // Darken left and bottom for legible text on any artwork.
        let dark = Color32::from_rgb(10, 10, 10);
        gradient(ui.painter(), Rect::from_min_max(rect.min, pos2(rect.left() + w * 0.62, rect.bottom())), fade(dark, 0.88), Color32::TRANSPARENT, Color32::TRANSPARENT, fade(dark, 0.88));
        gradient(ui.painter(), Rect::from_min_max(pos2(rect.left(), rect.bottom() - h * 0.45), rect.max), Color32::TRANSPARENT, Color32::TRANSPARENT, fade(dark, 0.7), fade(dark, 0.7));
        let white = Color32::from_rgb(245, 245, 245);
        let dim = Color32::from_rgb(200, 200, 200);
        let x = rect.left() + 32.0;
        let maxw = w * 0.5;
        let mut y = rect.bottom() - 34.0 - 36.0;
        // button
        let label = if it.progress().is_some() { t!("Fortsetzen") } else { t!("Abspielen") };
        let sub = it.progress().map(|_| tf!("noch {}", fmt_minutes(it.duration() - it.view_offset()))).unwrap_or_default();
        let mut play = false;
        {
            // A child Ui does not move the parent's cursor back up into the hero.
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(x, y), vec2(maxw, 36.0))));
            if play_pill(&mut child, &t, label, &sub).clicked() {
                play = true;
            }
        }
        y -= 14.0;
        let summary = wrapped(ui, &it.summary, FontId::proportional(13.0), dim, maxw, 2);
        y -= summary.size().y;
        ui.painter().galley(pos2(x, y), summary, dim);
        y -= 8.0;
        let meta = match it.kind {
            Kind::Episode => format!("{} · {}", it.episode_code(), it.title),
            _ => it.year.map(|y| y.to_string()).unwrap_or_default(),
        };
        let mg = wrapped(ui, &meta, theme::semibold(14.0), white, maxw, 1);
        y -= mg.size().y;
        ui.painter().galley(pos2(x, y), mg, white);
        y -= 4.0;
        let tg = wrapped(ui, &it.card_title(), theme::semibold(30.0), white, maxw, 2);
        y -= tg.size().y;
        ui.painter().galley(pos2(x, y), tg, white);
        if let Some(p) = it.progress() {
            let bar = Rect::from_min_size(pos2(x, rect.bottom() - 22.0), vec2(180.0, 3.0));
            ui.painter().rect_filled(bar, CornerRadius::same(2), fade(white, 0.25));
            ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(180.0 * p, 3.0)), CornerRadius::same(2), t.accent);
        }
        // dots + arrows
        if n > 1 {
            let dw = 7.0;
            let total = n as f32 * (dw + 6.0);
            let mut dx = rect.right() - 28.0 - total;
            for i in 0..n {
                let c = pos2(dx + dw / 2.0, rect.bottom() - 22.0);
                let dr = Rect::from_center_size(c, vec2(if i == idx { 18.0 } else { dw }, dw));
                ui.painter().rect_filled(dr, CornerRadius::same(4), if i == idx { t.accent } else { fade(white, 0.45) });
                if ui.interact(dr.expand(4.0), Id::new(("hero-dot", i)), Sense::click()).clicked() {
                    self.px.hero_idx = i;
                    self.px.hero_changed = Some(Instant::now());
                }
                dx += if i == idx { 18.0 + 6.0 } else { dw + 6.0 };
            }
            if hovered {
                for (dir, cx) in [(-1i32, rect.left() + 4.0 + 18.0), (1, rect.right() - 4.0 - 18.0)] {
                    let ar = Rect::from_center_size(pos2(cx, rect.center().y), vec2(34.0, 34.0));
                    let ar_resp = ui.interact(ar, Id::new(("hero-arrow", dir)), Sense::click());
                    ui.painter().circle_filled(ar.center(), 17.0, Color32::from_black_alpha(if ar_resp.hovered() { 200 } else { 130 }));
                    ui.painter().text(ar.center(), Align2::CENTER_CENTER, if dir < 0 { ph::CARET_LEFT } else { ph::CARET_RIGHT }, theme::icon(16.0), white);
                    if ar_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        self.px.hero_idx = ((idx as i32 + dir).rem_euclid(n as i32)) as usize;
                        self.px.hero_changed = Some(Instant::now());
                    }
                }
            }
        }
        // A click right after the banner rotated was aimed at the previous item.
        let settled = self.px.hero_changed.map(|c| c.elapsed() > Duration::from_millis(600)).unwrap_or(true);
        if play && settled {
            acts.push(Act::Play { item: it.clone(), start_at: it.progress().map(|_| it.view_offset()), mode: PlayMode::Play, shuffle: false });
        } else if resp.clicked() && settled {
            acts.push(if it.kind == Kind::Episode { Act::ToShow(it.clone()) } else { Act::Open(it.clone()) });
        }
        resp.context_menu(|ui| {
            ui.set_min_width(230.0);
            self.item_menu(ui, it, acts);
        });
    }

    /// Side column of the server overview: the featured "continue" item large,
    /// everything else that is in progress as a compact list underneath.
    /// Clicking a row features it; the thumbnail's play button starts it.
    fn plex_feature_column(&mut self, ui: &mut Ui, items: &[Item], acts: &mut Vec<Act>) {
        let t = self.t;
        let n = items.len();
        if n == 0 {
            return;
        }
        let idx = self.px.hero_idx % n;
        egui::ScrollArea::vertical().id_salt("plex-feature").auto_shrink([false, false]).show(ui, |ui| {
            let w = ui.available_width() - 8.0;
            ui.set_width(w);
            ui.spacing_mut().item_spacing.y = 6.0;
            let it = &items[idx];

            // Artwork: the show's backdrop for episodes reads better than a still.
            let ah = (w * 9.0 / 16.0).round();
            let (ar, aresp) = ui.allocate_exact_size(vec2(w, ah), Sense::click());
            let cr = CornerRadius::same(theme::R_LG + 2);
            ui.painter().rect_filled(ar, cr, t.surface);
            let art = if it.kind == Kind::Episode && !it.grandparent_art.is_empty() { it.grandparent_art.clone() } else { it.backdrop().to_string() };
            match self.plex.image(&art, w, ah) {
                Some(mut s) => {
                    let since = self.px.hero_changed.map(|c| c.elapsed().as_secs_f32()).unwrap_or(1.0);
                    s.alpha = s.alpha.min((since / 0.3).min(1.0));
                    if s.alpha < 1.0 {
                        ui.ctx().request_repaint();
                    }
                    paint_image(ui, ar, &s, cr);
                }
                None => placeholder(ui, &t, ar, it, cr),
            }
            let dark = Color32::from_rgb(10, 10, 10);
            gradient(ui.painter(), Rect::from_min_max(pos2(ar.left(), ar.bottom() - ah * 0.35), ar.max), Color32::TRANSPARENT, Color32::TRANSPARENT, fade(dark, 0.75), fade(dark, 0.75));
            if aresp.hovered() {
                ui.painter().rect_filled(ar, cr, Color32::from_black_alpha(60));
                ui.painter().circle_filled(ar.center(), 28.0, t.accent);
                ui.painter().text(ar.center() + vec2(2.0, 0.0), Align2::CENTER_CENTER, ph::PLAY, theme::icon(24.0), t.on_accent);
            }
            if let Some(p) = it.progress() {
                let bar = Rect::from_min_max(pos2(ar.left() + 14.0, ar.bottom() - 12.0), pos2(ar.right() - 14.0, ar.bottom() - 9.0));
                ui.painter().rect_filled(bar, CornerRadius::same(2), Color32::from_white_alpha(60));
                ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * p, 3.0)), CornerRadius::same(2), t.accent);
            }
            let play_act = |it: &Item| Act::Play { item: it.clone(), start_at: it.progress().map(|_| it.view_offset()), mode: PlayMode::Play, shuffle: false };
            if aresp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                acts.push(play_act(it));
            }

            ui.add_space(8.0);
            ui.label(RichText::new(t!("WEITERSCHAUEN")).size(11.0).color(t.accent));
            let title = ui.add(egui::Label::new(RichText::new(it.card_title()).font(theme::semibold(22.0)).color(t.text)).wrap().sense(Sense::click()));
            if title.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                acts.push(if it.kind == Kind::Episode { Act::ToShow(it.clone()) } else { Act::Open(it.clone()) });
            }
            let mut meta: Vec<String> = Vec::new();
            match it.kind {
                Kind::Episode => meta.push(it.card_sub()),
                _ => {
                    if let Some(y) = it.year {
                        meta.push(y.to_string());
                    }
                    if !it.content_rating.is_empty() {
                        meta.push(content_rating(&it.content_rating));
                    }
                }
            }
            if it.progress().is_some() && it.duration_ms > 0 {
                meta.push(tf!("noch {}", fmt_minutes(it.duration() - it.view_offset())));
            }
            ui.add(egui::Label::new(RichText::new(meta.join("  ·  ")).size(13.0).color(t.text_dim)).wrap());
            if !it.summary.is_empty() {
                let g = wrapped(ui, &it.summary, FontId::proportional(13.5), t.text_dim, w, 5);
                let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                ui.painter().galley(r.min, g, t.text_dim);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let label = if it.progress().is_some() { t!("Fortsetzen") } else { t!("Abspielen") };
                if play_pill(ui, &t, label, "").clicked() {
                    acts.push(play_act(it));
                }
                let details = if it.kind == Kind::Episode { t!("Zur Serie") } else { "Details" };
                if action_icon(ui, &t, ph::INFO, details, false).clicked() {
                    acts.push(if it.kind == Kind::Episode { Act::ToShow(it.clone()) } else { Act::Open(it.clone()) });
                }
                let more = action_icon(ui, &t, ph::DOTS_THREE, t!("Mehr"), false);
                egui::Popup::menu(&more).width(240.0).show(|ui| self.item_menu(ui, it, acts));
            });

            if n > 1 {
                ui.add_space(20.0);
                ui.label(RichText::new(t!("Ebenfalls angefangen")).font(theme::semibold(14.5)).color(t.text));
                ui.add_space(2.0);
                for (i, o) in items.iter().enumerate() {
                    if i == idx {
                        continue;
                    }
                    let (row, resp) = ui.allocate_exact_size(vec2(w, 66.0), Sense::click());
                    let hovered = resp.hovered();
                    if hovered {
                        ui.painter().rect_filled(row, CornerRadius::same(theme::R), t.hover);
                    }
                    let thumb = Rect::from_min_size(pos2(row.left() + 6.0, row.top() + 6.0), vec2(96.0, 54.0));
                    let tcr = CornerRadius::same(theme::R);
                    match self.plex.image(o.still(), thumb.width(), thumb.height()) {
                        Some(s) => paint_image(ui, thumb, &s, tcr),
                        None => placeholder(ui, &t, thumb, o, tcr),
                    }
                    if let Some(p) = o.progress() {
                        let bar = Rect::from_min_max(pos2(thumb.left() + 5.0, thumb.bottom() - 6.0), pos2(thumb.right() - 5.0, thumb.bottom() - 3.0));
                        ui.painter().rect_filled(bar, CornerRadius::same(2), Color32::from_black_alpha(150));
                        ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * p, 3.0)), CornerRadius::same(2), t.accent);
                    }
                    let play_resp = ui.interact(thumb, Id::new(("feature-play", &o.rating_key)), Sense::click());
                    if hovered || play_resp.hovered() {
                        ui.painter().rect_filled(thumb, tcr, Color32::from_black_alpha(90));
                        ui.painter().circle_filled(thumb.center(), 14.0, if play_resp.hovered() { t.accent } else { Color32::from_black_alpha(160) });
                        ui.painter().text(thumb.center() + vec2(1.0, 0.0), Align2::CENTER_CENTER, ph::PLAY, theme::icon(13.0), Color32::WHITE);
                    }
                    let tx = thumb.right() + 12.0;
                    let tw = row.right() - tx - 8.0;
                    widgets::painter_text_trunc(ui.painter(), pos2(tx, row.top() + 14.0), &o.card_title(), theme::semibold(13.5), t.text, tw);
                    let sub = if o.progress().is_some() && o.duration_ms > 0 {
                        let left = tf!("noch {}", fmt_minutes(o.duration() - o.view_offset()));
                        if o.kind == Kind::Episode { format!("{}  ·  {left}", o.episode_code()) } else { left }
                    } else {
                        o.card_sub()
                    };
                    widgets::painter_text_trunc(ui.painter(), pos2(tx, row.top() + 36.0), &sub, FontId::proportional(12.0), t.text_faint, tw);
                    if play_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        acts.push(play_act(o));
                    } else if resp.clicked() {
                        self.px.hero_idx = i;
                        self.px.hero_changed = Some(Instant::now());
                    }
                    resp.on_hover_cursor(CursorIcon::PointingHand).context_menu(|ui| {
                        ui.set_min_width(230.0);
                        self.item_menu(ui, o, acts);
                    });
                }
            }
            ui.add_space(20.0);
        });
    }

    /// Search row shared by the server overview and the results page. Results
    /// update live while typing (debounced); the field keeps its id across both
    /// pages so focus and caret survive the switch.
    fn plex_search_bar(&mut self, ui: &mut Ui, acts: &mut Vec<Act>, current: Option<&str>, compact: bool) {
        let t = self.t;
        let id = Id::new("plex-search-field");
        // Mirror the location into the field when arriving via history/back.
        match current {
            Some(q) if self.px.search_synced.as_deref() != Some(q) => {
                self.px.home_search = q.to_string();
                self.px.search_synced = Some(q.to_string());
                self.px.search_typed = None;
            }
            None if self.px.search_synced.is_some() && !ui.memory(|m| m.has_focus(id)) => {
                self.px.home_search.clear();
                self.px.search_synced = None;
            }
            _ => {}
        }
        ui.horizontal(|ui| {
            // Phones: button for the libraries drawer.
            if compact {
                // Finger-sized target: the whole square, not just the icon.
                let size = crate::mobile::m().icon_btn.max(40.0);
                let (r, b) = ui.allocate_exact_size(vec2(size, size), Sense::click());
                let b = b.on_hover_text(t!("Mediatheken"));
                // The edge-swipe strip starts below this, so it does not
                // swallow taps on the button.
                self.mob.drawer.button = Some(r);
                if b.is_pointer_button_down_on() || b.hovered() || self.mob.drawer.is_open() {
                    ui.painter().rect_filled(r, CornerRadius::same(theme::R + 2), t.hover);
                }
                ui.painter().text(r.center(), Align2::CENTER_CENTER, ph::LIST, theme::icon(20.0), t.text);
                // Right after the swipe hint: light up once, so it is clear
                // the button opens the same drawer.
                if let Some(k) = self.mob.drawer.pulse() {
                    let c = b.rect.center();
                    let wave = (k * std::f32::consts::PI).sin();
                    ui.painter().circle_filled(c, b.rect.width() * 0.5 + 2.0, t.accent.gamma_multiply(0.35 * wave));
                    ui.painter().circle_stroke(c, b.rect.width() * (0.5 + 0.5 * k), Stroke::new(2.0, t.accent.gamma_multiply(1.0 - k)));
                    ui.painter().text(c, Align2::CENTER_CENTER, ph::LIST, theme::icon(18.0), t.accent.gamma_multiply(wave.max(0.0)));
                    ui.ctx().request_repaint();
                }
                if b.clicked() {
                    self.mob.drawer.toggle();
                    self.data.settings.plex_drawer_hint_seen = true;
                }
                ui.add_space(4.0);
            }
            let avail = ui.available_width() - 40.0;
            let w = if compact { avail } else { avail.min(560.0) };
            let out = widgets::search_field(ui, &t, id, &mut self.px.home_search, t!("Filme, Serien, Personen suchen…"), w);
            if out.changed {
                self.px.search_typed = Some(Instant::now());
            }
            let q = self.px.home_search.trim().to_string();
            let mut go = out.submitted;
            if let Some(at) = self.px.search_typed {
                if at.elapsed() >= Duration::from_millis(350) {
                    go = true;
                } else {
                    ui.ctx().request_repaint_after(Duration::from_millis(60));
                }
            }
            if go {
                self.px.search_typed = None;
                if q.chars().count() >= 2 && current != Some(q.as_str()) {
                    self.px.search_synced = Some(q.clone());
                    let loc = LibLoc::Plex(PlexLoc::Search(q));
                    // Refining a query replaces the page instead of piling up history.
                    if current.is_some() {
                        self.lib.loc = loc;
                    } else {
                        acts.push(Act::Go(loc));
                    }
                } else if q.is_empty() && current.is_some() {
                    self.px.search_synced = None;
                    self.lib.loc = LibLoc::Plex(PlexLoc::Home);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, &t, ph::ARROWS_CLOCKWISE, t!("Aktualisieren"), false).clicked() {
                    self.plex.clear_cache();
                }
            });
        });
        ui.add_space(14.0);
    }

    fn plex_search(&mut self, ui: &mut Ui, query: &str, acts: &mut Vec<Act>, compact: bool) {
        let t = self.t;
        self.plex_search_bar(ui, acts, Some(query), compact);
        let path = format!("/hubs/search?query={}&limit=20&includeCollections=1", plex::api::url_encode(query));
        let hubs = match self.plex.hubs(&path) {
            St::Ready(h) => h,
            St::Loading => return spinner(ui, &t),
            St::Failed(e) => return self.plex_error(ui, &e, None),
        };
        egui::ScrollArea::vertical().id_salt(("plex-search", query)).auto_shrink([false, false]).show(ui, |ui| {
            let hubs: Vec<&Hub> = hubs.iter().filter(|h| h.items.iter().any(|i| i.kind != Kind::Other)).collect();
            if hubs.is_empty() {
                empty_state(ui, &t, ph::MAGNIFYING_GLASS, t!("Keine Treffer"), &tf!("Nichts gefunden für „{query}“."));
            }
            for (i, h) in hubs.iter().enumerate() {
                let items: Vec<Item> = h.items.iter().filter(|i| i.kind != Kind::Other).cloned().collect();
                self.shelf(ui, &format!("search{i}{query}"), &h.title, &items, None, acts, compact);
            }
        });
    }

    // ----------------------------------------------------------- section

    fn plex_section(&mut self, ui: &mut Ui, key: &str, title: &str, tab: PlexTab, acts: &mut Vec<Act>, compact: bool) {
        let t = self.t;
        let section = self.plex.sections.iter().find(|s| s.key == key).cloned().unwrap_or_default();
        let ty = section.type_num();
        let mut b = self.px.browse.entry(key.to_string()).or_default().clone();
        // Debounced search.
        if let Some(at) = b.typed_at {
            if at.elapsed() > Duration::from_millis(350) {
                b.applied = b.search.trim().to_string();
                b.typed_at = None;
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(120));
            }
        }
        let mut new_tab = tab;
        // Tabs + tools
        ui.horizontal(|ui| {
            let tabs_w = ui.available_width().min(if compact { ui.available_width() } else { 470.0 });
            ui.allocate_ui(vec2(tabs_w, 38.0), |ui| {
                widgets::tabs(
                    ui,
                    &t,
                    &mut new_tab,
                    &[
                        (PlexTab::Recommended, ph::SPARKLE, t!("Empfohlen")),
                        (PlexTab::Library, ph::SQUARES_FOUR, t!("Bibliothek")),
                        (PlexTab::Collections, ph::STACK, t!("Sammlungen")),
                        (PlexTab::Playlists, ph::PLAYLIST, "Playlists"),
                    ],
                );
            });
            if tab == PlexTab::Library && !compact {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if icon_button(ui, &t, ph::MAGNIFYING_GLASS, t!("In dieser Mediathek suchen"), b.search_open).clicked() {
                        b.search_open = !b.search_open;
                        if b.search_open {
                            ui.memory_mut(|m| m.request_focus(Id::new(("plex-section-search", key))));
                        } else {
                            b.search.clear();
                            b.applied.clear();
                        }
                    }
                    if icon_button(ui, &t, if b.unwatched { ph::EYE_SLASH } else { ph::EYE }, if b.unwatched { t!("Nur ungesehene (aktiv)") } else { t!("Nur ungesehene zeigen") }, b.unwatched).clicked() {
                        b.unwatched = !b.unwatched;
                    }
                    // genre filter
                    let genres = self.plex.values(&format!("/library/sections/{key}/genre?type={ty}"));
                    ui.add_space(6.0);
                    let gl = b.genre.as_ref().map(|g| g.0.clone()).unwrap_or_else(|| t!("Alle Genres").into());
                    let gr = tool_chip(ui, &t, ph::TAG, &gl, b.genre.is_some());
                    egui::Popup::menu(&gr).width(220.0).show(|ui| {
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            if widgets::menu_item(ui, &t, "", t!("Alle Genres"), "", b.genre.is_none()).clicked() {
                                b.genre = None;
                                ui.close();
                            }
                            if let St::Ready(g) = &genres {
                                for (name, k) in g.iter() {
                                    let sel = b.genre.as_ref().map(|x| &x.1 == k).unwrap_or(false);
                                    if widgets::menu_item(ui, &t, "", name, "", sel).clicked() {
                                        b.genre = Some((name.clone(), k.clone()));
                                        ui.close();
                                    }
                                }
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let sr = tool_chip(ui, &t, if b.desc { ph::SORT_DESCENDING } else { ph::SORT_ASCENDING }, b.sort.label(), b.sort != SortKey::Title || b.desc);
                    egui::Popup::menu(&sr).width(220.0).show(|ui| {
                        for s in SortKey::ALL {
                            if widgets::menu_item(ui, &t, "", s.label(), "", b.sort == s).clicked() {
                                if b.sort == s {
                                    b.desc = !b.desc;
                                } else {
                                    b.sort = s;
                                    b.desc = s.natural_desc();
                                }
                                ui.close();
                            }
                        }
                        widgets::separator(ui, &t);
                        if widgets::menu_item(ui, &t, if b.desc { ph::SORT_DESCENDING } else { ph::SORT_ASCENDING }, if b.desc { t!("Absteigend") } else { t!("Aufsteigend") }, "", false).clicked() {
                            b.desc = !b.desc;
                            ui.close();
                        }
                    });
                });
            }
        });
        if new_tab != tab {
            self.lib.loc = LibLoc::Plex(PlexLoc::Section { key: key.to_string(), title: title.to_string(), tab: new_tab });
        }
        if tab == PlexTab::Library && b.search_open {
            ui.add_space(6.0);
            let w = ui.available_width().min(560.0);
            let out = widgets::search_field(ui, &t, Id::new(("plex-section-search", key)), &mut b.search, &tf!("{} durchsuchen…", section.noun_plural()), w);
            if out.changed {
                b.typed_at = Some(Instant::now());
            }
            if out.submitted || out.cleared {
                b.applied = b.search.trim().to_string();
                b.typed_at = None;
            }
        }
        ui.add_space(8.0);
        self.px.browse.insert(key.to_string(), b.clone());

        match tab {
            PlexTab::Recommended => {
                let hubs = match self.plex.hubs(&format!("/hubs/sections/{key}?count=16&includeLibraryPlaylists=0")) {
                    St::Ready(h) => h,
                    St::Loading => return spinner(ui, &t),
                    St::Failed(e) => return self.plex_error(ui, &e, None),
                };
                egui::ScrollArea::vertical().id_salt(("plex-rec", key)).auto_shrink([false, false]).show(ui, |ui| {
                    if hubs.is_empty() {
                        empty_state(ui, &t, ph::SPARKLE, t!("Keine Empfehlungen"), t!("Plex hat für diese Mediathek noch keine Empfehlungen."));
                    }
                    for (i, h) in hubs.iter().enumerate() {
                        let more = (h.more && !h.key.is_empty()).then(|| LibLoc::Plex(PlexLoc::Hub { key: h.key.clone(), title: h.title.clone() }));
                        self.shelf(ui, &format!("rec{key}-{i}"), &h.title, &h.items, more, acts, compact);
                    }
                });
            }
            PlexTab::Library => {
                let mut path = format!("/library/sections/{key}/all?type={ty}&sort={}:{}", b.sort.field(), if b.desc { "desc" } else { "asc" });
                if b.unwatched {
                    path.push_str(if ty == 2 { "&unwatchedLeaves=1" } else { "&unwatched=1" });
                }
                if let Some((_, g)) = &b.genre {
                    path.push_str(&format!("&genre={}", plex::api::url_encode(g)));
                }
                if !b.applied.is_empty() {
                    path.push_str(&format!("&title={}", plex::api::url_encode(&b.applied)));
                }
                let plain = b.sort == SortKey::Title && !b.desc && !b.unwatched && b.genre.is_none() && b.applied.is_empty();
                let chars = if plain {
                    match self.plex.chars(&format!("/library/sections/{key}/firstCharacter?type={ty}")) {
                        St::Ready(c) => Some(c),
                        _ => None,
                    }
                } else {
                    None
                };
                let shape = if ty == 8 { CardShape::Square } else { CardShape::Poster };
                if let St::Ready(n) = self.plex.paged_total(&path) {
                    ui.label(RichText::new(format!("{} {}", fmt_count(n), section.noun_plural())).size(12.0).color(t.text_faint));
                    ui.add_space(4.0);
                }
                self.paged_grid(ui, &path, shape, chars.as_deref().map(|v| v.as_slice()), acts, compact);
            }
            PlexTab::Collections => {
                let path = format!("/library/sections/{key}/collections");
                self.paged_grid(ui, &path, CardShape::Poster, None, acts, compact);
            }
            PlexTab::Playlists => {
                let kind = if ty == 8 { "audio" } else { "video" };
                let path = format!("/playlists?playlistType={kind}&sectionID={key}");
                self.paged_grid(ui, &path, CardShape::Square, None, acts, compact);
            }
        }
    }

    // ------------------------------------------------------------- items

    fn plex_item(&mut self, ui: &mut Ui, key: &str, acts: &mut Vec<Act>, compact: bool) {
        let t = self.t;
        let item = match self.plex.meta(key) {
            St::Ready(i) => i,
            St::Loading => return spinner(ui, &t),
            St::Failed(e) => return self.plex_error(ui, &e, None),
        };
        let mut page = |app: &mut App, ui: &mut Ui| {
            egui::ScrollArea::vertical().id_salt(("plex-item", key)).auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width() - 8.0);
                match item.kind {
                    Kind::Collection | Kind::Playlist | Kind::Artist | Kind::Album => app.plex_collection(ui, &item, acts, compact),
                    // Episodes get the show page with their season selected.
                    _ => app.plex_detail(ui, &item, acts, compact),
                }
                ui.add_space(30.0);
            });
        };
        // Side layout: artwork + info in a fixed column (drawn by
        // `detail_header` into `side_col`), seasons/episodes/cast scroll right.
        if let Some(sw) = side_width(ui.available_width(), self.data.settings.plex_side_layout, compact) {
            let full = ui.available_rect_before_wrap();
            self.px.side_col = Some(Rect::from_min_size(full.min, vec2(sw, full.height())));
            let right = Rect::from_min_max(pos2(full.left() + sw + 28.0, full.top()), full.max);
            let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(Layout::top_down(Align::Min)));
            page(self, &mut rui);
            self.px.side_col = None;
            ui.advance_cursor_after_rect(full);
        } else {
            page(self, ui);
        }
    }

    /// Info column of the side layout: artwork on top, then the page's info
    /// block (title, meta, actions, summary). Scrolls on its own if it is long.
    fn side_info(&mut self, ui: &mut Ui, col: Rect, item: &Item, acts: &mut Vec<Act>, body: &mut dyn FnMut(&mut App, &mut Ui, &mut Vec<Act>)) {
        let t = self.t;
        // Drawn from inside the right column's scroll area, so take over the
        // column's own rect and clip; it does not grow the scroll content.
        let mut col_ui = ui.new_child(egui::UiBuilder::new().max_rect(col).layout(Layout::top_down(Align::Min)));
        col_ui.set_clip_rect(col);
        egui::ScrollArea::vertical().id_salt(("plex-side", &item.rating_key)).auto_shrink([false, false]).show(&mut col_ui, |ui| {
            let w = ui.available_width() - 8.0;
            ui.set_width(w);
            let cr = CornerRadius::same(theme::R_LG + 2);
            let wide = !item.art.is_empty() || (item.kind == Kind::Episode && !item.grandparent_art.is_empty());
            if item.kind.square() {
                let (r, _) = ui.allocate_exact_size(vec2(w, w), Sense::hover());
                match self.plex.image(item.poster(), w, w) {
                    Some(s) => paint_image(ui, r, &s, cr),
                    None => placeholder(ui, &t, r, item, cr),
                }
            } else if wide {
                let h = (w * 9.0 / 16.0).round();
                let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                ui.painter().rect_filled(r, cr, t.surface);
                if let Some(s) = self.plex.image(item.backdrop(), w, h) {
                    paint_image(ui, r, &s, cr);
                }
                // Poster tucked into the corner keeps the title art visible.
                if !item.poster().is_empty() && item.poster() != item.backdrop() {
                    let pw = (w * 0.24).round();
                    let pr = Rect::from_min_size(pos2(r.left() + 12.0, r.bottom() - pw * 1.5 - 12.0), vec2(pw, pw * 1.5));
                    let pcr = CornerRadius::same(theme::R);
                    ui.painter().rect_filled(pr.expand(1.5), pcr, Color32::from_black_alpha(140));
                    match self.plex.image(item.poster(), pr.width(), pr.height()) {
                        Some(s) => paint_image(ui, pr, &s, pcr),
                        None => placeholder(ui, &t, pr, item, pcr),
                    }
                }
            } else {
                let pw = (w * 0.5).round();
                let (r, _) = ui.allocate_exact_size(vec2(w, pw * 1.5), Sense::hover());
                let pr = Rect::from_min_size(r.min, vec2(pw, pw * 1.5));
                match self.plex.image(item.poster(), pr.width(), pr.height()) {
                    Some(s) => paint_image(ui, pr, &s, cr),
                    None => placeholder(ui, &t, pr, item, cr),
                }
            }
            ui.add_space(12.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            body(self, ui, acts);
            ui.add_space(24.0);
        });
    }

    /// Backdrop + poster + title block shared by detail pages. Returns the content x inset.
    fn detail_header(&mut self, ui: &mut Ui, item: &Item, compact: bool, hero: bool, acts: &mut Vec<Act>, body: &mut dyn FnMut(&mut App, &mut Ui, &mut Vec<Act>)) {
        if let Some(col) = self.px.side_col.take() {
            return self.side_info(ui, col, item, acts, body);
        }
        let t = self.t;
        let bg = if compact { t.panel } else { t.bg };
        let w = ui.available_width();
        let hero_h = if !hero { 0.0 } else if compact { (w * 0.5).clamp(140.0, 230.0) } else { (w * 0.36).clamp(230.0, 420.0) };
        let top = ui.cursor().top();
        if hero_h > 0.0 {
            let (r, _) = ui.allocate_exact_size(vec2(w, hero_h), Sense::hover());
            let cr = CornerRadius::same(theme::R_LG + 2);
            if let Some(s) = self.plex.image(item.backdrop(), w, hero_h) {
                paint_image(ui, r, &s, cr);
            } else {
                ui.painter().rect_filled(r, cr, t.surface);
            }
            gradient(ui.painter(), Rect::from_min_max(pos2(r.left(), r.top() + hero_h * 0.25), r.max), Color32::TRANSPARENT, Color32::TRANSPARENT, bg, bg);
            if !compact {
                gradient(ui.painter(), Rect::from_min_max(r.min, pos2(r.left() + w * 0.5, r.bottom())), fade(bg, 0.7), Color32::TRANSPARENT, Color32::TRANSPARENT, fade(bg, 0.7));
            }
        }
        let poster_w = if compact { 0.0 } else { (w * 0.15).clamp(130.0, 190.0) };
        let overlap = if hero_h > 0.0 { if compact { 46.0 } else { 120.0 } } else { 0.0 };
        let inset = if compact { 0.0 } else { 26.0 };
        let block_top = top + hero_h - overlap;
        let left = ui.max_rect().left() + inset;
        let mut used_bottom = block_top;
        if poster_w > 0.0 {
            let pr = Rect::from_min_size(pos2(left, block_top), vec2(poster_w, poster_w * if item.kind.square() { 1.0 } else { 1.5 }));
            let cr = CornerRadius::same(theme::R_LG);
            ui.painter().rect_filled(pr.expand(1.0), cr, t.border);
            match self.plex.image(item.poster(), pr.width(), pr.height()) {
                Some(s) => paint_image(ui, pr, &s, cr),
                None => placeholder(ui, &t, pr, item, cr),
            }
            used_bottom = pr.bottom();
        }
        let text_left = if poster_w > 0.0 { left + poster_w + 26.0 } else { left };
        let text_rect = Rect::from_min_max(pos2(text_left, block_top + if poster_w > 0.0 { overlap - 50.0 } else { 0.0 }), pos2(ui.max_rect().right() - inset, block_top + 2000.0));
        let inner = ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            body(self, ui, acts);
        });
        used_bottom = used_bottom.max(inner.response.rect.bottom());
        let cur = ui.cursor().top();
        if used_bottom > cur {
            ui.add_space(used_bottom - cur);
        }
        ui.add_space(18.0);
    }

    fn plex_detail(&mut self, ui: &mut Ui, item: &Arc<Item>, acts: &mut Vec<Act>, compact: bool) {
        let t = self.t;
        let side = self.px.side_col.is_some();
        let it = item.as_ref().clone();
        let show_key = match it.kind {
            Kind::Show => Some(it.rating_key.clone()),
            Kind::Episode => Some(it.grandparent_key.clone()),
            _ => None,
        };
        if it.kind == Kind::Episode {
            if let Some(sk) = &show_key {
                if !it.parent_key.is_empty() {
                    self.px.season.entry(sk.clone()).or_insert(it.parent_key.clone());
                }
            }
        }
        // Seasons (shows) for the play button and the episode list.
        let seasons: Option<Arc<Vec<Item>>> = match &show_key {
            Some(k) if it.kind == Kind::Show => match self.plex.items(&format!("/library/metadata/{k}/children?excludeAllLeaves=1")) {
                St::Ready(s) => Some(s),
                _ => None,
            },
            _ => None,
        };
        let expanded = self.px.expanded.contains(&it.rating_key);
        let mut toggle_expand = false;
        let width = ui.available_width();
        self.detail_header(ui, &it, compact, true, acts, &mut |app, ui, acts| {
            let t = app.t;
            if it.kind == Kind::Episode {
                if link(ui, &t, &format!("{}  {}", it.grandparent_title, ph::CARET_RIGHT)).clicked() {
                    acts.push(Act::ToShow(it.clone()));
                }
            }
            let title_size = if compact { 22.0 } else if it.title.len() > 34 { 26.0 } else { 32.0 };
            ui.add(egui::Label::new(RichText::new(&it.title).font(theme::semibold(title_size)).color(t.text)).wrap());
            if !it.original_title.is_empty() && it.original_title != it.title {
                ui.label(RichText::new(&it.original_title).size(13.0).color(t.text_faint));
            }
            // meta line
            let mut meta: Vec<String> = Vec::new();
            if it.kind == Kind::Episode {
                meta.push(it.episode_code());
            }
            if let Some(y) = it.year {
                meta.push(y.to_string());
            }
            if !it.content_rating.is_empty() {
                meta.push(content_rating(&it.content_rating));
            }
            if it.duration_ms > 0 {
                meta.push(fmt_minutes(it.duration()));
            }
            if it.kind == Kind::Show && it.child_count > 0 {
                meta.push(if it.child_count == 1 { t!("1 Staffel").into() } else { tf!("{} Staffeln", it.child_count) });
            }
            if !it.genres.is_empty() {
                meta.push(it.genres.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
            }
            ui.label(RichText::new(meta.join("  ·  ")).size(13.5).color(t.text_dim));
            // chips: quality + ratings
            let ratings: Vec<(String, &str)> = it.ratings.iter().filter_map(rating_label).collect();
            let quality = it.media.first().map(|m| m.label()).unwrap_or_default();
            if !ratings.is_empty() || !quality.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    if !quality.is_empty() {
                        chip(ui, &t, ph::MONITOR_PLAY, &quality);
                    }
                    for (l, i) in &ratings {
                        chip(ui, &t, i, l);
                    }
                });
            }
            ui.add_space(6.0);
            // actions (wrap in the narrow side column instead of widening it)
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                match it.kind {
                    Kind::Show => {
                        let deck = it.on_deck.as_deref().cloned();
                        let (label, sub, target, at) = match &deck {
                            Some(ep) if ep.progress().is_some() => (tf!("{} fortsetzen", ep.episode_code()), tf!("noch {}", fmt_minutes(ep.duration() - ep.view_offset())), Some(ep.clone()), Some(ep.view_offset())),
                            Some(ep) => (tf!("{} abspielen", ep.episode_code()), String::new(), Some(ep.clone()), Some(0.0)),
                            None if it.watched() => (t!("Erneut ansehen").into(), String::new(), None, Some(0.0)),
                            None => (t!("Abspielen").into(), String::new(), None, Some(0.0)),
                        };
                        if play_pill(ui, &t, &label, &sub).clicked() {
                            match target {
                                Some(ep) => acts.push(Act::Play { item: ep, start_at: at, mode: PlayMode::Play, shuffle: false }),
                                None => {
                                    // First regular season from the start.
                                    let first = seasons.as_ref().and_then(|s| s.iter().find(|x| x.index.unwrap_or(0) > 0).or(s.first()).cloned());
                                    if let Some(s) = first {
                                        acts.push(Act::Play { item: s, start_at: at, mode: PlayMode::Play, shuffle: false });
                                    } else {
                                        acts.push(Act::Play { item: it.clone(), start_at: at, mode: PlayMode::Play, shuffle: false });
                                    }
                                }
                            }
                        }
                        if side {
                            ui.end_row();
                        }
                        if action_icon(ui, &t, ph::SHUFFLE, t!("Zufallswiedergabe"), false).clicked() {
                            acts.push(Act::Play { item: it.clone(), start_at: Some(0.0), mode: PlayMode::Play, shuffle: true });
                        }
                    }
                    _ => {
                        let resume = it.progress().filter(|_| !it.watched()).is_some();
                        let sub = if resume { tf!("noch {}", fmt_minutes(it.duration() - it.view_offset())) } else { String::new() };
                        if play_pill(ui, &t, if resume { t!("Fortsetzen") } else { t!("Abspielen") }, &sub).clicked() {
                            acts.push(Act::Play { item: it.clone(), start_at: Some(if resume { it.view_offset() } else { 0.0 }), mode: PlayMode::Play, shuffle: false });
                        }
                        if side {
                            ui.end_row();
                        }
                        if resume && action_icon(ui, &t, ph::SKIP_BACK, t!("Von vorne abspielen"), false).clicked() {
                            acts.push(Act::Play { item: it.clone(), start_at: Some(0.0), mode: PlayMode::Play, shuffle: false });
                        }
                    }
                }
                let w = it.watched();
                if action_icon(ui, &t, if w { ph::CHECK_CIRCLE } else { ph::CHECK }, if w { t!("Als ungesehen markieren") } else { t!("Als gesehen markieren") }, w).clicked() {
                    acts.push(Act::Watched(it.clone(), !w));
                }
                if action_icon(ui, &t, ph::QUEUE, t!("Zur Warteschlange hinzufügen"), false).clicked() {
                    acts.push(Act::Play { item: it.clone(), start_at: None, mode: PlayMode::Enqueue, shuffle: false });
                }
                let more = action_icon(ui, &t, ph::DOTS_THREE, t!("Mehr"), false);
                egui::Popup::menu(&more).width(240.0).show(|ui| {
                    if widgets::menu_item(ui, &t, ph::ARROW_BEND_DOWN_RIGHT, t!("Als Nächstes abspielen"), "", false).clicked() {
                        acts.push(Act::Play { item: it.clone(), start_at: None, mode: PlayMode::Next, shuffle: false });
                        ui.close();
                    }
                    if let Some(s) = &app.plex.server {
                        let url = format!("https://app.plex.tv/desktop/#!/server/{}/details?key=%2Flibrary%2Fmetadata%2F{}", s.id, it.rating_key);
                        if widgets::menu_item(ui, &t, ph::ARROW_SQUARE_OUT, t!("In Plex Web öffnen"), "", false).clicked() {
                            plex::open_browser(&url);
                            ui.close();
                        }
                    }
                });
            });
            if !it.tagline.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(&it.tagline).italics().color(t.text_dim));
            }
            if !it.summary.is_empty() {
                ui.add_space(4.0);
                let maxw = ui.available_width().min(880.0);
                let collapsed = if side { 7 } else { 4 };
                let rows = if expanded { 100 } else { collapsed };
                let g = wrapped(ui, &it.summary, FontId::proportional(14.0), t.text_dim, maxw, rows);
                let elided = g.rows.len() >= collapsed && !expanded && g.rows.last().map(|r| r.ends_with_newline == false).unwrap_or(false);
                let (r, resp) = ui.allocate_exact_size(g.size(), Sense::click());
                ui.painter().galley(r.min, g, t.text_dim);
                if elided || expanded {
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        toggle_expand = true;
                    }
                    if link(ui, &t, if expanded { t!("Weniger") } else { t!("Mehr lesen") }).clicked() {
                        toggle_expand = true;
                    }
                }
            }
        });
        if toggle_expand {
            if !self.px.expanded.remove(&it.rating_key) {
                self.px.expanded.insert(it.rating_key.clone());
            }
        }
        let pad = if compact || side { 0.0 } else { 26.0 };
        let content = |ui: &mut Ui, f: &mut dyn FnMut(&mut Ui)| {
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.vertical(|ui| {
                    ui.set_width(width - pad * 2.0);
                    f(ui);
                });
            });
        };
        // Seasons + episodes
        if let (Some(show), Some(seasons)) = (show_key.clone(), seasons.clone()) {
            if !seasons.is_empty() {
                let sel = self
                    .px
                    .season
                    .get(&show)
                    .filter(|k| seasons.iter().any(|s| &s.rating_key == *k))
                    .cloned()
                    .or_else(|| it.on_deck.as_ref().map(|e| e.parent_key.clone()).filter(|k| seasons.iter().any(|s| &s.rating_key == k)))
                    .or_else(|| seasons.iter().find(|s| s.index.unwrap_or(0) > 0).map(|s| s.rating_key.clone()))
                    .unwrap_or_else(|| seasons[0].rating_key.clone());
                let mut acts2 = Vec::new();
                content(ui, &mut |ui| {
                    ui.label(RichText::new(t!("Episoden")).font(theme::semibold(18.0)).color(t.text));
                    ui.add_space(6.0);
                    if seasons.len() > 1 {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                            for s in seasons.iter() {
                                let mut label = s.title.clone();
                                if s.leaf_count > 0 && s.watched() {
                                    label = format!("{label}  {}", ph::CHECK);
                                }
                                if pill(ui, &t, &label, s.rating_key == sel).clicked() {
                                    acts2.push(Act::Season(show.clone(), s.rating_key.clone()));
                                }
                            }
                        });
                        ui.add_space(8.0);
                    }
                });
                acts.extend(acts2);
                let eps = self.plex.items(&format!("/library/metadata/{sel}/children"));
                match eps {
                    St::Ready(eps) => {
                        let mut acts3 = Vec::new();
                        content(ui, &mut |ui| {
                            for e in eps.iter() {
                                self.episode_row(ui, e, &mut acts3, compact);
                            }
                        });
                        acts.extend(acts3);
                    }
                    St::Loading => spinner(ui, &t),
                    St::Failed(e) => {
                        ui.label(RichText::new(e).color(t.text_dim));
                    }
                }
                ui.add_space(18.0);
            }
        }
        // Technical details (movies, episodes)
        if it.kind.playable() {
            if let Some(m) = it.media.first() {
                let part = m.parts.first();
                let audio: Vec<String> = part
                    .map(|p| p.streams.iter().filter(|s| s.stream_type == 2).map(|s| if s.display.is_empty() { s.language.clone() } else { s.display.clone() }).collect())
                    .unwrap_or_default();
                let subs: Vec<String> = part
                    .map(|p| p.streams.iter().filter(|s| s.stream_type == 3).map(|s| if s.display.is_empty() { s.language.clone() } else { s.display.clone() }).collect())
                    .unwrap_or_default();
                let mut file = String::new();
                if let Some(p) = part {
                    let name = p.file.rsplit(['/', '\\']).next().unwrap_or("").to_string();
                    file = if p.size > 0 { format!("{name}  ·  {}", fmt_size(p.size)) } else { name };
                }
                let mut rows: Vec<(&str, String)> = vec![];
                let video = m.label();
                if !video.is_empty() {
                    rows.push(("Video", video));
                }
                if !audio.is_empty() {
                    rows.push(("Audio", audio.join(", ")));
                }
                if !subs.is_empty() {
                    rows.push((t!("Untertitel"), subs.join(", ")));
                }
                if !it.directors.is_empty() {
                    rows.push((t!("Regie"), it.directors.join(", ")));
                }
                if !it.writers.is_empty() {
                    rows.push((t!("Drehbuch"), it.writers.iter().take(4).cloned().collect::<Vec<_>>().join(", ")));
                }
                if !it.studio.is_empty() {
                    rows.push(("Studio", it.studio.clone()));
                }
                if !it.originally_available.is_empty() {
                    rows.push((t!("Erschienen"), fmt_date(&it.originally_available)));
                }
                if !file.is_empty() {
                    rows.push((t!("Datei"), file));
                }
                if it.media.len() > 1 {
                    rows.push((t!("Versionen"), it.media.iter().map(|m| m.label()).collect::<Vec<_>>().join("  |  ")));
                }
                content(ui, &mut |ui| {
                    ui.label(RichText::new("Details").font(theme::semibold(18.0)).color(t.text));
                    ui.add_space(4.0);
                    egui::Grid::new(("plex-details", &it.rating_key)).num_columns(2).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                        for (k, v) in &rows {
                            ui.label(RichText::new(*k).color(t.text_faint));
                            ui.add(egui::Label::new(RichText::new(v).color(t.text_dim)).wrap());
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(18.0);
            }
        }
        // Cast
        if !it.roles.is_empty() {
            let roles = it.roles.clone();
            content(ui, &mut |ui| {
                ui.label(RichText::new(t!("Besetzung")).font(theme::semibold(18.0)).color(t.text));
                ui.add_space(6.0);
                egui::ScrollArea::horizontal()
                    .id_salt(("cast", &it.rating_key))
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        for r in roles.iter().take(30) {
                            let (rect, _) = ui.allocate_exact_size(vec2(96.0, 136.0), Sense::hover());
                            let c = pos2(rect.center().x, rect.top() + 40.0);
                            let img_r = Rect::from_center_size(c, vec2(80.0, 80.0));
                            match self.plex.image(&r.thumb, 80.0, 80.0) {
                                Some(s) => {
                                    ui.painter().circle_filled(c, 40.0, t.surface);
                                    paint_image(ui, img_r, &s, CornerRadius::same(40));
                                }
                                None => {
                                    ui.painter().circle_filled(c, 40.0, t.elevated);
                                    let initials: String = r.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
                                    ui.painter().text(c, Align2::CENTER_CENTER, initials, theme::semibold(20.0), t.text_faint);
                                }
                            }
                            let g = wrapped(ui, &r.name, FontId::proportional(12.5), t.text, 96.0, 1);
                            let gw = g.size().x;
                            ui.painter().galley(pos2(rect.center().x - gw / 2.0, rect.top() + 88.0), g, t.text);
                            let g = wrapped(ui, &r.role, FontId::proportional(11.5), t.text_faint, 96.0, 2);
                            let gw = g.size().x;
                            ui.painter().galley(pos2(rect.center().x - gw / 2.0, rect.top() + 106.0), g, t.text_faint);
                        }
                    });
                });
            });
            ui.add_space(18.0);
        }
        // Related hubs ("Mehr wie das", collections …)
        let rel_key = if it.kind == Kind::Episode { it.grandparent_key.clone() } else { it.rating_key.clone() };
        if let St::Ready(hubs) = self.plex.hubs(&format!("/library/metadata/{rel_key}/related?count=16")) {
            let mut acts4 = Vec::new();
            for (i, h) in hubs.iter().enumerate() {
                content(ui, &mut |ui| {
                    let more = (h.more && !h.key.is_empty()).then(|| LibLoc::Plex(PlexLoc::Hub { key: h.key.clone(), title: h.title.clone() }));
                    self.shelf(ui, &format!("rel{rel_key}-{i}"), &h.title, &h.items, more, &mut acts4, compact);
                });
            }
            acts.extend(acts4);
        }
    }

    fn episode_row(&mut self, ui: &mut Ui, e: &Item, acts: &mut Vec<Act>, compact: bool) {
        let t = self.t;
        let w = ui.available_width();
        let th_w = if compact { 128.0 } else { 196.0 };
        let th_h = th_w * 9.0 / 16.0;
        let h = th_h + 16.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let current = plex::remote(&self.current_path()).map(|r| r.rating_key == e.rating_key).unwrap_or(false);
        if current {
            ui.painter().rect_filled(rect, CornerRadius::same(theme::R_LG), t.accent_soft);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(theme::R_LG), t.hover);
        }
        let img = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(th_w, th_h));
        let cr = CornerRadius::same(theme::R);
        match self.plex.image(&e.thumb, th_w, th_h) {
            Some(s) => {
                ui.painter().rect_filled(img, cr, t.surface);
                paint_image(ui, img, &s, cr);
            }
            None => placeholder(ui, &t, img, e, cr),
        }
        if let Some(p) = e.progress().filter(|_| !e.watched()) {
            widgets::progress_line(ui.painter(), Rect::from_min_max(pos2(img.left(), img.bottom() - 4.0), img.max), p, &t);
        }
        if e.watched() {
            check_badge(ui, &t, img);
        }
        if resp.hovered() {
            let c = Rect::from_center_size(img.center(), vec2(40.0, 30.0));
            ui.painter().rect_filled(c, CornerRadius::same(4), t.accent);
            ui.painter().text(c.center(), Align2::CENTER_CENTER, ph::PLAY, theme::fill_icon(15.0), t.on_accent);
        }
        let x = img.right() + 16.0;
        let right_w = if compact { 8.0 } else { 120.0 };
        let tw = rect.right() - x - right_w;
        let num = e.index.map(|i| format!("{i}. ")).unwrap_or_default();
        let title_col = if current { t.accent } else if e.watched() { t.text_dim } else { t.text };
        let g = wrapped(ui, &format!("{num}{}", e.title), theme::semibold(14.5), title_col, tw, 1);
        let mut y = rect.top() + 12.0;
        let gh = g.size().y;
        ui.painter().galley(pos2(x, y), g, title_col);
        y += gh + 4.0;
        let mut meta = vec![];
        if e.duration_ms > 0 {
            meta.push(fmt_minutes(e.duration()));
        }
        if !e.originally_available.is_empty() {
            meta.push(fmt_date(&e.originally_available));
        }
        if let Some(m) = e.media.first() {
            let l = m.resolution_label();
            if !l.is_empty() {
                meta.push(l);
            }
        }
        let mg = wrapped(ui, &meta.join("  ·  "), FontId::proportional(12.0), t.text_faint, tw, 1);
        let mh = mg.size().y;
        ui.painter().galley(pos2(x, y), mg, t.text_faint);
        y += mh + 6.0;
        if !compact && !e.summary.is_empty() {
            let rows = (((rect.bottom() - 8.0 - y) / 18.0).floor() as usize).clamp(1, 3);
            let sg = wrapped(ui, &e.summary, FontId::proportional(13.0), t.text_dim, tw, rows);
            ui.painter().galley(pos2(x, y), sg, t.text_dim);
        }
        if !compact {
            let label = if e.watched() {
                t!("Gesehen").to_string()
            } else if let Some(_) = e.progress() {
                tf!("noch {}", fmt_minutes(e.duration() - e.view_offset()))
            } else {
                String::new()
            };
            ui.painter().text(pos2(rect.right() - 14.0, rect.top() + 20.0), Align2::RIGHT_CENTER, label, FontId::proportional(12.0), if e.watched() { t.accent } else { t.text_faint });
        }
        let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked() {
            acts.push(Act::Play { item: e.clone(), start_at: None, mode: PlayMode::Play, shuffle: false });
        }
        resp.context_menu(|ui| {
            ui.set_min_width(230.0);
            self.item_menu(ui, e, acts);
        });
        ui.add_space(4.0);
    }

    /// Collections, playlists, artists, albums: header + grid/list of children.
    fn plex_collection(&mut self, ui: &mut Ui, item: &Arc<Item>, acts: &mut Vec<Act>, compact: bool) {
        let t = self.t;
        let it = item.as_ref().clone();
        let path = match it.kind {
            Kind::Collection => format!("/library/collections/{}/children", it.rating_key),
            Kind::Playlist => format!("/playlists/{}/items", it.rating_key),
            _ => format!("/library/metadata/{}/children", it.rating_key),
        };
        let children = self.plex.items(&path);
        let count = match &children {
            St::Ready(c) => c.len(),
            _ => 0,
        };
        let has_art = !it.art.is_empty();
        let side = self.px.side_col.is_some();
        self.detail_header(ui, &it, compact, has_art, acts, &mut |app, ui, acts| {
            let t = app.t;
            let kind = match it.kind {
                Kind::Collection => t!("Sammlung"),
                Kind::Playlist => "Playlist",
                Kind::Artist => t!("Interpret"),
                Kind::Album => "Album",
                _ => "",
            };
            ui.label(RichText::new(kind.to_uppercase()).size(11.5).color(t.accent));
            ui.add(egui::Label::new(RichText::new(&it.title).font(theme::semibold(if compact { 22.0 } else { 30.0 })).color(t.text)).wrap());
            let mut meta = vec![];
            if count > 0 {
                meta.push(if count == 1 { t!("1 Element").to_string() } else { tf!("{count} Elemente") });
            }
            if it.duration_ms > 0 {
                meta.push(fmt_minutes(it.duration()));
            }
            if let Some(y) = it.year {
                meta.push(y.to_string());
            }
            ui.label(RichText::new(meta.join("  ·  ")).size(13.5).color(t.text_dim));
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                if play_pill(ui, &t, t!("Alle abspielen"), "").clicked() {
                    acts.push(Act::Play { item: it.clone(), start_at: Some(0.0), mode: PlayMode::Play, shuffle: false });
                }
                if action_icon(ui, &t, ph::SHUFFLE, t!("Zufallswiedergabe"), false).clicked() {
                    acts.push(Act::Play { item: it.clone(), start_at: Some(0.0), mode: PlayMode::Play, shuffle: true });
                }
                if action_icon(ui, &t, ph::QUEUE, t!("Alle zur Warteschlange hinzufügen"), false).clicked() {
                    acts.push(Act::Play { item: it.clone(), start_at: None, mode: PlayMode::Enqueue, shuffle: false });
                }
            });
            if !it.summary.is_empty() {
                ui.add_space(4.0);
                let maxw = ui.available_width().min(880.0);
                let g = wrapped(ui, &it.summary, FontId::proportional(14.0), t.text_dim, maxw, 5);
                let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                ui.painter().galley(r.min, g, t.text_dim);
            }
        });
        match children {
            St::Ready(c) => {
                let pad = if compact || side { 0.0 } else { 26.0 };
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        if it.kind == Kind::Playlist && c.iter().any(|x| x.kind.landscape() || x.kind == Kind::Track) {
                            for e in c.iter() {
                                self.episode_row(ui, e, acts, compact);
                            }
                        } else {
                            self.item_grid(ui, &it.rating_key, &c, acts, compact);
                        }
                    });
                });
            }
            St::Loading => spinner(ui, &t),
            St::Failed(e) => self.plex_error(ui, &e, None),
        }
    }

    // --------------------------------------------------- library integration

    /// "Weiterschauen auf <Server>" shelf for Velour's own start page.
    pub fn plex_home_shelf(&mut self, ui: &mut Ui) {
        if !self.plex.ready() || !self.data.settings.plex_on_home {
            return;
        }
        let St::Ready(hubs) = self.plex.hubs("/hubs?count=16&includeLibraryPlaylists=0") else { return };
        let items = continue_items(&hubs);
        if items.is_empty() {
            return;
        }
        let mut acts = Vec::new();
        let name = self.plex.server.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| "Plex".into());
        self.shelf(ui, "vlb-home-plex", &tf!("Weiterschauen auf {name}"), &items, Some(LibLoc::Plex(PlexLoc::Home)), &mut acts, false);
        self.plex_handle(acts, None);
    }

    /// Navigation entries for the full library sidebar.
    pub fn plex_nav(&mut self, ui: &mut Ui, nav_item: fn(&mut Ui, &Theme, &str, &str, bool) -> egui::Response) {
        let t = self.t;
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("PLEX").size(11.0).color(t.text_faint));
            let (dot, tip) = match &self.plex.status {
                Status::Ready => (t.accent, t!("Verbunden").to_string()),
                Status::Connecting(m) => (t.text_faint, m.clone()),
                Status::Error(e) => (t.danger, e.clone()),
                _ => (Color32::TRANSPARENT, String::new()),
            };
            if dot != Color32::TRANSPARENT {
                let (r, resp) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 3.5, dot);
                resp.on_hover_text(tip);
            }
        });
        ui.add_space(2.0);
        let cur = match &self.lib.loc {
            LibLoc::Plex(p) => Some(p.clone()),
            _ => None,
        };
        if !self.plex.signed_in() && !matches!(self.plex.status, Status::Pin { .. }) {
            if nav_item(ui, &t, ph::PLUGS_CONNECTED, t!("Mit Plex verbinden"), matches!(cur, Some(PlexLoc::Home))).clicked() {
                self.lib.go(LibLoc::Plex(PlexLoc::Home));
            }
            return;
        }
        let server = if self.data.plex.server_name.is_empty() { t!("Übersicht").to_string() } else { self.data.plex.server_name.clone() };
        let home_sel = matches!(cur, Some(PlexLoc::Home) | Some(PlexLoc::Search(_)) | Some(PlexLoc::Hub { .. }));
        if nav_item(ui, &t, ph::HARD_DRIVES, &server, home_sel).clicked() {
            self.lib.go(LibLoc::Plex(PlexLoc::Home));
        }
        let cur_section = match &cur {
            Some(PlexLoc::Section { key, .. }) => Some(key.clone()),
            Some(PlexLoc::Item { section: Some((k, _)), .. }) => Some(k.clone()),
            _ => None,
        };
        for s in self.plex.sections.clone() {
            let sel = cur_section.as_deref() == Some(s.key.as_str());
            if nav_item(ui, &t, s.icon(), &s.title, sel).clicked() {
                let tab = match &self.lib.loc {
                    LibLoc::Plex(PlexLoc::Section { key, tab, .. }) if *key == s.key => *tab,
                    _ => PlexTab::Recommended,
                };
                self.lib.go(LibLoc::Plex(PlexLoc::Section { key: s.key.clone(), title: s.title.clone(), tab }));
            }
        }
    }
}

fn check_badge(ui: &Ui, t: &Theme, img: Rect) {
    let b = Rect::from_min_size(img.right_top() + vec2(-28.0, 6.0), vec2(22.0, 22.0));
    ui.painter().rect_filled(b, CornerRadius::same(11), Color32::from_black_alpha(170));
    ui.painter().text(b.center(), Align2::CENTER_CENTER, ph::CHECK, theme::icon(13.0), t.accent);
}

fn section_chip(ui: &mut Ui, t: &Theme, icon: &str, label: &str) -> egui::Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(13.0), t.text);
    let w = (g.size().x + 46.0).min(260.0);
    let (r, resp) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
    ui.painter().rect_filled(r, CornerRadius::same(theme::R_LG), if resp.hovered() { t.hover } else { t.surface });
    ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::CENTER_CENTER, icon, theme::icon(16.0), t.accent);
    widgets::painter_text_trunc(ui.painter(), pos2(r.left() + 34.0, r.center().y - 9.0), label, FontId::proportional(13.0), t.text, w - 40.0);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn grid_metrics(w: f32, shape: CardShape, compact: bool) -> (usize, f32, f32) {
    let target = match shape {
        CardShape::Poster => if compact { 116.0 } else { 156.0 },
        CardShape::Square => if compact { 130.0 } else { 172.0 },
        CardShape::Wide => if compact { 200.0 } else { 260.0 },
    };
    let gap = if compact { 10.0 } else { 16.0 };
    let cols = ((w + gap) / (target + gap)).floor().max(1.0) as usize;
    let cw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
    (cols, cw, gap)
}

fn fmt_count(n: i64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(if crate::i18n::is_en() { ',' } else { '.' });
        }
        out.push(c);
    }
    out
}
