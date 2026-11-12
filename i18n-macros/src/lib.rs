//! `t!` and `tf!`: German source strings with compile-time checked English
//! translations from `lang/en.json` (keyed by the German text).
//!
//! - `t!("Abspielen")` → `&'static str` in the active language.
//! - `tf!("noch {}", x)` → `String`, like `format!`; both templates are real
//!   format strings, so placeholders and inline captures are checked by rustc.
//!
//! A German string without an English entry is a compile error, so no text
//! can silently stay untranslated.

use proc_macro::{Literal, TokenStream, TokenTree};
use std::collections::HashMap;
use std::sync::OnceLock;

fn table() -> &'static HashMap<String, String> {
    static T: OnceLock<HashMap<String, String>> = OnceLock::new();
    T.get_or_init(|| {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
        let path = std::path::Path::new(&dir).join("lang").join("en.json");
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    })
}

/// Decodes the contents of a Rust string literal token (`"..."`).
fn unescape(lit: &str) -> Option<String> {
    let inner = lit.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            'u' => {
                chars.next()?; // '{'
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            '\n' => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

fn error(msg: &str) -> TokenStream {
    format!("compile_error!({})", Literal::string(msg)).parse().unwrap()
}

/// Splits the input into the source literal and the remaining tokens.
fn split(input: TokenStream) -> Result<(String, String, String), TokenStream> {
    let mut it = input.into_iter();
    let lit = match it.next() {
        Some(TokenTree::Literal(l)) => l.to_string(),
        _ => return Err(error("expected a string literal")),
    };
    let rest: TokenStream = it.collect();
    let de = unescape(&lit).ok_or_else(|| error("expected a plain string literal"))?;
    match table().get(&de) {
        Some(en) => Ok((lit, Literal::string(en).to_string(), rest.to_string())),
        None => Err(error(&format!("missing English translation in lang/en.json for: {de:?}"))),
    }
}

/// Translated `&'static str`.
#[proc_macro]
pub fn t(input: TokenStream) -> TokenStream {
    match split(input) {
        Ok((de, en, _)) => format!("(if crate::i18n::is_en() {{ {en} }} else {{ {de} }})").parse().unwrap(),
        Err(e) => e,
    }
}

/// Translated `format!`.
#[proc_macro]
pub fn tf(input: TokenStream) -> TokenStream {
    match split(input) {
        Ok((de, en, rest)) => format!("(if crate::i18n::is_en() {{ ::std::format!({en} {rest}) }} else {{ ::std::format!({de} {rest}) }})")
            .parse()
            .unwrap(),
        Err(e) => e,
    }
}
