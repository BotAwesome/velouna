//! File and folder pickers and storage access.
//!
//! Desktop: the native dialog, answered right away. Android: the system
//! picker runs as an activity of its own and answers a few frames later;
//! `App::poll_pick` then hands the paths to `App::picked`.

use crate::app::{App, Dialog, SubScope};
use crate::library::LibLoc;
use crate::media;

/// What a picked file or folder is for.
#[derive(Clone, Debug)]
pub enum Pick {
    /// Play files.
    OpenFiles,
    /// Play a folder.
    OpenFolder,
    QueueFiles,
    QueueFolder,
    /// One subtitle file for what plays now.
    Subtitle,
    /// "Add subtitles" dialog.
    SubtitlesFor(SubScope),
    /// Add to the library; `show` opens it afterwards.
    LibraryFolder { music: bool, show: bool },
    ScreenshotDir,
    RecordDir,
    /// New playlist with these items so far.
    PlaylistFiles(Vec<String>),
    /// A `.velounaplugin` file to install.
    Plugin,
}

/// What Velouna may read on the device (desktop: everything).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageAccess {
    pub videos: bool,
    pub music: bool,
    /// Every file, subtitles and playlists next to the media included.
    pub all_files: bool,
}

impl StorageAccess {
    pub const FULL: StorageAccess = StorageAccess { videos: true, music: true, all_files: true };

    pub fn media(&self) -> bool {
        self.videos && self.music
    }

    pub fn current() -> StorageAccess {
        #[cfg(target_os = "android")]
        {
            crate::platform::android::storage()
                .map(|s| StorageAccess { videos: s.videos, music: s.music, all_files: s.all_files })
                .unwrap_or(StorageAccess::FULL)
        }
        #[cfg(not(target_os = "android"))]
        {
            StorageAccess::FULL
        }
    }
}

/// Asks for the media permissions (Android; nothing to do elsewhere).
pub fn request_storage() {
    #[cfg(target_os = "android")]
    crate::platform::android::request_storage();
}

/// Opens the system page for "all files access" (Android).
pub fn request_all_files() {
    #[cfg(target_os = "android")]
    crate::platform::android::open_all_files_access();
}

impl App {
    /// Opens the picker for `p`.
    pub fn pick(&mut self, p: Pick) {
        #[cfg(target_os = "android")]
        {
            use crate::platform::android::{PickMode, pick};
            match &p {
                Pick::OpenFiles | Pick::QueueFiles | Pick::PlaylistFiles(_) => pick(PickMode::Media, true),
                Pick::OpenFolder | Pick::QueueFolder | Pick::LibraryFolder { .. } | Pick::ScreenshotDir | Pick::RecordDir => pick(PickMode::Folder, false),
                // The subtitle mode is the picker for any single file.
                Pick::Subtitle | Pick::SubtitlesFor(SubScope::File) | Pick::Plugin => pick(PickMode::Subtitles, false),
                Pick::SubtitlesFor(_) => pick(PickMode::Subtitles, true),
            }
            self.pending_pick = Some(p);
        }
        #[cfg(not(target_os = "android"))]
        {
            let paths = self.desktop_dialog(&p);
            if !paths.is_empty() {
                self.picked(p, paths);
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    fn desktop_dialog(&self, p: &Pick) -> Vec<String> {
        use std::path::PathBuf;
        let one = |f: Option<PathBuf>| f.map(|f| vec![f]).unwrap_or_default();
        let mut media_ext: Vec<&str> = media::VIDEO_EXT.to_vec();
        media_ext.extend_from_slice(media::AUDIO_EXT);
        let picked: Vec<PathBuf> = match p {
            Pick::OpenFiles => {
                let mut all = media_ext.clone();
                all.extend_from_slice(media::PLAYLIST_EXT);
                let mut d = rfd::FileDialog::new()
                    .set_title(t!("Medien öffnen"))
                    .add_filter(t!("Medien"), &all)
                    .add_filter("Video", media::VIDEO_EXT)
                    .add_filter("Audio", media::AUDIO_EXT)
                    .add_filter(t!("Alle Dateien"), &["*"]);
                if let Some(dir) = self.current_folder() {
                    d = d.set_directory(dir);
                }
                d.pick_files().unwrap_or_default()
            }
            Pick::OpenFolder => one(rfd::FileDialog::new().set_title(t!("Ordner öffnen")).pick_folder()),
            Pick::QueueFiles => rfd::FileDialog::new().add_filter(t!("Medien"), &media_ext).add_filter(t!("Alle Dateien"), &["*"]).pick_files().unwrap_or_default(),
            Pick::QueueFolder | Pick::ScreenshotDir | Pick::RecordDir => one(rfd::FileDialog::new().pick_folder()),
            Pick::Subtitle | Pick::SubtitlesFor(_) => {
                let mut d = rfd::FileDialog::new().add_filter(t!("Untertitel"), media::SUB_EXT).add_filter(t!("Alle Dateien"), &["*"]);
                if let Some(dir) = self.current_folder() {
                    d = d.set_directory(dir);
                }
                match p {
                    Pick::SubtitlesFor(SubScope::Folder | SubScope::Queue) => d.pick_files().unwrap_or_default(),
                    _ => one(d.pick_file()),
                }
            }
            Pick::LibraryFolder { .. } => one(rfd::FileDialog::new().set_title(t!("Ordner zur Bibliothek hinzufügen")).pick_folder()),
            Pick::PlaylistFiles(_) => rfd::FileDialog::new().pick_files().unwrap_or_default(),
            Pick::Plugin => one(
                rfd::FileDialog::new()
                    .set_title(t!("Plugin installieren"))
                    .add_filter(t!("Velouna-Plugin"), &[crate::plugins::EXT])
                    .pick_file(),
            ),
        };
        picked.into_iter().map(|p| p.to_string_lossy().to_string()).collect()
    }

    /// Android: hands the picker's answer over once it is there.
    #[cfg(target_os = "android")]
    pub fn poll_pick(&mut self, ctx: &eframe::egui::Context) {
        if self.pending_pick.is_none() {
            return;
        }
        match crate::platform::android::take_pick() {
            Some(paths) => {
                if let Some(p) = self.pending_pick.take() {
                    self.picked(p, paths);
                }
            }
            // Resolving the picked files runs in the background.
            None => ctx.request_repaint_after(std::time::Duration::from_millis(150)),
        }
    }

    /// Android: follows the storage permission (the user may change it in
    /// the system settings at any time) and asks once on the first start.
    #[cfg(target_os = "android")]
    pub fn android_storage_tick(&mut self, ctx: &eframe::egui::Context) {
        if self.storage_checked.elapsed() < std::time::Duration::from_millis(1500) {
            return;
        }
        self.storage_checked = std::time::Instant::now();
        // Station logo for the notification, once it is downloaded.
        let path = self.current_path();
        let art = self.stream_logos.get(&path).and_then(|url| self.logos.cached_file(url)).map(|p| p.to_string_lossy().to_string());
        crate::platform::android::set_media_art(art);
        let before = self.storage;
        let now = StorageAccess::current();
        self.storage = now;
        if (now.videos && !before.videos) || (now.music && !before.music) || (now.all_files && !before.all_files) {
            // Just allowed: list again what was hidden so far.
            self.lib.refresh();
            self.lib.music_files.clear();
            self.lib.scanning = false;
            self.lib.start_music_scan(&self.data, ctx);
        }
        // First start: ask by itself once the welcome dialog is done.
        if !self.storage_prompted && !now.media() && self.dialog.is_none() && self.data.settings.onboarding_done && !crate::platform::android::storage_asked() {
            self.storage_prompted = true;
            request_storage();
        }
    }

    /// Does what the pick was for.
    pub fn picked(&mut self, p: Pick, paths: Vec<String>) {
        // "!name": a file Android lets Velouna see but not open.
        let (bad, paths): (Vec<String>, Vec<String>) = paths.into_iter().partition(|s| s.starts_with('!'));
        if let Some(b) = bad.first() {
            self.osd(tf!("Kann nicht geöffnet werden: {}", &b[1..]));
        }
        if paths.is_empty() {
            return;
        }
        let first = paths[0].clone();
        match p {
            Pick::Plugin => self.offer_plugin(std::path::Path::new(&first)),
            Pick::OpenFiles | Pick::OpenFolder => self.open_paths(paths),
            Pick::QueueFiles => self.enqueue(&paths),
            Pick::QueueFolder => {
                let files: Vec<String> = media::list_dir(std::path::Path::new(&first), false).1.into_iter().map(|x| x.to_string_lossy().to_string()).collect();
                if files.is_empty() {
                    self.osd(t!("Keine abspielbaren Dateien gefunden"));
                } else {
                    self.enqueue(&files);
                }
            }
            Pick::Subtitle => {
                if let Some(pl) = &self.player {
                    pl.sub_add(&first, true);
                }
                self.osd(tf!("Untertitel geladen: {}", media::display_name(&first)));
            }
            Pick::SubtitlesFor(SubScope::File) => {
                let cur = self.current_path();
                if let Some(pl) = &self.player {
                    pl.sub_add(&first, true);
                }
                self.data.sub_map.insert(cur, first.clone());
                self.mark_dirty();
                self.osd(tf!("Untertitel geladen: {}", media::display_name(&first)));
                self.dialog = None;
            }
            Pick::SubtitlesFor(scope) => {
                let videos: Vec<String> = if scope == SubScope::Folder {
                    self.current_folder()
                        .map(|f| media::list_dir(&f, false).1.into_iter().map(|p| p.to_string_lossy().to_string()).filter(|p| media::is_video(p)).collect())
                        .unwrap_or_default()
                } else {
                    self.player.as_ref().map(|p| p.st.playlist.iter().map(|e| e.filename.clone()).filter(|f| !media::is_url(f)).collect()).unwrap_or_default()
                };
                let m = media::match_subtitles(&videos, &paths);
                self.dlg.sub_files = paths;
                self.dlg.sub_matches = videos.into_iter().zip(m).collect();
            }
            Pick::LibraryFolder { music, show } => {
                let d = std::path::PathBuf::from(&first);
                let s = &mut self.data.settings;
                let list = if music { &mut s.music_folders } else { &mut s.video_folders };
                if !list.contains(&d) {
                    list.push(d.clone());
                }
                if music {
                    self.lib.music_files.clear();
                    self.lib.scanning = false;
                } else if show {
                    self.lib.go(LibLoc::Videos(Some(d)));
                }
                self.mark_dirty();
                // The folder only shows its files with the storage permission.
                if !StorageAccess::current().media() {
                    request_storage();
                }
            }
            Pick::ScreenshotDir | Pick::RecordDir => {
                let d = std::path::PathBuf::from(&first);
                let _ = std::fs::create_dir_all(&d);
                if matches!(p, Pick::ScreenshotDir) {
                    if let Some(pl) = &self.player {
                        let _ = pl.mpv.set_str("screenshot-directory", &first);
                    }
                    self.data.settings.screenshot_dir = d;
                } else {
                    self.data.settings.record_dir = d;
                }
                self.mark_dirty();
            }
            Pick::PlaylistFiles(mut items) => {
                items.extend(paths);
                self.dialog = Some(Dialog::NewPlaylist(items));
            }
        }
    }
}
