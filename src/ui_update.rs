//! Update dialog (check, changelog, download, install) and "What's new"
//! after an update.

use crate::app::{App, Dialog};
use crate::theme::{self, Theme};
use crate::ui_dialogs::dlg_width;
use crate::update::{self, Status};
use crate::widgets;
use eframe::egui::{self, CornerRadius, RichText, Sense, Ui, vec2};
use std::sync::atomic::Ordering;

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// `[text](url)` → text, `**bold**` / `` `code` `` → plain.
fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match (after.find("]("), after.find(')')) {
            (Some(a), Some(b)) if a < b => {
                out.push_str(&after[..a]);
                rest = &after[b + 1..];
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "")
}

#[derive(Debug, PartialEq)]
enum Block {
    Gap,
    Heading(String),
    Subheading(String),
    /// Indent (spaces before the "-"), text.
    Bullet(usize, String),
    Para(String),
}

/// Lines → blocks. A line that is neither blank, heading nor bullet
/// continues the bullet or paragraph above it (wrapped changelog lines).
fn blocks(text: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for line in text.lines() {
        let l = line.trim_end();
        let trimmed = l.trim_start();
        if trimmed.is_empty() {
            out.push(Block::Gap);
        } else if let Some(h) = trimmed.strip_prefix("### ") {
            out.push(Block::Subheading(plain(h)));
        } else if let Some(h) = trimmed.strip_prefix("## ").or_else(|| trimmed.strip_prefix("# ")) {
            // "[0.1.0] - 2026-11-12" → "0.1.0 – 2026-11-12"
            out.push(Block::Heading(plain(h).replace(['[', ']'], "").replace(" - ", " – ")));
        } else if let Some(b) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            out.push(Block::Bullet(l.len() - trimmed.len(), plain(b)));
        } else if let Some(Block::Bullet(_, s) | Block::Para(s)) = out.last_mut() {
            s.push(' ');
            s.push_str(&plain(trimmed));
        } else {
            out.push(Block::Para(plain(trimmed)));
        }
    }
    out
}

/// The bit of markdown release notes use: headings, bullets, paragraphs.
pub fn markdown(ui: &mut Ui, t: &Theme, text: &str) {
    for b in blocks(text) {
        match b {
            Block::Gap => ui.add_space(4.0),
            Block::Subheading(h) => {
                ui.add_space(4.0);
                ui.label(RichText::new(h).font(theme::semibold(13.5)).color(t.text));
            }
            Block::Heading(h) => {
                ui.add_space(6.0);
                ui.label(RichText::new(h).font(theme::semibold(15.5)).color(t.text));
            }
            Block::Bullet(indent, s) => {
                ui.horizontal_top(|ui| {
                    ui.add_space(4.0 + indent as f32 * 6.0);
                    ui.label(RichText::new("•").color(t.accent));
                    ui.add(egui::Label::new(RichText::new(s).color(t.text_dim)).wrap());
                });
            }
            Block::Para(s) => {
                ui.add(egui::Label::new(RichText::new(s).color(t.text_dim)).wrap());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Block, blocks};

    #[test]
    fn wrapped_bullets_join() {
        let b = blocks("### Android\n- The app is signed. If you installed it, uninstall it once and\n  install this APK\n- Next\n\nText\nmore");
        assert_eq!(
            b,
            vec![
                Block::Subheading("Android".into()),
                Block::Bullet(0, "The app is signed. If you installed it, uninstall it once and install this APK".into()),
                Block::Bullet(0, "Next".into()),
                Block::Gap,
                Block::Para("Text more".into()),
            ]
        );
    }
}

fn notes_box(ui: &mut Ui, t: &Theme, notes: &str, max_h: f32) {
    egui::Frame::new().fill(t.elevated).corner_radius(CornerRadius::same(theme::R)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if notes.trim().is_empty() {
                ui.label(RichText::new(t!("Keine Änderungsnotizen.")).color(t.text_faint));
            } else {
                markdown(ui, t, notes);
            }
        });
    });
}

impl App {
    pub(crate) fn dlg_update(&mut self, ui: &mut Ui) {
        let t = self.t;
        dlg_width(ui, 540.0);
        let status = self.update.status.clone();
        let title = match &status {
            Status::Available(r) | Status::Downloading(r) | Status::Ready(r, _) | Status::Installing(r) => tf!("Velouna {} ist verfügbar", r.version),
            Status::UpToDate => t!("Velouna ist aktuell").to_string(),
            Status::Failed(_) => t!("Update fehlgeschlagen").to_string(),
            Status::Idle | Status::Checking => t!("Nach Updates suchen").to_string(),
        };
        if self.dlg_title(ui, &title) {
            return;
        }
        let max_notes = (ui.ctx().screen_rect().height() * 0.45).clamp(140.0, 340.0);
        match status {
            Status::Idle | Status::Checking => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().color(t.accent));
                    ui.label(RichText::new(t!("Suche nach Updates…")).color(t.text_dim));
                });
            }
            Status::UpToDate => {
                ui.label(RichText::new(tf!("Du hast die neueste Version ({}).", update::CURRENT)).color(t.text_dim));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if widgets::accent_button(ui, &t, t!("OK")).clicked() {
                        self.dialog = None;
                    }
                    if widgets::ghost_button(ui, &t, t!("Alle Versionen")).clicked() {
                        crate::platform::open_url(&update::releases_page());
                    }
                });
            }
            Status::Failed(e) => {
                ui.label(RichText::new(t!("Die Update-Suche oder der Download hat nicht geklappt:")).color(t.text_dim));
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(e).color(t.danger)).wrap());
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if widgets::accent_button(ui, &t, t!("Erneut versuchen")).clicked() {
                        self.update.check(ui.ctx(), true);
                    }
                    if widgets::ghost_button(ui, &t, t!("Auf GitHub öffnen")).clicked() {
                        crate::platform::open_url(&update::releases_page());
                    }
                });
            }
            Status::Available(rel) | Status::Downloading(rel) | Status::Ready(rel, _) | Status::Installing(rel) => {
                ui.label(RichText::new(tf!("Installiert: {} · Neu: {}", update::CURRENT, rel.version)).size(12.5).color(t.text_faint));
                ui.add_space(8.0);
                notes_box(ui, &t, &rel.notes, max_notes);
                ui.add_space(12.0);
                self.update_actions(ui, &t, &rel);
            }
        }
    }

    fn update_actions(&mut self, ui: &mut Ui, t: &Theme, rel: &update::Release) {
        let ctx = ui.ctx().clone();
        match self.update.status.clone() {
            Status::Available(_) => {
                let Some(asset) = &rel.asset else {
                    ui.label(RichText::new(t!("Dieses Release enthält keine Datei für dieses Gerät.")).color(t.text_dim));
                    ui.add_space(6.0);
                    if widgets::accent_button(ui, t, t!("Auf GitHub öffnen")).clicked() {
                        crate::platform::open_url(&rel.page);
                    }
                    return;
                };
                ui.horizontal_wrapped(|ui| {
                    if widgets::accent_button(ui, t, &tf!("Jetzt aktualisieren ({})", mb(asset.size))).clicked() {
                        self.update.download(&ctx);
                    }
                    if widgets::ghost_button(ui, t, t!("Später")).clicked() {
                        self.dialog = None;
                    }
                    if widgets::ghost_button(ui, t, t!("Version überspringen")).clicked() {
                        self.data.settings.update_skip = rel.version.clone();
                        self.mark_dirty();
                        self.dialog = None;
                    }
                });
            }
            Status::Downloading(_) => {
                let (done, total) = (self.update.done.load(Ordering::Relaxed), self.update.total.load(Ordering::Relaxed).max(1));
                let w = ui.available_width();
                let (r, _) = ui.allocate_exact_size(vec2(w, 6.0), Sense::hover());
                widgets::progress_line(ui.painter(), r, (done as f32 / total as f32).clamp(0.0, 1.0), t);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tf!("Wird heruntergeladen… {} von {}", mb(done), mb(total))).size(12.5).color(t.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::ghost_button(ui, t, t!("Abbrechen")).clicked() {
                            self.update.cancel_download();
                        }
                    });
                });
            }
            Status::Ready(..) => {
                // Android: Velouna needs "install unknown apps" once; ask for it
                // here instead of letting the system dialog block the install.
                #[cfg(target_os = "android")]
                let allowed = crate::platform::android::can_install_packages();
                #[cfg(not(target_os = "android"))]
                let allowed = true;
                if !allowed {
                    ui.label(
                        RichText::new(t!("Damit Velouna sich aktualisieren kann, musst du es einmal erlauben: Tippe auf „Erlauben“ und schalte „Apps aus dieser Quelle zulassen“ ein. Danach geht es hier weiter."))
                            .size(12.5)
                            .color(t.text_dim),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if widgets::accent_button(ui, t, t!("Erlauben")).clicked() {
                            self.update.remember_ready();
                            #[cfg(target_os = "android")]
                            crate::platform::android::open_install_permission();
                        }
                        if widgets::ghost_button(ui, t, t!("Später")).clicked() {
                            self.dialog = None;
                        }
                    });
                    // Picks the change up when the user comes back.
                    ctx.request_repaint_after(std::time::Duration::from_millis(500));
                    return;
                }
                let (label, info) = if crate::platform::MOBILE {
                    (t!("Installieren"), t!("Android fragt gleich nach, ob Velouna aktualisiert werden darf."))
                } else {
                    (t!("Installieren und neu starten"), t!("Velouna wird kurz geschlossen und startet danach von selbst neu."))
                };
                #[cfg(target_os = "linux")]
                let info = if crate::update::linux_deb() {
                    t!("Zum Installieren des Pakets fragt Linux gleich nach deinem Passwort. Danach startet Velouna von selbst neu.")
                } else {
                    info
                };
                ui.label(RichText::new(info).size(12.5).color(t.text_dim));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if widgets::accent_button(ui, t, label).clicked() && self.update.install() {
                        // Windows: the setup waits for Velouna to quit.
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if widgets::ghost_button(ui, t, t!("Später")).clicked() {
                        self.dialog = None;
                    }
                });
            }
            Status::Installing(_) => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().color(t.accent));
                    let text = if crate::platform::MOBILE { t!("Bestätige die Installation im Android-Dialog.") } else { t!("Wird installiert…") };
                    ui.label(RichText::new(text).color(t.text_dim));
                });
            }
            _ => {}
        }
    }

    pub(crate) fn dlg_whats_new(&mut self, ui: &mut Ui, notes: String) {
        let t = self.t;
        dlg_width(ui, 540.0);
        if self.dlg_title(ui, &tf!("Neu in Velouna {}", update::CURRENT)) {
            return;
        }
        let max_notes = (ui.ctx().screen_rect().height() * 0.5).clamp(160.0, 380.0);
        notes_box(ui, &t, &notes, max_notes);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if widgets::accent_button(ui, &t, t!("Weiter")).clicked() {
                self.dialog = None;
            }
            if widgets::ghost_button(ui, &t, t!("Alle Versionen")).clicked() {
                crate::platform::open_url(&update::releases_page());
            }
        });
    }

    /// Settings rows for updates (General page).
    pub(crate) fn update_settings_status(&self) -> String {
        match &self.update.status {
            Status::Checking => t!("Suche nach Updates…").to_string(),
            Status::UpToDate => t!("Velouna ist aktuell.").to_string(),
            Status::Available(r) | Status::Ready(r, _) => tf!("Version {} ist verfügbar.", r.version),
            Status::Downloading(_) => t!("Update wird heruntergeladen…").to_string(),
            Status::Installing(_) => t!("Update wird installiert…").to_string(),
            Status::Failed(_) => t!("Die letzte Suche hat nicht geklappt.").to_string(),
            Status::Idle => t!("Updates kommen von GitHub.").to_string(),
        }
    }

    pub(crate) fn open_update_dialog(&mut self, ctx: &egui::Context) {
        if !matches!(self.update.status, Status::Available(_) | Status::Downloading(_) | Status::Ready(..) | Status::Installing(_)) {
            self.update.check(ctx, true);
        }
        self.dialog = Some(Dialog::Update);
    }
}
