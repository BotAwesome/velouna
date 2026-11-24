//! Crash reports, only on request: a panic writes `crash.txt` into the data
//! folder; on the next start Velouna shows it and asks before sending it.
//! The report has no identifier: version, system, the panic message and
//! where it happened. Home folder paths are shortened to `~`.

use std::path::PathBuf;

pub const ENDPOINT: &str = "https://botaweso.me/velouna/crash/";

fn file() -> PathBuf {
    crate::config::data_dir().join("crash.txt")
}

fn scrub(s: &str) -> String {
    let mut s = s.to_string();
    for home in [dirs::home_dir(), std::env::var_os("USERPROFILE").map(PathBuf::from)].into_iter().flatten() {
        let h = home.to_string_lossy().to_string();
        if h.len() > 3 {
            s = s.replace(&h, "~").replace(&h.replace('\\', "/"), "~");
        }
    }
    s
}

/// Installs the panic hook; the default hook still runs afterwards.
pub fn install() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "?".into());
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        // Frames without names are only addresses: useless and noisy.
        // Only Velouna's own frames; std internals and bare addresses are noise.
        let frames: Vec<&str> = bt.lines().map(str::trim).filter(|l| l.contains("velouna") && !l.contains("crash::install") && !l.starts_with("at /rustc")).take(30).collect();
        let report = format!(
            "Velouna {}\n{} {}\nThread: {thread}\nPanic: {msg}\nAt: {at}\n\n{}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            frames.join("\n")
        );
        let _ = std::fs::write(file(), scrub(&report));
        prev(info);
    }));
}

/// A report from the last run, waiting for the user's answer.
pub fn pending() -> Option<String> {
    std::fs::read_to_string(file()).ok().filter(|s| !s.trim().is_empty())
}

pub fn discard() {
    let _ = std::fs::remove_file(file());
}

/// Sends the report in the background and removes it once it is out.
pub fn send(report: String) {
    std::thread::spawn(move || {
        let r = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .post(ENDPOINT)
            .set("Content-Type", "text/plain; charset=utf-8")
            .send_string(&report);
        match r {
            Ok(_) => discard(),
            Err(e) => log::warn!("crash report: {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    /// Real round trip: `VELOUNA_DATA_DIR=<scratch> cargo test crash_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn crash_live() {
        super::install();
        let _ = std::thread::Builder::new().name("crash-test".into()).spawn(|| panic!("crash report test")).unwrap().join();
        let r = super::pending().expect("report written");
        println!("{r}");
        assert!(r.contains("Panic: crash report test") && r.contains("crash.rs:"));
        super::send(r);
        for _ in 0..100 {
            if super::pending().is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        panic!("not sent");
    }
}
