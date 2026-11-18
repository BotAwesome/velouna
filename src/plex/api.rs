//! Blocking HTTP calls against plex.tv and a Plex Media Server.
//! Everything here runs on worker threads, never on the UI thread.

use super::model::{self, Server};
use serde_json::Value;
use std::time::Duration;

const PLEX_TV: &str = "https://plex.tv";

#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
    quick: ureq::Agent,
    pub client_id: String,
    device_name: String,
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn err_text(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(401, _) => t!("Nicht autorisiert (401)").into(),
        ureq::Error::Status(code, r) => format!("HTTP {code} {}", r.status_text()),
        ureq::Error::Transport(t) => {
            let k = t.kind();
            match k {
                ureq::ErrorKind::Dns => t!("Server nicht gefunden").into(),
                ureq::ErrorKind::ConnectionFailed => t!("Verbindung fehlgeschlagen").into(),
                ureq::ErrorKind::Io => t!("Zeitüberschreitung oder Netzwerkfehler").into(),
                _ => t.to_string(),
            }
        }
    }
}

pub fn is_unauthorized(e: &str) -> bool {
    e.contains("401")
}

impl Http {
    pub fn new(client_id: String) -> Http {
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(6)).timeout_read(Duration::from_secs(25)).build();
        let quick = ureq::AgentBuilder::new().timeout_connect(Duration::from_millis(2500)).timeout(Duration::from_millis(3500)).build();
        let host = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());
        Http { agent, quick, client_id, device_name: format!("Velouna ({host})") }
    }

    fn prep(&self, r: ureq::Request, token: Option<&str>) -> ureq::Request {
        let mut r = r
            .set("Accept", "application/json")
            .set("X-Plex-Product", "Velouna")
            .set("X-Plex-Version", env!("CARGO_PKG_VERSION"))
            .set("X-Plex-Client-Identifier", &self.client_id)
            .set("X-Plex-Platform", crate::platform::PLEX_PLATFORM)
            .set("X-Plex-Device", if crate::platform::MOBILE { "Android" } else { "PC" })
            .set("X-Plex-Device-Name", &self.device_name)
            .set("X-Plex-Provides", "player")
            .set("X-Plex-Language", crate::i18n::code());
        if let Some(t) = token {
            r = r.set("X-Plex-Token", t);
        }
        r
    }

    fn json(r: Result<ureq::Response, ureq::Error>) -> Result<Value, String> {
        let resp = r.map_err(err_text)?;
        let text = resp.into_string().map_err(|e| e.to_string())?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| tf!("Ungültige Antwort: {e}"))
    }

    // ------------------------------------------------------------- plex.tv

    /// Starts the PIN login. Returns (pin id, code).
    pub fn create_pin(&self) -> Result<(i64, String), String> {
        let v = Self::json(self.prep(self.agent.post(&format!("{PLEX_TV}/api/v2/pins?strong=true")), None).call())?;
        let id = v.get("id").and_then(|x| x.as_i64()).ok_or(t!("Keine PIN erhalten"))?;
        let code = v.get("code").and_then(|x| x.as_str()).ok_or(t!("Keine PIN erhalten"))?.to_string();
        Ok((id, code))
    }

    pub fn auth_url(&self, code: &str) -> String {
        format!(
            "https://app.plex.tv/auth#?clientID={}&code={}&context%5Bdevice%5D%5Bproduct%5D=Velouna",
            url_encode(&self.client_id),
            url_encode(code)
        )
    }

    /// Polls a PIN; Some(token) once the user confirmed it.
    pub fn check_pin(&self, id: i64) -> Result<Option<String>, String> {
        let v = Self::json(self.prep(self.agent.get(&format!("{PLEX_TV}/api/v2/pins/{id}")), None).call())?;
        Ok(v.get("authToken").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string()))
    }

    pub fn user(&self, token: &str) -> Result<model::User, String> {
        let v = Self::json(self.prep(self.agent.get(&format!("{PLEX_TV}/api/v2/user")), Some(token)).call())?;
        Ok(model::parse_user(&v))
    }

    pub fn resources(&self, token: &str) -> Result<Vec<Server>, String> {
        let v = Self::json(self.prep(self.agent.get(&format!("{PLEX_TV}/api/v2/resources?includeHttps=1&includeRelay=1")), Some(token)).call())?;
        Ok(model::parse_resources(&v))
    }

    /// Finds a working base URL for a server: tests all connections in
    /// parallel and prefers local → remote → relay.
    pub fn pick_connection(&self, server: &Server, last: Option<&str>, local_only: bool) -> Result<String, String> {
        let mut cands: Vec<(u8, String)> = Vec::new();
        if let Some(l) = last {
            cands.push((0, l.to_string()));
        }
        for c in &server.connections {
            if local_only && !c.local {
                continue;
            }
            let prio = if c.relay { 4 } else if c.local { 1 } else { 3 };
            cands.push((prio, c.uri.clone()));
            if c.local && !c.address.is_empty() && c.port > 0 {
                cands.push((2, format!("http://{}:{}", c.address, c.port)));
            }
        }
        cands.sort();
        cands.dedup_by(|a, b| a.1 == b.1);
        if cands.is_empty() {
            return Err(t!("Keine Verbindung zum Server bekannt").into());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        for (prio, uri) in cands.clone() {
            let tx = tx.clone();
            let me = self.clone();
            let token = server.token.clone();
            std::thread::spawn(move || {
                let ok = me.prep(me.quick.get(&format!("{uri}/identity")), Some(&token)).call().is_ok();
                let _ = tx.send((prio, uri, ok));
            });
        }
        drop(tx);
        let mut ok: Vec<(u8, String)> = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_millis(4000);
        let mut remaining = cands.len();
        while remaining > 0 {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok((p, u, good)) => {
                    remaining -= 1;
                    if good {
                        ok.push((p, u));
                        // The best possible candidate answered: no need to wait.
                        if ok.iter().map(|x| x.0).min() == cands.iter().map(|x| x.0).min() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        ok.sort();
        ok.into_iter().next().map(|x| x.1).ok_or_else(|| t!("Server ist nicht erreichbar").into())
    }

    // -------------------------------------------------------------- server

    pub fn get(&self, base: &str, token: &str, path: &str) -> Result<Value, String> {
        Self::json(self.prep(self.agent.get(&format!("{base}{path}")), Some(token)).call())
    }

    /// Fire-and-forget style call (timeline, scrobble); body ignored.
    pub fn send(&self, base: &str, token: &str, method: &str, path: &str) -> Result<(), String> {
        let r = match method {
            "PUT" => self.agent.put(&format!("{base}{path}")),
            "POST" => self.agent.post(&format!("{base}{path}")),
            _ => self.agent.get(&format!("{base}{path}")),
        };
        self.prep(r, Some(token)).call().map(|_| ()).map_err(err_text)
    }

    /// Jellyfin/Emby request: JSON in and out, MediaBrowser `Authorization`
    /// header, none of the Plex headers. `quick` uses the short timeouts.
    pub fn mb(&self, method: &str, url: &str, auth: &str, body: Option<&Value>, quick: bool) -> Result<Value, String> {
        let agent = if quick { &self.quick } else { &self.agent };
        let r = agent.request(method, url).set("Accept", "application/json").set("Authorization", auth);
        let r = match body {
            Some(b) => r.set("Content-Type", "application/json").send_string(&b.to_string()),
            None if method == "POST" => r.send_string(""),
            None => r.call(),
        };
        Self::json(r)
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        let resp = self.prep(self.agent.get(url), None).set("Accept", "image/jpeg,image/png,image/webp,*/*").call().map_err(err_text)?;
        let mut buf = Vec::new();
        use std::io::Read;
        resp.into_reader().take(40 * 1024 * 1024).read_to_end(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    }
}
