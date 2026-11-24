//! Queue rows (shared by the queue panel and the now-playing page) and the
//! now-playing page for music.
//!
//! One layout for everything; parts show up by what the queue holds:
//! the list once there is more than one title, track numbers when all come
//! from one album, a picture per row when they don't, the artist per row
//! when it differs, details when there is room (or on request).

use crate::app::{App, RightPanel, ThumbState};
use crate::media;
use crate::player::{PlayerState, PlaylistEntry};
use crate::theme::{self, Theme};
use crate::widgets;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Id, Rect, Sense, Stroke, StrokeKind, TextureId, Ui, pos2, vec2};
use egui_phosphor::regular as ph;
use std::hash::{Hash, Hasher};
use std::path::Path;

// ------------------------------------------------------------- origin

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OriginKind {
    /// Loose files, or nothing that names the list.
    #[default]
    Queue,
    Folder,
    Playlist,
    Album,
    Artist,
    Genre,
}

/// Where the queue came from, and how many titles were added since.
#[derive(Clone, Debug, Default)]
pub struct QueueOrigin {
    pub kind: OriginKind,
    pub name: String,
    pub added: usize,
}

impl QueueOrigin {
    /// Named after a playlist file, or the folder all files share.
    pub fn of_list(list: &[String]) -> Self {
        let Some(first) = list.first() else { return Self::default() };
        if list.len() == 1 {
            if media::is_playlist(first) {
                let name = Path::new(first).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                return Self { kind: OriginKind::Playlist, name, added: 0 };
            }
            return Self::default();
        }
        let dir = |p: &str| if media::is_url(p) { None } else { Path::new(p).parent().map(|d| d.to_path_buf()) };
        match dir(first) {
            Some(d) if list.iter().all(|p| dir(p).as_ref() == Some(&d)) => {
                let name = d.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                if name.is_empty() { Self::default() } else { Self { kind: OriginKind::Folder, name, added: 0 } }
            }
            _ => Self::default(),
        }
    }
}

// ---------------------------------------------------------------- rows

/// How the rows of one queue look; decided for the whole list.
#[derive(Clone, Copy, Debug)]
pub struct RowLook {
    /// All titles from one album: track numbers instead of pictures.
    pub numbers: bool,
    /// Artists differ: show them under the titles.
    pub artist: bool,
}

impl Default for RowLook {
    fn default() -> Self {
        Self { numbers: false, artist: true }
    }
}

impl RowLook {
    pub fn row_h(&self) -> f32 {
        match (self.numbers, crate::mobile::touch()) {
            (true, false) => 40.0,
            (true, true) => 48.0,
            (false, false) => 46.0,
            (false, true) => 54.0,
        }
    }
}

pub enum Pic {
    /// Nothing to show: a stream, or a file without picture.
    None,
    /// Still loading (probe or thumbnail).
    Pending,
    /// Texture plus the part of it to show.
    Image(TextureId, Rect),
}

pub struct RowInfo {
    pub title: String,
    pub sub: String,
    pub audio: bool,
    pub stream: bool,
    pub track: Option<u32>,
    pub dur: f64,
    pub pic: Pic,
    /// Watched share of a video, unless finished.
    pub progress: Option<f32>,
}

struct RowMeta {
    audio: bool,
    album: Option<String>,
    artist: Option<String>,
}

/// The centre part of a `w`×`h` image with the aspect `aspect`, in uv.
fn crop_uv(w: f32, h: f32, aspect: f32) -> Rect {
    if w <= 0.0 || h <= 0.0 {
        return Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    }
    let a = w / h;
    if a > aspect {
        let k = aspect / a;
        Rect::from_min_max(pos2(0.5 - k / 2.0, 0.0), pos2(0.5 + k / 2.0, 1.0))
    } else {
        let k = a / aspect;
        Rect::from_min_max(pos2(0.0, 0.5 - k / 2.0), pos2(1.0, 0.5 + k / 2.0))
    }
}

/// A calm colour of its own for every name.
fn name_color(s: &str) -> Color32 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    let hue = (h.finish() % 360) as f32 / 360.0;
    egui::ecolor::HsvaGamma { h: hue, s: 0.42, v: 0.36, a: 1.0 }.into()
}

fn image(p: &egui::Painter, r: Rect, id: TextureId, uv: Rect, radius: u8, tint: Color32) {
    p.add(egui::epaint::RectShape::filled(r, CornerRadius::same(radius), tint).with_texture(id, uv));
}

/// Placeholder cover: initials on a colour of their own.
fn placeholder(p: &egui::Painter, r: Rect, radius: u8) {
    let [tr, tg, tb, _] = crate::logo::LOGO_TILE;
    let [dr, dg, db, _] = crate::logo::LOGO_DRAPE;
    p.rect_filled(r, CornerRadius::same(radius), Color32::from_rgb(tr, tg, tb));
    widgets::draw_logo(p, Rect::from_center_size(r.center(), r.size() * 0.5), Color32::from_rgb(dr, dg, db));
}

/// Paints one row (no interaction). `right_w` stays free for the caller.
pub fn paint_row(p: &egui::Painter, t: &Theme, rect: Rect, info: &RowInfo, look: RowLook, idx: usize, current: bool, paused: bool, right_w: f32) {
    let numbered = look.numbers || (info.stream && matches!(info.pic, Pic::None));
    let slot_w = if look.numbers { 26.0 } else { 58.0 };
    let slot = Rect::from_min_size(pos2(rect.left() + 6.0, rect.top()), vec2(slot_w, rect.height()));
    let ph_h = (rect.height() - 12.0).min(34.0);
    // playing: dancing bars; paused: the pause sign
    let mark = |c: egui::Pos2, color: Color32| {
        if paused {
            p.text(c, Align2::CENTER_CENTER, ph::PAUSE, theme::fill_icon(13.0), color);
        } else {
            eq_bars(p, c, color);
        }
    };
    if numbered {
        if current {
            mark(slot.center(), t.accent);
        } else {
            let n = if look.numbers { info.track.map(|n| n as usize).unwrap_or(idx + 1) } else { idx + 1 };
            p.text(slot.center(), Align2::CENTER_CENTER, n.to_string(), FontId::proportional(11.5), t.text_faint);
        }
    } else {
        let size = if info.audio { vec2(ph_h, ph_h) } else { vec2(ph_h * 16.0 / 9.0, ph_h) };
        let r = Rect::from_center_size(slot.center(), size);
        match &info.pic {
            Pic::Image(id, uv) => image(p, r, *id, *uv, 4, Color32::WHITE),
            Pic::Pending => {
                p.rect_filled(r, CornerRadius::same(4), t.elevated);
            }
            Pic::None if info.audio => placeholder(p, r, 4),
            Pic::None => {
                p.rect_filled(r, CornerRadius::same(4), t.elevated);
                p.text(r.center(), Align2::CENTER_CENTER, ph::FILM_STRIP, theme::icon(14.0), t.text_faint);
            }
        }
        if let Some(f) = info.progress.filter(|f| *f > 0.02 && !current) {
            let bar = Rect::from_min_max(pos2(r.left(), r.bottom() - 2.0), pos2(r.left() + r.width() * f.min(1.0), r.bottom()));
            p.rect_filled(bar, CornerRadius::ZERO, t.accent);
        }
        if current {
            p.rect_filled(r, CornerRadius::same(4), Color32::from_black_alpha(130));
            mark(r.center(), Color32::WHITE);
        }
    }
    let x = slot.right() + 10.0;
    let max_w = (rect.right() - right_w - 6.0 - x).max(10.0);
    let show_sub = !info.sub.is_empty() && (!info.audio || look.artist);
    let color = if current { t.accent } else { t.text };
    if show_sub {
        widgets::painter_text_trunc(p, pos2(x, rect.center().y - 17.0), &info.title, FontId::proportional(13.0), color, max_w);
        widgets::painter_text_trunc(p, pos2(x, rect.center().y + 1.0), &info.sub, FontId::proportional(11.5), t.text_dim, max_w);
    } else {
        widgets::painter_text_trunc(p, pos2(x, rect.center().y - 9.0), &info.title, FontId::proportional(13.0), color, max_w);
    }
}

/// Three bars dancing next to the title that plays.
fn eq_bars(p: &egui::Painter, c: egui::Pos2, color: Color32) {
    let time = p.ctx().input(|i| i.time) as f32;
    for (k, (speed, phase)) in [(5.3f32, 0.0f32), (7.1, 1.9), (4.4, 3.4)].into_iter().enumerate() {
        let h = 3.0 + 9.0 * (0.5 + 0.5 * (time * speed + phase).sin());
        let x = c.x + (k as f32 - 1.0) * 4.5;
        p.rect_filled(Rect::from_min_max(pos2(x - 1.25, c.y + 6.0 - h), pos2(x + 1.25, c.y + 6.0)), CornerRadius::same(1), color);
    }
    p.ctx().request_repaint_after(std::time::Duration::from_millis(50));
}

// ----------------------------------------------------------------- cover

/// Side of the cover taken from the file.
const ART: usize = 512;
/// Side of the blurred copy behind the page.
const BACK: usize = 32;

struct ArtPixels {
    cover: Vec<u8>,
    backdrop: Vec<u8>,
    accent: Option<Color32>,
}

/// The current title's cover, big, and the colours the page takes from it.
pub struct NpArt {
    path: String,
    asked: bool,
    rx: Option<std::sync::mpsc::Receiver<Option<ArtPixels>>>,
    cover: Option<egui::TextureHandle>,
    backdrop: Option<egui::TextureHandle>,
    accent: Option<Color32>,
    /// The previous title's backdrop (fading out), colour and cover (shown
    /// until this title's cover is read).
    old: Option<(egui::TextureHandle, Option<Color32>, Option<egui::TextureHandle>)>,
    since: f64,
}

fn analyse(rgba: Vec<u8>) -> Option<ArtPixels> {
    use image::imageops::{self, FilterType};
    let img = image::RgbaImage::from_raw(ART as u32, ART as u32, rgba)?;
    let small = imageops::resize(&img, BACK as u32, BACK as u32, FilterType::Triangle);
    let mut backdrop = imageops::blur(&small, 2.2).into_raw();
    // stronger colours, so dark covers still tint the page
    // (plain RGB: an HSV round trip left stray black texels on dark covers)
    for c in backdrop.chunks_exact_mut(4) {
        let luma = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
        for v in &mut c[..3] {
            let sat = luma + (*v as f32 - luma) * 1.5;
            *v = (sat * 1.35 + 10.0).clamp(0.0, 255.0) as u8;
        }
    }
    let accent = accent_of(imageops::resize(&img, 48, 48, FilterType::Triangle).as_raw());
    Some(ArtPixels { cover: img.into_raw(), backdrop, accent })
}

/// The cover's most striking colour, made light enough for text on a
/// dark page; none for a grey cover.
fn accent_of(px: &[u8]) -> Option<Color32> {
    const BINS: usize = 18;
    let mut bins = [[0f32; 4]; BINS];
    let n = (px.len() / 4) as f32;
    for c in px.chunks_exact(4) {
        let hsv = egui::ecolor::HsvaGamma::from(Color32::from_rgb(c[0], c[1], c[2]));
        if hsv.s < 0.3 || hsv.v < 0.25 {
            continue;
        }
        let w = hsv.s * hsv.v;
        let b = &mut bins[((hsv.h * BINS as f32) as usize).min(BINS - 1)];
        b[0] += w;
        for k in 0..3 {
            b[k + 1] += w * c[k] as f32;
        }
    }
    let best = bins.iter().max_by(|a, b| a[0].total_cmp(&b[0]))?;
    if best[0] < n * 0.03 {
        return None;
    }
    let avg = Color32::from_rgb((best[1] / best[0]) as u8, (best[2] / best[0]) as u8, (best[3] / best[0]) as u8);
    let mut hsv = egui::ecolor::HsvaGamma::from(avg);
    hsv.s = hsv.s.clamp(0.35, 0.68);
    hsv.v = 0.96;
    Some(hsv.into())
}

/// A colour fading from `color` in the middle to nothing at `radius`.
fn glow(p: &egui::Painter, center: egui::Pos2, radius: egui::Vec2, color: Color32) {
    let mut m = egui::Mesh::default();
    m.colored_vertex(center, color);
    let n = 64;
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        m.colored_vertex(center + vec2(a.cos() * radius.x, a.sin() * radius.y), Color32::TRANSPARENT);
        if i > 0 {
            m.add_triangle(0, i, i + 1);
        }
    }
    p.add(m);
}

/// How bright the blurred cover is at height `f` (0 top, 1 bottom): dim
/// for text, a little darker under the title bar, fading out towards the
/// controls.
fn backdrop_light(f: f32) -> f32 {
    let top = 0.62 + 0.38 * (f / 0.3).clamp(0.0, 1.0);
    let s = ((f - 0.55) / 0.45).clamp(0.0, 1.0);
    let bottom = 1.0 - 0.9 * s * s * (3.0 - 2.0 * s);
    0.45 * top * bottom
}

/// The blurred cover over the whole stage, dark enough for text. The
/// darkening sits in the vertex colours: a black layer on top would turn
/// dithered pixels on dark covers black.
fn backdrop(p: &egui::Painter, r: Rect, tex: TextureId, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let uv = crop_uv(1.0, 1.0, r.width() / r.height().max(1.0));
    let mut m = egui::Mesh::with_texture(tex);
    let rows = 24;
    for i in 0..=rows {
        let f = i as f32 / rows as f32;
        let g = (255.0 * backdrop_light(f) * alpha) as u8;
        let c = Color32::from_rgba_premultiplied(g, g, g, (255.0 * alpha) as u8);
        let y = egui::lerp(r.top()..=r.bottom(), f);
        let v = egui::lerp(uv.top()..=uv.bottom(), f);
        m.vertices.push(egui::epaint::Vertex { pos: pos2(r.left(), y), uv: pos2(uv.left(), v), color: c });
        m.vertices.push(egui::epaint::Vertex { pos: pos2(r.right(), y), uv: pos2(uv.right(), v), color: c });
        if i > 0 {
            let k = 2 * i as u32;
            m.add_triangle(k - 2, k - 1, k);
            m.add_triangle(k - 1, k + 1, k);
        }
    }
    p.add(m);
}

fn dur_text(secs: f64) -> String {
    let m = (secs / 60.0).round() as i64;
    if m >= 60 { tf!("{} Std {} Min", m / 60, m % 60) } else { tf!("{m} Min") }
}

impl App {
    fn row_meta(&self, f: &str) -> RowMeta {
        if let Some(r) = crate::plex::remote(f) {
            let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
            return RowMeta { audio: r.kind == crate::plex::model::Kind::Track, album: some(&r.album), artist: some(&r.sub) };
        }
        match self.tools.probe_cache.get(f) {
            Some(p) => RowMeta { audio: !p.has_video, album: p.album.clone(), artist: p.artist.clone() },
            None => RowMeta { audio: media::is_audio(f), album: None, artist: None },
        }
    }

    /// Rules for the rows of `list` (cheap: cached tags only).
    pub(crate) fn queue_look(&self, list: &[PlaylistEntry]) -> RowLook {
        if list.len() < 2 || list.len() > 2000 {
            return RowLook::default();
        }
        let metas: Vec<RowMeta> = list.iter().map(|e| self.row_meta(&e.filename)).collect();
        let all_audio = metas.iter().all(|m| m.audio);
        let same = |get: fn(&RowMeta) -> &Option<String>| {
            let first = get(&metas[0]);
            first.is_some() && metas.iter().all(|m| get(m) == first)
        };
        RowLook { numbers: all_audio && same(|m| &m.album), artist: !(all_audio && same(|m| &m.artist)) }
    }

    /// Everything one row shows; asks for tags and pictures it lacks.
    pub(crate) fn row_info(&mut self, ctx: &egui::Context, e: &PlaylistEntry, want_pic: bool) -> RowInfo {
        let f = e.filename.as_str();
        if let Some(r) = crate::plex::remote(f) {
            let audio = r.kind == crate::plex::model::Kind::Track;
            let pic = if !want_pic || r.thumb.is_empty() {
                Pic::None
            } else {
                let (w, h) = if audio { (80.0, 80.0) } else { (160.0, 90.0) };
                match self.plex.image(&r.thumb, w, h) {
                    Some(s) => Pic::Image(s.id, crop_uv(s.size.x, s.size.y, if audio { 1.0 } else { 16.0 / 9.0 })),
                    None => Pic::Pending,
                }
            };
            let title = if r.name.is_empty() { e.display() } else { r.name.clone() };
            return RowInfo { title, sub: r.sub, audio, stream: false, track: None, dur: r.duration, pic, progress: None };
        }
        if media::is_url(f) {
            return RowInfo { title: e.display(), sub: String::new(), audio: true, stream: true, track: None, dur: 0.0, pic: Pic::None, progress: None };
        }
        let probe = self.tools.request_probe(f).cloned();
        let audio = probe.as_ref().map(|p| !p.has_video).unwrap_or_else(|| media::is_audio(f));
        // without ffprobe (Android) songs read their cover straight from the file
        let direct = probe.is_none() && self.tools.ffprobe.is_none() && media::is_audio(f);
        let pic = match &probe {
            _ if !want_pic => Pic::None,
            None if !direct => Pic::Pending,
            _ if direct || probe.as_ref().is_some_and(|p| p.has_video || p.has_cover) => match self.thumb_for(ctx, f) {
                // Thumbnails are 16:9 with the cover padded in the middle.
                Some(id) if audio => Pic::Image(id, crop_uv(media::THUMB_W as f32, media::THUMB_H as f32, 1.0)),
                Some(id) => Pic::Image(id, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                None if matches!(self.thumb_keys.get(f).and_then(|k| self.thumbs.get(k)), Some(ThumbState::Failed)) => Pic::None,
                None if !self.tools.frames() && !audio => Pic::None,
                None => Pic::Pending,
            },
            _ => Pic::None,
        };
        let (title, sub, track, dur) = match &probe {
            Some(p) if audio => (
                p.title.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| e.display()),
                p.artist.clone().unwrap_or_default(),
                p.track,
                p.duration,
            ),
            Some(p) => (e.display(), if p.width > 0 { format!("{}×{}", p.width, p.height) } else { String::new() }, None, p.duration),
            None => (e.display(), String::new(), None, 0.0),
        };
        let progress = if audio { None } else { self.data.history_for(f).filter(|h| !h.finished).map(|h| h.progress()) };
        let sub = if !audio && self.data.history_for(f).is_some_and(|h| h.finished) {
            if sub.is_empty() { t!("gesehen").to_string() } else { format!("{sub} · {}", t!("gesehen")) }
        } else {
            sub
        };
        RowInfo { title, sub, audio, stream: false, track, dur, pic, progress }
    }

    // ------------------------------------------------------ now playing

    /// The page for music (and radio): cover, title, the queue and details.
    pub(crate) fn now_playing_page(&mut self, ui: &mut Ui, rect: Rect, st: &PlayerState, bottom_inset: f32) {
        let base = self.t;
        // The stage stays dark in both themes, like the video area.
        let mut t = base;
        t.text = Color32::from_gray(236);
        t.text_dim = Color32::from_gray(160);
        t.text_faint = Color32::from_gray(108);
        t.elevated = Color32::from_gray(40);
        t.hover = Color32::from_white_alpha(12);
        t.accent_soft = Color32::from_rgba_unmultiplied(base.accent.r(), base.accent.g(), base.accent.b(), 46);
        let bg = base.video_bg;
        let ctx = ui.ctx().clone();
        let painter = ui.painter().clone();

        // -- stage: the blurred cover, faded over from the last title's
        self.update_np_art(&ctx, st);
        let now = ctx.input(|i| i.time);
        let (art_cover, art_back, art_accent, old, since, loading, failed) = match &self.np_art {
            Some(a) => (
                a.cover.as_ref().map(|t| t.id()),
                a.backdrop.as_ref().map(|t| t.id()),
                a.accent,
                a.old.as_ref().map(|(t, c, cv)| (t.id(), *c, cv.as_ref().map(|x| x.id()))),
                a.since,
                a.rx.is_some() || (!a.asked && st.tracks.is_empty()),
                a.asked && a.rx.is_none() && a.cover.is_none(),
            ),
            None => (None, None, None, None, now, false, false),
        };
        let k = ((now - since) / 0.45).clamp(0.0, 1.0) as f32;
        let k = k * k * (3.0 - 2.0 * k);
        if let Some((tex, _, _)) = old {
            backdrop(&painter, rect, tex, if art_back.is_none() && loading { 1.0 } else { 1.0 - k });
        }
        if let Some(tex) = art_back {
            backdrop(&painter, rect, tex, k);
        }
        if k < 1.0 || loading {
            ctx.request_repaint();
        } else if let Some(a) = self.np_art.as_mut() {
            a.old = None;
        }
        let has_stage = art_back.is_some() || old.is_some();
        // the cover's colour, or the last one's while the new cover loads
        let accent = art_accent.or_else(|| old.and_then(|o| o.1).filter(|_| loading));
        if let Some(c) = accent {
            t.accent = c;
            t.accent_soft = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 34);
        }

        let mut area = rect.shrink2(vec2(32.0, 26.0));
        area.max.y -= bottom_inset;
        if area.width() < 120.0 || area.height() < 120.0 {
            return;
        }
        let wide = area.width() >= 620.0 && area.height() >= 240.0;

        // -- what there is to show; while mpv still opens the file, the
        // tags ffprobe read for the list stand in
        let probe = if media::is_url(&st.path) { None } else { self.tools.probe_cache.get(&st.path).cloned() };
        let opening = st.tracks.is_empty();
        let tag = |v: Option<&str>, f: fn(&media::ProbeInfo) -> &Option<String>| {
            v.map(str::to_string).or_else(|| probe.as_ref().and_then(|p| f(p).clone())).filter(|s| !s.trim().is_empty())
        };
        let title = tag(st.meta("title"), |p| &p.title).unwrap_or_else(|| {
            // mpv names a file without tags after its file name; the list
            // shows it without the extension, so does the page.
            let file = Path::new(&st.path).file_name().map(|s| s.to_string_lossy().to_string());
            if !media::is_url(&st.path) && file.as_deref() == Some(st.media_title.as_str()) { media::display_name(&st.path) } else { st.media_title.clone() }
        });
        let artist = st.now_playing().map(|s| s.to_string()).or_else(|| tag(st.meta("artist").or_else(|| st.meta("album_artist")), |p| &p.artist));
        let album_tag = tag(st.meta("album"), |p| &p.album);
        let cover_known = st.has_cover() || probe.as_ref().is_some_and(|p| p.has_cover);
        let year = st.meta("date").or_else(|| st.meta("year")).map(|d| d.chars().take(4).collect::<String>());
        let album_line = [album_tag.clone(), year.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let list = st.playlist.clone();
        let n = list.len();
        let pos = st.playlist_pos.max(0) as usize;
        let look = self.queue_look(&list);
        let panel_open = self.right == RightPanel::Queue;
        let can_list = n > 1 && !panel_open;
        let details = self.np_details(st);

        // -- layout
        let row_h = look.row_h();
        let (cover, col) = if wide {
            let s = area.height().min(area.width() * 0.42).clamp(120.0, 440.0);
            let col_w = (area.width() - s - 40.0).min(560.0);
            let total = s + 40.0 + col_w;
            let left = area.center().x - total / 2.0;
            let h = area.height().min(s.max(300.0));
            let cover = Rect::from_min_size(pos2(left, area.center().y - s / 2.0), vec2(s, s));
            let col = Rect::from_min_size(pos2(cover.right() + 40.0, area.center().y - h / 2.0), vec2(col_w, h));
            (cover, col)
        } else {
            let s = (area.width() * 0.62).min(area.height() * 0.42).clamp(80.0, 320.0);
            let cover = Rect::from_min_size(pos2(area.center().x - s / 2.0, area.top()), vec2(s, s));
            let w = area.width().min(520.0);
            let col = Rect::from_min_max(pos2(area.center().x - w / 2.0, cover.bottom() + 18.0), pos2(area.center().x + w / 2.0, area.bottom()));
            (cover, col)
        };

        // -- cover
        let r = 12.0;
        let logo = self.stream_logos.get(&st.path).cloned().and_then(|u| self.logos.get(&u, || u.clone()));
        let plex_cover = crate::plex::remote(&st.path)
            .filter(|rm| !rm.thumb.is_empty())
            .and_then(|rm| self.plex.image(&rm.thumb, 600.0, 600.0).map(|s| (s.id, crop_uv(s.size.x, s.size.y, 1.0))));
        if !has_stage {
            // no cover to blur: a glow in the placeholder's colour
            let c = name_color(artist.as_deref().unwrap_or(&title));
            glow(&painter, cover.center(), cover.size() * 1.25, c.gamma_multiply(0.55));
        }
        let shadow = egui::epaint::Shadow { offset: [0, 14], blur: 48, spread: 0, color: Color32::from_black_alpha(140) };
        painter.add(shadow.as_shape(cover, CornerRadius::same(r as u8)));
        if let Some(id) = art_cover {
            image(&painter, cover, id, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), r as u8, Color32::WHITE);
        } else if let Some((id, uv)) = plex_cover {
            image(&painter, cover, id, uv, r as u8, Color32::WHITE);
        } else if let Some(id) = old.and_then(|o| o.2).filter(|_| (cover_known || opening) && !failed) {
            // the last title's cover until this one's is read (on an album
            // it is the same)
            image(&painter, cover, id, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), r as u8, Color32::WHITE);
        } else if (cover_known || opening) && !failed {
            // ffmpeg is reading the cover: a quiet square for a moment
            painter.rect_filled(cover, CornerRadius::same(r as u8), Color32::from_white_alpha(8));
        } else if st.has_cover() {
            if let Some(v) = &self.video {
                v.paint(&painter, cover);
                // round the corners: a ring in the background colour
                let k = 8.0;
                painter.rect_stroke(cover.expand(k / 2.0), CornerRadius::same((r + k / 2.0) as u8), Stroke::new(k, bg), StrokeKind::Middle);
            }
        } else if let Some(img) = logo {
            painter.rect_filled(cover, CornerRadius::same(r as u8), Color32::from_gray(235));
            let k = (cover.width() * 0.78 / img.size.x).min(cover.height() * 0.78 / img.size.y);
            let fit = Rect::from_center_size(cover.center(), img.size * k);
            painter.image(img.id, fit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::from_white_alpha((img.alpha * 255.0) as u8));
        } else if media::is_url(&st.path) || title.is_empty() {
            painter.rect_filled(cover, CornerRadius::same(r as u8), Color32::from_gray(26));
            let mix = |c: u8| (c as f32 * 0.4 + 40.0 * 0.6) as u8;
            let muted = Color32::from_rgb(mix(base.accent.r()), mix(base.accent.g()), mix(base.accent.b()));
            widgets::draw_logo(&painter, Rect::from_center_size(cover.center(), cover.size() * 0.5), muted);
        } else {
            placeholder(&painter, cover, r as u8);
        }
        if can_list && ui.interact(cover, Id::new("np_cover"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.np_info = !self.np_info;
        }

        // -- title block
        let align_x = if wide { col.left() } else { col.center().x };
        let anchor = if wide { Align2::LEFT_TOP } else { Align2::CENTER_TOP };
        let mut y = col.top();
        let text_at = |y: f32, s: &str, font: FontId, color: Color32| {
            if wide {
                widgets::painter_text_trunc(&painter, pos2(align_x, y), s, font, color, col.width());
            } else {
                let g = painter.layout_no_wrap(s.to_string(), font.clone(), color);
                if g.size().x <= col.width() {
                    painter.text(pos2(align_x, y), anchor, s, font, color);
                } else {
                    widgets::painter_text_trunc(&painter, pos2(col.left(), y), s, font, color, col.width());
                }
            }
        };
        let title_size = if wide { 30.0 } else { 21.0 };
        text_at(y, &title, theme::semibold(title_size), Color32::WHITE);
        y += title_size + 12.0;
        if let Some(a) = &artist {
            let c = accent.unwrap_or_else(|| {
                Color32::from_rgb(
                    ((base.accent.r() as u16 + 255) / 2) as u8,
                    ((base.accent.g() as u16 + 255) / 2) as u8,
                    ((base.accent.b() as u16 + 255) / 2) as u8,
                )
            });
            text_at(y, a, FontId::proportional(if wide { 17.0 } else { 15.0 }), c);
            y += if wide { 26.0 } else { 22.0 };
        }
        if !album_line.is_empty() {
            text_at(y, &album_line, FontId::proportional(if wide { 14.0 } else { 13.0 }), t.text_dim);
            y += 22.0;
        }
        y += 20.0;
        let rest_h = col.bottom() - y;

        // -- list
        let header_h = 26.0;
        let fits = ((rest_h - header_h) / row_h).floor().max(0.0) as usize;
        let list_shown = can_list && !self.np_info && fits >= 1;
        let mut play: Option<usize> = None;
        if list_shown {
            let hr = Rect::from_min_size(pos2(col.left(), y), vec2(col.width(), header_h));
            let icon = match self.queue_origin.kind {
                _ if look.numbers => ph::VINYL_RECORD,
                OriginKind::Album => ph::VINYL_RECORD,
                OriginKind::Folder => ph::FOLDER,
                OriginKind::Playlist => ph::PLAYLIST,
                OriginKind::Artist => ph::MICROPHONE_STAGE,
                OriginKind::Genre => ph::TAG,
                OriginKind::Queue => ph::QUEUE,
            };
            let o = &self.queue_origin;
            // One album: its name from the tags beats the folder's.
            let album = album_tag.as_deref().filter(|_| look.numbers && matches!(o.kind, OriginKind::Folder | OriginKind::Queue));
            let mut label = match album {
                Some(a) => a.to_string(),
                None if o.name.is_empty() => t!("Warteschlange").to_string(),
                None => o.name.clone(),
            };
            if o.added > 0 && !o.name.is_empty() {
                label.push_str(&format!(" + {}", o.added));
            }
            let save = n >= 2 && (o.name.is_empty() || o.added > 0);
            // right side: position and time left
            let mut right = format!("{} / {n}", pos + 1);
            let durs: Vec<f64> = list.iter().skip(pos).take(500).map(|e| self.entry_dur(&e.filename)).collect();
            if n - pos <= 500 && !durs.is_empty() && durs.iter().all(|d| *d > 0.0) {
                let left = durs.iter().sum::<f64>() - st.time.max(0.0);
                right.push_str(&format!(" · {}", tf!("noch {}", dur_text(left.max(0.0)))));
            }
            let btn_w = 26.0;
            let mut bx = hr.right();
            let mut btn = |ui: &mut Ui, icon: &str, tip: &str, id: &str| -> bool {
                let r = Rect::from_min_max(pos2(bx - btn_w, hr.top()), pos2(bx, hr.bottom()));
                bx -= btn_w + 2.0;
                let resp = ui.interact(r, Id::new(("np_btn", id)), Sense::click()).on_hover_text(tip);
                if resp.hovered() {
                    painter.rect_filled(r, CornerRadius::same(theme::R), t.hover);
                }
                painter.text(r.center(), Align2::CENTER_CENTER, icon, theme::icon(15.0), if resp.hovered() { t.text } else { t.text_dim });
                resp.clicked()
            };
            if btn(ui, ph::INFO, &t!("Details"), "info") {
                self.np_info = true;
            }
            if save && btn(ui, ph::FLOPPY_DISK, &t!("Als Playlist speichern"), "save") {
                let items: Vec<String> = list.iter().map(|e| e.filename.clone()).collect();
                self.dlg.playlist_name = String::new();
                self.dialog = Some(crate::app::Dialog::NewPlaylist(items));
            }
            let rg = painter.layout_no_wrap(right, FontId::proportional(11.5), t.text_dim);
            let rx = bx - 6.0 - rg.size().x;
            painter.galley(pos2(rx, hr.center().y - rg.size().y / 2.0), rg, t.text_dim);
            painter.text(pos2(hr.left() + 8.0, hr.center().y), Align2::LEFT_CENTER, icon, theme::icon(13.0), t.text_dim);
            widgets::painter_text_trunc(&painter, pos2(hr.left() + 28.0, hr.center().y - 7.5), &label, FontId::proportional(12.0), t.text_dim, (rx - hr.left() - 36.0).max(10.0));
            y += header_h;

            // one title before the current one, the rest after it
            let count = fits.min(n).min(8);
            let from = pos.saturating_sub(1).min(n.saturating_sub(count));
            for i in from..(from + count).min(n) {
                let e = &list[i];
                let rr = Rect::from_min_size(pos2(col.left(), y), vec2(col.width(), row_h));
                y += row_h;
                let current = e.current || i as i64 == st.playlist_pos;
                let resp = ui.interact(rr, Id::new(("np_row", i)), Sense::click());
                if current {
                    painter.rect_filled(rr, CornerRadius::same(theme::R_LG), t.accent_soft);
                } else if resp.hovered() {
                    painter.rect_filled(rr, CornerRadius::same(theme::R_LG), t.hover);
                }
                let info = self.row_info(&ctx, e, true);
                let past = i < pos;
                let mut tt = t;
                if past {
                    tt.text = t.text_faint;
                    tt.text_dim = t.text_faint;
                }
                paint_row(&painter, &tt, rr, &info, look, i, current, st.paused, 48.0);
                if info.dur > 0.0 {
                    painter.text(pos2(rr.right() - 8.0, rr.center().y), Align2::RIGHT_CENTER, media::fmt_time_short(info.dur), FontId::proportional(11.5), t.text_faint);
                }
                if resp.clicked() && !current {
                    play = Some(i);
                }
            }
        }

        // -- details: instead of the list, or below it when there is room
        let line = 21.0;
        let room = col.bottom() - y - 14.0;
        if !list_shown || room >= line * details.len() as f32 {
            if list_shown {
                y += 14.0;
            }
            if can_list && self.np_info {
                let hr = Rect::from_min_size(pos2(col.left(), y), vec2(col.width(), header_h));
                let r = Rect::from_min_max(pos2(hr.right() - 26.0, hr.top()), hr.max);
                let resp = ui.interact(r, Id::new(("np_btn", "list")), Sense::click()).on_hover_text(t!("Warteschlange"));
                if resp.hovered() {
                    painter.rect_filled(r, CornerRadius::same(theme::R), t.hover);
                }
                painter.text(r.center(), Align2::CENTER_CENTER, ph::QUEUE, theme::icon(15.0), if resp.hovered() { t.text } else { t.text_dim });
                if resp.clicked() {
                    self.np_info = false;
                }
                painter.text(pos2(hr.left() + 8.0, hr.center().y), Align2::LEFT_CENTER, t!("Details"), FontId::proportional(12.0), t.text_dim);
                y += header_h + 4.0;
            }
            let label_w = 92.0;
            let x0 = if wide { col.left() + 8.0 } else { col.center().x - (label_w + 180.0) / 2.0 };
            for (k, v) in &details {
                if y + line > col.bottom() + 4.0 {
                    break;
                }
                painter.text(pos2(x0, y), Align2::LEFT_TOP, k, FontId::proportional(12.0), t.text_faint);
                widgets::painter_text_trunc(&painter, pos2(x0 + label_w, y), v, FontId::proportional(12.0), t.text_dim, (col.right() - x0 - label_w).max(10.0));
                y += line;
            }
        }

        if let Some(i) = play {
            if let Some(p) = &self.player {
                p.ctl.playlist_play(i);
                p.ctl.set_pause(false);
            }
        }
    }

    /// Keeps `np_art` on the current title: has ffmpeg read its cover once
    /// mpv reports one, and picks the result up.
    fn update_np_art(&mut self, ctx: &egui::Context, st: &PlayerState) {
        use std::sync::mpsc::{TryRecvError, channel};
        let now = ctx.input(|i| i.time);
        if self.np_art.as_ref().map_or(true, |a| a.path != st.path) {
            let old = self.np_art.take().and_then(|a| match a.backdrop {
                Some(b) => Some((b, a.accent, a.cover)),
                None => a.old,
            });
            self.np_art = Some(NpArt { path: st.path.clone(), asked: false, rx: None, cover: None, backdrop: None, accent: None, old, since: now });
        }
        let ffmpeg = self.tools.ffmpeg.clone();
        let probed = self.tools.probe_cache.get(&st.path).is_some_and(|p| p.has_cover)
            || (self.tools.ffprobe.is_none() && media::is_audio(&st.path));
        let Some(a) = self.np_art.as_mut() else { return };
        if !a.asked && (st.has_cover() || probed) && !media::is_url(&st.path) {
            a.asked = true;
            let (tx, rx) = channel();
            let (path, c) = (st.path.clone(), ctx.clone());
            std::thread::spawn(move || {
                let _ = tx.send(media::cover_art(ffmpeg.as_deref(), &path, ART).and_then(analyse));
                c.request_repaint();
            });
            a.rx = Some(rx);
        }
        let got = match a.rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(px)) => Some(px),
            Some(Err(TryRecvError::Disconnected)) => Some(None),
            _ => None,
        };
        if let Some(px) = got {
            a.rx = None;
            if let Some(px) = px {
                let img = |side: usize, rgba: &[u8]| egui::ColorImage::from_rgba_unmultiplied([side, side], rgba);
                a.cover = Some(ctx.load_texture("np_cover", img(ART, &px.cover), egui::TextureOptions::LINEAR));
                a.backdrop = Some(ctx.load_texture("np_backdrop", img(BACK, &px.backdrop), egui::TextureOptions::LINEAR));
                a.accent = px.accent;
                a.since = now;
            }
        }
    }

    fn entry_dur(&self, f: &str) -> f64 {
        crate::plex::remote(f).map(|r| r.duration).or_else(|| self.tools.probe_cache.get(f).map(|p| p.duration)).unwrap_or(0.0)
    }

    /// Label and value pairs for the details block.
    fn np_details(&self, st: &PlayerState) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = Vec::new();
        let mut add = |k: String, val: Option<String>| {
            if let Some(val) = val.filter(|s| !s.trim().is_empty()) {
                v.push((k, val));
            }
        };
        // ffprobe's tags while mpv still opens the file
        let probe = if media::is_url(&st.path) { None } else { self.tools.probe_cache.get(&st.path) };
        add(t!("Genre").to_string(), st.meta("genre").map(str::to_string).or_else(|| probe.and_then(|p| p.genre.clone())));
        let track = st.meta("track").map(|tr| match st.meta("disc").filter(|d| *d != "1" && *d != "1/1") {
            Some(d) => format!("{tr} · CD {d}"),
            None => tr.to_string(),
        });
        let track = track.or_else(|| probe.and_then(|p| p.track).map(|n| n.to_string()));
        add(t!("Titelnummer").to_string(), track);
        add(t!("Komponist").to_string(), st.meta("composer").map(str::to_string));
        let codec = st.audio_codec.to_uppercase();
        let size = (st.file_size > 0).then(|| format!("{:.1} MB", st.file_size as f64 / 1_048_576.0));
        add(t!("Format").to_string(), Some([Some(codec).filter(|c| !c.is_empty()), size].into_iter().flatten().collect::<Vec<_>>().join(" · ")));
        let file = if media::is_url(&st.path) { None } else { Path::new(&st.path).file_name().map(|s| s.to_string_lossy().to_string()) };
        add(t!("Datei").to_string(), file);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_names() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let o = QueueOrigin::of_list(&s(&["C:/Musik/Neon Hours/01.flac", "C:/Musik/Neon Hours/02.flac"]));
        assert_eq!((o.kind, o.name.as_str()), (OriginKind::Folder, "Neon Hours"));
        let o = QueueOrigin::of_list(&s(&["C:/Listen/Autofahrt.m3u"]));
        assert_eq!((o.kind, o.name.as_str()), (OriginKind::Playlist, "Autofahrt"));
        let o = QueueOrigin::of_list(&s(&["C:/a/1.mp3", "C:/b/2.mp3"]));
        assert_eq!(o.kind, OriginKind::Queue);
        assert_eq!(QueueOrigin::of_list(&s(&["C:/a/1.mp3"])).kind, OriginKind::Queue);
    }

    #[test]
    fn cover_crop() {
        // cover padded into a 16:9 thumbnail: the middle square
        let uv = crop_uv(320.0, 180.0, 1.0);
        assert!((uv.min.x - 0.21875).abs() < 1e-4 && (uv.max.x - 0.78125).abs() < 1e-4);
    }
}
