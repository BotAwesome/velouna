//! Helpers for Lua plugins: JSON <-> Lua, hashes, URLs, a small XML reader
//! and the host check behind `network:<host>`.

use mlua::{Lua, Value};
use serde_json::{Map, Value as Json};

const MAX_DEPTH: usize = 64;

/// A JSON value as a Lua value (objects and arrays become tables).
pub fn to_lua(lua: &Lua, v: &Json) -> mlua::Result<Value> {
    to_lua_d(lua, v, 0)
}

fn to_lua_d(lua: &Lua, v: &Json, depth: usize) -> mlua::Result<Value> {
    if depth > MAX_DEPTH {
        return Err(mlua::Error::runtime("data nested too deep"));
    }
    Ok(match v {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => Value::String(lua.create_string(s)?),
        Json::Array(a) => {
            let t = lua.create_table_with_capacity(a.len(), 0)?;
            for x in a {
                t.raw_push(to_lua_d(lua, x, depth + 1)?)?;
            }
            Value::Table(t)
        }
        Json::Object(o) => {
            let t = lua.create_table_with_capacity(0, o.len())?;
            for (k, x) in o {
                t.raw_set(k.as_str(), to_lua_d(lua, x, depth + 1)?)?;
            }
            Value::Table(t)
        }
    })
}

/// A Lua value as JSON. Tables with keys 1..n are arrays, others objects
/// (keys turned into strings); functions and userdata are left out.
pub fn from_lua(v: &Value) -> mlua::Result<Json> {
    from_lua_d(v, 0)
}

fn from_lua_d(v: &Value, depth: usize) -> mlua::Result<Json> {
    if depth > MAX_DEPTH {
        return Err(mlua::Error::runtime("data nested too deep (or a table contains itself)"));
    }
    Ok(match v {
        Value::Nil => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(i) => Json::from(*i),
        Value::Number(n) => serde_json::Number::from_f64(*n).map(Json::Number).unwrap_or(Json::Null),
        Value::String(s) => Json::String(s.to_string_lossy()),
        Value::Table(t) => {
            let len = t.raw_len();
            let mut count = 0usize;
            for pair in t.pairs::<Value, Value>() {
                pair?;
                count += 1;
            }
            if len > 0 && count == len {
                let mut a = Vec::with_capacity(len);
                for i in 1..=len {
                    a.push(from_lua_d(&t.raw_get::<Value>(i)?, depth + 1)?);
                }
                Json::Array(a)
            } else if count == 0 {
                // An empty table: an empty object would surprise as often as
                // an empty array; arrays are the more common case in lists.
                Json::Object(Map::new())
            } else {
                let mut o = Map::new();
                for pair in t.pairs::<Value, Value>() {
                    let (k, x) = pair?;
                    let key = match &k {
                        Value::String(s) => s.to_string_lossy(),
                        Value::Integer(i) => i.to_string(),
                        Value::Number(n) => n.to_string(),
                        Value::Boolean(b) => b.to_string(),
                        _ => continue,
                    };
                    if matches!(x, Value::Function(_) | Value::UserData(_) | Value::Thread(_) | Value::LightUserData(_)) {
                        continue;
                    }
                    o.insert(key, from_lua_d(&x, depth + 1)?);
                }
                Json::Object(o)
            }
        }
        _ => Json::Null,
    })
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// MD5 (RFC 1321). Only for APIs that still sign with it (Last.fm).
pub fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4,
        11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut msg = input.to_vec();
    let bits = (input.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f2 = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f2.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..].copy_from_slice(&d0.to_le_bytes());
    out
}

/// Percent-encoding for URL parts (RFC 3986 unreserved characters stay).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parts of an http(s) URL.
#[derive(Debug, PartialEq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
    pub query: String,
}

pub fn parse_url(u: &str) -> Option<Url> {
    let (scheme, rest) = u.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let auth = &rest[..end];
    let tail = &rest[end..];
    // user:pass@ is not allowed: it would only hide where a request goes.
    if auth.contains('@') || auth.is_empty() {
        return None;
    }
    let (host, port) = if let Some(h) = auth.strip_prefix('[') {
        let (h, p) = h.split_once(']')?;
        (h.to_string(), p.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(None))
    } else {
        match auth.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p.parse().ok()?)),
            None => (auth.to_string(), None),
        }
    };
    let tail = tail.split('#').next().unwrap_or("");
    let (path, query) = tail.split_once('?').unwrap_or((tail, ""));
    Some(Url { scheme, host: host.trim_end_matches('.').to_ascii_lowercase(), port, path: if path.is_empty() { "/".into() } else { path.into() }, query: query.into() })
}

/// A host in the home network or on this device.
pub fn is_local_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".lan") || host.ends_with(".home.arpa") {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v) => v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified(),
            std::net::IpAddr::V6(v) => v.is_loopback() || v.is_unspecified() || (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80,
        };
    }
    !host.contains('.')
}

/// Whether the granted permissions allow a request to `host`.
/// `network:example.com` allows that host only, `network:*.example.com`
/// its subdomains too, `network:*` every public host and `network:local`
/// the home network. A host named exactly is allowed even if it is local.
pub fn host_allowed(granted: &[String], host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    for g in granted {
        let Some(p) = g.strip_prefix("network:") else { continue };
        let p = p.to_ascii_lowercase();
        if p == host {
            return true;
        }
        if let Some(base) = p.strip_prefix("*.") {
            if host == base || host.ends_with(&format!(".{base}")) {
                return true;
            }
        }
    }
    if is_local_host(&host) {
        granted.iter().any(|g| g == "network:local")
    } else {
        granted.iter().any(|g| g == "network:*")
    }
}

/// An XML element: name, attributes, children and text.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct XmlNode {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlNode>,
    pub text: String,
}

/// A forgiving XML reader for feeds and simple APIs: no DTDs, no
/// namespaces resolved (names keep their prefix, `itunes:image`).
pub fn parse_xml(s: &str) -> Result<XmlNode, String> {
    let mut stack: Vec<XmlNode> = vec![XmlNode { name: "#document".into(), ..Default::default() }];
    let b = s.as_bytes();
    let mut i = 0;
    let mut nodes = 0usize;
    while i < b.len() {
        if b[i] == b'<' {
            if s[i..].starts_with("<!--") {
                i = s[i..].find("-->").map(|e| i + e + 3).ok_or("unclosed comment")?;
                continue;
            }
            if s[i..].starts_with("<![CDATA[") {
                let e = s[i..].find("]]>").ok_or("unclosed CDATA")?;
                stack.last_mut().unwrap().text.push_str(&s[i + 9..i + e]);
                i += e + 3;
                continue;
            }
            if s[i..].starts_with("<?") || s[i..].starts_with("<!") {
                i = s[i..].find('>').map(|e| i + e + 1).ok_or("unclosed declaration")?;
                continue;
            }
            let end = find_tag_end(b, i).ok_or("unclosed tag")?;
            let inner = &s[i + 1..end];
            i = end + 1;
            if let Some(name) = inner.strip_prefix('/') {
                let name = name.trim();
                // Close up to the matching element (tolerates unclosed ones).
                if let Some(pos) = stack.iter().rposition(|n| n.name == name) {
                    if pos == 0 {
                        continue;
                    }
                    while stack.len() > pos {
                        let n = stack.pop().unwrap();
                        stack.last_mut().unwrap().children.push(n);
                    }
                }
                continue;
            }
            let self_close = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let (name, attrs) = parse_tag(inner);
            nodes += 1;
            if nodes > 200_000 || stack.len() > 256 {
                return Err("document too large".into());
            }
            let node = XmlNode { name, attrs, ..Default::default() };
            if self_close {
                stack.last_mut().unwrap().children.push(node);
            } else {
                stack.push(node);
            }
        } else {
            let e = s[i..].find('<').map(|e| i + e).unwrap_or(b.len());
            let text = &s[i..e];
            if !text.trim().is_empty() {
                stack.last_mut().unwrap().text.push_str(&unescape(text));
            }
            i = e;
        }
    }
    while stack.len() > 1 {
        let n = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(n);
    }
    let mut doc = stack.pop().unwrap();
    match doc.children.len() {
        0 => Err("no element".into()),
        _ => Ok(doc.children.remove(0)),
    }
}

fn find_tag_end(b: &[u8], start: usize) -> Option<usize> {
    let mut q: Option<u8> = None;
    for (k, &c) in b.iter().enumerate().skip(start + 1) {
        match q {
            Some(x) if c == x => q = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => q = Some(c),
            None if c == b'>' => return Some(k),
            None => {}
        }
    }
    None
}

fn parse_tag(inner: &str) -> (String, Vec<(String, String)>) {
    let inner = inner.trim();
    let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
    let name = inner[..name_end].to_string();
    let mut attrs = Vec::new();
    let mut rest = inner[name_end..].trim_start();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let (val, next) = match after.chars().next() {
            Some(q @ ('"' | '\'')) => match after[1..].find(q) {
                Some(e) => (&after[1..1 + e], &after[e + 2..]),
                None => (&after[1..], ""),
            },
            _ => {
                let e = after.find(char::is_whitespace).unwrap_or(after.len());
                (&after[..e], &after[e..])
            }
        };
        attrs.push((key, unescape(val)));
        rest = next.trim_start();
    }
    (name, attrs)
}

/// `&amp;`, `&lt;`, `&#228;` … in XML and HTML text.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let Some(e) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..e];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Text of an HTML fragment without tags (for descriptions in feeds).
pub fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let t = unescape(&out);
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn xml_to_lua(lua: &Lua, n: &XmlNode) -> mlua::Result<mlua::Table> {
    let t = lua.create_table()?;
    t.raw_set("name", n.name.as_str())?;
    let a = lua.create_table()?;
    for (k, v) in &n.attrs {
        a.raw_set(k.as_str(), v.as_str())?;
    }
    t.raw_set("attrs", a)?;
    t.raw_set("text", n.text.trim())?;
    let c = lua.create_table_with_capacity(n.children.len(), 0)?;
    for ch in &n.children {
        c.raw_push(xml_to_lua(lua, ch)?)?;
    }
    t.raw_set("children", c)?;
    Ok(t)
}

/// The YouTube video id of a URL or of a file name in yt-dlp's
/// `Title [id].ext` form.
pub fn youtube_id(path: &str) -> Option<String> {
    let ok = |s: &str| s.len() == 11 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if let Some(u) = parse_url(path) {
        let host = u.host.trim_start_matches("www.").trim_start_matches("m.").trim_start_matches("music.");
        if host == "youtu.be" {
            let id = u.path.trim_start_matches('/');
            return ok(id).then(|| id.to_string());
        }
        if host == "youtube.com" || host == "youtube-nocookie.com" {
            for kv in u.query.split('&') {
                if let Some(v) = kv.strip_prefix("v=") {
                    if ok(v) {
                        return Some(v.to_string());
                    }
                }
            }
            for pre in ["/shorts/", "/embed/", "/live/", "/v/"] {
                if let Some(id) = u.path.strip_prefix(pre) {
                    let id = id.split('/').next().unwrap_or("");
                    if ok(id) {
                        return Some(id.to_string());
                    }
                }
            }
        }
        return None;
    }
    let name = std::path::Path::new(path).file_stem()?.to_string_lossy().to_string();
    let open = name.rfind('[')?;
    let id = name[open + 1..].strip_suffix(']')?;
    ok(id).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_vectors() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"The quick brown fox jumps over the lazy dog")), "9e107d9d372bb6826bd81d3542a419d6");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(&md5(&[b'a'; 64])), "014842d480b571495a4a0363793f7367");
    }

    #[test]
    fn hosts() {
        let g = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(host_allowed(&g(&["network:api.example.com"]), "api.example.com"));
        assert!(!host_allowed(&g(&["network:api.example.com"]), "evil.com"));
        assert!(!host_allowed(&g(&["network:example.com"]), "api.example.com"));
        assert!(host_allowed(&g(&["network:*.example.com"]), "api.example.com"));
        assert!(!host_allowed(&g(&["network:*.example.com"]), "badexample.com"));
        assert!(host_allowed(&g(&["network:*"]), "anything.org"));
        assert!(!host_allowed(&g(&["network:*"]), "192.168.1.10"));
        assert!(!host_allowed(&g(&["network:*"]), "localhost"));
        assert!(host_allowed(&g(&["network:local"]), "192.168.1.10"));
        assert!(host_allowed(&g(&["network:192.168.1.10"]), "192.168.1.10"));
        assert!(!host_allowed(&g(&[]), "example.com"));
    }

    #[test]
    fn urls() {
        let u = parse_url("https://Example.com:8443/a/b?x=1#f").unwrap();
        assert_eq!((u.scheme.as_str(), u.host.as_str(), u.port, u.path.as_str(), u.query.as_str()), ("https", "example.com", Some(8443), "/a/b", "x=1"));
        assert!(parse_url("https://user@evil.com/").is_none());
        assert_eq!(parse_url("http://[::1]:80/").unwrap().host, "::1");
        assert_eq!(url_encode("a b&ü"), "a%20b%26%C3%BC");
        assert_eq!(url_decode("a%20b%26%C3%BC+c"), "a b&ü c");
        assert_eq!(url_decode("100%"), "100%");
    }

    #[test]
    fn xml() {
        let x = parse_xml(r#"<?xml version="1.0"?><rss><channel><title>A &amp; B</title><item><enclosure url="http://x/1.mp3" length='3'/><title><![CDATA[<b>Ep</b>]]></title></item></channel></rss>"#).unwrap();
        assert_eq!(x.name, "rss");
        let ch = &x.children[0];
        assert_eq!(ch.children[0].text, "A & B");
        assert_eq!(ch.children[1].children[0].attrs[0], ("url".into(), "http://x/1.mp3".into()));
        assert_eq!(ch.children[1].children[1].text, "<b>Ep</b>");
        assert_eq!(strip_tags("<p>Hi&nbsp;<b>there</b></p>"), "Hi there");
    }

    #[test]
    fn youtube() {
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://youtu.be/dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("C:\\v\\Song [dQw4w9WgXcQ].mp4").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("C:\\v\\Song.mp4"), None);
    }
}
