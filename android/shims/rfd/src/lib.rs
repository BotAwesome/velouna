//! Android stand-in for the subset of `rfd::FileDialog` Velour uses.
//! Every dialog reports "cancelled" for now; a Storage Access Framework
//! picker replaces this later.

use std::path::{Path, PathBuf};

#[derive(Default, Clone, Debug)]
pub struct FileDialog;

impl FileDialog {
    pub fn new() -> Self {
        FileDialog
    }
    pub fn set_title(self, _title: impl Into<String>) -> Self {
        self
    }
    pub fn add_filter(self, _name: impl Into<String>, _extensions: &[impl ToString]) -> Self {
        self
    }
    pub fn set_directory<P: AsRef<Path>>(self, _path: P) -> Self {
        self
    }
    pub fn set_file_name(self, _name: impl Into<String>) -> Self {
        self
    }
    pub fn pick_file(self) -> Option<PathBuf> {
        None
    }
    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        None
    }
    pub fn pick_folder(self) -> Option<PathBuf> {
        None
    }
    pub fn save_file(self) -> Option<PathBuf> {
        None
    }
}
