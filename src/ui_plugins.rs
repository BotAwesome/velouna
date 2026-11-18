//! Extensions page in the settings and the install / remove dialogs.
//! One row per plugin: what it is, whether it runs, what is wrong with it.
//! Everything rare (folder, remove) sits behind the row's "…" menu.

use crate::app::{App, Dialog};
use crate::plugins::repo::{self, Fetched, Index, Repo, Version};
use crate::plugins::{self, Kind, Package};
use crate::theme;
use crate::widgets::{self, icon_button};
use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as ph;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ExtTab {
    #[default]
    Installed,
    Catalog,
    Repos,
    /// "Build your own repo" (desktop only).
    Build,
}

/// One plugin to fetch from a repo and offer for install.
#[derive(Clone)]
pub struct Job {
    repo: Repo,
    id: String,
    version: Version,
}

enum Msg {
    Index(String, Result<Fetched, String>),
    /// The repo is validly signed, but with another key than the one kept.
    KeyChanged(String, Fetched),
    Probe(Result<Fetched, String>),
    Downloaded(Job, Result<PathBuf, String>),
}

/// Repos and catalog: what was fetched, what is picked, what installs next.
pub struct RepoUi {
    pub tab: ExtTab,
    input: String,
    adding: bool,
    add_error: Option<String>,
    indexes: BTreeMap<String, Result<Index, String>>,
    /// Repos now signed with another key, with what they serve now.
    key_changed: BTreeMap<String, Fetched>,
    /// When the "accept new key" dialog opened (for its countdown).
    key_since: Option<f64>,
    loading: BTreeSet<String>,
    /// (repo url, plugin id) ticked in the catalog.
    picked: BTreeSet<(String, String)>,
    /// Plugin ids ticked in the "add repo" dialog.
    add_picked: BTreeSet<String>,
    queue: VecDeque<Job>,
    downloading: bool,
    ready: Option<Box<Package>>,
    /// The settings were open before a dialog of ours replaced them.
    back_to_settings: bool,
    /// "Build your own repo": plugin folder, repo folder, name, result.
    build_src: String,
    build_out: String,
    build_name: String,
    build_by: String,
    build_log: Option<Result<Vec<String>, String>>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for RepoUi {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tab: ExtTab::Installed,
            input: String::new(),
            adding: false,
            add_error: None,
            indexes: BTreeMap::new(),
            key_changed: BTreeMap::new(),
            key_since: None,
            loading: BTreeSet::new(),
            picked: BTreeSet::new(),
            add_picked: BTreeSet::new(),
            queue: VecDeque::new(),
            downloading: false,
            ready: None,
            back_to_settings: false,
            build_src: String::new(),
            build_out: String::new(),
            build_name: String::new(),
            build_by: String::new(),
            build_log: None,
            tx,
            rx,
        }
    }
}

/// Where a plugin in the catalog stands against what is installed.
enum Status {
    New,
    Update(String),
    Installed,
    /// Installed from another repo or a file: no updates from here.
    Elsewhere,
    TooNew,
}

pub(crate) const WARN: Color32 = Color32::from_rgb(0xE0, 0xA0, 0x30);

/// One line of the build form: same height whether it has a button or not.
fn form_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), Layout::left_to_right(Align::Center), add);
}

/// Left-aligned label of fixed width in front of a form field.
fn form_label(ui: &mut Ui, text: &str, w: f32, color: Color32) {
    ui.allocate_ui_with_layout(vec2(w, 30.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(w);
        ui.label(RichText::new(text).size(12.5).color(color));
    });
}

fn kind_icon(k: Kind) -> &'static str {
    match k {
        Kind::Visualizer => ph::SPARKLE,
        Kind::Lua => ph::PUZZLE_PIECE,
        Kind::Theme => ph::PALETTE,
    }
}

/// Text of a permission for the dialog, and whether it is critical.
fn permission(p: &str) -> (String, bool) {
    crate::ui_ext::permission_text(p)
}

impl App {
    /// The official repo first, then the ones the user added.
    pub fn plugin_repos(&self) -> Vec<Repo> {
        let off = Repo::official();
        let hidden = self.data.plugin_repos.iter().any(|r| r.url == off.url && r.hidden);
        let mut v = vec![Repo { hidden, ..off.clone() }];
        v.extend(self.data.plugin_repos.iter().filter(|r| r.url != off.url).cloned());
        v
    }

    fn load_repo(&mut self, ctx: &egui::Context, r: &Repo) {
        if !self.repo_ui.loading.insert(r.url.clone()) {
            return;
        }
        let (tx, url, key, c) = (self.repo_ui.tx.clone(), r.url.clone(), r.key.clone(), ctx.clone());
        std::thread::spawn(move || {
            // fetched against its own key first, so a changed key can be
            // told apart from a broken or tampered index
            let msg = match repo::fetch(&url, None) {
                Ok(f) if !key.is_empty() && f.index.key != key => Msg::KeyChanged(url, f),
                res => Msg::Index(url, res),
            };
            let _ = tx.send(msg);
            c.request_repaint();
        });
    }

    fn enqueue_install(&mut self, r: &Repo, id: &str) {
        let Some(Ok(idx)) = self.repo_ui.indexes.get(&r.url) else { return };
        let Some(v) = idx.plugins.iter().find(|e| e.id == id).and_then(|e| e.best()) else { return };
        if !self.repo_ui.queue.iter().any(|j| j.id == id) {
            self.repo_ui.queue.push_back(Job { repo: r.clone(), id: id.to_string(), version: v.clone() });
        }
    }

    /// Shows one of our dialogs, remembering to come back to the settings.
    fn show_ours(&mut self, d: Dialog) {
        if matches!(self.dialog, Some(Dialog::Settings)) {
            self.repo_ui.back_to_settings = true;
        }
        self.dialog = Some(d);
    }

    /// Every frame: picks up fetched indexes and downloads, and works the
    /// install queue one plugin at a time (each through its dialog).
    pub fn poll_repos(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.repo_ui.rx.try_recv() {
            match m {
                Msg::KeyChanged(url, f) => {
                    self.repo_ui.loading.remove(&url);
                    self.repo_ui.indexes.insert(url.clone(), Err(t!("Der Schlüssel dieses Repos hat sich geändert. Updates daraus sind gesperrt.").into()));
                    self.repo_ui.key_changed.insert(url, f);
                }
                Msg::Index(url, res) => {
                    self.repo_ui.loading.remove(&url);
                    self.repo_ui.key_changed.remove(&url);
                    if let Ok(f) = &res {
                        if let Some(r) = self.data.plugin_repos.iter_mut().find(|r| r.url == url && r.name != f.index.name) {
                            r.name = f.index.name.clone();
                            self.mark_dirty();
                        }
                    }
                    self.repo_ui.indexes.insert(url, res.map(|f| f.index));
                }
                Msg::Probe(res) => {
                    self.repo_ui.adding = false;
                    let have = res.as_ref().ok().and_then(|f| self.plugin_repos().into_iter().find(|r| r.url == f.url));
                    match (res, have) {
                        (Ok(f), Some(r)) if f.only.as_ref().map_or(r.only.is_empty(), |id| r.shows(id)) => {
                            self.repo_ui.add_error = Some(if f.only.is_some() { t!("Dieses Plugin ist schon hinzugefügt") } else { t!("Dieses Repo ist schon hinzugefügt") }.into());
                        }
                        (Ok(f), Some(r)) if r.key != f.index.key => {
                            self.repo_ui.add_error = Some(t!("Der Schlüssel dieses Repos hat sich geändert. Updates daraus sind gesperrt.").into());
                        }
                        (Ok(f), _) => {
                            self.repo_ui.add_error = None;
                            self.repo_ui.add_picked.clear();
                            if let Some(id) = &f.only {
                                self.repo_ui.add_picked.insert(id.clone());
                            }
                            self.show_ours(Dialog::RepoAdd(Box::new(f)));
                        }
                        (Err(e), _) => self.repo_ui.add_error = Some(e),
                    }
                }
                Msg::Downloaded(job, res) => {
                    self.repo_ui.downloading = false;
                    self.plugins.ensure();
                    let res = res.and_then(|file| plugins::inspect(&file, &self.plugins)).and_then(|mut pkg| {
                        let m = &pkg.manifest;
                        if m.id != job.id || m.version != job.version.version {
                            return Err(t!("Das Paket passt nicht zum Eintrag im Repo").to_string());
                        }
                        if !repo::same_permissions(&job.version.permissions, m) {
                            return Err(t!("Das Paket verlangt andere Berechtigungen als im Repo angegeben").to_string());
                        }
                        pkg.origin = Some(job.repo.clone());
                        Ok(pkg)
                    });
                    match res {
                        Ok(pkg) => self.repo_ui.ready = Some(Box::new(pkg)),
                        Err(e) => {
                            self.repo_ui.queue.clear();
                            self.show_ours(Dialog::Error(tf!("Plugin kann nicht installiert werden: {}", e)));
                        }
                    }
                }
            }
        }
        let free = matches!(self.dialog, None | Some(Dialog::Settings));
        if free {
            if let Some(pkg) = self.repo_ui.ready.take() {
                self.show_ours(Dialog::PluginInstall(pkg));
            } else if !self.repo_ui.downloading {
                if let Some(job) = self.repo_ui.queue.pop_front() {
                    self.repo_ui.downloading = true;
                    let (tx, c) = (self.repo_ui.tx.clone(), ctx.clone());
                    std::thread::spawn(move || {
                        let res = repo::download(&job.repo.url, &job.version);
                        let _ = tx.send(Msg::Downloaded(job, res));
                        c.request_repaint();
                    });
                } else if self.dialog.is_none() && self.repo_ui.back_to_settings {
                    self.repo_ui.back_to_settings = false;
                    self.dialog = Some(Dialog::Settings);
                }
            }
        }
    }

    /// Where plugin `e` of repo `r` stands against what is installed.
    fn status(&self, r: &Repo, e: &repo::Entry) -> Status {
        let Some(best) = e.best() else { return Status::TooNew };
        match self.plugins.get(&e.id) {
            None => Status::New,
            Some(_) if self.data.plugins.get(&e.id).map(|s| s.repo.as_str()).unwrap_or("") != r.url => Status::Elsewhere,
            Some(p) if plugins::version_lt(&p.manifest.version, &best.version) => Status::Update(best.version.clone()),
            Some(_) => Status::Installed,
        }
    }

    /// Settings → Extensions: tabs on top, then the chosen one.
    pub(crate) fn extensions_page(&mut self, ui: &mut Ui, filter: &str) {
        let t = self.t;
        let ctx = ui.ctx().clone();
        self.plugins.ensure();
        // repo indexes are fetched once per session, when first needed
        for r in self.plugin_repos() {
            if !r.hidden && !self.repo_ui.indexes.contains_key(&r.url) {
                self.load_repo(&ctx, &r);
            }
        }
        if !filter.is_empty() {
            self.installed_tab(ui, filter);
            return;
        }
        let updates = self.update_count();
        let repos = if updates > 0 { tf!("Repos ({})", updates) } else { t!("Repos").to_string() };
        let mut tab = self.repo_ui.tab;
        if crate::platform::MOBILE && tab == ExtTab::Build {
            tab = ExtTab::Installed;
        }
        let mut tabs = vec![(ExtTab::Installed, ph::PUZZLE_PIECE, t!("Installiert")), (ExtTab::Catalog, ph::STOREFRONT, t!("Katalog")), (ExtTab::Repos, ph::LINK, repos.as_str())];
        if !crate::platform::MOBILE {
            tabs.push((ExtTab::Build, ph::HAMMER, t!("Eigenes Repo")));
        }
        widgets::tabs(ui, &t, &mut tab, &tabs);
        self.repo_ui.tab = tab;
        ui.add_space(4.0);
        match tab {
            ExtTab::Installed => self.installed_tab(ui, filter),
            ExtTab::Catalog => self.catalog_tab(ui),
            ExtTab::Repos => self.repos_tab(ui),
            ExtTab::Build => self.build_tab(ui),
        }
    }

    /// The update a plugin can get from the repo it came from.
    fn update_for(&self, id: &str) -> Option<(Repo, String)> {
        let from = self.data.plugins.get(id).map(|s| s.repo.clone()).filter(|u| !u.is_empty())?;
        let r = self.plugin_repos().into_iter().find(|r| r.url == from)?;
        let idx = self.repo_ui.indexes.get(&r.url)?.as_ref().ok()?;
        let e = idx.plugins.iter().find(|e| e.id == id)?;
        match self.status(&r, e) {
            Status::Update(v) => Some((r, v)),
            _ => None,
        }
    }

    fn update_count(&self) -> usize {
        self.plugins.list.iter().filter(|p| self.update_for(&p.manifest.id).is_some()).count()
    }

    fn catalog_tab(&mut self, ui: &mut Ui) {
        let t = self.t;
        let repos: Vec<Repo> = self.plugin_repos().into_iter().filter(|r| !r.hidden).collect();
        let picked_n = self.repo_ui.picked.len();
        let busy = self.repo_ui.downloading || !self.repo_ui.queue.is_empty() || self.repo_ui.ready.is_some();
        ui.horizontal(|ui| {
            let label = if picked_n > 0 { tf!("Ausgewählte installieren ({})", picked_n) } else { t!("Ausgewählte installieren").to_string() };
            let resp = ui.add_enabled_ui(picked_n > 0 && !busy, |ui| widgets::icon_text_button(ui, &t, ph::DOWNLOAD_SIMPLE, &label)).inner;
            if resp.clicked() {
                let picked: Vec<(String, String)> = std::mem::take(&mut self.repo_ui.picked).into_iter().collect();
                for (url, id) in picked {
                    if let Some(r) = repos.iter().find(|r| r.url == url) {
                        self.enqueue_install(r, &id);
                    }
                }
            }
            if widgets::icon_text_button(ui, &t, ph::ARROWS_CLOCKWISE, t!("Neu laden")).clicked() {
                self.repo_ui.indexes.clear();
            }
            if busy {
                ui.add(egui::Spinner::new().size(14.0).color(t.accent));
                ui.label(RichText::new(t!("Lädt herunter …")).size(12.5).color(t.text_dim));
            }
        });
        ui.add_space(6.0);
        let mut any = false;
        for r in &repos {
            match self.repo_ui.indexes.get(&r.url).cloned() {
                None => {
                    plugin_row(ui, &t, ph::HOURGLASS, false, |ui| {
                        ui.label(RichText::new(repo_title(r)).font(theme::semibold(14.5)).color(t.text));
                        ui.label(RichText::new(t!("Lädt …")).size(12.5).color(t.text_dim));
                    }, |ui| {
                        ui.add(egui::Spinner::new().size(16.0).color(t.accent));
                    });
                }
                Some(Err(e)) => {
                    plugin_row(ui, &t, ph::WARNING, true, |ui| {
                        ui.label(RichText::new(repo_title(r)).font(theme::semibold(14.5)).color(t.text));
                        ui.add(egui::Label::new(RichText::new(e).size(12.5).color(t.danger)).wrap());
                    }, |_| {});
                }
                Some(Ok(idx)) => {
                    for e in idx.plugins.iter().filter(|e| r.shows(&e.id)) {
                        any = true;
                        let status = self.status(r, e);
                        let key = (r.url.clone(), e.id.clone());
                        let mut on = self.repo_ui.picked.contains(&key);
                        let ver = e.best().or(e.versions.first()).map(|v| v.version.clone()).unwrap_or_default();
                        let line = format!("{ver} · {} · {}", e.kind.label(), repo_title(r));
                        let mut desc = e.description.get();
                        let perms = e.best().map(|v| v.permissions.len()).unwrap_or(0);
                        if perms > 0 {
                            desc = format!("{desc}  ·  {}", tf!("{} Berechtigungen", perms));
                        }
                        let mut changed = false;
                        plugin_row(ui, &t, kind_icon(e.kind), false, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(e.name.get()).font(theme::semibold(14.5)).color(t.text));
                                ui.label(RichText::new(&line).size(12.0).color(t.text_faint));
                            });
                            ui.add(egui::Label::new(RichText::new(&desc).size(12.5).color(t.text_dim)).wrap());
                        }, |ui| match &status {
                            Status::New | Status::Update(_) => {
                                let label = if let Status::Update(v) = &status { tf!("Update {}", v) } else { t!("Installieren").to_string() };
                                changed = ui.checkbox(&mut on, RichText::new(label).size(12.5).color(t.text)).changed();
                            }
                            Status::Installed => {
                                ui.label(RichText::new(tf!("{}  Installiert", ph::CHECK)).size(12.5).color(t.text_dim));
                            }
                            Status::Elsewhere => {
                                ui.label(RichText::new(t!("Aus anderer Quelle installiert")).size(12.0).color(t.text_faint));
                            }
                            Status::TooNew => {
                                ui.label(RichText::new(t!("Braucht ein neueres Velouna")).size(12.0).color(t.text_faint));
                            }
                        });
                        if changed {
                            if on {
                                self.repo_ui.picked.insert(key);
                            } else {
                                self.repo_ui.picked.remove(&key);
                            }
                        }
                    }
                }
            }
        }
        if !any && repos.iter().all(|r| self.repo_ui.indexes.get(&r.url).is_some_and(|i| i.is_ok())) {
            ui.label(RichText::new(t!("In deinen Repos sind noch keine Plugins.")).size(12.5).color(t.text_dim));
        }
    }

    /// Plugins installed from repo `r` that it has a newer version of:
    /// (id, name, installed version, new version).
    fn updates_in(&self, r: &Repo) -> Vec<(String, String, String, String)> {
        let Some(Ok(idx)) = self.repo_ui.indexes.get(&r.url) else { return Vec::new() };
        idx.plugins
            .iter()
            .filter(|e| r.shows(&e.id))
            .filter_map(|e| match self.status(r, e) {
                Status::Update(v) => {
                    let p = self.plugins.get(&e.id)?;
                    Some((e.id.clone(), p.manifest.name.get(), p.manifest.version.clone(), v))
                }
                _ => None,
            })
            .collect()
    }

    /// Extensions → Repos: the repos you have and the updates they offer.
    fn repos_tab(&mut self, ui: &mut Ui) {
        let t = self.t;
        let busy = self.repo_ui.downloading || !self.repo_ui.queue.is_empty() || self.repo_ui.ready.is_some();
        let repos = self.plugin_repos();
        let all: Vec<(Repo, String)> = repos.iter().filter(|r| !r.hidden).flat_map(|r| self.updates_in(r).into_iter().map(move |u| (r.clone(), u.0))).collect();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if widgets::icon_text_button(ui, &t, ph::PLUS, t!("Repo hinzufügen…")).clicked() {
                self.repo_ui.add_error = None;
                self.show_ours(Dialog::RepoLink);
            }
            if widgets::icon_text_button(ui, &t, ph::ARROWS_CLOCKWISE, t!("Nach Updates suchen")).clicked() {
                self.repo_ui.indexes.clear();
            }
            if !all.is_empty() {
                let label = tf!("Alle aktualisieren ({})", all.len());
                if ui.add_enabled_ui(!busy, |ui| widgets::accent_button(ui, &t, &label)).inner.clicked() {
                    for (r, id) in &all {
                        self.enqueue_install(r, id);
                    }
                }
            }
            if busy {
                ui.add(egui::Spinner::new().size(14.0).color(t.accent));
                ui.label(RichText::new(t!("Lädt herunter …")).size(12.5).color(t.text_dim));
            }
        });
        ui.add_space(8.0);
        let mut remove: Option<String> = None;
        let mut toggle_hide: Option<(String, bool)> = None;
        let mut reload: Option<String> = None;
        let mut update: Option<(Repo, String)> = None;
        let mut take_key: Option<String> = None;
        let mut whole: Option<String> = None;
        for r in repos {
            let updates = if r.hidden { Vec::new() } else { self.updates_in(&r) };
            let state = match self.repo_ui.indexes.get(&r.url) {
                _ if r.hidden => t!("Ausgeblendet").to_string(),
                None => t!("Lädt …").to_string(),
                Some(Ok(i)) => {
                    let shown: Vec<&repo::Entry> = i.plugins.iter().filter(|e| r.shows(&e.id)).collect();
                    let mine = shown.iter().filter(|e| matches!(self.status(&r, e), Status::Installed | Status::Update(_))).count();
                    let mut s = if r.only.is_empty() {
                        tf!("{} Plugins", i.plugins.len())
                    } else {
                        let names: Vec<String> = shown.iter().map(|e| e.name.get()).collect();
                        tf!("Nur {}", names.join(", "))
                    };
                    if mine > 0 {
                        if r.only.is_empty() {
                            s = format!("{s}, {}", tf!("{} installiert", mine));
                        }
                        s = format!("{s}  ·  {}", match updates.len() {
                            0 => t!("alles aktuell").to_string(),
                            1 => t!("1 Update").to_string(),
                            n => tf!("{} Updates", n),
                        });
                    }
                    s
                }
                // the address is right above, no need to repeat it
                Some(Err(e)) => e.replace(&format!(": {}", r.url), "").replace(&r.url, "").trim_end_matches([' ', ':']).to_string(),
            };
            let bad = !r.hidden && matches!(self.repo_ui.indexes.get(&r.url), Some(Err(_)));
            let official = r.is_official();
            let changed = self.repo_ui.key_changed.get(&r.url);
            plugin_row(ui, &t, if official { ph::SEAL_CHECK } else { ph::LINK }, bad, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(repo_title(&r)).font(theme::semibold(14.5)).color(if r.hidden { t.text_dim } else { t.text }));
                    if official {
                        ui.label(RichText::new(t!("offiziell")).size(12.0).color(t.accent));
                    }
                });
                ui.add(egui::Label::new(RichText::new(&r.url).size(12.0).color(t.text_faint)).truncate());
                let key = tf!("Schlüssel {}", repo::fingerprint(&r.key));
                if let Some(f) = changed.filter(|_| bad) {
                    ui.add(egui::Label::new(RichText::new(&state).size(12.0).color(t.danger)).wrap());
                    ui.label(RichText::new(format!("{}  {}  {}", repo::fingerprint(&r.key), ph::ARROW_RIGHT, repo::fingerprint(&f.index.key))).font(egui::FontId::monospace(12.0)).color(t.text_dim));
                    if !official {
                        ui.add_space(4.0);
                        if widgets::ghost_button(ui, &t, t!("Neuen Schlüssel annehmen…")).clicked() {
                            take_key = Some(r.url.clone());
                        }
                    }
                } else if bad {
                    ui.add(egui::Label::new(RichText::new(&state).size(12.0).color(t.danger)).wrap());
                    ui.label(RichText::new(key).size(12.0).color(t.text_dim));
                } else {
                    ui.add(egui::Label::new(RichText::new(format!("{state}  ·  {key}")).size(12.0).color(t.text_dim)).wrap());
                }
                for (id, name, from, to) in &updates {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(ph::ARROW_CIRCLE_UP).font(theme::icon(15.0)).color(t.accent));
                        ui.label(RichText::new(name).size(13.0).color(t.text));
                        ui.label(RichText::new(format!("{from}  {}  {to}", ph::ARROW_RIGHT)).size(12.5).color(t.text_dim));
                        if ui.add_enabled_ui(!busy, |ui| widgets::ghost_button(ui, &t, t!("Aktualisieren"))).inner.clicked() {
                            update = Some((r.clone(), id.clone()));
                        }
                    });
                }
            }, |ui| {
                let more = icon_button(ui, &t, ph::DOTS_THREE, t!("Mehr"), false);
                egui::Popup::menu(&more).width(200.0).show(|ui| {
                    if widgets::menu_item(ui, &t, ph::ARROWS_CLOCKWISE, t!("Neu laden"), "", false).clicked() {
                        ui.close();
                        reload = Some(r.url.clone());
                    }
                    if !r.only.is_empty() && widgets::menu_item(ui, &t, ph::LIST_PLUS, t!("Ganzes Repo zeigen"), "", false).clicked() {
                        ui.close();
                        whole = Some(r.url.clone());
                    }
                    let (icon, label) = if r.hidden { (ph::EYE, t!("Einblenden")) } else { (ph::EYE_SLASH, t!("Ausblenden")) };
                    if widgets::menu_item(ui, &t, icon, label, "", false).clicked() {
                        ui.close();
                        toggle_hide = Some((r.url.clone(), !r.hidden));
                    }
                    if !official && widgets::menu_item(ui, &t, ph::TRASH, t!("Entfernen"), "", false).clicked() {
                        ui.close();
                        remove = Some(r.url.clone());
                    }
                });
            });
        }
        if let Some((r, id)) = update {
            self.enqueue_install(&r, &id);
        }
        if let Some(url) = whole {
            if let Some(r) = self.data.plugin_repos.iter_mut().find(|r| r.url == url) {
                r.only.clear();
                self.mark_dirty();
            }
        }
        if let Some(url) = take_key {
            self.repo_ui.key_since = None;
            self.show_ours(Dialog::RepoKey(url));
        }
        if let Some(url) = reload {
            self.repo_ui.indexes.remove(&url);
        }
        if let Some((url, hide)) = toggle_hide {
            match self.data.plugin_repos.iter_mut().find(|r| r.url == url) {
                Some(r) => r.hidden = hide,
                None => self.data.plugin_repos.push(Repo { hidden: hide, ..Repo::official() }),
            }
            self.mark_dirty();
        }
        if let Some(url) = remove {
            self.data.plugin_repos.retain(|r| r.url != url);
            self.repo_ui.indexes.remove(&url);
            self.repo_ui.key_changed.remove(&url);
            self.repo_ui.picked.retain(|(u, _)| *u != url);
            self.mark_dirty();
        }
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(t!("Plugins aus fremden Repos sind nicht von Velouna geprüft. Füge nur Repos hinzu, denen du vertraust.")).size(12.0).color(t.text_faint)).wrap());
    }

    /// A repo signed with a new key: both fingerprints, what it can mean,
    /// and an explicit yes. Never offered in the middle of an update.
    pub(crate) fn dlg_repo_key(&mut self, ui: &mut Ui, url: &str) {
        let t = self.t;
        crate::ui_dialogs::dlg_width(ui, 500.0);
        if self.dlg_title(ui, t!("Sicherheitsschlüssel geändert")) {
            return;
        }
        let Some(f) = self.repo_ui.key_changed.get(url).cloned() else {
            self.dialog = None;
            return;
        };
        let Some(old) = self.data.plugin_repos.iter().find(|r| r.url == url).cloned() else {
            self.dialog = None;
            return;
        };
        ui.label(RichText::new(repo_title(&old)).font(theme::semibold(16.0)).color(t.text));
        ui.add(egui::Label::new(RichText::new(url).size(12.0).color(t.text_faint)).truncate());
        ui.add_space(8.0);
        let mono = egui::FontId::monospace(12.5);
        egui::Grid::new("repo_keys").num_columns(2).spacing(vec2(14.0, 6.0)).show(ui, |ui| {
            ui.label(RichText::new(t!("Bisher")).size(12.5).color(t.text_dim));
            ui.label(RichText::new(repo::fingerprint(&old.key)).font(mono.clone()).color(t.text));
            ui.end_row();
            ui.label(RichText::new(t!("Jetzt")).size(12.5).color(t.text_dim));
            ui.label(RichText::new(repo::fingerprint(&f.index.key)).font(mono).color(t.danger));
            ui.end_row();
        });
        ui.add_space(10.0);
        egui::Frame::new().fill(WARN.gamma_multiply(0.14)).corner_radius(CornerRadius::same(theme::R)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(t!("Velouna hat bemerkt, dass sich der Sicherheitsschlüssel dieses Repos geändert hat.")).font(theme::semibold(13.0)).color(t.text));
            ui.add_space(4.0);
            for line in [
                t!("Das Repo könnte gehackt oder übernommen worden sein."),
                t!("Neue Versionen könnten Schadcode enthalten."),
                t!("Velouna kann nicht prüfen, ob der Wechsel echt ist."),
            ] {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("•").size(12.5).color(WARN));
                    ui.label(RichText::new(line).size(12.5).color(t.text));
                });
            }
            ui.add_space(4.0);
            ui.label(RichText::new(t!("Es wird empfohlen, abzulehnen und 48 Stunden zu warten.")).font(theme::semibold(12.5)).color(t.text));
        });
        ui.add_space(12.0);
        // accepting unlocks only after 30 s with the dialog open
        let now = ui.input(|i| i.time);
        let since = *self.repo_ui.key_since.get_or_insert(now);
        let left = (30.0 - (now - since)).ceil().max(0.0) as u32;
        if left > 0 {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        let mut take = false;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if left > 0 { tf!("Annehmen ({})", left) } else { t!("Annehmen").to_string() };
                if ui.add_enabled_ui(left == 0, |ui| widgets::ghost_button(ui, &t, &label)).inner.clicked() {
                    take = true;
                }
                if widgets::accent_button(ui, &t, t!("Ablehnen")).clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.repo_ui.key_since = None;
            self.dialog = None;
        }
        if take {
            if let Some(r) = self.data.plugin_repos.iter_mut().find(|r| r.url == url) {
                r.key = f.index.key.clone();
                r.name = f.index.name.clone();
            }
            self.repo_ui.key_changed.remove(url);
            self.repo_ui.key_since = None;
            self.repo_ui.indexes.insert(url.to_string(), Ok(f.index.clone()));
            self.mark_dirty();
            self.dialog = None;
        }
    }

    /// "Add repo": the link, then on to the repo's own dialog.
    pub(crate) fn dlg_repo_link(&mut self, ui: &mut Ui) {
        let t = self.t;
        let ctx = ui.ctx().clone();
        crate::ui_dialogs::dlg_width(ui, 480.0);
        if self.dlg_title(ui, t!("Repo hinzufügen")) {
            return;
        }
        ui.label(RichText::new(t!("Link zu einem Repo oder zum Ordner eines einzelnen Plugins darin")).size(12.5).color(t.text_dim));
        ui.add_space(6.0);
        let resp = ui.add(egui::TextEdit::singleline(&mut self.repo_ui.input).desired_width(f32::INFINITY).hint_text(RichText::new("github.com/name/repo").color(t.text_faint)));
        if !self.repo_ui.adding && self.repo_ui.input.is_empty() && !resp.has_focus() {
            resp.request_focus();
        }
        let mut go = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if let Some(e) = &self.repo_ui.add_error {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(e).size(12.5).color(t.danger)).wrap());
        }
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(t!("Plugins aus fremden Repos sind nicht von Velouna geprüft. Füge nur Repos hinzu, denen du vertraust.")).size(12.0).color(t.text_faint)).wrap());
        ui.add_space(12.0);
        let adding = self.repo_ui.adding;
        let filled = !self.repo_ui.input.trim().is_empty();
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled_ui(!adding && filled, |ui| widgets::accent_button(ui, &t, t!("Weiter"))).inner.clicked() {
                    go = true;
                }
                if widgets::ghost_button(ui, &t, t!("Abbrechen")).clicked() {
                    self.dialog = None;
                }
                if adding {
                    ui.add(egui::Spinner::new().size(16.0).color(t.accent));
                }
            });
        });
        if go && !adding && filled {
            self.repo_ui.adding = true;
            self.repo_ui.add_error = None;
            let (tx, input) = (self.repo_ui.tx.clone(), self.repo_ui.input.clone());
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Probe(repo::probe(&input)));
                ctx.request_repaint();
            });
        }
    }

    /// "Build your own repo": packs plugin folders, writes and signs
    /// repo.json. The folder goes onto any web space (a GitHub repo will
    /// do); its link is what others add under Repos.
    fn build_tab(&mut self, ui: &mut Ui) {
        let t = self.t;
        ui.add_space(6.0);
        ui.label(RichText::new(t!("Eigenes Repo bauen")).font(theme::semibold(13.5)).color(t.text));
        ui.add(
            egui::Label::new(
                RichText::new(t!("Packt deine Plugin-Ordner, schreibt die repo.json und signiert sie. Lade den Repo-Ordner danach hoch, zum Beispiel in ein GitHub-Repo. Dessen Link fügen andere unter Repos → „Repo hinzufügen“ ein."))
                    .size(12.0)
                    .color(t.text_dim),
            )
            .wrap(),
        );
        ui.add_space(6.0);
        let label_w = 120.0;
        // all fields equally wide, folder buttons after the first two
        let field_w = (ui.available_width() - label_w - 52.0).max(100.0);
        let field = |ui: &mut Ui, label: &str, value: &mut String, hint: &str, folder: bool| {
            form_row(ui, |ui| {
                form_label(ui, label, label_w, t.text_dim);
                ui.add(egui::TextEdit::singleline(value).desired_width(field_w).hint_text(RichText::new(hint).color(t.text_faint)));
                // (the build tab is desktop only; Android has no folder picker)
                #[cfg(target_os = "android")]
                let _ = folder;
                #[cfg(not(target_os = "android"))]
                if folder && icon_button(ui, &t, ph::FOLDER_OPEN, t!("Ordner wählen"), false).clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        *value = d.to_string_lossy().to_string();
                    }
                }
            });
        };
        let u = &mut self.repo_ui;
        field(ui, t!("Plugins"), &mut u.build_src, t!("Ordner mit einem Unterordner pro Plugin"), true);
        field(ui, t!("Repo-Ordner"), &mut u.build_out, t!("Hier entsteht das Repo"), true);
        field(ui, t!("Name"), &mut u.build_name, t!("z. B. Meine Visualizer"), false);
        field(ui, t!("Betreuer"), &mut u.build_by, t!("dein Name"), false);
        let key_file = crate::config::data_dir().join("repo-signing.key");
        let key = repo::load_key(&key_file).ok();
        form_row(ui, |ui| {
            form_label(ui, t!("Schlüssel"), label_w, t.text_dim);
            match &key {
                Some(k) => {
                    ui.label(RichText::new(repo::fingerprint(&repo::public_key(k))).font(egui::FontId::monospace(12.0)).color(t.text));
                    if icon_button(ui, &t, ph::FOLDER_OPEN, crate::platform::show_in_folder_label(), false).clicked() {
                        crate::ui_panels::show_in_explorer(&key_file.to_string_lossy());
                    }
                }
                None => {
                    ui.label(RichText::new(t!("wird beim ersten Bauen erzeugt")).size(12.5).color(t.text_faint));
                }
            }
        });
        ui.add(
            egui::Label::new(
                RichText::new(t!("Sichere die Schlüsseldatei. Ohne sie kannst du dein Repo nie wieder aktualisieren: Velouna lehnt ein Repo ab, dessen Schlüssel sich ändert."))
                    .size(12.0)
                    .color(WARN),
            )
            .wrap(),
        );
        ui.add_space(6.0);
        let u = &self.repo_ui;
        let ready = !u.build_src.trim().is_empty() && !u.build_out.trim().is_empty() && !u.build_name.trim().is_empty();
        if ui.add_enabled_ui(ready, |ui| widgets::accent_button(ui, &t, t!("Repo bauen"))).inner.clicked() {
            let res = (|| -> Result<Vec<String>, String> {
                let sk = match key {
                    Some(k) => k,
                    None => {
                        let (secret, _) = repo::keygen()?;
                        std::fs::create_dir_all(crate::config::data_dir()).map_err(|e| e.to_string())?;
                        std::fs::write(&key_file, format!("{secret}\n")).map_err(|e| e.to_string())?;
                        repo::load_key(&key_file)?
                    }
                };
                let u = &self.repo_ui;
                let out = std::path::Path::new(u.build_out.trim());
                std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
                let info = repo::RepoInfo { name: u.build_name.trim().to_string(), maintainer: u.build_by.trim().to_string(), homepage: String::new() };
                repo::build(std::path::Path::new(u.build_src.trim()), out, &sk, &info)
            })();
            self.repo_ui.build_log = Some(res);
        }
        match &self.repo_ui.build_log {
            Some(Ok(lines)) => {
                ui.label(RichText::new(tf!("{}  Fertig: {} Plugins, repo.json signiert", ph::CHECK_CIRCLE, lines.len())).size(12.5).color(t.text));
                for l in lines {
                    ui.label(RichText::new(l).font(egui::FontId::monospace(11.5)).color(t.text_dim));
                }
            }
            Some(Err(e)) => {
                ui.add(egui::Label::new(RichText::new(e).size(12.5).color(t.danger)).wrap());
            }
            None => {}
        }
    }

    /// "Add repo": who runs it, its key, and which plugins to install now.
    pub(crate) fn dlg_repo_add(&mut self, ui: &mut Ui, f: &Fetched) {
        let t = self.t;
        crate::ui_dialogs::dlg_width(ui, 500.0);
        if self.dlg_title(ui, t!("Repo hinzufügen")) {
            return;
        }
        let idx = &f.index;
        ui.label(RichText::new(&idx.name).font(theme::semibold(16.0)).color(t.text));
        let mut who = Vec::new();
        if !idx.maintainer.is_empty() {
            who.push(tf!("von {}", idx.maintainer));
        }
        if f.only.is_none() {
            who.push(tf!("{} Plugins", idx.plugins.len()));
        }
        ui.label(RichText::new(who.join(" · ")).size(12.5).color(t.text_dim));
        ui.add(egui::Label::new(RichText::new(&f.url).size(12.0).color(t.text_faint)).truncate());
        ui.label(RichText::new(tf!("Schlüssel {}", repo::fingerprint(&idx.key))).font(egui::FontId::monospace(12.0)).color(t.text_dim));
        ui.add_space(8.0);
        egui::Frame::new().fill(WARN.gamma_multiply(0.14)).corner_radius(CornerRadius::same(theme::R)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(t!("Plugins aus diesem Repo sind nicht von Velouna geprüft. Füge nur Repos hinzu, denen du vertraust. Velouna merkt sich den Schlüssel und lehnt das Repo ab, falls er sich ändert."))
                        .size(12.5)
                        .color(t.text),
                )
                .wrap(),
            );
        });
        if !idx.plugins.is_empty() {
            ui.add_space(10.0);
            let head = if f.only.is_some() { t!("Nur dieses Plugin aus dem Repo:") } else { t!("Gleich installieren:") };
            ui.label(RichText::new(head).font(theme::semibold(13.0)).color(t.text));
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for e in idx.plugins.iter().filter(|e| f.only.as_ref().is_none_or(|id| *id == e.id)) {
                    let fits = e.best();
                    let mut on = self.repo_ui.add_picked.contains(&e.id);
                    let changed = ui
                        .add_enabled_ui(fits.is_some(), |ui| {
                            ui.horizontal(|ui| {
                                let c = ui.checkbox(&mut on, "").changed();
                                ui.label(RichText::new(kind_icon(e.kind)).font(theme::icon(15.0)).color(t.text_dim));
                                ui.vertical(|ui| {
                                    let ver = fits.map(|v| v.version.clone()).unwrap_or_else(|| t!("braucht ein neueres Velouna").to_string());
                                    ui.label(RichText::new(format!("{}  {}", e.name.get(), ver)).size(13.0).color(t.text));
                                    let d = e.description.get();
                                    if !d.is_empty() {
                                        ui.add(egui::Label::new(RichText::new(d).size(12.0).color(t.text_dim)).wrap());
                                    }
                                });
                                c
                            })
                            .inner
                        })
                        .inner;
                    if changed {
                        if on {
                            self.repo_ui.add_picked.insert(e.id.clone());
                        } else {
                            self.repo_ui.add_picked.remove(&e.id);
                        }
                    }
                }
            });
        }
        ui.add_space(12.0);
        let n = self.repo_ui.add_picked.len();
        let mut go = false;
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if n > 0 { tf!("Hinzufügen und {} installieren", n) } else { t!("Hinzufügen").to_string() };
                if widgets::accent_button(ui, &t, &label).clicked() {
                    go = true;
                }
                if widgets::ghost_button(ui, &t, t!("Abbrechen")).clicked() {
                    self.dialog = None;
                }
            });
        });
        if go {
            // a plugin link adds to the plugins of that repo shown so far;
            // a repo link shows the whole repo
            let had = self.data.plugin_repos.iter().find(|x| x.url == f.url).map(|x| x.only.clone());
            let only = match (&f.only, had) {
                (None, _) => Vec::new(),
                (Some(id), None) => vec![id.clone()],
                (Some(_), Some(o)) if o.is_empty() => o,
                (Some(id), Some(mut o)) => {
                    o.push(id.clone());
                    o
                }
            };
            let r = Repo { url: f.url.clone(), name: idx.name.clone(), key: idx.key.clone(), hidden: false, only };
            self.data.plugin_repos.retain(|x| x.url != r.url);
            self.data.plugin_repos.push(r.clone());
            self.repo_ui.indexes.insert(r.url.clone(), Ok(idx.clone()));
            self.repo_ui.input.clear();
            for id in std::mem::take(&mut self.repo_ui.add_picked) {
                self.enqueue_install(&r, &id);
            }
            self.repo_ui.tab = if n > 0 { ExtTab::Installed } else { ExtTab::Catalog };
            self.repo_ui.add_error = None;
            self.mark_dirty();
            self.dialog = None;
        }
    }

    /// Reads a dropped or picked plugin file and asks before installing it.
    pub fn offer_plugin(&mut self, file: &std::path::Path) {
        self.plugins.ensure();
        match plugins::inspect(file, &self.plugins) {
            Ok(pkg) => self.dialog = Some(Dialog::PluginInstall(Box::new(pkg))),
            Err(e) => self.dialog = Some(Dialog::Error(tf!("Plugin kann nicht installiert werden: {}", e))),
        }
    }

    /// Extensions → Installed.
    fn installed_tab(&mut self, ui: &mut Ui, filter: &str) {
        let t = self.t;
        self.plugins.ensure();
        if filter.is_empty() {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if widgets::icon_text_button(ui, &t, ph::FILE_PLUS, t!("Plugin installieren…")).clicked() {
                    self.pick(crate::pick::Pick::Plugin);
                }
                if !crate::platform::MOBILE && widgets::icon_text_button(ui, &t, ph::ARROWS_CLOCKWISE, t!("Neu einlesen")).on_hover_text(t!("Liest die Plugin-Ordner neu ein, auch geänderte Shader und score.lua")).clicked() {
                    self.ext_restart_all();
                    self.viz_manifests.clear();
                    self.viz.lock().unwrap().reload();
                }
            });
            ui.add_space(8.0);
        }
        let f = filter.to_lowercase();
        let ids: Vec<usize> = (0..self.plugins.list.len())
            .filter(|&i| {
                let m = &self.plugins.list[i].manifest;
                f.is_empty() || m.name.get().to_lowercase().contains(&f) || m.description.get().to_lowercase().contains(&f)
            })
            .collect();
        if self.plugins.list.is_empty() && self.plugins.broken.is_empty() && filter.is_empty() {
            ui.label(RichText::new(t!("Noch keine Erweiterungen installiert")).font(theme::semibold(14.0)).color(t.text));
            ui.label(
                RichText::new(if crate::platform::MOBILE {
                    t!("Installiere eine .velounaplugin-Datei über „Plugin installieren“.")
                } else {
                    t!("Ziehe eine .velounaplugin-Datei ins Fenster oder wähle „Plugin installieren“.")
                })
                .size(12.5)
                .color(t.text_dim),
            );
        }
        let mut remove: Option<String> = None;
        let mut open_dialog: Option<Dialog> = None;
        let mut toggled: Option<(String, bool)> = None;
        for i in ids {
            let (id, name, version, kind, desc, mut error, dir, has_settings, missing, other_device) = {
                let p = &self.plugins.list[i];
                let m = &p.manifest;
                let missing = if m.kind == Kind::Lua { crate::plugins::host::Host::missing_grants(p, self.data.plugins.get(&m.id)) } else { vec![] };
                (m.id.clone(), m.name.get(), m.version.clone(), m.kind, m.description.get(), p.error.clone(), p.dir.clone(), !m.settings.is_empty(), missing, !m.made_for(crate::mobile::touch()))
            };
            let lua = kind == Kind::Lua;
            let running = self.ext.is_running(&id);
            let no_plugins = self.ext.disabled;
            if error.is_none() {
                error = self.ext.errors.get(&id).cloned();
            }
            let mut on = self.data.plugins.get(&id).is_some_and(|s| s.enabled);
            let was_on = on;
            let update = self.update_for(&id);
            let busy = self.repo_ui.downloading || !self.repo_ui.queue.is_empty() || self.repo_ui.ready.is_some();
            let mut do_update = false;
            plugin_row(ui, &t, kind_icon(kind), error.is_some(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&name).font(theme::semibold(14.5)).color(t.text));
                    ui.label(RichText::new(format!("{version} · {}", kind.label())).size(12.0).color(t.text_faint));
                });
                match &error {
                    Some(e) => ui.label(RichText::new(tf!("Angehalten: {}", e)).size(12.5).color(t.danger)),
                    None => ui.label(RichText::new(&desc).size(12.5).color(t.text_dim)),
                };
                if lua && was_on && !missing.is_empty() {
                    ui.label(RichText::new(t!("Wartet auf deine Erlaubnis")).size(12.0).color(WARN));
                } else if lua && running {
                    ui.label(RichText::new(t!("● läuft")).size(12.0).color(Color32::from_rgb(60, 190, 110)));
                } else if lua && was_on && no_plugins {
                    ui.label(RichText::new(t!("Plugins sind für diesen Start ausgeschaltet (--no-plugins)")).size(12.0).color(t.text_faint));
                }
                if other_device {
                    ui.label(RichText::new(t!("Für andere Geräte gemacht")).size(12.0).color(t.text_faint));
                }
            }, |ui| {
                let more = icon_button(ui, &t, ph::DOTS_THREE, t!("Mehr"), false);
                egui::Popup::menu(&more).width(220.0).show(|ui| {
                    if has_settings && widgets::menu_item(ui, &t, ph::SLIDERS_HORIZONTAL, t!("Einstellungen…"), "", false).clicked() {
                        ui.close();
                        open_dialog = Some(Dialog::PluginSettings(id.clone()));
                    }
                    if lua && widgets::menu_item(ui, &t, ph::TERMINAL_WINDOW, t!("Konsole…"), "", false).clicked() {
                        ui.close();
                        open_dialog = Some(Dialog::PluginConsole(id.clone()));
                    }
                    if !crate::platform::MOBILE && widgets::menu_item(ui, &t, ph::FOLDER_OPEN, crate::platform::show_in_folder_label(), "", false).clicked() {
                        ui.close();
                        crate::ui_panels::show_in_explorer(&dir.join("plugin.json").to_string_lossy());
                    }
                    if widgets::menu_item(ui, &t, ph::TRASH, t!("Entfernen…"), "", false).clicked() {
                        ui.close();
                        remove = Some(id.clone());
                    }
                });
                ui.add_space(6.0);
                if widgets::toggle(ui, &t, &mut on).on_hover_text(if on { t!("Ausschalten") } else { t!("Einschalten") }).changed() {
                    if lua && on && !missing.is_empty() {
                        // Ask first; the dialog switches it on.
                        open_dialog = Some(Dialog::PluginGrant(id.clone()));
                    } else {
                        self.data.plugins.entry(id.clone()).or_default().enabled = on;
                        self.mark_dirty();
                        if lua {
                            toggled = Some((id.clone(), on));
                        }
                    }
                }
                if let Some((_, v)) = &update {
                    ui.add_space(6.0);
                    if ui.add_enabled_ui(!busy, |ui| widgets::accent_button(ui, &t, &tf!("Update {}", v))).inner.clicked() {
                        do_update = true;
                    }
                }
            });
            if do_update {
                if let Some((r, _)) = &update {
                    self.enqueue_install(r, &id);
                }
            }
        }
        for b in &self.plugins.broken {
            let name = b.dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if !f.is_empty() && !name.to_lowercase().contains(&f) {
                continue;
            }
            let dir = b.dir.clone();
            plugin_row(ui, &t, ph::WARNING, true, |ui| {
                ui.label(RichText::new(&name).font(theme::semibold(14.5)).color(t.text));
                ui.label(RichText::new(tf!("Wird nicht geladen: {}", b.error)).size(12.5).color(t.danger));
            }, |ui| {
                if !crate::platform::MOBILE && icon_button(ui, &t, ph::FOLDER_OPEN, crate::platform::show_in_folder_label(), false).clicked() {
                    crate::ui_panels::show_in_explorer(&dir.to_string_lossy());
                }
            });
        }
        if let Some(id) = remove {
            self.dialog = Some(Dialog::PluginRemove(id));
        }
        if let Some(d) = open_dialog {
            self.dialog = Some(d);
            self.ext_ui.back_to_settings = true;
        }
        match toggled {
            Some((id, true)) => self.ext_start(&id),
            Some((id, false)) => self.ext_stop(&id),
            None => {}
        }
        if filter.is_empty() && !self.plugins.list.is_empty() && !crate::platform::MOBILE {
            ui.add_space(6.0);
            ui.label(RichText::new(t!("Weitere Plugins installierst du, indem du ihre Datei ins Fenster ziehst.")).size(12.0).color(t.text_faint));
        }
    }

    pub(crate) fn dlg_plugin_install(&mut self, ui: &mut Ui, pkg: &Package) {
        let t = self.t;
        crate::ui_dialogs::dlg_width(ui, 460.0);
        let update = pkg.installed.is_some();
        if self.dlg_title(ui, if update { t!("Plugin aktualisieren") } else { t!("Plugin installieren") }) {
            return;
        }
        let m = &pkg.manifest;
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(theme::R_LG), t.elevated);
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, kind_icon(m.kind), theme::icon(22.0), t.accent);
            ui.vertical(|ui| {
                ui.label(RichText::new(m.name.get()).font(theme::semibold(16.0)).color(t.text));
                let ver = match &pkg.installed {
                    Some(old) => format!("{old} → {}", m.version),
                    None => m.version.clone(),
                };
                let mut line = format!("{ver} · {}", m.kind.label());
                if !m.author.is_empty() {
                    line.push_str(&format!(" · {}", m.author));
                }
                ui.label(RichText::new(line).size(12.5).color(t.text_dim));
            });
        });
        let desc = m.description.get();
        if !desc.is_empty() {
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(desc).size(13.0).color(t.text_dim)).wrap());
        }
        ui.add_space(10.0);
        if m.permissions.is_empty() {
            ui.label(RichText::new(tf!("{}  Braucht keine Berechtigungen", ph::CHECK_CIRCLE)).size(13.0).color(t.text));
        } else {
            ui.label(RichText::new(t!("Das Plugin möchte:")).font(theme::semibold(13.0)).color(t.text));
            for p in &m.permissions {
                let (text, critical) = permission(p);
                let new = update && pkg.new_permissions.contains(p);
                let col = if critical { t.danger } else { WARN };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(if critical { ph::WARNING_OCTAGON } else { ph::WARNING }).font(theme::icon(15.0)).color(col));
                    ui.add(egui::Label::new(RichText::new(text).size(13.0).color(t.text)).wrap());
                    if crate::ui_ext::permission_pending(p) {
                        ui.label(RichText::new(t!("(noch ohne Funktion)")).size(11.5).color(t.text_faint)).on_hover_text(t!("Diese Velouna-Version bietet die Funktionen hinter dieser Berechtigung noch nicht an."));
                    }
                    if new {
                        ui.label(RichText::new(t!("neu")).font(theme::semibold(11.5)).color(t.accent));
                    }
                });
            }
        }
        ui.add_space(10.0);
        match &pkg.origin {
            Some(r) if r.is_official() => {
                ui.label(RichText::new(tf!("{}  Aus dem offiziellen Velouna-Repo", ph::SEAL_CHECK)).size(12.5).color(t.text_dim));
            }
            origin => {
                let text = match origin {
                    Some(r) => tf!("Aus dem Repo „{}“. Velouna hat dieses Plugin nicht geprüft. Installiere nur, was aus einer Quelle kommt, der du vertraust.", repo_title(r)),
                    None => t!("Aus einer Datei, nicht aus einem Repo: niemand hat dieses Plugin geprüft. Installiere nur, was du aus einer Quelle hast, der du vertraust.").to_string(),
                };
                egui::Frame::new().fill(WARN.gamma_multiply(0.14)).corner_radius(CornerRadius::same(theme::R)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(t.text)).wrap());
                });
            }
        }
        ui.add_space(12.0);
        let mut go = false;
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = match (update, m.permissions.is_empty()) {
                    (true, _) => t!("Aktualisieren"),
                    (false, true) => t!("Installieren"),
                    (false, false) => t!("Erlauben und installieren"),
                };
                if widgets::accent_button(ui, &t, label).clicked() {
                    go = true;
                }
                if widgets::ghost_button(ui, &t, t!("Abbrechen")).clicked() {
                    self.dialog = None;
                }
            });
        });
        if go {
            match plugins::install(pkg) {
                Ok(()) => {
                    let st = self.data.plugins.entry(m.id.clone()).or_default();
                    if !update {
                        st.enabled = true;
                    }
                    st.granted = m.permissions.clone();
                    st.granted_version = m.version.clone();
                    st.repo = pkg.origin.as_ref().map(|r| r.url.clone()).unwrap_or_default();
                    self.mark_dirty();
                    let was_running = self.ext.is_running(&m.id);
                    self.ext_stop(&m.id);
                    self.plugins.rescan();
                    self.dialog = None;
                    if m.kind == Kind::Lua && (was_running || !update) {
                        self.ext_start(&m.id);
                    }
                    self.osd(if update { tf!("{} aktualisiert", m.name.get()) } else { tf!("{} installiert", m.name.get()) });
                }
                Err(e) => self.dialog = Some(Dialog::Error(tf!("Plugin kann nicht installiert werden: {}", e))),
            }
        }
    }

    pub(crate) fn dlg_plugin_remove(&mut self, ui: &mut Ui, id: &str) {
        let t = self.t;
        crate::ui_dialogs::dlg_width(ui, 420.0);
        self.plugins.ensure();
        let Some(name) = self.plugins.get(id).map(|p| p.manifest.name.get()) else {
            self.dialog = None;
            return;
        };
        if self.dlg_title(ui, &tf!("„{}“ entfernen?", name)) {
            return;
        }
        ui.add(egui::Label::new(RichText::new(t!("Seine Einstellungen und Daten kannst du behalten, falls du es später wieder installierst.")).size(13.0).color(t.text_dim)).wrap());
        ui.add_space(12.0);
        let mut act: Option<bool> = None;
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::accent_button(ui, &t, t!("Daten behalten")).clicked() {
                    act = Some(true);
                }
                if widgets::ghost_button(ui, &t, t!("Alles löschen")).clicked() {
                    act = Some(false);
                }
                if widgets::ghost_button(ui, &t, t!("Abbrechen")).clicked() {
                    self.dialog = None;
                }
            });
        });
        if let Some(keep) = act {
            self.ext_stop(id);
            let res = self.plugins.get(id).map(|p| plugins::uninstall(p, keep)).unwrap_or(Ok(()));
            match res {
                Ok(()) => {
                    if keep {
                        if let Some(s) = self.data.plugins.get_mut(id) {
                            s.enabled = false;
                        }
                    } else {
                        self.data.plugins.remove(id);
                    }
                    self.mark_dirty();
                    self.plugins.rescan();
                    self.dialog = None;
                    self.osd(tf!("{} entfernt", name));
                }
                Err(e) => self.dialog = Some(Dialog::Error(e)),
            }
        }
    }
}

/// One plugin: icon, two text lines, controls on the right.
fn plugin_row(ui: &mut Ui, t: &theme::Theme, icon: &str, bad: bool, text: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    let top = ui.cursor().top();
    let w = ui.available_width();
    ui.painter().line_segment([egui::pos2(ui.cursor().left(), top), egui::pos2(ui.cursor().left() + w, top)], Stroke::new(1.0, t.border.gamma_multiply(0.6)));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.set_min_height(40.0);
        let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
        let (bg, fg) = if bad { (t.danger.gamma_multiply(0.18), t.danger) } else { (t.elevated, t.text_dim) };
        ui.painter().rect_filled(r, CornerRadius::same(theme::R_LG), bg);
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, icon, theme::icon(19.0), fg);
        ui.add_space(4.0);
        // Right side first, so the text gets exactly the room that is left.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            right(ui);
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(vec2(ui.available_width().max(80.0), 40.0), Layout::top_down(Align::Min), text);
        });
    });
    ui.add_space(8.0);
}

fn repo_title(r: &Repo) -> String {
    if r.is_official() {
        t!("Velouna-Plugins").to_string()
    } else if r.name.is_empty() {
        r.url.clone()
    } else {
        r.name.clone()
    }
}
