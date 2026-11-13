//! Flags for the language lists of TV and radio. The images come from
//! flagcdn.com (public domain flags) through the logo loader.

/// (ISO 639-1, ISO 639-3, country) – the country whose flag stands for the
/// language.
const LANGS: &[(&str, &str, &str)] = &[
    ("de", "deu", "de"),
    ("en", "eng", "gb"),
    ("es", "spa", "es"),
    ("fr", "fra", "fr"),
    ("it", "ita", "it"),
    ("pt", "por", "pt"),
    ("ru", "rus", "ru"),
    ("zh", "zho", "cn"),
    ("", "yue", "hk"),
    ("ja", "jpn", "jp"),
    ("ko", "kor", "kr"),
    ("ar", "ara", "sa"),
    ("hi", "hin", "in"),
    ("bn", "ben", "bd"),
    ("pa", "pan", "in"),
    ("ta", "tam", "in"),
    ("te", "tel", "in"),
    ("mr", "mar", "in"),
    ("gu", "guj", "in"),
    ("kn", "kan", "in"),
    ("ml", "mal", "in"),
    ("or", "ori", "in"),
    ("ur", "urd", "pk"),
    ("fa", "fas", "ir"),
    ("", "pes", "ir"),
    ("ps", "pus", "af"),
    ("tr", "tur", "tr"),
    ("nl", "nld", "nl"),
    ("pl", "pol", "pl"),
    ("uk", "ukr", "ua"),
    ("be", "bel", "by"),
    ("ro", "ron", "ro"),
    ("hu", "hun", "hu"),
    ("cs", "ces", "cz"),
    ("sk", "slk", "sk"),
    ("sl", "slv", "si"),
    ("hr", "hrv", "hr"),
    ("sr", "srp", "rs"),
    ("bs", "bos", "ba"),
    ("mk", "mkd", "mk"),
    ("bg", "bul", "bg"),
    ("el", "ell", "gr"),
    ("sq", "sqi", "al"),
    ("sv", "swe", "se"),
    ("da", "dan", "dk"),
    ("no", "nor", "no"),
    ("nb", "nob", "no"),
    ("nn", "nno", "no"),
    ("fi", "fin", "fi"),
    ("is", "isl", "is"),
    ("et", "est", "ee"),
    ("lv", "lav", "lv"),
    ("lt", "lit", "lt"),
    ("lb", "ltz", "lu"),
    ("mt", "mlt", "mt"),
    ("ga", "gle", "ie"),
    ("cy", "cym", "gb-wls"),
    ("gd", "gla", "gb-sct"),
    ("he", "heb", "il"),
    ("id", "ind", "id"),
    ("ms", "msa", "my"),
    ("", "zsm", "my"),
    ("th", "tha", "th"),
    ("vi", "vie", "vn"),
    ("tl", "tgl", "ph"),
    ("", "fil", "ph"),
    ("my", "mya", "mm"),
    ("km", "khm", "kh"),
    ("lo", "lao", "la"),
    ("si", "sin", "lk"),
    ("ne", "nep", "np"),
    ("mn", "mon", "mn"),
    ("ka", "kat", "ge"),
    ("hy", "hye", "am"),
    ("az", "aze", "az"),
    ("kk", "kaz", "kz"),
    ("uz", "uzb", "uz"),
    ("ky", "kir", "kg"),
    ("tg", "tgk", "tj"),
    ("tk", "tuk", "tm"),
    ("sw", "swa", "tz"),
    ("", "swh", "tz"),
    ("am", "amh", "et"),
    ("so", "som", "so"),
    ("ha", "hau", "ng"),
    ("yo", "yor", "ng"),
    ("ig", "ibo", "ng"),
    ("zu", "zul", "za"),
    ("af", "afr", "za"),
    ("ht", "hat", "ht"),
    ("ca", "cat", "ad"),
    ("gl", "glg", "es"),
    ("eu", "eus", "es"),
    ("la", "lat", "va"),
];

/// Flag image URL for an ISO 639-1 or 639-3 language code.
pub fn for_language(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_lowercase();
    if code.is_empty() {
        return None;
    }
    LANGS.iter().find(|(a, b, _)| *a == code || *b == code).map(|(_, _, cc)| url(cc))
}

pub fn url(country: &str) -> String {
    format!("https://flagcdn.com/w160/{}.png", country.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    #[test]
    fn languages() {
        assert_eq!(super::for_language("deu").as_deref(), Some("https://flagcdn.com/w160/de.png"));
        assert_eq!(super::for_language("en").as_deref(), Some("https://flagcdn.com/w160/gb.png"));
        assert_eq!(super::for_language("xyz"), None);
    }
}
