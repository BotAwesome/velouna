//! Plex data model, parsed leniently from the JSON the server returns
//! (`Accept: application/json`). Unknown or missing fields fall back to defaults.

use serde_json::Value;

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}
fn i(v: &Value, k: &str) -> i64 {
    match v.get(k) {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
        Some(Value::String(x)) => x.parse().unwrap_or(0),
        Some(Value::Bool(b)) => *b as i64,
        _ => 0,
    }
}
fn oi(v: &Value, k: &str) -> Option<i64> {
    v.get(k).filter(|x| !x.is_null()).map(|_| i(v, k))
}
fn of(v: &Value, k: &str) -> Option<f64> {
    match v.get(k) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(x)) => x.parse().ok(),
        _ => None,
    }
}
fn b(v: &Value, k: &str) -> bool {
    match v.get(k) {
        Some(Value::Bool(x)) => *x,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        Some(Value::String(x)) => x == "1" || x == "true",
        _ => false,
    }
}
fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}
fn tags(v: &Value, k: &str) -> Vec<String> {
    arr(v, k).iter().map(|t| s(t, "tag")).filter(|t| !t.is_empty()).collect()
}

/// The `MediaContainer` object of a response.
pub fn container(v: &Value) -> &Value {
    v.get("MediaContainer").unwrap_or(v)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Kind {
    Movie,
    Show,
    Season,
    Episode,
    Collection,
    Playlist,
    Artist,
    Album,
    Track,
    Clip,
    Photo,
    #[default]
    Other,
}

impl Kind {
    pub fn parse(t: &str) -> Kind {
        match t {
            "movie" => Kind::Movie,
            "show" => Kind::Show,
            "season" => Kind::Season,
            "episode" => Kind::Episode,
            "collection" => Kind::Collection,
            "playlist" => Kind::Playlist,
            "artist" => Kind::Artist,
            "album" => Kind::Album,
            "track" => Kind::Track,
            "clip" => Kind::Clip,
            "photo" => Kind::Photo,
            _ => Kind::Other,
        }
    }
    /// Can be handed to the player directly.
    pub fn playable(self) -> bool {
        matches!(self, Kind::Movie | Kind::Episode | Kind::Track | Kind::Clip)
    }
    /// Shown as a 16:9 still rather than a 2:3 poster.
    pub fn landscape(self) -> bool {
        matches!(self, Kind::Episode | Kind::Clip)
    }
    pub fn square(self) -> bool {
        matches!(self, Kind::Artist | Kind::Album | Kind::Track | Kind::Playlist)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Rating {
    pub source: String,
    pub value: f64,
    pub audience: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Role {
    pub name: String,
    pub role: String,
    pub thumb: String,
}

#[derive(Clone, Debug, Default)]
pub struct StreamInfo {
    /// 1 video, 2 audio, 3 subtitle
    pub stream_type: i64,
    pub codec: String,
    pub language: String,
    pub title: String,
    pub display: String,
    pub channels: i64,
    pub selected: bool,
    pub default: bool,
    pub forced: bool,
    pub external_key: String,
}

#[derive(Clone, Debug, Default)]
pub struct Part {
    pub id: i64,
    pub key: String,
    pub file: String,
    pub size: i64,
    pub duration_ms: i64,
    pub container: String,
    pub streams: Vec<StreamInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct MediaVersion {
    pub id: i64,
    pub resolution: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub audio_channels: i64,
    pub container: String,
    pub bitrate: i64,
    pub width: i64,
    pub height: i64,
    pub hdr: String,
    pub parts: Vec<Part>,
}

impl MediaVersion {
    pub fn resolution_label(&self) -> String {
        let r = self.resolution.to_lowercase();
        match r.as_str() {
            "4k" => "4K".into(),
            "" => String::new(),
            x if x.chars().all(|c| c.is_ascii_digit()) => format!("{x}p"),
            "sd" => "SD".into(),
            _ => self.resolution.clone(),
        }
    }
    pub fn label(&self) -> String {
        let mut v = vec![];
        let res = self.resolution_label();
        if !res.is_empty() {
            v.push(res);
        }
        if !self.hdr.is_empty() {
            v.push(self.hdr.clone());
        }
        if !self.video_codec.is_empty() {
            v.push(codec_name(&self.video_codec));
        }
        v.join(" · ")
    }
}

pub fn codec_name(c: &str) -> String {
    match c.to_lowercase().as_str() {
        "h264" => "H.264".into(),
        "hevc" | "h265" => "HEVC".into(),
        "av1" => "AV1".into(),
        "mpeg4" => "MPEG-4".into(),
        "mpeg2video" => "MPEG-2".into(),
        "vc1" => "VC-1".into(),
        "eac3" => "E-AC3".into(),
        "ac3" => "AC3".into(),
        "dca" | "dts" => "DTS".into(),
        "truehd" => "TrueHD".into(),
        "aac" => "AAC".into(),
        "flac" => "FLAC".into(),
        "mp3" => "MP3".into(),
        "opus" => "Opus".into(),
        "srt" | "subrip" => "SRT".into(),
        "ass" => "ASS".into(),
        "pgs" => "PGS".into(),
        x => x.to_uppercase(),
    }
}

pub fn channels_name(n: i64) -> String {
    match n {
        0 => String::new(),
        1 => "Mono".into(),
        2 => "Stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        x => tf!("{x} Kanäle"),
    }
}

#[derive(Clone, Debug, Default)]
pub struct Item {
    pub rating_key: String,
    pub key: String,
    pub kind: Kind,
    pub subtype: String,
    pub title: String,
    pub title_sort: String,
    pub original_title: String,
    pub parent_title: String,
    pub grandparent_title: String,
    pub parent_key: String,
    pub grandparent_key: String,
    pub index: Option<i64>,
    pub parent_index: Option<i64>,
    pub year: Option<i64>,
    pub summary: String,
    pub tagline: String,
    pub studio: String,
    pub content_rating: String,
    pub duration_ms: i64,
    pub view_offset_ms: i64,
    pub view_count: i64,
    pub leaf_count: i64,
    pub viewed_leaf_count: i64,
    pub child_count: i64,
    pub thumb: String,
    pub art: String,
    pub parent_thumb: String,
    pub grandparent_thumb: String,
    pub grandparent_art: String,
    pub composite: String,
    pub originally_available: String,
    pub added_at: i64,
    pub ratings: Vec<Rating>,
    pub audience_rating: Option<f64>,
    pub rating: Option<f64>,
    pub audience_rating_image: String,
    pub rating_image: String,
    pub genres: Vec<String>,
    pub directors: Vec<String>,
    pub writers: Vec<String>,
    pub roles: Vec<Role>,
    pub collections: Vec<String>,
    pub media: Vec<MediaVersion>,
    pub smart: bool,
    pub library_section_id: String,
    pub library_section_title: String,
    /// Next episode to watch (shows, with `includeOnDeck=1`).
    pub on_deck: Option<Box<Item>>,
}

impl Item {
    pub fn parse(v: &Value) -> Item {
        let media = arr(v, "Media")
            .iter()
            .map(|m| MediaVersion {
                id: i(m, "id"),
                resolution: s(m, "videoResolution"),
                video_codec: s(m, "videoCodec"),
                audio_codec: s(m, "audioCodec"),
                audio_channels: i(m, "audioChannels"),
                container: s(m, "container"),
                bitrate: i(m, "bitrate"),
                width: i(m, "width"),
                height: i(m, "height"),
                hdr: {
                    let dp = s(m, "videoDynamicRange");
                    if !dp.is_empty() && dp != "SDR" { dp } else if b(m, "DOVIPresent") { "Dolby Vision".into() } else { String::new() }
                },
                parts: arr(m, "Part")
                    .iter()
                    .map(|p| Part {
                        id: i(p, "id"),
                        key: s(p, "key"),
                        file: s(p, "file"),
                        size: i(p, "size"),
                        duration_ms: i(p, "duration"),
                        container: s(p, "container"),
                        streams: arr(p, "Stream")
                            .iter()
                            .map(|st| StreamInfo {
                                stream_type: i(st, "streamType"),
                                codec: s(st, "codec"),
                                language: {
                                    let l = s(st, "language");
                                    if l.is_empty() { s(st, "languageTag") } else { l }
                                },
                                title: s(st, "title"),
                                display: s(st, "displayTitle"),
                                channels: i(st, "channels"),
                                selected: b(st, "selected"),
                                default: b(st, "default"),
                                forced: b(st, "forced"),
                                external_key: s(st, "key"),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect();
        let mut ratings: Vec<Rating> = arr(v, "Rating")
            .iter()
            .map(|r| Rating { source: s(r, "image"), value: of(r, "value").unwrap_or(0.0), audience: s(r, "type") == "audience" })
            .filter(|r| r.value > 0.0)
            .collect();
        if ratings.is_empty() {
            if let Some(x) = of(v, "rating") {
                ratings.push(Rating { source: s(v, "ratingImage"), value: x, audience: false });
            }
            if let Some(x) = of(v, "audienceRating") {
                ratings.push(Rating { source: s(v, "audienceRatingImage"), value: x, audience: true });
            }
        }
        Item {
            rating_key: s(v, "ratingKey"),
            key: s(v, "key"),
            kind: Kind::parse(&s(v, "type")),
            subtype: s(v, "subtype"),
            title: s(v, "title"),
            title_sort: s(v, "titleSort"),
            original_title: s(v, "originalTitle"),
            parent_title: s(v, "parentTitle"),
            grandparent_title: s(v, "grandparentTitle"),
            parent_key: s(v, "parentRatingKey"),
            grandparent_key: s(v, "grandparentRatingKey"),
            index: oi(v, "index"),
            parent_index: oi(v, "parentIndex"),
            year: oi(v, "year").filter(|y| *y > 0),
            summary: s(v, "summary"),
            tagline: s(v, "tagline"),
            studio: s(v, "studio"),
            content_rating: s(v, "contentRating"),
            duration_ms: i(v, "duration"),
            view_offset_ms: i(v, "viewOffset"),
            view_count: i(v, "viewCount"),
            leaf_count: i(v, "leafCount"),
            viewed_leaf_count: i(v, "viewedLeafCount"),
            child_count: i(v, "childCount"),
            thumb: s(v, "thumb"),
            art: s(v, "art"),
            parent_thumb: s(v, "parentThumb"),
            grandparent_thumb: s(v, "grandparentThumb"),
            grandparent_art: s(v, "grandparentArt"),
            composite: s(v, "composite"),
            originally_available: s(v, "originallyAvailableAt"),
            added_at: i(v, "addedAt"),
            ratings,
            audience_rating: of(v, "audienceRating"),
            rating: of(v, "rating"),
            audience_rating_image: s(v, "audienceRatingImage"),
            rating_image: s(v, "ratingImage"),
            genres: tags(v, "Genre"),
            directors: tags(v, "Director"),
            writers: tags(v, "Writer"),
            roles: arr(v, "Role").iter().map(|r| Role { name: s(r, "tag"), role: s(r, "role"), thumb: s(r, "thumb") }).collect(),
            collections: tags(v, "Collection"),
            media,
            smart: b(v, "smart"),
            library_section_id: s(v, "librarySectionID"),
            library_section_title: s(v, "librarySectionTitle"),
            on_deck: v
                .get("OnDeck")
                .and_then(|o| o.get("Metadata"))
                .and_then(|m| if m.is_array() { m.get(0) } else { Some(m) })
                .map(|m| Box::new(Item::parse(m))),
        }
    }

    pub fn watched(&self) -> bool {
        match self.kind {
            Kind::Show | Kind::Season => self.leaf_count > 0 && self.viewed_leaf_count >= self.leaf_count,
            _ => self.view_count > 0 && self.view_offset_ms == 0,
        }
    }
    pub fn unwatched_leaves(&self) -> i64 {
        (self.leaf_count - self.viewed_leaf_count).max(0)
    }
    /// 0..1 progress for partially watched items.
    pub fn progress(&self) -> Option<f32> {
        if self.view_offset_ms > 0 && self.duration_ms > 0 {
            Some((self.view_offset_ms as f32 / self.duration_ms as f32).clamp(0.0, 1.0))
        } else {
            None
        }
    }
    pub fn duration(&self) -> f64 {
        self.duration_ms as f64 / 1000.0
    }
    pub fn view_offset(&self) -> f64 {
        self.view_offset_ms as f64 / 1000.0
    }
    /// "S1 E3" for episodes.
    pub fn episode_code(&self) -> String {
        match (self.parent_index, self.index) {
            (Some(s), Some(e)) => format!("S{s} E{e}"),
            (None, Some(e)) => format!("E{e}"),
            _ => String::new(),
        }
    }
    /// Title as shown on a card: episodes are listed under their show.
    pub fn card_title(&self) -> String {
        match self.kind {
            Kind::Episode if !self.grandparent_title.is_empty() => self.grandparent_title.clone(),
            Kind::Season if !self.parent_title.is_empty() => self.parent_title.clone(),
            _ => self.title.clone(),
        }
    }
    /// Secondary line on a card.
    pub fn card_sub(&self) -> String {
        match self.kind {
            Kind::Episode => {
                let code = self.episode_code();
                if code.is_empty() { self.title.clone() } else { format!("{code} · {}", self.title) }
            }
            Kind::Season => self.title.clone(),
            Kind::Show => {
                let mut v = vec![];
                if let Some(y) = self.year {
                    v.push(y.to_string());
                }
                if self.child_count > 0 {
                    v.push(if self.child_count == 1 { t!("1 Staffel").into() } else { tf!("{} Staffeln", self.child_count) });
                }
                v.join(" · ")
            }
            Kind::Collection | Kind::Playlist => {
                let n = if self.leaf_count > 0 { self.leaf_count } else { self.child_count };
                if n == 1 { t!("1 Element").into() } else { tf!("{n} Elemente") }
            }
            Kind::Album => self.parent_title.clone(),
            Kind::Track => self.grandparent_title.clone(),
            _ => self.year.map(|y| y.to_string()).unwrap_or_default(),
        }
    }
    /// Title used for the player (window title, queue).
    pub fn play_title(&self) -> String {
        match self.kind {
            Kind::Episode => {
                let code = self.episode_code();
                format!("{} – {}{}", self.grandparent_title, if code.is_empty() { String::new() } else { format!("{code} · ") }, self.title)
            }
            Kind::Track if !self.grandparent_title.is_empty() => format!("{} – {}", self.grandparent_title, self.title),
            Kind::Movie => match self.year {
                Some(y) => format!("{} ({y})", self.title),
                None => self.title.clone(),
            },
            _ => self.title.clone(),
        }
    }
    /// Poster image for grid cards.
    pub fn poster(&self) -> &str {
        match self.kind {
            Kind::Episode if !self.grandparent_thumb.is_empty() => &self.grandparent_thumb,
            Kind::Playlist if !self.composite.is_empty() && self.thumb.is_empty() => &self.composite,
            _ => &self.thumb,
        }
    }
    /// Wide still for 16:9 cards.
    pub fn still(&self) -> &str {
        if !self.thumb.is_empty() && matches!(self.kind, Kind::Episode | Kind::Clip) {
            &self.thumb
        } else if !self.art.is_empty() {
            &self.art
        } else if !self.grandparent_art.is_empty() {
            &self.grandparent_art
        } else {
            &self.thumb
        }
    }
    pub fn backdrop(&self) -> &str {
        if !self.art.is_empty() {
            &self.art
        } else if !self.grandparent_art.is_empty() {
            &self.grandparent_art
        } else {
            &self.thumb
        }
    }
    pub fn first_part(&self) -> Option<&Part> {
        self.media.first().and_then(|m| m.parts.first())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Section {
    pub key: String,
    pub kind: String,
    pub title: String,
    pub agent: String,
}

impl Section {
    pub fn parse(v: &Value) -> Section {
        Section { key: s(v, "key"), kind: s(v, "type"), title: s(v, "title"), agent: s(v, "agent") }
    }
    /// Plex `type` number used for `/all?type=`.
    pub fn type_num(&self) -> u8 {
        match self.kind.as_str() {
            "movie" => 1,
            "show" => 2,
            "artist" => 8,
            "photo" => 13,
            _ => 1,
        }
    }
    pub fn noun_plural(&self) -> &'static str {
        match self.kind.as_str() {
            "movie" => t!("Filme"),
            "show" => t!("Serien"),
            "artist" => t!("Interpreten"),
            "photo" => t!("Fotos"),
            _ => if crate::i18n::is_en() { "items" } else { "Titel" },
        }
    }
    pub fn icon(&self) -> &'static str {
        use egui_phosphor::regular as ph;
        match self.kind.as_str() {
            "movie" => ph::FILM_SLATE,
            "show" => ph::TELEVISION_SIMPLE,
            "artist" => ph::MUSIC_NOTES,
            "photo" => ph::IMAGE,
            _ => ph::FOLDER_SIMPLE,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Hub {
    pub title: String,
    pub key: String,
    pub identifier: String,
    pub kind: String,
    pub more: bool,
    pub size: i64,
    pub items: Vec<Item>,
}

impl Hub {
    pub fn parse(v: &Value) -> Hub {
        Hub {
            title: s(v, "title"),
            key: s(v, "key"),
            identifier: s(v, "hubIdentifier"),
            kind: s(v, "type"),
            more: b(v, "more"),
            size: i(v, "size"),
            items: arr(v, "Metadata").iter().chain(arr(v, "Directory").iter()).map(Item::parse).collect(),
        }
    }
}

pub fn parse_items(v: &Value) -> (Vec<Item>, i64) {
    let c = container(v);
    let items: Vec<Item> = arr(c, "Metadata").iter().chain(arr(c, "Directory").iter()).map(Item::parse).collect();
    let total = oi(c, "totalSize").unwrap_or(items.len() as i64);
    (items, total)
}

pub fn parse_hubs(v: &Value) -> Vec<Hub> {
    arr(container(v), "Hub").iter().map(Hub::parse).filter(|h| !h.items.is_empty()).collect()
}

pub fn parse_sections(v: &Value) -> Vec<Section> {
    arr(container(v), "Directory").iter().map(Section::parse).collect()
}

/// `(label, count)` for the A–Z jump bar.
pub fn parse_first_chars(v: &Value) -> Vec<(String, i64)> {
    arr(container(v), "Directory").iter().map(|d| (s(d, "title"), i(d, "size"))).collect()
}

/// Simple filter values (genres, years …): `(title, key)`.
pub fn parse_filter_values(v: &Value) -> Vec<(String, String)> {
    arr(container(v), "Directory").iter().map(|d| (s(d, "title"), s(d, "key"))).collect()
}

#[derive(Clone, Debug, Default)]
pub struct Connection {
    pub uri: String,
    pub address: String,
    pub port: i64,
    pub local: bool,
    pub relay: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub token: String,
    pub owned: bool,
    pub owner: String,
    pub presence: bool,
    pub connections: Vec<Connection>,
}

pub fn parse_resources(v: &Value) -> Vec<Server> {
    v.as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter(|r| s(r, "provides").split(',').any(|p| p == "server"))
        .map(|r| Server {
            id: s(r, "clientIdentifier"),
            name: s(r, "name"),
            token: s(r, "accessToken"),
            owned: b(r, "owned"),
            owner: s(r, "sourceTitle"),
            presence: b(r, "presence"),
            connections: arr(r, "connections")
                .iter()
                .map(|c| Connection { uri: s(c, "uri"), address: s(c, "address"), port: i(c, "port"), local: b(c, "local"), relay: b(c, "relay") })
                .collect(),
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct User {
    pub name: String,
    pub thumb: String,
}

pub fn parse_user(v: &Value) -> User {
    let name = {
        let t = s(v, "friendlyName");
        if t.is_empty() { s(v, "username") } else { t }
    };
    User { name, thumb: s(v, "thumb") }
}
