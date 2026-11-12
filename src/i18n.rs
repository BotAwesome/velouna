//! UI language (German/English). Literal UI strings go through `t!`/`tf!`
//! (compile-time checked against `lang/en.json`); strings that live in
//! constant tables (hotkey names, presets) are looked up at runtime with `tr`.

use crate::config::Language;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// English translations keyed by the German source text.
static TABLE: LazyLock<HashMap<String, String>> = LazyLock::new(|| serde_json::from_str(include_str!("../lang/en.json")).unwrap_or_default());

pub fn is_en() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// ISO code of the active language ("de"/"en"), e.g. for Plex metadata.
pub fn code() -> &'static str {
    if is_en() { "en" } else { "de" }
}

/// Resolves the user's choice (System follows the Windows display language).
pub fn apply(pref: Language) {
    let en = match pref {
        Language::German => false,
        Language::English => true,
        Language::System => !sys_locale::get_locale().is_some_and(|l| l.to_lowercase().starts_with("de")),
    };
    ENGLISH.store(en, Ordering::Relaxed);
}

/// Runtime lookup for German keys stored in constant tables.
pub fn tr(de: &str) -> &str {
    if !is_en() {
        return de;
    }
    TABLE.get(de).map(String::as_str).unwrap_or(de)
}
